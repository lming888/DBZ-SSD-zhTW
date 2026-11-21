use crate::canvas::GameView;
use crate::debugger::{self, Banner, StepKind};
use crate::egui_input::InputAdapter;
use crate::filter;
use crate::hd::{self, Hd};
use crate::hd_art;
use crate::hook_setup::{DEFAULT_DIVERGENCE_DIR, HookSetup, HookSwitch, HooksArg};
use crate::hotkeys::{self, Speed};
use crate::input::InputState;

use crate::options::audio::AudioSettings;

use crate::options::controls::{self as bindings, Bindings};

use crate::options::video::{Start, VideoSettings};
use crate::packs;
use crate::panel::{
    Controls, DebugAction, DivergenceWatch, Effect, FpsCounter, HooksSwitch, KeyRoute, PanelAction,
    PanelState, ReplayAction, Tab, route_key,
};
use crate::quit_confirm::{self, QuitConfirm};
use crate::replay_io::Session;
use crate::replays::{self, Outcome, REPLAYS_DIR, Verification};
use crate::screenshot;
use crate::snapshot;
use crate::tab_memory;
use crate::tab_routines::{self, FrameHistory};
use crate::touch::{self, Touch};
use crate::ui::{self, Live, Status, View};
use crate::ui_script::{Command, Line, MenuCommand, MenuShow};
use crate::{audio, paths, saves, video};
use anyhow::Context;
use sdl3::event::{Event, WindowEvent};
use sdl3::gamepad::Gamepad;
use sdl3::keyboard::Scancode;
use ssd_game::Ruleset;
use ssd_game::menu::driver::Curtain;
use ssd_hd::world::WideFrame;
use ssd_hd::{HdFrame, HdOptions, png};
use ssd_menus::Screen;
use ssd_menus::options::Values;
use ssd_oracle::report::ReproHeader;
use ssd_oracle::{
    AudioSink, CORE_VERSION, Frame, FrameSize, HookAction, HookMode, HookStats, HostEvent, Joypad,
    LockstepRun, LockstepStep, MODE_ENHANCED, Machine, NullAudio, Replay, StepOptions, Stopped,
    Stops, Symbols, repro,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub mod bench;

/// The dispatch counters of a run's hooked routines, in table order (empty without hooks).
pub type RunStats = Vec<(String, HookStats)>;

/// Verification work per window frame: the run goes on this long, then the frame is drawn.
const VERIFY_SLICE: Duration = Duration::from_millis(10);
/// Frames of an HD menu choice run per window frame while the choice runs (the curtain is up, so
/// the picture waits for it): a choice takes a few hundred frames.
const HD_BURST: u32 = 64;

/// The machine of a run at power-on (the replay's when `session` plays one, else with the battery
/// save of the saves folder, see [`saves::battery_saves`]), hooks installed.
fn new_machine(
    rom: Vec<u8>,
    hooks: Option<&HookSetup>,
    session: &mut Session,
) -> anyhow::Result<Machine> {
    let mut machine = session.power_on(rom, saves::battery_saves)?;
    if let Some(hooks) = hooks {
        hooks.install(&mut machine);
    }
    Ok(machine)
}

/// A machine at power-on for `session` (its replay's, or with the save files), with the hook mode
/// `mode` installed when there is a symbols file to take its table from.
fn fresh_machine(
    rom: &[u8],
    session: &mut Session,
    mode: HooksArg,
    switch: Option<&HookSwitch>,
) -> anyhow::Result<Machine> {
    let mut machine = session.power_on(rom.to_vec(), saves::battery_saves)?;
    if let Some(switch) = switch.filter(|_| mode != HooksArg::Original) {
        switch.apply(&mut machine, mode)?;
    }
    Ok(machine)
}

/// Ends a run: SRAM is flushed whether the run succeeded or not, since the core only writes it
/// every 30 frames and the last half second of a save would be lost. The run's own error wins
/// over a flush error.
fn flush_after<T>(machine: &mut Machine, result: anyhow::Result<T>) -> anyhow::Result<T> {
    match (result, machine.flush_saves()) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(flush)) => Err(flush).context("saving SRAM"),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(flush)) => {
            log::warn!("saving SRAM after the error failed too: {flush}");
            Err(error)
        }
    }
}

/// Runs `frames` frames without a window or audio (or fewer: a replay played ends the run with
/// its last frame), optionally writing the last frame as a BMP. Without a replay no button is
/// pressed.
pub fn run_headless(
    rom: Vec<u8>,
    frames: u64,
    screenshot: Option<&Path>,
    hooks: Option<&HookSetup>,
    mut session: Session,
) -> anyhow::Result<RunStats> {
    let mut machine = new_machine(rom, hooks, &mut session)?;
    let result = (|| {
        for _ in 0..frames {
            if session.playback_over(&machine) {
                log::info!("the replay ends at frame {}", machine.frame_count());
                break;
            }
            session.run_frame(&mut machine, Joypad::default(), &mut NullAudio)?;
        }
        if let Some(path) = screenshot {
            write_bmp(path, machine.frame())?;
        }
        Ok(())
    })();
    let result = flush_after(&mut machine, result);
    session.finish(result)?;
    Ok(machine.hook_stats())
}

/// Minimal 24-bit BMP writer (no image dependency).
pub fn write_bmp(path: &Path, frame: ssd_oracle::Frame<'_>) -> anyhow::Result<()> {
    let (w, h) = (frame.size.width as usize, frame.size.height as usize);
    let row = (w * 3).div_ceil(4) * 4;
    let size = 54 + row * h;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54_u32.to_le_bytes());
    out.extend_from_slice(&40_u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&24_u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for y in (0..h).rev() {
        for x in 0..w {
            let p = frame.pixels[y * w + x];
            out.extend_from_slice(&[p.b, p.g, p.r]);
        }
        out.resize(out.len() + (row - w * 3), 0);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, out)?;
    Ok(())
}

fn window_title(speed: Speed, slot: u8) -> String {
    format!("{} — ×{} — slot {}", crate::TITLE, speed.multiplier(), slot)
}

/// Writes the machine state to the slot file. The write goes through a temporary file so a crash
/// or full disk never leaves a truncated state where a good one was.
fn save_state(machine: &Machine, slot: u8) -> anyhow::Result<PathBuf> {
    let path = hotkeys::state_path(slot);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = path.with_extension("ssdstate.tmp");
    let written = std::fs::write(&tmp, machine.save_state())
        .and_then(|()| std::fs::rename(&tmp, &path))
        .with_context(|| format!("writing {}", path.display()));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written.map(|()| path)
}

/// The file beside a state file that keeps the HD menus' part of the state (`slot1.hd` beside `slot1.ssdstate`).
fn session_path(state: &std::path::Path) -> PathBuf {
    state.with_extension("hd")
}

/// Writes the HD menus' part of the state at `state` beside it, or removes an older one (a Classic state has none).
fn save_session(state: &std::path::Path, session: Option<&[u8]>) {
    let path = session_path(state);
    let written = match session {
        Some(bytes) => std::fs::write(&path, bytes),
        None => match std::fs::remove_file(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
    };
    if let Err(e) = written {
        log::warn!("the bag and the cheats of the state were not written: {e}");
    }
}

/// The HD menus' part of the state at `state`, if it has one.
fn read_session(state: &std::path::Path) -> Option<Vec<u8>> {
    std::fs::read(session_path(state)).ok()
}

/// Loads the slot file; on any error the machine is left untouched.
fn load_state(machine: &mut Machine, slot: u8) -> anyhow::Result<()> {
    let path = hotkeys::state_path(slot);
    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    machine.load_state(&bytes).with_context(|| format!("loading {}", path.display()))
}

/// What the windowed loop keeps between iterations besides the machine.
struct WindowState {
    controls: Controls,
    panel: PanelState,
    watch: DivergenceWatch,
    /// One frame to run although paused ("next frame").
    step_once: bool,
    /// The frames the header and the Routines tab show figures of.
    history: FrameHistory,
    /// The mean instructions the routines measured, by name: given to every table and machine that
    /// replaces the one they were measured on (see [`keep_means`]).
    means: BTreeMap<String, f64>,
    /// Whether the end of the replay being played has been logged.
    replay_over_logged: bool,
    /// The HD menus of Enhanced mode, and the 16:9 view of the run; `None` in Classic.
    hd: Option<Hd>,
    /// The last frame's 16:9 view and the frame count it was made at: the window shows it while
    /// the machine is at that frame (after a state is loaded, the PPU's own frame until one runs).
    view: Option<(u64, WideFrame)>,
    /// Whether the Enhanced picture is drawn at a scale (`--hd-scale`): it is the Classic frame's
    /// proportions, so the 16:9 view is not made and the menus' backdrop is the PPU's own frame.
    scaled: Option<HdOptions>,
    /// The Enhanced picture of the last frame in the 16:9 view (with `--hd-scale` in Enhanced
    /// mode) and the frame count it was made at.
    hd_view: Option<(u64, ssd_hd::world::SharedHd)>,
    /// Where the time of the frame loop went since the counters were last taken (the bench).
    stages: Stages,
    /// The last frames run were a fast-forward's batch (see [`run_frames`]).
    fast_forward: bool,
    /// The game runs at speed 1: a batch is frames the clock owes (a screen slower than the game), not a
    /// fast-forward, so every frame's sound is heard (see [`run_frames`]).
    real_time: bool,
    /// The Gameplay page's battle speed runs ([`set_battle_tempo`]): the frames of a window frame are not a
    /// fast-forward's batch but the game's own at 1.5 or 2 times, each run whole (its render log, its pixels) and followed
    /// as at speed 1, and the last one drawn and shown, so what the window shows of a frame is what speed 1 shows of
    /// it (see [`run_frames`]).
    whole_frames: bool,
    /// The Video page's choices as the run shows them (the command line's scale and look included).
    video: VideoSettings,
    /// The choices the PC file held when they were last read: the player's changes are measured from them.
    video_saved: VideoSettings,
    /// The window is to be told of a change of the choices.
    video_apply: bool,
    /// The save states a run that does not persist made, by slot: they stay in memory, as no state file is written
    /// (see [`apply_controls`]).
    kept_states: BTreeMap<u8, Vec<u8>>,
    /// The HD menus' part of each of those states (see [`Hd::session_bytes`]).
    kept_sessions: BTreeMap<u8, Vec<u8>>,
    /// The keys and buttons of the player's Controls page.
    bindings: Bindings,
    /// The Controls page changed the map: the keys held under the old one are let go.
    release_input: bool,
    /// The touch controls (M9): off unless the run has them (Android, or `--touch-ui`).
    touch: Touch,
    /// The turbo of held buttons (Options > Controls > Turbo): how long A and B have been held.
    turbo: crate::turbo::Turbo,
    /// A screenshot asked for, taken once the frame is drawn (windowed runs only).
    shot: Option<screenshot::Kind>,
    /// The window frame being run starts where the picture shown left off: the Enhanced picture on show is that of the
    /// frame the machine is at, so nothing was loaded, reset or stepped since (see [`window_frames`]).
    continuous: bool,
    /// The last frame run left the PPU's pixels undrawn (see [`may_skip_pixels`]): the picture is the one drawn from the
    /// render log, and [`Machine::frame`] holds an older frame.
    ppu_stale: bool,
    /// The last frame run was one the Enhanced picture refuses with the PPU's pixels undrawn: the window keeps the
    /// picture it had, for that one frame.
    held_picture: bool,
    /// The frames left in which the PPU draws its pixels whatever the run (see [`may_skip_pixels`]): a frame the
    /// Enhanced picture refused was seen lately.
    compose_hold: u32,
    /// "Quit the game?", while Esc's question is on screen: the game is held until it closes.
    quit_confirm: Option<QuitConfirm>,
    /// Goku frame by frame, for the voice of his first Super Saiyan transformation (see [`crate::voice`]).
    ssj_watch: crate::voice::SsjWatch,
    /// A frame run started Goku's Super Saiyan transformation: the voice starts with the window frame's sound.
    voice_due: bool,
}

/// The time the frame loop spent in its first stages: the HD menu's driver, the machine's frames,
/// the figures of the Routines tab, and the Enhanced picture of the frame shown.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Stages {
    pub driver: Duration,
    pub emulate: Duration,
    pub history: Duration,
    pub hd: Duration,
}

impl Stages {
    /// Everything the frames cost: what the speed multiplies.
    pub fn frames(&self) -> Duration {
        self.driver + self.emulate + self.history + self.hd
    }
}

/// A window frame that takes longer than this, from its start to its present's end, is logged ([`slow_frame_line`]).
const SLOW_FRAME: Duration = Duration::from_millis(100);
/// The most slow window frames logged in a second; the others are counted only.
const SLOW_LINES_A_SECOND: u32 = 2;
/// How often the slow window frames are summed up in the log (as the sound is, `audio::REPORT_EVERY`).
const SLOW_REPORT_EVERY: Duration = Duration::from_secs(5);

/// Where the Enhanced picture of a window frame was drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrawnOn {
    /// On the window's thread, in the window frame (no Latency Smooth, or a picture that must be shown at once).
    Window,
    /// On the thread `hd-draw`: Latency Smooth's picture, which came back in this window frame (the window waited
    /// for it in the `hd` stage when it was not done).
    Worker,
    /// By the Enhanced renderer of `--hd-scale`, when the 16:9 view has no picture.
    Fallback,
}

impl std::fmt::Display for DrawnOn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            DrawnOn::Window => "on the window's thread",
            DrawnOn::Worker => "in the background",
            DrawnOn::Fallback => "by the --hd-scale renderer",
        })
    }
}

/// What a window frame cost, for the log of a slow one.
#[derive(Debug, Clone, Copy, Default)]
struct FrameCost {
    /// From the window frame's start to its present's end (the pacer's wait for the next one left out).
    total: Duration,
    /// The stages of the frames it ran (zero when none ran).
    stages: Stages,
    /// The present: the interface and the swap.
    present: Duration,
    /// The frames of the game it asked for and ran.
    requested: u32,
    run: u32,
    /// What the Enhanced pictures that came back in it cost, and where the last was drawn.
    picture: Option<(ssd_hd::DrawCost, DrawnOn)>,
}

fn ms(time: Duration) -> f64 {
    time.as_secs_f64() * 1000.0
}

/// The log line of a window frame that took longer than [`SLOW_FRAME`]; `None` for one that did not.
fn slow_frame_line(cost: &FrameCost) -> Option<String> {
    use std::fmt::Write as _;
    if cost.total <= SLOW_FRAME {
        return None;
    }
    let stages = &cost.stages;
    // (the rest: the events, the interface's actions, a screenshot, a window moved or resized)
    let other = cost.total.saturating_sub(stages.frames() + cost.present);
    let mut line = format!(
        "slow window frame: {:.1} ms (emulate {:.1}, driver {:.1}, history {:.1}, hd {:.1}, present {:.1}, other \
         {:.1} ms; {}/{} frames)",
        ms(cost.total),
        ms(stages.emulate),
        ms(stages.driver),
        ms(stages.history),
        ms(stages.hd),
        ms(cost.present),
        ms(other),
        cost.requested,
        cost.run
    );
    match &cost.picture {
        Some((picture, on)) => {
            let _ = write!(
                line,
                "; picture drawn {on}: {:.1} ms (begin_bg {:.1}, find_poses {:.1}, draw {:.1}, art {:.1} ms; loads \
                 {:.1} ms: requests {}, received {}: {} bg, {} pictures, {} objects, {} variants, {} planes, {} bytes); \
                 {} bg slots{}, {} poses, bg art evicted {}",
                ms(picture.total),
                ms(picture.begin_bg),
                ms(picture.find_poses),
                ms(picture.draw),
                ms(picture.art_compose),
                ms(picture.loads),
                picture.requests,
                picture.received(),
                picture.bg_loads,
                picture.pictures,
                picture.objects,
                picture.variants,
                picture.planes,
                picture.load_bytes,
                picture.bg_slots,
                if picture.plane { " (one the Mode 7 plane)" } else { "" },
                picture.poses,
                picture.bg_evicted
            );
        }
        None => line.push_str("; no picture drawn"),
    }
    Some(line)
}

/// The slow window frames of a run: counted for the F1 pill, logged at most [`SLOW_LINES_A_SECOND`] a second, and
/// summed up every [`SLOW_REPORT_EVERY`].
#[derive(Debug)]
struct SlowFrames {
    /// Since the run started: the slow window frames and the slowest.
    count: u32,
    slowest: Duration,
    /// Since the last summary: when it was, the slow window frames, the slowest, and the background layers the art's
    /// cache let go to keep within its budget.
    since: Instant,
    recent: u32,
    recent_slowest: Duration,
    bg_evicted: u32,
    /// The second the lines logged are counted in, and the lines logged in it.
    second: Instant,
    lines: u32,
}

impl SlowFrames {
    fn new(now: Instant) -> Self {
        Self {
            count: 0,
            slowest: Duration::ZERO,
            since: now,
            recent: 0,
            recent_slowest: Duration::ZERO,
            bg_evicted: 0,
            second: now,
            lines: 0,
        }
    }

    /// Notes a window frame that ended at `now`; returns the line to log for a slow one, unless the second has its
    /// lines already.
    fn note(&mut self, cost: &FrameCost, now: Instant) -> Option<String> {
        if let Some((picture, _)) = &cost.picture {
            self.bg_evicted += picture.bg_evicted;
        }
        let line = slow_frame_line(cost)?;
        self.count += 1;
        self.slowest = self.slowest.max(cost.total);
        self.recent += 1;
        self.recent_slowest = self.recent_slowest.max(cost.total);
        if now.duration_since(self.second) >= Duration::from_secs(1) {
            self.second = now;
            self.lines = 0;
        }
        if self.lines >= SLOW_LINES_A_SECOND {
            return None;
        }
        self.lines += 1;
        Some(line)
    }

    /// The summary of the window frames since the last one, once [`SLOW_REPORT_EVERY`] has gone by at `now`.
    fn report(&mut self, now: Instant) -> Option<String> {
        let elapsed = now.duration_since(self.since);
        if elapsed < SLOW_REPORT_EVERY {
            return None;
        }
        let line = format!(
            "slow window frames: {} over {} ms in {:.1} s, the slowest {:.1} ms; the background art evicted {} layers",
            self.recent,
            SLOW_FRAME.as_millis(),
            elapsed.as_secs_f64(),
            ms(self.recent_slowest),
            self.bg_evicted
        );
        self.since = now;
        self.recent = 0;
        self.recent_slowest = Duration::ZERO;
        self.bg_evicted = 0;
        Some(line)
    }
}

impl WindowState {
    fn new(controls: Controls) -> Self {
        Self {
            controls,
            panel: PanelState::default(),
            watch: DivergenceWatch::default(),
            step_once: false,
            history: FrameHistory::default(),
            means: BTreeMap::new(),
            replay_over_logged: false,
            hd: None,
            view: None,
            scaled: None,
            hd_view: None,
            stages: Stages::default(),
            fast_forward: false,
            real_time: false,
            whole_frames: false,
            video: VideoSettings::default(),
            video_saved: VideoSettings::default(),
            video_apply: false,
            kept_states: BTreeMap::new(),
            kept_sessions: BTreeMap::new(),
            bindings: Bindings::default(),
            release_input: false,
            turbo: crate::turbo::Turbo::default(),
            touch: Touch::new(
                false,
                touch::settings::Settings::default(),
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(932.0, 430.0)),
            ),
            shot: None,
            continuous: false,
            ppu_stale: false,
            held_picture: false,
            compose_hold: 0,
            quit_confirm: None,
            ssj_watch: crate::voice::SsjWatch::default(),
            voice_due: false,
        }
    }

    /// The Video page changed to the PC file's `values`: what the player changed takes effect, and the picture is
    /// made at the new scale and look from the next frame.
    fn video_changed(&mut self, values: &Values) {
        let new = VideoSettings::from_values(values);
        self.video.follow(&self.video_saved, &new);
        self.video_saved = new;
        if self.scaled.is_some() {
            self.scaled = Some(self.video.hd());
        }
        self.video_apply = true;
    }
}

/// The HD layer of a run: built from the ROM only in Enhanced mode, so Classic does not read the
/// catalogue at all.
fn hd_layer(rom: &[u8], ruleset: Ruleset) -> anyhow::Result<Option<Hd>> {
    ruleset.is_enhanced().then(|| Hd::new(rom, ruleset)).transpose()
}

/// The session of a run with `ruleset`: a recording is made in its mode, and a replay recorded in the
/// enhanced mode needs it.
fn enhanced_session(session: &mut Session, ruleset: Ruleset) -> anyhow::Result<()> {
    if ruleset.is_enhanced() {
        session.set_mode(MODE_ENHANCED);
    }
    anyhow::ensure!(
        session.playback_mode() != MODE_ENHANCED || ruleset.is_enhanced(),
        "the replay was recorded in the enhanced mode: play it with --mode enhanced"
    );
    Ok(())
}

/// What the HD menu does for a frame: its joypad, and the host events made before the frame, which
/// the session applies. The menu runs on every frame, so its bag and the original's list stay in
/// step. A replay that plays gives its recorded joypad and events, and its recorded bind goes to the
/// bag and its recorded cheats to the run. A save that changed its bag or its cheats is kept in the sidecar
/// files, unless a replay plays (and a run that does not persist writes none: see [`hd::write_beside`]).
fn hd_frame(
    state: &mut WindowState,
    machine: &mut Machine,
    session: &Session,
    live: Joypad,
) -> (Joypad, Vec<HostEvent>) {
    let playing = session.is_playing();
    let Some(hd) = state.hd.as_mut() else { return (live, Vec::new()) };
    // The 16:9 view needs the PPU's render log of every frame, from its start (output only)
    machine.set_line_log(true);
    // The cheat wrappers belong to an Enhanced run's hook table, whatever table the run has now: a change of
    // the table (a hook mode, the start of a recording) takes them out, so they go back before the frame.
    ensure_cheats(machine);
    hd.note_cheats(machine.wram());
    // The save editor: it opens only when no replay plays or records, and a write it made goes to the machine's cartridge save
    // (flushed to `ssd.sav` at once) before its sidecar files, which `take_editor_commit` re-keyed, are written.
    hd.allow_editor(!playing && state.panel.replays.recording.is_none());
    hd.set_level_cap(ssd_game::mods::level_cap_in(|address| machine.peek(address).unwrap_or(0)));
    if let Some(commit) = hd.take_editor_commit() {
        match machine.write_sram(0, &commit.sram) {
            Ok(()) => save_sram(machine),
            Err(error) => {
                log::warn!("the save editor's write did not reach the cartridge save: {error}");
            }
        }
        for (what, result) in [
            ("bags", hd.save_sidecar()),
            ("cheats", hd.save_cheats_sidecar()),
            ("mod state", hd.save_mods_sidecar()),
            ("options", hd.save_options_sidecar()),
        ] {
            if let Err(error) = result {
                log::warn!("the save editor's {what} file was not written: {error:#}");
            }
        }
    }
    let frame = hd.frame(machine.wram(), machine.sram(), live);
    let (write_bags, write_cheats, write_mods) = sidecar_writes(&frame, playing);
    let settings = frame.settings.clone().filter(|_| !playing);
    let (pad, events) = match session.playback_at(machine.frame_count()) {
        Some((pad, events)) => {
            for event in &events {
                if let HostEvent::App { tag, payload } = event
                    && let Err(error) = hd.replay_event(*tag, payload, machine.wram())
                {
                    log::warn!("the replay's bag or cheats event failed: {error:#}");
                }
            }
            (pad, events)
        }
        None => (frame.pad, frame.events),
    };
    // The ending of the game (the event of `ssd_game::mods::chapter`): the `completed` file of the saves folder.
    for event in &events {
        if let HostEvent::App { tag, .. } = event
            && *tag == ssd_game::mods::chapter::COMPLETED_TAG
            && let Err(error) = crate::chapter::write_completed()
        {
            log::warn!("the completion of the game is not recorded: {error:#}");
        }
    }
    if write_bags && let Err(error) = hd.save_sidecar() {
        log::warn!("{error:#}");
    }
    if let Some(values) = settings.as_ref() {
        hd.follow_language(values);
        state.touch.apply_settings(touch::settings::Settings::from_values(values));
        state.touch.set_words(crate::lang::interface_text(values));
        let map = Bindings::from_values(values);
        if map != state.bindings {
            state.bindings = map;
            state.release_input = true;
        }
    }
    if let Some(settings) = settings
        && let Err(error) = crate::options::write_settings(&settings)
    {
        log::warn!("{error:#}");
    }
    if write_cheats && let Err(error) = hd.save_options_sidecar() {
        log::warn!("{error:#}");
    }
    if write_cheats && let Err(error) = hd.save_cheats_sidecar() {
        log::warn!("{error:#}");
    }
    if write_mods && let Err(error) = hd.save_mods_sidecar() {
        log::warn!("{error:#}");
    }
    if let Some(values) = frame.video.as_ref() {
        state.video_changed(values);
    }
    (pad, events)
}

/// The keys of the touch MENU's windows pressed since `previous`: the d-pad, A and B, as the HD menus read them.
fn touch_keys(previous: Joypad, now: Joypad) -> Vec<touch::PadKey> {
    hd::keys_pressed(previous, now)
        .into_iter()
        .filter_map(|key| match key {
            ssd_menus::Key::Up => Some(touch::PadKey::Up),
            ssd_menus::Key::Down => Some(touch::PadKey::Down),
            ssd_menus::Key::Left => Some(touch::PadKey::Left),
            ssd_menus::Key::Right => Some(touch::PadKey::Right),
            ssd_menus::Key::Confirm => Some(touch::PadKey::Confirm),
            ssd_menus::Key::Cancel => Some(touch::PadKey::Back),
            _ => None,
        })
        .collect()
}

