//! A start that fails on Android shows why: a system dialog with the report of `launch_report` (the build, the device,
//! the graphics adapter, the memory, and what went wrong, scrubbed of paths), also put on the clipboard so the player
//! can paste it into a report. Without it the app closed on a black screen and the reason was only in `adb logcat`,
//! which a player cannot read.
//!
//! Two ways a start fails: the game ends with an error (`run` returns a failure; its message went to stderr, which
//! `android.rs` forwards here line by line, or the setup screens failed) and the dialog shows after it; or the program
//! panics, and the panic hook shows the dialog before the process aborts. Only the first panic of the process reports;
//! a later one (another thread, or a panic while reporting) is only logged, and the dialog waits a bounded time, so a
//! dialog that cannot show never holds the process.
//!
//! A start can also die where nothing reports it (a crash inside a graphics driver). So each start writes how far it got
//! to a stage file in the data folder ([`stage`]), which its first picture deletes ([`first_frame`]); [`begin`] reads
//! what the last start left and lets `launch_guard` decide the graphics backend of this one: on OpenGL ES after a start
//! that died before its first picture, remembered for this installation, with a dialog that says so.
#![cfg(target_os = "android")]
#![allow(unsafe_code)]

use crate::exit_info::{self, ExitKind, ExitRecord};
use crate::launch_guard::{self, Backend, Last, Presents, Stage, StageExtra, Unshown};
use crate::launch_report::{self, Facts, Headline};
use ssd_menus::ui_text::UiText;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// The lines of stderr kept, and the most a report shows.
const STDERR_KEPT: usize = 12;
const STDERR_SHOWN: usize = 6;
/// The longest line of details a report shows (the dialog does not scroll far).
const DETAIL_CHARS: usize = 400;
/// How long a dialog shown from the panic hook may hold the process before it ends anyway.
const PANIC_WAIT: Duration = Duration::from_secs(180);
/// How long the dialog after a failed run may wait for the player.
const ERROR_WAIT: Duration = Duration::from_secs(600);
/// In the data folder: how far the start in progress got (deleted by its first picture).
const STAGE_FILE: &str = "launch-stage.txt";
/// In the data folder: OpenGL ES remembered for an installation.
const MEMORY_FILE: &str = "gpu-backend.txt";
/// In the data folder: the panic of the last start, for the next start's dialog.
const CRASH_FILE: &str = "last-crash.txt";
/// In the data folder: the foreground time since the graphics (of the setup screens or of the game) began to open,
/// written by the activity (`GameActivity`) between the messages [`COUNT_START`] and [`COUNT_STOP`].
const FOREGROUND_FILE: &str = "foreground.txt";
/// The activity's commands (`GameActivity.onUnhandledMessage`): count the foreground time from now, stop counting.
const COUNT_START: u32 = 0x8004;
const COUNT_STOP: u32 = 0x8005;
/// In the data folder: how the app's last processes ended (Android 11+), written by the activity (`GameActivity`).
const EXITS_FILE: &str = "process-exits.txt";
/// How often the stage file is written again while the game's presents show nothing.
const PRESENTS_WRITE: Duration = Duration::from_secs(1);

/// What a present of the window came to (`video.rs`, `Video::present`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentOutcome {
    /// A picture reached the screen.
    Shown,
    /// Not tried: the window is minimised, of no size, or has no surface yet.
    Skipped,
    /// The surface's texture was refused: outdated, lost, timed out, occluded.
    Outdated,
    Lost,
    Timeout,
    Occluded,
}

