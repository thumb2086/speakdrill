# LESSONS.md — speakdrill 踩坑紀錄（以免再犯）

> 每一條都來自真實發生過的事故。寫在改 code 之前先讀一遍。

---

## 1. 不要憑肉眼「覺得正則不對」就改（v0.2.14 → v0.2.24 事故）

**症狀**：使用者 TUI 跑課，每句都是 0~7 分、`#0` 無限重試，整課永遠完不成。

**根因**：MyET10 實際 console 輸出是：

```
CallApp:<FlashGame Action="RENEW" SentenceIdxArray="0" APSessionID="291866" />
```

我在 v0.2.14 把原本**其實正確**的正則（能匹配上面格式）「修」成
`RENEW\s+SentenceIdxArray="..."`——少了前面的 `Action="RENEW"`，永遠不匹配。
後果：`cur_idx` 恆為 `None` → **TTS 從來沒播放** → 麥克風只收到環境雜音 → 每句 5 分
→ 全部 ✗ 重試到 max_retry 放棄。

**修正**（v0.2.24）：`Action="RENEW"\s+SentenceIdxArray="([0-9]+)"`，全課 9/9 完成、平均 91.2。

**規則**：
1. 正則／解析器看起來不對 → **先抓真實樣本再改**。用 `SPEAKDRILL_DEBUG=1`（見下）
   讓程式印出 console event 原文，對照實際格式寫 regex，不要腦補。
2. 改完**一定要實測跑完一整課**，看到 `▶ #n` 出現、分數 80+、`✓ <LessonID> 平均 xx`，
   才能宣稱修好。`cargo build --release` 成功 ≠ 功能正常。
3. 子代理／文件說的「應該是 clamp／索引／型別問題」只是假設。
   **實際 log 優先於任何診斷。**

---

## 2. `SPEAKDRILL_DEBUG=1` 是標準除錯入口

TUI／CLI 執行時設環境變數即可看到 MyET 的 console event 原文與對話框內容：

```powershell
$env:SPEAKDRILL_DEBUG="1"      # PowerShell：不是 CMD 的 set X=Y
speakdrill                      # 或 speakdrill run --lesson ...  
```

log 會出現：

```
[cd] CallApp:<FlashGame Action="RENEW" SentenceIdxArray="0" APSessionID="..." />
RESUME_DIALOG|<對話框內文前 300 字>
```

- `[cd]` = MyET console event 原文（從 CDP `Runtime.consoleAPICalled` 抓到的）
- `RESUME_DIALOG|` = 「繼續進行未完成的句子」對話框出現時印出的 body 前 300 字

**沒看到 `[cd]` 開頭的行 → 代表根本沒抓到 console event，先查 CDP 連線／debug port，再查正則。**

---

## 3. 「修完」的定義：跑完整一課驗收

本專案的驗收標準（依序）：

1. `cargo build --release` 成功
2. `speakdrill doctor` 全部正常
3. **`SPEAKDRILL_DEBUG=1 speakdrill run --lesson <某一課>` 跑完整課**：
   - 有 `▶ #n`（TTS 有播）
   - 每句分數 ≥ pass_score（預設 80）
   - 結尾印出 `✓ <LessonID> 平均 xx（N 句 / xx 秒）`
4. 若改的是 TUI／GUI 流程：啟動後目錄載入正常、課本列表有完成率
5. 才考慮 bump 版本、tag、push、等 CI、npm 安裝驗證

v0.2.14 的事故就是在第 3 步缺席的情況下發布的。**不准跳過。**

---

## 4. PowerShell 與 git 的相處規則

**事故**：`git commit -m "fix: ... \d+ ..."`
→ PowerShell 雙引號內 `\d`／`\"` 不會照 bash 規則處理；`\"` 不是跳脫，字串提前結束，
後半段被當成指令執行 → `\d+: 無法辨識…`，commit 失敗，
但 **tag v0.2.24 已經建到舊 commit**。事後只好刪 tag、重建、`git push --force` tag。

**規則**：
1. commit message 只要含**雙引號或反斜線**（尤其正則 `\d`、`\s`）→ 一律寫檔 + `git commit -F <file>`。
2. PowerShell 雙引號字串的跳脫字元是反引號 `` ` ``，不是 `\`。跨 shell 相容的 message 用 `-F`。
3. tag 推上去才發現指錯 commit → `git tag -d <tag>` 本地刪、重建、`git push --force origin <tag>`；
   CI 的 `verify` job 會核對 tag ↔ Cargo.toml ↔ npm/package.json 版本一致才放行，所以 force 後要確認 CI 重跑。
4. PowerShell 環境變數：`$env:NAME="value"`，不要用 CMD 的 `set NAME=value`。

---

## 5. MyET 對話框／UI 文字不要寫死完整句子

**現象**：`繼續進行未完成的句子` 對話框，早期版本用完整字串 `body.includes('繼續進行未完成的句子')`
比對，不同 MyET build 文案不同就匹配不到。

**現在的做法**：
- 用寬鬆關鍵字 `未完成` 匹配 body
- 按鈕用**多候選**依序嘗試：`['否','不要','從頭開始','重新開始','重來']`（resume='是' 時用 `['是','繼續','繼續練習']`）
- 匹配到就 `console.log('RESUME_DIALOG|' + body.slice(0,300))`，方便事後確認實際文案
- 對話框配 `SPEAKDRILL_DEBUG=1` 才會留 log

**規則**：凡是依賴「App 畫面上的文字」的判斷，都必須寬鬆匹配 + 多候選 + 除錯輸出，
**不要**依賴某一個 build 的完整文案。

---

## 6. 驗證「修好」的操作順序（除錯 SOP）

```
1. speakdrill doctor            # 先確認 CDP/裝置/TTS 全部 OK；不 OK 先修環境
2. $env:SPEAKDRILL_DEBUG="1"
   speakdrill run --lesson <一課>   # 看 [cd]、▶、✗、平均