/// A finger's place in SDL's normalised window coordinates, in egui points.
fn touch_point(video: &video::Video, x: f32, y: f32) -> egui::Pos2 {
    let screen = video.screen_points();
    egui::pos2(screen.min.x + x * screen.width(), screen.min.y + y * screen.height())
}

/// Writes the touch controls' choices and layout into the PC settings file (`ssd.settings`), keeping the other keys
/// and the copy the HD menus hold, so the Options screen does not write the old layout back.
fn persist_touch(state: &mut WindowState) {
    let mut values = match state.hd.as_mut() {
        Some(hd) => hd.pc_options_mut().clone(),
        None => crate::options::read_settings().unwrap_or_default(),
    };
    state.touch.settings.store(&mut values);
    if let Some(hd) = state.hd.as_mut() {
        *hd.pc_options_mut() = values.clone();
    }
    if let Err(error) = crate::options::write_settings(&values) {
        log::warn!("the touch layout was not saved: {error:#}");
    }
}

/// Forgets the language in the PC settings file and the HD menus' copy of it, so the next start asks for it again.
fn forget_language(state: &mut WindowState) {
    let mut values = match state.hd.as_mut() {
        Some(hd) => hd.pc_options_mut().clone(),
        None => crate::options::read_settings().unwrap_or_default(),
    };
    values.remove(ssd_menus::Language::KEY);
    if let Some(hd) = state.hd.as_mut() {
        *hd.pc_options_mut() = values.clone();
    }
    if let Err(error) = crate::options::write_settings(&values) {
        log::warn!("the language was not forgotten: {error:#}");
    }
}

/// Which sidecar files a frame writes, as (the bags, the cheats, the mod state): the files of the saves the frame
/// wrote, and none while a replay plays (the replay's own events carry the bookkeeping, and the saves' sidecars stay
/// as they are).
fn sidecar_writes(frame: &ssd_game::menu::driver::Frame, playing: bool) -> (bool, bool, bool) {
    (
        frame.saved.is_some() && !playing,
        frame.saved_cheats.is_some() && !playing,
        frame.saved_mods.is_some() && !playing,
    )
}

/// Puts the cheat wrappers and the pack redirectors into the hook table of `machine` when the table lacks
/// them (see [`hd_frame`]). Only the Enhanced run calls it: Classic gets neither.
fn ensure_cheats(machine: &mut Machine) {
    let table = machine.hook_table();
    if ssd_game::cheats::installed_in(table) && ssd_game::mods::installed_in(table) {
        return;
    }
    let mut table = machine.hook_table().cloned().unwrap_or_else(ssd_oracle::HookTable::new);
    ssd_game::cheats::install(&mut table);
    ssd_game::mods::install(&mut table);
    machine.set_hook_table(Some(table));
}

/// The HD layer of a frame: the menu on screen, else the field's overlay, with the textures they paint with.
fn frame_layer<'a>(
    scene: Option<&'a ssd_menus::DrawList>,
    overlay: Option<&'a ssd_menus::DrawList>,
    art: Option<&'a mut hd::Art>,
) -> Option<ui::Layer<'a>> {
    let (list, menu) = match (scene, overlay) {
        (Some(list), _) => (list, true),
        (None, Some(list)) => (list, false),
        (None, None) => return None,
    };
    art.map(|art| ui::Layer { list, art, menu })
}

/// The HD layer's lists with Esc's question over them while it asks: on the menu on screen, else on the field's
/// overlay (a new one when there is none). Its words are the HD menus' (translated, styled), English in Classic.
fn with_quit_confirm(
    state: &WindowState,
    scene: Option<ssd_menus::DrawList>,
    overlay: Option<ssd_menus::DrawList>,
) -> (Option<ssd_menus::DrawList>, Option<ssd_menus::DrawList>) {
    let Some(question) = state.quit_confirm.as_ref() else {
        return (scene, overlay);
    };
    let (width, height) = scene.as_ref().map_or(
        (ssd_menus::layout::WIDTH, ssd_menus::layout::HEIGHT),
        ssd_menus::DrawList::canvas_size,
    );
    let list = question.scene_sized(width, height);
    let list = match state.hd.as_ref() {
        Some(hd) => hd.dressed(list),
        None => list,
    };
    match scene {
        Some(mut scene) => {
            scene.append(list);
            (Some(scene), overlay)
        }
        None => {
            let mut overlay = overlay.unwrap_or_default();
            overlay.append(list);
            (None, Some(overlay))
        }
    }
}

/// Whether the machine runs frames this window frame: not during a verification (`verifying`), not while paused (but
/// for a step), and not while Esc's question is on screen.
fn game_runs(state: &WindowState, verifying: bool) -> bool {
    !verifying && state.quit_confirm.is_none() && (!state.controls.paused || state.step_once)
}

/// What the windowed loop works with besides the machine and the window.
struct Env<'a> {
    /// The ROM: the panel starts machines from it and checks replays with it.
    rom: &'a [u8],
    /// The symbols and the hook modes, if the symbols file could be read.
    switch: Option<&'a HookSwitch>,
    /// The verification run in progress, if one is.
    verify: Option<LockstepRun>,
}

impl Env<'_> {
    fn symbols(&self) -> Option<&Symbols> {
        self.switch.map(|switch| switch.symbols().as_ref())
    }

    fn divergence_dir(&self) -> PathBuf {
        self.switch.map_or_else(
            || PathBuf::from(DEFAULT_DIVERGENCE_DIR),
            |switch| switch.divergence_dir().to_path_buf(),
        )
    }
}

/// Sets what the controls and the hook switch allow from the session, the machine and the
/// verification run.
fn refresh_controls(state: &mut WindowState, session: &Session, machine: &Machine, env: &Env<'_>) {
    let recording = session.is_recording();
    let verifying = env.verify.is_some();
    // Loading waits while a replay records or plays, and comes back once the replay has ended.
    state.controls.load_allowed =
        !recording && (!session.is_playing() || session.playback_over(machine));
    state.controls.verifying = verifying;
    state.controls.can_step = !recording && !verifying;
    state.controls.freezes_allowed = !session.is_active();
    state.controls.hooks_switch = if recording {
        HooksSwitch::Locked("A recording needs the original code: hooks cannot change.")
    } else if env.switch.is_none() {
        HooksSwitch::Locked("The symbols file could not be read: hooks cannot change.")
    } else {
        HooksSwitch::Allowed
    };
}

/// Carries out what an action asks for: the controls and the panel change in `state`, the
/// machine (state files, hook mode, steps, breakpoints, replays) here.
#[allow(clippy::too_many_arguments)]
fn apply_action(
    action: PanelAction,
    state: &mut WindowState,
    machine: &mut Machine,
    session: &mut Session,
    env: &mut Env<'_>,
    live: Joypad,
    audio: &mut dyn AudioSink,
) {
    match action {
        PanelAction::Debug(action) => {
            debug_action(action, state, machine, session, env, live, audio);
        }
        PanelAction::Replay(action) => replay_action(action, state, machine, session, env),
        _ => apply_controls(action, state, machine, env),
    }
}

/// The actions the emulation controls and the hotkeys make.
fn apply_controls(
    action: PanelAction,
    state: &mut WindowState,
    machine: &mut Machine,
    env: &Env<'_>,
) {
    let was_paused = state.controls.paused;
    match state.controls.apply(action, &mut state.panel) {
        None if action == PanelAction::LoadState => {
            log::warn!("loading a state is off while recording, and while a replay plays");
        }
        None => {}
        Some(Effect::StepFrame) => state.step_once = true,
        Some(Effect::BattleCheats) => {
            let opened = ssd_game::gameplay::in_battle(machine.wram())
                && state.hd.as_mut().is_some_and(crate::hd::Hd::open_battle_cheats);
            if !opened {
                log::info!("the Cheats open over a battle in Enhanced, while the game is in one");
            }
        }
        Some(Effect::Screenshot(kind)) => {
            // A screenshot is not a save, but only a window has a picture to take: a headless, snapshot or
            // replay run (which does not persist) writes nothing.
            if paths::persists() {
                state.shot = Some(kind);
            } else {
                log::info!("a run that does not persist takes no screenshot");
            }
        }
        Some(Effect::Save(slot)) => {
            // The HD menus' part of the state (the bag, the cheats), which the machine's state does not hold.
            let session = state.hd.as_ref().and_then(Hd::session_bytes);
            if paths::persists() {
                match save_state(machine, slot) {
                    Ok(path) => {
                        save_session(&path, session.as_deref());
                        log::info!("state saved to {}", path.display());
                    }
                    Err(e) => log::warn!("saving state failed: {e:#}"),
                }
            } else {
                // A run that does not persist keeps the state in memory: no state file is written.
                state.kept_states.insert(slot, machine.save_state());
                match session {
                    Some(session) => state.kept_sessions.insert(slot, session),
                    None => state.kept_sessions.remove(&slot),
                };
                log::info!("state of slot {slot} kept in memory: this run writes no state file");
            }
        }
        Some(Effect::Load(slot)) => {
            // A state this run kept is loaded from memory; any other is read from the states folder.
            let (loaded, session) = match state.kept_states.get(&slot) {
                Some(bytes) => (
                    machine.load_state(bytes).map_err(anyhow::Error::from),
                    state.kept_sessions.get(&slot).cloned(),
                ),
                None => (load_state(machine, slot), read_session(&hotkeys::state_path(slot))),
            };
            match loaded {
                Ok(()) => {
                    // The stop banner described the position the load has just left.
                    state.panel.debug.clear_banner();
                    // The HD menu and the bag described the game that was there: the state's own come back with it.
                    if let Some(hd) = state.hd.as_mut() {
                        hd.reset();
                        if let Some(session) = &session
                            && let Err(e) = hd.restore_session(session, machine)
                        {
                            log::warn!(
                                "the bag and the cheats of the state were not restored: {e:#}"
                            );
                        }
                    }
                    log::info!("state loaded from slot {slot}");
                }
                Err(e) => log::warn!("loading state failed: {e:#}"),
            }
        }
        Some(Effect::SetHooks(mode)) => {
            let Some(switch) = env.switch else { return };
            keep_means(state, machine);
            match switch.apply(machine, mode) {
                Ok(()) => {
                    after_table_change(state, machine);
                    log::info!("hooks: {}", mode.label());
                    state.controls.set_hooks(mode);
                }
                Err(e) => log::warn!("changing the hook mode failed: {e:#}"),
            }
        }
        Some(Effect::SetRoutineMode { key, mode }) => {
            let Some(switch) = env.switch else { return };
            let rows = tab_routines::rows_of(machine, Some(switch.symbols().as_ref()));
            let Some(modes) = tab_routines::mode_change(&rows, key, mode) else { return };
            let name = rows.iter().find(|row| row.key == key).map_or("?", |row| row.name.as_str());
            keep_means(state, machine);
            match switch.apply_modes(machine, &modes) {
                Ok(()) => {
                    after_table_change(state, machine);
                    log::info!("routine {name}: {}", HooksArg::of(mode).label());
                    match tab_routines::uniform_mode(&modes) {
                        Some(all) => state.controls.set_hooks(all),
                        None => state.controls.set_mixed(),
                    }
                }
                Err(e) => log::warn!("changing the mode of {name} failed: {e:#}"),
            }
        }
        Some(Effect::FreezeSelected) => {
            let symbols = env.symbols();
            let memory = &mut state.panel.memory;
            match tab_memory::new_freeze(machine, memory.selection, symbols) {
                Ok(freeze) => {
                    let mut all = memory.active_freezes();
                    all.push(freeze.freeze.clone());
                    match machine.set_freezes(all) {
                        Ok(()) => {
                            log::info!("frozen: {}", freeze.name);
                            memory.add_freeze(freeze);
                        }
                        Err(e) => memory.message = Some(e.to_string()),
                    }
                }
                Err(message) => memory.message = Some(message),
            }
        }
        Some(Effect::ApplyFreezes) => {
            let memory = &mut state.panel.memory;
            if let Err(e) = machine.set_freezes(memory.active_freezes()) {
                memory.message = Some(e.to_string());
            }
        }
    }
    if was_paused && !state.controls.paused {
        state.panel.debug.clear_banner();
    }
}

/// Whether a routine of the machine is checked against the original, so that a divergence is to
/// be reported.
fn any_verify(machine: &Machine) -> bool {
    machine.hook_table().is_some_and(|table| table.entries().any(|e| e.mode == HookMode::Verify))
}

/// The mode of every routine in the machine's hook table, by name (none without a table).
fn routine_modes(machine: &Machine) -> Vec<(String, HookMode)> {
    machine.hook_table().map_or_else(Vec::new, |table| {
        table.entries().map(|entry| (entry.name.clone(), entry.mode)).collect()
    })
}

/// Keeps the mean instructions `machine` has measured, before its table or the machine itself is
/// replaced: a table change drops the counters of the routines it does not keep, and a new machine
/// has none. [`after_table_change`] gives them to the one that replaces it.
fn keep_means(state: &mut WindowState, machine: &Machine) {
    keep_measured(&mut state.means, &machine.hook_stats());
}

/// Records the routines of `stats` that have a measured mean ([`HookStats::mean_instructions`]).
fn keep_measured(means: &mut BTreeMap<String, f64>, stats: &[(String, HookStats)]) {
    for (name, counters) in stats {
        if counters.mean_instructions > 0.0 {
            means.insert(name.clone(), counters.mean_instructions);
        }
    }
}

/// What a table or machine change re-bases once it is in place: the kept means go back to the
/// machine; the window's idle RNG share restarts, as the machine's own per-frame figures do; and
/// the divergence watch takes the counts it finds for its baseline, so only later divergences are
/// news.
fn after_table_change(state: &mut WindowState, machine: &mut Machine) {
    let means: Vec<(String, f64)> =
        state.means.iter().map(|(name, mean)| (name.clone(), *mean)).collect();
    machine.set_mean_instructions(&means);
    state.history.rebase(tab_routines::idle_rng_total(machine));
    state.watch.reseed(&machine.hook_stats());
}

/// Saves the SRAM of `machine` before it is replaced: the machine that takes its place may load the
/// same save file.
fn save_sram(machine: &mut Machine) {
    if let Err(error) = machine.flush_saves() {
        log::warn!("saving SRAM failed: {error}");
    }
}

/// What the Routines tab keeps of a frame the machine rendered: its statistics and the idle RNG's
/// share of its Rust instructions (see [`FrameHistory::record`]).
fn note_render(history: &mut FrameHistory, machine: &Machine) {
    history.record(machine.last_frame_stats(), tab_routines::idle_rng_total(machine));
}

/// The WRAM the Memory tab marks changed bytes against, while the tab is shown; hidden, the
/// baseline is dropped instead.
fn note_wram(state: &mut WindowState, machine: &Machine) {
    let shown = state.panel.open && state.panel.tab == Tab::Memory;
    if shown {
        state.panel.memory.observe(machine.frame_count(), machine.wram());
    } else {
        state.panel.memory.forget();
    }
}

/// What the snapshot paths note after each frame they run: the statistics, the WRAM and the view.
fn observe(state: &mut WindowState, machine: &Machine) {
    note_render(&mut state.history, machine);
    note_wram(state, machine);
    note_view(state, machine, Draw::Now);
}

/// What the 16:9 view does with the frame that just ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Draw {
    /// Draw it: it is the one the window shows.
    Now,
    /// Only follow its work RAM: another frame follows before the window draws.
    Skip,
    /// Draw the frame skipped last, after all: the run stopped on it.
    Again,
    /// Draw the frame skipped last in the background, and show the picture drawn the time before:
    /// a fast-forward shows its pictures a window frame late, so its drawing does not hold up the
    /// frames that follow.
    Later,
    /// As [`Draw::Later`] for a run at its own speed (Latency Smooth): the picture of the frame before is waited for
    /// when it is not done, so no frame is dropped, and the drawing of this one overlaps the next window frame's.
    Next,
}

/// The 16:9 view of the frame that just ran, in Enhanced mode: the picture the window shows, and
/// the backdrop of the HD menus. A frame that is [`Draw::Skip`]ped costs the state's upkeep only:
/// the picture of a fast-forward's batch is that of its last frame.
fn note_view(state: &mut WindowState, machine: &Machine, draw: Draw) {
    let Some(hd) = state.hd.as_mut() else { return };
    hd.prepare_view(machine.wram(), state.video.narrow);
    // The transformation's lightning, by the watch of its voice: none for a frame the watch did not note (a state loaded)
    hd.set_ssj_lightning(state.ssj_watch.lightning_at(machine.frame_count()));
    if draw == Draw::Skip {
        hd.skip_frame(machine.wram());
        return;
    }
    if let Some(options) = state.scaled {
        // The Enhanced picture in the 16:9 view; a frame it refuses is drawn as the 256-wide one
        let view = match draw {
            Draw::Again => hd.redraw_frame_hd_shared(machine.frame_log(), options),
            Draw::Later => hd
                .redraw_frame_hd_pipelined(machine.frame_log(), options)
                .map(ssd_hd::world::SharedHd::new),
            Draw::Next => hd
                .redraw_frame_hd_next(machine.frame_log(), options)
                .map(ssd_hd::world::SharedHd::new),
            _ => hd.note_frame_hd_shared(machine.wram(), machine.frame_log(), options),
        };
        if matches!(draw, Draw::Again | Draw::Later | Draw::Next) && view.is_none() {
            if matches!(draw, Draw::Later | Draw::Next) || state.held_picture {
                // No picture came back yet (or the frame is refused and the PPU did not draw it): the window keeps
                // showing the last one
                if let Some((frame, _)) = state.hd_view.as_mut() {
                    *frame = machine.frame_count();
                }
            }
            return;
        }
        state.hd_view = view.map(|view| (machine.frame_count(), view));
        match current_hd_view(state.hd_view.as_ref(), machine) {
            Some(view) => hd.note_field_picture(view),
            None => hd.note_field(&machine.frame()),
        }
        return;
    }
    let view = match draw {
        Draw::Again | Draw::Later | Draw::Next => hd.redraw_frame(machine.frame_log()),
        _ => hd.note_frame(machine.wram(), machine.frame_log()),
    };
    if matches!(draw, Draw::Again | Draw::Later | Draw::Next) && view.is_none() {
        return;
    }
    state.view = view.map(|view| (machine.frame_count(), view));
    let view = current_view(state.view.as_ref(), machine);
    hd.note_field(&game_frame(view, machine));
}

/// The Enhanced 16:9 view of the machine's frame, if it was made at the frame the machine is at.
fn current_hd_view<'a>(
    view: Option<&'a (u64, ssd_hd::world::SharedHd)>,
    machine: &Machine,
) -> Option<&'a ssd_hd::world::SharedHd> {
    view.filter(|(frame, _)| *frame == machine.frame_count()).map(|(_, view)| view)
}

/// The 16:9 view of the machine's frame, if the view was made at the frame the machine is at.
fn current_view<'a>(
    view: Option<&'a (u64, WideFrame)>,
    machine: &Machine,
) -> Option<&'a WideFrame> {
    view.filter(|(frame, _)| *frame == machine.frame_count()).map(|(_, view)| view)
}

/// The picture of the game the window shows: the 16:9 view of the frame when there is one, else
/// the PPU's own frame.
fn game_frame<'a>(view: Option<&'a WideFrame>, machine: &'a Machine) -> Frame<'a> {
    match view {
        Some(view) => Frame {
            pixels: &view.frame.pixels,
            size: FrameSize { width: view.frame.width as u32, height: view.frame.height as u32 },
            options: machine.frame().options,
        },
        None => machine.frame(),
    }
}

/// What the header shows of the machine: its frame, the rate, the share of the instructions of the
/// last 60 frames that ran as Rust (none while the means are missing), and its freezes.
fn header_status(machine: &Machine, history: &FrameHistory, fps: f32) -> Status {
    let means_missing = tab_routines::means_missing(machine);
    Status {
        frame: machine.frame_count(),
        fps,
        rust_percent: tab_routines::window_summary(history, means_missing)
            .map(|figures| figures.total),
        means_missing,
        freezes: machine.freezes().len(),
        live: None,
    }
}

// --- the debugger ----------------------------------------------------------------------------

fn debug_action(
    action: DebugAction,
    state: &mut WindowState,
    machine: &mut Machine,
    session: &mut Session,
    env: &Env<'_>,
    live: Joypad,
    audio: &mut dyn AudioSink,
) {
    state.panel.debug.notice = None;
    match action {
        DebugAction::ToggleStopAtInterrupts => {
            let debug = &mut state.panel.debug;
            debug.stop_at_interrupts = !debug.stop_at_interrupts;
        }
        DebugAction::AddBreakpoint => add_breakpoint(state, machine, session, env),
        DebugAction::ToggleBreakpoint(index) => {
            state.panel.debug.toggle(index);
            sync_breakpoints(machine, &state.panel.debug, session.is_recording());
        }
        DebugAction::RemoveBreakpoint(index) => {
            state.panel.debug.remove(index);
            sync_breakpoints(machine, &state.panel.debug, session.is_recording());
        }
        DebugAction::Step(kind) => step(kind, state, machine, session, env, live, audio),
    }
}

fn add_breakpoint(
    state: &mut WindowState,
    machine: &mut Machine,
    session: &Session,
    env: &Env<'_>,
) {
    if session.is_recording() {
        let note = "Breakpoints wait for the recording to end: a recording runs without hooks.";
        state.panel.debug.notice = Some(note.to_owned());
        return;
    }
    let empty = Symbols::default();
    let symbols = env.symbols().unwrap_or(&empty);
    let debug = &mut state.panel.debug;
    let result = debugger::parse_target(&debug.target, symbols)
        .and_then(|key| debug.add(key, debugger::name_of(symbols, key)));
    match result {
        Ok(()) => debug.target.clear(),
        Err(notice) => debug.notice = Some(notice),
    }
    sync_breakpoints(machine, &state.panel.debug, false);
}

/// Sets the enabled breakpoints on the machine and clears the rest. While recording there are
/// none: a recording runs without hooks, and a breakpoint is one.
fn sync_breakpoints(machine: &mut Machine, debug: &debugger::DebugState, recording: bool) {
    let wanted = if recording { Vec::new() } else { debug.enabled_keys() };
    let set: Vec<u32> = machine.breakpoints().iter().copied().collect();
    for key in set {
        if !wanted.contains(&key) {
            machine.clear_breakpoint(key);
        }
    }
    for key in wanted {
        machine.set_breakpoint(key);
    }
}

/// One step of the debugger, while paused: into, over or out, with the machine's inputs of the
/// replay (or the live joypad).
fn step(
    kind: StepKind,
    state: &mut WindowState,
    machine: &mut Machine,
    session: &Session,
    env: &Env<'_>,
    live: Joypad,
    audio: &mut dyn AudioSink,
) {
    if !(state.controls.paused && state.controls.can_step) {
        return;
    }
    let options = StepOptions {
        stop_at_interrupts: state.panel.debug.stop_at_interrupts,
        ..StepOptions::default()
    };
    let mut input = |frame: u64| session.input_at(frame, live);
    let result = match kind {
        StepKind::Into => machine.step_into(&mut input, audio),
        StepKind::Over => machine.step_over(&options, &mut input, audio),
        StepKind::Out => machine.step_out(&options, &mut input, audio),
    };
    match result {
        Ok(stepped) => {
            let empty = Symbols::default();
            let symbols = env.symbols().unwrap_or(&empty);
            state.panel.debug.stepped(&stepped, |key| debugger::name_of(symbols, key));
            state.controls.paused = true;
        }
        Err(error) => {
            log::warn!("the step failed: {error}");
            state.panel.debug.notice = Some(format!("The step failed: {error}"));
        }
    }
}

/// A breakpoint stopped the run at `key`: the banner names it, and the machine is paused there.
fn breakpoint_stop(
    state: &mut WindowState,
    machine: &Machine,
    symbols: Option<&Symbols>,
    key: u32,
) {
    let empty = Symbols::default();
    let symbols = symbols.unwrap_or(&empty);
    let name = debugger::name_of(symbols, key);
    // Only a routine the machine dispatches has a mode: the banner says what its step does.
    let mode = machine
        .hook_table()
        .and_then(|table| table.get(key))
        .filter(|entry| matches!(entry.action, HookAction::Routine { .. }))
        .map(|entry| entry.mode);
    state.panel.debug.stopped_at_breakpoint(key, &name, mode);
    state.controls.paused = true;
    state.step_once = false;
}

