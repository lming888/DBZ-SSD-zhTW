> **這個 Fork 的 Android 顯示功能，是為 Samsung DeX 接 CRT-TV 的復古遊玩配置而設。**
>
> 用 Samsung 手機運行遊戲，經有線 Samsung DeX 輸出至真正的 CRT 電視，享受映像管自然產生的掃描線與復古觀感，同時保留加強模式及 HD 美術帶來的遊戲體驗。

**訊號接法**

`支援有線 Samsung DeX 的手機 → USB-C 轉 HDMI → HDMI 輸入／S-Video 輸出的主動式轉換器 → CRT-TV`

HDMI 的數位訊號須由轉換器轉成 S-Video 類比訊號，再接到 CRT；不是使用反方向的轉換器或單純的轉接線。

**設定方法**

1. 先到 App 的觸控選單 `MENU → Options`，將「畫面（Picture）」切換為「拉伸（Stretch）」。
2. 再到遊戲內選單 `Options → Video`，將「長寬比（Aspect）」設為 `4:3`。
3. 同一頁將「顯示（Display）」設為「全螢幕（Fullscreen）」。
4. 啟用有線 Samsung DeX，按上述 `Android → HDMI → S-Video → CRT` 訊號鏈連接電視，開始遊玩。

**為甚麼加入拉伸模式？** 這個模式正是為擁有以上配置的玩家而加入：讓 Android／DeX 的輸出配合 4:3 CRT，目標是把真實、自然的 CRT 掃描線，與 HD 美術及加強模式結合，打造專屬這套配置的「Perfect Retro」體驗。

> **硬體與版本注意**：手機必須支援有線 DeX，轉換器與電視也須相容；實際比例與掃描線觀感會受硬體及輸出制式影響。此處的 HD 體驗指遊戲美術與內部渲染，S-Video 傳送到 CRT 的仍是類比標清訊號，並非真正的 HD 輸出。以上設定需要包含「拉伸」及 CRT 比例修正的新版 Android 建置，不代表現有 RC2 下載已包含這些改動。

---

# 七龍珠Z 超賽亞傳說　繁體中文 Remaster（台灣、香港）

English: [README.en.md](README.en.md)

