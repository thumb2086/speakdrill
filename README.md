# speakdrill

> 口說課程自動化練習工具：用 TTS 產生示範語音、逐句驅動課程演練、輸出可追蹤的分數報告。

> **speakdrill 僅支援 Windows（x64）**：用 VB-CABLE 或 VoiceMeeter 虛擬音效迴路把 TTS 變成麥克風輸入。
> **macOS / Linux 不支援**：VB-CABLE 及 per-app 麥克風綁定（Windows Audio 架構）無法在 macOS/Linux 等同運作。
> **npm 套件只提供 TUI（終端機介面）**；GUI 原生應用仍在 GitHub Releases 發佈但**不在 npm 安裝流程中**。

[![license](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

---

## 用途

口說課（例如學校配發的 MyET 課程）需要一句一句跟著念、拿分數，
一課十幾句、一冊三四十課，重複練起來很耗時間。

speakdrill 把這個流程自動化：

1. 讀出課程的句子清單
2. 用本機 TTS 逐句產生示範音訊
3. 透過 Chrome DevTools Protocol 操作課程頁面，在錄音開始時播放對應句子
4. 收集每句的分數，輸出 JSON 報告（總分／發音／語調／流利／重音）

適合：**大量複習、確認哪些句子還卡住、追蹤進步**。
不是要取代真的開口練習——分數報告看得出哪幾句該回頭練。

---

## 架構

```
TTS（Piper）播放 ──▶ CABLE Input（播放裝置）
                         │
                         ▼
              CABLE Output（麥克風錄音端）
                         │
                         ▼
                  課程 App 的 per-app 麥克風 ──▶ 評分引擎
```

（備選路線：VoiceMeeter — TTS ──▶ Voicemeeter Input ──▶ VAIO strip → B1 bus ──▶ Voicemeeter Out B1。
需要 strip → B1 路由設定；VB-CABLE 裝得上就用 VB-CABLE。）

**全域預設麥克風不會被改動**，只有指定的那支 App 會被指向虛擬迴路。

---

## 硬體需求

**不高。** speakdrill 本身只是單一 Rust 執行檔，真正吃資源的是 MyET 課程 App 本身（CEF，常駐多個程序）。

| 項目 | 最低建議 | 實測機器（開發／測試） |
|---|---|---|
| OS | Windows 10 / 11（64-bit） | Windows 11 26H2 |
| CPU | 雙核心即可 | i9-13900K（24C/32T） |
| RAM | 4 GB（speakdrill 本體 < 50 MB；MyET 多程序合計約 0.7 GB） | 32 GB |
| GPU | 不需要（Piper TTS 與音訊全走 CPU） | — |
| 磁碟 | 約 100 MB（程式 + Piper 聲音檔 ~82 MB）＋ 少量 TTS 快取 | — |

重點：

- speakdrill 本體記憶體用量在 **20 MB 級**（TUI 實測約 21 MB），幾乎可以忽略
- MyET10（CEF）實測 7 個程序合計約 **720 MB**——能順跑 MyET，就能順跑 speakdrill
- 本機 Piper TTS 用 CPU 合成，**不需要 GPU、不需要 Python**
- 需要一個虛擬音效驅動：VB-CABLE（1.3 MB，零設定）或 VoiceMeeter（~30 MB）；安裝後**必須重開機**
- 安裝驅動與 per-app 麥克風綁定需要管理員權限（會跳 UAC）
- 需要 MyET10 已安裝（預設 `C:\Program Files (x86)\LLabs\MyET10\MyET10.exe`），並以 `--remote-debugging-port=9222 --remote-allow-origins=*` 啟動（TUI 會自動處理）

---

## 安裝

### npm

```bash
npm i -g speakdrill
speakdrill init
```

`postinstall` 會從 GitHub Releases 下載對應平台的原生二進位。

### 從原始碼

```bash
git clone https://github.com/thumb2086/speakdrill
cd speakdrill
cargo build --release
./target/release/speakdrill init
```

需要 Rust 1.75+。**TTS 不需要 Python**：`speakdrill init` 會自動下載 Piper + 英文聲音檔（約 82 MB）。

---

## 新手上路（10 分鐘）

目標：裝完後直接 `speakdrill`，機器自己講完一課並收到分數。

**0. 前置**：MyET10 已安裝並能登入看到「我的課程」。若還沒裝虛擬音效驅動，先跑 `speakdrill init --install-driver`（跳一次 UAC），**裝完重開機**再繼續。

**1. 安裝 + 初始化**

```bash
npm i -g speakdrill
speakdrill init
```

`init` 依序做完（你不用動手）：偵測輸出裝置 → 下載 Piper TTS → 偵測虛擬迴路（VB-CABLE 或 VoiceMeeter）→ 播放端與**錄音端**音量拉到 100% → 寫設定 → 自動綁 per-app 麥克風（registry → COM → UIA）。
最後一步看到 `✓ … 已綁定` 就是完成。

**2. 檢查**

```bash
speakdrill doctor
```

每一項都應顯示 OK。有 ✗ 先看最下方「疑難排解」。

**3. 第一次跑**

```bash
speakdrill
```

你會看到（範例，載入過程會先顯示預設那本、其他課本背景陸續加入）：

```text
┌speakdrill 口說自動練習────────────┐
│ speakdrill  v0.2.33   204 課   完成 0 句  平均 0.0   MyET 累計 15.6 小時
└──────────────────────────────────┘
│ spawn_catalog: fetch_catalog_opts 回傳 39 課
│ 目錄載入：39 課
```

- 左側是課本，括號內是**官方**完成率／成績（來自 MyET，不是自算）
- `↑`/`↓` 移動、`Space` 勾選、`Enter` 執行（自動略過已完成的課）
- `r` 重新載入目錄、`m` 啟動 MyET、`s` 請求中止、`q` 離開

**4. 跑之後**

- 每課完成會寫報告：`%APPDATA%\speakdrill\reports\<LessonID>.json`
- 成績會送回 MyET（完成率會更新為官方數字）
- 想加課本：批次執行中按 `Space`，本批結束後自動接續跑（標題顯示「追加 N 本」）

---

## 設定

```bash
speakdrill init                  # 偵測裝置 + 寫設定 + 自動偵測 VB-CABLE／VoiceMeeter 迴路 + 拉雙端音量 + 自動綁 per-app 麥克風
speakdrill doctor                # 檢查 port / 登入 / 裝置 / VoiceMeeter / TTS / 麥克風綁定
speakdrill init --device "Voicemeeter Input" --server https://140.125.168.213 --code BKM_1A
```

`init` 會自動處理：

| 項目 | 說明 |
|---|---|
| VoiceMeeter 路由 | 只有 TTS 那條 strip 送到 B1，其他關掉（VoiceMeeter 路線） |
| 播放裝置音量 | 拉到 100% |
| **錄音裝置音量**（CABLE Output／Voicemeeter Out B1） | 拉到 100%（B1 預設 24.3% 會讓評分引擎判「太小聲」，v0.2.33 起 VB-CABLE 也一起拉） |
| TTS 引擎 | 自動下載 Piper（單一 exe，不需要 Python） |
| **per-app 麥克風綁定** | 自動跑 `speakdrill setup`（registry → COM → UIA），無需手動；失敗時才印手動步驟 |

### 虛擬音效迴路

程式需要一個「虛擬音效迴路」，把 TTS 播出去的聲音變成麥克風輸入。
Windows **沒有** user-mode 的方法可以建立麥克風，所以一定得裝一個虛擬音效驅動。

`speakdrill` 支援兩種，`init` 會自動偵測：

| | VB-CABLE（推薦） | VoiceMeeter |
|---|---|---|
| 大小 | 1.3 MB | ~30 MB |
| 設定 | **零設定**（播到 `CABLE Input`，`CABLE Output` 就有聲音） | 要設 strip → B1 路由 |
| 安裝 | `speakdrill init --install-driver`（跳一次 UAC） | 自行安裝 |

```bash
speakdrill init --install-driver     # 沒偵測到裝置時，自動下載並安裝 VB-CABLE
```

> ⚠ **虛擬音效驅動安裝後必須重開機。**
>
> VB-CABLE / VoiceMeeter 這類核心驅動不會在安裝瞬間完整生效。
> 常見現象是：「裝置看得到，但 cpal 打不開」、或 App 錄音靜音。
> 安裝後請直接重開機，再執行 `speakdrill doctor`。
> `doctor` 會檢查「迴路能不能真的開、App 的 per-app 麥克風綁定正不正確」。

（Windows 26H2 + VB-Audio 驅動特別需要重開機；`--install-driver` 會跳一次 UAC，請按「是」。）


### 自動設定 per-app 麥克風（setup）

Windows 不讓 App 用介面改自己的裝置（登錄值裡有一組 Windows 產生的綁定 GUID，自己手寫無效），
而且只有 App 有 audio session 時 mixer 才會列出它。`speakdrill setup` 用三層策略解決
（**`speakdrill init` 已經會自動呼叫**，通常不需要手動跑）：

1. **讀 registry** 確認 App 的 per-app 麥克風已綁定 → 已綁定就直接結束（什麼都不動）。
2. **COM API** `SetPersistedDefaultAudioEndpoint` 直接寫入 → 成功就 registry 驗證並結束。
3. **UI Automation**：自動開啟「設定 → 系統 → 聲音 → 音量混合器」，
   用「進程 PID → EntityItem」精確定位到目標 App 那一列，展開後選定裝置，
   再讀回驗證；只操作該列、不用 SendKeys、不用座標點擊；
   Settings 沒回應時直接提示，不硬卡 45 秒。

```bash
speakdrill setup                     # 自動設定麥克風（registry → COM → UIA）
speakdrill setup --set-output "..."  # 同時設定輸出裝置（例如 Voicemeeter AUX Input）
speakdrill setup --dry-run           # 只預覽會做什麼，不實際更改
speakdrill setup --no-uia            # 不碰設定視窗（COM 失敗就印手動步驟）
speakdrill setup --force             # 即使已綁定也重設
speakdrill setup --restart-settings  # 「設定」視窗卡死時，先關閉再重開並自動設定
```

GUI 原生應用已調整為**不在 npm 套件內**；若要使用 GUI，請至 GitHub Releases 下載 `speakdrill-gui.exe`。
終端機介面（TUI）：`Space` 勾/取消課本（黃色 ●）、`i` 顯示課本的課程列表（單元名 + 課號，一眼對照）、`Enter` 執行（有佇列跑佇列，沒有就跑選取；已完成的課自動略過）、`a` 全選/取消、`x` 清空佇列、`b` 或 `B` 跑全部、`r` 重新載入目錄、`m` 自動啟動 MyET、`s` 請求中止、`PgUp`/`PgDn` 捲動紀錄、`Home` 跳到最早、`End` 回到最新、`q`/`Esc`/`Ctrl+C` 離開。

**跑到一半想加課本**：批次執行中按 `Space` 勾選新課本，本批課程跑完後會留在佇列等你按 `Enter` 開始（左側面板標題顯示「追加 N 本」與面板上課本顯示 [1] [2] 的順序）；按 `s` 中止時附加的課本不會自動開始，可按 `Enter` 手動開始，或 `x` 清空。
注意：**目標 App 必須啟動過且用過麥克風**（Windows mixer 只列有 audio session 的 App）。
手動 fallback：`ms-settings:apps-volume` → 找到目標 App（MyET 會顯示為 **English-Chinese-Japanese**）→ 展開 → 輸入裝置 → `CABLE Output (VB-Audio Virtual Cable)`。

### MyET 的 debug port（已自動化）

TUI 啟動時會自動檢查 port 9222，沒連上就自動啟動 MyET（帶 `--remote-debugging-port=9222 --remote-allow-origins=*`）。也可以在 TUI 裡按 `m` 手動啟動，或用下面指令手動開：

```powershell
& "C:\Program Files (x86)\LLabs\MyET10\MyET10.exe" `
    --remote-debugging-port=9222 --remote-allow-origins=*
```

---

## 使用

```bash
speakdrill                      # 終端機介面（預設；課程列表 + 即時分數）

speakdrill list                 # 列出目錄課程（名稱 + 代碼 + 已練分數）
speakdrill list --refresh       # 忽略快取重抓（目錄快取 10 分鐘；抓到的 0 個有成績時不會覆蓋舊快取）

speakdrill run  --lesson BKM-GAR-1A027 --course BKM-B1A-001
                                 # 不用加 --code：自動從目錄定位這一課所屬的 CodeName
speakdrill batch --code BKM_1A --skip-completed   # 跳過已練過的課
speakdrill batch --code BKM_1A --limit 5
speakdrill batch --code BKM_1A --start 10 --limit 3

speakdrill reset                  # 停止把 TTS 送到 B1，並提示如何還原 per-app 設定
speakdrill reset --yes            # 連設定檔一起刪

speakdrill update                 # 檢查 npm 上的最新版，顯示更新指令（npm 不自動跑）
speakdrill self-update            # 以 npm 自動升級到最新版（背景執行，幾秒後請重開）
```

輸出：

- 報告：`%APPDATA%\speakdrill\reports\<LessonID>.json`
- TTS 快取：`%LOCALAPPDATA%\speakdrill\audio\<LessonID>\`
- 課程完成率與成績：來自 MyET 官方目錄（`CategoryLessonScores.aspx`，含 best_score），不是本工具自己算的
- 累計時數：標題列顯示 **MyET 官方累計**（Dashboard「學習時間」，每次啟動讀取）；
  讀取完成前顯示「累計 …」，讀取失敗才退回本機累計（只算本工具跑的課次）
- 取得 MyET 官方狀態需要課程 App 已登入；重開機後首次啟動時工具會自動等頁面載入完成再抓

報告範例（`BKM-GAR-2B001.json`，節錄）：

```json
{
  "lesson_id": "BKM-GAR-2B001",
  "total_lines": 16,
  "title": "Unit 5.1 - Invitation 1",
  "lines": [
    { "index": 1, "text": "Hello?", "total": 2, "pronunciation": 1,
      "pitch": 4, "fluency": 4, "stress": 4, "attempts": 1 }
  ],
  "avg": 91.2,
  "elapsed_secs": 812
}
```

（各句滿分 100，分項為發音/語調/流利/重音；`attempts` 是重試次數。）

---

## 實測

| 課程 | 結果 |
|---|---|
| BKM-GAR-1A027（17 句） | 17/17，平均 88.7 |
| BKM-GAR-1A028（13 句） | 11/11（其餘為既有進度跳過），平均 89.1 |

---

## 常見問題（FAQ）

**左側為什麼只有走天下 6 本？**
伺服器只回了這 6 個 CodeName 給 GEN 類型；其餘（MBA／TOEIC／MICE 等）抓不回來就不會出現在清單。
抓到 0 個有成績時不會覆蓋既有快取（v0.2.30 起）。

**2B 的課號為什麼長得像 GAR？**
走天下系列課程的 lesson_id 都以 `BKM-GAR-…` 開頭（內容商 GAR = Getting Around in American English）。
課本分組看 course_id 前綴（`BKM-B2B-001` → 書林美語走天下-2B）。
TUI 選中後按 `i`（GUI 按「📋 課本內容」）可直接預覽該課本裡有哪些課。

**完成率／成績是哪來的？**
MyET 官方目錄（`CategoryLessonScores.aspx`，含 best_score），不是本工具自己算的。

**累計時數為什麼一開始顯示「累計 …」？**
MyET 官方值是 JS 非同步填進畫面的，等它填完（通常幾秒）才換成「MyET 累計 X 小時」；
讀取失敗才退回本機累計（只算本工具跑的課次，會是 0.0）。

**選了 B2B，頁面卻顯示「2A」（第一句 2 分）？**
v0.2.29 已修：跑課網址改用課程抓取時的 CodeName。升級 `npm i -g speakdrill@latest`。

**第一句「Hello?」2 分？**
超短句容易抓到環境音或沒量到；後續句子通常正常（87～99 分），短句可直接忽略。

**MyET 視窗比例變形／超寬？**
是 MyET（CEF）視窗大小被調壞了，不是本工具造成的。關掉 MyET 重開，或直接最大化即可。

**「per-app 麥克風」是什麼？**
Windows 的每應用程式輸入裝置。只讓 MyET 收到我們播的 TTS，不會動到你其他程式的全域麥克風。

**跑課會寫入我的成績單嗎？**
會 — MyET 伺服器會記錄成績並更新完成率（這也是左側「100%」的來源）。

---

## 疑難排解

| 症狀 | 可能原因 | 解法 |
|---|---|---|
| `doctor` ✗ CDP / 登入 | MyET 沒開，或 debug port 沒帶 | 按 `m`（或 GUI「🚀 啟動 MyET」），或重開 `speakdrill` |
| 分數全 ✗／MyET 說「太小聲」 | 錄音端（CABLE Output／Voicemeeter Out B1）音量低 | 重跑 `speakdrill init`（v0.2.33 起會拉滿錄音端） |
| 全部 0%、官方「累計 0.0」 | MyET 剛開機時抓到相容模式的資料 | `speakdrill list --refresh`（v0.2.30 起再現不會覆蓋舊快取） |
| 目錄載入「抓不到課程」 | MyET 未登入 | 先登入 MyET（能看到「我的課程」）再 `speakdrill` |
| npm 安裝 `EBUSY`／`EPERM` | 舊 exe 被鎖住（TUI／MyET 在跑） | 關閉後再 `npm i -g speakdrill@latest` |
| `doctor` ✗ 迴路打不開／錄音靜音 | 驅動未生效 | 重開機後再 `speakdrill doctor` |
| `speakdrill` 一直卡「連不上 MyET」 | MyET 沒啟動 | 先開 MyET，或按 `m` 啟動 |
| 追加的課本按 `s` 沒開始跑 | 中止模式不自動開始 | 按 `Enter` 手動開始，或 `x` 清空 |

---

## 實作筆記

### 為什麼要自己寫 CDP client

只用得到 `Runtime.evaluate` 與 `Runtime.consoleAPICalled`，
用 `chromiumoxide` 反而要順著它的瀏覽器啟動假設。這裡自己包一層約 300 行。

### 為什麼音訊要常駐

評分 App 的錄音視窗只有約 **2.7 秒**。每次啟動外部播放器都會吃掉 0.5~1 秒，
所以 `cpal` 的 stream 一開始就開好，播放只是把樣本塞進環形緩衝。
另外 cpal 的 `Stream` 是 `!Send`，裝置列舉與 stream 建立都必須在音訊執行緒內完成。

### 踩過的坑

| 做法 | 結果 |
|---|---|
| 直接送控制指令給 App 的原生 HTTP 端點 | App `c0000005` 崩潰，必須走頁面層 |
| 手寫 `DefaultEndpoint` 登錄的 `001_*` | 無效，要用 Windows 介面 |
| `IAudioPolicyConfigFactory` P/Invoke | AV / ArgumentException，vtable 未公開，別猜 |
| 從 `console.log` 讀分數 | 讀不到，頁面沒印 xml |
| hook 頁面的 `adjustScores` | 在閉包內，不是 global |

### 移植時自己弄壞的兩件事（教訓）

1. **`0` 是合法分數**，不能當成「DOM 還沒更新」而丟棄——否則分數表永遠空的、迴圈空轉。
2. **重試次數不能綁在「讀到分數」上**——讀不到就不會增加，重試按鈕會被無限點。

另外 TTS 前處理一定要做 `"..." → ", "`：`Let's see... food` 會從 2.19 秒變 3.19 秒，
超過 2.7 秒的錄音視窗就會被截斷成 0 分。

---

## 專案結構

```
crates/
  speakdrill-core/   CDP client、課程解析、音訊（cpal）、VoiceMeeter、TTS、per-app 音訊設定（COM + UIA）、流程
  speakdrill-cli/    speakdrill（init / doctor / reset / run / batch / all / list / tui / setup）
  speakdrill-gui/    speakdrill-gui（egui，原生，無 webview）
npm/                 npm 包裝器（下載 GitHub Releases 的原生二進位）
scripts/             audio-uia.ps1（UIA 助手，隨 binary 發布）、kokoro_tts.py（開發用）
```

## 授權

MIT