/// Runs `frames` frames, the ones of one window frame (at least one; a next frame is one), and
/// hands the audio of the first to `audio`. A breakpoint ends them early, and pauses the machine.
/// Each frame the machine renders is noted for the Routines tab. None is drawn in the 16:9 view
/// (only the one the window shows is: see [`draw_shown`]). Returns the frames run (a frame a
/// breakpoint stopped is not one).
fn run_frames(
    state: &mut WindowState,
    machine: &mut Machine,
    session: &mut Session,
    symbols: Option<&Symbols>,
    live: Joypad,
    audio: &mut dyn AudioSink,
    frames: u32,
) -> anyhow::Result<u32> {
    let stops = Stops { frame_end: true, breakpoint: true, ..Stops::default() };
    let frames = if state.step_once { 1 } else { frames.max(1) };
    let mut null = NullAudio;
    let mut ran = 0;
    // Only the last frame of a batch is shown, and only the first one's sound is heard: the
    // others run without drawing their pixels, logging their render or resampling their sound.
    // A breakpoint stops a frame wherever it is, and the HD menu's frames behind its curtain are
    // few: both run every frame whole. At speed 1 the batch is the frames the clock owes (a phone
    // whose screen fell behind): each one's sound is heard, or the sound would starve.
    let every_sound = state.real_time;
    // The battle speed's frames are the game's own at 1.5 times, not a fast-forward: every one is run whole
    let fast_forward = frames > 1
        && !state.whole_frames
        && !state.step_once
        && machine.breakpoints().is_empty()
        && !state.hd.as_ref().is_some_and(hd::Hd::busy);
    state.fast_forward = fast_forward;
    let logging = state.hd.is_some() || machine.line_log_enabled();
    // Whether the PPU may leave its pixels undrawn: the picture is drawn from the render log (the window frame starts
    // where its last picture left off, and nothing needs the pixels)
    let skip_pixels = state.continuous && may_skip_pixels(state, machine, session);
    for n in 0..frames {
        let started = Instant::now();
        // The turbo of the Controls page pulses a held A (and B) one machine frame at a time, here where the frame's
        // joypad is made, so a recording holds what the game saw. Only the Enhanced run has the page (Classic has no
        // HD layer and its joypad goes through untouched); the HD menus' keys are made from the raw joypad elsewhere.
        let live = match state.hd.as_ref() {
            Some(hd) => {
                let settings = crate::turbo::Settings::from_values(&hd.driver().options().pc);
                state.turbo.step(settings, live)
            }
            None => live,
        };
        // While the HD menu has the curtain, its presses are the frame's joypad (the game gets no
        // other buttons); otherwise the live joypad goes through.
        let (pad, events) = hd_frame(state, machine, session, live);
        // The last frame of the window frame is the one that is shown. The PPU draws its pixels for it unless the
        // picture is drawn from the log: then none is drawn, in a fast-forward's frames either
        let shown = n + 1 == frames;
        let compose = !skip_pixels && (shown || !fast_forward);
        machine.set_compose_pixels(compose);
        if fast_forward {
            machine.set_line_log(logging && shown);
            machine.set_audio_resampling(n == 0 || every_sound);
        }
        let driven = Instant::now();
        let stopped = if n == 0 || every_sound {
            session.run(machine, pad, &events, audio, &stops)?
        } else {
            session.run(machine, pad, &events, &mut null, &stops)?
        };
        let ran_at = Instant::now();
        state.stages.driver += driven - started;
        state.stages.emulate += ran_at - driven;
        // Goku's first Super Saiyan transformation: every frame is looked at (a fast-forward's too), Classic and Enhanced
        if state.ssj_watch.observe(machine.frame_count(), crate::voice::goku(machine.wram())) {
            state.voice_due = true;
        }
        if let Stopped::Breakpoint { key } = stopped {
            breakpoint_stop(state, machine, symbols, key);
            break;
        }
        ran += 1;
        if shown {
            note_pixels(state, machine, compose);
        }
        note_render(&mut state.history, machine);
        let noted = Instant::now();
        state.stages.history += noted - ran_at;
        // The picture is drawn from the last frame, which starts from the state the one before
        // ended in; every frame of the batch is followed all the same, as the translation keeps a
        // page the message engine names for two frames only (the Item help)
        // A battle speed's frame not shown is not drawn either: run whole and followed, the frame shown after it is
        // speed 1's picture of that frame (the translation's pages, the card tooltip), and drawing it as well made a
        // window frame of two cost two pictures, more than a slow machine draws in a 60 Hz refresh
        note_view(state, machine, Draw::Skip);
        state.stages.hd += noted.elapsed();
    }
    // Back to the whole of every frame, for the other runs (a state load, a single step)
    machine.set_compose_pixels(true);
    if fast_forward {
        machine.set_audio_resampling(true);
    }
    Ok(ran)
}

/// The frames the PPU draws its pixels for after one the Enhanced picture refused (a mode of the screen it does not
/// draw): the pixels are the picture then, and a screen like that lasts.
const COMPOSE_HOLD: u32 = 120;

/// Whether the frames about to run may leave the PPU's pixels undrawn (`Machine::set_compose_pixels`). In Enhanced
/// mode the picture of every frame is drawn again from the render log, which the parity check shows equal to the PPU's,
/// and nothing else reads the pixels. The PPU draws them all the same
///
/// * in Classic, and where the picture is not the Enhanced one at a scale (the 4:3 and 16:9 views without it);
/// * while the log is off, or a frame the picture cannot be drawn from was seen lately (`compose_hold`);
/// * while a recording runs, a routine is verified against the original, a breakpoint is set, or a single frame is
///   stepped, and while a screenshot of the PPU's own frame is waiting: those read the frame as the PPU drew it;
/// * when the window frame does not start where the picture on show left off (a state loaded, a reset, a new machine,
///   the first frame: `WindowState::continuous`), which the caller checks.
///
/// The replays' verification (xtask) never leaves them undrawn: it runs the machine whole.
fn may_skip_pixels(state: &WindowState, machine: &Machine, session: &Session) -> bool {
    state.hd.is_some()
        && state.scaled.is_some()
        && state.compose_hold == 0
        && !state.step_once
        && state.shot != Some(screenshot::Kind::Frame)
        && machine.line_log_enabled()
        && machine.breakpoints().is_empty()
        && !session.is_recording()
        && !any_verify(machine)
}

/// Whether the Enhanced picture can draw `log`: a complete frame in the modes of the game (0, 1 and 7), not hi-res or
/// interlaced. A frame that fails this is refused by the renderer (`ssd_hd::Unsupported`).
fn drawable(log: Option<&ssd_oracle::FrameLog>) -> bool {
    let Some(log) = log else { return false };
    log.complete
        && !log.h_hi_res
        && !log.v_hi_res
        && log.entries.iter().all(|entry| {
            let regs = &entry.regs;
            !(regs.interlaced || regs.pseudo_h_hi_res || regs.pseudo_obj_hi_res)
                && matches!(regs.bg_mode, 0 | 1 | 7)
        })
}

/// Notes what the frame shown left in the PPU's buffer: whether it is the frame (`composed`), and, when it is not and
/// the Enhanced picture cannot draw the frame, that the window keeps its last picture for this one frame and that the
/// PPU draws its pixels from now on for a while.
fn note_pixels(state: &mut WindowState, machine: &Machine, composed: bool) {
    state.ppu_stale = !composed;
    let drawable = drawable(machine.frame_log());
    state.held_picture = !composed && !drawable;
    state.compose_hold = if drawable { state.compose_hold.saturating_sub(1) } else { COMPOSE_HOLD };
}

/// Draws the 16:9 view of the last frame run to its end: the picture the window shows for the
/// frames it just ran ([`run_frames`] draws none).
fn draw_shown(state: &mut WindowState, machine: &Machine) {
    let drawn = Instant::now();
    let draw = if state.fast_forward {
        Draw::Later
    } else if drawn_next(state, machine) {
        Draw::Next
    } else {
        Draw::Again
    };
    note_view(state, machine, draw);
    state.stages.hd += drawn.elapsed();
}

/// Whether the picture of the frame just run is drawn while the next window frame runs and shown then (Latency
/// Smooth). Not when it must be shown now: the picture on show was not that of the frame before (the first frame, a
/// load, a reset), a single frame is stepped, a screenshot is waiting (the picture and the state beside it are one
/// frame), a breakpoint is set, or a menu has the field (its curtain, a press under way): those draw the frame and
/// show it at once.
fn drawn_next(state: &WindowState, machine: &Machine) -> bool {
    state.video.smooth
        && state.scaled.is_some()
        && state.continuous
        && !state.step_once
        && state.shot.is_none()
        && machine.breakpoints().is_empty()
        && state.hd.as_ref().is_some_and(Hd::field_free)
}

/// A window frame that runs no frame (paused) shows the picture of the last frame run, which Latency Smooth may still be
/// drawing: waits for it. A picture of a machine that moved since (a state loaded) is dropped.
fn settle_draw(state: &mut WindowState, machine: &Machine) {
    let Some(hd) = state.hd.as_mut().filter(|hd| hd.drawing()) else { return };
    let picture = hd.finish_drawing();
    if let Some(picture) = picture
        && current_hd_view(state.hd_view.as_ref(), machine).is_some()
    {
        let shared = ssd_hd::world::SharedHd::new(picture);
        hd.note_field_picture(&shared);
        state.hd_view = Some((machine.frame_count(), shared));
    }
}

/// Runs the frames of one window frame while the machine runs: `frames` of them, or as many as the
/// HD menu's choice takes behind its curtain, and hands the audio of the first to the device. The
/// 16:9 view is drawn for the last one only. The frames run are returned.
fn window_frames(
    state: &mut WindowState,
    machine: &mut Machine,
    session: &mut Session,
    env: &Env<'_>,
    pad: Joypad,
    audio: &mut audio::Audio,
    frames: u32,
) -> anyhow::Result<u32> {
    if !state.replay_over_logged && session.playback_over(machine) {
        log::info!("the replay is over: all buttons released");
        state.replay_over_logged = true;
    }
    let symbols = env.symbols();
    // The Audio page's volume and output, as the menu holds them now: they shape the samples sent to the device only.
    if let Some(hd) = state.hd.as_ref() {
        audio.set_settings(AudioSettings::from_values(&hd.driver().options().pc));
    }
    // A choice of the HD menu runs its presses behind the curtain: as many frames as it
    // takes, in one go, without waiting for the clock or drawing them.
    let burst = if state.hd.as_ref().is_some_and(hd::Hd::busy) { HD_BURST } else { 1 };
    let mut frames_run = 0_u32;
    // The picture on show is that of the frame the machine is at (nothing moved it since): only then are the frames
    // that follow drawn from where it left off
    let started = current_hd_view(state.hd_view.as_ref(), machine).is_some();
    state.continuous = started;
    let mut rounds = 0_u32;
    for round in 0..burst {
        if round > 0 && !state.hd.as_ref().is_some_and(hd::Hd::busy) {
            break;
        }
        frames_run += run_frames(state, machine, session, symbols, pad, audio, frames)?;
        // (the frames of a menu's press, behind its curtain, run from where the first left off)
        state.continuous = false;
        rounds += 1;
    }
    state.continuous = started && rounds == 1;
    if std::mem::take(&mut state.voice_due) {
        audio.play_voice();
    }
    send_then_draw(|| audio.end_frame(), || draw_shown(state, machine));
    state.step_once = false;
    Ok(frames_run)
}

/// Ends a window frame: the sound of its frames goes to the device (`send`), then its picture is drawn (`draw`).
/// Drawing may wait for the picture of the window frame before (Latency Smooth); the sound does not wait with it, or
/// the device's queue runs low while it does.
fn send_then_draw(send: impl FnOnce(), draw: impl FnOnce()) {
    send();
    draw();
}

/// The Enhanced picture of the frame just run (`--hd-scale`) when the 16:9 view of the frame is
/// missing, unless a verification shows its two Classic ones.
fn fallback_picture(
    state: &WindowState,
    machine: &mut Machine,
    enhanced: Option<&mut Enhanced>,
    verifying: bool,
) -> Option<HdFrame> {
    let hd_view = current_hd_view(state.hd_view.as_ref(), machine);
    match enhanced {
        Some(enhanced) if !verifying && hd_view.is_none() => {
            enhanced.picture(machine, state.ssj_watch.lightning_at(machine.frame_count()))
        }
        _ => None,
    }
}

/// The pictures the window shows: the Enhanced picture of the frame just run (`--hd-scale`, from
/// the 16:9 view or `picture`), unless a verification shows its two Classic ones; else the 16:9
/// view of Enhanced mode, else the PPU's own frame.
fn game_views<'a>(
    hd_view: Option<&'a (u64, ssd_hd::world::SharedHd)>,
    view: Option<&'a (u64, WideFrame)>,
    machine: &'a Machine,
    picture: Option<&'a HdFrame>,
    verify: Option<&'a LockstepRun>,
    smooth: bool,
) -> Vec<GameView<'a>> {
    // The 16:9 picture is named by its serial: a picture the window showed last is not uploaded again
    let shared = current_hd_view(hd_view, machine).filter(|_| verify.is_none());
    let shown = match shared {
        Some(shared) => Some((&shared.picture.picture, Some(shared.serial))),
        None => picture.map(|picture| (picture, None)),
    };
    match (shown, verify) {
        (Some((picture, serial)), _) => {
            let view = GameView::enhanced(picture, &machine.frame(), smooth);
            vec![match serial {
                Some(serial) => view.with_serial(serial),
                None => view,
            }]
        }
        (None, Some(run)) => vec![
            GameView::classic(&run.machine_a().frame()),
            GameView::classic(&run.machine_b().frame()),
        ],
        (None, None) => {
            let wide = game_frame(current_view(view, machine), machine);
            vec![GameView::classic(&wide)]
        }
    }
}

/// The pictures of the window with the last pass of the Video page: the 4:3 crop, the scanlines and the CRT filter
/// over the one picture of the game (a verification's two pictures are shown as they are). With nothing on, the
/// pictures are the renderer's own.
fn shape_views<'a>(
    views: Vec<GameView<'a>>,
    shape: filter::Shape,
    buffer: &'a mut Vec<u8>,
) -> Vec<GameView<'a>> {
    match <[GameView<'a>; 1]>::try_from(views) {
        Ok([one]) => vec![filter::shape(one, shape, buffer)],
        Err(views) => views,
    }
}

// --- replays ---------------------------------------------------------------------------------

/// The name of a file without its extension, for the lists and the notices.
fn stem(path: &Path) -> String {
    path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned())
}

/// A new recording's file: `recording-<seconds since 1970>.ssdreplay` in the replays directory.
fn recording_path() -> PathBuf {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    PathBuf::from(REPLAYS_DIR).join(format!("recording-{secs}.ssdreplay"))
}

/// Rescans the replay files and the divergence reports.
fn refresh_replays(state: &mut WindowState, env: &Env<'_>) {
    let crc = ssd_rom::crc32(env.rom);
    state.panel.replays.refresh(crc, &env.divergence_dir());
}

/// Puts `next` in place of `session`, finishing the old one (its recording is written).
fn replace_session(session: &mut Session, next: Session) {
    let old = std::mem::take(session);
    if let Err(error) = old.finish(Ok(())) {
        log::warn!("{error:#}");
    }
    *session = next;
}

/// A recording's session and its machine at power-on, without hooks.
fn start_recording(path: &Path, rom: &[u8]) -> anyhow::Result<(Session, Machine)> {
    let mut next = Session::new(None, Some(path), rom)?;
    let machine = fresh_machine(rom, &mut next, HooksArg::Original, None)?;
    Ok((next, machine))
}

/// A replay's session and its machine at power-on, with the hook mode `mode` installed.
fn start_playback(
    path: &Path,
    rom: &[u8],
    mode: HooksArg,
    switch: Option<&HookSwitch>,
) -> anyhow::Result<(Session, Machine)> {
    let mut next = Session::new(Some(path), None, rom)?;
    let machine = fresh_machine(rom, &mut next, mode, switch)?;
    Ok((next, machine))
}

/// A verification run of a replay, and the replay's frames.
fn start_verification(
    path: &Path,
    rom: &[u8],
    switch: &HookSwitch,
) -> anyhow::Result<(LockstepRun, u64)> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let replay = Replay::from_bytes(&bytes, ssd_rom::crc32(rom), CORE_VERSION)?;
    let frames = replay.frames();
    let config = switch.lockstep_config()?;
    let run =
        LockstepRun::new(rom.to_vec(), Arc::new(replay), &config, Some(switch.symbols().as_ref()))?;
    Ok((run, frames))
}

fn replay_action(
    action: ReplayAction,
    state: &mut WindowState,
    machine: &mut Machine,
    session: &mut Session,
    env: &mut Env<'_>,
) {
    state.panel.replays.notice = None;
    match action {
        ReplayAction::Record => record(state, machine, session, env),
        ReplayAction::StopRecording => stop_recording(state, machine, session, env),
        ReplayAction::Refresh => refresh_replays(state, env),
        ReplayAction::StopPlayback => stop_playback(state, session),
        ReplayAction::Play(row) => play(row, state, machine, session, env),
        ReplayAction::Verify(row) => verify_start(row, state, session, env),
        ReplayAction::TogglePauseVerify => {
            if let Some(run) = state.panel.replays.verification.as_mut() {
                run.paused = !run.paused;
            }
        }
        ReplayAction::StopVerify => stop_verify(state, env),
        ReplayAction::Reproduce(row) => reproduce_report(row, state, env),
        ReplayAction::OpenState(row) => open_state(row, state, machine),
    }
}

/// Starts a recording: the machine restarts at power-on with the original code. Refused while a
/// recording or a verification runs, or with WRAM freezes set.
fn record(state: &mut WindowState, machine: &mut Machine, session: &mut Session, env: &Env<'_>) {
    let refusal = replays::refuse_recording(
        session.is_recording(),
        env.verify.is_some(),
        !machine.freezes().is_empty(),
    );
    if let Some(reason) = refusal {
        state.panel.replays.notice = Some(reason.to_owned());
        return;
    }
    begin_recording(state, machine, session, env.rom, &recording_path());
}

/// Starts the recording at `path` from power-on. The old machine's SRAM is saved first: the new
/// machine loads the same save file.
fn begin_recording(
    state: &mut WindowState,
    machine: &mut Machine,
    session: &mut Session,
    rom: &[u8],
    path: &Path,
) {
    keep_means(state, machine);
    save_sram(machine);
    match start_recording(path, rom) {
        Ok((next, fresh)) => {
            replace_session(session, next);
            *machine = fresh;
            after_table_change(state, machine);
            state.controls.set_hooks(HooksArg::Original);
            state.controls.paused = false;
            state.panel.replays.recording = Some(stem(path));
            state.panel.replays.playing = None;
            state.panel.debug.push_log("replay", "recording from power-on");
            sync_breakpoints(machine, &state.panel.debug, true);
        }
        Err(error) => state.panel.replays.notice = Some(format!("Cannot record: {error:#}")),
    }
}

/// Ends the playback: the machine goes on with the live buttons (a recording goes on with them
/// too). Loading and freezes follow the session from then on.
fn stop_playback(state: &mut WindowState, session: &mut Session) {
    if !session.is_playing() {
        return;
    }
    session.stop_playback();
    state.panel.replays.playing = None;
    state.replay_over_logged = false;
    state.panel.debug.push_log("replay", "playback stopped");
}

/// Ends the recording, writing its file, and carries on with the machine as it is.
fn stop_recording(
    state: &mut WindowState,
    machine: &mut Machine,
    session: &mut Session,
    env: &Env<'_>,
) {
    let Some(name) = state.panel.replays.recording.take() else { return };
    let finished = std::mem::take(session).finish(Ok(()));
    state.panel.replays.notice = Some(match finished {
        Ok(()) => format!("Saved {name}.ssdreplay."),
        Err(error) => format!("Saving {name} failed: {error:#}"),
    });
    sync_breakpoints(machine, &state.panel.debug, false);
    refresh_replays(state, env);
}

/// Plays a replay from its power-on, with the hook mode the panel has.
fn play(
    row: usize,
    state: &mut WindowState,
    machine: &mut Machine,
    session: &mut Session,
    env: &Env<'_>,
) {
    if session.is_recording() {
        state.panel.replays.notice = Some("Stop the recording first.".to_owned());
        return;
    }
    if env.verify.is_some() {
        state.panel.replays.notice = Some("Stop the verification first.".to_owned());
        return;
    }
    let Some(entry) = state.panel.replays.replays.get(row).cloned() else { return };
    if let Err(reason) = &entry.frames {
        state.panel.replays.notice = Some(format!("{}: {reason}", entry.name));
        return;
    }
    keep_means(state, machine);
    // Before the new machine is built: the old machine's SRAM is saved first, as in `record`.
    save_sram(machine);
    match start_playback(&entry.path, env.rom, state.controls.hooks, env.switch) {
        Ok((next, mut fresh)) => {
            // A new machine has neither the routines' own modes nor the freezes: the ones the
            // panel shows are given to it, so the panel and the machine agree.
            if let Some(switch) = env.switch.filter(|_| state.controls.hooks_mixed)
                && let Err(error) = switch.apply_modes(&mut fresh, &routine_modes(machine))
            {
                log::warn!("keeping the routines' modes failed: {error:#}");
            }
            if let Err(error) = fresh.set_freezes(state.panel.memory.active_freezes()) {
                log::warn!("keeping the freezes failed: {error}");
            }
            replace_session(session, next);
            *machine = fresh;
            after_table_change(state, machine);
            state.controls.paused = false;
            state.panel.debug.clear_banner();
            state.panel.debug.push_log("replay", format!("playing {}", entry.name));
            state.panel.replays.playing = Some(entry.name.clone());
            state.replay_over_logged = false;
            sync_breakpoints(machine, &state.panel.debug, false);
        }
        Err(error) => {
            state.panel.replays.notice = Some(format!("Cannot play {}: {error:#}", entry.name));
        }
    }
}

/// Starts verifying a replay, with the Rust routines of the symbols file.
fn verify_start(row: usize, state: &mut WindowState, session: &Session, env: &mut Env<'_>) {
    if session.is_recording() {
        state.panel.replays.notice = Some("Stop the recording first.".to_owned());
        return;
    }
    if env.verify.is_some() {
        state.panel.replays.notice = Some("A verification is already running.".to_owned());
        return;
    }
    let Some(switch) = env.switch else {
        let note = "The symbols file could not be read: there is nothing to check against.";
        state.panel.replays.notice = Some(note.to_owned());
        return;
    };
    let Some(entry) = state.panel.replays.replays.get(row).cloned() else { return };
    match start_verification(&entry.path, env.rom, switch) {
        Ok((run, frames)) => {
            let report = run.report();
            state.panel.replays.verification = Some(Verification::new(entry.name, frames, report));
            env.verify = Some(run);
        }
        Err(error) => {
            state.panel.replays.notice = Some(format!("Cannot verify {}: {error:#}", entry.name));
        }
    }
}

/// Runs a verification for up to [`VERIFY_SLICE`] (at least one step), unless it is paused, and
/// records how it ends. A divergence is saved as a report. Returns whether the run ended.
fn verify_tick(state: &mut WindowState, env: &mut Env<'_>) -> bool {
    let paused = state.panel.replays.verification.as_ref().is_none_or(|run| run.paused);
    let dir = env.divergence_dir();
    let name = state.panel.replays.verification.as_ref().map(|run| run.name.clone());
    let Some(run) = env.verify.as_mut() else { return false };
    if paused {
        return false;
    }
    let name = name.unwrap_or_default();
    let started = Instant::now();
    let mut finished = None;
    loop {
        match run.next_within(1) {
            Ok(Some(LockstepStep::Diverged(divergence))) => {
                let event = run.report().events;
                let note = match replays::save_lockstep_report(
                    &dir,
                    &name,
                    event,
                    &divergence,
                    run.machine_b(),
                ) {
                    Ok(path) => format!("Divergence saved: {}", path.display()),
                    Err(error) => format!("The divergence could not be saved: {error}"),
                };
                finished = Some((Outcome::Diverged { event }, Some(note)));
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                finished = Some((replays::outcome_of(&run.report()), None));
                break;
            }
            Err(error) => {
                let frame = run.machine_a().frame_count();
                let note = format!("The verification failed: {error}");
                finished = Some((Outcome::Stopped { frame }, Some(note)));
                break;
            }
        }
        if started.elapsed() >= VERIFY_SLICE {
            break;
        }
    }
    let report = run.report();
    if let Some(view) = state.panel.replays.verification.as_mut() {
        view.report = report;
    }
    let Some((outcome, note)) = finished else { return false };
    env.verify = None;
    state.panel.replays.finished(outcome);
    if note.is_some() {
        state.panel.replays.notice = note;
    }
    true
}

/// Stops the verification run where it is; its report stays on the panel.
fn stop_verify(state: &mut WindowState, env: &mut Env<'_>) {
    let Some(run) = env.verify.take() else { return };
    let frame = run.machine_a().frame_count();
    state.panel.replays.finished(Outcome::Stopped { frame });
}

/// Runs a routine's divergence report again in-process; the verdict goes on its row.
fn reproduce_report(row: usize, state: &mut WindowState, env: &Env<'_>) {
    let Some(report) = state.panel.replays.reports.get(row).cloned() else { return };
    let Some(header) = report.header.clone() else {
        let note = "A replay check has no routine to run again: use Open state.";
        state.panel.replays.notice = Some(note.to_owned());
        return;
    };
    let Some(switch) = env.switch else {
        let note = "The symbols file could not be read: there is no routine to run.";
        state.panel.replays.notice = Some(note.to_owned());
        return;
    };
    let verdict = match reproduce_text(&report.state, &header, env.rom, switch) {
        Ok(text) => text,
        Err(error) => format!("Cannot run it: {error:#}"),
    };
    if let Some(entry) = state.panel.replays.reports.get_mut(row) {
        entry.verdict = Some(verdict);
    }
}

/// What running the report's call again says, in a line.
fn reproduce_text(
    state_path: &Path,
    header: &ReproHeader,
    rom: &[u8],
    switch: &HookSwitch,
) -> anyhow::Result<String> {
    let state =
        std::fs::read(state_path).with_context(|| format!("reading {}", state_path.display()))?;
    let table = switch.resolver_table()?;
    let resolve = |name: &str| table.entries().find(|entry| entry.name == name).cloned();
    let symbols = Some(Arc::clone(switch.symbols()));
    let verdict = repro::reproduce(rom, &state, header, &resolve, symbols)?;
    Ok(match verdict {
        repro::Verdict::Matches { frame, cost_warning: None } => {
            format!("Matches the original at frame {frame}.")
        }
        repro::Verdict::Matches { frame, cost_warning: Some(warning) } => {
            format!("Matches at frame {frame}; {warning}.")
        }
        repro::Verdict::Diverges(divergence) => {
            let differences = divergence.differences.len() + divergence.truncated;
            format!("Still diverges: {differences} differences.")
        }
        repro::Verdict::Unreproducible(reason) => format!("Cannot reproduce: {reason}"),
    })
}