#[derive(Default)]
struct State {
    /// The menus' table of the player's language (`None`: English).
    words: Option<Arc<UiText>>,
    /// The adapter the window's graphics opened on, as the report names it.
    gpu: String,
    /// Backends that failed before one opened (kept for a report, not a failure by themselves).
    gpu_failures: Vec<String>,
    /// Failures noted by the app (the setup screens), each a reason to report.
    notes: Vec<String>,
    /// The last lines of stderr.
    stderr: VecDeque<String>,
    /// The data folder, where the stage, memory and crash files are.
    dir: Option<PathBuf>,
    /// The backend this process asked for (`begin`).
    backend: Option<Backend>,
    /// The stage of the start in progress, until its first picture.
    stage: Option<Stage>,
    /// When this process started (ms since 1970), for the stage file.
    started_ms: Option<u64>,
    /// The presents of the start in progress that showed nothing, the last one's time and the stage file's last write.
    presents: Presents,
    last_present: Option<std::time::Instant>,
    presents_written: Option<std::time::Instant>,
    /// The black screen was logged (once a start).
    black_logged: bool,
    /// The run returned with a failure while its graphics were open: written to the stage file it keeps.
    returned_failed: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
/// Set by the first report of the process (a panic, or a failed run): a second one only logs.
static REPORTED: AtomicBool = AtomicBool::new(false);

fn state() -> MutexGuard<'static, Option<State>> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    f(state().get_or_insert_with(State::default))
}

/// As [`with`], for the panic hook: `None` when the state is held (the panic struck while it was), never waiting.
fn try_with<T>(f: impl FnOnce(&mut State) -> T) -> Option<T> {
    let mut guard = match STATE.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        Err(std::sync::TryLockError::WouldBlock) => return None,
    };
    Some(f(guard.get_or_insert_with(State::default)))
}

/// Reads what the last start left in the data folder `dir` and decides this start's graphics backend
/// (`launch_guard::decide`): sets `WGPU_BACKEND=gl` when it is OpenGL ES, remembers that for this installation, and
/// shows the dialog that says why. Called once, before any window, while the process has no other thread that reads
/// the environment.
pub fn begin(dir: &Path) {
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).ok();
    let forced = std::env::var_os("WGPU_BACKEND").is_some();
    let stage_text = std::fs::read(dir.join(STAGE_FILE))
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
    let mut last = launch_guard::parse_stage(stage_text.as_deref());
    let extra = launch_guard::parse_extra(stage_text.as_deref().unwrap_or_default());
    // How that start's process ended (Android 11+): the user or the system ending it is no failure.
    let exits = exit_info::parse_exits(&read(EXITS_FILE).unwrap_or_default());
    let exit = extra.pid.and_then(|pid| exit_info::find(&exits, pid, extra.started_ms)).cloned();
    let kind = exit.as_ref().map(|e| exit_info::classify(e.reason, e.status));
    let panicked = dir.join(CRASH_FILE).is_file();
    // How long it stayed in the foreground after its graphics began to open, with no picture.
    let foreground = extra.pid.and_then(|pid| {
        launch_guard::foreground_ms(&read(FOREGROUND_FILE).unwrap_or_default(), pid)
    });
    let last_stage = match &last {
        Last::Stopped { stage, .. } => *stage,
        Last::Fine => None,
    };
    let evidence = launch_guard::Evidence {
        black_screen: extra.presents.black_screen(),
        panicked,
        returned_failed: extra.returned_failed,
        stalled: launch_guard::stalled(last_stage, foreground),
    };
    if matches!(last, Last::Stopped { .. }) && launch_guard::left_by_earlier_version(&extra) {
        log::info!(
            "the stage file was left by an earlier version (no process id): not counted, no dialog"
        );
        last = Last::Fine;
    } else if matches!(last, Last::Stopped { .. }) {
        log::info!(
            "the last start's process ended {exit:?} ({kind:?}); its presents {:?}",
            extra.presents
        );
        if !launch_guard::counts_as_failure(kind, evidence) {
            log::info!(
                "the last start was ended by the user or the system: not a failure, no dialog"
            );
            last = Last::Fine;
        }
    }
    let install = install_key();
    let remembered = launch_guard::remembers_gl(read(MEMORY_FILE).as_deref(), &install);
    let decision = launch_guard::decide(&last, remembered, forced);
    log::info!("last start {last:?}, OpenGL ES remembered {remembered}: {decision:?}");
    if decision.remember_gl
        && !remembered
        && let Err(error) =
            std::fs::write(dir.join(MEMORY_FILE), launch_guard::memory_text(&install))
    {
        log::warn!("OpenGL ES was not remembered: {error}");
    }
    if decision.backend == Backend::Gl && !forced {
        // SAFETY: called before any thread that reads the environment starts (`SDL_main`, as its other `set_var`).
        unsafe { std::env::set_var("WGPU_BACKEND", "gl") };
    }
    let _ = std::fs::remove_file(dir.join(STAGE_FILE));
    let crash = read(CRASH_FILE);
    let _ = std::fs::remove_file(dir.join(CRASH_FILE));
    let started_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|since| u64::try_from(since.as_millis()).ok());
    let words = with(|state| {
        state.dir = Some(dir.to_path_buf());
        state.backend = Some(decision.backend);
        state.started_ms = started_ms;
        state.words.clone()
    });
    if let Some(notice) = &decision.notice {
        let words = words.as_deref();
        let mut lead = launch_guard::notice_lines(words, notice);
        lead.extend(exit_lines(words, exit.as_ref(), &extra));
        // Only when stopping is all it did: a black screen, a panic or a failure returned are their own reports.
        if let Some(ms) = foreground.filter(|_| evidence.stopped_alone()) {
            lead.push(launch_report::tr(words, launch_guard::STALLED));
            let seconds = format!("{:.1}", ms as f64 / 1000.0);
            lead.push(launch_report::labelled(words, launch_guard::FOREGROUND_SECONDS, &seconds));
        }
        let gpu = match &last {
            Last::Stopped { gpu, .. } => gpu.clone().unwrap_or_default(),
            Last::Fine => String::new(),
        };
        let mut details: Vec<String> = Vec::new();
        // The system's description only for a crash: for an end by the user or the system it may name other apps'
        // providers or processes.
        if let Some(exit) =
            exit.as_ref().filter(|e| exit_info::classify(e.reason, e.status) == ExitKind::Crash)
        {
            details.extend((!exit.description.is_empty()).then(|| exit.description.clone()));
            details.extend(trace_summary(dir, exit));
        }
        if extra.presents.attempts > 0 {
            details.push(extra.presents.summary());
        }
        details.extend(crash.as_deref().unwrap_or_default().lines().map(str::to_owned));
        let details: Vec<String> =
            details.iter().map(|line| clip(&launch_report::scrub(line))).collect();
        let text = launch_report::report(words, &lead, &facts(gpu), &details);
        dialog(&launch_guard::notice_title(words), &text, ERROR_WAIT);
    }
}

