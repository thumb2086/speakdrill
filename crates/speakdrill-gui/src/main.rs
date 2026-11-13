//! speakdrill-gui — 原生 egui 介面（無 webview）

use anyhow::Result;
use speakdrill_core::{fetch_catalog, run_lesson, save_report, AudioPlayer, Cdp, LessonRef, Progress, RunOptions};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

#[derive(Debug, Clone)]
enum Ev {
    Catalog(Vec<LessonRef>),
    Log(String),
    Line(speakdrill_core::LineScore),
    LessonDone(speakdrill_core::LessonReport),
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

struct App {
    lessons: Vec<LessonRef>,
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
    /// 佇列（以 lesson_id 為鍵，catalog 重抓後仍然有效）
    queue: HashSet<String>,
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
            sel: 0,
            logs: vec!["按「載入目錄」開始".into()],
            running: false,
            stop: stop.clone(),
            tx: tx.clone(),
            rx,
            status: String::from("閒置"),
            done: 0,
            avg_sum: 0.0,
            avg_n: 0,
            pass: 80,
            setup_busy: false,
            queue: HashSet::new(),
        };
        load_catalog(tx.clone(), stop);
        app
    }
}

fn load_catalog(tx: Sender<Ev>, _stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let cfg = speakdrill_core::Config::load_or_default();
            match Cdp::connect("127.0.0.1", cfg.cdp_port).await {
                Ok(cdp) => match fetch_catalog(&cdp, &cfg.server, &cfg.code, &cfg.content_type).await {
                    Ok(l) => { let _ = tx.send(Ev::Catalog(l)); }
                    Err(e) => { let _ = tx.send(Ev::Fatal(format!("抓目錄失敗：{e}"))); }
                },
                Err(e) => { let _ = tx.send(Ev::Fatal(format!("連不上 MyET：{e}"))); }
            }
        });
    });
}