/// Loads the state of a report into the machine, paused there.
fn open_state(row: usize, state: &mut WindowState, machine: &mut Machine) {
    if !state.controls.load_allowed {
        let note = "Loading a state waits for a replay to end, and is off while recording.";
        state.panel.replays.notice = Some(note.to_owned());
        return;
    }
    let Some(report) = state.panel.replays.reports.get(row).cloned() else { return };
    let loaded = std::fs::read(&report.state)
        .with_context(|| format!("reading {}", report.state.display()))
        .and_then(|bytes| machine.load_state(&bytes).map_err(anyhow::Error::from));
    match loaded {
        Ok(()) => {
            state.controls.paused = true;
            if let Some(hd) = state.hd.as_mut() {
                hd.reset();
            }
            state.panel.debug.banner = Some(Banner {
                label: "Opened".to_owned(),
                title: stem(&report.state),
                detail: "The machine is paused on the state of the report.".to_owned(),
            });
            state.panel.debug.push_log("state", format!("opened {}", stem(&report.state)));
        }
        Err(error) => {
            state.panel.replays.notice = Some(format!("Cannot open the state: {error:#}"));
        }
    }
}

/// The window's pictures read the pack's art on a thread of its own (see [`ssd_hd::Renderer::set_art_async`]): a
/// picture draws the art read so far, and the threads that draw (the window's, and the one that draws in the
/// background) never wait for an art file.
fn read_art_apart(state: &mut WindowState, enhanced: Option<&mut Enhanced>) {
    if let Some(hd) = state.hd.as_mut() {
        hd.set_art_async(true);
    }
    if let Some(enhanced) = enhanced {
        enhanced.renderer.set_art_async(true);
    }
}

/// Runs the windowed loop: the window, its event loop and the frame loop of [`run_windowed`],
/// until the player quits.
/// The Enhanced picture of a run (`--hd-scale`): the renderer that draws it, and whether a frame it
/// refused has been reported.
struct Enhanced {
    renderer: ssd_hd::Renderer,
    options: HdOptions,
    refused: bool,
    /// What the pictures drawn since [`Enhanced::take_cost`] cost.
    cost: Option<ssd_hd::DrawCost>,
}

impl Enhanced {
    fn new(options: HdOptions, rom: &[u8]) -> Self {
        let mut renderer = ssd_hd::Renderer::new();
        renderer.set_art(hd_art::pack(rom));
        renderer.set_font(hd_art::font(rom));
        renderer.set_lightning(hd_art::lightning());
        Self { renderer, options, refused: false, cost: None }
    }

    /// What the pictures drawn since the last call cost, added up; `None` when none was drawn.
    fn take_cost(&mut self) -> Option<ssd_hd::DrawCost> {
        self.cost.take()
    }

    /// The Enhanced picture of the machine's last frame, or `None` when there is none to draw: the
    /// PPU's render log is switched on when it is off (its frames are logged from then on), and a
    /// frame the renderer refuses is reported once. The Classic picture is shown instead. `lightning` is the frames since
    /// Goku's first Super Saiyan transformation started while it goes on ([`crate::voice::SsjWatch::lightning_at`]).
    fn picture(&mut self, machine: &mut Machine, lightning: Option<u32>) -> Option<HdFrame> {
        if machine.frame_log().is_none() {
            machine.set_line_log(true);
            return None;
        }
        self.renderer.set_conditions(
            ssd_hd::Conditions::of_work_ram(machine.wram()).with_ssj_lightning(lightning),
        );
        let log = machine.frame_log()?;
        let started = Instant::now();
        match self.renderer.render_hd(log, self.options) {
            Ok(picture) => {
                let drawn =
                    ssd_hd::DrawCost { total: started.elapsed(), ..self.renderer.art_cost() };
                self.cost.get_or_insert_default().add(&drawn);
                Some(picture)
            }
            Err(reason) => {
                if !self.refused {
                    log::warn!(
                        "the Enhanced picture is not drawn ({reason}): the Classic one is shown"
                    );
                    self.refused = true;
                }
                None
            }
        }
    }
}

/// The longest a window frame spends emulating, when the machine cannot keep up with the speed:
/// the frames it is behind by are run in one go (one picture, one pass of the interface, one
/// present for all of them) up to this long, so a fast-forward the machine cannot keep up with
/// still runs flat out instead of paying for every picture.
const BATCH_TIME: Duration = Duration::from_millis(50);
/// The most frames a window frame runs.
const MAX_BATCH: u32 = 64;
/// The most frames a window frame runs at speed 1 to keep up with the clock when the screen is slower than the game
/// (a phone's adaptive refresh rate drops to 24 Hz or so while nothing touches it, and the present waits for it): the
/// game and its sound keep their time, and only the picture is less smooth.
const CATCH_UP: u32 = 4;

/// The clock of the frame loop: how many frames a window frame runs, and when the next one starts.
///
/// At speed 1 a window frame runs one frame and the clock is one frame long (and the frames it is behind by, up to
/// [`CATCH_UP`], when the screen is slower than the game). At speed N it runs N
/// frames of a clock that is still one display refresh long, and more when it is behind (the
/// frames it owes, as many as [`BATCH_TIME`] holds at the cost a frame has had lately): the speed is
/// the machine's to give, never the picture's.
struct Pacer {
    /// One display refresh: a frame of the game at speed 1.
    frame_time: Duration,
    /// When the next window frame starts.
    next: Instant,
    /// What a frame cost lately (emulation and the upkeep of its picture), smoothed.
    cost: Duration,
    /// The Gameplay page's battle speed the window frames run, if they run one ([`battle_tempo_now`]): (frames of the
    /// game, display refreshes). At speed 1 a frame of the game is then refreshes/frames of a display refresh: 2/3 at
    /// 1.5x, so 3 frames run to 2 refreshes (1, 1, 2, 1, 2, … a window frame), 1/2 at 2x, so 2 frames run to each
    /// refresh, each one's sound heard. Set before [`Pacer::frames`] and left for the [`Pacer::wait`] of the same
    /// window frame.
    tempo: Option<(u32, u32)>,
}

impl Pacer {
    fn new(frame_time: Duration) -> Self {
        Self { frame_time, next: Instant::now(), cost: Duration::from_millis(2), tempo: None }
    }

    /// A frame of the game at speed 1: a display refresh, or refreshes/frames of one at the battle speed (2/3 at 1.5x,
    /// 1/2 at 2x).
    fn game_frame_time(&self) -> Duration {
        match self.tempo {
            Some((frames, refreshes)) => self.frame_time * refreshes / frames.max(1),
            None => self.frame_time,
        }
    }

    /// The frames to run in the window frame that starts at `now`, at `speed`.
    fn frames(&self, speed: u32, now: Instant) -> u32 {
        if speed <= 1 {
            let owed = now.saturating_duration_since(self.next).as_secs_f64()
                / self.game_frame_time().as_secs_f64();
            return (1 + owed as u32).min(CATCH_UP);
        }
        let period = self.frame_time.as_secs_f64() / f64::from(speed);
        let owed = (now.saturating_duration_since(self.next).as_secs_f64() / period) as u32;
        let fits = (BATCH_TIME.as_secs_f64() / self.cost.as_secs_f64().max(1e-5)) as u32;
        (speed.saturating_add(owed)).min(fits.max(speed)).min(MAX_BATCH.max(speed))
    }

    /// Notes that `frames` frames took `time` to run (emulation and the picture's upkeep).
    fn note_cost(&mut self, frames: u32, time: Duration) {
        if frames > 0 {
            let each = time / frames;
            self.cost = (self.cost * 3 + each) / 4;
        }
    }

    /// Moves the start of the next window frame on by the time of the `requested` frames (0: it ran none, one display
    /// refresh) at `speed`.
    fn advance(&mut self, speed: u32, requested: u32) {
        self.next += if requested == 0 {
            self.frame_time
        } else if speed <= 1 && self.tempo.is_some() {
            self.game_frame_time() * requested
        } else {
            self.frame_time * requested / speed.max(1)
        };
    }

    /// Ends a window frame that asked for `requested` frames (0: it ran none) at `speed`: waits for
    /// the next one's time. A verification keeps no pace, and a clock that fell far behind (a window
    /// drag, a breakpoint, a machine too slow) starts over.
    fn wait(&mut self, speed: u32, requested: u32, verifying: bool) {
        if verifying {
            self.next = Instant::now();
            return;
        }
        self.advance(speed, requested);
        let now = Instant::now();
        if self.next > now {
            std::thread::sleep(self.next - now);
        } else if now - self.next > self.frame_time * 5 {
            self.next = now;
        }
    }
}

/// Whether the window frame runs the Gameplay page's battle speed: the page has it on in a battle now (`in_battle`,
/// [`ssd_game::gameplay::battle_tempo`]), the speed is 1 (not the hotkey's fast-forward, nor the animation speed's
/// attack animation: those run batches whose sound is the first frame's only), and no replay is recorded or played
/// (the sound chip at another clock changes what the CPU sees of it, so a replay plays only what it was recorded at:
/// the hardware's ratio).
fn battle_tempo_now(
    multiplier: u32,
    replay_active: bool,
    battle_tempo: Option<(u32, u32)>,
) -> Option<(u32, u32)> {
    battle_tempo.filter(|_| multiplier == 1 && !replay_active)
}

/// Sets the window frame's battle speed ([`battle_tempo_now`]): the sound chip's clock (the inverse of the tempo with
/// it, 2/3 at 1.5x and 1/2 at 2x; the hardware's without), the pacer's frame of the game (2/3 or 1/2 of a display
/// refresh with it) and every frame of a window frame run whole (`WindowState::whole_frames`).
fn set_battle_tempo(
    state: &mut WindowState,
    machine: &mut Machine,
    pacer: &mut Pacer,
    tempo: Option<(u32, u32)>,
) {
    let (num, den) = tempo.map_or((1, 1), ssd_game::gameplay::tempo_apu);
    machine.set_apu_clock_ratio(num, den);
    pacer.tempo = tempo;
    state.whole_frames = tempo.is_some();
}

#[allow(clippy::too_many_arguments)]
fn windowed_loop(
    machine: &mut Machine,
    rom: &[u8],
    session: &mut Session,
    mode: HooksArg,
    switch: Option<&HookSwitch>,
    layer: Option<Hd>,
    enhanced: Option<HdOptions>,
    start: Start,
    debug: bool,
) -> anyhow::Result<()> {
    let sdl = sdl3::init()?;
    let video_subsystem = sdl.video()?;
    let mut input = InputState::default();
    let mut env = Env { rom, switch, verify: None };
    let mut enhanced = enhanced.map(|options| Enhanced::new(options, rom));
    if enhanced.is_some() {
        machine.set_line_log(true);
    }
    let mut state = window_start_state(mode, session, debug);
    state.hd = layer;
    read_art_apart(&mut state, enhanced.as_mut());
    state.scaled = enhanced.as_ref().map(|enhanced| enhanced.options);
    (state.video, state.video_saved) = (start.shown, start.saved);
    let settings_file = crate::options::read_settings().unwrap_or_default();
    state.bindings = Bindings::from_values(&settings_file);
    // The touch controls (M9). SDL turns a touch into mouse events too; the controls take the fingers themselves, so the
    // interface under them (the debug panel's pill) does not also click.
    let touch_on = touch::wanted();
    if touch_on {
        sdl3::hint::set("SDL_TOUCH_MOUSE_EVENTS", "0");
    }
    // A launch after the system killed the app in the background continues from the state kept then.
    #[cfg(target_os = "android")]
    if paths::persists() && !session.is_active() {
        let kept = read_session(&crate::android_lifecycle::path());
        if crate::android_lifecycle::take(machine)
            && let Some(hd) = state.hd.as_mut()
        {
            hd.reset();
            if let Some(kept) = &kept
                && let Err(e) = hd.restore_session(kept, machine)
            {
                log::warn!(
                    "the bag and the cheats of the background state were not restored: {e:#}"
                );
            }
        }
    }
    refresh_controls(&mut state, session, machine, &env);
    refresh_replays(&mut state, &env);
    let mut shown = (state.controls.speed, state.controls.slot);
    // Android: how far the start got, until its first picture (`android_report::stage`).
    #[cfg(target_os = "android")]
    crate::android_report::stage(crate::launch_guard::Stage::Window);
    let mut video = video::Video::new(&video_subsystem, &window_title(shown.0, shown.1))?;
    // Test builds only: a start that fails here, before the first frame (`android_report::test_failure`).
    #[cfg(all(target_os = "android", debug_assertions))]
    crate::android_report::test_failure();
    state.touch = window_touch(
        touch_on,
        touch::settings::Settings::from_values(&settings_file),
        video.screen_points(),
        state.controls.debug,
    );
    state.touch.set_words(crate::lang::interface_text(&settings_file));
    // Set while the touch MENU, not the player, paused the game: only then does Resume unpause.
    let mut touch_paused = false;
    video.set_fullscreen(state.video.fullscreen);
    video.set_vsync(state.video.vsync);
    let mut shape_buffer: Vec<u8> = Vec::new();
    let mut ui_input = InputAdapter::default();
    let mut hd_previous = Joypad::default();
    // The pad as the touch MENU's windows last saw it (their keys are the presses since).
    let mut touch_previous = Joypad::default();
    let mut actions: Vec<PanelAction> = Vec::new();
    let mut fps = FpsCounter::default();
    #[cfg(target_os = "android")]
    crate::android_report::stage(crate::launch_guard::Stage::Audio);
    let mut audio = audio::Audio::open(&sdl);
    audio.set_settings(AudioSettings::from_values(
        &crate::options::read_settings().unwrap_or_default(),
    ));
    audio.load_voice(&crate::paths::data_dir());
    machine.set_audio_frequency(audio::Audio::frequency());
    #[cfg(target_os = "android")]
    crate::android_report::stage(crate::launch_guard::Stage::FirstFrame);
    let mut events = sdl.event_pump()?;
    // Keyboard keeps working without gamepad support. Opened pads are kept alive here; SDL also
    // announces the pads already connected at start-up as `GamepadAdded` events.
    let gamepad_subsystem =
        sdl.gamepad().inspect_err(|e| log::warn!("gamepad support unavailable: {e}")).ok();
    let mut gamepads: Vec<Gamepad> = Vec::new();

    // The first Enhanced launch asks for the language of the text, in this window, before the first frame.
    if let Some(hd) = state.hd.as_mut()
        && crate::language_pick::needed(true, &settings_file)
    {
        let pads = (gamepad_subsystem.as_ref(), &mut gamepads);
        let Some(language) =
            crate::language_pick::run(&mut video, &mut events, &mut ui_input, pads)?
        else {
            return Ok(());
        };
        let values = hd.pc_options_mut();
        language.store(values);
        let values = values.clone();
        hd.follow_language(&values);
        state.touch.set_words(crate::lang::interface_text(&values));
        if let Err(error) = crate::options::write_settings(&values) {
            log::warn!("the language was not saved: {error:#}");
        }
    }
    let mut pacer = Pacer::new(Duration::from_secs_f64(1.0 / machine.target_fps()));
    let mut stats_start = pacer.next;
    let mut stats_frames = 0_u32;
    let mut slow = SlowFrames::new(Instant::now());
    // The Performance Hint session of the window's frames (Android; elsewhere it only keeps its target).
    // (the thread that draws the pictures in the background is one of them: it starts now so the session names it)
    let draw_threads: Vec<i32> =
        state.hd.as_mut().and_then(Hd::start_draw_worker).into_iter().collect();
    let mut perf = crate::perf_hint::Session::new(pacer.frame_time, &draw_threads);
    // Android: set between the events that send the app to the background and bring it back. While it is set
    // nothing runs, and the event pump blocks (SDL waits there until the app is in the foreground again).
    #[cfg(target_os = "android")]
    let mut backgrounded = false;
    #[cfg(target_os = "android")]
    let mut setup_hold: Option<Instant> = None;
    // Paints Esc's question in Classic, which has no HD art (the window needs no picture).
    let mut plain_art = hd::Art::empty();
    'main: loop {
        // The window frame's work for the Performance Hint session: from here to just before the present.
        let frame_started = Instant::now();
        for event in events.poll_iter() {
            video.handle_event(&mut ui_input, &event);
            match event {
                Event::Quit { .. } => break 'main,
                // Android: the app lost the focus (home button, another app, the screen off; the system takes the
                // window's surface a moment later) or is ending. SDL3 sends this window event first, and the
                // `App...Background` events where the SDL build has them.
                #[cfg(target_os = "android")]
                Event::Window { win_event: WindowEvent::FocusLost, .. }
                | Event::AppWillEnterBackground { .. }
                | Event::AppDidEnterBackground { .. }
                | Event::AppTerminating { .. } => {
                    input.clear();
                    if !backgrounded {
                        backgrounded = true;
                        video.suspend();
                        // The battery save as on desktop, then the whole state for a kill in the background.
                        save_sram(machine);
                        if paths::persists() && !session.is_active() && env.verify.is_none() {
                            match crate::android_lifecycle::save(machine) {
                                Ok(()) => {
                                    let kept = state.hd.as_ref().and_then(Hd::session_bytes);
                                    save_session(
                                        &crate::android_lifecycle::path(),
                                        kept.as_deref(),
                                    );
                                    log::info!("in the background: SRAM and state saved");
                                }
                                Err(e) => log::warn!("saving the background state failed: {e:#}"),
                            }
                        }
                    }
                }
                #[cfg(target_os = "android")]
                Event::Window { win_event: WindowEvent::FocusGained, .. }
                | Event::AppWillEnterForeground { .. }
                | Event::AppDidEnterForeground { .. } => {
                    if backgrounded {
                        backgrounded = false;
                        video.resume();
                        // The state file is for a kill only: a later battery save must not lose to it.
                        crate::android_lifecycle::discard();
                        // The clock starts over: the time spent away is not owed to the game.
                        pacer.next = Instant::now();
                        stats_start = pacer.next;
                        stats_frames = 0;
                        log::info!("in the foreground again");
                    }
                }
                #[cfg(target_os = "android")]
                Event::AppLowMemory { .. } => log::warn!("the system is low on memory"),
                // The Android back key opens the touch MENU (and backs out of it).
                Event::KeyDown { scancode: Some(Scancode::AcBack), repeat: false, .. }
                    if state.touch.enabled =>
                {
                    state.touch.back_key();
                }
                Event::KeyDown { scancode: Some(sc), repeat: false, .. } => {
                    if let Some(device) = state.hd.as_ref().and_then(Hd::listening) {
                        // A key box of the Controls page waits: it gets the press, the game and the
                        // shortcuts do not.
                        if let (Some(capture), Some(hd)) =
                            (bindings::capture_key(device, sc), state.hd.as_mut())
                        {
                            hd.capture(capture);
                        }
                    } else {
                        // Esc asks before the game ends (see `quit_confirm`); while it asks, the game gets no key.
                        let hotkey = state.bindings.key_hotkey(sc);
                        let route = route_key(hotkey, video.text_focus());
                        let press = quit_confirm::key_press(&state.bindings, sc);
                        match quit_confirm::key(&mut state.quit_confirm, route, press) {
                            quit_confirm::Step::Quit => break 'main,
                            quit_confirm::Step::Action(action) => actions.push(action),
                            quit_confirm::Step::Game => input.key(&state.bindings, sc, true),
                            quit_confirm::Step::Opened | quit_confirm::Step::Closed => {
                                input.clear();
                            }
                            quit_confirm::Step::Nothing => {}
                        }
                    }
                }
                Event::KeyUp { scancode: Some(sc), .. } => input.key(&state.bindings, sc, false),
                Event::FingerDown { finger_id, x, y, .. } if state.touch.enabled => {
                    state.touch.finger_down(finger_id as i64, touch_point(&video, x, y));
                }
                Event::FingerMotion { finger_id, x, y, .. } if state.touch.enabled => {
                    state.touch.finger_motion(finger_id as i64, touch_point(&video, x, y));
                }
                Event::FingerUp { finger_id, .. } if state.touch.enabled => {
                    state.touch.finger_up(finger_id as i64, false);
                }
                Event::FingerCanceled { finger_id, .. } if state.touch.enabled => {
                    state.touch.finger_up(finger_id as i64, true);
                }
                Event::GamepadButtonDown { button, .. } => {
                    state.touch.pad_used();
                    if let Some(device) = state.hd.as_ref().and_then(Hd::listening) {
                        if let (Some(capture), Some(hd)) =
                            (bindings::capture_button(device, button), state.hd.as_mut())
                        {
                            hd.capture(capture);
                        }
                    } else if let Some(hotkey) = state.bindings.pad_hotkey(button) {
                        actions.extend(PanelAction::from_hotkey(hotkey));
                    } else if state.quit_confirm.is_some() {
                        let press = quit_confirm::button_press(&state.bindings, button);
                        match quit_confirm::key(&mut state.quit_confirm, KeyRoute::Game, press) {
                            quit_confirm::Step::Quit => break 'main,
                            quit_confirm::Step::Closed => input.clear(),
                            _ => {}
                        }
                    } else {
                        input.gamepad_button(&state.bindings, button, true);
                    }
                }
                Event::GamepadButtonUp { button, .. } => {
                    input.gamepad_button(&state.bindings, button, false);
                }
                Event::GamepadAdded { which, .. } => {
                    if let Some(subsystem) = &gamepad_subsystem {
                        match subsystem.open(which) {
                            Ok(pad) => {
                                log::info!("gamepad connected: {}", pad.name().unwrap_or_default());
                                gamepads.push(pad);
                            }
                            Err(e) => log::warn!("opening gamepad failed: {e}"),
                        }
                    }
                }
                Event::GamepadRemoved { which, .. } => {
                    gamepads.retain(|pad| pad.id().ok() != Some(which));
                    input.clear();
                }
                Event::Window { win_event, .. } => match win_event {
                    WindowEvent::Minimized => video.set_minimized(true),
                    WindowEvent::Restored | WindowEvent::Maximized => {
                        video.set_minimized(false);
                        video.handle_resize();
                    }
                    WindowEvent::Resized(..) | WindowEvent::PixelSizeChanged(..) => {
                        video.handle_resize();
                    }
                    WindowEvent::FocusLost => input.clear(),
                    _ => {}
                },
                _ => {}
            }
        }
        #[cfg(target_os = "android")]
        if backgrounded {
            // Not drawn and not run; the next poll blocks until the app is back.
            std::thread::sleep(Duration::from_millis(20));
            continue 'main;
        }
        if state.touch.enabled {
            state.touch.set_screen(video.screen_points());
            let label = match state.controls.speed.multiplier() {
                2 => "x2",
                4 => "x4",
                8 => "x8",
                _ => "x1",
            };
            let battle = ssd_game::gameplay::in_battle(machine.wram());
            // A real pad has a stick and face buttons: the keyboard-like devices some phones and emulators list do not.
            let pad_here = gamepads.iter().any(|pad| {
                pad.has_axis(sdl3::gamepad::Axis::LeftX)
                    && pad.has_button(sdl3::gamepad::Button::South)
            });
            state.touch.set_status(label, state.controls.slot, pad_here, battle);
            for out in state.touch.take_out() {
                match out {
                    touch::Out::Speed => actions.push(PanelAction::CycleSpeed),
                    touch::Out::Save => actions.push(PanelAction::SaveState),
                    touch::Out::Load => actions.push(PanelAction::LoadState),
                    touch::Out::Slot(slot) => actions.push(PanelAction::SelectSlot(slot)),
                    touch::Out::Cheats => actions.push(PanelAction::BattleCheats),
                    touch::Out::Pause(true) => {
                        if !state.controls.paused && !touch_paused {
                            actions.push(PanelAction::TogglePause);
                            touch_paused = true;
                        }
                    }
                    touch::Out::Pause(false) => {
                        if touch_paused {
                            touch_paused = false;
                            if state.controls.paused {
                                actions.push(PanelAction::TogglePause);
                            }
                        }
                    }
                    touch::Out::Haptic => touch::haptic(),
                    touch::Out::Persist => persist_touch(&mut state),
                    touch::Out::Quit => break 'main,
                    touch::Out::Language | touch::Out::Restart => {
                        if out == touch::Out::Language {
                            forget_language(&mut state);
                        }
                        // The launch loop starts the game again (Android); a desktop run has none, and Restart
                        // and Change language there wait for the next start.
                        #[cfg(target_os = "android")]
                        {
                            crate::android_setup::request(crate::android_setup::Forced::Relaunch);
                            break 'main;
                        }
                    }
                }
            }
            input.set_touch(state.touch.pressed());
        }
        if std::mem::take(&mut state.release_input) {
            input.clear();
        }
        // The game does not see the keys a text field is typing, nor any while Esc's question is on screen.
        let mut pad = if video.text_focus() || state.quit_confirm.is_some() {
            Joypad::default()
        } else {
            input.joypad()
        };
        if state.touch.enabled && state.hd.is_some() {
            // Enhanced: Select + Start held for two seconds opens the touch MENU (the game sees neither while both are held).
            (pad.select, pad.start) = state.touch.chord(Instant::now(), pad.select, pad.start);
        }
        if state.touch.enabled && state.touch.mode != touch::Mode::Play {
            // A window of the touch MENU is up: the pad moves its cursor, and the game (paused) and the HD menus see nothing.
            for key in touch_keys(touch_previous, pad) {
                state.touch.pad_key(key);
            }
            touch_previous = pad;
            if state.touch.mode == touch::Mode::Play {
                // The press that closed it is let go, so the game does not get it.
                input.clear();
            }
            pad = Joypad::default();
        } else {
            touch_previous = pad;
        }
        // The keys of the host are the HD's navigation while the HD has the curtain; the game only
        // sees them when it has no curtain (the driver ignores them otherwise).
        if let Some(hd) = state.hd.as_mut() {
            // Y held moves the pack under the Data packs screen's cursor (see `Nav::key_mods`).
            hd.set_y_held(pad.y);
            for key in hd::keys_pressed(hd_previous, pad) {
                hd.key(key, machine.wram());
            }
            // Start closes the Cards screen (a key of its own: the other screens have none)
            if pad.start && !hd_previous.start {
                hd.close_menu();
            }
        }
        hd_previous = pad;
        // A request of the Data packs screen: the game restarts into the set or the chapter it asks for, and this run ends.
        // A refusal (a set the checks do not take) stays on the screen.
        // "Install a pack...": the picker's file is copied while the game goes on, and the screen lists it.
        if let Some(answer) = packs::poll_install()
            && let Some(hd) = state.hd.as_mut()
        {
            match answer {
                Ok(info) => {
                    hd.set_mods_alert(format!(
                        "{} is installed: switch it on and apply.",
                        info.name
                    ));
                    hd.add_pack(info);
                }
                Err(message) => hd.set_mods_alert(message),
            }
        }
        if let Some(request) = state.hd.as_mut().and_then(hd::Hd::take_mods_request) {
            if request == ssd_menus::packs::Request::Install {
                packs::pick_install();
            } else {
                match packs::carry_out(request) {
                    Ok(()) => break 'main,
                    Err(message) => {
                        if let Some(hd) = state.hd.as_mut() {
                            hd.set_mods_alert(message);
                        }
                    }
                }
            }
        }
        // Android: the Options' Game page asks for the setup (Enhanced); in Classic, Select + Start held for two seconds
        // does (Classic has no Options). The game ends and the launch loop shows the page.
        #[cfg(target_os = "android")]
        {
            use crate::android_setup::{Forced, request};
            let asked = match state.hd.as_mut().and_then(Hd::take_setup_request) {
                Some(ssd_menus::options::Opens::Mode) => Some(Forced::Mode),
                Some(ssd_menus::options::Opens::Pack) => Some(Forced::Pack),
                _ => None,
            };
            let held = state.hd.is_none() && pad.select && pad.start;
            let long_hold = if held {
                setup_hold.get_or_insert_with(Instant::now).elapsed() >= Duration::from_secs(2)
            } else {
                setup_hold = None;
                false
            };
            if let Some(forced) = asked.or(long_hold.then_some(Forced::Mode)) {
                request(forced);
                break 'main;
            }
        }
        for action in std::mem::take(&mut actions) {
            apply_action(action, &mut state, machine, session, &mut env, pad, &mut audio);
        }
        refresh_controls(&mut state, session, machine, &env);
        let speed = state.controls.speed;
        if shown != (speed, state.controls.slot) {
            shown = (speed, state.controls.slot);
            video.set_title(&window_title(shown.0, shown.1));
        }

        // The Gameplay page's battle animation speed: the attack animation runs that many machine frames to one frame
        // of the screen (every frame whole, so the outcome is the original's), on top of the speed of the hotkey.
        let boost = state.hd.as_ref().map_or(1, |hd| hd.driver().animation_factor(machine.wram()));
        let multiplier = speed.multiplier() * boost;
        // The Gameplay page's battle speed: 3 frames of the game to 2 of the screen at 1.5x, 2 to 1 at 2x, the sound
        // chip at the inverse of its clock.
        let tempo = battle_tempo_now(
            multiplier,
            session.is_active(),
            state.hd.as_ref().and_then(|hd| hd.driver().battle_tempo(machine.wram())),
        );
        set_battle_tempo(&mut state, machine, &mut pacer, tempo);
        let mut frames_run = 0_u32;
        let mut requested = 0_u32;
        let mut stages = Stages::default();
        if game_runs(&state, env.verify.is_some()) {
            requested = if state.step_once { 1 } else { pacer.frames(multiplier, Instant::now()) };
            state.stages = Stages::default();
            state.real_time = multiplier <= 1;
            frames_run =
                window_frames(&mut state, machine, session, &env, pad, &mut audio, requested)?;
            pacer.note_cost(frames_run, state.stages.frames());
            stages = state.stages;
            stats_frames += frames_run;
        } else {
            // Nothing ran: the picture on show is the last frame's, which Latency Smooth may still be drawing
            settle_draw(&mut state, machine);
        }
        if std::mem::take(&mut state.video_apply) {
            video.set_fullscreen(state.video.fullscreen);
            video.set_vsync(state.video.vsync);
            if let Some(enhanced) = enhanced.as_mut() {
                enhanced.options = state.video.hd();
            }
        }
        if verify_tick(&mut state, &mut env) {
            refresh_replays(&mut state, &env);
        }
        let now = video.now();
        let rate = fps.add(now, frames_run);
        if state.touch.enabled && state.touch.settings.show_fps {
            let (queued_ms, gaps) = audio.health();
            state.touch.stats =
                Some(format!("{rate:.1} fps · sound {queued_ms:.0} ms · {gaps} gaps"));
        }
        note_wram(&mut state, machine);
        if any_verify(machine)
            && let Some(routine) = state.watch.update(&machine.hook_stats())
        {
            state.panel.push_toast(
                format!("Divergence in {routine} \u{2014} saved for repro"),
                Tab::Replays,
                now,
            );
        }

        // While paused this re-presents the last frame (a state load shows up here too). Skipped
        // while minimised, but the loop keeps pacing below so it does not spin the CPU.
        let (queued_ms, gaps) = audio.health();
        let live = Live {
            queued_ms,
            gaps,
            tempo: pacer.tempo,
            requested,
            run: frames_run,
            slow: slow.count,
            slowest_ms: ms(slow.slowest) as f32,
        };
        let status = Status { live: Some(live), ..header_status(machine, &state.history, rate) };
        let registers = machine.cpu_registers();
        let controls = state.controls;
        let verify_labels = env.verify.as_ref().map(|run| {
            let routines = run.report().hook_stats.len();
            [
                "machine A \u{b7} pure original".to_owned(),
                format!("machine B \u{b7} {routines} routines in Rust"),
            ]
        });
        let picture = fallback_picture(&state, machine, enhanced.as_mut(), env.verify.is_some());
        let views = game_views(
            state.hd_view.as_ref(),
            state.view.as_ref(),
            machine,
            picture.as_ref(),
            env.verify.as_ref(),
            state.video.linear(),
        );
        let shape = state.video.shape();
        let views = shape_views(views, shape, &mut shape_buffer);
        // The HD menu on screen, from this frame's state: nothing on the field, nothing in Classic.
        let scene = state.hd.as_ref().and_then(|hd| hd.scene(machine.wram()));
        let overlay = state.hd.as_ref().and_then(|hd| hd.overlay(machine.wram(), machine.sram()));
        let (scene, overlay) = with_quit_confirm(&state, scene, overlay);
        let history = &state.history;
        let touch_view = state.touch.enabled.then_some(&state.touch);
        let panel = &mut state.panel;
        let asking = &mut state.quit_confirm;
        let mut answer = None;
        let mut hd_art = state.hd.as_mut().map(|hd| hd.art(shape));
        if hd_art.is_none() && asking.is_some() {
            hd_art = Some(&mut plain_art);
        }
        let symbols = env.symbols();
        let shown: &Machine = machine;
        perf.report_frame(frame_started.elapsed(), pacer.frame_time);
        let presenting = Instant::now();
        video.present(&views, &mut ui_input, |ui, games| {
            let view = View {
                status,
                controls: &controls,
                registers: &registers,
                recording: panel.replays.recording.is_some(),
                verify_labels: verify_labels.clone(),
                machine: shown,
                symbols,
                history,
                touch: touch_view,
            };
            let layer = frame_layer(scene.as_ref(), overlay.as_ref(), hd_art.as_deref_mut());
            ui::draw(ui, &view, games, panel, &mut actions, layer);
            if let Some(question) = asking.as_mut() {
                answer = ui::quit_confirm_pointer(ui, question);
            }
        })?;
        let present = presenting.elapsed();
        if let Some(outcome) = answer {
            match quit_confirm::settle(&mut state.quit_confirm, outcome) {
                quit_confirm::Step::Quit => break 'main,
                _ => input.clear(),
            }
        }

        if let Some(kind) = state.shot.take() {
            let result = take_screenshot(
                kind,
                &mut video,
                &views,
                (scene.as_ref(), overlay.as_ref()),
                hd_art,
                machine,
                state.ppu_stale,
                state.touch.stretches_picture(),
            );
            match &result {
                Ok((png, state)) => log::info!(
                    "screenshot saved to {} (state beside it at {})",
                    png.display(),
                    state.display()
                ),
                Err(e) => log::warn!("screenshot failed: {e}"),
            }
            let now = video.now();
            state.panel.push_notice(screenshot::notice(&result), now);
        }

        let elapsed = stats_start.elapsed();
        if elapsed >= Duration::from_secs(5) {
            log::debug!("{:.2} frames/s", f64::from(stats_frames) / elapsed.as_secs_f64());
            stats_start = Instant::now();
            stats_frames = 0;
        }

        let picture = match state.hd.as_mut().and_then(Hd::take_draw_cost) {
            Some((cost, worker)) => {
                Some((cost, if worker { DrawnOn::Worker } else { DrawnOn::Window }))
            }
            None => enhanced
                .as_mut()
                .and_then(Enhanced::take_cost)
                .map(|cost| (cost, DrawnOn::Fallback)),
        };
        let cost = FrameCost {
            total: frame_started.elapsed(),
            stages,
            present,
            requested,
            run: frames_run,
            picture,
        };
        let now = Instant::now();
        if let Some(line) = slow.note(&cost, now) {
            log::warn!("{line}");
        }
        if let Some(line) = slow.report(now) {
            log::info!("{line}");
        }

        pacer.wait(multiplier, requested, env.verify.is_some());
    }
    Ok(())
}