/// The notice's lines of how the last start's process ended: the reason and the seconds from its start to its end
/// (Android 11+), and the black screen when its game ran without showing a picture.
fn exit_lines(
    words: Option<&UiText>,
    exit: Option<&ExitRecord>,
    extra: &StageExtra,
) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(exit) = exit {
        let reason = format!(
            "{} ({})",
            launch_report::tr(words, exit_info::reason_words(exit.reason, exit.status)),
            exit_info::reason_name(exit.reason)
        );
        lines.push(launch_report::labelled(words, launch_guard::EXIT_REASON, &reason));
        if let Some(started) = extra.started_ms.filter(|started| exit.time_ms >= *started) {
            let seconds = format!("{:.1}", (exit.time_ms - started) as f64 / 1000.0);
            lines.push(launch_report::labelled(words, launch_guard::EXIT_SECONDS, &seconds));
        }
    }
    if extra.presents.black_screen() {
        lines.push(launch_report::tr(words, launch_guard::BLACK_SCREEN));
    }
    lines
}

/// What the trace of `exit` says (a native crash's tombstone, an ANR's stacks), from its file in `dir`.
fn trace_summary(dir: &Path, exit: &ExitRecord) -> Vec<String> {
    // The activity's own name for the file: no folder in it.
    if exit.trace.is_empty() || exit.trace.contains(['/', '\\']) {
        return Vec::new();
    }
    let Ok(bytes) = std::fs::read(dir.join(&exit.trace)) else { return Vec::new() };
    if exit_info::classify(exit.reason, exit.status) != ExitKind::Crash {
        return Vec::new();
    }
    if exit.reason == exit_info::CRASH_NATIVE {
        exit_info::tombstone_summary(&bytes).unwrap_or_default()
    } else {
        exit_info::anr_summary(&String::from_utf8_lossy(&bytes))
    }
}

