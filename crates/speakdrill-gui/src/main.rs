//! speakdrill-gui — 原生 egui 介面（無 webview）
//!
//! 與 TUI 行為對齊：啟動時連線 MyET（沒連上就自動啟動並等待），載入所有課本、
//! 左側列出課本與完成率、勾選加入佇列後一次跑整組課本。

use anyhow::Result;
use speakdrill_core::{save_report, AudioPlayer, Cdp, LessonRef, LessonReport, Progress, RunOptions};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

#[derive(Debug, Clone)]
enum Ev {
    Catalog(Vec<LessonRef>),
    CatalogAppend(Vec<LessonRef>),
    Log(String),
    Line(speakdrill_core::LineScore),
    LessonDone(LessonReport),
    Done,
    SetupDone,
    Fatal(String),
}

struct ChannelProgress {
    tx: Sender<Ev>,
    stop: Arc<AtomicBool>,
}

impl Progress for ChannelProgress {
    fn log(&mut self, msg: &str) {
        let _ = self.tx.send(Ev::Log(msg.to_string()));
    }
    fn line_done(&mut self, s: &speakdrill_core::LineScore) {
        let _ = self.tx.send(Ev::Line(s.clone()));
    }
    fn should_stop(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

/// 已載入的課本：(cb_key, 顯示名稱, lessons)
type Coursebook = (String, String, Vec<LessonRef>);

struct App {
    /// 所有課程（扁平化，供「跑全部」用）
    lessons: Vec<LessonRef>,
    /// 課本清單（與 TUI 相同的分組與完成率）
    coursebooks: Vec<Coursebook>,
    /// 已完成的課程 ID（best_score 有值）
    completed: HashSet<String>,
    /// 佇列：課本索引（與 TUI 相同語意）
    queue: HashSet<usize>,
    /// 目前選取的課本索引
    sel: usize,
    logs: Vec<String>,
    running: bool,
    stop: Arc<AtomicBool>,
    tx: Sender<Ev>,
    rx: Receiver<Ev>,
    status: String,
    done: usize,
    avg_sum: f64,
    avg_n: usize,
    pass: u32,
    setup_busy: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.panel_fill = egui::Color32::from_rgb(22, 24, 30);
        style.visuals.window_fill = egui::Color32::from_rgb(22, 24, 30);
        style.visuals.extreme_bg_color = egui::Color32::from_rgb(14, 16, 20);
        cc.egui_ctx.set_style(style);
        setup_cjk_fonts(&cc.egui_ctx);

        let app = Self {
            lessons: Vec::new(),
            coursebooks: Vec::new(),
            completed: HashSet::new(),
            queue: HashSet::new(),
            sel: 0,
            logs: vec!["載入目錄中…".into()],
            running: false,
            stop: stop.clone(),
            tx: tx.clone(),
            rx,
            status: "載入目錄中…".into(),
            done: 0,
            avg_sum: 0.0,
            avg_n: 0,
            pass: 80,
            setup_busy: false,
        };
        load_catalog(tx.clone(), false);
        app
    }
}

/// 背景載入所有課本目錄；MyET 沒連線就自動啟動並等待最多 20 秒（與 TUI 相同）
fn load_catalog(tx: Sender<Ev>, refresh: bool) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            use tokio::time::Duration as TokioDuration;

            let cfg = speakdrill_core::Config::load_or_default();
            const MAX_WAIT_SECS: u64 = 20;

            // MyET 未啟動 → 自動啟動（帶 debug port）並輪詢最多 20 秒
            if !Cdp::check_myet_running("127.0.0.1", cfg.cdp_port).await {
                let myet_path = std::env::var("SPEAKDRILL_MYET_PATH")
                    .unwrap_or_else(|_| r"C:\Program Files (x86)\LLabs\MyET10\MyET10.exe".to_string());
                if !std::path::Path::new(&myet_path).exists() {
                    let _ = tx.send(Ev::Log("找不到 MyET 執行檔，請手動啟動".into()));
                } else {
                    let port_str = cfg.cdp_port.to_string();
                    let _ = std::process::Command::new(&myet_path)
                        .args([
                            format!("--remote-debugging-port={}", port_str),
                            "--remote-allow-origins=*".to_string(),
                        ])
                        .spawn();
                    let _ = tx.send(Ev::Log(format!("正在啟動 MyET (debug port {})…", port_str)));
                    for _ in 0..MAX_WAIT_SECS {
                        tokio::time::sleep(TokioDuration::from_secs(1)).await;
                        if Cdp::check_myet_running("127.0.0.1", cfg.cdp_port).await {
                            break;
                        }
                    }
                }
            }

