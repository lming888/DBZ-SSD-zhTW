# regress45：繁中覆蓋層的回歸測試

一條指令跑完整套場景，每個場景印一行 PASS／FAIL／XFAIL／XPASS，最後印總數。
場景涵蓋全遊戲各區已修好的覆蓋層問題（英文殘字、中文畫錯視窗、▼ 繼續記號、名字插槽等），以及各區目前乾淨的守門畫面。
這套工具直接用本 repo 的 `xtask shots` 畫面，不需要其他 repo。

## 怎麼跑

```sh
python3 tools/regress45/run.py --inputs <放 ROM 與輸入錄影的資料夾>
```

- `--inputs DIR`（或環境變數 `DBZ_REGRESS_INPUTS`）：輸入資料夾，從裡面取
  `en.sfc`（英文版 ROM）、`pc039/hd/A8CF4F2A/1`（已安裝的 HD 美術包資料夾）、`saves/`（grav、grav-piccolo、nail-porunga 存檔；grav-piccolo 是 `xtask save-edit repro_base.srm grav-piccolo.sav --set party=piccolo --set map=21`，基底要用 `repro_base.srm`：探針的幀範圍是照這個基底量的；正式劇情裡太空船只有悟空，這個存檔是測機制用的）、
  `secrets/replays/`（劇情錄影；`tools/saves/secrets.sh` 產生，界王星重力試煉的兩支是 `king-kai-gravity-piccolo`、`king-kai-gravity-goku`）。也可以分別指定：`--rom`、`--pack`、`--saves`、`--replays`
  （環境變數 `DBZ_ROM`、`DBZ_HD_PACK`、`DBZ_SAVES`、`DBZ_REPLAYS`）。這些檔案都不進 repo，需要自備。
- `-k 字串`：只跑名字含這個字串的場景（可重複），例如 `-k porunga`。
- `-j N`：同時跑幾個掃描（預設 4；每個掃描是 2～4 支 shots 行程）。
- `--list`：只列出場景與說明，不跑。
- 偵測規則本身的測試（不用編譯、不用輸入資料）：`python3 -I tools/regress45/test_scan.py`（測 `row_fits` 對字列的判斷）。
- `--xtask BIN`：不自己編譯，直接用這支 xtask（必須是套了 `overlay_log.patch` 的版本，見下面「限制」）。
- `--legacy-env`：`--xtask` 是較舊的版本，用環境變數 `SHOTS_APP_HOOKS`、`SHOTS_RAW` 代替 `--app-hooks`、`--raw-out`
  參數（用來確認已修的場景在修正之前的版本上會 FAIL）。
- `--reuse`：參數、xtask、譯文表都沒變的掃描不重跑，直接重新判定（改了 `scenarios.py` 的判定時好用；
  改了探針（probe）的掃描會重跑）。
- `--work DIR`（或 `DBZ_REGRESS_WORK`）：工作資料夾，預設 worktree 裡的 `.work/regress45/`（git 已忽略）。
- `--keep`：保留每幀的 `report.tsv`（預設跑完就刪，只留摘要、事件、探針結果與證據 PNG）。

結束碼：0＝全部是 PASS 或 XFAIL；1＝有 FAIL、XPASS 或 ERROR。

兩支 `run.py` 共用同一個 `--work` 會互相清掉對方的輸出，不要同時跑。

時間：第一次要編譯 xtask（幾分鐘），之後增量編譯。整套在 12 核的機器上 `-j 4` 約 15～25 分鐘
（機器忙時更久）。磁碟：輸出約 100～200 MB（證據 PNG），編譯產物在 `.work/regress45/target`。

### 它做了什麼

1. **編譯**：把目前的 worktree（含未 commit 的改動，照 `.gitignore` 排除）用 rsync 複製到 `WORK/src`，
   在副本的 `crates/ssd-hd/src/translate.rs` 套上 `overlay_log.patch`，`cargo build --release -p xtask`
   （`CARGO_TARGET_DIR=WORK/target`）。worktree 本身的產品原始碼不動。修 bug 時直接在 worktree 改，
   再跑這條指令，測的就是改過的程式。
2. **準備**：英文 ROM 放進 `WORK/data/cache/rom/canonical.sfc`（`SSD_DATA_DIR`）；`xtask text-dump` 產生英文原文表
   （偵測規則要用）；用到的劇情錄影複製到 `WORK/replays` 後 `xtask rebase-replays`（改成這個核心版本，按鍵與存檔不變）；
   `scenarios.py` 的 `STATES` 做存檔狀態（`shots --state-out`）；對照組的譯文表（見下面「判定」）。