/// The start in progress has got to `stage`: the stage file says so (with the backend and the adapter), until the
/// first picture deletes it.
pub fn stage(stage: Stage) {
    let file = with(|state| {
        if matches!(stage, Stage::Setup | Stage::Start) {
            // A run of its own: its adapter is not known yet, and the last run's failures are not its own.
            state.gpu.clear();
            state.gpu_failures.clear();
            state.stderr.clear();
            state.presents = Presents::default();
            state.last_present = None;
            state.presents_written = None;
            state.black_logged = false;
            state.returned_failed = false;
        }
        state.stage = Some(stage);
        stage_file(state)
    });
    write_stage(file);
    if matches!(stage, Stage::Setup | Stage::Window) {
        // The activity counts the foreground time from now (the setup screens' window or the game's begins to open): a
        // start stuck with its graphics open is told apart from one the player left at once.
        // SAFETY: a plain SDL call with a command the activity knows.
        unsafe { sdl3::sys::system::SDL_SendAndroidMessage(COUNT_START, 0) };
    }
}

/// The activity stops counting the foreground time: the start is over, or a dialog of its own is up (the time the
/// player reads it is not the game's).
fn stop_counting() {
    // SAFETY: a plain SDL call with a command the activity knows.
    unsafe { sdl3::sys::system::SDL_SendAndroidMessage(COUNT_STOP, 0) };
}

/// The stage file of `state`'s start in progress: where, and what it says.
fn stage_file(state: &State) -> Option<(PathBuf, String)> {
    let (Some(dir), Some(stage)) = (&state.dir, state.stage) else { return None };
    let mut text =
        launch_guard::stage_text(stage, state.backend.unwrap_or(Backend::Auto), Some(&state.gpu));
    text.push_str(&launch_guard::extra_text(&StageExtra {
        pid: Some(std::process::id()),
        started_ms: state.started_ms,
        presents: state.presents,
        returned_failed: state.returned_failed,
    }));
    Some((dir.join(STAGE_FILE), text))
}

/// Writes a stage file, outside the state's lock. Not synced: a crash of the process leaves what it wrote to the
/// system's cache, which is all a crash of the game (not of the phone) needs.
/// The stage file is written and deleted on SDL's main thread only: called from another thread, a write that
/// started earlier could land after a later one, and an older stage would overwrite the newer.
fn write_stage(file: Option<(PathBuf, String)>) {
    let Some((path, text)) = file else { return };
    // Written aside and renamed: a crash in the middle of the write never leaves half a file.
    let part = path.with_extension("part");
    let written = std::fs::write(&part, text).and_then(|()| std::fs::rename(&part, &path));
    if let Err(error) = written {
        log::warn!("the stage file was not written: {error}");
    }
}

/// Deletes the stage file of the start in progress, if there is one: the start is over. On SDL's main thread only,
/// as `write_stage`: from another thread an older stage could overwrite the newer.
fn end_stage(why: &str) {
    // the folder is cloned only when a stage was taken (once a start), not on every frame's call
    let ended = with(|state| state.stage.take().and_then(|_| state.dir.clone()));
    if let Some(dir) = ended {
        let _ = std::fs::remove_file(dir.join(STAGE_FILE));
        log::info!("{why}");
        stop_counting();
    }
}

/// A window presented a picture: the start in progress has worked, and its stage file goes.
pub fn first_frame() {
    // A lock and a check a frame: nothing else is done once the start is over.
    end_stage("the first picture is drawn: the start worked");
}

