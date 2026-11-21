//! Which graphics backend an Android start uses, from what the last start left behind (`android_report.rs` keeps the
//! files): while a start has not drawn its first picture, a stage file says how far it got; the first picture deletes
//! it. A stage file found at the next start means that start died before its first picture (a crash in a driver as
//! much as a panic), and the game then starts on OpenGL ES instead of Vulkan, remembers that for this installation of
//! the app, and says so. If OpenGL ES dies too, the next start says that neither backend could start the game, and
//! stays on OpenGL ES: it never goes back and forth between the two.
//!
//! The choice is remembered for one installation of the APK: an update (or a reinstall) tries Vulkan again. A start the
//! system killed half way (a call, the app swiped away) looks like a failure too; that costs this installation Vulkan,
//! which the game hardly needs (it draws one picture and the menus), until the next update.
//!
//! Pure, so the host tests it; the desktop program does not use it.

use crate::exit_info::ExitKind;
use crate::launch_report::tr;
use ssd_menus::ui_text::UiText;
use std::fmt::Write as _;

/// How far a start got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The setup screens' window (the ROM, the mode, the HD pack).
    Setup,
    /// The game loads: the ROM, the packs, the machine.
    Start,
    /// The game's window and its graphics.
    Window,
    /// The sound.
    Audio,
    /// Everything is open; the first picture is being drawn.
    FirstFrame,
}

impl Stage {
    const ALL: [Stage; 5] =
        [Stage::Setup, Stage::Start, Stage::Window, Stage::Audio, Stage::FirstFrame];

    /// The stage's name in the stage file.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Stage::Setup => "setup",
            Stage::Start => "start",
            Stage::Window => "window",
            Stage::Audio => "audio",
            Stage::FirstFrame => "first-frame",
        }
    }

    fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|stage| stage.code() == code)
    }

    /// The stage in the player's words (a row of the menus' tables).
    fn words(stage: Option<Self>) -> &'static str {
        match stage {
            Some(Stage::Setup) => "the setup screens",
            Some(Stage::Start) => "loading the game",
            Some(Stage::Window) => "opening the game's window and graphics",
            Some(Stage::Audio) => "opening the sound",
            Some(Stage::FirstFrame) => "drawing the first picture",
            None => "an unknown step",
        }
    }
}

/// The graphics backend a start asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// wgpu's order: Vulkan, then OpenGL ES where Vulkan cannot open (`canvas::backends_to_try`).
    Auto,
    /// OpenGL ES alone.
    Gl,
    /// Whatever `WGPU_BACKEND` named before the guard looked (the arguments file of a test build).
    Forced,
}

impl Backend {
    fn code(self) -> &'static str {
        match self {
            Backend::Auto => "auto",
            Backend::Gl => "gl",
            Backend::Forced => "forced",
        }
    }

    fn from_code(code: &str) -> Option<Self> {
        [Backend::Auto, Backend::Gl, Backend::Forced]
            .into_iter()
            .find(|backend| backend.code() == code)
    }
}

/// What the last start left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Last {
    /// No stage file: it drew its first picture, or there was none before.
    Fine,
    /// A stage file: it died before its first picture. A part the file does not say (or says in a way not known) is
    /// `None`.
    Stopped { stage: Option<Stage>, backend: Option<Backend>, gpu: Option<String> },
}

/// The stage file of a start at `stage` on `backend`, with the graphics adapter once it opened.
#[must_use]
pub fn stage_text(stage: Stage, backend: Backend, gpu: Option<&str>) -> String {
    let mut text = format!("stage={}\nbackend={}\n", stage.code(), backend.code());
    if let Some(gpu) = gpu.filter(|gpu| !gpu.is_empty()) {
        text.push_str("gpu=");
        text.push_str(&gpu.replace(['\n', '\r'], " "));
        text.push('\n');
    }
    text
}

/// What the stage file `text` (`None`: there is none) says of the last start. A file that cannot be read whole still
/// means the start died: its unknown parts are `None`.
#[must_use]
pub fn parse_stage(text: Option<&str>) -> Last {
    let Some(text) = text else { return Last::Fine };
    let value = |key: &str| {
        text.lines()
            .find_map(|line| line.trim().strip_prefix(key)?.strip_prefix('='))
            .map(str::trim)
    };
    Last::Stopped {
        stage: value("stage").and_then(Stage::from_code),
        backend: value("backend").and_then(Backend::from_code),
        gpu: value("gpu").filter(|gpu| !gpu.is_empty()).map(str::to_owned),
    }
}

/// Whether the stage file stays when a run (of the setup screens or of the game) returns before its first picture, at
/// `stage`, having `succeeded` or not. A run that returns on its own did not crash: one that ended as asked (the player
/// quit while it loaded, a benchmark that draws offscreen) or failed before any graphics (a ROM or a pack refused, which
/// its own dialog has explained) says nothing of the backend, and its file goes. One that failed with the graphics open,
/// or in the setup screens (whose failures are their window's), keeps it: the next start counts it as a failure.
#[must_use]
pub fn stage_stays_after_return(stage: Option<Stage>, succeeded: bool) -> bool {
    !succeeded
        && matches!(stage, Some(Stage::Setup | Stage::Window | Stage::Audio | Stage::FirstFrame))
}

