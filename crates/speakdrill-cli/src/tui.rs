//! 終端機介面（ratatui）

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

use speakdrill_core::{
    run_lesson, save_report, AudioPlayer, Cdp, LessonRef, LessonReport, LineScore,
    Progress, RunOptions, Vm,
};
use std::process::Command;
use tokio::time::Duration as TokioDuration;

use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line as TLine, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
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

/// 產生正常模式下的 status 字串
fn build_status(app: &App) -> String {
    let queued = app.queue.len();
    if app.lessons.is_empty() {
        "目錄是空的 — 可能沒登入或 CodeName 不對；按 r 重試".into()
    } else {
        format!(
            "{} 課（{}）  ↑/↓ 移動 · Space 單選 · Shift+Space 整課程 · g 加入課本 · Enter 執行（{}）· b 全部 · c 切換課本 · q 離開",
            app.lessons.len(),
            if queued > 0 { format!("已勾 {queued} 課") } else { "Enter 跑單一課程".to_string() },
            if queued > 0 { "佇列" } else { "選取" },
        )
    }
}

/// 互動式 App 狀態
struct App {
    lessons: Vec<(String, String)>,
    titles: Vec<String>,
    sel: usize,
    logs: Vec<String>,
    running: bool,
    stop: Arc<AtomicBool>,
    tx: UnboundedSender<Ev>,
    total_done: usize,
    avg_sum: f64,
    avg_n: usize,
    status: String,
    /// 佇列（以 lesson_id 為鍵，reload 後仍然有效）
    queue: HashSet<String>,
    /// CodeName 選單模式
    code_select_mode: bool,
    code_select_idx: usize,
    /// CodeName 多選模式下的已選索引
    code_select_selected: HashSet<usize>,
    /// 儲存開啟 CodeName 選單前的 status，關閉時恢復
    prev_status: String,
}

