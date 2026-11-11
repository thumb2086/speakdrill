# myet-practice

> MyET 口說自動練習：用 TTS 餵麥克風，讓 MyET 自己評分。
> **Rust 實作**，同時提供 CLI、TUI 與原生 GUI（egui，**不是** Electron / Tauri webview）。

[![release](https://img.shields.io/github/v/release/yourname/myet-practice)](https://github.com/yourname/myet-practice/releases)
[![license](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

---

## 這是什麼

MyET 是 CEF（Chromium）包裝的網頁 App，錄音由原生層（`ASAS3.dll`）執行。
本工具：

1. 用 Chrome DevTools Protocol 操作 MyET 的練習頁
2. 每一句在 MyET 送出 `STARTREC` 的瞬間，把該句 TTS 播進 VoiceMeeter
3. VoiceMeeter 把聲音送到 B1 bus → 這就是 MyET 的（per-app）麥克風
4. 讀 `#total_score` 取分數，低分自動重試，超過次數跳過

```
Kokoro / Piper TTS ──▶ Voicemeeter Input (VAIO strip)
                              │  strip[VAIO].B1 = 1
                              ▼
                          B1 bus ──▶ 「Voicemeeter Out B1」錄音裝置
                                            │
                            MyET 的 per-app 輸入裝置 ──▶ 評分
```

**全域預設麥克風不會被改動**，只有 MyET 這支 App 會被指向 B1。

---

## 安裝

### npm（推薦）

```bash
npm i -g myet-practice
myet init
```

`postinstall` 會從 GitHub Releases 下載對應平台的原生二進位。

### 從原始碼

```bash
git clone https://github.com/yourname/myet-practice
cd myet-practice
cargo build --release
./target/release/myet init
```

需要 Rust 1.75+。TTS 另外需要 `pip install kokoro-onnx soundfile` 或自備 piper。

---

## 設定（`myet init` 會做掉大部分）

```bash
myet init                       # 偵測裝置 + 寫設定 + 設 VoiceMeeter 路由 + 拉音量
myet init --device "Voicemeeter Input" --server https://140.125.168.213 --code BKM_1A
myet doctor                     # 檢查 port / 登入 / 裝置 / VoiceMeeter / TTS
```

`init` 自動處理：

| 項目 | 說明 |
|---|---|
| VoiceMeeter 路由 | 只有 TTS 那條 strip 送到 B1，其他關掉 |
| **B1 裝置音量** | 預設竟然是 **24.3%**，會讓 MyET 判「太小聲」。自動拉到 100% |
| 播放裝置音量 | 同樣拉到 100% |
| TTS 引擎 | 有 piper 用 piper，否則用內附的 Kokoro 轉接器 |

### 唯一需要手動的一步

Windows 只允許「用介面」設定 per-app 音訊裝置（登錄值裡有一組 Windows
產生的綁定 GUID，自己手寫無效）：

1. `Win + R` → `ms-settings:apps-volume`
2. 找到 **English-Chinese-Japanese**（就是 MyET）→ 展開
3. **輸入裝置** → `Voicemeeter Out B1 (VB-Audio Voicemeeter VAIO)`

> `myet reset` 會告訴你怎麼還原，並停止把 TTS 送到 B1。

### MyET 要開 debug port

```powershell
& "C:\Program Files (x86)\LLabs\MyET10\MyET10.exe" `
    --remote-debugging-port=9222 --remote-allow-origins=*
```

---

## 使用

```bash
# 終端機介面（列表 + 即時分數）
myet tui

# 原生 GUI（egui）
myet-gui

# 單課
myet run --lesson BKM-GAR-1A027 --course BKM-B1A-001

# 批次：整個 CodeName 的所有課程
myet batch --code BKM_1A --limit 5
myet batch --code BKM_1A --start 10 --limit 3

# 還原
myet reset
myet reset --yes     # 連設定檔一起刪
```

分數報告：`%APPDATA%\myet-practice\reports\<LessonID>.json`
TTS 快取：`%LOCALAPPDATA%\myet-practice\audio\<LessonID>\`

---

## 實測結果

| 版本 | 課程 | 結果 |
|---|---|---|
| Node 原型 | BKM-GAR-1A027（17 句） | **17/17，平均 88.7** |
| Rust 0.1 | BKM-GAR-1A027 | **16/16，平均 89.4**，173 秒 |

---

## 踩過的坑（別再試）

| 做法 | 結果 |
|---|---|
| 直接 POST `STARTREC` 給 `localhost:50175` | **MyET10.exe `c0000005` 崩潰** |
| 手寫 `DefaultEndpoint` 登錄的 `001_*` | 無效，要用 Windows 介面設定 |
| `IAudioPolicyConfigFactory` P/Invoke | AV / ArgumentException，vtable 未公開，別猜 |
| 從 `console.log` 讀分數 | 讀不到，`Scoring.js` 沒印 xml |
| hook `window.adjustScores` | 它在閉包內，不是 global |
| 把全域預設麥克風改成 B1 | 會影響其他程式（本專案禁止） |
| `edge-tts` 雲端語音 | 403，微軟擋了 |

### 移植到 Rust 時自己弄壞的兩件事（教訓）

1. **`0` 是合法分數**，不能當成「DOM 還沒更新」而丟棄——否則分數表永遠空的、迴圈空轉。
2. **重試次數不能綁在「讀到分數」上**——讀不到就不會增加，`再試一次` 會被無限點。

另外 TTS 前處理一定要做 `"..." → ", "`：`Let's see... food` 會從 2.19 秒變 3.19 秒，
超過 MyET 只有 **~2.7 秒**的錄音視窗就會被截斷成 0 分。

---

## 架構

```
crates/
  myet-core/     CDP client、課程解析、音訊（cpal）、VoiceMeeter、TTS、流程
  myet-cli/      myet（init/doctor/reset/run/batch/tui）
  myet-gui/      myet-gui（egui，原生，無 webview）
npm/             npm 包裝器（下載 GitHub Releases 的原生二進位）
scripts/         kokoro_tts.py（命令式 TTS 轉接器）
```

## 授權

MIT