/// What else the stage file says of the start in progress: its process and when it started (to find how the process
/// ended, `exit_info`), and, once the game's loop runs, how its presents went (a game that runs but never shows a
/// picture, a black screen with the sound playing).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StageExtra {
    pub pid: Option<u32>,
    /// When the process started, in milliseconds since 1970.
    pub started_ms: Option<u64>,
    pub presents: Presents,
    /// The run returned with a failure while its graphics were open (`android_report::run_returned`): the stage file
    /// was kept on purpose.
    pub returned_failed: bool,
}

/// The presents of a start that has not shown its first picture: each try of the window, and why it showed nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Presents {
    pub attempts: u32,
    /// Skipped: the window minimised, of no size, or without a surface yet.
    pub skipped: u32,
    /// The surface's texture refused: outdated or lost (configured again), timed out or occluded (none this time).
    pub outdated: u32,
    pub lost: u32,
    pub timeout: u32,
    pub occluded: u32,
    /// How long the game's loop has run in the foreground without showing a picture, in milliseconds.
    pub waited_ms: u64,
    /// The surface was made again after a black screen (once a start, [`Presents::count`]).
    pub rebuilt: u32,
}

/// Why a present of the window showed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unshown {
    /// The window minimised, of no size, or without a surface yet.
    Skipped,
    Outdated,
    Lost,
    Timeout,
    Occluded,
}

/// The longest gap between two presents counted as time the game waited for a picture: a longer one is time away
/// (the app in the background), which does not count.
pub const PRESENT_GAP_MS: u64 = 250;

/// How long a game in the foreground may go without showing a picture before it counts as a black screen.
pub const BLACK_SCREEN_MS: u64 = 3000;

impl Presents {
    /// The game ran in the foreground for [`BLACK_SCREEN_MS`] and more and no picture reached the screen.
    #[must_use]
    pub fn black_screen(&self) -> bool {
        // Made again after a black screen and still nothing shown: as black.
        self.rebuilt > 0 || (self.attempts > 0 && self.waited_ms >= BLACK_SCREEN_MS)
    }

    /// Counts a present that showed nothing (`unshown`), `gap_ms` after the one before (`None`: the first). The gap
    /// counts as waiting up to [`PRESENT_GAP_MS`]. Whether the surface is to be made again now: the first time this
    /// start's game has waited [`BLACK_SCREEN_MS`] with nothing shown (once a start), and the counts then start over, so
    /// a black screen that stays is seen again [`BLACK_SCREEN_MS`] later.
    pub fn count(&mut self, unshown: Unshown, gap_ms: Option<u64>) -> bool {
        self.attempts = self.attempts.saturating_add(1);
        let count = match unshown {
            Unshown::Skipped => &mut self.skipped,
            Unshown::Outdated => &mut self.outdated,
            Unshown::Lost => &mut self.lost,
            Unshown::Timeout => &mut self.timeout,
            Unshown::Occluded => &mut self.occluded,
        };
        *count = count.saturating_add(1);
        if let Some(gap) = gap_ms {
            self.waited_ms = self.waited_ms.saturating_add(gap.min(PRESENT_GAP_MS));
        }
        if self.rebuilt == 0 && self.attempts > 0 && self.waited_ms >= BLACK_SCREEN_MS {
            *self = Self { rebuilt: 1, ..Self::default() };
            return true;
        }
        false
    }

    /// A picture reached the screen: the counts start over.
    pub fn shown(&mut self) {
        *self = Self::default();
    }

    fn text(&self) -> String {
        format!(
            "attempts:{},skipped:{},outdated:{},lost:{},timeout:{},occluded:{},waited_ms:{},rebuilt:{}",
            self.attempts,
            self.skipped,
            self.outdated,
            self.lost,
            self.timeout,
            self.occluded,
            self.waited_ms,
            self.rebuilt
        )
    }

    fn parse(text: &str) -> Self {
        let mut presents = Self::default();
        for (key, value) in text.split(',').filter_map(|part| part.split_once(':')) {
            let value = value.trim();
            match key.trim() {
                "attempts" => presents.attempts = value.parse().unwrap_or(0),
                "skipped" => presents.skipped = value.parse().unwrap_or(0),
                "outdated" => presents.outdated = value.parse().unwrap_or(0),
                "lost" => presents.lost = value.parse().unwrap_or(0),
                "timeout" => presents.timeout = value.parse().unwrap_or(0),
                "occluded" => presents.occluded = value.parse().unwrap_or(0),
                "waited_ms" => presents.waited_ms = value.parse().unwrap_or(0),
                "rebuilt" => presents.rebuilt = value.parse().unwrap_or(0),
                _ => {}
            }
        }
        presents
    }

    /// The presents in one line for the report.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "presents: {} tried over {:.1} s in the foreground, none shown (skipped {}, outdated {}, lost {}, timeout {}, occluded {}); surface made again {} time(s)",
            self.attempts,
            self.waited_ms as f64 / 1000.0,
            self.skipped,
            self.outdated,
            self.lost,
            self.timeout,
            self.occluded,
            self.rebuilt
        )
    }
}

