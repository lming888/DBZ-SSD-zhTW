//! The report of a start that failed on Android (`android_report.rs` gathers the facts and shows it in a system
//! dialog): its text in the player's language, and the scrubbing that keeps out of it anything that names the player
//! or their files. Pure, so the host tests it; the desktop program does not use it.

use ssd_menus::ui_text::UiText;

/// Where the dialog asks the player to paste the report.
pub const ISSUES_URL: &str = "https://github.com/lming888/DBZ-SSD-zhTW/issues";

/// What the report says of the device and the build; an empty text is "not known".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// The APK's version name and code, `v1.0.1 (30)`.
    pub version: String,
    pub manufacturer: String,
    pub model: String,
    /// The Android release (`9`) and its API level (`28`).
    pub android: String,
    pub api: String,
    /// The chip platform (`msm8998`, or the model of the system on chip where the system names it).
    pub chip: String,
    /// The graphics adapter: name, backend, driver and driver version.
    pub gpu: String,
    /// The memory in megabytes: total and available.
    pub memory_mb: Option<(u64, u64)>,
}

/// What happened, for the dialog's title and first line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Headline {
    /// The game ended with an error.
    CouldNotStart,
    /// The program panicked.
    Panicked,
}

// The English words of the report: each is a row of the menus' tables (`ui-zh-tw.tsv`, `ui-zh-hk.tsv`).
const TITLE_START: &str = "The game could not start";
const TITLE_PANIC: &str = "The game stopped with an error";
const LINE_START: &str = "The game could not start. The details are below.";
const LINE_PANIC: &str = "The game stopped because of an error. The details are below.";
const VERSION: &str = "Version:";
const DEVICE: &str = "Device:";
const ANDROID: &str = "Android:";
const CHIP: &str = "Chip:";
const GPU: &str = "Graphics:";
const MEMORY: &str = "Memory:";
const MEMORY_FREE: &str = "Free memory:";
const NOT_KNOWN: &str = "Not known";
const WHAT: &str = "What went wrong:";
const PASTE: &str = "This text is on the clipboard. Please paste it into a report at:";

/// Every English text of the report, so a test can check the tables have them all.
pub const WORDS: &[&str] = &[
    TITLE_START,
    TITLE_PANIC,
    LINE_START,
    LINE_PANIC,
    VERSION,
    DEVICE,
    ANDROID,
    CHIP,
    GPU,
    MEMORY,
    MEMORY_FREE,
    NOT_KNOWN,
    WHAT,
    PASTE,
];

/// `english` in the language of `words` (the menus' table; `None` is English), English where the table has no row.
#[must_use]
pub fn tr(words: Option<&UiText>, english: &str) -> String {
    words.and_then(|table| table.get(english)).unwrap_or_else(|| english.to_owned())
}

/// The dialog's title for `headline`.
#[must_use]
pub fn title(words: Option<&UiText>, headline: Headline) -> String {
    tr(
        words,
        match headline {
            Headline::CouldNotStart => TITLE_START,
            Headline::Panicked => TITLE_PANIC,
        },
    )
}

/// The first line of the report for `headline`.
#[must_use]
pub fn headline(words: Option<&UiText>, headline: Headline) -> String {
    tr(
        words,
        match headline {
            Headline::CouldNotStart => LINE_START,
            Headline::Panicked => LINE_PANIC,
        },
    )
}

/// A label and its value on one line: a label that ends in a full-width colon takes no space after it.
#[must_use]
pub fn labelled(words: Option<&UiText>, label: &str, value: &str) -> String {
    let label = tr(words, label);
    if label.ends_with('：') { format!("{label}{value}") } else { format!("{label} {value}") }
}