/// Takes a screenshot into the screenshots folder: the picture of the window without the debug panel (`Window`:
/// the game views and the HD layer, drawn as the window draws them, at the window's size in pixels), or the PPU's own
/// frame (`Frame`). Writes the machine's state beside it, named the same, so the picture can be played back with
/// `xtask shots --state-in`. Returns the two paths.
#[allow(clippy::too_many_arguments)]
fn take_screenshot(
    kind: screenshot::Kind,
    video: &mut video::Video,
    views: &[GameView<'_>],
    (scene, overlay): (Option<&ssd_menus::DrawList>, Option<&ssd_menus::DrawList>),
    mut art: Option<&mut hd::Art>,
    machine: &Machine,
    ppu_stale: bool,
    stretch: bool,
) -> std::io::Result<(PathBuf, PathBuf)> {
    let (width, height, rgba) = match kind {
        screenshot::Kind::Window => video
            .capture(views, |ui, games| {
                ui::draw_game(ui, games, frame_layer(scene, overlay, art.as_deref_mut()), stretch);
            })
            .map_err(std::io::Error::other)?,
        screenshot::Kind::Frame => original_frame(machine, ppu_stale),
    };
    let png =
        screenshot::save(&screenshot::folder(), screenshot::Stamp::now(), width, height, &rgba)?;
    let state = screenshot::save_state(&png, &machine.save_state())?;
    Ok((png, state))
}

/// The PPU's own frame as RGBA, with its size. When the PPU left the frame undrawn (`stale`: the picture on show was
/// drawn from the render log) the frame is drawn from the log by the plain renderer, which the parity check shows
/// equal to the PPU's.
fn original_frame(machine: &Machine, stale: bool) -> (u32, u32, Vec<u8>) {
    if stale
        && let Some(log) = machine.frame_log()
        && let Ok(frame) = ssd_hd::Renderer::new().render(log)
    {
        let rgba = frame.pixels.iter().flat_map(|c| [c.r, c.g, c.b, 255]).collect();
        return (frame.width as u32, frame.height as u32, rgba);
    }
    let frame = machine.frame();
    (frame.size.width, frame.size.height, screenshot::frame_rgba(&frame))
}

/// The loop state a window run starts with: the controls of `mode` and of `session`, and the debug tools on with
/// `debug`.
fn window_start_state(mode: HooksArg, session: &Session, debug: bool) -> WindowState {
    let mut state = WindowState::new(Controls::new(
        mode,
        HooksSwitch::Allowed,
        !session.is_active(),
        !session.is_active(),
    ));
    state.controls.debug = debug;
    state
}

/// The touch controls of a window run; they offer the debug tools' Show FPS only with `debug`.
fn window_touch(
    enabled: bool,
    settings: touch::settings::Settings,
    screen: egui::Rect,
    debug: bool,
) -> Touch {
    let mut touch = Touch::new(enabled, settings, screen);
    touch.debug = debug;
    touch
}

/// Windowed run: clock-paced at the core's target rate, audio queued to SDL, video via wgpu.
/// `mode` is the hook mode the machine starts in; `switch` lets the panel change it and run
/// the replays' checks. `debug` turns the debug tools on: the F1 panel, the "F1 debug" pill and the touch Show FPS.
#[allow(clippy::too_many_arguments)]
pub fn run_windowed(
    rom: Vec<u8>,
    hooks: Option<&HookSetup>,
    mode: HooksArg,
    switch: Option<&HookSwitch>,
    ruleset: Ruleset,
    mut session: Session,
    enhanced: Option<HdOptions>,
    start: Start,
    debug: bool,
) -> anyhow::Result<RunStats> {
    enhanced_session(&mut session, ruleset)?;
    let mut layer = hd_layer(&rom, ruleset)?;
    if let Some(hd) = layer.as_mut().filter(|_| session.is_playing()) {
        hd.set_replaying();
    }
    let mut machine = new_machine(rom.clone(), hooks, &mut session)?;
    let result = windowed_loop(
        &mut machine,
        &rom,
        &mut session,
        mode,
        switch,
        layer,
        enhanced,
        start,
        debug,
    );
    let result = flush_after(&mut machine, result);
    session.finish(result)?;
    Ok(machine.hook_stats())
}

// --- UI snapshots ----------------------------------------------------------------------------

/// Runs the frames of `request` headless, applies its script, and writes the debug panel, drawn
/// offscreen by the window's own code, to its PNG. No window is opened and no input is read. The
/// result is that of [`run_headless`], so a verify run still reports its divergences.
pub fn run_ui_snapshot(
    rom: Vec<u8>,
    request: &snapshot::Request<'_>,
    hooks: Option<&HookSetup>,
    switch: Option<&HookSwitch>,
    mode: HooksArg,
    ruleset: Ruleset,
    mut session: Session,
) -> anyhow::Result<RunStats> {
    enhanced_session(&mut session, ruleset)?;
    let mut machine = new_machine(rom.clone(), hooks, &mut session)?;
    let result = draw_snapshot(&mut machine, &rom, request, switch, mode, ruleset, &mut session);
    let result = flush_after(&mut machine, result);
    session.finish(result)?;
    Ok(machine.hook_stats())
}

/// The body of [`run_ui_snapshot`]: the frames, the script, then one frame of the panel.
fn draw_snapshot(
    machine: &mut Machine,
    rom: &[u8],
    request: &snapshot::Request<'_>,
    switch: Option<&HookSwitch>,
    mode: HooksArg,
    ruleset: Ruleset,
    session: &mut Session,
) -> anyhow::Result<()> {
    // The GPU comes first: without an adapter the run fails before it plays a frame.
    let (width, height) = request.size;
    let mut offscreen = snapshot::Offscreen::new(width, height)?;
    let mut state = WindowState::new(Controls::new(
        mode,
        HooksSwitch::Allowed,
        !session.is_active(),
        !session.is_active(),
    ));
    // A UI snapshot pictures the debug panel: the debug tools are on.
    state.controls.debug = true;
    state.hd = hd_layer(rom, ruleset)?;
    state.touch = Touch::new(
        request.touch,
        touch::settings::Settings::default(),
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width as f32, height as f32)),
    );
    state.touch.debug = true;
    if let Some(hd) = state.hd.as_mut().filter(|_| session.is_playing()) {
        hd.set_replaying();
    }
    let mut env = Env { rom, switch, verify: None };
    state.scaled = request.enhanced;
    if let Some(options) = request.enhanced {
        (state.video.scale, state.video.look) = (options.scale, options.look);
    }
    if request.enhanced.is_some() {
        machine.set_line_log(true);
    }
    refresh_controls(&mut state, session, machine, &env);
    refresh_replays(&mut state, &env);
    for _ in 0..request.frames {
        if session.playback_over(machine) {
            log::info!("the replay ends at frame {}", machine.frame_count());
            break;
        }
        // The HD menu runs on every frame, and a replay's host events are made with the frame.
        let (pad, events) = hd_frame(&mut state, machine, session, Joypad::default());
        let stops = Stops { frame_end: true, ..Stops::default() };
        session.run(machine, pad, &events, &mut NullAudio, &stops)?;
        observe(&mut state, machine);
    }
    // The menu lines press live: a replay that has ended hands the buttons to the script.
    if session.playback_over(machine) {
        session.stop_playback();
    }
    for line in request.script {
        run_script_line(line, &mut state, machine, session, &mut env)
            .with_context(|| format!("UI script, line {}", line.number))?;
        refresh_controls(&mut state, session, machine, &env);
    }

    let hd_view = current_hd_view(state.hd_view.as_ref(), machine);
    let picture = match request.enhanced {
        Some(options) if hd_view.is_none() => Enhanced::new(options, rom)
            .picture(machine, state.ssj_watch.lightning_at(machine.frame_count())),
        _ => None,
    };
    let shown_picture = hd_view.map(|view| &view.picture.picture).or(picture.as_ref());
    let registers = machine.cpu_registers();
    // Nothing is paced here, so the header shows the rate the window runs at.
    let status = header_status(machine, &state.history, machine.target_fps() as f32);
    let controls = state.controls;
    let wide = game_frame(current_view(state.view.as_ref(), machine), machine);
    let mut shape_buffer = Vec::new();
    let views = match shown_picture {
        Some(picture) => GameView::enhanced(picture, &machine.frame(), state.video.linear()),
        None => GameView::classic(&wide),
    };
    let shape = state.video.shape();
    let views = [filter::shape(views, shape, &mut shape_buffer)];
    let mut actions = Vec::new();
    let scene = state.hd.as_ref().and_then(|hd| hd.scene(machine.wram()));
    let overlay = state.hd.as_ref().and_then(|hd| hd.overlay(machine.wram(), machine.sram()));
    let (scene, overlay) = with_quit_confirm(&state, scene, overlay);
    let mut plain_art = hd::Art::empty();
    let asking = state.quit_confirm.is_some();
    let mut hd_art = state.hd.as_mut().map(|hd| hd.art(shape));
    if hd_art.is_none() && asking {
        hd_art = Some(&mut plain_art);
    }
    let view = View {
        status,
        controls: &controls,
        registers: &registers,
        recording: state.panel.replays.recording.is_some(),
        verify_labels: None,
        machine,
        symbols: env.symbols(),
        history: &state.history,
        touch: state.touch.enabled.then_some(&state.touch),
    };
    let pixels = offscreen.draw(&views, |ui, games| {
        let layer = frame_layer(scene.as_ref(), overlay.as_ref(), hd_art.as_deref_mut());
        ui::draw(ui, &view, games, &mut state.panel, &mut actions, layer);
    })?;
    // Without input the panel asks for nothing: an action here would be a bug in the drawing.
    debug_assert!(actions.is_empty(), "drawing asks for nothing without input");
    png::write_rgba(request.out, width, height, &pixels)
        .with_context(|| format!("writing {}", request.out.display()))?;
    log::info!("UI snapshot written to {}", request.out.display());
    Ok(())
}

/// Carries out one line of a UI script, with the actions the window's controls and panel make.
fn run_script_line(
    line: &Line,
    state: &mut WindowState,
    machine: &mut Machine,
    session: &mut Session,
    env: &mut Env<'_>,
) -> anyhow::Result<()> {
    match &line.command {
        Command::Panel { open } => {
            if state.panel.open != *open {
                act(PanelAction::TogglePanel, state, machine, session, env);
            }
        }
        Command::Tab(tab) => act(PanelAction::OpenTab(*tab), state, machine, session, env),
        Command::Pause => {
            if !state.controls.paused {
                act(PanelAction::TogglePause, state, machine, session, env);
            }
        }
        Command::Step(kind) => {
            // The step buttons are disabled while the game runs: the script says so too.
            if state.controls.paused {
                act(PanelAction::Debug(DebugAction::Step(*kind)), state, machine, session, env);
            } else {
                log::warn!("UI script, line {}: a step needs the game paused", line.number);
            }
        }
        Command::Frames(count) => {
            snapshot_frames(*count, state, machine, session, env)?;
        }
        Command::Select(target) => {
            let address =
                tab_memory::parse_target(target, env.symbols()).map_err(anyhow::Error::msg)?;
            state.panel.memory.select(address, target.as_str());
        }
        Command::Breakpoint(target) => {
            // The field's own check first: a name that is nothing is an error, not a notice.
            let empty = Symbols::default();
            let symbols = env.symbols().unwrap_or(&empty);
            debugger::parse_target(target, symbols).map_err(anyhow::Error::msg)?;
            state.panel.debug.target.clone_from(target);
            act(PanelAction::Debug(DebugAction::AddBreakpoint), state, machine, session, env);
        }
        Command::Mode { routine, mode } => {
            let key = routine_key(routine, env.symbols())?;
            act(PanelAction::SetRoutineMode { key, mode: *mode }, state, machine, session, env);
        }
        Command::Quit(press) => {
            let route = if press.is_some() { KeyRoute::Game } else { KeyRoute::Quit };
            if quit_confirm::key(&mut state.quit_confirm, route, *press) == quit_confirm::Step::Quit
            {
                log::info!("UI script, line {}: Yes would end the run here", line.number);
            }
        }
        Command::Touch(text) => {
            anyhow::ensure!(state.touch.enabled, "touch lines need --touch-ui");
            state.touch.script(text).map_err(anyhow::Error::msg)?;
        }
        Command::Menu(menu) => {
            anyhow::ensure!(state.hd.is_some(), "menu lines need --mode enhanced");
            // A menu line presses live: a replay still playing stops here, and the script goes on
            // with the buttons of the menu.
            if session.is_playing() {
                log::info!(
                    "UI script, line {}: the replay stops; the menu presses live",
                    line.number
                );
                session.stop_playback();
            }
            match *menu {
                MenuCommand::Open => {
                    if let Some(hd) = state.hd.as_mut() {
                        hd.open();
                    }
                    run_hd_choice(state, machine, session)?;
                }
                MenuCommand::Key(key) => {
                    if let Some(hd) = state.hd.as_mut() {
                        hd.key(key, machine.wram());
                    }
                    run_hd_choice(state, machine, session)?;
                }
                MenuCommand::Close => {
                    if let Some(hd) = state.hd.as_mut() {
                        hd.close_menu();
                    }
                    run_hd_choice(state, machine, session)?;
                }
                MenuCommand::Tab(tab) => {
                    if let Some(hd) = state.hd.as_mut() {
                        hd.set_tab(tab);
                    }
                    run_hd_choice(state, machine, session)?;
                }
                MenuCommand::Press(capture) => {
                    let hd = state.hd.as_mut().context("menu lines need --mode enhanced")?;
                    anyhow::ensure!(
                        hd.listening().is_some(),
                        "line {}: no key box waits for a key",
                        line.number
                    );
                    hd.capture(capture);
                }
                MenuCommand::BattleCheats => {
                    let battle = ssd_game::gameplay::in_battle(machine.wram());
                    let hd = state.hd.as_mut().context("menu lines need --mode enhanced")?;
                    anyhow::ensure!(
                        battle && hd.open_battle_cheats(),
                        "line {}: the Cheats open over a battle only (in a battle: {battle})",
                        line.number
                    );
                }
                MenuCommand::Show(show) => {
                    let wanted = match show {
                        MenuShow::Field => Screen::Command,
                        MenuShow::Menu => Screen::Menu,
                        MenuShow::Items => Screen::Items,
                        MenuShow::Cheats => Screen::Cheats,
                        MenuShow::Options => Screen::Options,
                        MenuShow::Mods => Screen::Mods,
                        MenuShow::Status => Screen::Status,
                        MenuShow::Cards => Screen::Cards,
                        MenuShow::Text => Screen::Text,
                        MenuShow::Save => Screen::Save,
                        MenuShow::Order => Screen::Order,
                    };
                    let hd = state.hd.as_ref().context("menu lines need --mode enhanced")?;
                    let curtain = hd.driver().curtain();
                    let on_screen = curtain == Curtain::Up && hd.driver().nav().screen() == wanted;
                    anyhow::ensure!(
                        on_screen,
                        "the HD shows {curtain:?} on {:?}, not {wanted:?} (the last choice: {:?})",
                        hd.driver().nav().screen(),
                        hd.driver().last_outcome()
                    );
                }
            }
        }
    }
    Ok(())
}

/// Frames a menu line's choice may run before it counts as stuck.
const HD_SNAPSHOT_LIMIT: u32 = 20_000;

/// Runs the presses of the HD choice that a menu line started, to their end, with the picture not
/// drawn (the curtain hides the game meanwhile). Nothing runs when no choice is under way.
fn run_hd_choice(
    state: &mut WindowState,
    machine: &mut Machine,
    session: &mut Session,
) -> anyhow::Result<()> {
    for _ in 0..HD_SNAPSHOT_LIMIT {
        if !state.hd.as_ref().is_some_and(hd::Hd::busy) {
            return Ok(());
        }
        let (pad, events) = hd_frame(state, machine, session, Joypad::default());
        let stops = Stops { frame_end: true, ..Stops::default() };
        session.run(machine, pad, &events, &mut NullAudio, &stops)?;
        observe(state, machine);
    }
    anyhow::bail!("the HD menu's choice did not end in {HD_SNAPSHOT_LIMIT} frames")
}

/// What the window does for `action`, for a snapshot: no live input and no sound.
fn act(
    action: PanelAction,
    state: &mut WindowState,
    machine: &mut Machine,
    session: &mut Session,
    env: &mut Env<'_>,
) {
    apply_action(action, state, machine, session, env, Joypad::default(), &mut NullAudio);
}

