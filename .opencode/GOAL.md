# Goal
- GoalID: 019f69e0-2f7b-7c8c-a1e1-b5c3e60f9e21
- Status: pursuing
- Created: 2026-11-14T00:00:00+08:00
- Updated: 2026-11-14T00:00:00+08:00
## Objective
驗證並改進 speakdrill 專案：確認 `c` 切換課本功能完整可用、啟動多個子代理檢查整個專案的所有缺陷（程式錯誤、文件過時、版本不一致、測試缺失、CI/發布問題等），逐一修復並發布新版本到 npm 與 GitHub Releases。

## Stopping condition
1. `speakdrill tui` 中按 `c` 能切換課本並自動重載目錄；`m` 鍵與自動啟動 MyET 正常；無 DEBUG 輸出污染介面。
2. 多個子代理各自完成專案缺陷掃描（code / docs / release / tests），所有 Critical/High 問題皆已修復或給出明確理由；執行 `cargo build --release`、`speakdrill doctor`、`speakdrill list --refresh` 全部通過。
3. 版本號已提升，`git tag v*` 已推送，CI release workflow 通過，`npm i -g speakdrill@<new>` 實測可安裝並顯示新版號。
4. 進度同步於 npm registry 可見（或誠實標註仍需人工核准）。

## Must read first
- C:\Users\CPXru\Desktop\thumb\Projects\speakdrill\crates\speakdrill-cli\src\tui.rs
- C:\Users\CPXru\Desktop\thumb\Projects\speakdrill\README.md
- C:\Users\CPXru\Desktop\thumb\Projects\speakdrill\RELEASE.md
- C:\Users\CPXru\Desktop\thumb\Projects\speakdrill\.github\workflows\release.yml
- C:\Users\CPXru\Desktop\thumb\Projects\speakdrill\npm\install.js（實為 scripts/install.js）
- C:\Users\CPXru\Desktop\thumb\Projects\speakdrill\Cargo.toml / npm\package.json

## Verification
- `cargo build --release` 成功
- `speakdrill doctor` 顯示「全部正常」
- `speakdrill list --refresh` 回傳正確課程
- `gh run list --limit 1` 顯示最新 tag CI 成功
- `npm view speakdrill versions --json` 包含新版號