3. **掃描**：每個 run 跑一次 `scan.py`，輸出在 `WORK/out/<run>/`。畫面經匿名管道（`shots --raw-out /dev/fd/N`）
   進記憶體，檔案系統上沒有具名管道。
4. **判定**：每個場景依它的 run 的命中與探針結果判 PASS／FAIL／XFAIL／XPASS，結果也寫到 `WORK/results.txt`。

## 判定方式

### run 與場景

`scenarios.py` 的 `RUNS` 是掃描（腳本、畫哪些幀、`--wram` 作弊寫入、存檔、對照組），`SCENARIOS` 是斷言；
好幾個場景可以共用一次掃描。場景分三種：

| kind | 意思 | 預期 |
|---|---|---|
| `fixed` | 已修好的問題 | PASS；FAIL 表示又壞了 |
| `guard` | 目前乾淨的畫面 | PASS；FAIL 表示有人把它改壞了 |
| `known` | 確認但還沒修的問題（expected failure） | XFAIL；修好後變 XPASS，結束碼為 1，提醒把 `kind` 改成 `fixed` |

每個場景的說明（`--list` 印出來的那一欄）寫的是它檢查的現象與幀範圍。

### 偵測工具 scan.py

同一個場景同時開 2～4 支 `xtask shots`（繁中表、空表＝原版英文、需要時再加「對照組」的修改過的表），
每一幀都畫 HD（2 倍，744×478），畫面逐幀比對，只有命中的幀寫成 PNG。繁中那一支另外有覆蓋層每一幀的回報
（點名哪一頁、畫了哪些頁和底紙、畫面上每個英文字母格）。

規則（`scan.py` 開頭有完整說明）：

| 規則 | 意思 | 組別 |
|---|---|---|
| `REVERT` | 覆蓋層以前蓋成中文的英文字格，原版沒變，現在又露出英文 | 英文殘字 `ENGLISH` |
| `INPAPER` | 覆蓋層正在畫的底紙範圍裡有和原版一樣的英文字格 | 英文殘字 `ENGLISH` |
| `WRONGPAGE` | 底紙下被蓋掉的英文和那一頁的英文原文對不上（別頁的中文畫進這個視窗） | 中文畫錯視窗 `WRONG` |
| `MASKED` | 原版在底紙下換了畫面，繁中沒跟著換（中文蓋住新視窗） | 中文畫錯視窗 `WRONG` |
| `UNDRAWN` | 引擎點名有譯文的頁、英文在畫面上，覆蓋層卻沒畫 | 守門用 |
| `ZHONLY` | 原版沒變的地方繁中變了（閃一下、提早畫） | 只在指定的幀範圍內用 |

「英文殘字 0 幀、中文畫錯視窗 0 幀」＝ `no(ENGLISH + WRONG)`：整段沒有 REVERT、INPAPER、WRONGPAGE、MASKED。
守門場景用 `no(CLEAN)`（再加 UNDRAWN）。`ZHONLY` 在換頁、捲動時有單幀的雜訊，不放進守門，只用在指定的幾幀。

同一次掃描裡，`known` 場景的檢查「認領」的幀，`fixed`／`guard` 場景不算；場景也可以用 `allow(...)` 明寫不算的幀與理由。

### 像素探針（偵測規則抓不到的問題）

有些問題規則抓不到（例如名字被擦掉、金錢框變空白、▼ 不見、USE 沒翻）。這些用 `scan.py --probes` 的像素探針，
在指定的框（2 倍畫面的像素座標）與幀範圍逐幀比：

| kind | 壞幀的條件 | 用在 |
|---|---|---|
| `equal a b` | 兩張畫面在框內不一樣的像素超過 `tol` | 與對照組比（下面） |
| `differ a b` | 兩張畫面在框內幾乎一樣（不超過 `tol`） | KI-USE：繁中應該和原版不同（畫中文） |
| `stable a ref` | 原版的框和第 `ref` 幀一樣，繁中卻和第 `ref` 幀不一樣 | 金錢框、HP 窗：原版沒動，繁中自己壞掉 |
| `blink a color` | 整段裡繁中框內這個顏色的像素數變化小於 `k`（▼ 沒在閃）；原版沒在閃也算壞（框或幀選錯） | ▼ |
| `ink a` | 框內深色像素少於 `min`（該有字的地方沒字）；給 `max` 時改成多於 `max`（不該有字的地方有字） | 名字插槽、KI 連字號 |
| `mark a color` | 逐幀：原版的 `obox` 有 ▼（`color` 至少 `min` 點）而繁中的 `box` 沒有；或原版沒有而繁中有（給了 `paper` 時，繁中框內只要有底紙色以外的像素就算） | ▼ 跟著原版出現、消失（換頁、假 ▼） |