/// The lines of `extra` for the stage file (after [`stage_text`]'s).
#[must_use]
pub fn extra_text(extra: &StageExtra) -> String {
    let mut text = String::new();
    if let Some(pid) = extra.pid {
        let _ = writeln!(text, "pid={pid}");
    }
    if let Some(started) = extra.started_ms {
        let _ = writeln!(text, "started_ms={started}");
    }
    if extra.presents.attempts > 0 || extra.presents.rebuilt > 0 {
        let _ = writeln!(text, "presents={}", extra.presents.text());
    }
    if extra.returned_failed {
        text.push_str("returned=failed\n");
    }
    text
}

/// What else the stage file `text` says (a part it does not say, or not readably, is left out).
#[must_use]
pub fn parse_extra(text: &str) -> StageExtra {
    let value = |key: &str| {
        text.lines()
            .find_map(|line| line.trim().strip_prefix(key)?.strip_prefix('='))
            .map(str::trim)
    };
    StageExtra {
        pid: value("pid").and_then(|v| v.parse().ok()),
        started_ms: value("started_ms").and_then(|v| v.parse().ok()),
        presents: value("presents").map(Presents::parse).unwrap_or_default(),
        returned_failed: value("returned") == Some("failed"),
    }
}

/// What a start that left its stage file says of itself, besides how its process ended: each is its own evidence of
/// failing, however the process then ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Evidence {
    /// The game ran and showed nothing ([`Presents::black_screen`]).
    pub black_screen: bool,
    /// It panicked (its dialog up, the player swiped the app away).
    pub panicked: bool,
    /// Its run returned with a failure while its graphics were open ([`StageExtra::returned_failed`]): the process then
    /// ends as asked, with nothing the system calls a crash.
    pub returned_failed: bool,
    /// It stayed in the foreground [`STALL_MS`] and more after its graphics opened with no picture ([`stalled`]): stuck
    /// in a driver, say, until the player gave up.
    pub stalled: bool,
}

impl Evidence {
    /// Whether the report says the start stopped (and for how long it was in the foreground): only when stopping is all
    /// it says. A start that showed a black screen, panicked or returned a failure was not stopped, whatever the
    /// foreground time counted while its dialog was up.
    #[must_use]
    pub fn stopped_alone(self) -> bool {
        self.stalled && !self.black_screen && !self.panicked && !self.returned_failed
    }
}

/// Whether the stage file of `extra` was left by a version before this one (RC3 and earlier), which wrote no process
/// id: how its process ended cannot be looked up, and the player has updated since. It is not counted (no switch, no
/// dialog): this version's own starts always write their process id.
#[must_use]
pub fn left_by_earlier_version(extra: &StageExtra) -> bool {
    extra.pid.is_none()
}

/// How long a start may stay in the foreground after its graphics opened with no picture before it counts as stuck.
/// A start on the emulator goes from opening its window to its first picture in 0.6 to 1.3 s (11-20, Classic and
/// Enhanced with the HD pack), a phone faster; 8 s is several times that, and past the black screen's 3 s and the
/// surface made again, so that a player who leaves at once is not counted.
pub const STALL_MS: u64 = 8000;

/// Whether a start left at `stage` stayed `foreground_ms` in the foreground (since its graphics began to open) without a
/// picture long enough to count as stuck. The setup screens' window counts as the game's does: its stage file is
/// written as it opens and deleted by its first picture, so it never holds the player's time on those screens. While
/// the game loads (`Start`, before its window) it is not counted: that time is the game's own work.
#[must_use]
pub fn stalled(stage: Option<Stage>, foreground_ms: Option<u64>) -> bool {
    matches!(stage, Some(Stage::Setup | Stage::Window | Stage::Audio | Stage::FirstFrame))
        && foreground_ms.is_some_and(|ms| ms >= STALL_MS)
}

/// The foreground time the activity wrote (`foreground.txt`: pid, then the milliseconds in the foreground since the
/// graphics of the setup screens or of the game began to open) for the process `pid`; `None` for another process's or a file not understood.
#[must_use]
pub fn foreground_ms(text: &str, pid: u32) -> Option<u64> {
    let mut cols = text.trim().split('\t');
    let file_pid: u32 = cols.next()?.trim().parse().ok()?;
    let ms: u64 = cols.next()?.trim().parse().ok()?;
    (file_pid == pid).then_some(ms)
}

/// Whether a start that left its stage file counts as a failure of the graphics: unless the system says the process
/// was ended by the user or the system (Home and reclaimed, swiped away, Force stop, low memory, an update), which
/// says nothing of the graphics, and the start has no evidence of its own ([`Evidence`]). Without the system's word
/// (Android 10 and earlier, or no record of the process) it counts, as before.
#[must_use]
pub fn counts_as_failure(exit: Option<ExitKind>, evidence: Evidence) -> bool {
    let own =
        evidence.black_screen || evidence.panicked || evidence.returned_failed || evidence.stalled;
    own || exit != Some(ExitKind::Normal)
}

/// The file that remembers OpenGL ES for the installation `install`.
#[must_use]
pub fn memory_text(install: &str) -> String {
    format!("backend=gl\ninstall={install}\n")
}

