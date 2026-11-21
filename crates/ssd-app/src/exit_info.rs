//! How the app's last process ended (Android 11+), from the file the activity writes at start (`GameActivity`,
//! `process-exits.txt`: the system's `ApplicationExitInfo`): which ends are crashes (and so count against the
//! graphics backend at the next start) and which are the user or the system ending the app (pressing Home and having
//! it reclaimed, swiping it away, Force stop), which do not; and, for the report, what a native crash's tombstone or an
//! ANR's stacks say, in a few lines with nothing that names the player. Pure, so the host tests it.

use crate::launch_report::scrub;
use std::fmt::Write as _;

/// One process's end, as the activity wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitRecord {
    pub pid: u32,
    /// `ApplicationExitInfo.REASON_*`.
    pub reason: i32,
    /// The exit status, or the signal for `REASON_SIGNALED` and `REASON_CRASH_NATIVE`.
    pub status: i32,
    /// When the process ended, in milliseconds since 1970.
    pub time_ms: u64,
    /// The file (in the data folder) of its trace: a native crash's tombstone, an ANR's stacks; empty when none.
    pub trace: String,
    /// The system's description.
    pub description: String,
}

/// The lines of `process-exits.txt` (pid, reason, status, time, trace, description, tab-separated); a line not
/// understood is skipped.
#[must_use]
pub fn parse_exits(text: &str) -> Vec<ExitRecord> {
    text.lines()
        .filter_map(|line| {
            let mut cols = line.splitn(6, '\t');
            Some(ExitRecord {
                pid: cols.next()?.trim().parse().ok()?,
                reason: cols.next()?.trim().parse().ok()?,
                status: cols.next()?.trim().parse().ok()?,
                time_ms: cols.next()?.trim().parse().ok()?,
                trace: cols.next()?.trim().to_owned(),
                description: cols.next().unwrap_or_default().trim().to_owned(),
            })
        })
        .collect()
}

/// The end of the process `pid` that started at `started_ms` (when known): a record of that pid that is not older
/// than the start (a pid can be used again).
#[must_use]
pub fn find(records: &[ExitRecord], pid: u32, started_ms: Option<u64>) -> Option<&ExitRecord> {
    records
        .iter()
        .find(|r| r.pid == pid && started_ms.is_none_or(|started| r.time_ms + 1000 >= started))
}

/// What an end says of the start it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitKind {
    /// The app crashed (Java or native), did not respond, was killed by a fault signal, or could not start.
    Crash,
    /// The user or the system ended it: Home and reclaimed, swiped away, Force stop, low memory, an update...
    Normal,
    /// The system does not say (`REASON_UNKNOWN`, or a reason newer than this table).
    Unknown,
}

const UNKNOWN: i32 = 0;
const EXIT_SELF: i32 = 1;
const SIGNALED: i32 = 2;
const LOW_MEMORY: i32 = 3;
const CRASH: i32 = 4;
/// `REASON_CRASH_NATIVE`: its trace is a tombstone.
pub const CRASH_NATIVE: i32 = 5;
const ANR: i32 = 6;
const INITIALIZATION_FAILURE: i32 = 7;
const PERMISSION_CHANGE: i32 = 8;
const EXCESSIVE_RESOURCE_USAGE: i32 = 9;
const USER_REQUESTED: i32 = 10;
const USER_STOPPED: i32 = 11;
const DEPENDENCY_DIED: i32 = 12;
const OTHER: i32 = 13;
const FREEZER: i32 = 14;
const PACKAGE_STATE_CHANGE: i32 = 15;
const PACKAGE_UPDATED: i32 = 16;

/// SIGKILL and SIGTERM: what the system and the user end a process with; any other signal is a fault.
const ENDING_SIGNALS: [i32; 2] = [9, 15];