**對照組**（`controls`）：用拿掉某一頁（`drop`）或改了某一頁（`set`）的譯文表再跑一支 shots，問的是
「這一頁不該影響這個框」。例如爺爺的選敵人窗：名字框在繁中（完整譯文表）要和「拿掉 help:7 的譯文表」一模一樣；
說明 help:7 若被畫進名字欄，兩者就不同。每個對照組的畫面都要先看圖確認是正確的樣子
（例如拿掉 help:1 後丟棄確認窗是完整的「金錢 000P」「丟掉嗎？」）。

每個探針壞的第一幀存成 `WORK/out/<run>/probe_<id>_f<幀>.png`（左起：繁中、比較對象、原版，框標紫色）。
規則命中的幀在 `WORK/out/<run>/frames/`（左繁中、右原版）。

### 構造的場景

`sd_guard_names_*`、`sd_empty_*` 這幾組不是自然遊玩走到的：grav 存檔繼續遊戲後在 2304 幀寫 `scene_id`（`$01D8`）與
`$1648`＝`$80` 直接啟動場景（`scenes/story_names/cont_mash.txt`、`scenes/story_empty/cont_slow.txt`），需要時再用 WRAM
寫入隊伍紀錄或劇情旗標。名字插槽的守門場景已做過突變檢查：換回修正前的譯文表，s51、s2D、s1E 變 FAIL；只把
`unit:0x54` 改回茲姆利，s2D 變 FAIL。

## 修好一個已知問題之後

1. 跑 `python3 tools/regress45/run.py --inputs ... -k <場景名>`，應該是 XPASS。
2. 打開那個 run 的 `frames/`、`probe_*.png` 看圖確認真的好了。
3. 把 `scenarios.py` 裡那個場景的 `kind` 從 `'known'` 改成 `'fixed'`，再跑一次整套。

## 加新場景

1. 時間軸（`scenes/<區>/*.txt`，格式見 `ssd_game::scripts::parse_timeline`）放進 `scenes/`；幀號先用 `probe.py` 量。
2. 在 `RUNS` 加一次掃描（`--from` 要比要檢查的視窗打開早，覆蓋層靠前面幾幀的記憶判斷；建議至少早 200 幀）。
3. 在 `SCENARIOS` 加斷言。探針的框用證據 PNG 量（scan 的 PNG 上方有 36 像素的標頭，探針 PNG 是 20 像素）。

`probe.py`：不畫圖、快速列出每一幀的 game_mode、menu_screen、message_state 等變化（約 400 幀/秒），
用法 `python3 tools/regress45/probe.py --bin <xtask> --data <WORK/data> --script <腳本> --upto <幀> --hooks`。

## 冷起跑的比對：sweep.py

場景的 run 都從 `lo` 起逐格畫，覆蓋層看得到前面每一幀。讀存檔、切換語言之後，覆蓋層是從中途才開始看，
有些英文殘字只有這時才會出現（例如上一頁伸出視窗的字：覆蓋層沒看到寫上它的那一頁）。
`sweep.py` 用整支錄影找這一類問題：

```sh
python3 tools/regress45/sweep.py --inputs <輸入資料夾>                          # secrets/replays 裡的每一支
python3 tools/regress45/sweep.py --inputs <輸入資料夾> secret-nail-porunga      # 只跑這幾支
python3 tools/regress45/sweep.py --inputs <輸入資料夾> --every 10 --from 9000 --to 10300 secret-nail-porunga
```

- 先跑過一次 `run.py`：它用 `run.py` 工作資料夾裡編好的 xtask 與資料夾（`--work`、`--xtask` 可另外指定），
  錄影複製到 `WORK/sweep_replays/` 再 rebase。
- 每支錄影開三支 shots：繁中逐格畫（覆蓋層看得到每一幀）、繁中每 `--every` 幀畫一幀（shots 只在畫的那一幀跑覆蓋層，
  等於每一幀都從頭開始看）、原版英文同樣的幀。取樣的幀上，「從頭開始看的繁中和原版一樣、逐格的繁中和原版不一樣」
  而且夠亮（`--bright`：底紙和字的格子，不算深色的畫面）的像素超過 `--min` 就算命中。
