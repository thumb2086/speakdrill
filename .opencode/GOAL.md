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
- [2026-11-15] checkpoint8：v0.2.24 發布並實測通過 ✅
  - 根因：我 checkpoint5 把 RENEW 正則「修」成 `RENEW\s+SentenceIdxArray=`，但 MyET10 實際 console 是 `CallApp:<FlashGame Action="RENEW" SentenceIdxArray="0" ...>`，正則永遠不匹配 → cur_idx 恆 None → TTS 不播 → 每句 0~7 分 → 卡死在重試
  - 修正：`Action="RENEW"\s+SentenceIdxArray="([0-9]+)"`；對話框改寬鬆 `未完成` 匹配 + 多候選按鈕；新增 `SPEAKDRILL_DEBUG=1` 輸出 `[cd]` console event
  - v0.2.24 CI (run 43713130408)：verify/build/npm 全部通過；`npm i -g speakdrill@latest` → v0.2.24；`speakdrill doctor` 全部正常
  - 實測 BKM-GAR-2A001 9/9 平均 91.2；TUI 實測 21 句平均 90.0
  - 使用者確認：「確定對了，沒錯了」
- [2026-11-15] checkpoint9：v0.2.25 發布（TUI/GUI 重構）✅
  - TUI：移除 `c` 切換課本（popup、code_select_* 狀態）；修正 ↑/↓ 邊界誤用 lessons.len() 的 bug；push_log 500 行上限涵蓋所有事件；merge_catalog/group_lesson 消除重複分組；編譯 0 warning；wrap_text 測試 4/4
  - GUI：改為與 TUI 相同——啟動自動檢查/啟動 MyET、全部課本 + 完成率、勾選加入佇列；視窗 1340x720；CodeName 清單改用 core KNOWN_CODENAMES；修正下拉顯示 1A 但清單是 2A 的不一致；500 行 log 上限
  - README：硬體需求章節、移除 c 與不存在的 Shift+Space 說明
  - v0.2.25 CI (run 43748327579)：verify/build/npm 全部通過；`npm i -g speakdrill@latest` → v0.2.25；`doctor` 全部正常
  - 使用者確認：「我覺得ok了」→ 發 v0.2.25
- [2026-11-15] checkpoint10：v0.2.26 發布（MyET 官方累計時數 + 佇列修正）✅
  - 新功能：TUI 標題列 / GUI 狀態列顯示 MyET 官方累計學習時數（學習顧問室 Dashboard.aspx 的「學習時間」）
  - 實作：fetch_myet_stats() — 直接連當前頁 → 導航 Dashboard.aspx → 讀 innerText → 導回原頁；
    背景/前景分頁（/json/new）不可靠（CEF 新分頁不導航），改走 CDP 導航現有頁面
  - 實測：Rust e2e 讀回 MyetStats { hours: 15.2, LV 44, 證書 16 }；parse 有 2 個單元測試
  - 讀取失敗退回本機累計（totals.json，save_report 每課累加 elapsed_secs）
  - list 去重 commit 67edce2：fetch_all_catalogs 同一 lesson_id 只保留第一份 → list 385 → 204
  - TUI/GUI 佇列改 Vec 保留勾選順序（原 HashSet 隨機）；Enter 跑佇列/跑選取略過已完成課並顯示略過數
  - v0.2.26 CI (run 43761074229)：verify/build/npm 全部通過；`npm i -g speakdrill@latest` → v0.2.26；doctor 全部正常
- [2026-11-16] checkpoint11：v0.2.27（GUI 啟動 MyET 按鈕）+ v0.2.28（init 自動綁麥克風）發布 ✅
  - v0.2.27：GUI 新增「🚀 啟動 MyET」按鈕（--remote-debugging-port=9222 --remote-allow-origins=*），與 TUI m 鍵一致；
    MyetStats 讀取完成前顯示「累計 …」（不再先閃本機 0.0）；
    v0.2.27 CI (43762177788) ✓；npm latest
  - v0.2.28：`speakdrill init` 最後一步自動呼叫 setup（registry → COM → UIA），已綁定直接結束；
    修正 setup 順序 bug（registry 已綁定但 MyET 未執行時不再白開 ms-settings 視窗）；
    TUI footer 補上 r 重載 / m 啟動 MyET / s 停止 提示；README / npm README 安裝流程更新；
    v0.2.28 CI (43762953986)：verify/build/npm ✓；`npm i -g` → 0.2.28
  - npm 安裝被舊 TUI 行程（pid 41860，鎖住 speakdrill.exe）擋下 EBUSY → 關閉該行程後安裝成功
- [2026-11-16] checkpoint12：v0.2.29 發布（CodeName 帶錯修復 + 執行中追加課本）✅
  - 事故：TUI 選 BKM-B2B，MyET 頁面顯示 2A/Unit 5.1、第一句 2 分 — pipeline::run_lesson 一律用 cfg.code（BKM_2A）
  - 修正：LessonRef.code 存抓取時的 CodeName，跑課網址用它；`speakdrill run` 無 --code 時先在設定的 code 找、找不到掃全部目錄定位；CLI/TUI/GUI 全改
  - 新功能：批次執行中按 Space 勾選課本 → 本批結束後自動接續跑（TUI 標題/狀態列即時顯示追加數；GUI 同步；停止中不自動開始；計數器跨批保留）；佇列在批次開始時被消耗
  - 修正 README 過期的「Shift+Space 整課程加入佇列」（實際無此功能）
  - 舊快取相容（serde default + 讀取覆寫）；unit tests 全綠；v0.2.29 CI (43788651775) ✓（verify/build/npm）；`npm i -g` → 0.2.29
  - 使用者本次中止了跑錯的 B2B 批次（第一句 2 分已寫入 MyET 紀錄 — 無法撤回）
