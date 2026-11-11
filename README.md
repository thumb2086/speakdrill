# speakdrill

> 口說課程自動化練習工具：用 TTS 產生示範語音、逐句驅動課程演練、輸出可追蹤的分數報告。
> **Rust** 實作，同時提供 CLI、TUI 與原生 GUI（egui，**不是** Electron / Tauri webview）。

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
TTS（Kokoro / Piper）──▶ Voicemeeter Input (VAIO strip)
                              │  strip[VAIO].B1 = 1
                              ▼
                          B1 bus ──▶ 「Voicemeeter Out B1」錄音裝置
                                            │
                            課程 App 的 per-app 輸入裝置 ──▶ 評分引擎
```

**全域預設麥克風不會被改動**，只有指定的那支 App 會被指向 B1。

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

## 設定

```bash
speakdrill init                  # 偵測裝置 + 寫設定 + 設 VoiceMeeter 路由 + 拉音量
speakdrill doctor                # 檢查 port / 登入 / 裝置 / VoiceMeeter / TTS
speakdrill init --device "Voicemeeter Input" --server https://140.125.168.213 --code BKM_1A
```

`init` 會自動處理：

| 項目 | 說明 |
|---|---|
| VoiceMeeter 路由 | 只有 TTS 那條 strip 送到 B1，其他關掉 |
| **B1 裝置音量** | 預設竟然是 **24.3%**，會讓評分引擎判「太小聲」。自動拉到 100% |
| 播放裝置音量 | 同樣拉到 100% |
| TTS 引擎 | 自動下載 Piper（單一 exe，不需要 Python） |

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

> 安裝驅動需要管理員權限，過程中會跳一次 UAC，請按「是」。

### 唯一需要手動的一步

Windows 只允許「用介面」設定 per-app 音訊裝置（登錄值裡有一組 Windows 產生的
綁定 GUID，自己手寫無效）：

1. `Win + R` → `ms-settings:apps-volume`
2. 找到目標 App（MyET 會顯示為 **English-Chinese-Japanese**）→ 展開
3. **輸入裝置** → `Voicemeeter Out B1 (VB-Audio Voicemeeter VAIO)`

### 目標 App 要開 debug port

```powershell
& "C:\Program Files (x86)\LLabs\MyET10\MyET10.exe" `
    --remote-debugging-port=9222 --remote-allow-origins=*
```

---

## 使用

```bash
speakdrill                      # 終端機介面（預設；課程列表 + 即時分數）
speakdrill-gui                  # 原生 GUI

speakdrill list                 # 列出目錄課程（名稱 + 代碼 + 已練分數）
speakdrill list --refresh       # 忽略快取重抓（目錄快取 10 分鐘）

speakdrill run  --lesson BKM-GAR-1A027 --course BKM-B1A-001
speakdrill batch --code BKM_1A --skip-completed   # 跳過已練過的課
speakdrill batch --code BKM_1A --limit 5
speakdrill batch --code BKM_1A --start 10 --limit 3

speakdrill reset                  # 停止把 TTS 送到 B1，並提示如何還原 per-app 設定
speakdrill reset --yes            # 連設定檔一起刪
```

輸出：

- 報告：`%APPDATA%\speakdrill\reports\<LessonID>.json`
- TTS 快取：`%LOCALAPPDATA%\speakdrill\audio\<LessonID>\`

---

## 實測

| 課程 | 結果 |
|---|---|
| BKM-GAR-1A027（17 句） | 17/17，平均 88.7 |
| BKM-GAR-1A028（13 句） | 11/11（其餘為既有進度跳過），平均 89.1 |

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
  speakdrill-core/   CDP client、課程解析、音訊（cpal）、VoiceMeeter、TTS、流程
  speakdrill-cli/    speakdrill（init / doctor / reset / run / batch / tui）
  speakdrill-gui/    speakdrill-gui（egui，原生，無 webview）
npm/                 npm 包裝器（下載 GitHub Releases 的原生二進位）
scripts/             kokoro_tts.py（命令式 TTS 轉接器）
```

## 授權

MIT