/// 常見 CodeName 清單（可依需求增減）
fn known_codenames() -> Vec<(&'static str, &'static str)> {
    vec![
        ("BKM_1A", "書林美語走天下-1A"),
        ("BKM_1B", "書林美語走天下-1B"),
        ("BKM_2A", "書林美語走天下-2A"),
        ("BKM_2B", "書林美語走天下-2B"),
        ("BKM_3A", "書林美語走天下-3A"),
        ("BKM_3B", "書林美語走天下-3B"),
        ("BKM_GAR_1A", "Getting Around in American English 1A"),
        ("BKM_GAR_2A", "Getting Around in American English 2A"),
        ("BKM_AME_1A", "American English 1A"),
        ("BKM_MBA_SALES", "MBA 英語-銷售管理"),
        ("BKM_MBA_PROD", "MBA 英語-生產管理"),
        ("BKM_MBA_FIN", "MBA 英語-財務管理"),
        ("BKM_MBA_HR", "MBA 英語-人資管理"),
        ("BKM_MBA_MKT", "MBA 英語-行銷管理"),
        ("BKM_MBA_RD", "MBA 英語-研發管理"),
        ("BKM_MICE", "MICE 英語"),
        ("BKM_TOEIC_1", "多益口說測驗攻略 Part 1"),
        ("BKM_TOEIC_2", "多益口說測驗攻略 Part 2"),
        ("BKM_TOEIC_3", "多益口說測驗攻略 Part 3"),
        ("BKM_TOEIC_SP", "TOEIC Speaking"),
    ]
}
pub fn run(rt: tokio::runtime::Runtime) -> Result<()> {
    let (tx, rx) = unbounded_channel::<Ev>();
    let stop = Arc::new(AtomicBool::new(false));

    let mut app = App {
        lessons: Vec::new(),
        titles: Vec::new(),
        sel: 0,
        logs: Vec::new(),
        running: false,
        stop: stop.clone(),
        tx: tx.clone(),
        total_done: 0,
        avg_sum: 0.0,
        avg_n: 0,
        status: "載入目錄中…".into(),
        queue: HashSet::new(),
        code_select_mode: false,
        code_select_idx: 0,
        code_select_selected: HashSet::new(),
        prev_status: String::new(),
    };

    // 背景載入目錄
    spawn_catalog(&rt, tx.clone(), false);

    // 背景檢查 MyET（CDP port 連線）；未連線時只啟動一次並等待 CDP 起來
    let tx_clone = tx.clone();
    rt.spawn(async move {
        tokio::time::sleep(TokioDuration::from_millis(500)).await;
        let cfg = speakdrill_core::Config::load_or_default();
        const MAX_WAIT_SECS: u64 = 20;

        if speakdrill_core::Cdp::check_myet_running("127.0.0.1", cfg.cdp_port).await {
            let _ = tx_clone.send(Ev::Log(format!("MyET 已連線 (CDP {})", cfg.cdp_port)));
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
                return;
            }
        }
        let _ = tx_clone.send(Ev::Log("MyET 已啟動，但 CDP 未連上".into()));
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
            apply(app, ev);
        }

        term.draw(|f| draw(f, app))?;

        if event::poll(std::time::Duration::from_millis(120))? {
            if let Event::Key(k) = event::read()? {
                if k.kind != KeyEventKind::Press {
                    continue;
                }
                if app.code_select_mode {
                    // CodeName 下拉選單模式
                    let codenames = known_codenames();
                    // 開啟選單時先 clamp idx，避免 reload 後陣列變小導致越界
                    if app.code_select_idx >= codenames.len() {
                        app.code_select_idx = codenames.len().saturating_sub(1);
                    }
                    match k.code {
                        KeyCode::Esc => {
                            // 恢復之前的 status
                            app.code_select_mode = false;
                            app.code_select_idx = 0;
                            app.status = std::mem::take(&mut app.prev_status);
                            if app.status.is_empty() {
                                app.status = build_status(app);
                            }
                        }
                        // Ctrl+C 在選單中也能退出
                        KeyCode::Char('c') | KeyCode::Char('C') if k.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
                        KeyCode::Enter => {
                            let mut selected = app.code_select_selected.clone();
                            if selected.is_empty() {
                                selected.insert(app.code_select_idx);
                            }
                            // 先切換到第一個並 reload
                            if let Some(&first_idx) = selected.iter().next() {
                                let (code, name) = codenames[first_idx];
                                let mut cfg = speakdrill_core::Config::load_or_default();
                                cfg.code = code.to_string();
                                cfg.save().ok();
                                app.logs.push(format!("已切換課本：{}（{}），重新載入目錄…", name, code));
                                app.code_select_mode = false;
                                app.code_select_idx = 0;
                                app.code_select_selected.clear();
                                app.status = std::mem::take(&mut app.prev_status);
                                if app.status.is_empty() {
                                    app.status = build_status(app);
                                }
                                spawn_catalog(rt, tx.clone(), true);
                                // 其餘選中的加入佇列
                                for &idx in &selected {
                                    if idx != first_idx {
                                        let (code, _name) = codenames[idx];
                                        let prefix = code.replace('_', "-B").replace("BKM-B", "BKM-B") + "-";
                                        for (lesson_id, course_id) in &app.lessons {
                                            if course_id.starts_with(&prefix) {
                                                app.queue.insert(lesson_id.clone());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // Space：切換多選
                        KeyCode::Char(' ') => {
                            if app.code_select_selected.contains(&app.code_select_idx) {
                                app.code_select_selected.remove(&app.code_select_idx);
                            } else {
                                app.code_select_selected.insert(app.code_select_idx);
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            // 使用 checked_add 避免溢位
                            if let Some(next) = app.code_select_idx.checked_add(1) {
                                if next < codenames.len() {
                                    app.code_select_idx = next;
                                }
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            app.code_select_idx = app.code_select_idx.saturating_sub(1);
                        }
                        _ => {}
                    }
                } else {
                    // 正常操作模式
                    match k.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        // Ctrl+C 也能退出 (處理 'c' 和 'C' 都可)
                        KeyCode::Char('c') | KeyCode::Char('C') if k.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
                        KeyCode::Down | KeyCode::Char('j') => {
                            if app.sel + 1 < app.lessons.len() {
                                app.sel += 1;
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            app.sel = app.sel.saturating_sub(1);
                        }
                        KeyCode::Char(' ') if !app.lessons.is_empty() => {
                            // 檢查是否按住 Shift
                            if k.modifiers.contains(KeyModifiers::SHIFT) {
                                // Shift+Space：把同一 course_id 的所有單元加入佇列
                                let current_course = app.lessons[app.sel].1.clone();
                                let count_before = app.queue.len();
                                for (lesson_id, course_id) in &app.lessons {
                                    if course_id == &current_course {
                                        app.queue.insert(lesson_id.clone());
                                    }
                                }
                                let added = app.queue.len() - count_before;
                                app.logs.push(format!("已加入同課程 {} 個單元（course: {}）", added, current_course));
                            } else {
                                // 普通 Space：切換單一單元
                                let id = app.lessons[app.sel].0.clone();
                                if !app.queue.insert(id.clone()) {
                                    app.queue.remove(&id);
                                }
                            }
                        }
                        KeyCode::Char('x') if !app.queue.is_empty() => {
                            app.queue.clear();
                            app.logs.push("(已清空佇列)".into());
                        }
                        KeyCode::Char('a') if !app.lessons.is_empty() => {
                            let all: HashSet<String> = app.lessons.iter().map(|(l, _)| l.clone()).collect();
                            if app.queue.len() == all.len() {
                                app.queue.clear();
                                app.logs.push("(取消全選)".into());
                            } else {
                                app.queue = all;
                                app.logs.push("(已全選)".into());
                            }
                        }
                        KeyCode::Char('r') if !app.running => {
                            app.logs.push("... 重新載入目錄".into());
                            spawn_catalog(rt, tx.clone(), true);
                        }
                        // m 鍵：手動啟動 MyET（含 debug port，只啟動一次）
                        KeyCode::Char('m') if !app.running => {
                            app.logs.push("... 正在啟動 MyET".into());
                            let myet_path = std::env::var("SPEAKDRILL_MYET_PATH")
                                .unwrap_or_else(|_| r"C:\Program Files (x86)\LLabs\MyET10\MyET10.exe".to_string());
                            if !std::path::Path::new(&myet_path).exists() {
                                app.logs.push("找不到 MyET 執行檔".into());
                            } else {
                                let port_str = speakdrill_core::Config::load_or_default().cdp_port.to_string();
                                let _ = std::process::Command::new(&myet_path)
                                    .args([format!("--remote-debugging-port={}", port_str), "--remote-allow-origins=*".to_string()])
                                    .spawn();
                                app.logs.push(format!("MyET 已啟動 (debug port {})，等待 CDP 連線...", port_str));
                            }
                        }
                        KeyCode::Char('s') => {
                            stop.store(true, Ordering::Relaxed);
                            app.logs.push("... 請求中止".into());
                        }
                        KeyCode::Enter if !app.running && !app.lessons.is_empty() => {
                            let picked = if app.queue.is_empty() {
                                app.lessons.get(app.sel).cloned().into_iter().collect::<Vec<_>>()
                            } else {
                                app.lessons
                                    .iter()
                                    .filter(|(l, _)| app.queue.contains(l))
                                    .cloned()
                                    .collect::<Vec<_>>()
                            };
                            if picked.is_empty() {
                                app.logs.push("(佇列與目錄對不上，請按 r 重新載入)".into());
                            } else {
                                start_batch(app, rt, tx.clone(), stop.clone(), picked);
                            }
                        }
                        KeyCode::Char('b') | KeyCode::Char('B') if !app.running && !app.lessons.is_empty() => {
                            let all = app.lessons.clone();
                            start_batch(app, rt, tx.clone(), stop.clone(), all);
                        }
                        // CodeName 切換：c 開啟下拉選單
                        KeyCode::Char('c') if !app.running => {
                            // 儲存目前 status，以便關閉選單時恢復
                            app.prev_status = std::mem::take(&mut app.status);
                            app.code_select_mode = true;
                            app.code_select_idx = 0;
                            app.status = "選擇課本（↑/↓ 選擇，Enter 確認，Esc 取消）".into();
                        }
                        // g 鍵：把目前課本（CodeName）的所有單元加入佇列
                        KeyCode::Char('g') if !app.running && !app.lessons.is_empty() => {
                            let cfg = speakdrill_core::Config::load_or_default();
                            let code = &cfg.code;
                            // CodeName 格式：BKM_1A -> course_id 前綴：BKM-B1A-
                            let prefix = code.replace('_', "-B").replace("BKM-B", "BKM-B") + "-";
                            let count_before = app.queue.len();
                            for (lesson_id, course_id) in &app.lessons {
                                if course_id.starts_with(&prefix) {
                                    app.queue.insert(lesson_id.clone());
                                }
                            }
                            let added = app.queue.len() - count_before;
                            app.logs.push(format!("已加入課本 {} 共 {} 個單元（course_id 前綴：{}）", code, added, prefix));
                        }
                        _ => {}
                    }
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
    lessons: Vec<(String, String)>,
) {
    // 清除先前中止請求，讓停止按鈕可重複使用
    stop.store(false, Ordering::Relaxed);
    app.running = true;
    app.total_done = 0;
    app.avg_sum = 0.0;
    app.avg_n = 0;
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

        for (lesson, course) in lessons {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let _ = tx.send(Ev::LessonStart(lesson.clone()));
            let mut p = ChannelProgress { tx: tx.clone(), stop: stop.clone() };
            match run_lesson(&cdp, &cfg, &player, &lesson, &course, &RunOptions::default(), &mut p).await {
                Ok(r) => {
                    let _ = save_report(&r);
                    let _ = tx.send(Ev::LessonDone(r));
                }
                Err(e) => {
                    let _ = tx.send(Ev::Log(format!("✗ {lesson}：{e}")));
                }
            }
        }
        let _ = tx.send(Ev::BatchDone);
    });
}
fn apply(app: &mut App, ev: Ev) {
    match ev {
        Ev::Catalog(list) => {
            app.logs.push(format!("目錄載入：{} 課", list.len()));
            app.titles = list.iter().map(|l| l.label()).collect();
            app.lessons = list.iter().map(|l| (l.lesson_id.clone(), l.course_id.clone())).collect();
            // 選取位置夾回有效範圍（reload 後目錄可能變小）
            app.sel = app.sel.min(app.lessons.len().saturating_sub(1));
            app.queue.retain(|id| app.lessons.iter().any(|(l, _)| l == id));
            // 更新 status（若不在 code_select_mode 才更新）
            if !app.code_select_mode {
                app.status = build_status(app);
            }
        }
        Ev::CatalogAppend(list) => {
            app.logs.push(format!("+{} 課", list.len()));
            for l in &list {
                if !app.lessons.iter().any(|(id, _)| id == &l.lesson_id) {
                    app.titles.push(l.label());
                    app.lessons.push((l.lesson_id.clone(), l.course_id.clone()));
                }
            }
            app.sel = app.sel.min(app.lessons.len().saturating_sub(1));
            app.queue.retain(|id| app.lessons.iter().any(|(l, _)| l == id));
            if !app.code_select_mode {
                app.status = build_status(app);
            }
        }
        Ev::Log(s) => {
            app.logs.push(s);
            if app.logs.len() > 500 {
                app.logs.remove(0);
            }
        }
        Ev::Score(s) => {
            app.logs.push(format!(
                "  #{} {} 分（發音 {} 語調 {} 流利 {} 重音 {}）",
                s.index, s.total, s.pronunciation, s.pitch, s.fluency, s.stress
            ));
            app.total_done += 1;
            app.avg_sum += s.total as f64;
            app.avg_n += 1;
        }
        Ev::LessonStart(l) => app.logs.push(format!("=== {l} ===")),
        Ev::LessonDone(r) => {
            app.logs.push(format!(
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
            app.logs.push(format!("✗ {e}"));
        }
    }
}
fn spawn_catalog(rt: &tokio::runtime::Runtime, tx: UnboundedSender<Ev>, refresh: bool) {
    let _ = tx.send(Ev::Log("spawn_catalog 開始...".into()));
    rt.spawn(async move {
        let cfg = speakdrill_core::Config::load_or_default();
        let _ = tx.send(Ev::Log(format!("spawn_catalog: code={}, refresh={}", cfg.code, refresh)));
        match Cdp::connect("127.0.0.1", cfg.cdp_port).await {
            Ok(cdp) => {
                let _ = tx.send(Ev::Log("spawn_catalog: CDP 已連線".into()));
                // 先載入預設 CodeName（快速，約 1-2 秒）
                let default_code = cfg.code.clone();
                let default_name = speakdrill_core::config::KNOWN_CODENAMES
                    .iter()
                    .find(|(c, _)| *c == default_code)
                    .map(|(_, n)| *n)
                    .unwrap_or("預設");
                match speakdrill_core::lesson::fetch_catalog_opts(&cdp, &cfg.server, &default_code, &cfg.content_type, refresh).await {
                    Ok(mut lessons) => {
                        let _ = tx.send(Ev::Log(format!("spawn_catalog: fetch_catalog_opts 回傳 {} 課", lessons.len())));
                        if !lessons.is_empty() {
                            for lesson in &mut lessons {
                                lesson.title = format!("{}  [{}]", lesson.title, default_name);
                            }
                            let _ = tx.send(Ev::Catalog(lessons));
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Ev::Fatal(format!("抓目錄失敗：{e}")));
                        return;
                    }
                }
                // 背景載入其餘 CodeName，逐批 append
                let other_codes: Vec<(&str, &str)> = speakdrill_core::config::KNOWN_CODENAMES
                    .iter()
                    .filter(|(c, _)| *c != default_code)
                    .copied()
                    .collect();
                for (code, name) in other_codes {
                    match speakdrill_core::lesson::fetch_catalog_opts(&cdp, &cfg.server, code, &cfg.content_type, refresh).await {
                        Ok(mut lessons) => {
                            if !lessons.is_empty() {
                                for lesson in &mut lessons {
                                    lesson.title = format!("{}  [{}]", lesson.title, name);
                                }
                                let _ = tx.send(Ev::CatalogAppend(lessons));
                            }
                        }
                        Err(_) => {
                            // 忽略無法存取的 CodeName
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
fn draw(f: &mut ratatui::Frame, app: &App) {
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
    ]))
    .block(Block::default().borders(Borders::ALL).title("speakdrill 口說自動練習"));
    f.render_widget(header, chunks[0]);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(46), Constraint::Percentage(54)])
        .split(chunks[1]);

    let items: Vec<ListItem> = app
        .titles
        .iter()
        .zip(app.lessons.iter())
        .map(|(t, (id, _))| {
            if app.queue.contains(id) {
                ListItem::new(TLine::from(format!("● {t}")))
                    .style(Style::default().fg(Color::Yellow))
            } else {
                ListItem::new(TLine::from(format!("  {t}")))
            }
        })
        .collect();
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("課程（已勾 {}）", app.queue.len())),
    )
    .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD));
    let mut st = ListState::default();
    st.select(Some(app.sel));
    f.render_stateful_widget(list, body[0], &mut st);

    let lines: Vec<TLine> = app
        .logs
        .iter()
        .rev()
        .take(200)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|s| TLine::from(s.clone()))
        .collect();
    let log = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("紀錄"))
        .wrap(Wrap { trim: true });
    f.render_widget(log, body[1]);

    let footer_text = if app.code_select_mode {
        format!("選擇課本（↑/↓ 選擇，Space 多選，Enter 套用，Esc 取消） — 已選 {} 本", app.code_select_selected.len())
    } else {
        app.status.clone()
    };
    let footer = Paragraph::new(footer_text)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);

    // CodeName 下拉選單彈窗
    if app.code_select_mode {
        let codenames = known_codenames();
        // 渲染前再 clamp 一次，確保 index 有效
        let clamped_idx = app.code_select_idx.min(codenames.len().saturating_sub(1));
        let items: Vec<ListItem> = codenames
            .iter()
            .enumerate()
            .map(|(i, (code, name))| {
                let selected_mark = if app.code_select_selected.contains(&i) { "☑ " } else { "☐ " };
                let style = if i == clamped_idx {
                    Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(TLine::from(format!("{} {}  {}", selected_mark, code, name))).style(style)
            })
            .collect();
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("選擇課本 (CodeName) — Space 多選，Enter 套用"))
            .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD));
        let mut st = ListState::default();
        st.select(Some(clamped_idx));

        // 計算彈窗區域（置中，寬 70%，高 80%）
        let area = centered_rect(70, 80, f.area());
        f.render_widget(Clear, area); // 清除背景
        f.render_stateful_widget(list, area, &mut st);
    }
}
/// 計算置中矩形（百分比寬高）
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}