            let cdp = match Cdp::connect("127.0.0.1", cfg.cdp_port).await {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(Ev::Fatal(format!("連不上 MyET：{e}")));
                    return;
                }
            };
            let _ = tx.send(Ev::Log(format!("CDP 已連線 ({})，載入目錄…", cfg.cdp_port)));

            // 先載入 config 指定的預設課本（快），其餘背景逐本 append
            let default_code = cfg.code.clone();
            match speakdrill_core::lesson::fetch_catalog_opts(&cdp, &cfg.server, &default_code, &cfg.content_type, refresh).await {
                Ok(l) if !l.is_empty() => {
                    let _ = tx.send(Ev::Log(format!("目錄載入：{} 課", l.len())));
                    let _ = tx.send(Ev::Catalog(l));
                }
                Ok(_) => {
                    let _ = tx.send(Ev::Log(format!("預設課本 {default_code} 目錄是空的")));
                }
                Err(e) => {
                    let _ = tx.send(Ev::Fatal(format!("抓目錄失敗：{e}")));
                    return;
                }
            }
            for (code, name) in speakdrill_core::config::KNOWN_CODENAMES.iter() {
                if *code == default_code {
                    continue;
                }
                match speakdrill_core::lesson::fetch_catalog_opts(&cdp, &cfg.server, code, &cfg.content_type, refresh).await {
                    Ok(l) if !l.is_empty() => {
                        let n = l.len();
                        let _ = tx.send(Ev::CatalogAppend(l));
                        let _ = tx.send(Ev::Log(format!("載入：{name}（{n} 課）")));
                    }
                    _ => {}
                }
            }
            let _ = tx.send(Ev::Log("目錄載入完成".into()));
        });
    });
}

fn start_setup(tx: Sender<Ev>) {
    std::thread::spawn(move || {
        let tx_log = tx.clone();
        let res = speakdrill_core::setup::run_setup(
            &speakdrill_core::setup::SetupOptions::default(),
            &mut |s| {
                let _ = tx_log.send(Ev::Log(s));
            },
        );
        match res {
            Ok(()) => {
                let _ = tx.send(Ev::Log("（設定流程結束）".into()));
                let _ = tx.send(Ev::SetupDone);
            }
            Err(e) => {
                let _ = tx.send(Ev::Fatal(format!("設定失敗：{e}")));
            }
        }
    });
}

fn start_run(tx: Sender<Ev>, stop: Arc<AtomicBool>, lessons: Vec<(String, String)>, pass: u32) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            // 重置 stop flag，讓停止按鈕可重複使用
            stop.store(false, Ordering::Relaxed);
            let mut cfg = speakdrill_core::Config::load_or_default();
            cfg.pass_score = pass;
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
                    let _ = tx.send(Ev::Fatal(format!("音訊：{e}")));
                    return;
                }
            };

            for (lesson_id, course_id) in lessons {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let _ = tx.send(Ev::Log(format!("=== {} ===", lesson_id)));
                let mut p = ChannelProgress {
                    tx: tx.clone(),
                    stop: stop.clone(),
                };
                match speakdrill_core::run_lesson(&cdp, &cfg, &player, &lesson_id, &course_id, &RunOptions::default(), &mut p).await
                {
                    Ok(r) => {
                        let _ = save_report(&r);
                        let _ = tx.send(Ev::LessonDone(r));
                    }
                    Err(e) => {
                        let _ = tx.send(Ev::Log(format!("✗ {}：{e}", lesson_id)));
                    }
                }
            }
            let _ = tx.send(Ev::Done);
        });
    });
}

/// 把單一 lesson 標記完成並歸組進 coursebooks
fn group_lesson(coursebooks: &mut Vec<Coursebook>, completed: &mut HashSet<String>, l: &LessonRef) {
    if l.best_score.is_some() {
        completed.insert(l.lesson_id.clone());
    }
    // 按 course_id 前綴分組（例：BKM-B2A-001 → BKM-B2A）
    let parts: Vec<&str> = l.course_id.split('-').collect();
    let cb_key = if parts.len() >= 2 {
        format!("{}-{}", parts[0], parts[1])
    } else {
        l.course_id.clone()
    };
    let cb_name = speakdrill_core::config::KNOWN_CODENAMES
        .iter()
        .find(|(code, _)| code.replace('_', "-B") == cb_key)
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| cb_key.clone());
    if let Some(cb) = coursebooks.iter_mut().find(|(c, _, _)| c == &cb_key) {
        if !cb.2.iter().any(|x| x.lesson_id == l.lesson_id) {
            cb.2.push(l.clone());
        }
    } else {
        coursebooks.push((cb_key, cb_name, vec![l.clone()]));
    }
}

