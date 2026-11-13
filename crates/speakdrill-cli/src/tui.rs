//! 終端機介面（ratatui）

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

use speakdrill_core::{
    fetch_catalog, run_lesson, save_report, AudioPlayer, Cdp, LessonRef, LessonReport, LineScore,
    Progress, RunOptions, Vm,
};

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
            "{} 課（{}）  ↑/↓ 移動 · Space 單選 · Shift+Space 整課程 · Enter 執行（{}）· b 全部 · c 切換課本 · q 離開",
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
        prev_status: String::new(),
    };

    // 背景載入目錄
    spawn_catalog(&rt, tx.clone(), false);

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
                // DEBUG: log all key presses
                app.logs.push(format!("KEY: {:?} modifiers={:?} code_select_mode={} running={}", k.code, k.modifiers, app.code_select_mode, app.running));
                if app.logs.len() > 500 { app.logs.remove(0); }
                if app.code_select_mode {
                    // CodeName 下拉選單模式
                    let codenames = known_codenames();
                    // 開啟選單時先 clamp idx，避免 reload 後陣列變小導致越界
                    if app.code_select_idx >= codenames.len() {
                        app.code_select_idx = codenames.len().saturating_sub(1);
                    }
                    match k.code {
                        KeyCode::Esc | KeyCode::Char('c') => {
                            // DEBUG: c/Esc key handler in code_select_mode
                            app.logs.push("DEBUG: 'c' or Esc key handler triggered in code_select_mode".into());
                            if app.logs.len() > 500 { app.logs.remove(0); }
                            // 恢復之前的 status
                            app.code_select_mode = false;
                            app.code_select_idx = 0;
                            app.status = std::mem::take(&mut app.prev_status);
                            if app.status.is_empty() {
                                app.status = build_status(app);
                            }
                        }
                        KeyCode::Enter => {
                            let idx = app.code_select_idx.min(codenames.len().saturating_sub(1));
                            let (code, name) = codenames[idx];
                            let mut cfg = speakdrill_core::Config::load_or_default();
                            cfg.code = code.to_string();
                            cfg.save().ok();
                            app.logs.push(format!("已切換課本：{}（{}），重新載入目錄…", name, code));
                            app.code_select_mode = false;
                            app.code_select_idx = 0;
                            app.status = std::mem::take(&mut app.prev_status);
                            if app.status.is_empty() {
                                app.status = build_status(app);
                            }
                            spawn_catalog(rt, tx.clone(), true);
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
                        // Ctrl+C 也能退出
                        KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
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
                        // m 鍵：自動啟動 MyET 並加上 debug port
                        KeyCode::Char('m') if !app.running => {
                            app.logs.push("... 正在啟動 MyET (debug port 9222)".into());
                            let myet_path = r"C:\Program Files (x86)\LLabs\MyET10\MyET10.exe";
                            if std::path::Path::new(myet_path).exists() {
                                std::process::Command::new(myet_path)
                                    .args(["--remote-debugging-port=9222", "--remote-allow-origins=*"])
                                    .spawn()
                                    .map(|_| app.logs.push("MyET 已啟動，等待 CDP 連線...".into()))
                                    .unwrap_or_else(|e| app.logs.push(format!("啟動 MyET 失敗：{}", e)));
                            } else {
                                app.logs.push("找不到 MyET 執行檔".into());
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
                        KeyCode::Char('b') if !app.running && !app.lessons.is_empty() => {
                            let all = app.lessons.clone();
                            start_batch(app, rt, tx.clone(), stop.clone(), all);
                        }
                        // CodeName 切換：c 開啟下拉選單
                        KeyCode::Char('c') if !app.running => {
                            // DEBUG: c key handler reached - write to file for verification
                            std::fs::write("C:\\Users\\CPXru\\.thumb-work\\c_key_triggered.txt", format!("c key handler triggered at {:?}, running={}, code_select_mode={}", std::time::SystemTime::now(), app.running, app.code_select_mode)).ok();
                            // DEBUG: c key handler reached
                            app.logs.push("DEBUG: 'c' key handler triggered in normal mode".into());
                            if app.logs.len() > 500 { app.logs.remove(0); }
                            // 儲存目前 status，以便關閉選單時恢復
                            app.prev_status = std::mem::take(&mut app.status);
                            app.code_select_mode = true;
                            app.code_select_idx = 0;
                            app.status = "選擇課本（↑/↓ 選擇，Enter 確認，Esc 取消）".into();
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
                let _ = tx.send(Ev::Fatal(format!("連不上 speakdrill：{e}")));
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
                let _ = vm.route_to_b1(strip);
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
    rt.spawn(async move {
        let cfg = speakdrill_core::Config::load_or_default();
        match Cdp::connect("127.0.0.1", cfg.cdp_port).await {
            Ok(cdp) => match speakdrill_core::lesson::fetch_catalog_opts(
                &cdp, &cfg.server, &cfg.code, &cfg.content_type, refresh,
            )
            .await
            {
                Ok(list) => {
                    let _ = tx.send(Ev::Catalog(list));
                }
                Err(e) => {
                    let _ = tx.send(Ev::Fatal(format!("抓目錄失敗：{e}")));
                }
            },
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
    let header = Paragraph::new(TLine::from(vec![
        Span::styled(" speakdrill ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
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
        format!("選擇課本（↑/↓ 選擇，Enter 確認，Esc 取消)")
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
                let style = if i == clamped_idx {
                    Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(TLine::from(format!("{}  {}", code, name))).style(style)
            })
            .collect();
        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title("選擇課本 (CodeName)"))
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