/// The report: `lead` (the lines that say what happened), the facts, `details` (what went wrong, already scrubbed)
/// and where to paste it. Plain text, for the dialog and the clipboard.
#[must_use]
pub fn report(
    words: Option<&UiText>,
    lead: &[String],
    facts: &Facts,
    details: &[String],
) -> String {
    let known = |text: &str| {
        let text = text.trim();
        if text.is_empty() { tr(words, NOT_KNOWN) } else { text.to_owned() }
    };
    let device = format!("{} {}", facts.manufacturer.trim(), facts.model.trim());
    let android = match (facts.android.trim(), facts.api.trim()) {
        ("", "") => tr(words, NOT_KNOWN),
        (release, "") => release.to_owned(),
        (release, api) => format!("{} (API {api})", known(release)),
    };
    let mut lines: Vec<String> = lead.to_vec();
    lines.push(String::new());
    lines.push(labelled(words, VERSION, &known(&facts.version)));
    lines.push(labelled(words, DEVICE, &known(&device)));
    lines.push(labelled(words, ANDROID, &android));
    lines.push(labelled(words, CHIP, &known(&facts.chip)));
    lines.push(labelled(words, GPU, &known(&facts.gpu)));
    match facts.memory_mb {
        Some((total, free)) => {
            lines.push(labelled(words, MEMORY, &format!("{total} MB")));
            lines.push(labelled(words, MEMORY_FREE, &format!("{free} MB")));
        }
        None => lines.push(labelled(words, MEMORY, &tr(words, NOT_KNOWN))),
    }
    if !details.is_empty() {
        lines.push(tr(words, WHAT));
        lines.extend(details.iter().map(|line| line.trim_end().to_owned()));
    }
    lines.push(String::new());
    lines.push(tr(words, PASTE));
    lines.push(ISSUES_URL.to_owned());
    lines.join("\n")
}

/// `text` with every absolute path and file address taken out: a path into a crate (Rust's panic locations, cargo's
/// registry) keeps its part from the crate on; any other path becomes `<path>`, and with it everything after it up to a
/// `:`, a quote or the end of the line (a name may hold spaces, full-width ones included: better too much than too
/// little); a `content://` or `file://` address becomes `<file>`. So the report never carries the name of the player's
/// folders, of the builder's home or of the ROM.
#[must_use]
pub fn scrub(text: &str) -> String {
    text.split_inclusive('\n').map(scrub_line).collect()
}

/// Characters a path may follow (`"/data/...`, `(/home/...`, `path=/x/...`, `key:/x/...`, `a,/x/...`, `讀取失敗：/x/...`,
/// `「/x/...`, `a;/x`, `a|/x`, `>/x`), besides spaces and the start of the line. More of them scrub more: better too
/// much than too little.
const OPENERS: &[char] =
    &['"', '\'', '(', '[', '{', '<', '=', '`', ':', ',', '：', '「', ';', '|', '>'];

/// What ends a path that is not a crate's: a name may hold anything else, spaces included.
fn ends_path(c: char) -> bool {
    matches!(c, ':' | '"' | '\'' | '`' | '\n' | '\r')
}

fn scrub_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut at = 0;
    let mut before: Option<char> = None;
    while let Some(c) = line[at..].chars().next() {
        let opens = before.is_none_or(|b| b.is_whitespace() || OPENERS.contains(&b));
        // `scheme://host/...` (a web address) is not a path after its `:` (`content://` and `file://` are handled
        // whole by path_at, from their scheme on)
        let address = before == Some(':') && line[at..].starts_with("//");
        if opens
            && !address
            && let Some((end, replacement)) = path_at(line, at)
        {
            out.push_str(&replacement);
            before = line[..end].chars().next_back();
            at = end;
            continue;
        }
        out.push(c);
        before = Some(c);
        at += c.len_utf8();
    }
    out
}

