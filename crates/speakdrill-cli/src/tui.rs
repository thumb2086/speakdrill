//! 終端機介面（ratatui）

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

use speakdrill_core::{
    AudioPlayer, Cdp, Config, LessonRef, LessonReport, LineScore,
    Progress, RunOptions, SilentProgress, Vm, fetch_myet_stats, run_lesson,
    save_report, totals_get, version_gt,
};
use std::process::Command;
use tokio::time::Duration as TokioDuration;

use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line as TLine, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};

use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

#[derive(Debug, Clone)]
pub enum Ev {
    Catalog(Vec<LessonRef>),
    CatalogAppend(Vec<LessonRef>),
    MyetStats(speakdrill_core::MyetStats),
    /// MyET 官方累計時數讀取失敗（才退回顯示本機值）
    MyetStatsFailed,
    /// 課本清單 (code, 顯示名稱)；用於課本分組的顯示名
    Books(Vec<(String, String)>),
    Log(String),
    Score(LineScore),
    LessonStart(String),
    LessonDone(LessonReport),
    BatchDone,
    Fatal(String),
}

pub struct ChannelProgress {
    pub tx: UnboundedSender<Ev>,
    pub stop: Arc<AtomicBool>,
}

impl Progress for ChannelProgress {
    fn log(&mut self, msg: &str) {
        let _ = self.tx.send(Ev::Log(msg.to_string()));
    }
    fn line_done(&mut self, s: &LineScore) {
        let _ = self.tx.send(Ev::Score(s.clone()));
    }
    fn should_stop(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

/// 推一行 log，並把紀錄上限維持在 500 行
fn push_log(app: &mut App, s: String) {
    app.logs.push(s);
    if app.logs.len() > 500 {
        app.logs.remove(0);
    }
}

/// 把一行文字依 unicode 顯示寬度 pre-wrap 成多列（CJK 每字 2 欄）
fn wrap_text(s: &str, width: usize, out: &mut Vec<String>) {
    use unicode_width::UnicodeWidthChar;
    if width == 0 {
        out.push(s.to_string());
        return;
    }
    if s.is_empty() {
        out.push(String::new());
        return;
    }
    let mut cur = String::new();
    let mut cur_w = 0usize;
    for ch in s.chars() {
        let w = ch.width().unwrap_or(0);
        if cur_w + w > width && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        cur.push(ch);
        cur_w += w;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
}

#[cfg(test)]
mod tests {
    use super::wrap_text;

    fn wrap(s: &str, w: usize) -> Vec<String> {
        let mut out = Vec::new();
        wrap_text(s, w, &mut out);
        out
    }

    #[test]
    fn ascii_wraps_by_columns() {
        assert_eq!(wrap("hello world foo", 5), vec!["hello", " worl", "d foo"]);
    }

    #[test]
    fn cjk_counts_double_width() {
        // 每個 CJK 字 2 欄，寬度 6 → 每列 3 個字
        assert_eq!(wrap("一二三四五六", 6), vec!["一二三", "四五六"]);
    }

    #[test]
    fn empty_line_is_preserved() {
        assert_eq!(wrap("", 10), vec![""]);
    }

    #[test]
    fn short_line_not_split() {
        assert_eq!(wrap("短句", 20), vec!["短句"]);
    }
}

/// 產生 status 字串（執行中顯示追加提示，平常顯示完整按鍵列）
fn build_status(app: &App) -> String {
    let queued = app.queue.len();
    if app.coursebooks.is_empty() {
        "目錄是空的 — 可能沒登入或 CodeName 不對；按 r 重試".into()
    } else if app.running {
        if queued > 0 {
            format!("■ 執行中 — 已追加 {queued} 本（本批結束後接著跑）· Space 續選/取消 · s 停止 · PgUp/PgDn 捲動 · End 回最新")
        } else {
            "■ 執行中 — Space 勾選課本＝本批結束後追加 · s 停止 · PgUp/PgDn 捲動紀錄 · End 回最新".into()
        }
    } else {
        format!(
            "{} 本課本（{}）  ↑/↓ 移動 · Space 勾選/追加 · Enter 執行（{}）· i 課本內容 · a 全選 · x 清空 · b 全部 · r 重載 · m 啟動 MyET · s 停止 · PgUp/PgDn 捲動 · End 回最新 · q 離開",
            app.coursebooks.len(),
            if queued > 0 { format!("已勾 {queued} 本") } else { "Enter 跑單一課本".to_string() },
            if queued > 0 { "佇列" } else { "選取" },
        )
    }
}

/// 互動式 App 狀態
struct App {
    /// 所有課程
    lessons: Vec<LessonRef>,
    /// 課本清單：(cb_key, 顯示名稱, [LessonRef])
    coursebooks: Vec<(String, String, Vec<LessonRef>)>,
    /// 佇列：課本索引（Vec 保留勾選順序）
    queue: Vec<usize>,
    /// 已完成的課程 ID
    completed: HashSet<String>,
    sel: usize,
    logs: Vec<String>,
    running: bool,
    total_done: usize,
    avg_sum: f64,
    avg_n: usize,
    /// 累計練習時數（秒，本機累計；MyET 官方時數在 myet_hours）
    total_secs: u64,
    /// MyET 官方累計學習時數（小時）
    myet_hours: Option<f64>,
    myet_level: Option<String>,
    myet_certs: Option<u64>,
    /// MyET 官方時數讀取失敗（顯示本機值）
    myet_stats_failed: bool,
    /// 課本清單：(CodeName, 顯示名稱)，用於課本分組顯示名
    books_names: std::collections::HashMap<String, String>,
    status: String,
    /// 紀錄面板：從最新往回捲的行數（0 = 跟隨最新輸出）
    log_scroll: usize,
    /// 紀錄面板可見行數（draw 時更新，供 PgUp/PgDn 翻頁用）
    log_page: usize,
}

pub fn run(rt: tokio::runtime::Runtime) -> Result<()> {
    let (tx, rx) = unbounded_channel::<Ev>();
    let stop = Arc::new(AtomicBool::new(false));

    let mut app = App {
        lessons: Vec::new(),
        coursebooks: Vec::new(),
        queue: Vec::new(),
        completed: HashSet::new(),
        sel: 0,
        logs: Vec::new(),
        running: false,
        total_done: 0,
        avg_sum: 0.0,
        avg_n: 0,
        total_secs: speakdrill_core::totals_get(),
        myet_hours: None,
        myet_level: None,
        myet_certs: None,
        myet_stats_failed: false,
        books_names: std::collections::HashMap::new(),
        status: "載入目錄中…".into(),
        log_scroll: 0,
        log_page: 10,
    };

    // 背景檢查 MyET（CDP port 連線）；未連線時只啟動一次並等待 CDP 起來
    let tx_clone = tx.clone();
    rt.spawn(async move {
        tokio::time::sleep(TokioDuration::from_millis(500)).await;
        let cfg = speakdrill_core::Config::load_or_default();
        const MAX_WAIT_SECS: u64 = 20;

        if speakdrill_core::Cdp::check_myet_running("127.0.0.1", cfg.cdp_port).await {
            let _ = tx_clone.send(Ev::Log(format!("MyET 已連線 (CDP {})", cfg.cdp_port)));
            spawn_catalog(tx_clone.clone(), false);
            return;
        }

        let myet_path = std::env::var("SPEAKDRILL_MYET_PATH")
            .unwrap_or_else(|_| r"C:\Program Files (x86)\LLabs\MyET10\MyET10.exe".to_string());
        if !std::path::Path::new(&myet_path).exists() {
            let _ = tx_clone.send(Ev::Log("找不到 MyET 執行檔，請手動啟動".into()));
            return;
        }

        let port_str = cfg.cdp_port.to_string();
        let _ = Command::new(&myet_path)
            .args([format!("--remote-debugging-port={}", port_str), "--remote-allow-origins=*".to_string()])
            .spawn();
        let _ = tx_clone.send(Ev::Log(format!("正在啟動 MyET (debug port {})...", port_str)));

        for _ in 0..MAX_WAIT_SECS {
            tokio::time::sleep(TokioDuration::from_secs(1)).await;
            if speakdrill_core::Cdp::check_myet_running("127.0.0.1", cfg.cdp_port).await {
                let _ = tx_clone.send(Ev::Log(format!("MyET 已連線 (CDP {})", cfg.cdp_port)));
                spawn_catalog(tx_clone.clone(), false);
                return;
            }
        }
        let _ = tx_clone.send(Ev::Log("MyET 已啟動，但 CDP 未連上".into()));
    });

    // 背景讀 MyET 官方累計學習時數（學習顧問室 Dashboard.aspx）；
    // 等到 MyET 有 page target 後讀背景分頁、讀完即關，不影響課堂分頁
    let tx_stats = tx.clone();
    rt.spawn(async move {
        let cfg = speakdrill_core::Config::load_or_default();
        for _ in 0..40 {
            tokio::time::sleep(TokioDuration::from_millis(500)).await;
            if speakdrill_core::Cdp::check_myet_running("127.0.0.1", cfg.cdp_port).await {
                break;
            }
        }
        match speakdrill_core::fetch_myet_stats(&cfg.server, cfg.cdp_port, true).await {
            Ok(s) => {
                let _ = tx_stats.send(Ev::Log(format!("MyET 官方累計學習時間：{:.1} 小時", s.hours)));
                let _ = tx_stats.send(Ev::MyetStats(s));
            }
            Err(e) => {
                let _ = tx_stats.send(Ev::Log(format!("MyET 累計時數讀取失敗（改用本機累計）：{e}")));
                let _ = tx_stats.send(Ev::MyetStatsFailed);
            }
        }
    });

    // 背景檢查新版本（每次啟動時檢查一次，避免頻繁請求）
    let tx_version = tx.clone();
    rt.spawn(async move {
        tokio::time::sleep(TokioDuration::from_secs(3)).await; // 等 UI 先顯示
        let current = env!("CARGO_PKG_VERSION");
        let client = reqwest::Client::new();
        let url = "https://api.github.com/repos/thumb2086/speakdrill/releases/latest";
        if let Ok(resp) = reqwest::Client::new()
            .get("https://api.github.com/repos/thumb2086/speakdrill/releases/latest")
            .header("User-Agent", "speakdrill")
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await
        {
            if let Ok(release) = resp.json::<serde_json::Value>().await {
                if let Some(tag) = release["tag_name"].as_str() {
                    let latest = tag.trim_start_matches('v');
                    if version_gt(latest, env!("CARGO_PKG_VERSION")) {
                        let _ = tx_version.send(Ev::Log(format!(
                            "🔔 發現新版本 v{}（目前 v{}），執行 `speakdrill self-update` 可一鍵更新",
                            latest,
                            env!("CARGO_PKG_VERSION")
                        )));
                    }
                }
            }
        }
    });

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend)?;

    let (tx2, stop2) = (tx.clone(), stop.clone());
    let mut rxc = rx;
    let res = event_loop(&mut term, &mut app, &mut rxc, &rt, tx2, stop2);

    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;
    term.show_cursor()?;
    res
}
fn event_loop(
    term: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    rx: &mut UnboundedReceiver<Ev>,
    rt: &tokio::runtime::Runtime,
    tx: UnboundedSender<Ev>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    loop {
        // 收事件
        while let Ok(ev) = rx.try_recv() {
            let is_batch_done = matches!(ev, Ev::BatchDone);
            apply(app, ev);
            // 本批結束後，自動接續跑執行中勾選的「追加課本」（停止中不自動開始）
            if is_batch_done {
                if stop.load(Ordering::Relaxed) {
                    if !app.queue.is_empty() {
                        push_log(app, format!("（停止中：追加的 {} 本課本不會自動開始，按 Enter 可開始）", app.queue.len()));
                    }
                } else if !app.queue.is_empty() {
                    let selected = std::mem::take(&mut app.queue);
                    let (picked, skipped) = expand_picked(app, &selected);
                    if skipped > 0 {
                        push_log(app, format!("略過已完成 {} 課", skipped));
                    }
                    if picked.is_empty() {
                        push_log(app, "（追加的課本都已完成）".into());
                    } else {
                        push_log(app, format!("接續跑追加：{} 課（{} 本課本）", picked.len(), selected.len()));
                        start_batch(app, rt, tx.clone(), stop.clone(), picked);
                    }
                }
            }
        }

        term.draw(|f| draw(f, app))?;

        if event::poll(std::time::Duration::from_millis(120))? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                    // 正常操作模式
                    match k.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        // Ctrl+C 也能退出 (處理 'c' 和 'C' 都可)
                        KeyCode::Char('c') | KeyCode::Char('C') if k.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
                        KeyCode::Down | KeyCode::Char('j') => {
                            if app.sel + 1 < app.coursebooks.len() {
                                app.sel += 1;
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            app.sel = app.sel.saturating_sub(1);
                        }
                        // 紀錄面板捲動（PgUp/PgDn 翻頁、Home 到最早、End 回最新）
                        KeyCode::PageUp => {
                            app.log_scroll = app.log_scroll.saturating_add(app.log_page.max(1));
                        }
                        KeyCode::PageDown => {
                            app.log_scroll = app.log_scroll.saturating_sub(app.log_page.max(1));
                        }
                        KeyCode::Home => {
                            app.log_scroll = usize::MAX; // draw 時會夾到最上緣
                        }
                        KeyCode::End => {
                            app.log_scroll = 0;
                        }
                        KeyCode::Char(' ') if !app.coursebooks.is_empty() => {
                            // Space：切換課本選取（佇列保留勾選順序）；執行中＝加入「本批結束後追加」
                            if app.queue.contains(&app.sel) {
                                app.queue.retain(|&i| i != app.sel);
                            } else {
                                app.queue.push(app.sel);
                            }
                            if app.running {
                                app.status = if app.queue.is_empty() {
                                    "執行中".into()
                                } else {
                                    format!("執行中 · 追加 {} 本（本批結束後接著跑）", app.queue.len())
                                };
                            }
                        }
                        KeyCode::Char('x') if !app.queue.is_empty() => {
                            app.queue.clear();
                            push_log(app, "(已清空佇列)".into());
                        }
                        KeyCode::Char('a') if !app.coursebooks.is_empty() => {
                            let all: Vec<usize> = (0..app.coursebooks.len()).collect();
                            if app.queue.len() == all.len() {
                                app.queue.clear();
                                push_log(app, "(取消全選)".into());
                            } else {
                                app.queue = all;
                                push_log(app, "(已全選)".into());
                            }
                        }
                        KeyCode::Char('r') if !app.running => {
                            push_log(app, "... 重新載入目錄".into());
                            spawn_catalog(tx.clone(), true);
                        }
                        KeyCode::Char('R') if !app.running => {
                            push_log(app, "... 強制重新整理 MyET 官方時數".into());
                            let tx_stats = tx.clone();
                            let cfg = speakdrill_core::Config::load_or_default();
                            rt.spawn(async move {
                                match speakdrill_core::fetch_myet_stats(&cfg.server, cfg.cdp_port, true).await {
                                    Ok(s) => {
                                        let _ = tx_stats.send(Ev::Log(format!("MyET 官方累計學習時間：{:.1} 小時", s.hours)));
                                        let _ = tx_stats.send(Ev::MyetStats(s));
                                    }
                                    Err(e) => {
                                        let _ = tx_stats.send(Ev::Log(format!("MyET 累計時數讀取失敗：{e}")));
                                        let _ = tx_stats.send(Ev::MyetStatsFailed);
                                    }
                                }
                            });
                        }
                        // m 鍵：手動啟動 MyET（含 debug port，只啟動一次）
                        KeyCode::Char('m') if !app.running => {
                            push_log(app, "... 正在啟動 MyET".into());
                            let myet_path = std::env::var("SPEAKDRILL_MYET_PATH")
                                .unwrap_or_else(|_| r"C:\Program Files (x86)\LLabs\MyET10\MyET10.exe".to_string());
                            if !std::path::Path::new(&myet_path).exists() {
                                push_log(app, "找不到 MyET 執行檔".into());
                            } else {
                                let port_str = speakdrill_core::Config::load_or_default().cdp_port.to_string();
                                let _ = std::process::Command::new(&myet_path)
                                    .args([format!("--remote-debugging-port={}", port_str), "--remote-allow-origins=*".to_string()])
                                    .spawn();
                                push_log(app, format!("MyET 已啟動 (debug port {})，等待 CDP 連線...", port_str));
                            }
                        }
                        KeyCode::Char('s') => {
                            stop.store(true, Ordering::Relaxed);
                            push_log(app, "... 請求中止".into());
                        }
                        KeyCode::Enter if !app.running && !app.coursebooks.is_empty() => {
                            // 把選中的課本（或佇列中的課本，保留勾選順序）展開為 lessons；
                            // 已完成的課自動略過
                            let selected: Vec<usize> = if app.queue.is_empty() {
                                vec![app.sel]
                            } else {
                                std::mem::take(&mut app.queue)
                            };
                            let (picked, skipped) = expand_picked(app, &selected);
                            if skipped > 0 {
                                push_log(app, format!("略過已完成 {} 課（佇列共 {} 課）", skipped, picked.len() + skipped));
                            }
                            if picked.is_empty() {
                                push_log(app, "（佇列中的課本都已完成；要全部重跑按 b）".into());
                            } else {
                                start_batch(app, rt, tx.clone(), stop.clone(), picked);
                            }
                        }
                        KeyCode::Char('i') if !app.coursebooks.is_empty() => {
                            // 顯示課本內容（單元名 + 課號），讓「選的課本」與「跑的課」一眼對得上
                            let mut lines = Vec::new();
                            if let Some((code, name, lessons)) = app.coursebooks.get(app.sel) {
                                lines.push(format!("── {} {} 課程列表（{}）──", code, name, lessons.len()));
                                for l in lessons {
                                    let mark = if app.completed.contains(&l.lesson_id) { "✓" } else { " " };
                                    lines.push(format!("  {mark} {}", lesson_label(&l.lesson_id, &l.title)));
                                }
                                lines.push("── Enter 開始 ──".to_string());
                            }
                            for l in lines {
                                push_log(app, l);
                            }
                        }
                        KeyCode::Char('b') | KeyCode::Char('B') if !app.running && !app.lessons.is_empty() => {
                            let all = app.lessons.clone();
                            start_batch(app, rt, tx.clone(), stop.clone(), all);
                        }
                        _ => {}
                    }
            }
        }
    }
}
fn start_batch(
    app: &mut App,
    rt: &tokio::runtime::Runtime,
    tx: UnboundedSender<Ev>,
    stop: Arc<AtomicBool>,
    lessons: Vec<LessonRef>,
) {
    // 清除先前中止請求，讓停止按鈕可重複使用
    stop.store(false, Ordering::Relaxed);
    // 保留累計計數（追加接續視為同一個 session）
    app.running = true;
    app.status = format!("執行中（{} 課）", lessons.len());

    rt.spawn(async move {
        let cfg = speakdrill_core::Config::load_or_default();
        let cdp = match Cdp::connect("127.0.0.1", cfg.cdp_port).await {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(Ev::Fatal(format!("連不上 MyET：{e}")));
                return;
            }
        };
        if let Err(e) = speakdrill_core::ensure_logged_in(&cdp, &cfg.server).await {
            let _ = tx.send(Ev::Fatal(format!("{e}")));
            return;
        }
        let player = match AudioPlayer::new(&cfg.play_device) {
            Ok(p) => p,
            Err(e) => {
                let _ = tx.send(Ev::Fatal(format!("音訊裝置：{e}")));
                return;
            }
        };
        if cfg.auto_vm_setup {
            if let Ok(vm) = Vm::connect() {
                let (strip, _bus) = cfg.vm_strip.zip(cfg.vm_bus).unwrap_or_else(|| vm.default_route());
                if let Err(e) = vm.route_to_b1(strip) {
                    let _ = tx.send(Ev::Log(format!("VoiceMeeter 路由設定失敗：{e}")));
                }
            } else {
                let _ = tx.send(Ev::Log("VoiceMeeter 未偵測到，跳過路由設定".into()));
            }
        }

        for l in lessons {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let _ = tx.send(Ev::LessonStart(lesson_label(&l.lesson_id, &l.title)));
            let mut p = ChannelProgress { tx: tx.clone(), stop: stop.clone() };
            match run_lesson(&cdp, &cfg, &player, &l.lesson_id, &l.course_id, &l.code, &l.contype, &RunOptions::default(), &mut p).await {
                Ok(r) => {
                    let _ = save_report(&r);
                    let _ = tx.send(Ev::LessonDone(r));
                }
                Err(e) => {
                    let _ = tx.send(Ev::Log(format!("✗ {}：{e}", l.lesson_id)));
                }
            }
        }
        let _ = tx.send(Ev::BatchDone);
    });
}
/// 課程的人類可讀標籤：單元名稱優先，id 當後括號；沒有單元名就只印 id
fn lesson_label(id: &str, title: &str) -> String {
    let t = title.trim();
    if t.is_empty() || t == id {
        id.to_string()
    } else {
        format!("{t}（{id}）")
    }
}