fn start_setup(tx: Sender<Ev>) {
    std::thread::spawn(move || {
        let tx_log = tx.clone();
        let res = speakdrill_core::setup::run_setup(
            &speakdrill_core::setup::SetupOptions::default(),
            &mut |s| { let _ = tx_log.send(Ev::Log(s)); },
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

fn start_run(tx: Sender<Ev>, stop: Arc<AtomicBool>, lessons: Vec<LessonRef>, pass: u32) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            // 重置 stop flag，讓停止按鈕可重複使用
            stop.store(false, Ordering::Relaxed);
            let mut cfg = speakdrill_core::Config::load_or_default();
            cfg.pass_score = pass;
            let cdp = match Cdp::connect("127.0.0.1", cfg.cdp_port).await {
                Ok(c) => c,
                Err(e) => { let _ = tx.send(Ev::Fatal(format!("連不上 MyET：{e}"))); return; }
            };
            if let Err(e) = speakdrill_core::ensure_logged_in(&cdp, &cfg.server).await {
                let _ = tx.send(Ev::Fatal(format!("{e}")));
                return;
            }
            let player = match AudioPlayer::new(&cfg.play_device) {
                Ok(p) => p,
                Err(e) => { let _ = tx.send(Ev::Fatal(format!("音訊：{e}"))); return; }
            };

            for l in lessons {
                if stop.load(Ordering::Relaxed) { break; }
                let _ = tx.send(Ev::Log(format!("=== {} ===", l.lesson_id)));
                let mut p = ChannelProgress { tx: tx.clone(), stop: stop.clone() };
                match run_lesson(&cdp, &cfg, &player, &l.lesson_id, &l.course_id, &RunOptions::default(), &mut p).await {
                    Ok(r) => { let _ = save_report(&r); let _ = tx.send(Ev::LessonDone(r)); }
                    Err(e) => { let _ = tx.send(Ev::Log(format!("✗ {}：{e}", l.lesson_id))); }
                }
            }
            let _ = tx.send(Ev::Done);
        });
    });
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.055, 0.055, 0.07, 1.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Ev::Catalog(l) => {
                    self.logs.push(format!("目錄：{} 課", l.len()));
                    let keep: HashSet<String> = l.iter().map(|x| x.lesson_id.clone()).collect();
                    self.queue.retain(|id| keep.contains(id));
                    self.lessons = l;
                    self.sel = self.sel.min(self.lessons.len().saturating_sub(1));
                    self.status = "已載入目錄".into();
                }
                Ev::Log(s) => self.logs.push(s),
                Ev::Line(s) => {
                    self.logs.push(format!(
                        "  #{} {} 分（發音 {} 語調 {} 流利 {} 重音 {}）",
                        s.index, s.total, s.pronunciation, s.pitch, s.fluency, s.stress
                    ));
                    self.done += 1;
                    self.avg_sum += s.total as f64;
                    self.avg_n += 1;
                }
                Ev::LessonDone(r) => self.logs.push(format!(
                    "✓ {} 平均 {}（{}/{} 句 / {} 秒）", r.lesson_id, r.avg, r.lines.len(), r.total_lines, r.elapsed_secs
                )),
                Ev::Done => {
                    self.running = false;
                    let avg = if self.avg_n > 0 { self.avg_sum / self.avg_n as f64 } else { 0.0 };
                    self.status = format!("完成：{} 句，平均 {:.1}", self.done, avg);
                }
                Ev::SetupDone => {
                    self.setup_busy = false;
                    self.status = "音訊設定完成（詳見紀錄）".into();
                }
                Ev::Fatal(e) => {
                    self.running = false;
                    self.status = e.clone();
                    self.logs.push(format!("✗ {e}"));
                }
            }
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("MyET 口說自動練習");
                ui.separator();
                if ui.add_enabled(!self.running, egui::Button::new("▶ 跑選取")).clicked() {
                    if let Some(l) = self.lessons.get(self.sel).cloned() {
                        self.running = true;
                        start_run(self.tx.clone(), self.stop.clone(), vec![l], self.pass);
                    }
                }
                let qn = self.queue.len();
                if ui.add_enabled(!self.running && qn > 0, egui::Button::new(format!("▶ 跑佇列（{qn}）"))).clicked() {
                    let picked: Vec<LessonRef> = self
                        .lessons
                        .iter()
                        .filter(|l| self.queue.contains(&l.lesson_id))
                        .cloned()
                        .collect();
                    if !picked.is_empty() {
                        self.running = true;
                        start_run(self.tx.clone(), self.stop.clone(), picked, self.pass);
                    }
                }
                if ui.add_enabled(qn > 0, egui::Button::new("✖ 清佇列")).clicked() {
                    self.queue.clear();
                    self.logs.push("（已清空佇列）".into());
                }
                if ui.add_enabled(!self.running, egui::Button::new("▶▶ 跑全部")).clicked() {
                    self.running = true;
                    start_run(self.tx.clone(), self.stop.clone(), self.lessons.clone(), self.pass);
                }
                if ui.add_enabled(self.running, egui::Button::new("■ 停止")).clicked() {
                    self.stop.store(true, Ordering::Relaxed);
                    self.logs.push("… 請求中止".into());
                }
                ui.separator();
                if ui.add_enabled(!self.setup_busy, egui::Button::new("🔧 自動設定音訊")).clicked() {
                    self.setup_busy = true;
                    self.logs.push("== 自動設定 per-app 音訊 ==".into());
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

        egui::SidePanel::left("list").resizable(true).default_width(260.0).show(ctx, |ui| {
            ui.label("課程");
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (i, l) in self.lessons.iter().enumerate() {
                    let mut queued = self.queue.contains(&l.lesson_id);
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut queued, "").clicked() {
                            if queued {
                                self.queue.insert(l.lesson_id.clone());
                            } else {
                                self.queue.remove(&l.lesson_id);
                            }
                        }
                        let label = l.label();
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
            .with_inner_size([1000.0, 680.0])
            .with_title("MyET 口說自動練習").with_transparent(false),
        ..Default::default()
    };
    eframe::run_native(
        "speakdrill-gui",
        opts,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI 啟動失敗：{e}"))
}