/// The kind of the end `reason` with `status` (see [`ExitKind`]).
#[must_use]
pub fn classify(reason: i32, status: i32) -> ExitKind {
    match reason {
        CRASH | CRASH_NATIVE | ANR | INITIALIZATION_FAILURE => ExitKind::Crash,
        SIGNALED if !ENDING_SIGNALS.contains(&status) => ExitKind::Crash,
        SIGNALED
        | EXIT_SELF
        | LOW_MEMORY
        | PERMISSION_CHANGE
        | EXCESSIVE_RESOURCE_USAGE
        | USER_REQUESTED
        | USER_STOPPED
        | DEPENDENCY_DIED
        | OTHER
        | FREEZER
        | PACKAGE_STATE_CHANGE
        | PACKAGE_UPDATED => ExitKind::Normal,
        _ => ExitKind::Unknown,
    }
}

/// The name of `reason` as the API has it (`REASON_CRASH_NATIVE`).
#[must_use]
pub fn reason_name(reason: i32) -> String {
    let name = match reason {
        UNKNOWN => "UNKNOWN",
        EXIT_SELF => "EXIT_SELF",
        SIGNALED => "SIGNALED",
        LOW_MEMORY => "LOW_MEMORY",
        CRASH => "CRASH",
        CRASH_NATIVE => "CRASH_NATIVE",
        ANR => "ANR",
        INITIALIZATION_FAILURE => "INITIALIZATION_FAILURE",
        PERMISSION_CHANGE => "PERMISSION_CHANGE",
        EXCESSIVE_RESOURCE_USAGE => "EXCESSIVE_RESOURCE_USAGE",
        USER_REQUESTED => "USER_REQUESTED",
        USER_STOPPED => "USER_STOPPED",
        DEPENDENCY_DIED => "DEPENDENCY_DIED",
        OTHER => "OTHER",
        FREEZER => "FREEZER",
        PACKAGE_STATE_CHANGE => "PACKAGE_STATE_CHANGE",
        PACKAGE_UPDATED => "PACKAGE_UPDATED",
        _ => return format!("REASON_{reason}"),
    };
    format!("REASON_{name}")
}

/// The end in the player's words (a row of the menus' tables), for a crash or an unknown end.
#[must_use]
pub fn reason_words(reason: i32, status: i32) -> &'static str {
    match (reason, classify(reason, status)) {
        (CRASH_NATIVE, _) => "a crash in native code",
        (CRASH, _) => "a crash",
        (ANR, _) => "no response (ANR)",
        (SIGNALED, ExitKind::Crash) => "killed by a signal",
        (INITIALIZATION_FAILURE, _) => "failed to start",
        (_, ExitKind::Normal) => "ended by the user or the system",
        _ => "Not known",
    }
}

/// Every English text of this module the report shows, so a test can check the tables have them all.
pub const WORDS: &[&str] = &[
    "a crash in native code",
    "a crash",
    "no response (ANR)",
    "killed by a signal",
    "failed to start",
    "ended by the user or the system",
];

// ---- the tombstone of a native crash (protobuf: system/core/debuggerd/proto/tombstone.proto) ----

/// A field of a protobuf message: its number and its value (a varint, or the bytes of a length-delimited field).
enum Field<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
}

fn varint(bytes: &[u8], at: &mut usize) -> Option<u64> {
    let mut value = 0_u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(*at)?;
        *at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

/// A field read by [`next_field`]: one kept, or one of a kind not read (fixed 32 or 64 bits).
enum Next<'a> {
    Kept(u64, Field<'a>),
    Skipped,
}

/// The field of `bytes` at `at`, moving past it; `None` when it cannot be read whole.
fn next_field<'a>(bytes: &'a [u8], at: &mut usize) -> Option<Next<'a>> {
    let key = varint(bytes, at)?;
    let (number, wire) = (key >> 3, key & 7);
    let field = match wire {
        0 => Field::Varint(varint(bytes, at)?),
        1 | 5 => {
            let size = if wire == 1 { 8 } else { 4 };
            *at = at.checked_add(size).filter(|end| *end <= bytes.len())?;
            return Some(Next::Skipped);
        }
        2 => {
            let len = usize::try_from(varint(bytes, at)?).ok()?;
            let end = at.checked_add(len).filter(|end| *end <= bytes.len())?;
            let value = &bytes[*at..end];
            *at = end;
            Field::Bytes(value)
        }
        _ => return None,
    };
    Some(Next::Kept(number, field))
}