- 命中的幀、像素數、範圍印在畫面上，也寫到 `WORK/sweep/<錄影>/hits.txt`；每支前 `--png` 個命中存圖
  （左起：逐格、從頭開始看、原版，放大 3 倍）。
- 命中不一定是錯：兩頁之間引擎沒有點名任何頁的那一兩幀，覆蓋層沿用上一頁，從頭開始看的覆蓋層不知道是哪一頁，
  這一兩幀會命中。要看圖判斷。
- 每支印出取樣的幀數與實際比到的幀數（三支 shots 都畫出來的幀）；三支畫出的幀對不起來（某一支中途停下）算執行出錯。
- 時間：每支錄影 2～4 分鐘，`-j 4` 同時跑 4 支。結束碼：0＝沒有命中，1＝有命中，2＝執行出錯。

確認過的問題，在 `scenarios.py` 用 `STATES` 存中途的狀態、加一個從那裡起跑的 run 收成場景
（例如 `sd_porunga_cold`：第 9600 幀的狀態，從 9601 幀起跑）。

## 限制

- **`overlay_log.patch` 不在產品原始碼裡。** 偵測規則要讀覆蓋層每一幀的狀態（點名頁、畫的頁與底紙、字母格），這些是
  `ssd-hd` 裡 `Translator` 的私有欄位，xtask 讀不到，只能在 `Translator::draw` 結尾加一段只讀的輸出（設了
  `SSD_OVERLAY_LOG` 才印）。為了不讓產品程式多出測試用的輸出，這段做成 patch，由 `run.py` 套在複製出來的原始碼上編譯。
  `translate.rs` 的 `draw` 結尾改了之後 patch 可能套不上，`run.py` 會停下來說明；這時照新的程式碼更新 patch
  （內容只是把 `self.screen`、`self.kept`、`self.behind` 與 `cells` 印出來）。要根治，可以把這段（或一個公開的唯讀查詢）正式收進 `ssd-hd`。
- **xtask shots 的兩個參數**：`--app-hooks`（裝上遊戲本體的 hook 表：作弊與劇情修正、mods 的 redirector，和遊戲一樣）與
  `--raw-out FILE`（每一幀的 RGBA 寫到檔案或管道），附 `shots::tests` 的測試。
- **照不到的畫面**：遊戲本體自己畫的 HD 選單（`ssd-menus`：選項、作弊、資料包、存檔編輯、按鍵設定、HD 道具袋等）
  shots 不畫。所以地圖道具清單等場景看的是 shots 的「SNES 視窗＋覆蓋層」畫面，加強模式下玩家不一定看得到。
  經典模式、精靈（OAM）與背景圖裡的字、標點不檢查。
- 時間軸是固定幀數：遊戲是決定性的，同樣的輸入每次一樣；但改了遊戲邏輯（不只是覆蓋層）讓畫面出現的幀數變了，
  場景的幀範圍要重量。
- 探針的框是固定座標：視窗位置或譯文長度改了（例如名字插槽的修法改了排版），框可能要跟著調。
- 對照組的假設是「拿掉那一頁之後，那個框畫的就是正確的樣子」；修法若讓那一頁本來就該影響那個框
  （例如改成在名字欄畫別的東西），探針要重新設計。
- 各問題只收一個場景（多半是 Saibai 戰的版本）；同一個問題在 Frieza 戰（四人隊）、別的道具、別的時間點的變化沒有另外收。

## 檔案

| 檔案 | 用途 |
|---|---|
| `run.py` | 一條指令跑整套：編譯、準備、掃描、判定 |
| `scenarios.py` | 所有 run 與場景 |
| `scan.py` | 偵測工具（規則＋像素探針＋對照組） |
| `probe.py` | 找幀用：不畫圖，印出遊戲狀態變化 |
| `test_scan.py` | 偵測規則的單元測試 |
| `sweep.py` | 冷起跑的比對：逐格畫與每隔幾幀從頭看的繁中（讀檔、切換語言之後才會出現的殘字） |
| `overlay_log.patch` | 給偵測工具用的覆蓋層每幀輸出（只在編譯副本上套用） |
| `empty.tsv` | 空譯文表（原版英文的對照） |
| `scenes/` | 各區的按鍵時間軸 |