/// Runs up to `count` frames as the window does while it runs: a breakpoint ends the run early
/// and pauses the machine. A script runs its frames even while the machine is paused, since it
/// asks for them.
fn snapshot_frames(
    count: u64,
    state: &mut WindowState,
    machine: &mut Machine,
    session: &mut Session,
    env: &Env<'_>,
) -> anyhow::Result<()> {
    let stops = Stops { frame_end: true, breakpoint: true, ..Stops::default() };
    for _ in 0..count {
        // The HD menu, when it has the curtain, presses for the frame; otherwise nothing is pressed.
        let (pad, events) = hd_frame(state, machine, session, Joypad::default());
        let stopped = session.run(machine, pad, &events, &mut NullAudio, &stops)?;
        if let Stopped::Breakpoint { key } = stopped {
            breakpoint_stop(state, machine, env.symbols(), key);
            break;
        }
        observe(state, machine);
    }
    Ok(())
}

/// The key of a routine of the symbols file in the Routines tab: its address.
fn routine_key(name: &str, symbols: Option<&Symbols>) -> anyhow::Result<u32> {
    let symbols =
        symbols.context("the symbols file could not be read: there is no routine to set")?;
    symbols
        .routines()
        .iter()
        .find(|routine| routine.name == name)
        .map(|routine| routine.address)
        .with_context(|| format!("no routine named `{name}` in the symbols file"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{SharedStore, sram_counter_machine, sram_counter_rom};
    use crate::ui_script;
    use ssd_oracle::{FrameStats, FsSaveStore, MemorySaveStore};

    #[test]
    fn no_frame_runs_while_esc_asks_and_the_game_goes_on_once_it_closes() {
        let controls = Controls::new(HooksArg::Original, HooksSwitch::Allowed, true, true);
        let mut state = WindowState::new(controls);
        assert!(game_runs(&state, false));
        assert!(!game_runs(&state, true), "a verification paces itself");
        state.quit_confirm = Some(QuitConfirm::default());
        assert!(!game_runs(&state, false), "held while it asks");
        state.step_once = true;
        assert!(!game_runs(&state, false), "not even a step");
        state.quit_confirm = None;
        assert!(game_runs(&state, false), "closed, it goes on");
        state.step_once = false;
        state.controls.paused = true;
        assert!(!game_runs(&state, false), "the pause is the player's own");
    }

    #[test]
    fn esc_s_question_is_drawn_over_the_menu_or_the_field_and_only_while_it_asks() {
        let controls = Controls::new(HooksArg::Original, HooksSwitch::Allowed, true, true);
        let mut state = WindowState::new(controls);
        let menu = || {
            let mut list = ssd_menus::DrawList::new();
            list.text(
                1.0,
                1.0,
                "Menu",
                30.0,
                ssd_menus::layout::palette::BLACK,
                ssd_menus::Align::Left,
            );
            list
        };
        assert_eq!(with_quit_confirm(&state, None, None), (None, None), "closed: nothing");
        assert_eq!(with_quit_confirm(&state, Some(menu()), None), (Some(menu()), None));
        state.quit_confirm = Some(QuitConfirm::default());
        // Classic (no HD): a list of its own over the field, in English.
        let (scene, overlay) = with_quit_confirm(&state, None, None);
        assert!(scene.is_none());
        let texts: Vec<String> =
            overlay.expect("an overlay").texts().map(|(_, _, text)| text.to_owned()).collect();
        assert_eq!(texts, ["Quit the game?", "No", "Yes"]);
        // Over a menu: on top of it, the menu kept.
        let (scene, overlay) = with_quit_confirm(&state, Some(menu()), None);
        assert!(overlay.is_none());
        let texts: Vec<String> =
            scene.expect("the menu").texts().map(|(_, _, text)| text.to_owned()).collect();
        assert_eq!(texts, ["Menu", "Quit the game?", "No", "Yes"]);
    }

    #[test]
    fn a_change_of_the_page_reaches_the_picture_and_the_window_but_a_flag_keeps_its_choice() {
        let controls = Controls::new(HooksArg::Original, HooksSwitch::Allowed, true, true);
        let mut state = WindowState::new(controls);
        let start = crate::options::video::start(&Values::new(), Some(3), None);
        (state.video, state.video_saved) = (start.shown, start.saved);
        state.scaled = Some(state.video.hd());
        let mut values = Values::new();
        values.set("video.scanlines", 2);
        state.video_changed(&values);
        assert!(std::mem::take(&mut state.video_apply), "the window is told");
        assert_eq!(state.video.filters.scanlines, 2);
        assert_eq!(state.scaled.map(|o| o.scale), Some(3), "the flag's scale stays");
        values.set("video.scale", 4);
        state.video_changed(&values);
        assert_eq!(
            state.scaled.map(|o| o.scale),
            Some(6),
            "the page's scale shows once it is changed"
        );
        values.remove("video.scale");
        state.video_changed(&values);
        assert_eq!(state.scaled.map(|o| o.scale), Some(4), "a revert goes back to the default");
    }

    #[test]
    fn a_replay_that_plays_writes_no_sidecar_file() {
        use ssd_game::menu::driver::Frame;
        use ssd_menus::cheats::{Cheats, SavedCheats};
        let saved = SavedCheats { slot: 0, checksum: 1, marked: false, cheats: Cheats::neutral() };
        let frame = Frame {
            pad: Joypad::default(),
            events: Vec::new(),
            saved: None,
            saved_cheats: Some(saved),
            saved_mods: Some((0, 1)),
            saved_options: true,
            settings: None,
            video: None,
        };
        assert_eq!(
            sidecar_writes(&frame, false),
            (false, true, true),
            "a live run saves its cheats and mods"
        );
        assert_eq!(
            sidecar_writes(&frame, true),
            (false, false, false),
            "a replay keeps the sidecars"
        );
    }

    /// A machine that has run long enough for its SRAM to change, with that change not yet in
    /// its store (the core only persists every 30 frames).
    fn machine_with_unsaved_sram() -> (Machine, SharedStore) {
        let (mut machine, store) = sram_counter_machine();
        for _ in 0..10 {
            machine.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        }
        assert_ne!(store.get("sav").map(|sav| sav[0]), Some(machine.sram().unwrap()[0]));
        (machine, store)
    }

    fn assert_saved(machine: &Machine, store: &SharedStore) {
        let sav = store.get("sav").expect("SRAM persisted");
        assert_eq!(sav[0], machine.sram().unwrap()[0]);
    }

    #[test]
    fn a_failed_run_still_flushes_sram_and_its_own_error_wins() {
        let (mut machine, store) = machine_with_unsaved_sram();
        let error =
            flush_after(&mut machine, Err::<(), _>(anyhow::anyhow!("window lost"))).unwrap_err();
        assert_eq!(error.to_string(), "window lost");
        assert_saved(&machine, &store);
    }

    #[test]
    fn a_good_run_flushes_sram_and_returns_its_value() {
        let (mut machine, store) = machine_with_unsaved_sram();
        assert_eq!(flush_after(&mut machine, Ok(7)).unwrap(), 7);
        assert_saved(&machine, &store);
    }

    #[test]
    fn a_failing_flush_is_an_error_after_a_good_run_and_not_after_a_failed_one() {
        let (mut machine, store) = machine_with_unsaved_sram();
        store.refuse_writes(true);
        let error = flush_after(&mut machine, Ok(())).unwrap_err();
        assert!(format!("{error:#}").contains("saving SRAM"), "{error:#}");
        assert!(format!("{error:#}").contains("disk full"), "{error:#}");
        let error = flush_after(&mut machine, Err::<(), _>(anyhow::anyhow!("boom"))).unwrap_err();
        assert_eq!(error.to_string(), "boom");
    }

    #[test]
    fn a_window_run_has_the_debug_tools_it_is_started_with() {
        let session = Session::default();
        for debug in [false, true] {
            let state = window_start_state(HooksArg::Original, &session, debug);
            assert_eq!(state.controls.debug, debug, "the controls");
            assert!(state.controls.load_allowed && state.controls.freezes_allowed);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(932.0, 430.0));
            let touch = window_touch(true, touch::settings::Settings::default(), screen, debug);
            assert_eq!(touch.debug, debug, "the touch controls");
        }
    }

    fn window_state(hooks_switch: HooksSwitch) -> WindowState {
        WindowState::new(Controls::new(HooksArg::Original, hooks_switch, true, true))
    }

    /// A complete log of 224 lines with `lines` set to `regs` and nothing else.
    fn log_of(regs: ssd_oracle::LineRegs, lines: u16) -> ssd_oracle::FrameLog {
        ssd_oracle::FrameLog {
            complete: true,
            height: 224,
            h_hi_res: false,
            v_hi_res: false,
            entries: (1..=lines)
                .map(|scanline| ssd_oracle::LineEntry { scanline, regs, ..Default::default() })
                .collect(),
            cgram_start: vec![0; 256],
            cgram_writes: Vec::new(),
            vram: vec![0; 32768],
            vram_writes: Vec::new(),
            oam_low: vec![0; 512],
            oam_high: vec![0; 32],
            oam_writes: Vec::new(),
        }
    }

    #[test]
    fn a_frame_is_drawable_when_the_enhanced_renderer_does_not_refuse_it() {
        use ssd_oracle::LineRegs;
        let drawn = |log: &ssd_oracle::FrameLog| ssd_hd::Renderer::new().render(log).is_ok();
        let mut cases = Vec::new();
        for bg_mode in 0..=7 {
            cases.push(log_of(LineRegs { bg_mode, ..LineRegs::default() }, 224));
        }
        for tweak in [
            |regs: &mut LineRegs| regs.interlaced = true,
            |regs: &mut LineRegs| regs.pseudo_h_hi_res = true,
            |regs: &mut LineRegs| regs.pseudo_obj_hi_res = true,
        ] {
            let mut regs = LineRegs::default();
            tweak(&mut regs);
            // (from the middle of the frame on: the lines before are plain)
            let mut log = log_of(LineRegs::default(), 100);
            log.entries.extend(log_of(regs, 224).entries.split_off(100));
            cases.push(log);
        }
        let mut incomplete = log_of(LineRegs::default(), 224);
        incomplete.complete = false;
        let mut h_hi_res = log_of(LineRegs::default(), 224);
        h_hi_res.h_hi_res = true;
        let mut v_hi_res = log_of(LineRegs::default(), 224);
        v_hi_res.v_hi_res = true;
        cases.extend([incomplete, h_hi_res, v_hi_res, log_of(LineRegs::default(), 0)]);
        for (at, log) in cases.iter().enumerate() {
            assert_eq!(drawable(Some(log)), drawn(log), "case {at}");
        }
        assert!(!drawable(None), "no log, no picture");
    }

    fn symbols_switch() -> HookSwitch {
        let symbols = concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml");
        HookSwitch::load(Some(Path::new(symbols)), None).unwrap()
    }

    /// A machine that has played a frame in Rust for `rng_next_a`'s share (with a measured mean of
    /// 11, if `mean` is some), and a window with one frame of 90 original and 10 Rust instructions.
    fn header_fixture(mean: Option<f64>) -> (Machine, FrameHistory) {
        let (mut machine, _store) = sram_counter_machine();
        symbols_switch().apply(&mut machine, HooksArg::Rust).unwrap();
        if let Some(mean) = mean {
            machine.set_mean_instructions(&[("rng_next_a".to_owned(), mean)]);
        }
        let mut history = FrameHistory::default();
        let stats = FrameStats {
            frame: 1,
            original_instructions: 90,
            rust_instructions_estimated: 10,
            ..FrameStats::default()
        };
        history.record(Some(stats), 0);
        (machine, history)
    }

    #[test]
    fn the_header_shows_a_dash_while_the_means_of_the_routines_in_rust_are_missing() {
        let (machine, history) = header_fixture(None);
        let status = header_status(&machine, &history, 60.0);
        assert!(status.means_missing);
        assert_eq!(status.rust_percent, None, "no figure without the means");
        let (machine, history) = header_fixture(Some(11.0));
        let status = header_status(&machine, &history, 60.0);
        assert!(!status.means_missing);
        assert_eq!(status.rust_percent, Some(10.0), "10 Rust of 100 instructions of the window");
    }

    #[test]
    fn the_means_survive_a_table_that_is_dropped_and_installed_again() {
        let switch = symbols_switch();
        let (mut machine, _store) = sram_counter_machine();
        switch.apply(&mut machine, HooksArg::Rust).unwrap();
        machine.set_mean_instructions(&[("rng_next_a".to_owned(), 11.0)]);
        let mut state = window_state(HooksSwitch::Allowed);
        keep_means(&mut state, &machine);
        // Original code has no table, and a table without counters starts again at zero.
        switch.apply(&mut machine, HooksArg::Original).unwrap();
        switch.apply(&mut machine, HooksArg::Rust).unwrap();
        let mean = |machine: &Machine| {
            machine
                .hook_stats()
                .iter()
                .find(|(name, _)| name == "rng_next_a")
                .unwrap()
                .1
                .mean_instructions
        };
        assert_eq!(mean(&machine), 0.0, "the counters of the dropped table are gone");
        after_table_change(&mut state, &mut machine);
        assert_eq!(mean(&machine), 11.0, "the kept mean is back");
    }

    /// A replay of three frames on [`sram_counter_rom`], written to `path`.
    fn record_three_frames(path: &Path, rom: &[u8]) {
        let mut recording = Session::new(None, Some(path), rom).unwrap();
        let mut machine =
            recording.power_on(rom.to_vec(), || Ok(Box::new(MemorySaveStore::default()))).unwrap();
        for _ in 0..3 {
            recording.run_frame(&mut machine, Joypad::default(), &mut NullAudio).unwrap();
        }
        recording.finish(Ok(())).unwrap();
    }

    #[test]
    fn loading_waits_for_a_replay_to_end_and_the_freezes_for_it_to_stop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.ssdreplay");
        let rom = sram_counter_rom();
        record_three_frames(&path, &rom);
        let mut session = Session::new(Some(&path), None, &rom).unwrap();
        let mut machine =
            session.power_on(rom.clone(), || Ok(Box::new(MemorySaveStore::default()))).unwrap();
        let env = Env { rom: &rom, switch: None, verify: None };
        let mut state = window_state(HooksSwitch::Allowed);
        state.panel.replays.playing = Some("run".to_owned());
        state.replay_over_logged = true;

        refresh_controls(&mut state, &session, &machine, &env);
        assert!(!state.controls.load_allowed && !state.controls.freezes_allowed, "playing");
        for _ in 0..3 {
            session.run_frame(&mut machine, Joypad::default(), &mut NullAudio).unwrap();
        }
        refresh_controls(&mut state, &session, &machine, &env);
        assert!(state.controls.load_allowed, "the replay has ended: loading is back");
        assert!(!state.controls.freezes_allowed, "freezes wait for Stop replay");

        stop_playback(&mut state, &mut session);
        refresh_controls(&mut state, &session, &machine, &env);
        assert!(state.controls.freezes_allowed, "stopped: freezes are back");
        assert_eq!(state.panel.replays.playing, None);
        assert!(!state.replay_over_logged, "the next end is logged again");
        stop_playback(&mut state, &mut session);
    }

    #[test]
    fn a_recording_starts_from_the_sram_the_machine_it_replaces_has() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        let rom = sram_counter_rom();
        // The old machine saves to the file the recording's machine loads its SRAM from.
        let store = FsSaveStore::new(paths::saves_dir(), "ssd").unwrap();
        let mut machine = Machine::new(rom.clone(), Box::new(store)).unwrap();
        for _ in 0..10 {
            machine.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        }
        let sram = machine.sram().unwrap().to_vec();
        let mut state = window_state(HooksSwitch::Allowed);
        let mut session = Session::default();
        begin_recording(
            &mut state,
            &mut machine,
            &mut session,
            &rom,
            &dir.path().join("run.ssdreplay"),
        );
        assert!(session.is_recording());
        assert_eq!(machine.sram().unwrap(), &sram[..], "the SRAM the old machine had, saved");
        session.finish(Ok(())).unwrap();
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    #[test]
    fn actions_change_the_loop_state_and_reach_the_machine() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        paths::set_persistent(true);
        let switch = HookSwitch::load(
            Some(std::path::Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../re/symbols.toml"
            ))),
            None,
        )
        .unwrap();
        let rom = sram_counter_rom();
        let (mut machine, _store) = sram_counter_machine();
        let mut session = Session::default();
        let mut env = Env { rom: &rom, switch: Some(&switch), verify: None };
        let mut state = window_state(HooksSwitch::Allowed);
        let mut audio = NullAudio;
        let mut act = |state: &mut WindowState, machine: &mut Machine, action| {
            apply_action(
                action,
                state,
                machine,
                &mut session,
                &mut env,
                Joypad::default(),
                &mut audio,
            );
        };

        act(&mut state, &mut machine, PanelAction::NextFrame);
        assert!(!state.step_once, "nothing to step while running");
        act(&mut state, &mut machine, PanelAction::TogglePause);
        act(&mut state, &mut machine, PanelAction::NextFrame);
        assert!(state.controls.paused && state.step_once);

        act(&mut state, &mut machine, PanelAction::TogglePanel);
        assert!(!state.panel.open, "no panel without the debug tools");
        state.controls.debug = true;
        act(&mut state, &mut machine, PanelAction::TogglePanel);
        assert!(state.panel.open);

        act(&mut state, &mut machine, PanelAction::SetHooks(HooksArg::Rust));
        assert_eq!(state.controls.hooks, HooksArg::Rust);
        assert!(machine.hook_table().is_some());
        act(&mut state, &mut machine, PanelAction::SetHooks(HooksArg::Original));
        assert!(machine.hook_table().is_none());

        // Slot 2: a saved state loads again.
        machine.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        act(&mut state, &mut machine, PanelAction::SelectSlot(2));
        act(&mut state, &mut machine, PanelAction::SaveState);
        assert!(hotkeys::state_path(2).exists());
        machine.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        assert_eq!(machine.frame_count(), 2);
        act(&mut state, &mut machine, PanelAction::LoadState);
        assert_eq!(machine.frame_count(), 1, "the state of slot 2 is back");
        paths::set_persistent(false);
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    #[test]
    fn a_run_that_does_not_persist_keeps_its_states_in_memory() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        paths::set_persistent(false);
        let (mut machine, _store) = sram_counter_machine();
        let mut state = window_state(HooksSwitch::Allowed);
        let mut session = Session::default();
        let mut env = Env { rom: &[], switch: None, verify: None };
        let mut audio = NullAudio;
        machine.run_frame(Joypad::default(), &mut audio).unwrap();
        apply_action(
            PanelAction::SaveState,
            &mut state,
            &mut machine,
            &mut session,
            &mut env,
            Joypad::default(),
            &mut audio,
        );
        assert!(!paths::states_dir().exists(), "no state file, and no states folder");
        machine.run_frame(Joypad::default(), &mut audio).unwrap();
        assert_eq!(machine.frame_count(), 2);
        apply_action(
            PanelAction::LoadState,
            &mut state,
            &mut machine,
            &mut session,
            &mut env,
            Joypad::default(),
            &mut audio,
        );
        assert_eq!(machine.frame_count(), 1, "the state saved in this run is back");
        assert!(!paths::states_dir().exists());
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    #[test]
    fn a_screenshot_is_asked_for_by_a_window_and_ignored_by_a_run_that_does_not_persist() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let shots = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var(screenshot::DIR_ENV, shots.path().join("pictures")) };
        let (mut machine, _store) = sram_counter_machine();
        let mut state = window_state(HooksSwitch::Allowed);
        let mut session = Session::default();
        let mut env = Env { rom: &[], switch: None, verify: None };
        let mut audio = NullAudio;
        let mut press = |state: &mut WindowState, action| {
            apply_action(
                action,
                state,
                &mut machine,
                &mut session,
                &mut env,
                Joypad::default(),
                &mut audio,
            );
        };
        paths::set_persistent(false);
        press(&mut state, PanelAction::Screenshot);
        press(&mut state, PanelAction::ScreenshotFrame);
        assert_eq!(state.shot, None, "a headless, snapshot or replay run takes no screenshot");
        assert!(!shots.path().join("pictures").exists());
        paths::set_persistent(true);
        press(&mut state, PanelAction::Screenshot);
        assert_eq!(state.shot, Some(screenshot::Kind::Window));
        press(&mut state, PanelAction::ScreenshotFrame);
        assert_eq!(state.shot, Some(screenshot::Kind::Frame));
        paths::set_persistent(false);
        assert!(!shots.path().join("pictures").exists(), "asking writes nothing: the loop does");
        unsafe { std::env::remove_var(screenshot::DIR_ENV) };
    }

    #[test]
    fn a_locked_hook_mode_stays_as_it_is() {
        let (mut machine, _store) = sram_counter_machine();
        let mut state = window_state(HooksSwitch::Locked("recording"));
        let mut session = Session::default();
        let mut env = Env { rom: &[], switch: None, verify: None };
        let mut audio = NullAudio;
        apply_action(
            PanelAction::SetHooks(HooksArg::Rust),
            &mut state,
            &mut machine,
            &mut session,
            &mut env,
            Joypad::default(),
            &mut audio,
        );
        assert_eq!(state.controls.hooks, HooksArg::Original);
        assert!(machine.hook_table().is_none());
    }

    #[test]
    fn the_pacer_runs_the_speed_on_time_and_what_it_owes_when_behind_up_to_what_a_batch_holds() {
        let frame = Duration::from_millis(16);
        let mut pacer = Pacer::new(frame);
        let start = pacer.next;
        assert_eq!(pacer.frames(1, start), 1, "speed 1 on time is a frame");
        // a screen at 24 Hz: 42 ms a window frame, two frames owed on top of this one
        assert_eq!(pacer.frames(1, start + Duration::from_millis(42)), 3);
        assert_eq!(
            pacer.frames(1, start + Duration::from_secs(1)),
            CATCH_UP,
            "never more than that"
        );
        assert_eq!(pacer.frames(4, start), 4, "on time: the speed's frames");
        // 48 ms behind at speed 4 (4 ms a frame): 12 more frames are owed; 2 ms a frame lets 25 in
        // a batch
        assert_eq!(pacer.frames(4, start + Duration::from_millis(48)), 16);
        assert_eq!(pacer.frames(8, start + Duration::from_secs(10)), 25, "never more than fit");
        // A frame that costs 20 ms still gets the speed's frames: the speed is not the batch's to cut
        pacer.cost = Duration::from_millis(20);
        assert_eq!(pacer.frames(4, start + Duration::from_secs(1)), 4);
        // Frames that cost 0.1 ms: the batch stops at the cap
        pacer.cost = Duration::from_micros(100);
        assert_eq!(pacer.frames(8, start + Duration::from_secs(10)), MAX_BATCH);
    }

    /// The game frames a pacer of the game's `frame` runs at speed 1 in `time` on a vsync screen of `refresh` a
    /// refresh with a machine that keeps up (a window frame starts at the first refresh after the last one's and after
    /// the pacer's next start), and the most a window frame ran.
    fn paced(
        tempo: Option<(u32, u32)>,
        frame: Duration,
        refresh: Duration,
        time: Duration,
    ) -> (u32, u32) {
        let mut pacer = Pacer::new(frame);
        pacer.tempo = tempo;
        let start = pacer.next;
        let (mut now, mut total, mut most) = (start, 0, 0);
        while now < start + time {
            let requested = pacer.frames(1, now);
            pacer.advance(1, requested);
            total += requested;
            most = most.max(requested);
            let earliest = (now + refresh).max(pacer.next) - start;
            let refreshes = earliest.as_nanos().div_ceil(refresh.as_nanos());
            now = start + refresh * u32::try_from(refreshes).unwrap();
        }
        (total, most)
    }

    #[test]
    fn the_battle_speed_runs_three_frames_of_the_game_to_two_refreshes() {
        let frame = Duration::from_secs_f64(1.0 / 60.0988);
        let ten = Duration::from_secs(10);
        // 10 s on a screen of the game's refresh (601 refreshes): 901 frames of the game, one or two a window frame.
        let tempo = Some(ssd_game::gameplay::BATTLE_TEMPO);
        let (frames, most) = paced(tempo, frame, frame, ten);
        assert!((900..=902).contains(&frames), "{frames} frames in 10 s");
        assert_eq!(most, 2, "never more than two frames in a window frame");
        // Off: a frame a refresh, as ever.
        assert_eq!(paced(None, frame, frame, ten), (601, 1));
        // A 120 Hz screen: the same frames of the game, never two in a window frame.
        let (frames, most) = paced(tempo, frame, frame / 2, ten);
        assert!((900..=902).contains(&frames) && most == 1, "{frames} frames, {most} at most");
        assert_eq!(paced(None, frame, frame / 2, ten), (601, 1));
    }

    #[test]
    fn the_battle_speed_2x_runs_two_frames_of_the_game_to_each_refresh() {
        let frame = Duration::from_secs_f64(1.0 / 60.0988);
        let ten = Duration::from_secs(10);
        let tempo = Some(ssd_game::gameplay::BATTLE_TEMPO_2X);
        // 10 s on a screen of the game's refresh (601 refreshes): 1202 frames of the game, two every window frame.
        let (frames, most) = paced(tempo, frame, frame, ten);
        assert!((1201..=1203).contains(&frames), "{frames} frames in 10 s");
        assert_eq!(most, 2, "never more than two frames in a window frame");
        // A 120 Hz screen: the same frames of the game, one a window frame.
        let (frames, most) = paced(tempo, frame, frame / 2, ten);
        assert!((1201..=1203).contains(&frames) && most == 1, "{frames} frames, {most} at most");
        // The pacer's frame of the game is half a refresh
        let mut pacer = Pacer::new(frame);
        pacer.tempo = tempo;
        assert_eq!(pacer.game_frame_time(), frame / 2);
        pacer.tempo = Some(ssd_game::gameplay::BATTLE_TEMPO);
        assert_eq!(pacer.game_frame_time(), frame * 2 / 3);
    }

    #[test]
    fn the_battle_speed_gives_a_frame_two_thirds_of_the_sound_and_off_gives_it_back() {
        struct Count(u64);
        impl AudioSink for Count {
            fn push(&mut self, _left: f32, _right: f32) {
                self.0 += 1;
            }
        }
        let (mut machine, _) = sram_counter_machine();
        let mut pacer = Pacer::new(Duration::from_millis(16));
        let mut state = window_state(HooksSwitch::Allowed);
        let samples = |machine: &mut Machine| {
            let mut count = Count(0);
            for _ in 0..300 {
                machine.run_frame(Joypad::default(), &mut count).unwrap();
            }
            count.0
        };
        samples(&mut machine); // the resampler's start
        let before = samples(&mut machine);
        set_battle_tempo(&mut state, &mut machine, &mut pacer, Some((3, 2)));
        assert_eq!(machine.apu_clock_ratio(), (2, 3));
        assert!(pacer.tempo == Some((3, 2)) && state.whole_frames);
        let tempo = samples(&mut machine);
        set_battle_tempo(&mut state, &mut machine, &mut pacer, None);
        assert_eq!(machine.apu_clock_ratio(), (1, 1));
        assert!(pacer.tempo.is_none() && !state.whole_frames);
        let after = samples(&mut machine);
        set_battle_tempo(&mut state, &mut machine, &mut pacer, Some((2, 1)));
        assert_eq!(machine.apu_clock_ratio(), (1, 2));
        assert!(pacer.tempo == Some((2, 1)) && state.whole_frames);
        let double = samples(&mut machine);
        set_battle_tempo(&mut state, &mut machine, &mut pacer, None);
        let again = samples(&mut machine);
        assert!(before > 200 * 300, "the sound of 300 frames: {before}");
        assert!(
            tempo * 3 / 2 >= before - 3 && tempo * 3 / 2 <= before + 3,
            "2/3: {tempo} of {before}"
        );
        assert!(after.abs_diff(before) <= 2, "back to all of it: {after} of {before}");
        assert!(double * 2 >= before - 3 && double * 2 <= before + 3, "1/2: {double} of {before}");
        assert!(again.abs_diff(before) <= 2, "back to all of it after 2x: {again} of {before}");
    }

    /// A window frame of `total` ms whose picture came back from the background, with made-up stages.
    fn slow_cost(total: u64) -> FrameCost {
        let ms = Duration::from_millis;
        let picture = ssd_hd::DrawCost {
            total: ms(290),
            begin_bg: ms(2),
            find_poses: ms(3),
            draw: ms(20),
            art_compose: ms(5),
            loads: ms(250),
            requests: 9,
            bg_loads: 2,
            pictures: 1,
            objects: 3,
            variants: 1,
            planes: 0,
            load_bytes: 123_456,
            bg_slots: 4,
            plane: true,
            poses: 6,
            bg_evicted: 1,
        };
        FrameCost {
            total: ms(total),
            stages: Stages { driver: ms(1), emulate: ms(4), history: ms(0), hd: ms(295) },
            present: ms(8),
            requested: 2,
            run: 2,
            picture: Some((picture, DrawnOn::Worker)),
        }
    }

    #[test]
    fn a_window_frame_over_100_ms_is_logged_with_its_stages_and_one_under_is_not() {
        assert_eq!(
            slow_frame_line(&slow_cost(312)).as_deref(),
            Some(
                "slow window frame: 312.0 ms (emulate 4.0, driver 1.0, history 0.0, hd 295.0, present 8.0, other \
                 4.0 ms; 2/2 frames); picture drawn in the background: 290.0 ms (begin_bg 2.0, find_poses 3.0, draw 20.0, art \
                 5.0 ms; loads 250.0 ms: requests 9, received 7: 2 bg, 1 pictures, 3 objects, 1 variants, 0 planes, \
                 123456 bytes); 4 bg slots (one the Mode 7 plane), 6 poses, bg art evicted 1"
            )
        );
        assert_eq!(slow_frame_line(&slow_cost(50)), None, "50 ms is not slow");
        assert_eq!(slow_frame_line(&slow_cost(100)), None, "100 ms is not slow either");
        let bare = FrameCost { picture: None, ..slow_cost(150) };
        assert!(slow_frame_line(&bare).unwrap().ends_with("; no picture drawn"));
    }

    #[test]
    fn slow_window_frames_are_logged_two_a_second_and_summed_up_every_5_seconds() {
        let start = Instant::now();
        let mut slow = SlowFrames::new(start);
        let at = |ms: u64| start + Duration::from_millis(ms);
        let logged: Vec<bool> = [100, 200, 300, 400, 1150, 1200]
            .into_iter()
            .map(|t| slow.note(&slow_cost(150 + t / 100), at(t)).is_some())
            .collect();
        assert_eq!(logged, [true, true, false, false, true, true], "two a second");
        assert!(slow.note(&slow_cost(40), at(1300)).is_none(), "a quick one is not counted");
        assert_eq!((slow.count, slow.slowest), (6, Duration::from_millis(162)));
        assert_eq!(slow.report(at(4000)), None, "not 5 s yet");
        assert_eq!(
            slow.report(at(5000)).as_deref(),
            Some(
                "slow window frames: 6 over 100 ms in 5.0 s, the slowest 162.0 ms; the background art evicted 7 \
                 layers"
            )
        );
        assert_eq!(slow.recent, 0, "the next summary starts over");
        assert_eq!(slow.count, 6, "the pill's count goes on");
    }

    #[test]
    fn a_window_frame_sends_its_sound_before_it_draws_its_picture() {
        let order = std::cell::RefCell::new(Vec::new());
        send_then_draw(|| order.borrow_mut().push("send"), || order.borrow_mut().push("draw"));
        assert_eq!(order.into_inner(), ["send", "draw"], "the sound does not wait for the drawing");
    }

    #[test]
    fn the_battle_speed_runs_at_speed_1_without_a_replay_only() {
        for tempo in [(3, 2), (2, 1)] {
            assert_eq!(battle_tempo_now(1, false, Some(tempo)), Some(tempo));
            assert_eq!(
                battle_tempo_now(2, false, Some(tempo)),
                None,
                "the hotkey's speed or the animation speed"
            );
            assert_eq!(battle_tempo_now(1, true, Some(tempo)), None, "a replay recorded or played");
        }
        assert_eq!(battle_tempo_now(1, false, None), None, "off, or not in a battle");
    }

    #[test]
    fn the_pacer_smooths_what_a_frame_costs_and_starts_over_when_far_behind() {
        let mut pacer = Pacer::new(Duration::from_millis(16));
        pacer.cost = Duration::from_millis(4);
        pacer.note_cost(0, Duration::from_secs(9));
        assert_eq!(pacer.cost, Duration::from_millis(4), "no frame, no news");
        pacer.note_cost(4, Duration::from_millis(32));
        assert_eq!(pacer.cost, Duration::from_millis(5), "(3 x 4 + 8) / 4 ms");
        // A second behind: no wait, and the clock starts over from now
        pacer.next = Instant::now() - Duration::from_secs(1);
        let before = Instant::now();
        pacer.wait(4, 4, false);
        assert!(before.elapsed() < Duration::from_millis(100), "no sleep when behind");
        assert!(pacer.next + Duration::from_millis(100) > before, "the clock was reset");
    }

    #[test]
    fn window_title_shows_speed_and_slot() {
        let title = window_title(Speed::default().next(), 3);
        assert_eq!(title, format!("{} — ×2 — slot 3", crate::TITLE));
    }

    #[test]
    fn the_frame_goku_first_starts_to_transform_calls_for_the_voice() {
        let (mut machine, _store) = sram_counter_machine();
        let mut state = window_state(HooksSwitch::Allowed);
        let mut session = Session::default();
        let mut audio = NullAudio;
        // Goku in the third record of the party (flags $C0, unit id at +$01, level 47 at +$02, character id 0 at +$1C)
        // (work RAM starts seeded: the nine records are cleared first), in the Frieza fight 03:8683 builds the Super
        // Saiyan in (group $56 at $0EA3, $01BB 0 below the party size 3 at $0D9A), in phase $1A (battle_phase, $AB)
        machine.write_wram(0x7E_0B00, &[0; 9 * 0x20]).unwrap();
        let goku = 0x7E_0B00 + 2 * 0x20;
        machine.write_wram(0x7E_0EA3, &[0x56]).unwrap();
        machine.write_wram(0x7E_01BB, &[0]).unwrap();
        machine.write_wram(0x7E_0D9A, &[3]).unwrap();
        machine.write_wram(0x7E_00AB, &[0x1A]).unwrap();
        // Each frame: Goku's unit id, and side A's +$26 ($5F: the script command 05:D943 started the flashing)
        let mut frame_with =
            |machine: &mut Machine, state: &mut WindowState, unit: u8, side_a_26: u8| {
                machine.write_wram(goku, &[0xC0, unit, 47]).unwrap();
                machine.write_wram(0x7E_1426, &[side_a_26]).unwrap();
                run_frames(state, machine, &mut session, None, Joypad::default(), &mut audio, 1)
                    .unwrap();
                std::mem::take(&mut state.voice_due)
            };
        assert!(
            !frame_with(&mut machine, &mut state, 0x3A, 0x70),
            "the first frame only notes Goku"
        );
        assert!(
            !frame_with(&mut machine, &mut state, 0x3A, 0x70),
            "Goku's words before the flashing"
        );
        assert!(
            frame_with(&mut machine, &mut state, 0x3A, 0x5F),
            "the flashing starts: Goku shouts"
        );
        assert!(!frame_with(&mut machine, &mut state, 0x3A, 0x5F));
        assert!(
            !frame_with(&mut machine, &mut state, 0x3B, 0x5F),
            "not again when he is built a Super Saiyan"
        );
        assert!(!frame_with(&mut machine, &mut state, 0x00, 0x70));
        assert!(!frame_with(&mut machine, &mut state, 0x00, 0x5F), "once in a run");
    }

    #[test]
    fn the_lightning_follows_the_transformation_and_a_state_loaded_stops_it_before_a_frame_runs() {
        let (mut machine, _store) = sram_counter_machine();
        let mut state = window_state(HooksSwitch::Allowed);
        let mut session = Session::default();
        let mut audio = NullAudio;
        // The work RAM of the voice's test above: Goku (record 3, level 47) in the Frieza fight, in phase $1A
        machine.write_wram(0x7E_0B00, &[0; 9 * 0x20]).unwrap();
        let goku = 0x7E_0B00 + 2 * 0x20;
        machine.write_wram(0x7E_0EA3, &[0x56]).unwrap();
        machine.write_wram(0x7E_01BB, &[0]).unwrap();
        machine.write_wram(0x7E_0D9A, &[3]).unwrap();
        machine.write_wram(0x7E_00AB, &[0x1A]).unwrap();
        let mut frame_with =
            |machine: &mut Machine, state: &mut WindowState, unit: u8, side_a_26: u8| {
                machine.write_wram(goku, &[0xC0, unit, 47]).unwrap();
                machine.write_wram(0x7E_1426, &[side_a_26]).unwrap();
                run_frames(state, machine, &mut session, None, Joypad::default(), &mut audio, 1)
                    .unwrap();
                state.ssj_watch.lightning_at(machine.frame_count())
            };
        assert_eq!(frame_with(&mut machine, &mut state, 0x3A, 0x70), None);
        let before = machine.save_state();
        assert_eq!(frame_with(&mut machine, &mut state, 0x3A, 0x70), None, "Goku's words");
        assert_eq!(
            frame_with(&mut machine, &mut state, 0x3A, 0x5F),
            Some(0),
            "the flashing starts"
        );
        assert_eq!(frame_with(&mut machine, &mut state, 0x3A, 0x5F), Some(1));
        assert_eq!(frame_with(&mut machine, &mut state, 0x3A, 0x5F), Some(2));
        // A state of before the transformation loaded: none at once, before a frame runs
        machine.load_state(&before).unwrap();
        assert_eq!(state.ssj_watch.lightning_at(machine.frame_count()), None, "the state loaded");
        assert_eq!(frame_with(&mut machine, &mut state, 0x3A, 0x70), None);
        assert_eq!(
            frame_with(&mut machine, &mut state, 0x3A, 0x5F),
            Some(0),
            "the transformation again"
        );
        assert_eq!(
            frame_with(&mut machine, &mut state, 0x3B, 0x5F),
            None,
            "built a Super Saiyan: over"
        );
    }

    /// The frame the transformation's flashing starts in `secret-ssj-frieza.ssdreplay` (the frame index of the run, the
    /// frame count before it): the script command `05:D943` sets side A's `+$26` to `$5F` in it, and the next frame's
    /// picture is the first of the flashing backdrop. Goku's unit id becomes `$3B` (`03:85F0`) at frame 6464, 342
    /// frames (5.7 s) later; phase `$1A` starts at 5998, with Goku's words ("No... it can't be... Krillin...").
    const FRIEZA_REPLAY_FLASHING: u64 = 6122;
    const FRIEZA_REPLAY_BUILT: u64 = 6464;

    #[test]
    #[ignore = "needs SSD_ROM_PATH and SSD_REPLAY_DIR (the secret battles of the regression inputs)"]
    fn the_voice_starts_the_frame_the_flashing_of_the_transformation_starts_in_the_replays() {
        struct Silent;
        impl ssd_oracle::AudioSink for Silent {
            fn push(&mut self, _: f32, _: f32) {}
        }
        let bytes = std::fs::read(std::env::var("SSD_ROM_PATH").unwrap()).unwrap();
        let rom = ssd_rom::normalize(&bytes, None).unwrap().bytes;
        let dir = std::path::PathBuf::from(std::env::var("SSD_REPLAY_DIR").unwrap());
        let mut paths: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "ssdreplay"))
            .collect();
        paths.sort();
        let mut frieza = None;
        for path in &paths {
            let bytes = std::fs::read(path).unwrap();
            let (_, bytes) = ssd_oracle::Replay::rebase(&bytes, ssd_oracle::CORE_VERSION).unwrap();
            let replay = ssd_oracle::Replay::from_bytes(
                &bytes,
                ssd_rom::crc32(&rom),
                ssd_oracle::CORE_VERSION,
            )
            .unwrap();
            let mut machine = replay.power_on(rom.clone()).unwrap();
            let mut watch = crate::voice::SsjWatch::default();
            // The frames the watch fires at, and the frames Goku's unit id goes from a form before to $3B
            let (mut fired, mut built) = (Vec::new(), Vec::new());
            let mut unit = None;
            while machine.frame_count() < replay.frames() {
                let frame = machine.frame_count();
                ssd_oracle::apply_events(&mut machine, replay.events_at(frame)).unwrap();
                machine.run_frame_with(&mut |f| replay.input(f), &mut Silent).unwrap();
                let goku = crate::voice::goku(machine.wram());
                if watch.observe(machine.frame_count(), goku) {
                    fired.push(frame);
                }
                if unit.is_some_and(|u| [0x00, 0x39, 0x3A].contains(&u)) && goku.unit == Some(0x3B)
                {
                    built.push(frame);
                }
                unit = goku.unit;
            }
            let name = path.file_stem().unwrap().to_string_lossy();
            eprintln!("{name}: the voice at {fired:?}, Goku built a Super Saiyan at {built:?}");
            assert_eq!(
                fired.len(),
                built.len().min(1),
                "{name}: the voice once, with a transformation only"
            );
            if let (Some(&voice), Some(&made)) = (fired.first(), built.first()) {
                assert!(voice < made, "{name}: the voice before the end of the transformation");
            }
            if name == "secret-ssj-frieza" {
                frieza = Some((fired, built));
            }
        }
        let (fired, built) = frieza.expect("secret-ssj-frieza.ssdreplay in SSD_REPLAY_DIR");
        assert_eq!(fired, [FRIEZA_REPLAY_FLASHING], "the voice starts with the flashing");
        assert_eq!(built, [FRIEZA_REPLAY_BUILT], "Goku's unit id becomes $3B at the end");
    }

    /// The lightning of `secret-ssj-frieza.ssdreplay`: on from the frame the voice starts at to the last frame before
    /// Goku's unit id becomes `$3B`, every frame of it, counted from 0, and on no other frame.
    #[test]
    #[ignore = "needs SSD_ROM_PATH and SSD_REPLAY_DIR (the secret battles of the regression inputs)"]
    fn the_lightning_lasts_the_whole_transformation_in_the_frieza_replay() {
        struct Silent;
        impl ssd_oracle::AudioSink for Silent {
            fn push(&mut self, _: f32, _: f32) {}
        }
        let bytes = std::fs::read(std::env::var("SSD_ROM_PATH").unwrap()).unwrap();
        let rom = ssd_rom::normalize(&bytes, None).unwrap().bytes;
        let path = std::path::PathBuf::from(std::env::var("SSD_REPLAY_DIR").unwrap())
            .join("secret-ssj-frieza.ssdreplay");
        let bytes = std::fs::read(path).unwrap();
        let (_, bytes) = ssd_oracle::Replay::rebase(&bytes, ssd_oracle::CORE_VERSION).unwrap();
        let replay =
            ssd_oracle::Replay::from_bytes(&bytes, ssd_rom::crc32(&rom), ssd_oracle::CORE_VERSION)
                .unwrap();
        let mut machine = replay.power_on(rom).unwrap();
        let mut watch = crate::voice::SsjWatch::default();
        // (the frame index of the run, the lightning's count) of every frame it is on
        let mut lit = Vec::new();
        while machine.frame_count() < replay.frames() {
            let frame = machine.frame_count();
            ssd_oracle::apply_events(&mut machine, replay.events_at(frame)).unwrap();
            machine.run_frame_with(&mut |f| replay.input(f), &mut Silent).unwrap();
            watch.observe(machine.frame_count(), crate::voice::goku(machine.wram()));
            if let Some(count) = watch.lightning_at(machine.frame_count()) {
                lit.push((frame, count));
            }
        }
        eprintln!("the lightning: {:?} .. {:?}, {} frames", lit.first(), lit.last(), lit.len());
        let expected: Vec<(u64, u32)> = (FRIEZA_REPLAY_FLASHING..FRIEZA_REPLAY_BUILT)
            .map(|frame| (frame, (frame - FRIEZA_REPLAY_FLASHING) as u32))
            .collect();
        assert_eq!(
            lit, expected,
            "every frame of the transformation, from the voice's to the one before $3B"
        );
    }

    #[test]
    fn a_breakpoint_pauses_the_run_and_the_banner_names_it() {
        let (mut machine, _store) = sram_counter_machine();
        let mut state = window_state(HooksSwitch::Allowed);
        // The wait loop of the test ROM (`LDA $4210` at $800A).
        machine.set_breakpoint(0x00_800A);
        let mut session = Session::default();
        let mut audio = NullAudio;
        let ran = run_frames(
            &mut state,
            &mut machine,
            &mut session,
            None,
            Joypad::default(),
            &mut audio,
            1,
        )
        .unwrap();
        assert_eq!(ran, 0, "a frame the breakpoint stopped is not counted");
        assert!(state.history.is_empty(), "nor noted for the figures");
        assert!(state.controls.paused, "the breakpoint pauses the machine");
        let banner = state.panel.debug.banner.clone().expect("a stop banner");
        assert_eq!(banner.title, "$00800A");
        assert_eq!(state.panel.debug.breakpoints.len(), 0, "set on the machine, not in the list");
    }

    #[test]
    fn a_script_frames_line_stops_at_a_breakpoint_and_pauses() {
        let (mut machine, _store) = sram_counter_machine();
        let mut state = window_state(HooksSwitch::Allowed);
        machine.set_breakpoint(0x00_800A);
        let mut session = Session::default();
        let env = Env { rom: &[], switch: None, verify: None };
        snapshot_frames(5, &mut state, &mut machine, &mut session, &env).unwrap();
        assert!(machine.frame_count() < 5, "the breakpoint ends the run early");
        assert!(state.controls.paused);
        let banner = state.panel.debug.banner.clone().expect("a stop banner");
        assert_eq!(banner.title, "$00800A");
    }

    #[test]
    fn a_routine_key_is_the_address_the_routines_tab_uses() {
        let symbols =
            Symbols::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml")).unwrap();
        let rng = symbols.routines().iter().find(|r| r.name == "rng_next_a").unwrap();
        assert_eq!(routine_key("rng_next_a", Some(&symbols)).unwrap(), rng.address);
        assert!(routine_key("no_such_routine", Some(&symbols)).is_err());
        assert!(routine_key("rng_next_a", None).is_err(), "no symbols, no routine");
    }

    /// The panel drawn offscreen over the synthetic ROM: a PNG of the requested size, and four
    /// places of it in the colours the panel draws them (its ground, the game area, the header,
    /// the selected tab's underline). Probes, not an exact picture: the layout may move, the
    /// colours may not.
    #[test]
    fn a_ui_snapshot_writes_the_open_panel_at_the_requested_size() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        let switch = HookSwitch::load(
            Some(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml"))),
            None,
        )
        .unwrap();
        let script = ui_script::parse("panel open\ntab routines\n").unwrap();
        let out = dir.path().join("panel.png");
        let request = snapshot::Request {
            frames: 2,
            out: &out,
            size: (1440, 900),
            script: &script,
            enhanced: None,
            touch: false,
        };
        run_ui_snapshot(
            sram_counter_rom(),
            &request,
            None,
            Some(&switch),
            HooksArg::Original,
            Ruleset::classic(),
            Session::default(),
        )
        .unwrap();

        let (width, height, pixels) = png::decode_rgba(&std::fs::read(&out).unwrap());
        assert_eq!((width, height), (1440, 900));
        let at = |x: u32, y: u32| {
            let i = ((y * width + x) * 4) as usize;
            [pixels[i], pixels[i + 1], pixels[i + 2]]
        };
        let rgb = |colour: egui::Color32| {
            let [r, g, b, _] = colour.to_array();
            [r, g, b]
        };
        assert_eq!(at(1200, 600), rgb(crate::theme::PANEL), "the panel, below its rows");
        assert_eq!(at(500, 450), [0, 0, 0], "the game frame, centred (forced blank: black)");
        assert_eq!(at(700, 20), rgb(crate::theme::PANEL), "the header bar");
        assert_eq!(at(1045, 79), rgb(crate::theme::ACCENT), "the underline of the Routines tab");
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    #[test]
    fn a_script_line_that_cannot_run_fails_the_snapshot_and_writes_nothing() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        let switch = HookSwitch::load(
            Some(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml"))),
            None,
        )
        .unwrap();
        let script = ui_script::parse("panel open\nselect no_such_symbol\n").unwrap();
        let out = dir.path().join("never.png");
        let request = snapshot::Request {
            frames: 1,
            out: &out,
            size: (320, 200),
            script: &script,
            enhanced: None,
            touch: false,
        };
        let error = run_ui_snapshot(
            sram_counter_rom(),
            &request,
            None,
            Some(&switch),
            HooksArg::Original,
            Ruleset::classic(),
            Session::default(),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("UI script, line 2"), "{error:#}");
        assert!(!out.exists(), "no picture for a script that failed");
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    /// Needs `SSD_ROM_PATH` (canonical ROM); run with `cargo test -p ssd-app -- --ignored`.
    #[test]
    #[ignore = "needs SSD_ROM_PATH"]
    fn state_files_round_trip_and_corrupt_files_are_rejected() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        let rom = std::fs::read(std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH")).unwrap();
        let mut machine = new_machine(rom, None, &mut Session::default()).unwrap();
        for _ in 0..30 {
            machine.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        }

        let path = save_state(&machine, 2).unwrap();
        assert_eq!(path, hotkeys::state_path(2));
        assert!(path.exists());
        assert!(!path.with_extension("ssdstate.tmp").exists(), "temp file is renamed away");
        for _ in 0..30 {
            machine.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        }
        assert_eq!(machine.frame_count(), 60);
        load_state(&mut machine, 2).unwrap();
        assert_eq!(machine.frame_count(), 30, "the loaded state rewinds the machine");

        std::fs::write(hotkeys::state_path(3), [0xA5_u8; 4096]).unwrap();
        assert!(load_state(&mut machine, 3).is_err(), "corrupt state is rejected");
        assert!(load_state(&mut machine, 4).is_err(), "missing state is an error, not a panic");
        machine.run_frame(Joypad::default(), &mut NullAudio).unwrap(); // machine still usable
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    /// One window frame as `windowed_loop` runs it: the joypad's edges are the HD's keys (`hd::keys_pressed`), then
    /// the frames run with the joypad held.
    #[allow(clippy::too_many_arguments)]
    fn window_step(
        state: &mut WindowState,
        machine: &mut Machine,
        session: &mut Session,
        env: &Env<'_>,
        audio: &mut audio::Audio,
        frames: u32,
        pad: Joypad,
        previous: &mut Joypad,
    ) {
        if let Some(hd) = state.hd.as_mut() {
            for key in hd::keys_pressed(*previous, pad) {
                hd.key(key, machine.wram());
            }
        }
        *previous = pad;
        window_frames(state, machine, session, env, pad, audio, frames).unwrap();
    }

    /// Latency Smooth and the PPU's pixels left undrawn, on the window's own frame path with the real game: the work
    /// RAM is the same as with the pixels drawn, the picture of each frame is the one the pixels-drawn run shows, and
    /// Smooth shows it one window frame later (the first twice: it has nothing before it) and drops none.
    #[test]
    #[ignore = "needs SSD_ROM_PATH"]
    fn smooth_latency_shows_the_same_pictures_one_window_frame_late_with_the_pixels_undrawn() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        let rom = std::fs::read(std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH")).unwrap();
        let ruleset = Ruleset::enhanced();
        let env = Env { rom: &rom, switch: None, verify: None };
        let options = HdOptions { scale: 2, look: ssd_hd::Look::Crisp };
        // The pictures on show after each window frame, whether the last frame left the pixels undrawn, the work RAM
        let run = |smooth: bool, drawn: bool| -> (Vec<u32>, bool, Vec<u8>) {
            let mut audio = audio::Audio::muted();
            let mut session = Session::default();
            let mut machine = new_machine(rom.clone(), None, &mut session).unwrap();
            let mut state = WindowState::new(Controls::new(
                HooksArg::Original,
                HooksSwitch::Allowed,
                false,
                false,
            ));
            state.hd = hd_layer(&rom, ruleset).unwrap();
            state.scaled = Some(options);
            state.video.smooth = smooth;
            if drawn {
                state.compose_hold = u32::MAX;
            }
            machine.set_line_log(true);
            let mut previous = Joypad::default();
            let mut shown = Vec::new();
            for _ in 0..300 {
                window_step(
                    &mut state,
                    &mut machine,
                    &mut session,
                    &env,
                    &mut audio,
                    1,
                    Joypad::default(),
                    &mut previous,
                );
                let (frame, view) = state.hd_view.as_ref().expect("a picture is on show");
                assert_eq!(*frame, machine.frame_count(), "and it is the machine's frame");
                shown.push(ssd_rom::crc32(&view.picture.picture.rgba));
            }
            (shown, state.ppu_stale, machine.wram().to_vec())
        };
        let (reference, stale, wram) = run(false, true);
        assert!(!stale, "the reference draws the pixels");
        assert!(reference.windows(2).any(|pair| pair[0] != pair[1]), "the pictures change");
        let (low, stale, low_wram) = run(false, false);
        assert!(stale, "the PPU's pixels are left undrawn");
        assert_eq!(low, reference, "and the pictures are the same");
        assert_eq!(low_wram, wram, "and so is the game");
        let (smooth, stale, smooth_wram) = run(true, false);
        assert!(stale);
        assert_eq!(smooth_wram, wram);
        assert_eq!(smooth[0], reference[0]);
        for (n, picture) in smooth.iter().enumerate().skip(1) {
            assert_eq!(*picture, reference[n - 1], "window frame {n} shows the frame before it");
        }
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    /// One run of the reference battle's script on the window's own frame path with the real game (Enhanced mode, the
    /// sound chip at 1/1, so every run is the same game frame for frame): with `tempo`, in battle, the window frames
    /// run as the pacer gives them at 1.5 times (1, 2, 1, 2, … frames a window frame).
    struct BattleRun {
        /// The pictures on show by the frame they are of.
        shown: std::collections::BTreeMap<u64, u32>,
        /// The picture on show after each window frame: the frame the machine is at, the last frame of the window
        /// frame before, and the picture's CRC.
        windows: Vec<(u64, u64, u32)>,
        /// The window frames of two in the battle.
        pairs: u32,
        /// Those of them that drew more than one picture (new serials of `ssd_hd::world::next_serial`).
        pairs_drawn_twice: u32,
        /// Each window frame in the battle: what it took (ms), the frames it ran, their emulation and the 16:9 view's
        /// share of it (ms, `Stages`), and the pictures it drew.
        costs: Vec<(f64, u32, f64, f64, u64)>,
        wram: Vec<u8>,
    }

    /// A run of the window's frame path for test (c) of the art read apart: the art files read by the threads that
    /// draw, the slowest window frame (ms), and the frames run in battle.
    struct ArtRun {
        loads_on_draw: u64,
        slowest: f64,
        battle_frames: u32,
        wram: Vec<u8>,
    }

    /// `frames` frames of the real game on the window's frame path at 4x with the pack's art, the joypad `input` of
    /// each frame: the art read in the pictures, or (`apart`) as `windowed_loop` sets it (`read_art_apart`), with
    /// Smooth (`smooth`: the background thread draws) or not (the window's thread draws).
    fn art_run(
        rom: &[u8],
        frames: u64,
        input: fn(u64) -> Joypad,
        apart: bool,
        smooth: bool,
    ) -> ArtRun {
        let env = Env { rom, switch: None, verify: None };
        let mut audio = audio::Audio::muted();
        let mut session = Session::default();
        let mut machine = new_machine(rom.to_vec(), None, &mut session).unwrap();
        let mut state = window_state(HooksSwitch::Allowed);
        state.hd = hd_layer(rom, Ruleset::enhanced()).unwrap();
        state.scaled = Some(HdOptions { scale: 4, look: ssd_hd::Look::Crisp });
        state.real_time = true;
        state.video.smooth = smooth;
        if apart {
            read_art_apart(&mut state, None);
        }
        machine.set_line_log(true);
        let mut previous = Joypad::default();
        let before = ssd_hd::art_loads_on_draw();
        let (mut slowest, mut battle_frames) = (0.0_f64, 0);
        while machine.frame_count() < frames {
            let pad = input(machine.frame_count());
            let started = Instant::now();
            window_step(
                &mut state,
                &mut machine,
                &mut session,
                &env,
                &mut audio,
                1,
                pad,
                &mut previous,
            );
            slowest = slowest.max(started.elapsed().as_secs_f64() * 1000.0);
            battle_frames += u32::from(ssd_game::gameplay::in_battle(machine.wram()));
        }
        if let Some(hd) = state.hd.as_mut() {
            hd.finish_drawing();
        }
        let loads_on_draw = ssd_hd::art_loads_on_draw() - before;
        ArtRun { loads_on_draw, slowest, battle_frames, wram: machine.wram().to_vec() }
    }

    /// The title's Continue with the battery save of the saves folder: Start, Right (Continue), Start, A on the first
    /// slot, A on Yes; then the saved game runs, A pressed every 64 frames.
    fn continue_input(frame: u64) -> Joypad {
        let at = |start: u64| (start..start + 4).contains(&frame);
        Joypad {
            start: at(2400) || at(2540),
            right: at(2480),
            a: at(2700) || at(2860) || (frame >= 3200 && frame % 64 < 4),
            ..Joypad::default()
        }
    }

    /// The art read apart on the window's frame path with the real game and the shared pack at 4x: through the
    /// reference battle's script (the title, a new game, the field, the flight and a battle) and a Continue of a battery
    /// save, the threads that draw read no art file, with Smooth off (the window's thread draws) and on (the background
    /// thread draws), where the same runs reading the art in the pictures read it there. Each run's slowest window
    /// frame is printed. Needs `SSD_ROM_PATH`, `SSD_HD_CACHE` (a data folder's `cache` with the pack installed for the
    /// ROM) and `SSD_BATTERY_SAVE` (a battery save with a game in its first slot).
    #[test]
    #[ignore = "needs SSD_ROM_PATH, SSD_HD_CACHE and SSD_BATTERY_SAVE"]
    fn the_threads_that_draw_read_no_art_in_a_battle_and_a_continue_when_it_is_read_apart() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let cache = std::env::var_os("SSD_HD_CACHE").expect("SSD_HD_CACHE");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&cache, dir.path().join("cache")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&cache, dir.path().join("cache")).unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        let rom = std::fs::read(std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH")).unwrap();
        assert!(hd_art::pack(&rom).is_some(), "the pack is installed for the ROM");
        let report = |name: &str, run: &ArtRun| {
            eprintln!(
                "{name}: {} art files read by the threads that draw, slowest window frame {:.1} ms, {} frames in battle",
                run.loads_on_draw, run.slowest, run.battle_frames
            );
        };
        let battle = ssd_game::scripts::BATTLE_FRAMES;
        let input = ssd_game::scripts::battle_input;
        let in_pictures = art_run(&rom, battle, input, false, false);
        report("battle, read in the pictures", &in_pictures);
        assert!(in_pictures.loads_on_draw > 0 && in_pictures.battle_frames > 0);
        for smooth in [false, true] {
            let apart = art_run(&rom, battle, input, true, smooth);
            report(&format!("battle, read apart, Smooth {smooth}"), &apart);
            assert_eq!(apart.loads_on_draw, 0, "Smooth {smooth}");
            assert_eq!(apart.wram, in_pictures.wram, "the game is the same");
        }
        // The Continue of the battery save
        let save =
            std::fs::read(std::env::var("SSD_BATTERY_SAVE").expect("SSD_BATTERY_SAVE")).unwrap();
        std::fs::create_dir_all(paths::saves_dir()).unwrap();
        let frames = 4_800;
        let new_game = art_run(&rom, frames, continue_input, false, false).wram;
        std::fs::write(paths::saves_dir().join("ssd.sav"), save).unwrap();
        let in_pictures = art_run(&rom, frames, continue_input, false, false);
        report("continue, read in the pictures", &in_pictures);
        assert!(in_pictures.wram != new_game, "the saved game is loaded");
        assert!(in_pictures.loads_on_draw > 0);
        for smooth in [false, true] {
            let apart = art_run(&rom, frames, continue_input, true, smooth);
            report(&format!("continue, read apart, Smooth {smooth}"), &apart);
            assert_eq!(apart.loads_on_draw, 0, "Smooth {smooth}");
        }
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    fn battle_speed_run(
        rom: &[u8],
        tempo: Option<(u32, u32)>,
        smooth: bool,
        scale: u32,
    ) -> BattleRun {
        let env = Env { rom, switch: None, verify: None };
        let frames = ssd_game::scripts::BATTLE_FRAMES;
        let mut audio = audio::Audio::muted();
        let mut session = Session::default();
        let mut machine = new_machine(rom.to_vec(), None, &mut session).unwrap();
        let mut state = window_state(HooksSwitch::Allowed);
        state.hd = hd_layer(rom, Ruleset::enhanced()).unwrap();
        state.scaled = Some(HdOptions { scale, look: ssd_hd::Look::Crisp });
        state.real_time = true;
        state.video.smooth = smooth;
        machine.set_line_log(true);
        let mut previous = Joypad::default();
        let mut run = BattleRun {
            shown: std::collections::BTreeMap::new(),
            windows: Vec::new(),
            pairs: 0,
            pairs_drawn_twice: 0,
            costs: Vec::new(),
            wram: Vec::new(),
        };
        // The window frames of the battle so far: the pacer gives the k-th one floor((k + 1) f / r) - floor(k f / r)
        // frames at a tempo of f frames to r refreshes (1, 2, 1, 2, … at 1.5 times; 2, 2, … at 2 times)
        let mut window = 0_u64;
        let mut last_end = 0;
        while machine.frame_count() < frames {
            let at = machine.frame_count();
            let pad = ssd_game::scripts::battle_input(at);
            let in_battle = ssd_game::gameplay::in_battle(machine.wram());
            let paced = tempo.filter(|_| in_battle).map_or(1, |(f, r)| {
                let (f, r) = (u64::from(f), u64::from(r));
                let count = (window + 1) * f / r - window * f / r;
                window += 1;
                count
            });
            // Two frames in this window frame, in battle, when the script holds the same buttons for both (a window
            // frame's frames get one joypad)
            let batch =
                paced == 2 && ssd_game::scripts::battle_input(at + 1) == pad && at + 2 <= frames;
            state.whole_frames = tempo.is_some() && in_battle;
            let count = if batch { 2 } else { 1 };
            let serial = ssd_hd::world::next_serial();
            state.stages = Stages::default();
            let started = Instant::now();
            window_step(
                &mut state,
                &mut machine,
                &mut session,
                &env,
                &mut audio,
                count,
                pad,
                &mut previous,
            );
            let took = started.elapsed().as_secs_f64() * 1000.0;
            // (the call above takes a serial of its own)
            let drawn = ssd_hd::world::next_serial() - serial - 1;
            if batch {
                run.pairs += 1;
                run.pairs_drawn_twice += u32::from(drawn > 1);
            }
            if in_battle {
                let ms = |time: Duration| time.as_secs_f64() * 1000.0;
                run.costs.push((took, count, ms(state.stages.emulate), ms(state.stages.hd), drawn));
            }
            let (frame, view) = state.hd_view.as_ref().expect("a picture is on show");
            assert_eq!(*frame, machine.frame_count(), "and it is the machine's frame");
            let crc = ssd_rom::crc32(&view.picture.picture.rgba);
            run.shown.insert(*frame, crc);
            run.windows.push((*frame, last_end, crc));
            last_end = *frame;
        }
        run.wram = machine.wram().to_vec();
        run
    }

    /// The battle speed's window frames on the window's own frame path with the real game (the reference battle's
    /// script, in Enhanced mode): run in battle as the pacer gives them at 1.5 times (1, 2, 1, 2, … frames a window
    /// frame) and at 2 times (2, 2, …), every picture on show is the one speed 1 shows of that frame, so no page of the translation, no card
    /// tooltip and nothing else the 16:9 view follows from frame to frame stays up, comes late or goes early. A window
    /// frame of two draws one picture, the shown frame's, as speed 1 draws one a window frame: the frame not shown is
    /// run whole (its render log, its pixels) and followed, and drawing it as well cost a slow machine the 1.5 times
    /// (44 to 79 frames a second for 90 on one slow machine). The sound chip stays at 1/1 here, so both runs are the same game
    /// frame for frame: only the drawing differs.
    #[test]
    #[ignore = "needs SSD_ROM_PATH"]
    fn the_battle_speed_shows_each_frame_as_speed_1_shows_it() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        let rom = std::fs::read(std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH")).unwrap();
        let reference = battle_speed_run(&rom, None, false, 2);
        let digest =
            reference.shown.values().fold(0_u64, |sum, crc| sum.rotate_left(5) ^ u64::from(*crc));
        println!("speed 1: {} pictures, digest {digest:016x}", reference.shown.len());
        for (name, speed) in [
            ("1.5x", ssd_game::gameplay::BATTLE_TEMPO),
            ("2x", ssd_game::gameplay::BATTLE_TEMPO_2X),
        ] {
            let tempo = battle_speed_run(&rom, Some(speed), false, 2);
            assert!(
                tempo.pairs > 500,
                "{name}: window frames of two in the battle: {}",
                tempo.pairs
            );
            assert_eq!(tempo.wram, reference.wram, "{name}: the same game");
            let different: Vec<u64> = tempo
                .shown
                .iter()
                .filter(|&(frame, crc)| reference.shown.get(frame) != Some(crc))
                .map(|(frame, _)| *frame)
                .collect();
            println!(
                "{} pictures on show at {name}, {} window frames of two ({} drew two pictures); differing from 1x: {different:?}",
                tempo.shown.len(),
                tempo.pairs,
                tempo.pairs_drawn_twice
            );
            assert!(
                different.is_empty(),
                "{name}: every picture on show is speed 1's of its frame: {different:?}"
            );
            assert_eq!(
                tempo.pairs_drawn_twice, 0,
                "{name}: a window frame of two draws one picture, as speed 1 draws one"
            );
            // Latency Smooth shows its pictures a window frame late: every one on show is speed 1's picture of its
            // frame (a frame drawn and shown at once) or of the last frame of the window frame before
            let smooth = battle_speed_run(&rom, Some(speed), true, 2);
            assert_eq!(smooth.wram, reference.wram);
            assert_eq!(smooth.pairs_drawn_twice, 0, "{name}: Smooth too");
            let mut lags = std::collections::BTreeMap::new();
            for &(frame, before, crc) in &smooth.windows {
                let lag = [frame, before]
                    .into_iter()
                    .position(|of| reference.shown.get(&of) == Some(&crc));
                assert!(
                    lag.is_some(),
                    "{name}: frame {frame} shows a picture speed 1 shows of neither it nor {before}, the window frame before's"
                );
                *lags.entry(frame - [frame, before][lag.unwrap_or(0)]).or_insert(0) += 1;
            }
            println!(
                "{name} Smooth: window frames by how many frames late their picture is: {lags:?}"
            );
        }
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    /// What the battle speed's window frames cost on this machine, against speed 1's, on the reference battle (see
    /// [`battle_speed_run`]): the milliseconds of a window frame in the battle (p50, p95, max, and how many go over a
    /// 60 Hz refresh and over two), Latency Low (each picture drawn before the window frame ends) and Smooth. Nothing
    /// is asserted: a machine's speed is not a property of the code.
    #[test]
    #[ignore = "needs SSD_ROM_PATH; prints timings"]
    fn the_battle_speed_window_frame_cost() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        let rom = std::fs::read(std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH")).unwrap();
        for (scale, smooth) in [(2, false), (2, true), (4, false)] {
            for tempo in [
                None,
                Some(ssd_game::gameplay::BATTLE_TEMPO),
                Some(ssd_game::gameplay::BATTLE_TEMPO_2X),
            ] {
                let speed = match tempo {
                    Some((3, 2)) => "1.5x",
                    Some(_) => "2x",
                    None => "1x",
                };
                let run = battle_speed_run(&rom, tempo, smooth, scale);
                let mut all: Vec<f64> = run.costs.iter().map(|c| c.0).collect();
                all.sort_by(f64::total_cmp);
                let at = |q: f64| all[((all.len() - 1) as f64 * q).round() as usize];
                let pairs: Vec<f64> = run.costs.iter().filter(|c| c.1 == 2).map(|c| c.0).collect();
                let pair_mean = pairs.iter().sum::<f64>() / pairs.len().max(1) as f64;
                let frames: u32 = run.costs.iter().map(|c| c.1).sum();
                let emulate: f64 = run.costs.iter().map(|c| c.2).sum();
                let drawing: f64 = run.costs.iter().map(|c| c.3).sum();
                let drawn: u64 = run.costs.iter().map(|c| c.4).sum();
                println!(
                    "scale {scale} {} {}: {drawn} pictures drawn for {} window frames, {:.2} ms emulation a game frame, \
                     {:.2} ms of the 16:9 view's upkeep and waits a window frame",
                    if smooth { "Smooth" } else { "Low" },
                    speed,
                    all.len(),
                    emulate / f64::from(frames.max(1)),
                    drawing / all.len().max(1) as f64,
                );
                println!(
                    "scale {scale} {} {}: {} window frames ({frames} game frames) p50 {:.2} p95 {:.2} max {:.2} ms, \
                     over 16.7 ms {}, over 33.3 ms {}, window frames of two {} at {pair_mean:.2} ms, {:.2} ms a game frame",
                    if smooth { "Smooth" } else { "Low" },
                    speed,
                    all.len(),
                    at(0.5),
                    at(0.95),
                    at(1.0),
                    all.iter().filter(|&&ms| ms > 1000.0 / 60.0).count(),
                    all.iter().filter(|&&ms| ms > 2000.0 / 60.0).count(),
                    pairs.len(),
                    all.iter().sum::<f64>() / f64::from(frames.max(1)),
                );
            }
        }
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    /// Turbo on the window's own frame path with the real game: a held A reaches the game as the pulse the turbo makes
    /// (`turbo::Turbo`, 20 a second here), and the same pulse fed to a bare machine gives the same work RAM, which a
    /// held A without turbo does not. The ROM stands in for the ki training, which needs a save no copy of ours
    /// reaches; a recording keeps the pad `Session::run` is given, which is this one (a live Enhanced run cannot
    /// record: its cheat hooks are refused by the recorder, as before).
    #[test]
    #[ignore = "needs SSD_ROM_PATH"]
    fn a_held_a_with_turbo_reaches_the_game_as_the_pulse() {
        use ssd_menus::options::controls::{TURBO_MODE, TURBO_RATE};
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        let rom = std::fs::read(std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH")).unwrap();
        let ruleset = Ruleset::enhanced();
        let env = Env { rom: &rom, switch: None, verify: None };
        let mut audio = audio::Audio::muted();
        let held = Joypad { a: true, ..Joypad::default() };
        let frames = 600_u64;

        let mut session = Session::default();
        let mut machine = new_machine(rom.clone(), None, &mut session).unwrap();
        let mut state =
            WindowState::new(Controls::new(HooksArg::Original, HooksSwitch::Allowed, false, false));
        state.hd = hd_layer(&rom, ruleset).unwrap();
        {
            let values = state.hd.as_mut().unwrap().pc_options_mut();
            values.set(TURBO_MODE, 1);
            values.set(TURBO_RATE, 1);
        }
        let mut previous = Joypad::default();
        for _ in 0..frames {
            window_step(
                &mut state,
                &mut machine,
                &mut session,
                &env,
                &mut audio,
                1,
                held,
                &mut previous,
            );
        }

        // The same game fed the pulse straight, and fed the held A without turbo.
        let run = |pad_of: &mut dyn FnMut() -> Joypad| {
            let mut machine =
                Machine::new(rom.clone(), Box::new(ssd_oracle::MemorySaveStore::default()))
                    .unwrap();
            for _ in 0..frames {
                machine.run_frame(pad_of(), &mut NullAudio).unwrap();
            }
            machine.wram().to_vec()
        };
        let mut turbo = crate::turbo::Turbo::default();
        let settings = crate::turbo::Settings { buttons: crate::turbo::Buttons::A, rate: 20 };
        let pulsed = run(&mut || turbo.step(settings, held));
        let plain = run(&mut || held);
        assert_eq!(machine.wram(), pulsed.as_slice(), "the window path feeds the turbo's pulse");
        assert_ne!(pulsed, plain, "and the pulse is not a held A to the game");
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    /// The player's presses through the window's own frame path, as the window loop gives them. The replay plays to
    /// `MENU_FRAME` (on foot, no menu open: frame 3700 of `menus.ssdreplay`, the default). Then each press of
    /// `MENU_SEQ` (comma separated: a, b, up, down, or - for none) is held for six window frames, and the frames run
    /// until the HD's choice ends; the first `a` opens the original's command window, which the HD follows. The last
    /// press must reach `MENU_EXPECT` (command, menu or items). `MENU_FRAMES` is the emulated frames per window frame
    /// (1 at normal speed, 4 the fast-forward), `MENU_REPLAY` the replay in `.work/replays`, `MENU_HOOKS=rust` runs the
    /// routines in Rust (the shipped game).
    #[test]
    #[ignore = "needs SSD_ROM_PATH and the reference replays (.work/replays)"]
    fn the_players_presses_reach_the_sub_menu_on_the_window_frame_path() {
        let rom = std::fs::read(std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH")).unwrap();
        let name = std::env::var("MENU_REPLAY").unwrap_or_else(|_| "menus.ssdreplay".to_owned());
        let replay = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.work/replays").join(name);
        let until: u64 =
            std::env::var("MENU_FRAME").ok().and_then(|value| value.parse().ok()).unwrap_or(3700);
        let frames: u32 =
            std::env::var("MENU_FRAMES").ok().and_then(|value| value.parse().ok()).unwrap_or(1);
        let sequence =
            std::env::var("MENU_SEQ").unwrap_or_else(|_| "a,down,down,down,down,a".to_owned());
        let want = match std::env::var("MENU_EXPECT").unwrap_or_default().as_str() {
            "command" => Screen::Command,
            "items" => Screen::Items,
            _ => Screen::Menu,
        };
        let hooks_arg = if std::env::var("MENU_HOOKS").is_ok_and(|value| value == "rust") {
            HooksArg::Rust
        } else {
            HooksArg::Original
        };
        let ruleset = Ruleset::enhanced();
        let mut session = Session::new(Some(replay.as_path()), None, &rom).unwrap();
        enhanced_session(&mut session, ruleset).unwrap();
        let hooks = crate::hook_setup::prepare(hooks_arg, None, None).unwrap();
        let switch = HookSwitch::load(None, None).ok();
        let mut machine = new_machine(rom.clone(), hooks.as_ref(), &mut session).unwrap();
        let mut state =
            WindowState::new(Controls::new(hooks_arg, HooksSwitch::Allowed, false, false));
        state.hd = hd_layer(&rom, ruleset).unwrap();
        if let Some(hd) = state.hd.as_mut() {
            hd.set_replaying();
        }
        state.scaled = Some(HdOptions { scale: 4, look: ssd_hd::Look::default() });
        let env = Env { rom: &rom, switch: switch.as_ref(), verify: None };
        let mut audio = audio::Audio::muted();
        let idle = Joypad::default();
        let mut previous = Joypad::default();
        while machine.frame_count() < until && !session.playback_over(&machine) {
            window_step(
                &mut state,
                &mut machine,
                &mut session,
                &env,
                &mut audio,
                1,
                idle,
                &mut previous,
            );
        }
        session.stop_playback();
        for token in sequence.split(',').map(str::trim) {
            let pad = match token {
                "a" => Joypad { a: true, ..Joypad::default() },
                "b" => Joypad { b: true, ..Joypad::default() },
                "up" => Joypad { up: true, ..Joypad::default() },
                "down" => Joypad { down: true, ..Joypad::default() },
                "-" => idle,
                other => panic!("MENU_SEQ: no press named {other}"),
            };
            for _ in 0..6 {
                window_step(
                    &mut state,
                    &mut machine,
                    &mut session,
                    &env,
                    &mut audio,
                    frames,
                    pad,
                    &mut previous,
                );
            }
            for _ in 0..12 {
                window_step(
                    &mut state,
                    &mut machine,
                    &mut session,
                    &env,
                    &mut audio,
                    frames,
                    idle,
                    &mut previous,
                );
            }
            for _ in 0..20_000 {
                if !state.hd.as_ref().unwrap().busy() {
                    break;
                }
                window_step(
                    &mut state,
                    &mut machine,
                    &mut session,
                    &env,
                    &mut audio,
                    frames,
                    idle,
                    &mut previous,
                );
            }
            let driver = state.hd.as_ref().unwrap().driver();
            eprintln!(
                "press {token}: frame {} curtain {:?} screen {:?} outcome {:?}",
                machine.frame_count(),
                driver.curtain(),
                driver.nav().screen(),
                driver.last_outcome()
            );
        }
        for _ in 0..30 {
            window_step(
                &mut state,
                &mut machine,
                &mut session,
                &env,
                &mut audio,
                frames,
                idle,
                &mut previous,
            );
        }
        let driver = state.hd.as_ref().unwrap().driver();
        assert!(
            driver.curtain() == Curtain::Up && driver.nav().screen() == want,
            "the last press did not reach {want:?}"
        );
    }
}