/// A present of the window came to `outcome` (on SDL's main thread, as the stage file's writes). While a start has
/// not shown its first picture, the presents that showed nothing are counted, with the time the game waited in the
/// foreground (`launch_guard::Presents::count`), and written to the stage file once a second: a game that runs and
/// never shows a picture (a black screen) is told apart from a crash at the next start. After 3 s of nothing in the
/// foreground the surface is made again, once a start: returns `true` then, and the window drops its surface.
pub fn present_outcome(outcome: PresentOutcome) -> bool {
    let unshown = match outcome {
        PresentOutcome::Shown => {
            let rebuilt = with(|state| {
                let rebuilt = state.stage.is_some() && state.presents.rebuilt > 0;
                state.presents.shown();
                rebuilt
            });
            if rebuilt {
                log::warn!("the picture is back after the surface was made again");
            }
            first_frame();
            return false;
        }
        PresentOutcome::Skipped => Unshown::Skipped,
        PresentOutcome::Outdated => Unshown::Outdated,
        PresentOutcome::Lost => Unshown::Lost,
        PresentOutcome::Timeout => Unshown::Timeout,
        PresentOutcome::Occluded => Unshown::Occluded,
    };
    let now = std::time::Instant::now();
    let (rebuild, file) = with(|state| {
        if state.stage.is_none() {
            return (false, None);
        }
        let gap = state.last_present.map(|last| now.duration_since(last).as_millis() as u64);
        state.last_present = Some(now);
        let rebuild = state.presents.count(unshown, gap);
        if rebuild {
            log::warn!(
                "no picture reached the screen in 3 s: the surface is made again (once a start)"
            );
        } else if state.presents.waited_ms >= launch_guard::BLACK_SCREEN_MS && !state.black_logged {
            state.black_logged = true;
            log::warn!("still no picture: {}", state.presents.summary());
        }
        let due = rebuild
            || state.presents_written.is_none_or(|at| now.duration_since(at) >= PRESENTS_WRITE);
        if !due {
            return (rebuild, None);
        }
        state.presents_written = Some(now);
        (rebuild, stage_file(state))
    });
    write_stage(file);
    rebuild
}

/// A run (of the setup screens or of the game) returned, `succeeded` or not, before or after its first picture: its
/// stage file goes unless the failure is the graphics' (`launch_guard::stage_stays_after_return`). A run that ended as
/// asked (the player quit while it loaded, a benchmark drawn offscreen) or was refused before any graphics (the ROM, a
/// pack) is not counted against the backend at the next start.
pub fn run_returned(succeeded: bool) {
    let stage = with(|state| state.stage);
    if launch_guard::stage_stays_after_return(stage, succeeded) {
        log::warn!(
            "the run failed at {stage:?}: the next start counts it as the graphics' failure"
        );
        // The stage file says so: the process then ends as asked, which alone would not count.
        let file = with(|state| {
            state.returned_failed = true;
            stage_file(state)
        });
        write_stage(file);
    } else {
        end_stage("the run returned: its stage file goes");
    }
}

/// Whether the app noted a failure since the last report.
pub fn has_notes() -> bool {
    with(|state| !state.notes.is_empty())
}

/// The key of this installation of the APK: the path of the game's library (`dladdr`) and the size and time of the
/// file that holds it (the APK, or the library unpacked beside it).
fn install_key() -> String {
    // SAFETY: `Dl_info` is plain data, for which all zeroes is a valid value.
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    // SAFETY: `dladdr` looks up an address of this library and fills `info`; its name lives as long as the library.
    let found = unsafe { libc::dladdr(install_key as *const std::ffi::c_void, &raw mut info) } != 0;
    let library = if found && !info.dli_fname.is_null() {
        // SAFETY: a NUL-terminated name from `dladdr`.
        unsafe { std::ffi::CStr::from_ptr(info.dli_fname) }.to_string_lossy().into_owned()
    } else {
        String::new()
    };
    let file = library.split_once('!').map_or(library.as_str(), |(apk, _)| apk);
    let (size, modified) = std::fs::metadata(file).map_or((0, 0), |meta| {
        let modified = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |since| since.as_secs());
        (meta.len(), modified)
    });
    launch_guard::install_key(&library, size, modified)
}

/// The words of the dialog from now on: the menus' table of the language the player chose (as the setup screens have
/// it). Before this is called (a panic very early) the dialog is in English.
pub fn set_words(words: Option<Arc<UiText>>) {
    with(|state| state.words = words);
}

