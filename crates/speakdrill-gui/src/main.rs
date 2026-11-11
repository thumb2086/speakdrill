//! speakdrill-gui — 原生 egui 介面（無 webview）

use anyhow::Result;
use speakdrill_core::{fetch_catalog, run_lesson, save_report, AudioPlayer, Cdp, LessonRef, Progress, RunOptions};
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
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        cc.egui_ctx.set_style(style);

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

fn start_run(tx: Sender<Ev>, stop: Arc<AtomicBool>, lessons: Vec<LessonRef>, pass: u32) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Ev::Catalog(l) => {
                    self.logs.push(format!("目錄：{} 課", l.len()));
                    self.lessons = l;
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
                if ui.add_enabled(!self.running, egui::Button::new("▶▶ 跑全部")).clicked() {
                    self.running = true;
                    start_run(self.tx.clone(), self.stop.clone(), self.lessons.clone(), self.pass);
                }
                if ui.add_enabled(self.running, egui::Button::new("■ 停止")).clicked() {
                    self.stop.store(true, Ordering::Relaxed);
                    self.logs.push("… 請求中止".into());
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
                    if ui.selectable_label(i == self.sel, &l.lesson_id).clicked() {
                        self.sel = i;
                    }
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

        egui::CentralPanel::default().show(ctx, |ui| {
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

fn main() -> Result<()> {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 680.0])
            .with_title("MyET 口說自動練習"),
        ..Default::default()
    };
    eframe::run_native(
        "speakdrill-gui",
        opts,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI 啟動失敗：{e}"))
}