3. 定位問題層次：
   - 沒有 [cd] 行         → CDP 沒連上／沒抓到 console → 查 cdp.rs、debug port、自動啟動
   - 有 [cd] 但沒有 ▶     → cur_idx 沒抓到 → 查 RENEW 正則（見第 1 條）
   - 有 ▶ 但分數全 ✗      → TTS 播放或 per-app 麥克風／音量問題 → 查 doctor、audio.rs
   - 對話框卡住        → 查 RESUME_DIALOG 行，依實際文案改候選按鈕
4. 修完 → 重跑第 2 步 → 全課通過 → 才發版
```

---

## 7. commit message 暫存檔不要放 repo 根目錄（連犯兩次）

**事故**：`git add -A` 把 `.thumb-work-commit.txt`／`commitmsg.txt` 一起提交進 repo
（v0.2.24 發布與本次 refactor 各一次），事後只能再補一個 `git rm` 的 chore commit 清掉。

**規則**：
1. commit message 暫存檔一律寫到 `C:\Users\CPXru\.thumb-work\`，**不要**放在 repo 根目錄。
2. `git add` 用明確路徑（`git add README.md crates/...`），**不要用 `git add -A`**。
3. 「先 `add` 再 `Remove-Item` 暫存檔」的順序必然踩雷，直接把暫存檔放 repo 外才一勞永逸。

## 8. 小結（每條一行版）

| # | 一句話 |
|---|---|
| 1 | 正則先抓真實樣本（`SPEAKDRILL_DEBUG=1`）再改，不要肉眼改 |
| 2 | `SPEAKDRILL_DEBUG=1` 印 `[cd]` 與 `RESUME_DIALOG|`，是標準除錯入口 |
| 3 | 「修好」= 跑完整一課看到 `✓ 平均 xx`，不是 build 過 |
| 4 | commit message 含雙引號／反斜線 → 寫檔 + `git commit -F`；tag 指錯 → `push --force` |
| 5 | 依賴 App 畫面文字的判斷：寬鬆匹配 + 多候選按鈕 + 除錯輸出 |
| 6 | 除錯順序：doctor → DEBUG run → 依 [cd]/▶/✗ 定位層次 → 修 → 重跑驗收 |
| 7 | commit message 暫存檔寫到 `.thumb-work\`，`git add` 用明確路徑不用 `-A` |

---

## 9. 跑課網址的 CodeName 必須與目錄一致（v0.2.29）

**症狀**：TUI 選了 BKM-B2B，MyET 頁面卻顯示「2A/Unit 5.1」，第一句 2 分。

**根因**：`pipeline::run_lesson` 不分課程，一律把 `cfg.code`（設定檔的 BKM_2A）帶進 `ScoringPage.aspx` 的 `CodeName=`。
課程屬於 B2B，頁面開的是 2A → 內容對不上。

**修正（v0.2.29）**：目錄抓課程時就把當時用的 CodeName 存進 `LessonRef.code`，跑課網址用這個。
舊快取沒有 `code` 欄位 → `#[serde(default)]` + 讀取時以目前 CodeName 覆寫，相容。
`speakdrill run --lesson X`（無 `--code` 參數）也改為先在設定的 CodeName 找，找不到就掃所有目錄定位。

**教訓**：
1. 任何「全域設定」被用於「每個項目各自不同」的資料，一定會錯 — 要把來源存到項目上（`LessonRef.code`）。
2. 選單做對不代表跑進去也對；要沿著資料鏈一路檢查到「實際發出的 HTTP 請求」。

---

## 10. `readyState=complete` ≠ 資料填完（v0.2.30 事故）

**症狀**：MyET 重開後 TUI 顯示「MyET 官方累計 0.0 小時」，全部課本 0%。

**根因**（兩個獨立的賽跑）：
1. MyET 開機比 CDP port 早好，TUI 自動啟動後立刻抓目錄 → `CategoryLessonScores.aspx` 的 best_score 全是 null → 落回只有課名的 `CatalogPage.aspx` → **劣化結果被寫進快取蓋掉好資料** → 全部 0%。
2. Dashboard.aspx 的官方值（學習時間/LV/證書）是 **JS 非同步填進去的**，HTML 先畫 placeholder（時間 `0.0 分`、證書/LV 空），readyState=complete 就讀會直接吃到 placeholder。

**修正**：
- 目錄抓取前先等 `readyState==complete 且 URL 含 AudreyWeb`（最多 30 秒）。
- 官方統計改直接讀元素 id（`lblLearnTimeValue`/`Unit`、`divCertificateValue`、`lblExpValue`），輪詢到「證書與 LV 皆非空」才採用；讀不到才退回 innerText regex。
- 目錄快取保護：refresh=false 且「新抓 0 個有成績、舊快取 ≥1 個」→ 保留舊快取不覆蓋。

**教訓**：
1. 凡是「App 畫面上的數字」，先問一句：**這是 SSR 內容還是 JS 填的？** 填的是 → 直接讀元素 id + 輪詢非空，別讀 innerText。
2. 任何會**毀掉好資料**的覆寫路徑（寫快取/寫設定）都要有保留舊值的保護條件。
3. `shell.remove_item` 之外，「讀取+寫快取」也是一種破壞性動作，要像對待刪除一樣謹慎。