作者：Ming Liu（GitHub：[lming888](https://github.com/lming888)）

**下載**：[RC2 安裝程式與 Android 版 APK](https://github.com/lming888/DBZ-SSD-zhTW/releases/tag/rc2)

**目前狀態**：RC2，歡迎試玩回報。

![劇情對話的繁中畫面](docs/media/zh-tw-dialogue.png)
![戰鬥訊息的繁中畫面](docs/media/zh-tw-battle.png)

這是 SFC《七龍珠Z 超賽亞傳說》（1992）的繁體中文版，有台灣、香港兩套譯名，第一次啟動時選（見「[香港版譯名](#香港版譯名)」）。我做的部分：

- **完整繁中譯文**：劇情對話、戰鬥訊息、道具與招式說明、選單、存檔編輯器，共兩千多句。劇情和遊戲文字從日版原文翻譯，再對照英文版的分頁排進去；重製版新增的選單從英文翻譯。英文版留成只有頭像或「…」的 68 頁對話，照日版把台詞補回來；三段結局字幕與開場旁白也都翻了。譯名以台灣動畫配音版為準。
- **中文顯示**：在遊戲畫面上用霞鶩文楷畫出中文，這一層是另外寫的（約一萬七千行程式）：對話的斷行、換頁符號 ▼ 的位置、每一種視窗的排版都逐一處理；換頁、開選單、移動游標時，中文和原版畫面同時出現、同時消失。
- **安裝程式與啟動器**：全新寫的（約七千行程式），一個安裝程式裝好：選 ROM、套英文補丁都在裡面，不用自己搬檔案；HD 美術包可以代為下載並核對檔案，主要來源抓不到時改從備用來源下載；重裝會保留存檔，解除安裝預設也保留。
- **重繪的美術**：超賽悟空換新頭（73 張）與開場飛行的高畫質圖、龜派氣功的手、超級達爾去墊肩，以及變身時的閃電與語音，逐項出處見最末「授權與致謝」。
- **逐格檢查**：遊戲各區（劇情、地圖選單、戰鬥、商店、小遊戲、開場）的中文畫面逐格看過，另有 82 個場景的自動畫面比對，每次改版都重跑。
- **修正與調整**：見 Release 附件 `PATCH_NOTES.txt`。

遊戲本體改自 [JackRushante/DBZ-Super-Saiya-Densetsu-Rust](https://github.com/JackRushante/DBZ-Super-Saiya-Densetsu-Rust) v0.3.15（GPL-3.0）。中文只在加強模式（Remastered）顯示，經典模式（Classic）是原版英文。遊戲程式與 HD 美術包的出處和授權見最末「[授權與致謝](#授權與致謝)」；上面兩張截圖是加強模式掛上游 HD 美術包的實際畫面。

## 你需要準備

- Windows 10／11（64 位元）。Android 版要 Android 8.0 以上的 arm64（64 位元 ARM）手機或平板，見「[Android 版](#android-版)」。
- 自備遊戲 ROM：日版《ドラゴンボールZ 超サイヤ伝説》ROM 加 Klepto Software 英文補丁 v1.02 的 `dbz102.ips`，或已經打好 v1.02 補丁的英文版 ROM。本專案不含、也不提供任何 ROM、補丁檔（ips）、存檔或遊戲資料。
- （可選）HD 美術包 `hd-pack.zip`，請用 v0.3.9 的包（484,121,119 位元組）。舊版的包缺少超賽悟空豎髮與頭像修正用到的圖，啟動器會提示更新，但照常啟動。美術包不是本專案的作品，下載位置、作者與授權（CC BY-NC-SA 4.0）見最末「[授權與致謝](#授權與致謝)」。本 repo 與 Release 都不附美術包。上游下載不到時，啟動器會改從作者依 CC BY-NC-SA 4.0 原樣保留的副本（[lming888/DBZ-Super-Saiya-Densetsu-Rust 的 hd-pack-v0.3.9](https://github.com/lming888/DBZ-Super-Saiya-Densetsu-Rust/releases/tag/hd-pack-v0.3.9)）下載，檔案與上游完全相同。

ROM 與補丁的檢查碼：

| 檔案 | 大小（位元組） | CRC-32 | SHA-1 | SHA-256 |
|---|---|---|---|---|
| 日版 Rev 1 ROM（無檔頭） | 1,048,576 | `ADE7B968` | `6b5725765ddde078350ec61221ddb9bb252066a7` | `962aa7a09765a97164af67098877a8fe5b7f1ea9db738561eb466b7600fd241c` |
| Klepto Software 英文補丁 v1.02 `dbz102.ips` | 62,587 | `F6D6719F` | `171d8925e857985427c5995041bf4c6fdc47124e` | `a1607090a6534d6b1711413ae07252c492d6c4c809868a2b6265b32737bfeed8` |
| 英文版 ROM（打補丁後、檔頭檢查碼未修正） | 1,048,576 | `3BB02A05` | `252c1b3eaa1084dcde0aa8f7bad9caed726b53be` | `31cec05b84a1717be4fb914d5d9748bfa827f66d4b692751705d48c4a2646e57` |
| 英文版 ROM（檔頭檢查碼已修正，遊戲實際使用的版本） | 1,048,576 | `A8CF4F2A` | `6b612f6d99f2efc26f26fb22290bc732ada0ea6f` | `0cc43e7550e87e9d0f9a4e8736beb4f48af46c03bf5c5cc39d847aa3cbd17a4f` |

上述三種 ROM 任一種都可以（日版要再配 `dbz102.ips`）。Windows 可用 `certutil -hashfile 檔名 SHA1` 檢查。`dbz102.ips` 原本是做給帶 512 位元組檔頭的 ROM，遊戲會自己對好位置，不用先加檔頭。手上的 ROM 帶檔頭（1,049,088 位元組）也能用，遊戲會自動去掉檔頭；這種檔整個算出來的檢查碼會和上表不同。

## 安裝

### 方法一：下載 Release（建議）

RC2 的 Release 提供安裝程式 `DBZ-SSD-zh-TW-RC2-setup.exe`（已附在 Release 頁面上）：

- **安裝程式**：
  - 只裝給目前的使用者，不需要系統管理員權限。
  - 預設裝到 `%LOCALAPPDATA%\Programs\DBZ-SSD-zhTW\`，安裝時可以改到其他自己能寫入的資料夾；選到已有其他檔案的資料夾時，會改裝到它裡面的 `DBZ-SSD-zhTW` 資料夾。
  - 可以順便選好自己的 ROM 與 IPS；略過的話，第一次開啟時再選。
  - 選好 ROM 之後有一頁「HD 高畫質美術包」：左邊是同一格遊戲畫面的原版與 HD 對照圖，右邊說明美術包是上游作者的作品、約 462 MB。勾「安裝完成後下載 HD 美術包」（預設不勾）就會在安裝完成後下載；安裝位置已經有美術包時只告知、不再問。完成頁有「立即開啟遊戲」，兩個都勾時先下載美術包、下載完才開遊戲。
  - 建立桌面捷徑「七龍珠Z 超賽亞傳說 繁中版」與開始功能表資料夾「七龍珠Z 超賽亞傳說 繁中版」，資料夾裡有遊戲與「下載 HD 美術包」兩個捷徑。
  - 可在「新增或移除程式」解除安裝。預設保留存檔與 HD 美術包；要一起刪，就勾「同時刪除存檔、ROM 設定、HD 美術包與遊戲設定」。
  - 沒有程式碼簽章：第一次執行時 Windows SmartScreen 會顯示「Windows 已保護您的電腦」，點「其他資訊」再點「仍要執行」。

1. 執行安裝程式，照畫面完成即可。安裝位置選在雲端同步資料夾裡時，存檔也會被同步，偶爾會被鎖住。
2. 用桌面或開始功能表的捷徑開啟遊戲。安裝時沒選 ROM 的話，第一次開啟會依序問兩個檔：先選 ROM（`.sfc`／`.smc`），再選 `dbz102.ips`；如果剛才選的已經是英文版 ROM，第二步按「取消」。之後直接雙擊就進遊戲。
3. HD 美術：還沒有美術包時，第一次開啟會問要不要下載（約 462 MB；按「下載」前不連網，可隨時取消）。也可以在安裝時勾選，或之後用開始功能表的「下載 HD 美術包」。下載與核對的細節見下一節。
4. 存檔在 exe 旁的 `save\` 資料夾。錯誤紀錄在 `%LOCALAPPDATA%\dbz-ssd-zhtw\last-run.log`。

### HD 美術包的下載

- 啟動器詢問時有「下載」「這次不要」「不再詢問」三個按鈕。下載的是上游 GitHub 發行頁上 v0.3.9 的 `hd-pack.zip`；下載時顯示進度視窗（已下載／總大小、百分比），大小與 SHA-256 核對無誤才放到 exe 旁，之後就會使用。
- 上游抓不到檔（連不上、逾時、中途斷線、轉址到不允許的主機、大小或 SHA-256 不對）時，會自動改從作者原樣保留的副本下載同一個檔，核對方式和上游相同。你按取消、另一個啟動器正在下載、或寫檔失敗時，不會改抓副本。
- 也可以自己把 v0.3.9 的 `hd-pack.zip`（484,121,119 位元組）放在 exe 旁邊，下次啟動會自動安裝。放的不是 v0.3.9 的包時，啟動器會跳出提示並記在 `last-run.log`，遊戲照常開。

### 之後再下載 HD 美術包

之前選了「這次不要」或「不再詢問」、後來想要美術包：用開始功能表「七龍珠Z 超賽亞傳說 繁中版」資料夾裡的「下載 HD 美術包」捷徑。它只下載美術包、不開始遊戲，會先問一次，下載時一樣有進度視窗、可以取消。exe 旁原本的 `hd-pack.zip` 不是 v0.3.9 時，舊檔會改名保留，不會刪掉（細節見 `launcher/README.md`）。

這個捷徑就是加了 `--hd-pack` 的啟動器。也可以在命令提示字元執行 `"%LOCALAPPDATA%\Programs\DBZ-SSD-zhTW\DBZ_超賽亞傳說_繁中.exe" --hd-pack`（裝在別處就換成實際的位置）。`--help` 會列出所有參數。

### 預設按鍵（鍵盤）

| 遊戲按鈕 | 鍵盤 |
|---|---|
| 十字鍵 | 方向鍵 |
| A | X |
| B | Z |
| X | S |
| Y | A |
| L | Q |
| R | W |
| Start | Enter |
| Select | 右 Shift |

- 其他快速鍵：F5 即時存檔、F9 即時讀檔、1～4 選即時存檔欄、Tab 切換速度、P 暫停、F12 截圖、F1 面板、F3 戰鬥作弊、Esc 結束。F1 除錯面板只在加 `--debug` 時有效。
- HD 選單（選項、存檔編輯器）裡用 L／R（Q／W）換頁籤。
- 標題畫面按 Select（右 Shift）開存檔編輯器。
- 按鍵與手把對應在「選項 > 操作」裡改（數字鍵 1～4、F1、F3、Esc 固定不能改）。

安裝程式裝的啟動器 `DBZ_超賽亞傳說_繁中.exe` 裡面包著由本專案原始碼編出的遊戲本體 `ssd-recompiled.exe`、語言包與字型，第一次執行時解開到 `%LOCALAPPDATA%\dbz-ssd-zhtw\`。

### 方法二：原版 Rust 重製程式＋語言包

不換遊戲本體、只放語言包也能顯示中文：

1. 下載原版 Rust 重製程式 v0.3.15 的 `dbz-ssd-recompiled-windows.zip`（下載位置見最末「[授權與致謝](#授權與致謝)」），照裡面的 `QUICKSTART.txt` 用 `Set up my ROM.bat` 設定 ROM 一次。
2. 在本 repo 頁面按 Code → Download ZIP，解壓縮後把 `translations\zh-tw\` 裡的 `zh-tw.tsv`、`ui-zh-tw.tsv` 與 `fonts\LXGWWenKaiTC-Regular.ttf` 放進 `%LOCALAPPDATA%\dbz-ssd-recompiled\`（或 `ssd-recompiled.exe` 旁邊），保留 `fonts\` 子資料夾。要香港版譯名的話，再把 `translations\zh-hk\` 裡的 `zh-hk.tsv`、`ui-zh-hk.tsv` 放進同一個資料夾（字型共用上面那一個）。檔名必須是小寫 `zh-tw.tsv`、`ui-zh-tw.tsv`、`zh-hk.tsv`、`ui-zh-hk.tsv`。
3. 用 `Play Remastered.bat` 開遊戲，在 Options > Accessibility > Language 選「繁體中文（台灣）」；放了港版兩張表的話，也可以選「繁體中文（香港）」。

兩種方法的差別：方法二沒有我寫的修補。畫面上會看到英文殘字（戰鬥訊息開頭先閃英文名字、說明框退回英文、框外留英文字母等，文字覆蓋層／文字框的修正都沒有）、讀檔或存到另一格後背包道具變少、用 HD 美術包時部分頭像畫錯（達爾、基紐、尼爾、地圖上的飛里沙）。

### Android 版

1. 需要 Android 8.0 以上、arm64（64 位元 ARM）的手機或平板，以及上面「你需要準備」的同一份 ROM。
2. 從 Release 下載 `DBZ-SSD-zh-TW-RC2-arm64.apk`，在手機上開啟安裝。這個 APK 沒有上架商店，第一次安裝時系統會要求允許這個來源「安裝不明應用程式」。
3. 第一次開啟先出現語言頁（台灣、香港），接著用系統的檔案選擇器選 ROM；選日版 ROM 的話，會再要你選 `dbz102.ips`。
4. （可選）HD 美術包：先把 `hd-pack.zip` 下載到手機，再到「選項」裡的「安裝 HD 美術包」用檔案選擇器選它（安裝後約佔 0.9 GB）。

注意：這個 App 的套件名與上游的 Android 版相同、簽章不同，裝過上游 Android 版的要先解除安裝才能裝；Android 版解除安裝會一併清掉存檔。

### 方法三：從原始碼自己編

```powershell
git clone https://github.com/lming888/DBZ-SSD-zhTW.git
cd DBZ-SSD-zhTW
cargo build --release -p ssd-app
```

編好的 `target\release\ssd-recompiled.exe` 第一次執行時用 `--rom "ROM 路徑"` 指定 ROM，之後直接執行即可，再把 `translations/zh-tw/`（要香港版再加 `translations/zh-hk/`）的檔案照方法二第 2 步放好。建置需要 Rust（`rustup` 會裝 `rust-toolchain.toml` 指定的版本）、CMake 與 C 編譯器（Windows 用 MSVC 建置工具；SDL3 從原始碼編譯），不需要 ROM。要讓 exe 不留建置機器的資料夾路徑，`cargo build` 之前先執行 `. tools\no-local-paths.ps1`（macOS／Linux 是 `. tools/no-local-paths.sh`）。命令列參數見 `docs/DEVELOPER.md` 的 Build and run。

## 香港版譯名

同一個安裝程式含台灣、香港兩套譯文，兩套主要差在譯名：

- 人名改用香港通行的叫法，例如笛子魔童、比達、撒亞人、格古洛、八戒、基爾、彌太郎；招式名只換掉其中的人名。譯名由香港玩家協助校對。
- 開場搭檔那句是「孫悟空與笛子魔童」；卡牌的流派標記是「天神」；開場字幕有兩句因為寬度刪了幾個字；開場署名是「繁體中文化」。

怎麼選：
- 第一次啟動時會出現語言頁，依序列出台灣、香港，選一個即可。
- 之後要換，到「選項」用 R（鍵盤 W）切到「輔助功能」頁，在「語言」那一列左右切換，立即生效。這一列的順序是台灣、香港、English。
- 從 RC1 升級的玩家：已經有存檔的資料夾維持台灣版、不會出現語言頁，存檔不動；想用香港版就到上面那一列切換。

## 特色

- 中文範圍：遊戲文字、HD 選單、三段結局字幕與開場旁白都是繁體中文（加強模式）；英文補丁留成只有頭像或「…」、日版有台詞的 68 頁對話，也照日版整頁顯示中文。語言包在 `translations/zh-tw/`：`zh-tw.tsv` 遊戲文字、`ui-zh-tw.tsv` HD 選單用詞、`fonts/LXGWWenKaiTC-Regular.ttf` 字型、`licenses/` 各字型與模擬器核心的授權全文。香港版譯名在 `translations/zh-hk/`：`zh-hk.tsv` 遊戲文字、`ui-zh-hk.tsv` HD 選單用詞，字型與授權全文共用 `translations/zh-tw/` 的。
- 字型：遊戲文字與 HD 選單的中文用霞鶩文楷 TC（LXGW WenKai TC 1.530 Regular；「為」換成霞鶩文楷 1.530 的台灣寫法字形，兩者皆 SIL OFL 1.1）。想改回思源黑體（Noto Sans TC）：把 exe 旁 `save\` 裡 `zh-tw.tsv`、`ui-zh-tw.tsv` 第三列的 `#font` 改回 `NotoSansTC`，並把 `NotoSansTC-Regular.otf` 放進 `save\fonts\`（這個字型不隨本專案與 Release 附上，請到 [Google Fonts 的 Noto Sans TC 頁面](https://fonts.google.com/noto/specimen/Noto+Sans+TC)或 Noto 官方的 [notofonts/noto-cjk](https://github.com/notofonts/noto-cjk) 下載）。改過的 `#font` 會保留；之後的版本若譯文表有其他列更新，`#font` 要再改一次（細節見 `launcher/README.md`）。
- HD 美術與語音：超級達爾去墊肩、超賽悟空換新頭與開場飛行的高畫質圖、變身時的閃電與語音；結局飛行幕與抵達幕的悟空改回原作黑髮，抱著悟飯對超級達爾的蹲姿維持超級賽亞人（刻意保留）。
- 一個安裝程式就裝好，裝好的是單一 exe 的啟動器；HD 美術包可以由啟動器代為下載，上游抓不到時改從作者保留的副本下載。
- 修補：我寫的修補涵蓋文字框與殘字、▼ 繼續記號、背包道具、HD 頭像與補圖、美術覆蓋圖、音訊、啟動器與安裝程式，主要修正見 Release 附件 `PATCH_NOTES.txt`。

## 啟動器原始碼

安裝程式裝的啟動器 `DBZ_超賽亞傳說_繁中.exe`，原始碼在本專案的 `launcher/`（`dbz-zhtw-launcher`，GPL-3.0-or-later），exe 就是由它建出來的：把本專案編出的 `ssd-recompiled.exe`、`translations/zh-tw/` 與 `translations/zh-hk/` 的語言包與字型、授權檔嵌進一個檔。建置步驟見 `launcher/BUILD.zh-TW.md`。可選功能 `bundle-ips` 預設關閉；本 Release 的啟動器沒有開這個功能，不含 ROM，也不含 `dbz102.ips`。安裝程式的腳本在 `installer/`（見 `installer/README.md`）。啟動器原始碼裡有自動更新功能，只在建置時設定了更新來源才會啟用；本 Release 沒有設定，所以完全關閉，不會連網檢查更新。

## 已知問題

遊戲畫面
- 賽亞人篇結尾達爾撤退台詞的頭像（修正前畫成撒保的臉）：修正沒有在實際遊玩中驗證過。
- 戰鬥中「只有頭像或…」的對話頁、少數幾頁代入的名字，還沒在實際畫面上逐一看過；看到空白或位置不對請截圖回報。
- 有一頁「是／否」選項的中文比日版偏右一格，選擇不受影響。
- 存檔畫面：加強模式改用 HD 存檔畫面，原版存檔欄的逐像素檢查不再適用；HD 存檔畫面的文字有確認過是中文，版面沒有逐像素比對。
- 戰鬥 HUD 表頭 `HP`／`KI`／`BP` 刻意保留英文（中文字的底色會蓋住下一列數字）。
- 重力室選倍數那一頁，中文的「倍」比原版的「x」晚約四分之一秒出現，不影響選擇。
- HD 選單（選項、存檔編輯器等）的用詞這版逐列對過程式；還標成待校的列是用詞風格可以再斟酌，不是漏譯。
- 第一次啟動的語言頁是中文，只有頁面上的「HD」兩個字母是英文。
- 香港版的卡牌大字與能力畫面左側方印仍是單字「神」：那是圖，這版只把旁邊的流派標記字改成「天神」。
- 設定檔 `save\saves\ssd.settings` 若被改成不是 UTF-8 的編碼，可能每次啟動都會出現語言頁。
- 讀取存檔後，畫面上的英文會短暫出現一下才換成中文，不影響遊玩。
- 上游 v0.3.14、v0.3.15 帶進來的改動（GAME OVER 按鍵回標題、外觀「像素」、等級上限、「HD 美術」開關）還沒在中文畫面上完整看過。

操作提示（原版遊戲的規則，不是錯誤）
- 飛行時只有底下是平地、而且沒碰到地圖上的人或物件才能降落；在海上、樹林、山上或岸邊選「降落」會被拒絕，選單會自己關掉、繼續往前飛。先按 B 停在平地上空再選「降落」最穩。
- 和地圖上的人對話要緊貼對方：往對方走到走不動為止，再選「對話」。停在前面差一點點，會顯示「那個方向沒有任何人。」。

安裝與啟動
- 安裝程式在 125%～200% 縮放與沒有新細明體的 Windows 上，還沒在實機看過；文字被切掉時請回報（附上螢幕截圖與縮放比例）。
- 上游抓不到美術包、改從副本下載的那一段，還沒在實際斷線的情況下跑過。
- 回報問題請附 `last-run.log`、F12 截圖與發生的位置（劇情段落或地圖）。

## 譯名基準

- 人名、招式以東立出版社的舊版單行本（1992–1995 年出版）譯名為準，少數譯名另行選定（例：飛里沙、基紐特戰隊、大長老、賽魯）。
- 遊戲原創、沒有台灣譯名的角色與招式，沿用同作 SFC 版繁中化的譯名。
- 用語以台灣慣用說法為準（例：選單、卡牌、金錢）。
- 香港版譯名：人名改用香港通行的叫法，由香港玩家協助校對；細節見「[香港版譯名](#香港版譯名)」。

## 贊助

如果你喜歡這個繁中版，想支持背後投入的時間與工具，可以請我喝杯咖啡：[Ko-fi](https://ko-fi.com/ming888)。完全自願：贊助支持的是這個專案的製作，不是遊戲本身，遊戲的權利屬於 BANDAI 等各權利人。所有內容免費提供，贊助不是下載或使用的條件，也不會解鎖任何東西；贊助只針對我的翻譯與修補，與上游作者的程式和美術無關。

在台灣不方便用 Ko-fi 的話，可以用銀行 App 的掃碼轉帳掃下面的 QR（LINE Bank，銀行代碼 824）；同樣完全自願，條件與上面相同。

<img src="docs/media/linebank-qr.png" alt="LINE Bank 轉帳 QR" width="220">

本專案是不收費、不販售的同好作品，不附 ROM，與 Bandai Namco 無關（完整聲明見「[授權與致謝](#授權與致謝)」最後一項「免責聲明」）。

## 授權與致謝

### 致謝

- 遊戲程式：本專案的遊戲本體改自 [JackRushante/DBZ-Super-Saiya-Densetsu-Rust](https://github.com/JackRushante/DBZ-Super-Saiya-Densetsu-Rust)（以下稱上游；`ssd-recompiled` © JackRushante/DBZ-Super-Saiya-Densetsu-Rust 及貢獻者，GPL-3.0）v0.3.15（`b4a67c95`），之後由我獨立維護、自行修正；與上游的完整差異就是本 repo 最新的那一個 commit，主要修正見 Release 附件 `PATCH_NOTES.txt`。Release 的遊戲本體與啟動器都由本專案原始碼編出，原始碼就在本 repo。安裝方法二用的原版程式 `dbz-ssd-recompiled-windows.zip` 請到上游的 [Releases](https://github.com/JackRushante/DBZ-Super-Saiya-Densetsu-Rust/releases) 頁下載。
- 引擎技術說明：[`docs/ENGINE.en.md`](docs/ENGINE.en.md)（Rust 重製本體的原始英文技術說明，原作者撰寫，保留原文）。
- HD 美術包：上游專案 JackRushante/DBZ-Super-Saiya-Densetsu-Rust（*Dragon Ball Z: Super Saiya Densetsu - SNES Decomp / Classic / Remastered, Rewritten in Rust*）的作品，授權 [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/)（上游 `LICENSE-ART.md`；本 repo 也附一份 [`LICENSE-ART.md`](LICENSE-ART.md)）。請到上游 [v0.3.9 Release](https://github.com/JackRushante/DBZ-Super-Saiya-Densetsu-Rust/releases/tag/v0.3.9) 下載（之後的版本到 v0.3.15 都沿用同一包，沒有另附新包）。美術包不隨本專案與 Release 附上，由玩家自行下載；上游下載不到時，啟動器改從作者依同一授權原樣保留的副本（[hd-pack-v0.3.9](https://github.com/lming888/DBZ-Super-Saiya-Densetsu-Rust/releases/tag/hd-pack-v0.3.9)）下載，檔案與上游完全相同。
- 本 README 的兩張截圖（`docs/media/zh-tw-dialogue.png`、`zh-tw-battle.png`）：加強模式掛上游 HD 美術包的實際遊戲畫面，沒有修改。畫面裡的美術出自上游專案 JackRushante/DBZ-Super-Saiya-Densetsu-Rust，依 [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/) 使用並以同一授權提供。
- 安裝程式裡的對照圖（「HD 高畫質美術包」頁；`installer/ui/hd/plain_*.bmp` 與 `art_*.bmp`）：同一格遊戲畫面的兩張實際截圖，各自裁切同一範圍、上下排列、加上「原版」「HD」標籤，其中 HD 那一張（`art_*.bmp`）的畫面含上游 HD 美術包的圖。這組對照圖隨安裝程式一起散布，依 [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/) 使用並以同一授權提供：美術出自上游專案 JackRushante/DBZ-Super-Saiya-Densetsu-Rust，本專案只做裁切與標籤，沒有改畫（製作腳本 `installer/ui/make_hd_compare.py`）。美術包本身不在安裝程式裡；安裝程式裡用到美術包內容的，除了這組對照圖，還有下一項的悟空圖與圖示，以及啟動器內嵌的美術覆蓋圖（見後面各項）。
- 安裝程式的側邊圖、頁首圖（`installer/wizard.bmp`、`header.bmp` 與 `installer/ui/1x\`、`2x\` 裡的同名圖）與程式圖示（`launcher/icon/app_icon.ico`）：人物取自本專案的超賽悟空開場飛行圖（下面「超賽悟空開場飛行圖」那一項，AI 依上游 HD 美術包普通悟空的飛行圖改畫），程式只做縮放、裁切與排版；背景的速度線為 AI 生成；標題字是霞鶩文楷 TC。依 [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/) 提供。
- 美術覆蓋圖（`save\art-override\`）：以下各項都內嵌在 Release 的 exe 裡提供，不在原始碼 repo 中（`launcher/assets/` 只在打包前複製進去，不進 git）；第一次執行時寫進 `save\art-override\`，遊戲用它取代美術包同名的圖（`docs/mods/art.md`「Replacing a picture of the HD pack」）。啟動器同時在 `save\art-override\` 寫入 `LICENSE-ART.txt`（出處、授權與改作者）；原作 HD 美術包的授權見本 repo 的 [`LICENSE-ART.md`](LICENSE-ART.md)。刪除 `save\art-override` 即可改回原圖，啟動器不會再寫回來。其中依官方插圖或原作像素重繪的圖（超賽悟空集氣全身圖、超級達爾 `key-*.png` 姿勢、比克蓄力姿勢）：原作圖像權利屬原權利人，非商業同人圖。
  - 超級達爾去墊肩美術：Ming 以 Google Gemini 圖像模型編輯自 HD 美術包原圖，CC BY-NC-SA 4.0。子資料夾 `super-vegeta\` 的圖用普通達爾的圖名，遊戲只在畫面是超級達爾配色時使用，普通達爾照舊；子資料夾 `goku\` 是普通悟空的黑髮圖（Ming 以 Google Gemini 圖像模型把美術包只有超賽版的集氣全身 `battle-goku-ssj-020` 與臉部特寫 `battle-goku-ssj-019` 改成黑髮），用超賽圖的圖名，遊戲只在悟空的髮色不是超賽金色時使用，超級賽亞人照舊。GAME OVER 字幕上飛過的超級達爾（`super-vegeta\gallery-fx-1118.png`）同樣由 Ming 以 Google Gemini 圖像模型編輯自美術包原圖（去墊肩、改超賽金髮與綠瞳），CC BY-NC-SA 4.0。
  - 超賽悟空集氣全身圖（`save\art-override\variants\battle-goku-ssj-020.png`，穿破藍內衣的正面圖）：AI 依官方插圖重畫。
  - 超賽悟空新頭（`save\art-override\variants\` 裡 73 張：`battle-goku-ssj-*` 53 張、`gallery-party-goku-ssj-*` 20 張，編號見啟動器寫出的 `LICENSE-ART.txt`）：Ming 以 Google Gemini 圖像模型編輯原本的同名覆蓋圖，換成同一顆新頭，再去背、縮放與對位。其中 `gallery-party-goku-ssj-170`（龜派氣功推出氣功波的背面圖）的雙手另以 Google Gemini 圖像模型重畫：上游美術包的原圖在這個姿勢多畫了一隻手。底圖改作自 HD 美術包原圖，CC BY-NC-SA 4.0。
  - 一般悟空龜派氣功背面圖（`save\art-override\variants\gallery-party-goku-u00-170.png` 與 `save\art-override\gallery-party-goku-u00-170.png`，黑髮悟空推出氣功波的背面圖）：雙手取自超賽版 `gallery-party-goku-ssj-170` 由 Ming 以 Google Gemini 圖像模型重畫的手，貼到 HD 美術包的原圖上。底圖為上游 HD 美術包原圖，CC BY-NC-SA 4.0，改作沿用同一授權，僅限非商業使用。
  - 超賽悟空開場飛行圖（`save\art-override\mode7\cd83cbee861e14ad\t0_0.png`，戰鬥開場超賽悟空飛入畫面的 Mode 7 平面）：Ming 以 Google Gemini 圖像模型依美術包普通悟空的 Mode 7 平面改畫，先換超級賽亞人的頭髮，再換成破衣；程式只做去背、縮放與對位。底圖改作自 HD 美術包原圖，CC BY-NC-SA 4.0。
  - 超級達爾龜派氣功受招、閃避與出招姿勢（`save\art-override\super-vegeta\key-*.png`）：AI 依原作像素圖與既有圖重繪。
    - 受招三姿勢（`key-14b06ca30f359b50.png`、`key-9b36beaeedd4fd2b.png`、`key-9fb61f64d7d15be9.png`）：悟空龜派氣功命中超級達爾時的背面、側身被推與翻滾。
    - 閃避與出招姿勢（其餘 16 張 `key-*.png`）：閃開龜派氣功的背面、肉搏閃避飛出畫面邊緣、氣圓斬擲出、強力能量波特寫滑入、由畫面下緣升起；畫面邊緣被裁的姿勢由同批重繪圖或同資料夾既有的圖裁出。
    - 同名的 `.pose` 只記姿勢的鍵值、大小、用到的色號與鏡像鍵值（不含原版像素），遊戲用它核對姿勢。
  - 比克蓄力姿勢（`save\art-override\gallery-party-piccolo-u01-061.png`）：依原作像素重繪。
  - 變身閃電（`save\art-override\ssj-lightning\`，悟空第一次變身超級賽亞人時疊在畫面上）：AI 生成的電弧素材，以程式擺位，未取自原作或美術包。刪掉 `ssj-lightning` 資料夾即關閉閃電。
- **snesrecomp** 與其他 SNES 靜態重編譯專案的作者，證明了這個做法可行；**Mesen2** 與 **Ghidra**，用於 `docs/re/` 筆記背後的研究。
- 七龍珠與 SNES 保存社群，他們對遊戲秘密與資料的筆記幫了忙。

### 授權

- 遊戲本體重製：[JackRushante/DBZ-Super-Saiya-Densetsu-Rust](https://github.com/JackRushante/DBZ-Super-Saiya-Densetsu-Rust)。本專案是上游加修補的衍生作品，程式碼依 **GNU General Public License v3.0**（[`LICENSE`](LICENSE)）散布；Release 中的遊戲本體與啟動器都由本專案原始碼編出（啟動器在 `launcher/`），對應的原始碼就是本 repo。第三方元件的聲明見 [`NOTICE.md`](NOTICE.md)。
- SNES 模擬器核心 jgenesis（jsgroth 與貢獻者，GPL-3.0），授權全文見 `third_party/jgenesis/LICENSE` 與 `translations/zh-tw/licenses/jgenesis-GPL-3.0.txt`。
- 字型：霞鶩文楷 TC（中文，LXGW WenKai TC 1.530，SIL Open Font License 1.1，`translations/zh-tw/fonts/OFL.txt`、`translations/zh-tw/licenses/LXGWWenKaiTC-OFL.txt`）；上游使用的 IBM Plex Sans、JetBrains Mono、DotGothic16、Share Tech Mono（SIL Open Font License 1.1，`assets/fonts/` 與 `translations/zh-tw/licenses/`）。
- 英文補丁：Klepto Software v1.02。上游的文字與分頁以這個英文版為準。
- 譯文表（`translations/zh-tw/*.tsv`、`translations/zh-hk/*.tsv`）：© Ming，CC BY-NC-SA 4.0。
- 變身語音（`voice\ssj-transform.wav`，內嵌在 Release 的 exe，第一次執行時裝進 `save\voice\`）：語音為 AI 生成，非原作錄音。
- 本軟體按現狀提供（as is），不附任何形式的擔保。
- 權利人：如認為本專案有侵害你權利之處，請在本 repo 開 issue，會盡快處理。
- **免責聲明**：《七龍珠Z 超賽亞傳說》原作由 BANDAI 發行，原著為鳥山明，動畫為東映動畫；遊戲本身的程式、圖像、音樂與文字屬於各權利人，不包含在本專案中。本專案是非官方的愛好者作品，**與 BANDAI（Bandai Namco）、Toei Animation、Shueisha、Bird Studio 等原著權利人無關，未獲其授權、背書或贊助**；與上游作者無關、未獲其背書，上游的程式與美術依其公開授權（GPL-3.0／CC BY-NC-SA 4.0）使用。*Dragon Ball*、*Dragon Ball Z* 及所有相關名稱、角色與美術，是各權利人的商標與作品。
