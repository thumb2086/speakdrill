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
    MyetStats(speakdrill_core::MyetStats),
    /// MyET 官方累計時數讀取失敗（才退回顯示本機值）
    MyetStatsFailed,
    /// 課本清單 (code, 顯示名稱)；用於課本分組的顯示名
    Books(Vec<(String, String)>),
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
    /// 佇列：課本索引（Vec 保留勾選順序，與 TUI 相同語意）
    queue: Vec<usize>,
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
            queue: Vec::new(),
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
            total_secs: speakdrill_core::totals_get(),
            myet_hours: None,
            myet_level: None,
            myet_certs: None,
            myet_stats_failed: false,
            books_names: std::collections::HashMap::new(),
            pass: 80,
            setup_busy: false,
        };
        load_catalog(tx.clone(), false);
        load_myet_stats(tx.clone());
        app
    }
}

/// 背景讀 MyET 官方累計學習時數；失敗時保留本機累計
fn load_myet_stats(tx: Sender<Ev>) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            use tokio::time::Duration as TokioDuration;

            let cfg = speakdrill_core::Config::load_or_default();
            // 等 MyET 有 page target（最多 20 秒）
            for _ in 0..40 {
                tokio::time::sleep(TokioDuration::from_millis(500)).await;
                if Cdp::check_myet_running("127.0.0.1", cfg.cdp_port).await {
                    break;
                }
            }
            match speakdrill_core::fetch_myet_stats(&cfg.server, cfg.cdp_port, false).await {
                Ok(s) => {
                    let _ = tx.send(Ev::Log(format!("MyET 官方累計學習時間：{:.1} 小時", s.hours)));
                    let _ = tx.send(Ev::MyetStats(s));
                }
                Err(e) => {
                    let _ = tx.send(Ev::Log(format!("MyET 累計時數讀取失敗（用本機累計）：{e}")));
                    let _ = tx.send(Ev::MyetStatsFailed);
                }
            }
        });
    });
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

            // 從「我的課程」端點拿實際課本清單（含每本的 ContentType）
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
            // 先載入 config 指定的預設課本（快），其餘背景逐本 append
            let default_code = cfg.code.clone();
            if let Some((dcode, dtype, _dname)) = books.iter().find(|(c, _, _)| *c == default_code).cloned() {
                match speakdrill_core::lesson::fetch_catalog_opts(&cdp, &cfg.server, &dcode, &dtype, refresh).await {
                    Ok(l) if !l.is_empty() => {
                        let _ = tx.send(Ev::Log(format!("目錄載入：{} 課", l.len())));
                        let _ = tx.send(Ev::Catalog(l));
                    }
                    Ok(_) => {
                        let _ = tx.send(Ev::Log(format!("預設課本 {} 目錄是空的", dcode)));
                    }
                    Err(e) => {
                        let _ = tx.send(Ev::Fatal(format!("抓目錄失敗：{e}")));
                        return;
                    }
                }
            } else {
                let _ = tx.send(Ev::Log(format!("（找不到預設 CodeName {default_code}，直接背景載入全部）")));
            }
            for (code, contype, name) in books {
                if code == default_code {
                    continue;
                }
                match speakdrill_core::lesson::fetch_catalog_opts(&cdp, &cfg.server, &code, &contype, refresh).await {
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

/// 手動啟動 MyET（帶 CDP debug port）；已在執行就不重搶，啟動後自動重載目錄
fn start_myet(tx: Sender<Ev>) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            use tokio::time::Duration as TokioDuration;

            let cfg = speakdrill_core::Config::load_or_default();
            if Cdp::check_myet_running("127.0.0.1", cfg.cdp_port).await {
                let _ = tx.send(Ev::Log(format!("MyET 已在執行（CDP {} OK）", cfg.cdp_port)));
                return;
            }
            let myet_path = std::env::var("SPEAKDRILL_MYET_PATH")
                .unwrap_or_else(|_| r"C:\Program Files (x86)\LLabs\MyET10\MyET10.exe".to_string());
            if !std::path::Path::new(&myet_path).exists() {
                let _ = tx.send(Ev::Log("找不到 MyET 執行檔".into()));
                return;
            }
            let port_str = cfg.cdp_port.to_string();
            let _ = std::process::Command::new(&myet_path)
                .args([
                    format!("--remote-debugging-port={}", port_str),
                    "--remote-allow-origins=*".to_string(),
                ])
                .spawn();
            let _ = tx.send(Ev::Log(format!("正在啟動 MyET（debug port {}）…", port_str)));
            for _ in 0..20 {
                tokio::time::sleep(TokioDuration::from_secs(1)).await;
                if Cdp::check_myet_running("127.0.0.1", cfg.cdp_port).await {
                    let _ = tx.send(Ev::Log("MyET 已連線，重新載入目錄…".into()));
                    load_catalog(tx.clone(), false);
                    return;
                }
            }
            let _ = tx.send(Ev::Log("MyET 已啟動，但 CDP 未連上".into()));
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

/// 安裝 VB-CABLE 虛擬音效驅動；需管理員權限，安裝後需重開機
fn start_install_driver(tx: Sender<Ev>) {
    std::thread::spawn(move || {
        let tx_log = tx.clone();
        let _ = tx_log.send(Ev::Log("== 安裝 VB-CABLE 虛擬音效驅動 ==".into()));
        let _ = tx_log.send(Ev::Log("需要管理員權限（UAC），安裝後必須重開機才能生效".into()));
        let res = speakdrill_core::devices::install_vb_cable(&mut |m| {
            let _ = tx_log.send(Ev::Log(m.to_string()));
        });
        match res {
            Ok(()) => {
                let _ = tx_log.send(Ev::Log("✓ VB-CABLE 安裝完成".into()));
                let _ = tx_log.send(Ev::Log("⚠ 安裝後必須重開機才能生效（Windows 核心驅動限制）".into()));
                let _ = tx_log.send(Ev::Log("重開機後請再跑一次 `speakdrill init`".into()));
            }
            Err(e) => {
                let _ = tx_log.send(Ev::Log(format!("⚠ 安裝失敗：{e}")));
                let _ = tx_log.send(Ev::Log("請自行從 https://vb-audio.com/Cable/ 下載並安裝".into()));
            }
        }
    });
}

/// 把選中的課本索引展開為待跑課程（保留勾選順序，略過已完成）
fn expand_coursebooks(app: &App, selected: &[usize]) -> (Vec<LessonRef>, usize) {
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

/// 課程的人類可讀標籤：單元名稱優先，id 當後括號；沒有單元名就只印 id
fn lesson_label(id: &str, title: &str) -> String {
    let t = title.trim();
    if t.is_empty() || t == id {
        id.to_string()
    } else {
        format!("{t}（{id}）")
    }
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

            for l in lessons {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let lesson_id = l.lesson_id.clone();
                let course_id = l.course_id.clone();
                let code = l.code.clone();
                let contype = l.contype.clone();
                let _ = tx.send(Ev::Log(format!("▶ {}", lesson_label(&lesson_id, &l.title))));
                let mut p = ChannelProgress {
                    tx: tx.clone(),
                    stop: stop.clone(),
                };
                match speakdrill_core::run_lesson(&cdp, &cfg, &player, &lesson_id, &course_id, &code, &contype, &RunOptions::default(), &mut p).await
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
fn group_lesson(
    coursebooks: &mut Vec<Coursebook>,
    completed: &mut HashSet<String>,
    books_names: &std::collections::HashMap<String, String>,
    l: &LessonRef,
) {
    if l.done() {
        completed.insert(l.lesson_id.clone());
    }
    // 按 course_id 前綴分組（例：BKM-B2A-001 → BKM-B2A）
    let parts: Vec<&str> = l.course_id.split('-').collect();
    let cb_key = if parts.len() >= 2 {
        format!("{}-{}", parts[0], parts[1])
    } else {
        l.course_id.clone()
    };
    let cb_name = books_names.get(&l.code).cloned().unwrap_or_else(|| cb_key.clone());
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
            group_lesson(&mut app.coursebooks, &mut app.completed, &app.books_names, l);
        }
    } else {
        for l in &list {
            if !app.lessons.iter().any(|x| x.lesson_id == l.lesson_id) {
                app.lessons.push(l.clone());
            }
            group_lesson(&mut app.coursebooks, &mut app.completed, &app.books_names, l);
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
                Ev::MyetStats(s) => {
                    self.myet_hours = Some(s.hours);
                    self.myet_level = s.level;
                    self.myet_certs = s.certs;
                }
                Ev::MyetStatsFailed => {
                    self.myet_stats_failed = true;
                }
                Ev::Books(list) => {
                    self.books_names = list.into_iter().collect();
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
                    self.total_secs += r.elapsed_secs;
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
                    // 追加課本不自動接跑：保留在佇列，按「▶ 跑佇列」才開始
                    if !self.queue.is_empty() {
                        push_log(
                            self,
                            format!("已追加 {} 本課本 — 按「▶ 跑佇列」開始（或「✖ 清佇列」清空）", self.queue.len()),
                        );
                        self.status = format!("完成 · 待跑佇列 {} 本（按「▶ 跑佇列」開始）", self.queue.len());
                    }
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

        // 把選中的課本（或佇列中的課本）展開成 lessons；已完成的課自動略過
        let expand = |app: &App, queued: bool| -> (Vec<LessonRef>, usize) {
            let selected: Vec<usize> = if queued {
                app.queue.clone()
            } else {
                vec![app.sel]
            };
            expand_coursebooks(app, &selected)
        };

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(format!("MyET 口說自動練習  v{}", env!("CARGO_PKG_VERSION")));
                ui.separator();

                if ui.add_enabled(!self.running, egui::Button::new("📋 課本內容")).clicked() {
                    if let Some((code, name, lessons)) = self.coursebooks.get(self.sel) {
                        let mut lines = vec![format!("── {} {} 課程列表（{}）──", code, name, lessons.len())];
                        for l in lessons {
                            let mark = if self.completed.contains(&l.lesson_id) { "✓" } else { " " };
                            let title = l.title.trim();
                            let row = if title.is_empty() {
                                format!("  {} {}", mark, l.lesson_id)
                            } else {
                                format!("  {} {}（{}）", mark, title, l.lesson_id)
                            };
                            lines.push(row);
                        }
                        lines.push("── ▶ 跑選取 開始 ──".to_string());
                        for l in lines {
                            push_log(self, l);
                        }
                    }
                }
                if ui.add_enabled(!self.running, egui::Button::new("▶ 跑選取")).clicked() {
                    let (picked, skipped) = expand(self, false);
                    if skipped > 0 {
                        push_log(self, format!("略過已完成 {} 課", skipped));
                    }
                    if picked.is_empty() {
                        push_log(self, "（選取的課本都已完成；要全部重跑按「▶▶ 跑全部」）".into());
                    } else {
                        self.running = true;
                        start_run(self.tx.clone(), self.stop.clone(), picked.clone(), self.pass);
                    }
                }
                let qn = self.queue.len();
                if ui
                    .add_enabled(!self.running && qn > 0, egui::Button::new(format!("▶ 跑佇列（{qn}）")))
                    .clicked()
                {
                    let (picked, skipped) = expand(self, true);
                    if skipped > 0 {
                        push_log(self, format!("略過已完成 {} 課（佇列展開 {} 課）", skipped, picked.len() + skipped));
                    }
                    if picked.is_empty() {
                        push_log(self, "（佇列中的課本都已完成；要全部重跑按「▶▶ 跑全部」）".into());
                    } else {
                        self.running = true;
                        self.queue.clear();
                        start_run(self.tx.clone(), self.stop.clone(), picked.clone(), self.pass);
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
                    let picked = self.lessons.clone();
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
                if ui.add_enabled(!self.running, egui::Button::new("🔁 重新整理時數")).clicked() {
                    push_log(self, "... 強制重新整理 MyET 官方時數".into());
                    let tx = self.tx.clone();
                    let cfg = speakdrill_core::Config::load_or_default();
                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        rt.block_on(async move {
                            match speakdrill_core::fetch_myet_stats(&cfg.server, cfg.cdp_port, true).await {
                                Ok(s) => {
                                    let _ = tx.send(Ev::Log(format!("MyET 官方累計學習時間：{:.1} 小時", s.hours)));
                                    let _ = tx.send(Ev::MyetStats(s));
                                }
                                Err(e) => {
                                    let _ = tx.send(Ev::Log(format!("MyET 累計時數讀取失敗：{e}")));
                                    let _ = tx.send(Ev::MyetStatsFailed);
                                }
                            }
                        });
                    });
                }
                if ui.add_enabled(!self.running, egui::Button::new("🚀 啟動 MyET")).clicked() {
                    push_log(self, "... 啟動 MyET（debug port 9222）".into());
                    start_myet(self.tx.clone());
                }
                if ui.add_enabled(!self.running, egui::Button::new("🔧 安裝虛擬音效驅動")).clicked() {
                    push_log(self, "... 安裝 VB-CABLE（需管理員、重開機）".into());
                    start_install_driver(self.tx.clone());
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
                ui.separator();
                let vroute = if speakdrill_core::devices::has_vb_cable() {
                    "虛擬迴路：VB-CABLE ✓"
                } else {
                    "虛擬迴路：未偵測（點上方按鈕安裝）"
                };
                ui.label(vroute);
            });
        });

        egui::SidePanel::left("coursebooks")
            .resizable(true)
            .default_width(360.0)
            .show(ctx, |ui| {
                let qlabel = if !self.queue.is_empty() {
                    let order = self
                        .queue
                        .iter()
                        .map(|&i| self.coursebooks.get(i).map(|c| c.0.as_str()).unwrap_or("?"))
                        .collect::<Vec<_>>()
                        .join(" → ");
                    format!("課本 · 佇列 {} 本：{}", self.queue.len(), order)
                } else if self.running {
                    "課本（執行中 — 勾選即追加，本批結束按「▶ 跑佇列」開始）".to_string()
                } else {
                    "課本 — 勾選加入佇列，按「▶ 跑佇列」開始".to_string()
                };
        ui.label(qlabel);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, (code, name, lessons)) in self.coursebooks.iter().enumerate() {
                        let total = lessons.len();
                        let done = lessons.iter().filter(|l| self.completed.contains(&l.lesson_id)).count();
                        let pct = if total > 0 { (done as f64 / total as f64 * 100.0) as u32 } else { 0 };
                        let queued = self.queue.contains(&i);
                        let pos = self.queue.iter().position(|&x| x == i).map(|p| p + 1);
                        let mark = pos.map(|p| format!("●#{p} ")).unwrap_or_else(|| "    ".to_string());
                        let row = format!("{}{}  {}  {}/{} ({}%)", mark, code, name, done, total, pct);
                        ui.horizontal(|ui| {
                            let mut checked = queued;
                            if ui.checkbox(&mut checked, "").clicked() {
                                if checked {
                                    if !self.queue.contains(&i) {
                                        self.queue.push(i);
                                    }
                                } else {
                                    self.queue.retain(|&x| x != i);
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
                let acc = match &self.myet_hours {
                    Some(h) => format!("MyET 累計 {:.1} 小時", h),
                    None if self.myet_stats_failed => {
                        format!("累計 {:.1} 小時（本機）", self.total_secs as f64 / 3600.0)
                    }
                    None => "累計 …".to_string(),
                };
                if self.avg_n > 0 {
                    ui.label(format!(
                        "完成 {} 句  平均 {:.1}  {}",
                        self.done,
                        self.avg_sum / self.avg_n as f64,
                        acc
                    ));
                } else {
                    ui.label(format!("完成 {} 句  {}", self.done, acc));
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
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show_rows(ui, 18.0, self.logs.len(), |ui, row_range| {
                        for i in row_range {
                            ui.monospace(&self.logs[i]);
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