/// The graphics opened on this adapter (the stage file names it, so a crash in its driver is reported with it).
pub fn gpu_opened(text: String) {
    let file = with(|state| {
        state.gpu = text;
        stage_file(state)
    });
    write_stage(file);
}

/// A graphics backend that failed before the next was tried.
pub fn gpu_failed(text: String) {
    with(|state| state.gpu_failures.push(text));
}

/// A failure the app noted itself: the run is reported even if its exit code says nothing.
pub fn note(text: String) {
    with(|state| state.notes.push(text));
}

/// A line the program wrote to stderr (`android.rs` forwards them).
pub fn stderr_line(line: &str) {
    with(|state| {
        if state.stderr.len() == STDERR_KEPT {
            state.stderr.pop_front();
        }
        state.stderr.push_back(line.to_owned());
    });
}

/// After a run of the game (or of the setup screens): when it `failed` or noted a failure, shows the report and
/// returns `true`.
pub fn after_run(failed: bool) -> bool {
    if !failed && with(|state| state.notes.is_empty()) {
        return false;
    }
    // The stderr thread may still be passing on the last lines.
    std::thread::sleep(Duration::from_millis(200));
    if REPORTED.swap(true, Ordering::SeqCst) {
        return false;
    }
    stop_counting();
    let (words, gpu, details) = with(|state| {
        let notes = std::mem::take(&mut state.notes);
        let stderr: Vec<String> = state.stderr.drain(..).collect();
        let mut details = state.gpu_failures.clone();
        // The app's own notes say what failed; without one (an exit code with its reason printed), the last lines of
        // stderr do (with whatever a driver printed there too).
        if notes.is_empty() {
            details.extend(stderr.iter().rev().take(STDERR_SHOWN).rev().cloned());
        } else {
            details.extend(notes);
        }
        (state.words.clone(), state.gpu.clone(), details)
    });
    show(words.as_deref(), gpu, Headline::CouldNotStart, &details, ERROR_WAIT);
    // A later run of the same process (the launch loop) reports again.
    REPORTED.store(false, Ordering::SeqCst);
    true
}

/// The panic hook: logs the panic, then (the first panic only) shows the report and ends the process, whatever thread
/// panicked: a worker's panic would otherwise leave the game running under a dialog that says it stopped, and every
/// later failure unreported. A later panic (another thread, while the first one's dialog shows) is only logged.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        log::error!("panic: {info}");
        if REPORTED.swap(true, Ordering::SeqCst) {
            return;
        }
        let thread = std::thread::current();
        let details = vec![format!("panic ({}): {info}", thread.name().unwrap_or("SDL"))];
        // Never waits for the state: the panic may have struck while it was held.
        let (words, gpu, dir, stage) = try_with(|state| {
            (state.words.clone(), state.gpu.clone(), state.dir.clone(), state.stage)
        })
        .unwrap_or_default();
        if let Some(dir) = dir {
            // For the next start's dialog, in case this one cannot show.
            let scrubbed: Vec<String> =
                details.iter().map(|line| launch_report::scrub(line)).collect();
            let _ = std::fs::write(dir.join(CRASH_FILE), scrubbed.join("\n"));
            // As a run that returned with a failure: a panic before any graphics (or after the first picture, when
            // there is no stage file) does not switch the next start to OpenGL ES.
            if !launch_guard::stage_stays_after_return(stage, false) {
                let _ = std::fs::remove_file(dir.join(STAGE_FILE));
            }
        }
        stop_counting();
        show(words.as_deref(), gpu, Headline::Panicked, &details, PANIC_WAIT);
        std::process::abort();
    }));
}

/// The facts of this device and build, with the graphics adapter `gpu`.
pub fn facts(gpu: String) -> Facts {
    let chip = [property("ro.soc.model"), property("ro.board.platform"), property("ro.hardware")]
        .into_iter()
        .find(|value| !value.trim().is_empty())
        .unwrap_or_default();
    Facts {
        version: option_env!("SSD_APK_VERSION").unwrap_or_default().to_owned(),
        manufacturer: property("ro.product.manufacturer"),
        model: property("ro.product.model"),
        android: property("ro.build.version.release"),
        api: property("ro.build.version.sdk"),
        chip,
        gpu,
        memory_mb: memory_mb(),
    }
}

