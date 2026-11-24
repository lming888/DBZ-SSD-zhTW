# Dragon Ball Z: Super Saiya Densetsu　Traditional Chinese Remaster (Taiwan, Hong Kong)

繁體中文（首頁）：[README.md](README.md)

Author: Ming Liu (GitHub: [lming888](https://github.com/lming888))

**Download**: [the v1.0.1 installer and the Android APK](https://github.com/lming888/DBZ-SSD-zhTW/releases/tag/v1.0.1)

**Walkthrough** (in Traditional Chinese, PDF): [DBZ_SSD_guide_zhTW_v1.0.1.pdf](https://github.com/lming888/DBZ-SSD-zhTW/releases/download/v1.0.1/DBZ_SSD_guide_zhTW_v1.0.1.pdf), with the characters', cards' and moves' names as the game shows them; its contents (numbers, routes, secrets) are not checked item by item.

**Status**: v1.0.1 (the official release). Reports from playing it are welcome.

![The Chinese text in a story dialogue](docs/media/zh-tw-dialogue.png)
![The Chinese text in a battle message](docs/media/zh-tw-battle.png)

This is the Traditional Chinese edition of *Dragon Ball Z: Super Saiya Densetsu* (SNES, 1992), with two sets of names, Taiwan's and Hong Kong's, chosen at the first start (see "[Hong Kong names](#hong-kong-names)"). The game program is JackRushante's PC version rewritten in Rust ([DBZ-Super-Saiya-Densetsu-Rust](https://github.com/JackRushante/DBZ-Super-Saiya-Densetsu-Rust) v0.3.15), and I added a Traditional Chinese language pack and fixes on top of it (the main fixes are in the Release attachment `PATCH_NOTES.txt`). Chinese is shown in Enhanced mode only; Classic mode is the original English. Where the game code and the HD art pack come from, and their licences, are in the last section, "[License and credits](#license-and-credits)"; the two screenshots above are the running game in Enhanced mode with upstream's HD art pack.

## What you need

- Windows 10/11 (64-bit). The Android version needs an arm64 (64-bit ARM) phone or tablet with Android 8.0 or later; see "[Android version](#android-version)".
- Your own game ROM: the Japanese *Dragon Ball Z: Super Saiya Densetsu* ROM plus `dbz102.ips`, the Klepto Software English patch v1.02, or an English ROM already patched to v1.02. This project contains and provides no ROM, patch file (ips), save or game data.
- **Copyright notice: the game ROM and the IPS patch file are both protected by copyright. Please do not bundle the Chinese translation files with a ROM for others to download; that is illegal.**
- (Optional) the HD art pack `hd-pack.zip`; please use the v0.3.9 pack (484,121,119 bytes). Older packs lack the pictures used by Super Saiyan Goku's raised hair and the portrait fixes; the launcher asks you to update, but starts as usual. The art pack is not this project's work; where to download it, its authors and its licence (CC BY-NC-SA 4.0) are in the last section, "[License and credits](#license-and-credits)". Neither this repository nor the Release includes the art pack. When upstream cannot deliver it, the launcher downloads it instead from the copy the author keeps unchanged under CC BY-NC-SA 4.0 ([hd-pack-v0.3.9 of lming888/DBZ-Super-Saiya-Densetsu-Rust](https://github.com/lming888/DBZ-Super-Saiya-Densetsu-Rust/releases/tag/hd-pack-v0.3.9)); the file is identical to upstream's.

Checksums of the ROMs and the patch:

| File | Size (bytes) | CRC-32 | SHA-1 | SHA-256 |
|---|---|---|---|---|
| Japanese Rev 1 ROM (no header) | 1,048,576 | `ADE7B968` | `6b5725765ddde078350ec61221ddb9bb252066a7` | `962aa7a09765a97164af67098877a8fe5b7f1ea9db738561eb466b7600fd241c` |
| Klepto Software English patch v1.02 `dbz102.ips` | 62,587 | `F6D6719F` | `171d8925e857985427c5995041bf4c6fdc47124e` | `a1607090a6534d6b1711413ae07252c492d6c4c809868a2b6265b32737bfeed8` |
| English ROM (patched, header checksum not fixed) | 1,048,576 | `3BB02A05` | `252c1b3eaa1084dcde0aa8f7bad9caed726b53be` | `31cec05b84a1717be4fb914d5d9748bfa827f66d4b692751705d48c4a2646e57` |
| English ROM (header checksum fixed, the version the game uses) | 1,048,576 | `A8CF4F2A` | `6b612f6d99f2efc26f26fb22290bc732ada0ea6f` | `0cc43e7550e87e9d0f9a4e8736beb4f48af46c03bf5c5cc39d847aa3cbd17a4f` |

Any of the three ROMs works (the Japanese one together with `dbz102.ips`). On Windows, check with `certutil -hashfile <file> SHA1`. `dbz102.ips` was made for a ROM with a 512-byte header; the game puts it at the right place itself, so there is no need to add a header. A ROM that has the header (1,049,088 bytes) works too: the game removes the header. The checksums of such a file as a whole differ from the table.

## Installation

### Method 1: the release (recommended)

The v1.0.1 Release provides the installer `DBZ-SSD-zh-TW-v1.0.1-setup.exe` (attached to the Release page):

- **Installer**:
  - Installs for the current user only, without administrator rights.
  - Installs by default into `%LOCALAPPDATA%\Programs\DBZ-SSD-zhTW\`; another folder you can write to can be chosen during setup. A folder that already holds other files gets a `DBZ-SSD-zhTW` folder inside it instead.
  - Lets you pick your own ROM and IPS on the way; skip it and the launcher asks the first time.
  - After the ROM, a page "HD 高畫質美術包" (the HD art pack): on the left the original and the HD picture of the same game frame, one above the other; on the right a note that the art pack is the upstream author's work, about 462 MB. Tick "download the HD art pack after setup" (off by default) to download it when setup finishes; when the install folder already has the pack, the page only tells you so and does not ask. The finish page has "start the game"; when both are ticked, the pack is downloaded first and the game starts after the download.
  - Makes a desktop shortcut "七龍珠Z 超賽亞傳說 繁中版" and a Start menu folder "七龍珠Z 超賽亞傳說 繁中版" with the game and "下載 HD 美術包" (download the HD art pack).
  - **v1.0 or RC1 to RC3 already installed**: just run the newer installer; it upgrades in place, with no need to uninstall first. Keep the install folder it offers (the last one, the default): the program is replaced, the saves, the ROM setup, the HD art pack, the game settings and the language are kept, and "Add or remove programs" and the shortcuts still list one copy. The installer's first page says it found the installed v1.0 and will upgrade it in place to v1.0.1. The launchers of RC1 to v1.0 do not look for a newer version themselves: download and run the new installer once. With RC1 and RC2 installed in two different folders, the new version upgrades the one installed last (the one "Add or remove programs" points at). Installing into another folder leaves the old folder's program and saves where they are (the saves are not moved over) and points "Add or remove programs" at the new folder; remove the old copy, when you no longer want it, by running `uninstall-dbz-zhtw.exe` in the old folder (the saves are kept by default).
  - Can be removed in "Add or remove programs". By default the saves and the HD art pack are kept; to delete them too, tick "同時刪除存檔、ROM 設定、HD 美術包與遊戲設定" (also delete the saves, the ROM setup, the HD art pack and the game settings).
  - Has no code-signing certificate: the first time it runs, Windows SmartScreen says "Windows protected your PC"; click "More info", then "Run anyway".

1. Run the installer and follow the pages. When the install folder is inside a cloud-synced folder, the saves are synced too and are sometimes locked.
2. Open the game with the desktop or Start menu shortcut. If you did not pick the ROM during setup, the first start asks for two files in turn: first the ROM (`.sfc`/`.smc`), then `dbz102.ips`; if the ROM you picked is already the English one, press "Cancel" at the second step. After that, a double-click goes straight into the game.
3. HD art: while you have no art pack, the first start asks whether to download it (about 462 MB; nothing connects before you choose "Download", and it can be cancelled at any time). You can also tick it during setup, or use "下載 HD 美術包" in the Start menu later. The details of the download and its checks are in the next section.
4. Saves are in the `save\` folder beside the exe. The error log is `%LOCALAPPDATA%\dbz-ssd-zhtw\last-run.log`.

### Downloading the HD art pack

- The launcher asks with three buttons: "Download", "Not now", "Don't ask again". It downloads upstream's v0.3.9 `hd-pack.zip` from upstream's GitHub release page, shows a progress window (downloaded / total size, percentage), and puts the file beside the exe only when its size and SHA-256 match; it is used from then on.
- When upstream cannot deliver the file (cannot connect, timeout, connection lost midway, redirect to a host that is not allowed, wrong size or SHA-256), the launcher automatically downloads the same file from the copy the author keeps unchanged, checked the same way. If you press cancel, another launcher is already downloading, or writing the file fails, it does not fall back to the copy.
- You can also put the v0.3.9 `hd-pack.zip` (484,121,119 bytes) beside the exe yourself; it is installed at the next start. When the pack is not the v0.3.9 one, the launcher shows a notice and records it in `last-run.log`, and the game opens as usual.

### Downloading the HD art pack later

If you chose "Not now" or "Don't ask again" before and want the art pack later, use the shortcut "下載 HD 美術包" in the Start menu folder "七龍珠Z 超賽亞傳說 繁中版". It only downloads the art pack and does not start the game; it asks once, and shows the same progress window, which can be cancelled. When the `hd-pack.zip` already beside the exe is not the v0.3.9 one, the old file is renamed and kept, not deleted (details in `launcher/README.md`).

The shortcut is the launcher with `--hd-pack`. You can also run `"%LOCALAPPDATA%\Programs\DBZ-SSD-zhTW\DBZ_超賽亞傳說_繁中.exe" --hd-pack` from a Command Prompt (with the actual folder if you installed elsewhere). `--help` lists every argument.

### Default keys (keyboard)

| Game button | Keyboard |
|---|---|
| D-pad | Arrow keys |
| A | X |
| B | Z |
| X | S |
| Y | A |
| L | Q |
| R | W |
| Start | Enter |
| Select | Right Shift |

- Other shortcuts: F5 save state, F9 load state, 1 to 4 pick the save-state slot, Tab toggles the speed, P pause, F12 screenshot, F1 panel, F3 battle cheats, Esc quit. The F1 debug panel works only with `--debug`.
- In the HD menus (options, save editor), L/R (Q/W) switch tabs.
- Press Select (Right Shift) on the title screen to open the save editor.
- Keys and gamepad mapping can be changed in Options > Controls (the number keys 1 to 4, F1, F3 and Esc are fixed).

The launcher the installer installs, `DBZ_超賽亞傳說_繁中.exe`, holds the game `ssd-recompiled.exe` built from this project's source, the language pack and the font, and extracts them to `%LOCALAPPDATA%\dbz-ssd-zhtw\` on the first run.

### Method 2: the original Rust remake plus the language pack

Without replacing the game, the language pack alone also shows Chinese:

1. Download `dbz-ssd-recompiled-windows.zip` of the original Rust remake v0.3.15 (where to download it is in the last section, "[License and credits](#license-and-credits)"), and set up the ROM once with `Set up my ROM.bat` as its `QUICKSTART.txt` describes.
2. On this repository's page press Code → Download ZIP, unpack it, and put `zh-tw.tsv`, `ui-zh-tw.tsv` and `fonts\LXGWWenKaiTC-Regular.ttf` from `translations\zh-tw\` in `%LOCALAPPDATA%\dbz-ssd-recompiled\` (or beside `ssd-recompiled.exe`), keeping the `fonts\` subfolder. For the Hong Kong names, also put `zh-hk.tsv` and `ui-zh-hk.tsv` from `translations\zh-hk\` in the same folder (they share the font above). The file names must be lowercase `zh-tw.tsv`, `ui-zh-tw.tsv`, `zh-hk.tsv` and `ui-zh-hk.tsv`.
3. Start the game with `Play Remastered.bat` and pick 「繁體中文（台灣）」 in Options > Accessibility > Language; with the two Hong Kong tables in place, 「繁體中文（香港）」 can be picked too.

The difference between the two methods: method 2 does not have the fixes I wrote. You will see leftover English (the English name flashing first at the start of battle messages, help boxes falling back to English, English letters left outside boxes and so on: the fixes of the text overlay and text boxes are missing), fewer bag items after loading or saving to another slot, and some portraits drawn wrong with the HD art pack (Vegeta, Ginyu, Nail, Frieza on the map).

### Android version

1. You need an arm64 (64-bit ARM) phone or tablet with Android 8.0 or later, and the same ROM as in "What you need" above.
2. Download `DBZ-SSD-zh-TW-v1.0.1-arm64.apk` from the Release and open it on the phone to install it. If an earlier version of this project or a test build is installed, just install over it: the saves and settings are kept. The APK is not in a store, so at the first install the system asks you to let that source "install unknown apps".
3. At the first start the language page comes first (Taiwan, Hong Kong); then pick the ROM with the system's file picker. With the Japanese ROM it then asks for `dbz102.ips`.
4. (Optional) the HD art pack: download `hd-pack.zip` to the phone first, then pick it with the file picker from 「安裝 HD 美術包」 (Install the HD pack) in the options (about 0.9 GB once installed).

Note: the app has the same package name as upstream's Android version but another signature, so if you installed upstream's Android version, uninstall it first; uninstalling the Android version deletes its saves too.

When it does not start: if the game ends before it draws its first picture, the next start first shows a dialog with the reason (also copied to the clipboard, for a report; folder and file names are taken out of it) and tries again with OpenGL ES.

### Method 3: build it yourself from source

```powershell
git clone https://github.com/lming888/DBZ-SSD-zhTW.git
cd DBZ-SSD-zhTW
cargo build --release -p ssd-app
```

On its first run, give the built `target\release\ssd-recompiled.exe` the ROM with `--rom "path to ROM"`; after that just run it. Then place the files of `translations/zh-tw/` (and of `translations/zh-hk/` for the Hong Kong names) as in step 2 of method 2. Building needs Rust (`rustup` installs the version pinned in `rust-toolchain.toml`), CMake and a C compiler (the MSVC build tools on Windows; SDL3 is built from source); no ROM is needed. To keep the folders of the building machine out of the exe, run `. tools\no-local-paths.ps1` before `cargo build` (`. tools/no-local-paths.sh` on macOS or Linux). The command line is in "Build and run" of `docs/DEVELOPER.md`.

## Hong Kong names

One installer holds two Traditional Chinese translations, Taiwan's and Hong Kong's; they differ mainly in the names:

- Characters' names follow the usual Hong Kong names, for example 笛子魔童, 比達, 撒亞人, 格古洛, 八戒, 基爾, 彌太郎; the names inside technique names change with them. The names were checked with the help of players from Hong Kong.
- Technique names follow the Hong Kong names: 龜波氣功, 元氣彈 (超元氣彈), 魔貫光殺炮, 魔光炮, 氣功炮, 繰氣彈, 時間停頓, 破壞炮, 殛光球, 轉換身體.
- The opening's line naming the partners is 「孫悟空與笛子魔童」; the school mark of the cards is 「天神」; two lines of the opening captions lost a few characters to fit the width; the opening credit is 「繁體中文化」.

Choosing:
- The first start shows a language page listing Taiwan, then Hong Kong; pick one.
- To change it later, in Options press R (W on the keyboard) to reach the Accessibility tab, and switch the Language row left or right; it takes effect at once. The row's order is Taiwan, Hong Kong, English.
- Players upgrading from RC1: a folder that already has saves stays on Taiwan's names and shows no language page, and the saves are untouched; switch on that row for the Hong Kong names. Players upgrading from RC2 keep the language they chose.

## Features

- Chinese coverage: the game text, the HD menus, the captions of the three endings and the opening narration are in Traditional Chinese (Enhanced mode); the 68 dialogue pages that the English patch left as only a portrait or "…" but that have lines in the Japanese version are also shown whole in Chinese, following the Japanese version. The language pack is in `translations/zh-tw/`: `zh-tw.tsv` the game text, `ui-zh-tw.tsv` the words of the HD menus, `fonts/LXGWWenKaiTC-Regular.ttf` the font, `licenses/` the full licence texts of the fonts and of the emulator core. The Hong Kong names are in `translations/zh-hk/`: `zh-hk.tsv` the game text, `ui-zh-hk.tsv` the words of the HD menus; they share the font and the licence texts of `translations/zh-tw/`.
- Font: the Chinese of the game text and the HD menus uses LXGW WenKai TC 1.530 Regular (with 「為」 replaced by the Taiwan form of the glyph from LXGW WenKai 1.530, both SIL OFL 1.1). To go back to Noto Sans TC: in `save\` beside the exe, change `#font` on the third row of `zh-tw.tsv` and `ui-zh-tw.tsv` back to `NotoSansTC`, and put `NotoSansTC-Regular.otf` in `save\fonts\` (this font is not shipped with this project or its Releases; download it from the [Noto Sans TC page of Google Fonts](https://fonts.google.com/noto/specimen/Noto+Sans+TC) or from Noto's [notofonts/noto-cjk](https://github.com/notofonts/noto-cjk)). A changed `#font` is kept; when a later version updates other rows of the translation table, `#font` has to be changed again (details in `launcher/README.md`).
- HD art and voice: Super Vegeta without shoulder pads, Super Saiyan Goku's new head and the HD picture of his opening flight, the lightning and the voice of the transformation; Goku has the original black hair again in the ending's flight and arrival scenes, while his crouch holding Gohan facing Super Vegeta stays Super Saiyan (kept on purpose).
- One installer sets it all up, and what it installs is a single-exe launcher; the launcher can download the HD art pack for you, and falls back to the copy the author keeps when upstream cannot deliver it.
- Fixes: they cover text boxes and leftover letters, the ▼ continue marker, bag items, HD portraits and fills, art overrides, audio, the launcher and the installer; the main fixes are in the Release attachment `PATCH_NOTES.txt`.

## Launcher source

The launcher the installer installs, `DBZ_超賽亞傳說_繁中.exe`, has its source in this project's `launcher/` (`dbz-zhtw-launcher`, GPL-3.0-or-later), and the exe is built from it: it embeds `ssd-recompiled.exe` built from this project, the language packs and font of `translations/zh-tw/` and `translations/zh-hk/`, and the licence files into one file. The build steps are in `launcher/BUILD.zh-TW.md`. The optional feature `bundle-ips` is off by default; the launcher of this release does not turn it on and contains no ROM and no `dbz102.ips`. The installer's script is in `installer/` (see `installer/README.md`). The launcher source has an automatic update, enabled only when an update source is set at build time; this release sets none, so it is off entirely and never connects to check for updates.

## Changes

- **v1.0.1**: the gravity training's messages used to call every trainee Goku; they now name whoever actually trains. When Piccolo trains on King Kai's planet, "Goku, break through the gravity within N turns!" and "Goku is pinned down by the gravity!" now name Piccolo; when Goku trains they read as before. The narration "On King Kai's planet, Goku's training is about to begin…" now says "a training" instead of naming Goku. Both the Taiwan and the Hong Kong text; Classic (the original English) never had the problem. It also fixes a bug of the original game: Piccolo rejoining on Namek at a level outside 33 to 40 got an abnormal battle power (9,241,001 when he rejoins at level 41). Levels 41 to 48 now get the original's rejoin BP table extended upward by the same arithmetic (500,000 at level 41), level 32 or lower and level 49 or higher keep the battle power he had, and levels 33 to 40 are as in the original. This fix is in Enhanced mode only; Classic stays the original. A value already in a save is not changed automatically: set Piccolo's BP back with the built-in save editor ("Edit saves", Select on the title screen). The Android version also fixes, in Enhanced mode, choosing "Talk" or "Look" from the map menu sometimes closing the dialogue as soon as it opened, a held confirm button counting as a second press, and the music cutting out for a moment; contributed by [denlai-mshk](https://github.com/denlai-mshk) ([#22](https://github.com/lming888/DBZ-SSD-zhTW/pull/22)), thank you. This fix is in the Android version only in this release; the Windows version does not have it (no Windows player has reported the problem, and it has not been tested on Windows yet). The Windows installer is built from the tag `v1.0.1-windows`, which has the three lines above and Piccolo's return fix, and lacks this one fix only. Install it over v1.0 (on Android, the APK over the old one), the saves and settings are kept.
- Earlier releases: the `PATCH_NOTES.txt` attached to each Release.

## Known issues

The game screens
- The portrait of Vegeta's retreat line at the end of the Saiyan arc (it used to show Zarbon's face): the fix has not been verified in actual play.
- The battle pages that have only a portrait or "…", and a few pages whose names are filled in, have not each been seen on the actual screen; if you see a blank or text in the wrong place, please send a screenshot.
- On one page the Chinese "是／否" (yes/no) choice sits one character further right than in the Japanese version; choosing is not affected.
- Save screen: Enhanced mode uses the HD save screen, so the pixel-by-pixel check of the original save slots no longer applies; the text of the HD save screen was checked to be Chinese, its layout was not compared pixel by pixel.
- The battle HUD headers `HP`/`KI`/`BP` are deliberately kept in English (the background of Chinese characters covers the numbers of the next row).
- On the gravity-room page where you choose the multiplier, the Chinese "倍" appears about a quarter of a second later than the original "x"; this does not affect the choice.
- The wording of the HD menus (options, save editor and so on) was checked row by row against the program in this version; the rows still marked as to be reviewed are ones where the style of wording could be reconsidered, not missing translations.
- The language page of the first start is in Chinese; only the two letters "HD" on it are English.
- In the Hong Kong version, the big character on the cards and the square seal on the left of the status screen are still the single character 「神」: they are pictures, and this version changes only the school mark beside them to 「天神」.
- For about half a second after the item list (on the map and in battle) is opened, turned to a new page, or has a card dropped from it, the two Scouters are shown as 「探測器」 and the Frog and Turtle cards as the English `Turtle`, and then change to their own names; on a last page that is not full, the rows of the page before left below it are shown the same way. This does not affect play.
- If the settings file `save\saves\ssd.settings` is changed to an encoding other than UTF-8, the language page may appear at every start.
- The changes brought in by upstream v0.3.14 and v0.3.15 (a button leaves GAME OVER for the title, the "Pixel" look, the level cap, the "HD art" switch) have not been fully seen on the Chinese screens.
- After losing a battle and leaving the GAME OVER screen, "Continue" brings the cheats and the game options back to the settings of that save (as "Continue" after starting the game does).
- The first map screen after loading a save still pauses once, briefly.
- Up to RC2, "Continue" after GAME OVER lost cards (one of each stack; the single Gohan card disappeared). RC3 fixes it, but a save written after a card was lost does not get it back; add it again with 「卡牌產生器」 (Item spawner) on the 「作弊」 (Cheats) screen of the menu (it marks the save with the CHEATS badge).

Android
- v1.0 fixes the crash right after starting when the system refuses the Performance Hint (confirmed by a player on a real device). If it still closes right after starting, please paste the dialog's text into the report.

Installing and starting
- The installer has not been seen on a real machine at 125%-200% scaling or on Windows without the PMingLiU font; when text is cut off, please report it (with a screenshot and the scaling setting).
- Downloading the art pack from the copy when upstream cannot deliver it has not been run with a real loss of connection.
- The launchers of RC1 to v1.0 do not look for a newer version: download and run the new installer (on Android, install the APK over the old one). With RC1 and RC2 installed in two folders, the new version upgrades the one installed last; the other stays where it is: run `uninstall-dbz-zhtw.exe` there when you no longer want it.
- When reporting a problem, attach `last-run.log`, an F12 screenshot and where it happened (story part or map).
- On a Steam Deck running Windows 11, the game does not pick up the built-in controls; install [SWICD](https://github.com/mKenfenheuer/steam-deck-windows-usermode-driver) (it turns the built-in controls into a virtual Xbox 360 controller) and they work. That project is archived; its README says it is replaced by [steamdeck-kernel-driver](https://github.com/mKenfenheuer/steamdeck-kernel-driver), which has not been tested.

## Translation basis

- Names of characters and techniques follow the old Tong Li Publishing volumes (published 1992-1995), with a few names decided separately (for example 飛里沙, 基紐特戰隊, 大長老, 賽魯).
- Characters and techniques original to the game, without a Taiwanese name, use the names of the SNES Traditional Chinese translation of the same game.
- Wording follows usual Taiwanese usage (for example 選單, 卡牌, 金錢).
- Hong Kong names: characters' names follow the usual Hong Kong names, checked with the help of players from Hong Kong; details in "[Hong Kong names](#hong-kong-names)".

## Support

If you enjoy this Chinese edition and want to help with the time and tools behind it, you can buy me a coffee: [Ko-fi](https://ko-fi.com/ming888). Entirely optional: it supports the work on this project, not the game, whose rights belong to BANDAI and the other rights holders. Everything here is free: supporting is not a condition for downloading or using anything, and it unlocks nothing. Support is for my translation and fixes only, and has nothing to do with the upstream author's code and art.

This project is a fan work that is neither charged for nor sold, ships no ROM and is not affiliated with Bandai Namco (the full disclaimer is the last item, "Disclaimer", of "[License and credits](#license-and-credits)").

## License and credits

### Credits

- Game code: the game of this project is adapted from [JackRushante/DBZ-Super-Saiya-Densetsu-Rust](https://github.com/JackRushante/DBZ-Super-Saiya-Densetsu-Rust) (called upstream below; `ssd-recompiled` © JackRushante/DBZ-Super-Saiya-Densetsu-Rust and contributors, GPL-3.0) v0.3.15 (`b4a67c95`), and since then maintained and fixed by me on my own; the whole difference from upstream is the latest commit of this repository, and the main fixes are in the Release attachment `PATCH_NOTES.txt`. The game and the launcher of a release are built from this project's source, which is this repository. The original program `dbz-ssd-recompiled-windows.zip` used by installation method 2 is downloaded from upstream's [Releases](https://github.com/JackRushante/DBZ-Super-Saiya-Densetsu-Rust/releases) page.
- Engine technical notes: [`docs/ENGINE.en.md`](docs/ENGINE.en.md) (the original English technical notes of the Rust remake, written by its original author, kept as written).
- HD art pack: the work of the upstream project JackRushante/DBZ-Super-Saiya-Densetsu-Rust (*Dragon Ball Z: Super Saiya Densetsu - SNES Decomp / Classic / Remastered, Rewritten in Rust*), licensed [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/) (upstream `LICENSE-ART.md`; this repository has a copy too, [`LICENSE-ART.md`](LICENSE-ART.md)). Download it from upstream's [v0.3.9 release](https://github.com/JackRushante/DBZ-Super-Saiya-Densetsu-Rust/releases/tag/v0.3.9) (the later versions up to v0.3.15 use the same pack and attach no new one). The art pack is not shipped with this project or its releases; players download it themselves. When upstream cannot deliver it, the launcher downloads it from the copy the author keeps unchanged under the same licence ([hd-pack-v0.3.9](https://github.com/lming888/DBZ-Super-Saiya-Densetsu-Rust/releases/tag/hd-pack-v0.3.9)); the file is identical to upstream's.
- The two screenshots of this README (`docs/media/zh-tw-dialogue.png`, `zh-tw-battle.png`): the running game in Enhanced mode with upstream's HD art pack, unchanged. The art in them is the work of the upstream project JackRushante/DBZ-Super-Saiya-Densetsu-Rust, used under [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/) and offered under the same licence.
- The comparison picture in the installer (on the "HD art pack" page; `installer/ui/hd/plain_*.bmp` and `art_*.bmp`): two screenshots of the same frame of the running game, cut to the same area, placed one above the other and labelled "Original" and "HD"; the HD one (`art_*.bmp`) shows pictures of upstream's HD art pack. It is distributed with the installer, used under [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/) and offered under the same licence: the art is the work of the upstream project JackRushante/DBZ-Super-Saiya-Densetsu-Rust, and this project only cut and labelled it, without redrawing anything (`installer/ui/make_hd_compare.py`). The art pack itself is not in the installer; besides this picture, what the installer holds of the art pack's content is the Goku picture and the icon of the next item and the art overrides embedded in the launcher (see the items below).
- The installer's side and header pictures (`installer/wizard.bmp`, `header.bmp` and the files of the same names in `installer/ui/1x\` and `2x\`) and the program icon (`launcher/icon/app_icon.ico`): the figure is this project's picture of Super Saiyan Goku's opening flight (the item "Super Saiyan Goku's opening flight picture" below, redrawn by AI from the HD art pack's picture of normal Goku flying); the program only scaled, cut and laid it out. The speed lines behind him are AI-generated; the title lettering is LXGW WenKai TC. Offered under [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/).
- Art overrides (`save\art-override\`): each item below is provided inside the Release's launcher exe, not in the source repository (`launcher/assets/` is copied in only before packing and is not in git); it is written to `save\art-override\` on the first run, where the game uses it in place of the art-pack picture of the same name (`docs/mods/art.md`, "Replacing a picture of the HD pack"). The launcher also writes `LICENSE-ART.txt` (source, licence and who changed it) in `save\art-override\`; the licence of the original HD art pack is this repository's [`LICENSE-ART.md`](LICENSE-ART.md). Delete `save\art-override` to go back to the original pictures; the launcher will not write it again. The pictures redrawn from an official illustration or from the original pixels (Super Saiyan Goku's full-body charge, Super Vegeta's `key-*.png` poses, Piccolo's charging pose): the rights to the original image belong to their owners; non-commercial fan pictures.
  - Super Vegeta without shoulder pads: edited by Ming with the Google Gemini image model from the original pictures of the HD art pack, CC BY-NC-SA 4.0. The pictures of the `super-vegeta\` subfolder use normal Vegeta's picture names, and the game uses them only when the screen has Super Vegeta's palette; normal Vegeta is unchanged. The `goku\` subfolder holds black-haired pictures of normal Goku (Ming made the full-body charge `battle-goku-ssj-020` and the face close-up `battle-goku-ssj-019`, which the art pack has only in the Super Saiyan version, black-haired with the Google Gemini image model), under the Super Saiyan picture names, and the game uses them only when Goku's hair is not the Super Saiyan gold; the Super Saiyan is unchanged. The Super Vegeta flying over the GAME OVER caption (`super-vegeta\gallery-fx-1118.png`) was likewise edited by Ming with the Google Gemini image model from the art pack's original (no shoulder pads, Super Saiyan blond hair and green eyes), CC BY-NC-SA 4.0.
  - Super Saiyan Goku full-body charge picture (`save\art-override\variants\battle-goku-ssj-020.png`, the front view with the torn blue undershirt): redrawn by AI from an official illustration.
  - Super Saiyan Goku's new head (73 pictures in `save\art-override\variants\`: 53 `battle-goku-ssj-*` and 20 `gallery-party-goku-ssj-*`, numbered in the `LICENSE-ART.txt` the launcher writes): Ming edited the existing override pictures of the same names with the Google Gemini image model to give them one new head, then removed the background, scaled and aligned them. In `gallery-party-goku-ssj-170` (the back view of Goku pushing out the Kamehameha wave) the hands were redrawn separately with a Google Gemini image model: the art pack's original draws one hand too many in that pose. The base pictures are adapted from the HD art pack's original pictures, CC BY-NC-SA 4.0.
  - Normal Goku's Kamehameha back view (`save\art-override\variants\gallery-party-goku-u00-170.png` and `save\art-override\gallery-party-goku-u00-170.png`, the back view of the black-haired Goku pushing out the wave): the hands are the ones of the Super Saiyan `gallery-party-goku-ssj-170`, redrawn by Ming with a Google Gemini image model, put on the HD art pack's original picture. The base picture is the upstream HD art pack's original, CC BY-NC-SA 4.0; the adaptation keeps the same licence, non-commercial use only.
  - Super Saiyan Goku's opening flight picture (`save\art-override\mode7\cd83cbee861e14ad\t0_0.png`, the Mode 7 plane of Super Saiyan Goku flying in at the start of a battle): redrawn by Ming with the Google Gemini image model from the art pack's Mode 7 plane of normal Goku, first with Super Saiyan hair, then with torn clothes; the program only removed the background, scaled and aligned it. The base picture is adapted from the HD art pack's original, CC BY-NC-SA 4.0.
  - Super Vegeta's Kamehameha-hit, dodge and attack poses (`save\art-override\super-vegeta\key-*.png`): redrawn by AI from the original pixel art and existing pictures.
    - The three hit poses (`key-14b06ca30f359b50.png`, `key-9b36beaeedd4fd2b.png`, `key-9fb61f64d7d15be9.png`): the back view, pushed sideways and tumbling when Goku's Kamehameha hits Super Vegeta.
    - The dodge and attack poses (the other 16 `key-*.png`): the back view dodging the Kamehameha, flying off the screen edge dodging in melee, throwing the Kienzan, the close-up of the strong energy wave sliding in, rising from the bottom of the screen; the poses cut by the screen edge are cropped from the same batch of redrawn pictures or from existing pictures of the same folder.
    - The `.pose` of the same name holds only the pose's key, size, the colours it uses and the key of its mirror image (none of the original pixels); the game checks the pose with it.
  - Piccolo's charging pose (`save\art-override\gallery-party-piccolo-u01-061.png`): redrawn from the original pixels.
  - Transformation lightning (`save\art-override\ssj-lightning\`, drawn over the screen when Goku first becomes a Super Saiyan): AI-generated arc material placed by the program, not taken from the original game or the art pack. Delete the `ssj-lightning` folder to turn the lightning off.
- The authors of **snesrecomp** and the other SNES static recompilation projects, which showed the approach works; **Mesen2** and **Ghidra**, used for the research behind the notes in `docs/re/`.
- The Dragon Ball and SNES preservation communities, whose notes on the game's secrets and data helped.

### License

- Remake of the game: [JackRushante/DBZ-Super-Saiya-Densetsu-Rust](https://github.com/JackRushante/DBZ-Super-Saiya-Densetsu-Rust). This project is a derivative work of upstream plus fixes, and its code is distributed under the **GNU General Public License v3.0** ([`LICENSE`](LICENSE)); the game and the launcher of a release are both built from this project's source (the launcher is in `launcher/`), and the corresponding source is this repository. Third-party notices are in [`NOTICE.md`](NOTICE.md).
- The SNES emulator core jgenesis (jsgroth and contributors, GPL-3.0); full licence text in `third_party/jgenesis/LICENSE` and `translations/zh-tw/licenses/jgenesis-GPL-3.0.txt`.
- Fonts: LXGW WenKai TC (Chinese, LXGW WenKai TC 1.530, SIL Open Font License 1.1, `translations/zh-tw/fonts/OFL.txt`); IBM Plex Sans, JetBrains Mono, DotGothic16 and Share Tech Mono used by upstream (SIL Open Font License 1.1, `assets/fonts/` and `translations/zh-tw/licenses/`).
- English patch: Klepto Software v1.02. Upstream's text and paging follow this English version.
- Translation tables (`translations/zh-tw/*.tsv`, `translations/zh-hk/*.tsv`): © Ming, CC BY-NC-SA 4.0.
- Transformation voice (`voice\ssj-transform.wav`, inside the Release exe, installed into `save\voice\` on the first run): AI-generated voice, not an original recording.
- The software is provided "as is", without warranty of any kind.
- Rights holders: if you believe something here infringes your rights, open an issue on this repository and it will be handled promptly.
- **Disclaimer.** *Dragon Ball Z: Super Saiya Densetsu* was published by BANDAI, from the manga by Akira Toriyama and the anime by Toei Animation; the game's own code, graphics, music and text belong to their respective owners and are not part of this project. This is an unofficial fan project. It is **not affiliated with BANDAI (Bandai Namco), Toei Animation, Shueisha, Bird Studio or the other rights holders of the original work, and not licensed, endorsed or sponsored by them**; it is not affiliated with or endorsed by the upstream authors, and upstream's code and art are used under their public licences (GPL-3.0 / CC BY-NC-SA 4.0). *Dragon Ball*, *Dragon Ball Z* and all related names, characters and artwork are trademarks and works of their respective owners.