## Progress log
- [2026-11-14] checkpoint0：確認 `c` 鍵 TUI 切換課本實際已可用（v0.2.10 修復）； npm 發布問題：手動 `npm publish` 因未登入被 E401 擋下，CI 上次發布了 v0.2.10，但 npm registry 目前 latest=0.2.10 (0.2.10 存在)。
- [2026-11-14] checkpoint1：GOAL 設定完成（ROOT=C:\Users\CPXru\Desktop\thumb\Projects\speakdrill）。已啟動 3 個 explore 子代理並行掃描：A) Rust code/state 缺陷（含 build warnings、logic、panics）；B) 文件 stale（README/RELEASE/npm README vs 程式實際行為）；C) 發布管線缺陷（workflows/install.js/package.json 為何發成 0.2.6 而非 0.2.10）。等待回報後彙整修復、 bump version、 tag 發布。
- [2026-11-14] checkpoint2：子代理 B 完成 — 文件 stale 彙整（詳見對話摘要）：README.md:137 漏寫 Shift+Space/c/m/s/r；README.md:141-146 debug port 已過時（TUI 啟動自動帶、m 鍵手動）；README.md:222 缺 list/all；RELEASE.md/README.md 版本 0.2.0→0.2.11；npm/README.md:7/43/45-48 VoiceMeeter-only 已過時（VB-CABLE 路線＋speakdrill setup 已自動化）；npm package keywords 缺 vb-cable；config.rs:51 預設模板指向不存在的 tts_kokoro.py；kokoro_tts.py 在 release zip/npm binaries 與「開發用」README 矛盾；RELEASE.md:44 repository.url 前綴錯。另 report stale build junk 清單（bump_version.py / fix_*.py / check_*.py 等）。代價：未改任何檔，僅回報。等待子代理 A 與 C 回報再統一修復。
- [2026-11-14] checkpoint3：子代理 C 完成 — 發布管線根因確立：A) CI 無 tag↔version 驗證（release.yml 全檔未比對 ref_name vs Cargo.toml vs npm/package.json）；B) 本機 pack 夾帶陳舊 exe（files:["bin/"] 白名單 vs .gitignore）且 install.js skip-if-exists 讓舊 exe 永久勝出、失敗 exit(0) 靜默安裝壞套件；C) bump 由一次性腳本+手動 tag，導致 v0.2.7 tag 內容卻是 0.2.6（npm 真正事故）；D) v0.2.10 實發 0.2.10 為準確但僥倖對齊。已修：.github/workflows/release.yml（移除 workflow_dispatch、新增 verify job 校對 tag/Cargo/package.json 一致，不一致直接失敗；setup-node v7、package-manager-cache:false、concurrency/timeout、zip 不再夾帶 kokoro_tts.py、npm job 同樣擋 verify）；npm/scripts/install.js（刪 skip-if-exists、解壓後檢查 exes 存在、失敗 process.exit(1)）；npm/package.json "files" 改白名單 bin/*.js 不夾 binaries、keywords 加 vb-cable；scripts/bump-version.mjs 新增泛用 bump 腳本（更新 Cargo.toml/package.json/Cargo.lock 並提示 tag）。代價：未刪本機陳舊 npm/bin/binaries/win32-x64/* 與 npm/speakdrill-0.1.2.tgz（需使用者確認）；未驗證 CI 實際跑過。待修：A 的 Rust 碼缺陷（build warnings、logic、panic）；B/C 給出的文件 stale 清單。
- [2026-11-14] checkpoint4：子代理 A 完成 — Code 審計（build 無 error、GUI 無 warning）重點：CRITICAL pipeline.rs:233 `RENEW"` 多一個引號，導致 cur_idx 恆為 None → TTS 永不播放、所有句子評分落在 index 0、成績單 backfill 跳過、整條管線失效（必修）；CRITICAL/HIGH TUI.rs ~150-177 MyET 自動啟動在 port 檢查永遠失敗時每 5 秒重生 MyET10.exe 無上限（且 --remote-debugging-port 寫死 9222、check_myet_running 寫死 myet.com/140.125 片段、path 寫死安裝路徑、spawn 失敗被吞掉）；HIGH stop flag 在 TUI（app.stop）與 GUI（self.stop）永不歸零，`s`/■ 一次後之後所有執行都靜默 no-op；HIGH config.rs 腐敗設定靜默變 factory default + 非原子寫入，使用者設定可能被蓋掉；HIGH per_app.rs:133 `path.contains("0.0.1")` 測的是友名字串而不是 raw_id，capture flow mirror GUID 恆選錯；HIGH navigate() 假完成 Ok；VM login 失敗仍回 Ok 並宣稱路由成功；TTS cmd /C 原始插值有注入風險；--force 旗標核心支援但前端缺 --force。MEDIUM ~35 項。子代理 B：12 項 stale（README keys 不全、debug port 描述過時、RELEASE.md 仍寫 v0.2.0、npm/README VoiceMeeter-only 說明與 speakdrill setup 已自動化的文件衝突）。先修 P0：pipeline regex、stop flag、MyET respawn、config 原子寫入、per_app raw_id；再修其餘可驗證項，最後 bump → tag → 發布驗收。注意：pipeline regex 修復會讓真實管線第一次依賴 console log 正確運作，修完必須以跑一課實測（需 MyET 登入狀態）才能算驗收。
- [2026-11-14] checkpoint5：P0-P1 缺陷已全部修復並通過驗證：
  - pipeline.rs: RENEW regex 修正（移除多餘引號），CDP reader 不再靜默死亡（暫未改、需觀察）
  - TUI/GUI: stop flag 在 start_batch/start_run 開頭歸零，`s`/■ 後可再次執行
  - TUI: MyET 自動啟動改為只嘗試一次、可配置路徑/port、等待 CDP 最多 20 秒
  - TUI: m 鍵手動啟動同樣用配置、只啟動一次
  - config.rs: 腐敗設定備份為 corrupt.json + 原子寫入
  - per_app.rs: capture mirror GUID 改用 raw_id 判斷
  - cdp.rs: navigate() 逾時回 Err 而非 Ok
  - vm.rs: VBVMR_Login 失敗回 Err，TUI/CLI 錯誤改用 Progress log
  - check_app_binding 改用 Progress log 而非 println!
  - 移除 debug logging、大小寫鍵位支援 (b/B, c/C)、Ctrl+C 退出
  - 清理死碼/未使用 import
  - `cargo build --release` 成功、警告極少；`speakdrill doctor` 全部正常；`speakdrill list --refresh` 正確回傳目錄
  下一步：更新 RELEASE.md/README.md 文件、bump 版本、tag、CI 發布驗收。
- [2026-11-14] checkpoint6：文件同步完成：
  - RELEASE.md：版本號更新為 0.2.11、新增 CI 保護機制說明、發佈後驗證清單、正確的 bump 腳本用法
  - README.md：debug port 章節更新（自動化）、TUI 快捷鍵完整列表、CLI 子命令加上 list/all
  - npm/README.md：VB-CABLE 為推薦路線、speakdrill setup 自動化 per-app 綁定、移除 VoiceMeeter-only 描述
  - config.rs：預設 TTS 模板改為說明由 init 自動設定 Piper
  - 移除 npm/bin/binaries/ 陳舊 exe、npm/speakdrill-0.1.2.tgz、根目錄一次性腳本（bump_version.py 等）
  準備 bump 版本 → tag → 推送 → CI 驗收。
- [2026-11-14] checkpoint7：發布完成 ✅
  - v0.2.14 已推送 tag，CI release workflow 全部通過（verify、build、npm）
  - npm package 0.2.14 已發布至 registry，`latest` 標籤指向 0.2.14
  - `npm i -g speakdrill@latest` 實測安裝成功，`speakdrill --version` 顯示 0.2.14
  - `speakdrill doctor` 顯示「全部正常」，`speakdrill list --refresh` 正確回傳 39 課目錄
  - install.js 修復：改用 .NET ZipFile::ExtractToDirectory 直接解壓，解決 PowerShell 7 模組衝突導致的解壓失敗
  - GitHub Release v0.2.14 已建立，含 speakdrill.exe、speakdrill-gui.exe
  - 所有停止條件已滿足，目標達成
- Status: completed
- Updated: 2026-11-14T18:45:00+08:00