/// `line` cut to [`DETAIL_CHARS`] characters.
fn clip(line: &str) -> String {
    match line.char_indices().nth(DETAIL_CHARS) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line.to_owned(),
    }
}

/// Shows the report for `headline` with `details` (scrubbed here), and puts it on the clipboard.
fn show(
    words: Option<&UiText>,
    gpu: String,
    headline: Headline,
    details: &[String],
    wait: Duration,
) {
    let details: Vec<String> =
        details.iter().map(|line| clip(&launch_report::scrub(line))).collect();
    let lead = [launch_report::headline(words, headline)];
    let text = launch_report::report(words, &lead, &facts(gpu), &details);
    dialog(&launch_report::title(words, headline), &text, wait);
}

/// Puts `text` on the clipboard and shows it in a system dialog under `title`, waiting at most `wait` for the player to
/// close it. The dialog runs on a thread of its own, so a dialog that cannot show (the activity's thread held) costs
/// the wait, not the process.
pub fn dialog(title: &str, text: &str, wait: Duration) {
    use sdl3::sys::init::{SDL_INIT_VIDEO, SDL_InitSubSystem, SDL_QuitSubSystem};
    log::warn!("report: {title}\n{text}");
    // The clipboard is the video subsystem's: held for the copy (SDL counts the holds, so a window's is not ended).
    // SAFETY: plain SDL calls; the hold taken is given back below.
    let video = unsafe { SDL_InitSubSystem(SDL_INIT_VIDEO) };
    let clip = std::ffi::CString::new(text.replace('\0', " ")).unwrap_or_default();
    // SAFETY: a NUL-terminated string, which SDL copies.
    if !unsafe { sdl3::sys::clipboard::SDL_SetClipboardText(clip.as_ptr()) } {
        log::warn!("the report was not put on the clipboard: {}", sdl3::get_error());
    }
    if video {
        // SAFETY: the hold taken above.
        unsafe { SDL_QuitSubSystem(SDL_INIT_VIDEO) };
    }
    let (done, finished) = std::sync::mpsc::channel();
    let (title, text) = (title.to_owned(), text.to_owned());
    let shown = std::thread::Builder::new().name("report dialog".to_owned()).spawn(move || {
        if let Err(error) = sdl3::messagebox::show_simple_message_box(
            sdl3::messagebox::MessageBoxFlag::ERROR,
            &title,
            &text,
            None,
        ) {
            log::warn!("the report dialog did not show: {error}");
        }
        let _ = done.send(());
    });
    if shown.is_ok() && finished.recv_timeout(wait).is_err() {
        log::warn!("the report dialog was not closed in {} s: going on", wait.as_secs());
    }
}

/// The system property `name` (empty when it has none).
pub fn property(name: &str) -> String {
    let Ok(name) = std::ffi::CString::new(name) else { return String::new() };
    let mut value = [0 as std::ffi::c_char; 92]; // PROP_VALUE_MAX
    // SAFETY: `name` is NUL-terminated and `value` has the room the call writes (PROP_VALUE_MAX bytes).
    if unsafe { libc::__system_property_get(name.as_ptr(), value.as_mut_ptr()) } <= 0 {
        return String::new();
    }
    // SAFETY: the call wrote a NUL-terminated value into `value`, which is zeroed beyond it.
    unsafe { std::ffi::CStr::from_ptr(value.as_ptr()) }.to_string_lossy().trim().to_owned()
}

/// The memory in megabytes, total and available, from `/proc/meminfo`.
fn memory_mb() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
            .map(|kib| kib / 1024)
    };
    Some((field("MemTotal:")?, field("MemAvailable:")?))
}

