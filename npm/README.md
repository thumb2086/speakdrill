# speakdrill

口說課程自動化練習：用 TTS 產生示範語音、逐句驅動課程演練、輸出可追蹤的分數報告。

Rust 實作，含 CLI、TUI 與原生 GUI（egui，不是 Electron / Tauri webview）。

> 目前只支援 **Windows x64**。可選用 **VB-CABLE**（零設定、自動安裝）或 **VoiceMeeter**（Banana/Potato）。

## 安裝

```bash
npm i -g speakdrill
speakdrill init
speakdrill doctor
```

安裝時會從 GitHub Releases 下載原生二進位（`speakdrill.exe` / `speakdrill-gui.exe`）。


## 重開機需求

> 安裝／變更虛擬音效驅動後，**請重新開機**再跑 `speakdrill init`。
> 只重啟 App 不夠。`doctor` 會檢查：
> - 迴路能不能實際開啟
> - App 的每應用程式麥克風綁定是否一致
> - 是否選到真正可用的迴路

## 快速開始

```bash
speakdrill init                  # 偵測裝置、寫設定、自動安裝 VB-CABLE 或設 VoiceMeeter 路由、拉音量
speakdrill setup                 # 自動完成 per-app 麥克風綁定（原本需手動在 ms-settings:apps-volume 設定）
speakdrill doctor                # 檢查環境
speakdrill                       # 終端機介面（預設）
speakdrill list                  # 列出課程
speakdrill-gui                   # 原生 GUI
speakdrill run  --lesson <ID> --course <ID>
speakdrill batch --code <CODE> --limit 5
speakdrill reset                 # 還原音訊路由
```

## 前置需求

1. **VB-CABLE**（推薦，零設定）或 **VoiceMeeter**（Banana / Potato）
   - `speakdrill init --install-driver` 會自動下載並安裝 VB-CABLE（需管理員權限）
   - 已安裝 VoiceMeeter 者可直接使用，`init` 會偵測並設定路由
2. **不需要額外裝 TTS**：`speakdrill init` 自動下載 Piper
3. **MyET 的 debug port**：TUI 啟動時會自動檢查 port 9222，沒連上就自動啟動 MyET（帶 `--remote-debugging-port=9222 --remote-allow-origins=*`）；也可以在 TUI 裡按 `m` 手動啟動
4. **per-app 麥克風綁定**：`speakdrill setup` 會自動完成（原本需手動在 `ms-settings:apps-volume` 設定），VB-CABLE 時會綁定 `CABLE Output (VB-Audio Virtual Cable)`，VoiceMeeter 時綁定 `Voicemeeter Out B1`

## 文件

完整說明、架構與踩坑紀錄：<https://github.com/thumb2086/speakdrill>

## 授權

MIT