/// Whether the memory file `text` remembers OpenGL ES for the installation `install` (a file of another installation,
/// or one not understood, remembers nothing).
#[must_use]
pub fn remembers_gl(text: Option<&str>, install: &str) -> bool {
    let Some(text) = text else { return false };
    let has = |line: &str| text.lines().any(|l| l.trim() == line);
    has("backend=gl") && has(&format!("install={install}"))
}

/// The key of an installation of the APK, from the path of the game's library and the size and time of its file:
/// every update or reinstall makes another.
#[must_use]
pub fn install_key(library: &str, size: u64, modified: u64) -> String {
    // FNV-1a, 64 bits.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in library.bytes().chain(size.to_le_bytes()).chain(modified.to_le_bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// What the next start is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    /// The last start died on Vulkan (or the automatic order): this one is on OpenGL ES.
    SwitchedToGl(Option<Stage>),
    /// The last start died on OpenGL ES too: neither backend starts the game here.
    BothFailed(Option<Stage>),
    /// The last start died on a backend the arguments file forced: nothing is changed.
    Failed(Option<Stage>),
}

/// What this start does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// The backend asked for: `Gl` sets `WGPU_BACKEND=gl`; `Auto` and `Forced` leave the environment as it is.
    pub backend: Backend,
    /// What the player is told before the start; `None`: nothing.
    pub notice: Option<Notice>,
    /// Whether OpenGL ES is remembered for this installation from now on.
    pub remember_gl: bool,
}

/// The decision of a start: `last` is what the last start left, `remembered` whether OpenGL ES is remembered for this
/// installation, `forced` whether `WGPU_BACKEND` was set before the guard (the arguments file), which it never
/// overrides.
#[must_use]
pub fn decide(last: &Last, remembered: bool, forced: bool) -> Decision {
    let usual = if remembered { Backend::Gl } else { Backend::Auto };
    let Last::Stopped { stage, backend, .. } = last else {
        return Decision {
            backend: if forced { Backend::Forced } else { usual },
            notice: None,
            remember_gl: remembered,
        };
    };
    let stage = *stage;
    if forced || *backend == Some(Backend::Forced) {
        // A forced backend says nothing of the automatic one: the start is reported, nothing is switched.
        return Decision {
            backend: if forced { Backend::Forced } else { usual },
            notice: Some(Notice::Failed(stage)),
            remember_gl: remembered,
        };
    }
    // On OpenGL ES already (or not known, with OpenGL ES remembered): both have failed. Stay on OpenGL ES.
    let on_gl = *backend == Some(Backend::Gl) || (backend.is_none() && remembered);
    let notice = if on_gl { Notice::BothFailed(stage) } else { Notice::SwitchedToGl(stage) };
    Decision { backend: Backend::Gl, notice: Some(notice), remember_gl: true }
}

// The English words of the notice: each is a row of the menus' tables (`ui-zh-tw.tsv`, `ui-zh-hk.tsv`).
const TITLE: &str = "The last start failed";
const STOPPED: &str = "The last start stopped before its first picture, at:";
const SWITCHED: &str = "This start uses OpenGL ES instead of Vulkan, and so will the next ones.";
const BOTH: &str = "It was on OpenGL ES, after Vulkan had failed: neither could start the game on this device. This start tries OpenGL ES again.";
const FORCED: &str =
    "The last start's graphics backend was set by the arguments file, so nothing is changed.";
/// The label of how the last process ended, and of the seconds from its start to its end.
pub const EXIT_REASON: &str = "Exit reason:";
pub const EXIT_SECONDS: &str = "Seconds from start to exit:";
/// A start whose game ran without showing a picture (`Presents::black_screen`).
pub const BLACK_SCREEN: &str = "The game ran, but no picture reached the screen.";
/// A start stuck in the foreground with its graphics open ([`stalled`]), and the label of how long.
pub const STALLED: &str = "The game stopped before its first picture.";
pub const FOREGROUND_SECONDS: &str = "Seconds in the foreground with no picture:";

/// Every English text of the notice, so a test can check the tables have them all.
pub const WORDS: &[&str] = &[
    TITLE,
    STOPPED,
    SWITCHED,
    BOTH,
    FORCED,
    EXIT_REASON,
    EXIT_SECONDS,
    BLACK_SCREEN,
    STALLED,
    FOREGROUND_SECONDS,
    "the setup screens",
    "loading the game",
    "opening the game's window and graphics",
    "opening the sound",
    "drawing the first picture",
    "an unknown step",
];

/// The dialog's title for a notice.
#[must_use]
pub fn notice_title(words: Option<&UiText>) -> String {
    tr(words, TITLE)
}