/// Also, in `video.rs`: `present-error` (the first present fails, the run returns with an error), `blackscreen` and
/// `blackscreen-once`.
/// Test builds only (`debug_assertions`: never compiled into a release APK): `SSD_TEST_FAIL=panic`, `abort`, `quit` (the
/// quit event SDL sends when the activity closes) or `thread-panic` (a worker thread that panics three seconds later,
/// after the first picture), set
/// through the arguments file (`ENV:SSD_TEST_FAIL=abort`, which only a debuggable build lets `adb` write), fails the
/// start here, after the game's window opened and before its first picture, so the report can be tried on an emulator.
/// `abort` ends the process as a crash in a driver would, without the panic hook.
#[cfg(debug_assertions)]
pub fn test_failure() {
    match std::env::var("SSD_TEST_FAIL").as_deref() {
        Ok("panic") => panic!("SSD_TEST_FAIL=panic: a panic before the first frame"),
        Ok("quit") => {
            // The activity closed while the game loads: SDL's quit event, before the first picture.
            log::warn!("SSD_TEST_FAIL=quit: the game is asked to quit before the first frame");
            // SAFETY: `SDL_Event` is a plain C union, for which all zeroes is a valid value.
            let mut event: sdl3::sys::events::SDL_Event = unsafe { std::mem::zeroed() };
            event.r#type = sdl3::sys::events::SDL_EVENT_QUIT.into();
            // SAFETY: a quit event, which SDL copies into its queue.
            unsafe { sdl3::sys::events::SDL_PushEvent(&raw mut event) };
        }
        Ok("segv") => {
            log::warn!("SSD_TEST_FAIL=segv: a write through a null pointer before the first frame");
            // SAFETY: none: the point is the fault (a test build only).
            unsafe { std::ptr::null_mut::<u8>().write_volatile(1) };
        }
        Ok("anr") => {
            // The activity's thread sleeps (`GameActivity`, debuggable builds only): a touch then gets no answer.
            log::warn!("SSD_TEST_FAIL=anr: the activity's thread is asked to sleep");
            // SAFETY: a plain SDL call with a command the activity knows.
            unsafe { sdl3::sys::system::SDL_SendAndroidMessage(0x8003, 0) };
            // And the game waits too, so the start has not drawn its first picture when the system ends it.
            std::thread::sleep(Duration::from_secs(70));
        }
        Ok("stall") => {
            // The game's thread stops after opening its window, before any present: stuck as in a driver.
            log::warn!("SSD_TEST_FAIL=stall: the game's thread sleeps before its first picture");
            std::thread::sleep(Duration::from_secs(600));
        }
        Ok("thread-panic") => {
            // A worker that panics after the first picture: the process must end, and the next start not switch.
            let _ = std::thread::Builder::new().name("test worker".to_owned()).spawn(|| {
                std::thread::sleep(Duration::from_secs(3));
                panic!("SSD_TEST_FAIL=thread-panic: a worker thread panics");
            });
        }
        Ok("abort") => {
            log::warn!("SSD_TEST_FAIL=abort: the process ends before the first frame");
            std::process::abort();
        }
        _ => {}
    }
}

/// Test builds only: `SSD_TEST_FAIL=blackscreen` makes every present of the window come to nothing (as a lost surface),
/// so the game runs with a black screen; `blackscreen-once` does until the surface is made again
/// ([`test_surface_rebuilt`]).
#[cfg(debug_assertions)]
pub fn test_black_screen() -> bool {
    test_black_mode().load(Ordering::Relaxed) != 0
}

/// Test builds only: the surface was made again after a black screen (`blackscreen-once` then shows pictures).
#[cfg(debug_assertions)]
pub fn test_surface_rebuilt() {
    let _ = test_black_mode().compare_exchange(2, 0, Ordering::Relaxed, Ordering::Relaxed);
}

/// 0 off, 1 `blackscreen`, 2 `blackscreen-once` (until the surface is made again).
#[cfg(debug_assertions)]
fn test_black_mode() -> &'static std::sync::atomic::AtomicU8 {
    static MODE: std::sync::OnceLock<std::sync::atomic::AtomicU8> = std::sync::OnceLock::new();
    MODE.get_or_init(|| {
        std::sync::atomic::AtomicU8::new(match std::env::var("SSD_TEST_FAIL").as_deref() {
            Ok("blackscreen") => 1,
            Ok("blackscreen-once") => 2,
            _ => 0,
        })
    })
}