/// The path or address that starts at byte `at` of `line`, if one does: where it ends and what replaces it.
fn path_at(line: &str, at: usize) -> Option<(usize, String)> {
    let rest = &line[at..];
    let word_end = |from: usize| {
        rest[from..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`'))
            .map_or(line.len(), |i| at + from + i)
    };
    if ["content://", "file://"].iter().any(|scheme| rest.starts_with(scheme)) {
        return Some((word_end(0), "<file>".to_owned()));
    }
    let mut chars = rest.chars();
    let first = chars.next()?;
    let windows = first.is_ascii_alphabetic() && rest.get(1..3) == Some(":\\");
    let unix = first == '/' && chars.next().is_some_and(|c| !c.is_whitespace());
    if !windows && !unix {
        return None;
    }
    // A crate's path has no spaces: the word.
    let word = &line[at..word_end(0)];
    if let Some(tail) = crate_tail(word) {
        return Some((word_end(0), tail));
    }
    let from = if windows { 3 } else { 1 };
    let end = rest[from..].find(ends_path).map_or(line.len(), |i| at + from + i);
    Some((end, "<path>".to_owned()))
}

/// The part kept of an absolute path into a crate: from the crate on; `None` for any other path.
fn crate_tail(path: &str) -> Option<String> {
    let unix = path.replace('\\', "/");
    // cargo's registry: `.../registry/src/<index>/<crate>-<version>/src/...`
    if let Some(at) = unix.find("/registry/src/") {
        let after = &unix[at + "/registry/src/".len()..];
        if let Some(slash) = after.find('/') {
            return Some(after[slash + 1..].to_owned());
        }
    }
    // the standard library: `/rustc/<hash>/library/...`
    if let Some(at) = unix.find("/library/").filter(|_| unix.contains("/rustc/")) {
        return Some(unix[at + 1..].to_owned());
    }
    // a workspace: `.../crates/<crate>/...`
    unix.find("/crates/").map(|at| unix[at + 1..].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn taiwan() -> UiText {
        UiText::parse(include_str!("../../../translations/zh-tw/ui-zh-tw.tsv"))
    }

    fn hong_kong() -> UiText {
        UiText::parse(include_str!("../../../translations/zh-hk/ui-zh-hk.tsv"))
    }

    fn facts() -> Facts {
        Facts {
            version: "v1.0.1 (30)".to_owned(),
            manufacturer: "Sony".to_owned(),
            model: "G8441".to_owned(),
            android: "9".to_owned(),
            api: "28".to_owned(),
            chip: "msm8998".to_owned(),
            gpu: "Adreno (TM) 540 / Vulkan / Qualcomm 512.415".to_owned(),
            memory_mb: Some((3712, 1024)),
        }
    }

    #[test]
    fn the_report_has_every_fact_and_where_to_paste_it() {
        let lead = [headline(None, Headline::Panicked)];
        let text = report(
            None,
            &lead,
            &facts(),
            &["panicked at crates/ssd-app/src/app.rs:10:5:".to_owned()],
        );
        for part in [
            "The game stopped because of an error.",
            "Version: v1.0.1 (30)",
            "Device: Sony G8441",
            "Android: 9 (API 28)",
            "Chip: msm8998",
            "Graphics: Adreno (TM) 540 / Vulkan / Qualcomm 512.415",
            "Memory: 3712 MB",
            "Free memory: 1024 MB",
            "What went wrong:\npanicked at crates/ssd-app/src/app.rs:10:5:",
            ISSUES_URL,
        ] {
            assert!(text.contains(part), "{part:?} in\n{text}");
        }
        assert!(text.find("Version:") < text.find(ISSUES_URL), "the address comes last");
    }

    #[test]
    fn a_fact_not_known_says_so_and_no_details_leaves_out_their_heading() {
        let text = report(None, &[headline(None, Headline::CouldNotStart)], &Facts::default(), &[]);
        assert!(text.contains("Version: Not known"));
        assert!(text.contains("Device: Not known"));
        assert!(text.contains("Android: Not known"));
        assert!(text.contains("Graphics: Not known"));
        assert!(text.contains("Memory: Not known"));
        assert!(!text.contains("What went wrong"));
    }

    #[test]
    fn in_chinese_the_report_is_in_the_tables_words_with_full_width_colons() {
        let table = taiwan();
        let lead = [headline(Some(&table), Headline::CouldNotStart)];
        let text = report(Some(&table), &lead, &facts(), &["error: x".to_owned()]);
        for part in [
            "版本：v1.0.1 (30)",
            "裝置：Sony G8441",
            "Android：9 (API 28)",
            "記憶體：3712 MB",
            "剪貼簿",
        ] {
            assert!(text.contains(part), "{part:?} in\n{text}");
        }
        assert_eq!(title(Some(&table), Headline::Panicked), "遊戲發生錯誤");
        let hk = hong_kong();
        // The Hong Kong table is Taiwan's with the names changed (the translations repo's build_hk): the same words.
        assert_eq!(
            report(Some(&hk), &lead, &facts(), &[]),
            report(Some(&table), &lead, &facts(), &[]),
        );
    }

    #[test]
    fn both_tables_have_every_word_and_no_english_is_left_but_the_names() {
        // Kept as written: the names of the graphics interfaces and units, and the address.
        let kept = ["Android", "API", "MB", "GPU", "Vulkan", "OpenGL", "ES"];
        for (name, table) in [("ui-zh-tw.tsv", taiwan()), ("ui-zh-hk.tsv", hong_kong())] {
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
                assert!(!translated.contains("菜單"), "{name}: the glossary's 選單");
            }
        }
    }

    #[test]
    fn scrub_keeps_a_crates_path_from_the_crate_on_and_drops_any_other() {
        let cases = [
            (
                "panicked at /Users/someone/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wgpu-hal-30.0.1/src/vulkan/descriptor.rs:208:55:",
                "panicked at wgpu-hal-30.0.1/src/vulkan/descriptor.rs:208:55:",
            ),
            (
                "at /cargo/registry/src/index.crates.io-0000/sdl3-0.20.0/src/lib.rs:1:2",
                "at sdl3-0.20.0/src/lib.rs:1:2",
            ),
            (
                "crates/ssd-android/../ssd-app/src/app.rs:2273:28",
                "crates/ssd-android/../ssd-app/src/app.rs:2273:28",
            ),
            (
                "/home/alice/work/dbz/crates/ssd-app/src/video.rs:9:1",
                "crates/ssd-app/src/video.rs:9:1",
            ),
            ("/rustc/0123abcd/library/core/src/option.rs:5:9", "library/core/src/option.rs:5:9"),
            (
                "error: /data/user/0/io.github.dbz_ssd_recompiled/files/cache/rom/canonical.sfc: No such file",
                "error: <path>: No such file",
            ),
            (
                "cannot open content://com.android.externalstorage.documents/document/primary%3ADownload%2Fmy%20rom.sfc now",
                "cannot open <file> now",
            ),
            ("reading \"/storage/emulated/0/Download/dbz.sfc\"", "reading \"<path>\""),
            (r"C:\Users\bob\dbz\crates\ssd-app\src\main.rs:3:4", "crates/ssd-app/src/main.rs:3:4"),
            (r"D:\Games\dbz.sfc,", "<path>"),
            ("/storage/emulated/0/Download/Dragon Ball Z.sfc", "<path>"),
            ("open /storage/emulated/0/John Smith/dbz.sfc: denied", "open <path>: denied"),
            ("/sdcard/Download/七龍珠\u{3000}Z 超賽亞傳說.sfc", "<path>"),
            ("rom=/sdcard/Download/中文 檔名.sfc", "rom=<path>"),
            ("key:/storage/emulated/0/x.sfc", "key:<path>"),
            ("a,/data/x y", "a,<path>"),
            ("/sdcard", "<path>"),
            ("中文/路徑 and I/O stay", "中文/路徑 and I/O stay"),
            ("a/b and 1/2 and http stays", "a/b and 1/2 and http stays"),
            ("creating the wgpu surface: Failed", "creating the wgpu surface: Failed"),
            (
                "see https://github.com/lming888/DBZ-SSD-zhTW/issues and then /sdcard/x.sfc",
                "see https://github.com/lming888/DBZ-SSD-zhTW/issues and then <path>",
            ),
            ("http://example.com/a/b: 404", "http://example.com/a/b: 404"),
            ("讀取失敗：/sdcard/Download/x.sfc", "讀取失敗：<path>"),
            ("無法開啟「/sdcard/Download/七龍珠.sfc」", "無法開啟「<path>"),
            ("a;/data/x", "a;<path>"),
            ("a|/data/x", "a|<path>"),
            ("2>/data/x", "2><path>"),
        ];
        for (text, scrubbed) in cases {
            assert_eq!(scrub(text), scrubbed, "{text}");
        }
    }

    #[test]
    fn scrub_keeps_lines_and_spacing() {
        assert_eq!(
            scrub("a\n  /x/y/z.rs:1\tb /x/y:\n/x y\nz"),
            "a\n  <path>:1\tb <path>:\n<path>\nz"
        );
    }
}