/// The fields of the message `bytes`, in order; `None` when it is not a well-formed message.
fn fields(bytes: &[u8]) -> Option<Vec<(u64, Field<'_>)>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if let Next::Kept(number, field) = next_field(bytes, &mut at)? {
            out.push((number, field));
        }
    }
    Some(out)
}

/// The fields of the message `bytes` up to the first one that cannot be read whole: a trace cut short (the activity
/// keeps at most 512 KiB of it) still gives what comes before the cut.
fn fields_lenient(bytes: &[u8]) -> Vec<(u64, Field<'_>)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        match next_field(bytes, &mut at) {
            Some(Next::Kept(number, field)) => out.push((number, field)),
            Some(Next::Skipped) => {}
            None => break,
        }
    }
    out
}

fn text_of(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The first string field `number` of `fields`.
fn string(fields: &[(u64, Field<'_>)], number: u64) -> Option<String> {
    fields.iter().find_map(|(n, f)| match f {
        Field::Bytes(b) if *n == number => Some(text_of(b)),
        _ => None,
    })
}

fn number(fields: &[(u64, Field<'_>)], number: u64) -> Option<u64> {
    fields.iter().find_map(|(n, f)| match f {
        Field::Varint(v) if *n == number => Some(*v),
        _ => None,
    })
}

/// The last part of a file name (`/data/app/.../base.apk!lib/arm64-v8a/libmain.so` -> `libmain.so`): no folder.
fn base_name(name: &str) -> &str {
    name.rsplit(['/', '!', '\\']).next().unwrap_or(name)
}

/// The frames shown of the crashing thread.
const FRAMES: usize = 8;
/// The longest line of a summary.
const LINE: usize = 160;

fn short(line: String) -> String {
    match line.char_indices().nth(LINE) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line,
    }
}

/// What a native crash's tombstone says, in a few lines: the signal and the fault address, the thread that crashed, the
/// abort message and the cause, the page size, and the top frames of its stack (each the library's file name, without
/// its folder, and the function). `None` when `bytes` is not a tombstone.
#[must_use]
pub fn tombstone_summary(bytes: &[u8]) -> Option<Vec<String>> {
    // The top level is read up to a cut; each field kept is whole.
    let top = fields_lenient(bytes);
    let tid = number(&top, 6);
    let mut lines = Vec::new();
    if let Some(Field::Bytes(signal)) = top.iter().find(|(n, _)| *n == 10).map(|(_, f)| f)
        && let Some(signal) = fields(signal)
    {
        let mut line = format!(
            "signal {} ({}), code {} ({})",
            number(&signal, 1).unwrap_or(0),
            string(&signal, 2).unwrap_or_default(),
            number(&signal, 3).unwrap_or(0) as i32,
            string(&signal, 4).unwrap_or_default(),
        );
        if number(&signal, 8) == Some(1) {
            let _ = write!(line, ", fault addr 0x{:x}", number(&signal, 9).unwrap_or(0));
        }
        lines.push(line);
    }
    if let Some(message) = string(&top, 14).filter(|m| !m.is_empty()) {
        lines.push(short(format!("abort: {}", scrub(&message.replace('\n', " ")))));
    }
    for (_, cause) in top.iter().filter(|(n, _)| *n == 15) {
        if let Field::Bytes(cause) = cause
            && let Some(text) = fields(cause).and_then(|c| string(&c, 1))
        {
            lines.push(short(format!("cause: {}", scrub(&text))));
        }
    }
    // The threads: a map of tid to Thread (an entry: 1 the key, 2 the thread).
    let threads = top.iter().filter_map(|(n, f)| match f {
        Field::Bytes(entry) if *n == 16 => fields(entry),
        _ => None,
    });
    for entry in threads {
        let Some(Field::Bytes(thread)) = entry.iter().find(|(n, _)| *n == 2).map(|(_, f)| f) else {
            continue;
        };
        let Some(thread) = fields(thread) else { continue };
        let id = number(&thread, 1);
        if tid.is_some() && id != tid {
            continue;
        }
        lines.push(short(format!(
            "thread: {} (tid {})",
            scrub(&string(&thread, 2).unwrap_or_default()),
            id.unwrap_or(0)
        )));
        let frames = thread.iter().filter_map(|(n, f)| match f {
            Field::Bytes(frame) if *n == 4 => fields(frame),
            _ => None,
        });
        // A frame of the game's stripped library has no name: its build id (when the system gives one; a library read
        // from inside the APK has none) lets its pc be looked up in the build's own library. Other libraries' are left
        // out.
        let mut build = None;
        for (index, frame) in frames.take(FRAMES).enumerate() {
            let file = string(&frame, 6).unwrap_or_default();
            let function = string(&frame, 4).unwrap_or_default();
            let offset = number(&frame, 5).unwrap_or(0);
            let at = if function.is_empty() {
                if build.is_none() && matches!(base_name(&file), "libmain.so" | "base.apk") {
                    build = string(&frame, 8)
                        .filter(|id| !id.is_empty())
                        .map(|id| format!("build id of {}: {id}", base_name(&file)));
                }
                format!("pc 0x{:x}", number(&frame, 1).unwrap_or(0))
            } else {
                format!("{function}+{offset}")
            };
            lines.push(short(format!("#{index:02} {} {at}", base_name(&file))));
        }
        lines.extend(build.map(short));
        break;
    }
    if let Some(page) = number(&top, 22).filter(|p| *p > 0) {
        lines.push(format!("page size: {page}"));
    }
    (!lines.is_empty()).then_some(lines)
}

/// `line` with each absolute path in it cut to its file name (`/apex/.../libc.so` -> `libc.so`), as a tombstone's
/// frames are: the library is kept, its folder not.
fn file_names(line: &str) -> String {
    line.split(' ')
        .map(|word| if word.starts_with('/') { base_name(word) } else { word })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What an ANR's stacks say of the activity's thread (the process's first thread, `tid=1`: SDL names it
/// `SDLActivity`, others leave `main`): its state and the top of its stack, a few lines, scrubbed. Empty when the text
/// has no such thread.
#[must_use]
pub fn anr_summary(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(subject) = text.lines().find(|line| line.starts_with("Subject:")) {
        lines.push(short(scrub(subject.trim())));
    }
    let mut main =
        text.lines().skip_while(|line| !(line.starts_with('"') && line.contains(" tid=1 ")));
    if let Some(head) = main.next() {
        lines.push(short(scrub(head.trim())));
        let stack = main
            .take_while(|line| !line.trim().is_empty())
            .filter(|line| {
                let line = line.trim_start();
                line.starts_with("at ") || line.starts_with("native:") || line.starts_with("- ")
            })
            .take(FRAMES);
        lines.extend(stack.map(|line| short(scrub(&file_names(line.trim())))));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_tables_have_every_word_of_the_exit() {
        for text in [
            include_str!("../../../translations/zh-tw/ui-zh-tw.tsv"),
            include_str!("../../../translations/zh-hk/ui-zh-hk.tsv"),
        ] {
            let table = ssd_menus::ui_text::UiText::parse(text);
            for english in WORDS {
                let translated = table.get(english).unwrap_or_else(|| panic!("no row {english:?}"));
                assert!(
                    !translated.replace("ANR", "").chars().any(|c| c.is_ascii_alphabetic()),
                    "{translated:?}"
                );
            }
        }
    }

    #[test]
    fn the_exits_file_is_read_line_by_line_and_a_bad_line_is_skipped() {
        let text = "1234\t5\t11\t1700000005000\tprocess-exit-1234.trace\tcrash in libfoo\n\
                    garbage\n\
                    99\t10\t0\t1700000000000\t\tremove task\n";
        let records = parse_exits(text);
        assert_eq!(records.len(), 2);
        assert_eq!(
            records[0],
            ExitRecord {
                pid: 1234,
                reason: 5,
                status: 11,
                time_ms: 1_700_000_005_000,
                trace: "process-exit-1234.trace".to_owned(),
                description: "crash in libfoo".to_owned(),
            }
        );
        assert_eq!(records[1].description, "remove task");
        assert_eq!(find(&records, 1234, Some(1_700_000_000_000)).map(|r| r.reason), Some(5));
        assert_eq!(
            find(&records, 1234, Some(1_700_000_009_000)),
            None,
            "an end older than the start"
        );
        assert_eq!(find(&records, 77, None), None);
    }

    #[test]
    fn crashes_count_and_the_user_or_system_ending_the_app_does_not() {
        for (reason, status) in [
            (CRASH, 0),
            (CRASH_NATIVE, 11),
            (CRASH_NATIVE, 6),
            (ANR, 0),
            (INITIALIZATION_FAILURE, 0),
        ] {
            assert_eq!(classify(reason, status), ExitKind::Crash, "{}", reason_name(reason));
        }
        for signal in [4, 5, 6, 7, 8, 11, 31] {
            assert_eq!(classify(SIGNALED, signal), ExitKind::Crash, "signal {signal}");
        }
        for signal in ENDING_SIGNALS {
            assert_eq!(classify(SIGNALED, signal), ExitKind::Normal, "signal {signal}");
        }
        for reason in [
            EXIT_SELF,
            LOW_MEMORY,
            PERMISSION_CHANGE,
            EXCESSIVE_RESOURCE_USAGE,
            USER_REQUESTED,
            USER_STOPPED,
            DEPENDENCY_DIED,
            OTHER,
            FREEZER,
            PACKAGE_STATE_CHANGE,
            PACKAGE_UPDATED,
        ] {
            assert_eq!(classify(reason, 0), ExitKind::Normal, "{}", reason_name(reason));
        }
        assert_eq!(classify(UNKNOWN, 0), ExitKind::Unknown);
        assert_eq!(classify(99, 0), ExitKind::Unknown);
        assert_eq!(reason_name(5), "REASON_CRASH_NATIVE");
        assert_eq!(reason_name(99), "REASON_99");
        assert_eq!(reason_words(SIGNALED, 11), "killed by a signal");
        assert_eq!(reason_words(UNKNOWN, 0), "Not known");
    }

    /// A protobuf writer for the tests: fields of a message.
    #[derive(Default)]
    struct Message(Vec<u8>);

    impl Message {
        fn varint(mut self, number: u64, mut value: u64) -> Self {
            push_varint(&mut self.0, number << 3);
            loop {
                let byte = (value & 0x7f) as u8;
                value >>= 7;
                self.0.push(if value == 0 { byte } else { byte | 0x80 });
                if value == 0 {
                    break;
                }
            }
            self
        }

        fn bytes(mut self, number: u64, value: &[u8]) -> Self {
            push_varint(&mut self.0, (number << 3) | 2);
            push_varint(&mut self.0, value.len() as u64);
            self.0.extend_from_slice(value);
            self
        }

        fn text(self, number: u64, value: &str) -> Self {
            self.bytes(number, value.as_bytes())
        }

        fn message(self, number: u64, value: Message) -> Self {
            self.bytes(number, &value.0)
        }
    }

    fn push_varint(out: &mut Vec<u8>, mut value: u64) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            out.push(if value == 0 { byte } else { byte | 0x80 });
            if value == 0 {
                break;
            }
        }
    }

    fn frame(file: &str, function: &str, offset: u64) -> Message {
        Message::default().varint(1, 0x1234).text(4, function).varint(5, offset).text(6, file)
    }

    fn thread(id: u64, name: &str, frames: Vec<Message>) -> Message {
        let mut thread = Message::default().varint(1, id).text(2, name);
        for frame in frames {
            thread = thread.message(4, frame);
        }
        thread
    }

    #[test]
    fn a_tombstone_says_the_signal_the_thread_and_the_top_of_its_stack_without_folders() {
        let apk = "/data/app/~~abc==/io.github.dbz_ssd_recompiled-xyz==/base.apk!libmain.so";
        let tombstone = Message::default()
            .varint(5, 4321)
            .varint(6, 4400)
            .message(
                10,
                Message::default()
                    .varint(1, 11)
                    .text(2, "SIGSEGV")
                    .varint(3, 1)
                    .text(4, "SEGV_MAPERR")
                    .varint(8, 1)
                    .varint(9, 0x10),
            )
            .text(14, "boom at /storage/emulated/0/Download/my rom.sfc")
            .message(15, Message::default().text(1, "null pointer dereference"))
            .message(
                16,
                Message::default().varint(1, 4321).message(2, thread(4321, "main", vec![])),
            )
            .message(
                16,
                Message::default().varint(1, 4400).message(
                    2,
                    thread(
                        4400,
                        "SDLThread",
                        vec![
                            frame("/vendor/lib64/egl/libGLESv2_adreno.so", "glDrawArrays", 40),
                            frame(apk, "_ZN4main5video7present17h0123E", 12),
                            frame(apk, "", 0).text(8, "0a1b2c3d4e5f"),
                        ],
                    ),
                ),
            )
            .varint(22, 4096);
        let lines = tombstone_summary(&tombstone.0).expect("a tombstone");
        assert_eq!(
            lines,
            [
                "signal 11 (SIGSEGV), code 1 (SEGV_MAPERR), fault addr 0x10",
                "abort: boom at <path>",
                "cause: null pointer dereference",
                "thread: SDLThread (tid 4400)",
                "#00 libGLESv2_adreno.so glDrawArrays+40",
                "#01 libmain.so _ZN4main5video7present17h0123E+12",
                "#02 libmain.so pc 0x1234",
                "build id of libmain.so: 0a1b2c3d4e5f",
                "page size: 4096",
            ]
        );
        assert!(!lines.join("\n").contains("/data/"), "no folder of the app");
        // Cut short (the activity keeps 512 KiB of a trace): what comes before the cut is still read.
        let whole = tombstone.0.clone();
        let signal_end =
            whole.windows(10).position(|w| w == b"SEGV_MAPER").expect("the signal") + 40;
        for cut in [signal_end, whole.len() / 2, whole.len() - 3, whole.len() - 1] {
            let lines =
                tombstone_summary(&whole[..cut]).unwrap_or_else(|| panic!("cut at {cut}: nothing"));
            assert_eq!(
                lines[0], "signal 11 (SIGSEGV), code 1 (SEGV_MAPERR), fault addr 0x10",
                "cut at {cut}"
            );
        }
        assert_eq!(tombstone_summary(b"\xff\xff\xff"), None, "not a message");
        assert_eq!(tombstone_summary(b""), None, "empty");
    }

    #[test]
    fn an_anr_says_what_the_activitys_thread_was_doing() {
        let trace = "----- pid 4321 at 2026-11-20 -----\n\
                     Subject: Input dispatching timed out (Waiting to send key event)\n\
                     \n\
                     \"Signal Catcher\" daemon prio=10 tid=2 Runnable\n  at x.y(Z.java:1)\n\n\
                     \"main\" prio=5 tid=12 Native\n  at not.this(One.java:3)\n\n\
                     \"SDLActivity\" prio=5 tid=1 Sleeping\n\
                     \x20 | group=\"main\" sCount=1\n\
                     \x20 at java.lang.Thread.sleep(Native method)\n\
                     \x20 at io.github.dbz_ssd_recompiled.GameActivity.onUnhandledMessage(GameActivity.java:150)\n\
                     \x20 native: #00 pc 0001 /apex/com.android.runtime/lib64/bionic/libc.so (nanosleep+8)\n\
                     \n\"other\" prio=5\n  at q.r(S.java:2)\n";
        let lines = anr_summary(trace);
        assert_eq!(lines[0], "Subject: Input dispatching timed out (Waiting to send key event)");
        assert_eq!(lines[1], "\"SDLActivity\" prio=5 tid=1 Sleeping");
        assert!(lines.iter().any(|l| l.contains("GameActivity.onUnhandledMessage")));
        assert!(
            lines.iter().any(|l| l == "native: #00 pc 0001 libc.so (nanosleep+8)"),
            "the library's file name is kept, not its folder: {lines:?}"
        );
        assert!(
            !lines.iter().any(|l| l.contains("q.r") || l.contains("not.this")),
            "only the first thread"
        );
        assert_eq!(anr_summary("no threads here"), Vec::<String>::new());
    }
}
