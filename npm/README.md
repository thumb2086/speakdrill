# speakdrill

口說課程自動化練習：用 TTS 產生示範語音、逐句驅動課程演練、輸出可追蹤的分數報告。

Rust 實作，含 CLI、TUI 與原生 GUI（egui，不是 Electron / Tauri webview）。

> 目前只支援 **Windows x64**。可選用 **VB-CABLE**（零設定、自動安裝）或 **VoiceMeeter**（Banana/Potato）。

## 硬體需求

**不高。** speakdrill 本體只是單一 Rust 執行檔（記憶體約 20 MB 級），真正吃資源的是 MyET 課程 App 本身（CEF，多程序合計約 0.7 GB）。能順跑 MyET，就能順跑 speakdrill。

- OS：Windows 10 / 11（64-bit）
- CPU：雙核心即可（Piper TTS 與音訊全走 CPU，不需要 GPU）
- RAM：4 GB 以上
- 磁碟：約 100 MB（程式 + Piper 聲音檔 ~82 MB）＋ 少量 TTS 快取
- 需要虛擬音效驅動（VB-CABLE 1.3 MB 或 VoiceMeeter ~30 MB），安裝後**必須重開機**
- 安裝驅動與 per-app 麥克風綁定需要管理員權限（UAC）
- 需要 MyET10 已安裝，TUI 會以 `--remote-debugging-port=9222 --remote-allow-origins=*` 自動啟動

## 安裝

```bash
npm i -g speakdrill
speakdrill init
speakdrill doctor
```

安裝時會從 GitHub Releases 下載原生二進位（`speakdrill.exe` / `speakdrill-gui.exe`）。

## 使用時你會看到

- **完成率 / 本課本成績**：從 MyET 官方目錄抓取（含 best_score），不是本工具自算
- **累計時數**：TUI / GUI 標題列顯示 MyET 官方累計（Dashboard「學習時間」）；讀取前顯示「累計 …」，讀取失敗才退回本機累計
- **`speakdrill run --lesson <ID> --course <ID>`**：不用指定 CodeName，會自動從目錄定位課程所屬的那本
- **跑到一半想加課本**：批次執行中按 `Space` 勾選，本批課程跑完會自動接續跑（TUI 標題顯示「追加 N 本」）；GUI 相同（執行中勾選即追加）
- **目錄快取**：10 分鐘內的目錄直接從快取讀；抓到 0 個有成績時不會覆蓋舊快取



## 重開機需求

> 安裝／變更虛擬音效驅動後，**請重新開機**再跑 `speakdrill init`。
> 只重啟 App 不夠。`doctor` 會檢查：
> - 迴路能不能實際開啟
> - App 的每應用程式麥克風綁定是否一致
> - 是否選到真正可用的迴路

## 快速開始

```bash
speakdrill init                  # 偵測裝置、寫設定、自動安裝 VB-CABLE 或設 VoiceMeeter 路由、拉音量、自動綁 per-app 麥克風（registry → COM → UIA）
speakdrill setup                 # 手動重跑/修復 per-app 麥克風綁定（init 已自動呼叫過；失敗時才需手動）
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
4. **per-app 麥克風綁定**：`speakdrill init` 會自動完成（registry → COM → UIA；原本需手動在 `ms-settings:apps-volume` 設定）。自動失敗時可跑 `speakdrill setup` 重試；VB-CABLE 綁定 `CABLE Output (VB-Audio Virtual Cable)`，VoiceMeeter 綁定 `Voicemeeter Out B1`

## 文件

完整說明、架構與踩坑紀錄：<https://github.com/thumb2086/speakdrill>

## 授權

MIT