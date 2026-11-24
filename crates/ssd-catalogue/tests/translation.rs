//! The translation table read from a file: `tests/fixtures/zh-hk-sample.tsv`, a small table of made-up keys and
//! texts in the layout of `zh-hk.tsv` (none of it is the game's text).

use ssd_catalogue::translation::{Slot, Translation, page_key, render, string_key};
use std::path::Path;

fn sample() -> Translation {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zh-hk-sample.tsv");
    Translation::read(&path).expect("the fixture reads")
}

#[test]
fn the_sample_file_reads_its_rows_and_leaves_out_an_empty_text() {
    let table = sample();
    assert_eq!(table.len(), 6);
    assert_eq!(table.text(&string_key(0, 0)), None, "help:0 has no text: English stays");
    assert_eq!(table.text("nothing:1"), None);
    assert_eq!(table.text(&page_key(&string_key(3, 0), 1)), Some("第二頁。"));
}

#[test]
fn a_same_row_reads_the_row_it_names() {
    let table = sample();
    assert_eq!(table.text("str3:0x01"), table.text("str3:0x00"));
    let text = render(table.text("str3:0x01").unwrap(), |_| String::new());
    assert_eq!(text, "這是一段測試文字，\n用來檢查換行。");
}

#[test]
fn the_slots_are_filled_and_the_engines_codes_left_out() {
    let table = sample();
    let text = render(table.text(&string_key(1, 0)).unwrap(), |slot| match slot {
        Slot::Name(1) => "甲".to_owned(),
        Slot::Name(2) => "乙".to_owned(),
        Slot::Number(1) => "42".to_owned(),
        _ => "?".to_owned(),
    });
    assert_eq!(text, "甲向乙\n發出42點攻擊！");
    assert_eq!(render(table.text("str2:0x001").unwrap(), |_| String::new()), "「你好！」");
}

#[test]
fn a_missing_file_is_an_error_not_a_panic() {
    assert!(Translation::read(Path::new("no/such/zh-hk.tsv")).is_err());
}

/// The shipped zh-tw pack (its source is checked by the generator's lints, which
/// read the English ROM, so this only pins what they settled).
fn zh_tw() -> Translation {
    Translation::parse(include_str!("../../../translations/zh-tw/zh-tw.tsv"))
        .expect("the pack reads")
}

#[test]
fn the_zh_tw_pack_keeps_lakeys_two_row_windows_to_two_rows() {
    // object 27's sneeze and cheer are drawn in a window of two rows, as their English pages are
    let table = zh_tw();
    for key in ["str7:0x01", "str7:0x02"] {
        let rows =
            table.text(key).unwrap_or_else(|| panic!("{key} has no text")).split("\\n").count();
        assert!(rows <= 2, "{key} has {rows} rows in a window of two");
    }
}

#[test]
fn the_zh_tw_pack_writes_out_the_names_its_english_pages_have_no_slot_for() {
    let table = zh_tw();
    // pages whose English page has no name slot at all: the person is written in
    for key in [
        "battle:247",
        "str2:0x09E",
        "str2:0x165",
        "str2:0x166",
        "str2:0x16B",
        "str3:0x8A",
        "str3:0x8D",
        "str3:0x8E",
        "str3:0x8F",
    ] {
        let text = table.text(key).unwrap_or_else(|| panic!("{key} has no text"));
        assert!(!text.contains("{name:"), "{key} still has a name slot: {text}");
    }
    // pages whose English page has {name:1} only: the second slot is written out
    {
        let key = "battle:286";
        let text = table.text(key).unwrap_or_else(|| panic!("{key} has no text"));
        assert!(text.contains("{name:1}") && !text.contains("{name:2}"), "{key}: {text}");
    }
    // the gravity training's trainee (v51), though the English pages have no name slot: battle:162
    // (King Kai's, not shown in the spaceship) takes slot 1, the one the map's scene names (Goku on
    // map 7, Piccolo on map 1, even with Goku placed there), and the story brings only that one to
    // each map; battle:165 takes slot 2, the one who trains. v1.0 wrote Goku out and called Piccolo
    // Goku on King Kai's planet (in the story the spaceship trains Goku alone)
    for (key, slot) in [("battle:162", "{name:1}"), ("battle:165", "{name:2}")] {
        let text = table.text(key).unwrap_or_else(|| panic!("{key} has no text"));
        assert!(text.contains(slot) && !text.contains("悟空"), "{key}: {text}");
    }
    // pages whose slot changes with the scene (checked against the Japanese ROM): the
    // slot is kept, since the game fills it the same way in both versions
    for key in ["str2:0x06C", "str2:0x06D", "str2:0x06E"] {
        let text = table.text(key).unwrap_or_else(|| panic!("{key} has no text"));
        assert!(text.contains("{name:1}"), "{key}: {text}");
    }
    let text = table.text("str3:0x67").expect("str3:0x67 has no text");
    assert!(text.contains("{name:1}") && text.contains("{name:2}"), "str3:0x67: {text}");
    // the slot of str2:0x165 and str2:0x166 holds `我們` of the fifth name table, which the port does not read
    assert!(table.text("str2:0x165").unwrap().contains("請跟我們"));
    assert!(table.text("str2:0x166").unwrap().contains("跟我們"));
}
