# speakdrill

口說課程自動化練習：用 TTS 產生示範語音、逐句驅動課程演練、輸出可追蹤的分數報告。

Rust 實作，含 CLI、TUI 與原生 GUI（egui，不是 Electron / Tauri webview）。

> 目前只支援 **Windows x64**（相依 VoiceMeeter）。

## 安裝

```bash
npm i -g speakdrill
speakdrill init
speakdrill doctor
```

安裝時會從 GitHub Releases 下載原生二進位（`speakdrill.exe` / `speakdrill-gui.exe`）。

## 快速開始

```bash
speakdrill init                  # 偵測裝置、寫設定、設 VoiceMeeter 路由、拉音量
speakdrill doctor                # 檢查環境
speakdrill tui                   # 終端機介面
speakdrill-gui                   # 原生 GUI
speakdrill run  --lesson <ID> --course <ID>
speakdrill batch --code <CODE> --limit 5
speakdrill reset                 # 還原音訊路由
```

## 前置需求

1. **VoiceMeeter**（Banana 或 Potato）
2. **一個 TTS**：`pip install kokoro-onnx soundfile`，或自備 [piper](https://github.com/rhasspy/piper)
3. 目標 App 需開啟 debug port：
   `--remote-debugging-port=9222 --remote-allow-origins=*`
4. 在 `ms-settings:apps-volume` 把目標 App 的**輸入裝置**設成 `Voicemeeter Out B1`
   （這步只能用介面做，Windows 會產生綁定 GUID）

## 文件

完整說明、架構與踩坑紀錄：<https://github.com/thumb2086/speakdrill>

## 授權

MIT