/// 把選中的課本索引展開為待跑課程列表（保留勾選順序，略過已完成）
/// 回傳：(待跑課程，略過數)
fn expand_picked(app: &App, selected: &[usize]) -> (Vec<LessonRef>, usize) {
    let mut picked = Vec::new();
    let mut skipped = 0usize;
    for &idx in selected {
        if let Some((_, _, lessons)) = app.coursebooks.get(idx) {
            for l in lessons {
                if app.completed.contains(&l.lesson_id) {
                    skipped += 1;
                } else {
                    picked.push(l.clone());
                }
            }
        }
    }
    (picked, skipped)
}

/// 依 course_id 前綴把課程分組為課本；replace=true 時先清空重建（重載目錄用）
fn merge_catalog(app: &mut App, list: Vec<LessonRef>, replace: bool) {
    if replace {
        push_log(app, format!("目錄載入：{} 課", list.len()));
        app.coursebooks.clear();
        app.lessons = list.clone();
    }
    for l in &list {
        if l.done() {
            app.completed.insert(l.lesson_id.clone());
        }
        if !replace {
            if !app.lessons.iter().any(|x| x.lesson_id == l.lesson_id) {
                app.lessons.push(l.clone());
            }
        }
        // 按 course_id 前綴分組為課本（例：BKM-B2A-001 → BKM-B2A）
        let parts: Vec<&str> = l.course_id.split('-').collect();
        let cb_key = if parts.len() >= 2 {
            format!("{}-{}", parts[0], parts[1])
        } else {
            l.course_id.clone()
        };
        let cb_name = app
            .books_names
            .get(&l.code)
            .cloned()
            .unwrap_or_else(|| cb_key.clone());
        let triple = || l.clone();
        if let Some(cb) = app.coursebooks.iter_mut().find(|(c, _, _)| c == &cb_key) {
            if !cb.2.iter().any(|x| x.lesson_id == l.lesson_id) {
                cb.2.push(triple());
            }
        } else {
            app.coursebooks.push((cb_key, cb_name, vec![triple()]));
        }
    }
    // queue 保留索引，超出範圍的夾掉
    app.queue.retain(|&i| i < app.coursebooks.len());
    app.sel = app.sel.min(app.coursebooks.len().saturating_sub(1));
    app.status = build_status(app);
}