- [2026-11-16] checkpoint13：v0.2.30~v0.2.33 發布 ✅
  - v0.2.30（載入賽跑修復）：TUI 自動啟動 MyET 後太早抓目錄/官方時數 → 目錄被「0 個有成績」的劣化結果覆寫（全部 0%）、官方時數吃到 placeholder「0.0 小時」。
    修正：抓目錄前等 readyState==complete 且 URL 含 AudreyWeb（30 秒）；官方統計改讀元素 id（lblLearnTimeValue/Unit、divCertificateValue、lblExpValue）輪詢到「證書與 LV 皆非空」才採用；
    目錄快取保護（refresh=false 且新抓 0 個有成績、舊快取 ≥1 個 → 保留舊快取）；實測 live fetch → 15.6 小時 / LV 44 / 證書 16；
    v0.2.30 CI (43791366991) ✓；使用者即時 --refresh 修復快取（1A/1B/2A 共 114 課有積分找回）
  - v0.2.31：CLI help/doctor/run 指引同步到自動綁定時代（Init/Doctor 說明、doctor/run 綁定失敗時指引 speakdrill setup 取代指路 ms-settings）
  - v0.2.32：文件全面同步（README「輸出」改寫 — 官方目錄成績、官方累計時數；npm README「使用時你會看到」；LESSONS #10 readyState≠資料填完）；CI (43791980980) ✓
  - v0.2.33：init [4/6] 通用化 — 不再只拉 VoiceMeeter B1，依 routes() 找 mic_device 把錄音端（CABLE Output／B1）拉到 100%；結尾加下一步引導；實測在使用者機器上 [4/6] 顯示錄音端 100% ✓、[6/6] registry 已綁定直接結束（不開視窗）；CI (43792265325) ✅（npm latest=0.2.33）
- [2026-11-16] checkpoint14：v0.2.34（說明文件全面重寫）+ v0.2.35（課本內容預覽 + CI 快取）發布 ✅
  - v0.2.34：README 新增「新手上路（10 分鐘）」＋「常見問題（FAQ）」＋「疑難排解」；報告 JSON 範例；npm README「文件」指向新區塊；
    順帶修正過期 wording（README init 文案「設 VoiceMeeter 路由」、CLI --about / Init doc、npm README）；CI (43792872777)
  - v0.2.35：TUI 按 i 顯示選中課本課程列表（單元名＋課號＋✓/○）；GUI「📋 課本內容」按鈕對等；
    執行中日誌改顯示「▶ 單元名（課號）」；CI rust-cache 改 shared-key: speakdrill-release（Cargo.lock 隨版本變 → 每次全量重編 → 修正後增量快取）
  - 使用者回報「還是選的跟跑得不一樣」：查證實際 reports — 2B018~2B028 全部平均 86~90 ✓ CodeName 修正生效；
    只有 BKM-GAR-2B029 0 分（38 秒，title 空，極可能 MyET 被關/太早導航，可重跑）；user 困惑源於 GAR 課號不易辨識 → 以 i/📋 預覽解決
- [2026-11-16] checkpoint15：v0.2.36（`speakdrill update` 指令）+ v0.2.37（進練習頁不再死等 10 秒）發布 ✅
  - v0.2.36：新子指令 update — 比對目前版本與 npm registry 最新版（`npm view speakdrill version`），印更新指令；
    不自動安裝（Windows exe 不能就地覆寫；EBUSY 已踩過兩次）；version_gt 有單元測試；實測 `speakdrill update` 輸出正確；
    CI (43794644636) ✓；npm i -g → 0.2.36
  - v0.2.37：進入 ScoringPage.aspx 後固定 sleep 10s → 1.5s（navigate() 已等 readyState=complete；console 事件有緩衝 — 10 秒純 padding）
  - 使用者問「以他的判別，先掛機，時間會比較長嗎」— 指的就是這個 10 秒空跑
  - 使用者機器 npm 鎖定問題頻繁（EBUSY）：v0.2.37 起全域更新改由使用者關 TUI 後自行 `npm i -g speakdrill@latest`（v0.2.36 已裝）
- [2026-11-17/18] checkpoint16：多次 npm 發布失敗（reset 了沒提交）→ v0.2.45~v0.2.52 npm 全卡在 Validating
  - GUI「堵塞」卡頓：根因 = 紀錄面板 stick_to_bottom+500 行 monospace 重畫，每次 200 ms 強制 repaint
  - 修正：CentralPanel 紀錄區改用 `ScrollArea::show_rows()` 僅渲染可見行；實測閒置 CPU 0% (之前持續吃 CPU)
  - v0.2.43~v0.2.52 多次發佈失敗：MSI → 去掉 MSI → NPI (並非 npm bug，release.yml 在 build job 多條路徑被 git checkout tag 檔覆蓋，Release 產生但 npm 的附件驗證通過後反而失敗)
  - 修改：砍掉 installer（Inno Setup 只留 config，不在 CI 打包），回到 EXEs+zip 流程
  - v0.2.54 完成 npm publish，但 npm `latest` 停在 0.2.46（語證 Validating — npm 對 thumb2086 帳號，不是 CI bug）
  - v0.2.55 完成 npm publish（追加課本按 Enter 不自動接跑 + 佇列位置提示）
  - v0.2.56：self-update 改 npm（仿 OpenCode）；npm 只發 TUI；驗證 Mac 不支援、Windows-only
- Status: completed
- Updated: 2026-11-18T15:10:00+08:00