/// The lines that say what happened and what this start does.
#[must_use]
pub fn notice_lines(words: Option<&UiText>, notice: &Notice) -> Vec<String> {
    let (stage, then) = match notice {
        Notice::SwitchedToGl(stage) => (stage, SWITCHED),
        Notice::BothFailed(stage) => (stage, BOTH),
        Notice::Failed(stage) => (stage, FORCED),
    };
    let stopped = tr(words, STOPPED);
    let gap = if stopped.ends_with('：') { "" } else { " " };
    vec![format!("{stopped}{gap}{}", tr(words, Stage::words(*stage))), tr(words, then)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stopped(stage: Stage, backend: Backend) -> Last {
        parse_stage(Some(&stage_text(stage, backend, Some("Adreno (TM) 540 / Vulkan"))))
    }

    /// The stage file after a run returned at `stage` (`succeeded` or not), then the next start's decision.
    fn after_return(stage: Stage, succeeded: bool) -> Decision {
        let file = stage_text(stage, Backend::Auto, None);
        let left = stage_stays_after_return(Some(stage), succeeded).then_some(file);
        decide(&parse_stage(left.as_deref()), false, false)
    }

    #[test]
    fn a_run_that_returned_as_asked_leaves_no_failure_behind() {
        // A benchmark (drawn offscreen, never presented) or the player quitting while the game loads.
        for stage in Stage::ALL {
            assert!(!stage_stays_after_return(Some(stage), true), "{stage:?}");
            assert_eq!(
                after_return(stage, true),
                Decision { backend: Backend::Auto, notice: None, remember_gl: false }
            );
        }
        assert!(!stage_stays_after_return(None, true));
    }

    #[test]
    fn a_run_refused_before_its_graphics_is_not_a_graphics_failure() {
        // The ROM or a pack refused (main.rs's exit codes): reported then, never "both backends failed" later.
        assert!(!stage_stays_after_return(Some(Stage::Start), false));
        assert!(!stage_stays_after_return(None, false));
        assert_eq!(after_return(Stage::Start, false).notice, None);
        assert_eq!(after_return(Stage::Start, false).backend, Backend::Auto);
    }

    #[test]
    fn a_run_that_failed_with_its_graphics_open_still_switches() {
        for stage in [Stage::Setup, Stage::Window, Stage::Audio, Stage::FirstFrame] {
            assert!(stage_stays_after_return(Some(stage), false), "{stage:?}");
            assert_eq!(after_return(stage, false).notice, Some(Notice::SwitchedToGl(Some(stage))));
        }
    }

    /// The decision of a start after one that left its stage file at `stage` on `backend`, ended as `exit` says, with
    /// OpenGL ES `remembered` or not.
    fn after_exit(
        stage: Stage,
        backend: Backend,
        exit: Option<ExitKind>,
        remembered: bool,
    ) -> Decision {
        let last = stopped(stage, backend);
        let last = if counts_as_failure(exit, Evidence::default()) { last } else { Last::Fine };
        decide(&last, remembered, false)
    }

    #[test]
    fn the_user_or_the_system_ending_a_start_is_no_failure() {
        // Home then reclaimed, swiped away, Force stop: the stage file goes, nothing is switched, no dialog.
        for stage in Stage::ALL {
            for remembered in [false, true] {
                let backend = if remembered { Backend::Gl } else { Backend::Auto };
                let decision = after_exit(stage, backend, Some(ExitKind::Normal), remembered);
                assert_eq!(
                    decision,
                    Decision { backend, notice: None, remember_gl: remembered },
                    "{stage:?}, OpenGL ES remembered {remembered}"
                );
            }
        }
    }

    #[test]
    fn a_crash_still_switches_and_a_crash_on_gl_says_both_failed() {
        for stage in Stage::ALL {
            let first = after_exit(stage, Backend::Auto, Some(ExitKind::Crash), false);
            assert_eq!(
                first,
                Decision {
                    backend: Backend::Gl,
                    notice: Some(Notice::SwitchedToGl(Some(stage))),
                    remember_gl: true,
                }
            );
            let again = after_exit(stage, Backend::Gl, Some(ExitKind::Crash), true);
            assert_eq!(again.notice, Some(Notice::BothFailed(Some(stage))));
            assert_eq!(again.backend, Backend::Gl);
        }
    }

    #[test]
    fn without_the_systems_word_a_start_left_counts_as_before() {
        // Android 10 and earlier, no record of the process, or a reason the table does not know.
        for exit in [None, Some(ExitKind::Unknown)] {
            assert!(counts_as_failure(exit, Evidence::default()));
            let decision = after_exit(Stage::FirstFrame, Backend::Auto, exit, false);
            assert_eq!(decision.notice, Some(Notice::SwitchedToGl(Some(Stage::FirstFrame))));
        }
    }

    #[test]
    fn a_black_screen_or_a_panic_counts_whatever_ended_it() {
        let normal = Some(ExitKind::Normal);
        assert!(counts_as_failure(normal, Evidence { black_screen: true, ..Evidence::default() }));
        assert!(
            counts_as_failure(normal, Evidence { panicked: true, ..Evidence::default() }),
            "its dialog up, swiped away"
        );
        assert!(!counts_as_failure(normal, Evidence::default()));
        let presents =
            Presents { attempts: 180, lost: 180, waited_ms: 3000, ..Presents::default() };
        assert!(presents.black_screen());
        assert!(!Presents { waited_ms: 2999, ..presents }.black_screen(), "not long enough");
        assert!(!Presents { attempts: 0, ..presents }.black_screen(), "the loop never tried");
    }

    /// Presents that show nothing, one every `every_ms`, for `for_ms`; the times the surface was to be made again.
    fn unshown(presents: &mut Presents, every_ms: u64, for_ms: u64) -> u32 {
        let mut rebuilds = 0;
        for i in 0..for_ms / every_ms {
            if presents.count(Unshown::Lost, (i > 0).then_some(every_ms)) {
                rebuilds += 1;
            }
        }
        rebuilds
    }

    #[test]
    fn a_black_screen_makes_the_surface_again_once_a_start() {
        let mut presents = Presents::default();
        // 60 presents a second: nothing for 3 s, then the surface is made again, and the counts start over.
        assert_eq!(unshown(&mut presents, 16, 2900), 0, "not yet");
        assert!(!presents.black_screen());
        assert_eq!(unshown(&mut presents, 16, 200), 1, "made again at 3 s");
        assert_eq!(presents.rebuilt, 1);
        assert!(
            presents.attempts < 20 && presents.waited_ms < 250,
            "the counts started over: {presents:?}"
        );
        assert!(presents.black_screen(), "made again and still nothing shown: black");
        // Still nothing: never made again this start, however long.
        assert_eq!(unshown(&mut presents, 16, 20_000), 0);
        assert_eq!(presents.rebuilt, 1);
        assert!(presents.summary().contains("surface made again 1 time(s)"));
    }

    #[test]
    fn a_picture_shown_starts_the_counts_over() {
        let mut presents = Presents::default();
        unshown(&mut presents, 16, 3500);
        assert_eq!(presents.rebuilt, 1);
        presents.shown();
        assert_eq!(presents, Presents::default(), "recovered: nothing left to report");
        assert!(!presents.black_screen());
    }

    #[test]
    fn time_away_is_not_waiting() {
        let mut presents = Presents::default();
        // Ten presents a minute apart (the app in the background between them): 9 gaps of at most 250 ms.
        assert_eq!(unshown(&mut presents, 60_000, 600_000), 0);
        assert_eq!(presents.waited_ms, 9 * PRESENT_GAP_MS);
        assert!(!presents.black_screen());
        assert!(!presents.count(Unshown::Skipped, None), "the first present has no gap");
    }

    #[test]
    fn each_kind_of_present_that_showed_nothing_is_counted() {
        let mut presents = Presents::default();
        for unshown in [
            Unshown::Skipped,
            Unshown::Outdated,
            Unshown::Lost,
            Unshown::Timeout,
            Unshown::Occluded,
        ] {
            presents.count(unshown, Some(10));
        }
        assert_eq!(
            presents,
            Presents {
                attempts: 5,
                skipped: 1,
                outdated: 1,
                lost: 1,
                timeout: 1,
                occluded: 1,
                waited_ms: 50,
                rebuilt: 0,
            }
        );
    }

    #[test]
    fn the_stage_files_extra_lines_round_trip_and_bad_ones_are_left_out() {
        let extra = StageExtra {
            pid: Some(4321),
            started_ms: Some(1_700_000_000_000),
            presents: Presents {
                attempts: 90,
                skipped: 2,
                outdated: 1,
                lost: 80,
                timeout: 4,
                occluded: 3,
                waited_ms: 3100,
                rebuilt: 1,
            },
            returned_failed: true,
        };
        let text = stage_text(Stage::FirstFrame, Backend::Gl, Some("Adreno")) + &extra_text(&extra);
        assert_eq!(parse_extra(&text), extra);
        assert_eq!(
            parse_stage(Some(&text)),
            Last::Stopped {
                stage: Some(Stage::FirstFrame),
                backend: Some(Backend::Gl),
                gpu: Some("Adreno".to_owned())
            }
        );
        assert_eq!(
            parse_extra("pid=x\nstarted_ms=\npresents=lost:zz,attempts:3"),
            StageExtra {
                pid: None,
                started_ms: None,
                presents: Presents { attempts: 3, ..Presents::default() },
                returned_failed: false,
            }
        );
        assert_eq!(parse_extra(""), StageExtra::default());
        assert!(
            !extra_text(&StageExtra::default()).contains("presents"),
            "no presents line before the loop"
        );
        assert!(extra.presents.summary().contains("lost 80"));
    }

    #[test]
    fn a_run_that_returned_failing_with_its_graphics_open_still_switches() {
        // RC3's behaviour: the first picture's present failed (the surface failed validation), the run returned, the
        // stage file stayed; the process then ended as asked (EXIT_SELF, reclaimed, swiped away).
        let extra = StageExtra { returned_failed: true, ..StageExtra::default() };
        let text = stage_text(Stage::FirstFrame, Backend::Auto, None) + &extra_text(&extra);
        assert!(parse_extra(&text).returned_failed);
        let evidence =
            Evidence { returned_failed: parse_extra(&text).returned_failed, ..Evidence::default() };
        for exit in [Some(ExitKind::Normal), Some(ExitKind::Unknown), None] {
            assert!(counts_as_failure(exit, evidence), "{exit:?}");
        }
        let last = parse_stage(Some(&text));
        let decision = decide(&last, false, false);
        assert_eq!(decision.notice, Some(Notice::SwitchedToGl(Some(Stage::FirstFrame))));
        assert!(!parse_extra("returned=fine").returned_failed && !parse_extra("").returned_failed);
    }

    #[test]
    fn a_start_stuck_in_the_foreground_with_its_graphics_open_counts() {
        // The setup screens' window too: its stage file is gone at its first picture, so the player's time on those
        // screens is never in it.
        for stage in [Stage::Setup, Stage::Window, Stage::Audio, Stage::FirstFrame] {
            assert!(stalled(Some(stage), Some(STALL_MS)), "{stage:?}");
            assert!(!stalled(Some(stage), Some(STALL_MS - 1)), "{stage:?}: left at once");
            assert!(!stalled(Some(stage), None), "{stage:?}: not known");
        }
        for stage in [Some(Stage::Start), None] {
            assert!(!stalled(stage, Some(60_000)), "{stage:?}: before any window");
        }
        let evidence = Evidence { stalled: true, ..Evidence::default() };
        assert!(counts_as_failure(Some(ExitKind::Normal), evidence), "swiped away after 8 s");
    }

    #[test]
    fn a_setup_window_stuck_before_its_first_picture_switches_to_gl() {
        // A new player's first window: the driver stuck opening it, the player swiped the app away after 10 s.
        let text = stage_text(Stage::Setup, Backend::Auto, None)
            + &extra_text(&StageExtra { pid: Some(4321), ..StageExtra::default() });
        let foreground = foreground_ms("4321\t10000\n", 4321);
        let last = parse_stage(Some(&text));
        let Last::Stopped { stage, .. } = &last else { panic!("a stage file") };
        let evidence = Evidence { stalled: stalled(*stage, foreground), ..Evidence::default() };
        assert!(counts_as_failure(Some(ExitKind::Normal), evidence));
        assert_eq!(
            decide(&last, false, false).notice,
            Some(Notice::SwitchedToGl(Some(Stage::Setup)))
        );
    }

    #[test]
    fn the_report_says_stopped_only_when_nothing_else_happened() {
        let stalled = Evidence { stalled: true, ..Evidence::default() };
        assert!(stalled.stopped_alone());
        for other in [
            Evidence { black_screen: true, ..stalled },
            Evidence { panicked: true, ..stalled },
            Evidence { returned_failed: true, ..stalled },
        ] {
            assert!(
                !other.stopped_alone(),
                "{other:?}: its dialog was up while the time was counted"
            );
        }
        assert!(!Evidence::default().stopped_alone());
    }

    #[test]
    fn an_earlier_versions_stage_file_is_not_counted() {
        // RC3 wrote the stage, the backend and the adapter, no process id: the RC3 start it was left by cannot be
        // looked up, and the player has updated since.
        for backend in [Backend::Auto, Backend::Gl] {
            let rc3 = stage_text(Stage::FirstFrame, backend, Some("Adreno (TM) 710 / Vulkan"));
            assert!(left_by_earlier_version(&parse_extra(&rc3)), "{backend:?}");
        }
        let ours = stage_text(Stage::FirstFrame, Backend::Auto, None)
            + &extra_text(&StageExtra { pid: Some(4321), ..StageExtra::default() });
        assert!(!left_by_earlier_version(&parse_extra(&ours)), "this version's own");
    }

    #[test]
    fn the_foreground_file_is_the_processs_own() {
        assert_eq!(foreground_ms("4321\t9500\n", 4321), Some(9500));
        assert_eq!(foreground_ms("4321\t9500\n", 77), None, "another process's");
        assert_eq!(foreground_ms("", 4321), None);
        assert_eq!(foreground_ms("4321", 4321), None);
        assert_eq!(foreground_ms("x\t1", 4321), None);
    }

    #[test]
    fn a_start_that_drew_its_first_picture_changes_nothing() {
        let decision = decide(&parse_stage(None), false, false);
        assert_eq!(decision, Decision { backend: Backend::Auto, notice: None, remember_gl: false });
    }

    #[test]
    fn a_start_that_died_on_vulkan_switches_to_gl_and_remembers_it() {
        let last = stopped(Stage::Window, Backend::Auto);
        assert_eq!(
            last,
            Last::Stopped {
                stage: Some(Stage::Window),
                backend: Some(Backend::Auto),
                gpu: Some("Adreno (TM) 540 / Vulkan".to_owned()),
            }
        );
        let decision = decide(&last, false, false);
        assert_eq!(
            decision,
            Decision {
                backend: Backend::Gl,
                notice: Some(Notice::SwitchedToGl(Some(Stage::Window))),
                remember_gl: true,
            }
        );
    }

    #[test]
    fn gl_remembered_is_used_without_a_word() {
        let install = install_key(
            "/data/app/x/base.apk!/lib/arm64-v8a/libmain.so",
            46_000_000,
            1_700_000_000,
        );
        let memory = memory_text(&install);
        assert!(remembers_gl(Some(&memory), &install));
        let decision = decide(&Last::Fine, remembers_gl(Some(&memory), &install), false);
        assert_eq!(decision, Decision { backend: Backend::Gl, notice: None, remember_gl: true });
    }

    #[test]
    fn another_installation_forgets_gl_and_tries_vulkan_again() {
        let old = install_key("/data/app/a/base.apk!/lib/arm64-v8a/libmain.so", 46_000_000, 1);
        let new = install_key("/data/app/b/base.apk!/lib/arm64-v8a/libmain.so", 46_000_000, 1);
        let same_path_newer =
            install_key("/data/app/a/base.apk!/lib/arm64-v8a/libmain.so", 46_000_000, 2);
        assert_ne!(old, new);
        assert_ne!(old, same_path_newer);
        assert!(!remembers_gl(Some(&memory_text(&old)), &new));
        assert_eq!(decide(&Last::Fine, false, false).backend, Backend::Auto);
    }

    #[test]
    fn a_start_that_died_on_gl_too_says_both_failed_and_stays_on_gl() {
        let decision = decide(&stopped(Stage::FirstFrame, Backend::Gl), true, false);
        assert_eq!(
            decision,
            Decision {
                backend: Backend::Gl,
                notice: Some(Notice::BothFailed(Some(Stage::FirstFrame))),
                remember_gl: true,
            }
        );
        // And again, as long as it dies: never back to Vulkan.
        for _ in 0..3 {
            let again =
                decide(&stopped(Stage::Window, decision.backend), decision.remember_gl, false);
            assert_eq!(again.backend, Backend::Gl);
            assert!(matches!(again.notice, Some(Notice::BothFailed(_))));
        }
    }

    #[test]
    fn a_damaged_or_unknown_stage_file_still_counts_as_a_death() {
        for text in ["", "garbage", "stage=warp\nbackend=dx13\n", "\u{0}\u{1}stage", "backend=auto"]
        {
            let last = parse_stage(Some(text));
            let Last::Stopped { stage, .. } = &last else { panic!("{text:?} is a death") };
            assert_eq!(*stage, None, "{text:?}: no stage known");
            let decision = decide(&last, false, false);
            assert_eq!(decision.backend, Backend::Gl, "{text:?}");
            assert!(matches!(decision.notice, Some(Notice::SwitchedToGl(None))), "{text:?}");
        }
        // Not known which backend, but OpenGL ES remembered: it was OpenGL ES.
        let decision = decide(&parse_stage(Some("stage=audio")), true, false);
        assert_eq!(decision.notice, Some(Notice::BothFailed(Some(Stage::Audio))));
        // A memory file not understood remembers nothing.
        for text in ["", "backend=gl", "install=00\nbackend=vk"] {
            assert!(!remembers_gl(Some(text), "00"), "{text:?}");
        }
        assert!(!remembers_gl(None, "00"));
    }

    #[test]
    fn a_forced_backend_is_never_switched() {
        let now_forced = decide(&stopped(Stage::Window, Backend::Auto), false, true);
        assert_eq!(
            now_forced,
            Decision {
                backend: Backend::Forced,
                notice: Some(Notice::Failed(Some(Stage::Window))),
                remember_gl: false,
            }
        );
        let was_forced = decide(&stopped(Stage::Window, Backend::Forced), false, false);
        assert_eq!(
            was_forced,
            Decision {
                backend: Backend::Auto,
                notice: Some(Notice::Failed(Some(Stage::Window))),
                remember_gl: false,
            }
        );
        assert_eq!(decide(&Last::Fine, true, true).backend, Backend::Forced);
    }

    #[test]
    fn the_stage_file_round_trips_and_keeps_to_one_line_a_value() {
        for stage in Stage::ALL {
            for backend in [Backend::Auto, Backend::Gl, Backend::Forced] {
                let text = stage_text(stage, backend, Some("a\nb"));
                assert_eq!(
                    parse_stage(Some(&text)),
                    Last::Stopped {
                        stage: Some(stage),
                        backend: Some(backend),
                        gpu: Some("a b".to_owned())
                    }
                );
            }
        }
        assert_eq!(
            parse_stage(Some(&stage_text(Stage::Start, Backend::Auto, None))),
            Last::Stopped { stage: Some(Stage::Start), backend: Some(Backend::Auto), gpu: None }
        );
    }

    #[test]
    fn the_notice_says_where_it_stopped_and_what_this_start_does() {
        let lines = notice_lines(None, &Notice::SwitchedToGl(Some(Stage::Window)));
        assert_eq!(
            lines,
            [
                "The last start stopped before its first picture, at: opening the game's window and graphics",
                SWITCHED
            ]
        );
        let table = UiText::parse(include_str!("../../../translations/zh-tw/ui-zh-tw.tsv"));
        let lines = notice_lines(Some(&table), &Notice::BothFailed(None));
        assert!(lines[0].ends_with("：不明的步驟"), "{lines:?}");
        assert!(lines[1].contains("OpenGL ES"), "{lines:?}");
        assert_eq!(notice_title(Some(&table)), "上次啟動失敗");
    }

    #[test]
    fn both_tables_have_every_word_of_the_notice() {
        let kept = ["Vulkan", "OpenGL", "ES"];
        for (name, text) in [
            ("ui-zh-tw.tsv", include_str!("../../../translations/zh-tw/ui-zh-tw.tsv")),
            ("ui-zh-hk.tsv", include_str!("../../../translations/zh-hk/ui-zh-hk.tsv")),
        ] {
            let table = UiText::parse(text);
            for english in WORDS {
                let translated =
                    table.get(english).unwrap_or_else(|| panic!("{name} has no row {english:?}"));
                let mut rest = translated.clone();
                for word in kept {
                    rest = rest.replace(word, "");
                }
                assert!(
                    !rest.chars().any(|c| c.is_ascii_alphabetic()),
                    "{name}: {english:?} is {translated:?}"
                );
            }
        }
    }
}