fn apply(app: &mut App, ev: Ev) {
    match ev {
        Ev::Catalog(list) => merge_catalog(app, list, true),
        Ev::CatalogAppend(list) => merge_catalog(app, list, false),
        Ev::MyetStats(s) => {
            app.myet_hours = Some(s.hours);
            app.myet_level = s.level;
            app.myet_certs = s.certs;
        }
        Ev::MyetStatsFailed => {
            app.myet_stats_failed = true;
        }
        Ev::Books(list) => {
            app.books_names = list.into_iter().collect();
        }
        Ev::Log(s) => push_log(app, s),
        Ev::Score(s) => {
            push_log(app, format!(
                "  #{} {} 分（發音 {} 語調 {} 流利 {} 重音 {}）",
                s.index, s.total, s.pronunciation, s.pitch, s.fluency, s.stress
            ));
            app.total_done += 1;
            app.avg_sum += s.total as f64;
            app.avg_n += 1;
        }
        Ev::LessonStart(l) => push_log(app, format!("▶ {l}")),
        Ev::LessonDone(r) => {
            app.completed.insert(r.lesson_id.clone());
            app.total_secs += r.elapsed_secs;
            push_log(app, format!(
                "✓ {} 平均 {} （{} 句 / {} 秒）",
                r.lesson_id, r.avg, r.lines.len(), r.elapsed_secs
            ));
        }
        Ev::BatchDone => {
            app.running = false;
            let avg = if app.avg_n > 0 { app.avg_sum / app.avg_n as f64 } else { 0.0 };
            app.status = format!("完成：{} 句，平均 {:.1}", app.total_done, avg);
        }
        Ev::Fatal(e) => {
            app.running = false;
            app.status = e.clone();
            push_log(app, format!("✗ {e}"));
        }
    }
}
fn spawn_catalog(tx: UnboundedSender<Ev>, refresh: bool) {
    // Use tokio::spawn since we're called from within a tokio runtime context
    let _ = tx.send(Ev::Log("spawn_catalog 開始...".into()));
    tokio::spawn(async move {
        let cfg = speakdrill_core::Config::load_or_default();
        let _ = tx.send(Ev::Log(format!("spawn_catalog: code={}, refresh={}", cfg.code, refresh)));
        match Cdp::connect("127.0.0.1", cfg.cdp_port).await {
            Ok(cdp) => {
                let _ = tx.send(Ev::Log("spawn_catalog: CDP 已連線".into()));
                // 從「我的課程」端點拿實際課本清單（含每本的 ContentType）；拿不到就退回內建清單
                let enrolled = speakdrill_core::lesson::fetch_enrolled_books(&cdp).await.unwrap_or_default();
                let books: Vec<(String, String, String)> = if enrolled.is_empty() {
                    speakdrill_core::config::KNOWN_CODENAMES
                        .iter()
                        .map(|(c, n)| (c.to_string(), cfg.content_type.clone(), n.to_string()))
                        .collect()
                } else {
                    let _ = tx.send(Ev::Log(format!("找到 {} 本課本", enrolled.len())));
                    enrolled
                };
                let _ = tx.send(Ev::Books(books.iter().map(|(c, _, n)| (c.clone(), n.clone())).collect()));
                // 先載入預設 CodeName（快速，約 1-2 秒）
                let default_code = cfg.code.clone();
                let default_book = books.iter().find(|(c, _, _)| *c == default_code).cloned();
                if let Some((dcode, dtype, _dname)) = &default_book {
                    match speakdrill_core::lesson::fetch_catalog_opts(&cdp, &cfg.server, dcode, dtype, refresh).await {
                        Ok(lessons) => {
                            let _ = tx.send(Ev::Log(format!("spawn_catalog: fetch_catalog_opts 回傳 {} 課", lessons.len())));
                            if !lessons.is_empty() {
                                let _ = tx.send(Ev::Catalog(lessons));
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Ev::Fatal(format!("抓目錄失敗：{e}")));
                            return;
                        }
                    }
                } else {
                    let _ = tx.send(Ev::Log(format!("（找不到預設 CodeName {default_code}，直接背景載入全部）")));
                }
                // 背景載入其餘課本，逐批 append
                for (code, contype, _name) in &books {
                    if *code == default_code {
                        continue;
                    }
                    match speakdrill_core::lesson::fetch_catalog_opts(&cdp, &cfg.server, code, contype, refresh).await {
                        Ok(lessons) => {
                            if !lessons.is_empty() {
                                let _ = tx.send(Ev::CatalogAppend(lessons));
                            }
                        }
                        Err(_) => {
                            // 忽略無法存取的課本
                        }
                    }
                }
            }
            Err(e) => {
                let _ = tx.send(Ev::Fatal(format!("連不上 MyET：{e}")));
            }
        }
    });
}
fn draw(f: &mut ratatui::Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(6), Constraint::Length(3)])
        .split(f.area());

    let avg = if app.avg_n > 0 { app.avg_sum / app.avg_n as f64 } else { 0.0 };
    let version = env!("CARGO_PKG_VERSION");
    let header = Paragraph::new(TLine::from(vec![
        Span::styled(" speakdrill ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" v{} ", version), Style::default().fg(Color::Cyan).bg(Color::Black).add_modifier(Modifier::BOLD)),
        Span::raw(format!("  {} 課", app.lessons.len())),
        Span::raw(if app.running { "   ● 執行中" } else { "" }),
        Span::raw(format!("   完成 {} 句  平均 {:.1}", app.total_done, avg)),
        Span::raw(match &app.myet_hours {
            Some(h) => format!("   MyET 累計 {:.1} 小時", h),
            None if app.myet_stats_failed => {
                format!("   累計 {:.1} 小時（本機）", app.total_secs as f64 / 3600.0)
            }
            None => "   累計 …".to_string(),
        }),
    ]))
    .block(Block::default().borders(Borders::ALL).title("speakdrill 口說自動練習"));
    f.render_widget(header, chunks[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(46), Constraint::Percentage(54)])
        .split(chunks[1]);

    let items: Vec<ListItem> = app
        .coursebooks
        .iter()
        .enumerate()
        .map(|(i, (code, name, lessons))| {
            let total = lessons.len();
            let done = lessons.iter().filter(|l| app.completed.contains(&l.lesson_id)).count();
            let pct = if total > 0 { (done as f64 / total as f64 * 100.0) as u32 } else { 0 };
            let mark = if app.queue.contains(&i) { "●" } else { " " };
            ListItem::new(TLine::from(format!("{} {}  {}  {}/{} ({:.0}%)", mark, code, name, done, total, pct)))
                .style(if app.queue.contains(&i) {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                })
        })
        .collect();
    let list_title = if app.running && !app.queue.is_empty() {
        format!("課本（追加 {} 本·本批結束後接著跑）", app.queue.len())
    } else {
        format!("課本（已勾 {}）", app.queue.len())
    };
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(list_title),
    )
    .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD));
    let mut st = ListState::default();
    st.select(Some(app.sel));
    f.render_stateful_widget(list, body[0], &mut st);

    // 紀錄面板：自己把每行依面板寬度 pre-wrap（CJK 以 unicode 寬度計），
    // 每列剛好對應一個畫面行 → 捲動行數可以完全確定，新行永遠釘在底部
    let inner_width = body[1].width.saturating_sub(2).max(1) as usize;
    let visible = body[1].height.saturating_sub(2) as usize;
    app.log_page = visible.max(1);
    let mut rows: Vec<String> = Vec::new();
    for s in app.logs.iter() {
        wrap_text(s, inner_width, &mut rows);
    }
    let total = rows.len();
    let max_scroll = total.saturating_sub(visible);
    app.log_scroll = app.log_scroll.min(max_scroll);
    let start = if app.log_scroll == 0 {
        max_scroll
    } else {
        max_scroll.saturating_sub(app.log_scroll)
    };
    let end = (start + visible).min(total);
    let lines: Vec<TLine> = rows[start..end].iter().map(|s| TLine::from(s.clone())).collect();
    let log_title = if app.log_scroll == 0 {
        "紀錄（PgUp/PgDn 捲動 · End 回最新）".to_string()
    } else {
        format!("紀錄 ▲ 已回捲 {} 行（PgUp/PgDn · End 回最新）", app.log_scroll)
    };
    let log = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(log_title));
    f.render_widget(log, body[1]);

    let footer = Paragraph::new(app.status.clone())
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}