/// 分組；replace=true 時先清空重建（重載目錄用）
fn merge_catalog(app: &mut App, list: Vec<LessonRef>, replace: bool) {
    if replace {
        app.coursebooks.clear();
        app.lessons = list;
        for l in &app.lessons {
            group_lesson(&mut app.coursebooks, &mut app.completed, l);
        }
    } else {
        for l in &list {
            if !app.lessons.iter().any(|x| x.lesson_id == l.lesson_id) {
                app.lessons.push(l.clone());
            }
            group_lesson(&mut app.coursebooks, &mut app.completed, l);
        }
    }
    // 佇列保留索引，超出範圍的夾掉
    app.queue.retain(|&i| i < app.coursebooks.len());
    app.sel = app.sel.min(app.coursebooks.len().saturating_sub(1));
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.055, 0.055, 0.07, 1.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Ev::Catalog(l) => {
                    merge_catalog(self, l, true);
                    self.status = "已載入目錄".into();
                }
                Ev::CatalogAppend(l) => {
                    merge_catalog(self, l, false);
                }
                Ev::Log(s) => push_log(self, s),
                Ev::Line(s) => {
                    push_log(
                        self,
                        format!(
                            "  #{} {} 分（發音 {} 語調 {} 流利 {} 重音 {}）",
                            s.index, s.total, s.pronunciation, s.pitch, s.fluency, s.stress
                        ),
                    );
                    self.done += 1;
                    self.avg_sum += s.total as f64;
                    self.avg_n += 1;
                }
                Ev::LessonDone(r) => {
                    self.completed.insert(r.lesson_id.clone());
                    push_log(
                        self,
                        format!(
                            "✓ {} 平均 {}（{}/{} 句 / {} 秒）",
                            r.lesson_id, r.avg, r.lines.len(), r.total_lines, r.elapsed_secs
                        ),
                    );
                }
                Ev::Done => {
                    self.running = false;
                    let avg = if self.avg_n > 0 {
                        self.avg_sum / self.avg_n as f64
                    } else {
                        0.0
                    };
                    self.status = format!("完成：{} 句，平均 {:.1}", self.done, avg);
                }
                Ev::SetupDone => {
                    self.setup_busy = false;
                    self.status = "音訊設定完成（詳見紀錄）".into();
                }
                Ev::Fatal(e) => {
                    self.running = false;
                    self.status = e.clone();
                    push_log(self, format!("✗ {e}"));
                }
            }
        }

        // 把選中的課本（或佇列中的課本）展開成 lessons
        let expand = |app: &App, queued: bool| -> Vec<(String, String)> {
            let selected: Vec<usize> = if queued {
                app.queue.iter().copied().collect()
            } else {
                vec![app.sel]
            };
            let mut picked = Vec::new();
            for &idx in &selected {
                if let Some((_, _, lessons)) = app.coursebooks.get(idx) {
                    picked.extend(lessons.iter().map(|l| (l.lesson_id.clone(), l.course_id.clone())));
                }
            }
            picked
        };

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(format!("MyET 口說自動練習  v{}", env!("CARGO_PKG_VERSION")));
                ui.separator();

                if ui.add_enabled(!self.running, egui::Button::new("▶ 跑選取")).clicked() {
                    let picked = expand(self, false);
                    if picked.is_empty() {
                        push_log(self, "（沒有可跑的課程，請重載目錄）".into());
                    } else {
                        self.running = true;
                        start_run(self.tx.clone(), self.stop.clone(), picked, self.pass);
                    }
                }
                let qn = self.queue.len();
                if ui
                    .add_enabled(!self.running && qn > 0, egui::Button::new(format!("▶ 跑佇列（{qn}）")))
                    .clicked()
                {
                    let picked = expand(self, true);
                    if !picked.is_empty() {
                        self.running = true;
                        start_run(self.tx.clone(), self.stop.clone(), picked, self.pass);
                    }
                }
                if ui.add_enabled(qn > 0, egui::Button::new("✖ 清佇列")).clicked() {
                    self.queue.clear();
                    push_log(self, "（已清空佇列）".into());
                }
                if ui
                    .add_enabled(!self.running && !self.lessons.is_empty(), egui::Button::new("▶▶ 跑全部"))
                    .clicked()
                {
                    self.running = true;
                    let picked: Vec<(String, String)> = self
                        .lessons
                        .iter()
                        .map(|l| (l.lesson_id.clone(), l.course_id.clone()))
                        .collect();
                    start_run(self.tx.clone(), self.stop.clone(), picked, self.pass);
                }
                if ui.add_enabled(self.running, egui::Button::new("■ 停止")).clicked() {
                    self.stop.store(true, Ordering::Relaxed);
                    push_log(self, "… 請求中止".into());
                }
                ui.separator();
                if ui.add_enabled(!self.running, egui::Button::new("🔄 重新載入")).clicked() {
                    push_log(self, "... 重新載入目錄".into());
                    load_catalog(self.tx.clone(), true);
                }
                ui.separator();
                if ui.add_enabled(!self.setup_busy, egui::Button::new("🔧 自動設定音訊")).clicked() {
                    self.setup_busy = true;
                    push_log(self, "== 自動設定 per-app 音訊 ==".into());
                    start_setup(self.tx.clone());
                }
                if self.setup_busy {
                    ui.spinner();
                }
                ui.separator();
                ui.label("及格分");
                ui.add(egui::Slider::new(&mut self.pass, 50..=100));
            });
        });

        egui::SidePanel::left("coursebooks")
            .resizable(true)
            .default_width(360.0)
            .show(ctx, |ui| {
                ui.label(format!("課本（已勾 {}）— 勾選加入佇列", self.queue.len()));
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, (code, name, lessons)) in self.coursebooks.iter().enumerate() {
                        let total = lessons.len();
                        let done = lessons.iter().filter(|l| self.completed.contains(&l.lesson_id)).count();
                        let pct = if total > 0 { (done as f64 / total as f64 * 100.0) as u32 } else { 0 };
                        let queued = self.queue.contains(&i);
                        let mark = if queued { "● " } else { "   " };
                        let row = format!("{}{}  {}  {}/{} ({}%)", mark, code, name, done, total, pct);
                        ui.horizontal(|ui| {
                            let mut checked = queued;
                            if ui.checkbox(&mut checked, "").clicked() {
                                if checked {
                                    self.queue.insert(i);
                                } else {
                                    self.queue.remove(&i);
                                }
                            }
                            let color = if queued {
                                egui::Color32::from_rgb(255, 214, 10)
                            } else {
                                ui.style().visuals.text_color()
                            };
                            let label = egui::RichText::new(row).color(color).monospace();
                            if ui.selectable_label(i == self.sel, label).clicked() {
                                self.sel = i;
                            }
                        });
                    }
                });
            });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.separator();
                if self.avg_n > 0 {
                    ui.label(format!("完成 {} 句  平均 {:.1}", self.done, self.avg_sum / self.avg_n as f64));
                } else {
                    ui.label(format!("完成 {} 句", self.done));
                }
                if self.running {
                    ui.spinner();
                }
            });
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(egui::Color32::from_rgb(16, 18, 23)))
            .show(ctx, |ui| {
                ui.label("紀錄");
                egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                    for l in &self.logs {
                        ui.monospace(l);
                    }
                });
            });

        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
}

/// 推一行 log，紀錄上限 500 行
fn push_log(app: &mut App, s: String) {
    app.logs.push(s);
    if app.logs.len() > 500 {
        app.logs.remove(0);
    }
}

/// 載入系統 CJK 字型（egui 預設字型沒有中文字）
fn setup_cjk_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let candidates = [
        r"C:\Windows\Fonts\msjh.ttc",   // 微軟正黑體
        r"C:\Windows\Fonts\msyh.ttc",   // 微軟雅黑
        r"C:\Windows\Fonts\simsun.ttc", // 新細明體
    ];
    for path in candidates {
        if let Ok(data) = std::fs::read(path) {
            fonts
                .font_data
                .insert("cjk".to_owned(), egui::FontData::from_owned(data));
            if let Some(f) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                f.insert(0, "cjk".to_owned());
            }
            if let Some(f) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                f.push("cjk".to_owned());
            }
            break;
        }
    }
    ctx.set_fonts(fonts);
}

fn main() -> Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1340.0, 720.0])
            .with_min_inner_size([1280.0, 600.0])
            .with_title("MyET 口說自動練習").with_transparent(false),
        ..Default::default()
    };
    eframe::run_native(
        "speakdrill-gui",
        opts,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!(e.to_string()))
}
