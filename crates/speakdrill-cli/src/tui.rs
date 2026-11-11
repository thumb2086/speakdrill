//! 終端機介面（ratatui）

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use speakdrill_core::{
    fetch_catalog, run_lesson, save_report, AudioPlayer, Cdp, LessonRef, LessonReport, LineScore,
    Progress, RunOptions, Vm,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line as TLine, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Terminal,
};
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
                match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Down | KeyCode::Char('j') => {
                        if app.sel + 1 < app.lessons.len() {
                            app.sel += 1;
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.sel = app.sel.saturating_sub(1);
                    }
                    KeyCode::Char('r') if !app.running => {
                        app.logs.push("… 重新載入目錄".into());
                        spawn_catalog(rt, tx.clone(), true);
                    }
                    KeyCode::Char('s') => {
                        stop.store(true, Ordering::Relaxed);
                        app.logs.push("… 請求中止".into());
                    }
                    KeyCode::Enter if !app.running && !app.lessons.is_empty() => {
                        let (lesson, course) = app.lessons[app.sel].clone();
                        start_batch(app, rt, tx.clone(), stop.clone(), vec![(lesson, course)]);
                    }
                    KeyCode::Char('b') if !app.running && !app.lessons.is_empty() => {
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
            app.status = format!("{} 課（Enter 跑選取、b 跑全部、q 離開）", app.lessons.len());
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
        .map(|t| ListItem::new(TLine::from(t.clone())))
        .collect();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("課程"))
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

    let footer = Paragraph::new(app.status.clone())
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}

