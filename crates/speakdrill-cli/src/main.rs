//! speakdrill — speakdrill 口說自動練習 CLI / TUI

use anyhow::Result;
use clap::{Parser, Subcommand};
use speakdrill_core::{
    audio::list_outputs, config::{cache_dir, config_path, Config},
    ensure_logged_in, fetch_catalog, run_lesson, save_report, AudioPlayer, Cdp, LessonRef, LessonReport,
    RunOptions, SilentProgress, Vm,
};

mod tui;

#[derive(Parser)]
#[command(name = "speakdrill", version, about = "MyET 口說練習自動化：TTS 驅動、VoiceMeeter 迴路、分數報告")]
struct Cli {
    /// 不給子命令時直接進 TUI
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// 一次性設定：偵測裝置、寫設定、設 VoiceMeeter 路由與音量
    Init {
        #[arg(long)] device: Option<String>,
        #[arg(long)] server: Option<String>,
        #[arg(long)] code: Option<String>,
        /// 找不到虛擬音效裝置時，自動下載並安裝 VB-CABLE（會跳 UAC）
        #[arg(long)] install_driver: bool,
    },
    /// 檢查環境（port / 登入 / 裝置 / VoiceMeeter / TTS）
    Doctor,
    /// 還原：停止把 TTS 送到 B1，並印出如何清掉 per-app 裝置設定
    Reset {
        #[arg(long)] yes: bool,
    },
    /// 跑一課
    Run {
        #[arg(long)] lesson: String,
        #[arg(long)] course: String,
        #[arg(long)] resume: bool,
        #[arg(long, default_value_t = 80)] pass: u32,
    },
    /// 批次跑整個 CodeName 的所有課程
    Batch {
        /// 跳過已有成績的課程
        #[arg(long)] skip_completed: bool,
        #[arg(long)] code: Option<String>,
        #[arg(long)] content_type: Option<String>,
        #[arg(long)] limit: Option<usize>,
        #[arg(long)] start: Option<usize>,
        #[arg(long, default_value_t = 80)] pass: u32,
        #[arg(long)] resume: bool,
    },
    /// 跑所有 CodeName 的所有課程
    All {
        /// 跳過已有成績的課程
        #[arg(long)] skip_completed: bool,
        #[arg(long)] content_type: Option<String>,
        #[arg(long)] limit: Option<usize>,
        #[arg(long)] start: Option<usize>,
        #[arg(long, default_value_t = 80)] pass: u32,
        #[arg(long)] resume: bool,
    },
    /// 列出目錄中的課程（含名稱）
    List {
        #[arg(long)] code: Option<String>,
        /// 忽略快取，重新抓取
        #[arg(long)] refresh: bool,
    },
    /// 終端機介面
    Tui,
    /// 自動設定目標 App 的每應用程式麥克風；可加上 --set-output 同時設定輸出裝置
    /// 流程：先讀 registry 確認 → COM API 直接設定 → 失敗則用 UI Automation 操作 Windows 設定
    Setup {
        /// 同時設定 App 的輸出裝置（例如 "Voicemeeter AUX Input"）
        #[arg(long)]
        set_output: Option<String>,
        /// 只預覽要做的步驟，不實際寫入
        #[arg(long)]
        dry_run: bool,
        /// 不使用 UI Automation（COM 失敗時直接顯示手動步驟）
        #[arg(long)]
        no_uia: bool,
        /// 即使 registry 顯示已綁定，也重新套用設定
        #[arg(long)]
        force: bool,
        /// 「設定」視窗沒有回應時，先關閉再重新開啟（僅限你明確想自動重開時使用）
        #[arg(long)]
        restart_settings: bool,
    },
}

extern "system" {
    fn SetConsoleOutputCP(cp: u32) -> i32;
}

fn main() -> Result<()> {
    #[cfg(windows)]
    unsafe {
        SetConsoleOutputCP(65001);
    }
    let cli = Cli::parse();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let cmd = cli.cmd.unwrap_or(Cmd::Tui);
    match cmd {
        Cmd::Init { device, server, code, install_driver } => {
            rt.block_on(cmd_init(device, server, code, install_driver))
        }
        Cmd::Doctor => rt.block_on(cmd_doctor()),
        Cmd::Reset { yes } => rt.block_on(cmd_reset(yes)),
        Cmd::Run { lesson, course, resume, pass } => {
            rt.block_on(cmd_run(lesson, course, resume, pass))
        }
        Cmd::Batch { code, content_type, limit, start, pass, resume, skip_completed } => {
            rt.block_on(cmd_batch(code, content_type, limit, start, pass, resume, skip_completed))
        }
        Cmd::All { content_type, limit, start, pass, resume, skip_completed } => {
            rt.block_on(cmd_all(content_type, limit, start, pass, resume, skip_completed))
        }
        Cmd::List { code, refresh } => rt.block_on(cmd_list(code, refresh)),
        Cmd::Tui => tui::run(rt),
        Cmd::Setup { set_output, dry_run, no_uia, force, restart_settings } => {
            rt.block_on(cmd_setup(set_output, dry_run, no_uia, force, restart_settings))
        }
    }
}

// ---------------- init ----------------

async fn cmd_init(
    device: Option<String>,
    server: Option<String>,
    code: Option<String>,
    install_driver: bool,
) -> Result<()> {
    println!("== speakdrill init ==");
    let mut cfg = Config::load_or_default();
    if let Some(d) = device { cfg.play_device = d; }
    if let Some(s) = server { cfg.server = s; }
    if let Some(c) = code { cfg.code = c; }

    println!("\n[1/5] 輸出裝置");
    let outs = list_outputs();
    for o in &outs {
        let mark = if o.to_lowercase().contains(&cfg.play_device.to_lowercase()) { " <== 選用" } else { "" };
        println!("  {o}{mark}");
    }
    if !outs.iter().any(|o| o.to_lowercase().contains(&cfg.play_device.to_lowercase())) {
        println!("  ⚠ 找不到「{}」，請用 --device 指定", cfg.play_device);
    }

    println!("\n[2/5] TTS 引擎");
    match speakdrill_core::piper::ensure_piper(&mut |m| println!("  {m}")) {
        Ok(p) => {
            cfg.tts.engine = speakdrill_core::TtsEngine::Piper {
                exe: p.exe.display().to_string(),
                model: p.model.display().to_string(),
            };
            println!("  piper: {}", p.exe.display());
            println!("  voice: {}", p.model.display());
        }
        Err(e) => println!("  ⚠ 安裝 piper 失敗：{e}"),
    }

    if install_driver && !speakdrill_core::devices::has_vb_cable() {
        println!("  嘗試安裝 VB-CABLE…");
        if let Err(e) = speakdrill_core::devices::install_vb_cable(&mut |m| println!("  {m}")) {
            println!("  ⚠ 安裝失敗：{e}");
        }
    }

    println!("\n[3/5] 虛擬音效迴路");
    let mut chosen: Option<speakdrill_core::devices::VirtualRoute> = None;
    for r in speakdrill_core::devices::routes() {
        match AudioPlayer::new(&r.play_device) {
            Ok(p) => {
                println!("  使用 {}：", r.kind());
                println!("    播放 → {}", p.device_name);
                println!("    App 麥克風 → {}", r.mic_device);
                chosen = Some(r);
                break;
            }
            Err(e) => {
                println!("  {} 無法開啟（{}），換下一個", r.kind(), e);
            }
        }
    }

    if speakdrill_core::devices::has_vb_cable() && !cfg.play_device.to_lowercase().contains("cable") {
        println!("  提示：VB-CABLE 已安裝但開不起來，重開機後再跑一次 init 就會切過去");
    }

    match chosen {
        Some(r) => {
            cfg.play_device = r.play_device.clone();
            if r.needs_vm {
                if let Ok(vm) = Vm::connect() {
                    let (strip, bus) = cfg
                        .vm_strip
                        .zip(cfg.vm_bus)
                        .unwrap_or_else(|| vm.default_route());
                    if let Err(e) = vm.route_to_b1(strip) {
                        println!("    VoiceMeeter 路由設定失敗：{e}");
                    } else {
                        let _ = vm.set(&format!("bus[{bus}].mute"), 0.0);
                        cfg.vm_strip = Some(strip);
                        cfg.vm_bus = Some(bus);
                        println!("    VoiceMeeter 路由：strip[{strip}] → bus[{bus}]");
                    }
                } else {
                    println!("    VoiceMeeter 未偵測到，跳過路由設定");
                }
                cfg.auto_vm_setup = true;
            } else {
                cfg.vm_strip = None;
                cfg.vm_bus = None;
                cfg.auto_vm_setup = false;
            }
        }
        None => {
            println!("  ⚠ 沒有可用的虛擬音效迴路");
            println!("    加 --install-driver 安裝 VB-CABLE，或自行安裝 VoiceMeeter");
            println!("    若已安裝但被其他程式佔用，請重開機後再跑 init");
        }
    }

    println!("\n[4/5] Windows 裝置音量");
    #[cfg(windows)]
    {
        match speakdrill_core::winvol::set_volume("Voicemeeter Out B1", true, 1.0) {
            Ok(n) => println!("  錄音裝置 B1：{n} 個已設 100%"),
            Err(e) => println!("  ⚠ B1 音量：{e}"),
        }
        match speakdrill_core::winvol::set_volume(&cfg.play_device, false, 1.0) {
            Ok(n) => println!("  播放裝置：{n} 個已設 100%"),
            Err(e) => println!("  ⚠ 播放音量：{e}"),
        }
    }
    #[cfg(not(windows))]
    println!("  非 Windows，略過");

    cfg.save()?;
    println!("\n[5/5] 設定已寫入 {}", config_path().display());

    println!("\n{}", "─".repeat(64));
    println!("還剩一步（Windows 只允許用介面設定）：");
    println!("  1. Win+R → ms-settings:apps-volume");
    println!("  2. 找到「English-Chinese-Japanese」（就是 MyET）並展開");
    println!("  3. 輸入裝置 → 選「Voicemeeter Out B1」");
    println!("  （輸出裝置可留原值；全域預設麥克風不用改）");
    println!("{}", "─".repeat(64));
    Ok(())
}

// ---------------- doctor ----------------

async fn cmd_doctor() -> Result<()> {
    let cfg = Config::load_or_default();
    println!("== speakdrill doctor ==\n設定檔：{}", config_path().display());

    let mut bad = 0;

    print!("[CDP] port {} … ", cfg.cdp_port);
    match Cdp::connect("127.0.0.1", cfg.cdp_port).await {
        Ok(cdp) => {
            println!("OK  ({})", cdp.target_url);
            print!("[登入] … ");
            match ensure_logged_in(&cdp, &cfg.server).await {
                Ok(_) => println!("OK"),
                Err(e) => { println!("✗ {e}"); bad += 1; }
            }
        }
        Err(e) => { println!("✗ {e}"); bad += 1; }
    }

    print!("[迴路] … ");
    {
        let routes = speakdrill_core::devices::routes();
        let active = routes
            .iter()
            .find(|r| r.play_device == cfg.play_device)
            .cloned();
        match active {
            Some(r) => println!(
                "{}（播放 → {}／麥克風 → {}）",
                r.kind(),
                r.play_device,
                r.mic_device
            ),
            None => {
                if routes.is_empty() {
                    println!("✗ 找不到虛擬音效裝置（跑 speakdrill init --install-driver）");
                    bad += 1;
                } else {
                    println!(
                        "⚠ 設定用「{}」，但目前偵測到 {}（跑 speakdrill init 重設）",
                        cfg.play_device,
                        routes
                            .iter()
                            .map(|r| r.kind())
                            .collect::<Vec<_>>()
                            .join(" / ")
                    );
                }
            }
        }
    }

    print!("[App 麥克風] … ");
    {
        use speakdrill_core::devices;
        let route = devices::routes()
            .into_iter()
            .find(|r| r.play_device == cfg.play_device);
        match route {
            Some(r) => {
                let expected = speakdrill_core::winvol::find_endpoint_id(&r.mic_device, true);
                match speakdrill_core::binding::app_mic_binding(&cfg.app_exe) {
                    Some(b) => {
                        let ok = expected
                            .as_deref()
                            .map(|e| b.any_matches(e))
                            .unwrap_or(false);
                        if ok {
                            println!("OK（{} 已綁定 {}）", cfg.app_exe, r.mic_device)
                        } else {
                            println!(
                                "✗ {} 的 per-app 麥克風未綁定我們播到的迴路（{}）",
                                cfg.app_exe,
                                r.mic_device
                            );
                            if let Some(g) = b.best_guid() {
                                println!("      目前綁定：{}", g);
                            }
                            println!(
                                "      修正：ms-settings:apps-volume → {} → 輸入裝置選「{}」",
                                cfg.app_exe, r.mic_device
                            );
                            bad += 1;
                        }
                    }
                    None => {
                        println!("✗ 找不到 {} 的 per-app 設定", cfg.app_exe);
                        println!(
                            "      請到 ms-settings:apps-volume 把輸入裝置設成「{}」",
                            r.mic_device
                        );
                        bad += 1;
                    }
                }
            }
            None => println!("（沒有可對應的迴路，略過）"),
        }
    }

    print!("[輸出裝置] … ");
    match AudioPlayer::new(&cfg.play_device) {
        Ok(p) => println!("OK  {} @ {} Hz", p.device_name, p.sample_rate),
        Err(e) => { println!("✗ {e}"); bad += 1; }
    }

    let active_route = speakdrill_core::devices::routes()
        .into_iter()
        .find(|r| r.play_device == cfg.play_device);
    if let Some(r) = active_route.as_ref() {
        if r.needs_vm {
            print!("[VoiceMeeter] … ");
            match Vm::connect() {
                Ok(vm) => println!("OK  {}", vm.kind()),
                Err(e) => { println!("✗ {e}"); bad += 1; }
            }
        } else {
            println!("[VoiceMeeter] … （此路線不需要 VoiceMeeter）");
        }

        #[cfg(windows)]
        {
            print!("[裝置音量] … ");
            match speakdrill_core::winvol::list_endpoints() {
                Ok(list) => {
                    let mic = list.iter().find(|(k, n, _)| {
                        k == "capture" && n.to_lowercase().contains(&r.mic_device.to_lowercase())
                    });
                    let play = list.iter().find(|(k, n, _)| {
                        k == "render" && n.to_lowercase().contains(&r.play_device.to_lowercase())
                    });
                    match (mic, play) {
                        (Some((_, mn, mv)), Some((_, pn, pv))) => println!(
                            "mic {} = {:.1}%, play {} = {:.1}%{}",
                            mn,
                            mv * 100.0,
                            pn,
                            pv * 100.0,
                            if *mv < 0.9 || *pv < 0.9 {
                                "  ⚠ 太低，請跑 speakdrill init"
                            } else {
                                ""
                            }
                        ),
                        (None, _) => { println!("找不到麥克風「{}」", r.mic_device); bad += 1; }
                        (_, None) => { println!("找不到播放裝置「{}」", r.play_device); bad += 1; }
                    }
                }
                Err(e) => println!("✗ {e}"),
            }
        }
    } else {
        println!("[迴路檢查] … （沒有可供檢查的路由）");
    }

    print!("[TTS] … ");
    match &cfg.tts.engine {
        speakdrill_core::TtsEngine::Piper { exe, model } => {
            if std::path::Path::new(exe).exists() && std::path::Path::new(model).exists() {
                println!("piper OK");
            } else {
                println!("✗ piper 或 model 不存在"); bad += 1;
            }
        }
        speakdrill_core::TtsEngine::Command { template } => println!("command: {template}"),
    }

    println!("\n快取：{}", cache_dir().display());
    println!("{}", if bad == 0 { "全部正常 ✓" } else { "有問題，見上面 ✗" });
    Ok(())
}

// ---------------- reset ----------------

async fn cmd_reset(yes: bool) -> Result<()> {
    println!("== speakdrill reset ==");
    match Vm::connect() {
        Ok(vm) => {
            let n = match vm.vm_type { 1 => 3, 2 => 5, 3 => 8, _ => 5 };
            for j in 0..n {
                vm.set(&format!("strip[{j}].B1"), 0.0).ok();
            }
            println!("已停止所有 strip 送到 B1（TTS 不會再灌進麥克風）");
        }
        Err(e) => println!("⚠ {e}"),
    }

    if yes {
        let p = config_path();
        if p.exists() {
            std::fs::remove_file(&p).ok();
            println!("已刪除設定檔 {}", p.display());
        }
    } else {
        println!("\n要清掉 speakdrill 的 per-app 輸入裝置，只能手動：");
        println!("  ms-settings:apps-volume → English-Chinese-Japanese → 輸入裝置 → 預設");
        println!("\n（加 --yes 可一併刪除設定檔）");
    }
    Ok(())
}

// ---------------- setup ----------------

/// 綁定/修復 per-app 音訊：registry → COM → UIA，實作在 speakdrill-core::setup
async fn cmd_setup(
    set_output: Option<String>,
    dry_run: bool,
    no_uia: bool,
    force: bool,
    restart_settings: bool,
) -> Result<()> {
    let opts = speakdrill_core::setup::SetupOptions { set_output, dry_run, no_uia, force, restart_settings };
    speakdrill_core::setup::run_setup(&opts, &mut |s| println!("{s}"))
}

// ---------------- list ----------------

async fn cmd_list(code: Option<String>, refresh: bool) -> Result<()> {
    let mut cfg = Config::load_or_default();
    let has_code = code.is_some();
    if let Some(c) = code {
        cfg.code = c;
    }
    let cdp = Cdp::connect("127.0.0.1", cfg.cdp_port).await?;
    ensure_logged_in(&cdp, &cfg.server).await?;
    let all = if has_code {
        speakdrill_core::lesson::fetch_catalog_opts(&cdp, &cfg.server, &cfg.code, &cfg.content_type, refresh).await?
    } else {
        speakdrill_core::lesson::fetch_all_catalogs(&cdp, &cfg.server, &cfg.content_type, refresh).await?
    };
    println!("{} 課（{}）\n", all.len(), if has_code { cfg.code.clone() } else { "所有課本".to_string() });
    for (i, l) in all.iter().enumerate() {
        println!("{:3}. {}", i + 1, l.label());
    }
    Ok(())
}

// ---------------- run / batch ----------------

async fn make_session(cfg: &Config) -> Result<(Cdp, AudioPlayer)> {
    let cdp = Cdp::connect("127.0.0.1", cfg.cdp_port).await?;
    ensure_logged_in(&cdp, &cfg.server).await?;
    let player = AudioPlayer::new(&cfg.play_device)?;
    Ok((cdp, player))
}

async fn cmd_run(lesson: String, course: String, resume: bool, pass: u32) -> Result<()> {
    let mut cfg = Config::load_or_default();
    cfg.pass_score = pass;
    let (cdp, player) = make_session(&cfg).await?;
    let mut p = SilentProgress;
    let report = run_lesson(
        &cdp, &cfg, &player, &lesson, &course,
        &RunOptions { resume, ..Default::default() }, &mut p,
    ).await?;
    print_report(&report);
    Ok(())
}

async fn cmd_batch(
    code: Option<String>, content_type: Option<String>,
    limit: Option<usize>, start: Option<usize>, pass: u32, resume: bool,
    skip_completed: bool,
) -> Result<()> {
    let mut cfg = Config::load_or_default();
    cfg.pass_score = pass;
    if let Some(c) = code { cfg.code = c; }
    if let Some(t) = content_type { cfg.content_type = t; }

    let (cdp, player) = make_session(&cfg).await?;
    let all = fetch_catalog(&cdp, &cfg.server, &cfg.code, &cfg.content_type).await?;
    let done_n = all.iter().filter(|l| l.done()).count();
    println!("目錄共 {} 課（已練過 {} 課）", all.len(), done_n);

    let mut pool: Vec<&LessonRef> = all.iter().collect();
    if skip_completed {
        pool.retain(|l| !l.done());
        println!("跳過已練過 → 剩 {} 課", pool.len());
    }

    let begin = start.unwrap_or(0);
    let sel: Vec<_> = pool.iter().skip(begin).take(limit.unwrap_or(usize::MAX)).collect();

    let mut ok = 0;
    for (i, l) in sel.iter().enumerate() {
        println!("\n=== [{}/{}] {} ===", begin + i + 1, all.len(), l.label());
        let mut p = SilentProgress;
        match run_lesson(
            &cdp, &cfg, &player, &l.lesson_id, &l.course_id,
            &RunOptions { resume, ..Default::default() }, &mut p,
        ).await {
            Ok(r) => { print_report(&r); ok += 1; }
            Err(e) => println!("  ✗ 失敗：{e}"),
        }
    }
    println!("\n批次完成：{ok}/{} 課", sel.len());
    Ok(())
}

fn print_report(r: &LessonReport) {
    println!("  {} — {}/{} 句，平均 {}，{} 秒",
        r.title, r.lines.len(), r.total_lines, r.avg, r.elapsed_secs);
    if let Ok(p) = save_report(r) {
        println!("  報告：{}", p.display());
    }
}

/// 全部課程批次執行（所有 CodeName）
async fn cmd_all(
    content_type: Option<String>,
    limit: Option<usize>,
    start: Option<usize>,
    pass: u32,
    resume: bool,
    skip_completed: bool,
) -> Result<()> {
    let mut cfg = Config::load_or_default();
    cfg.pass_score = pass;
    if let Some(t) = content_type { cfg.content_type = t; }

    let (cdp, player) = make_session(&cfg).await?;
    let all = speakdrill_core::lesson::fetch_all_catalogs(&cdp, &cfg.server, &cfg.content_type, false).await?;
    let done_n = all.iter().filter(|l| l.done()).count();
    println!("全部課程共 {} 課（已練過 {} 課）", all.len(), done_n);

    let mut pool: Vec<&LessonRef> = all.iter().collect();
    if skip_completed {
        pool.retain(|l| !l.done());
        println!("跳過已練過 → 剩 {} 課", pool.len());
    }

    let begin = start.unwrap_or(0);
    let sel: Vec<_> = pool.iter().skip(begin).take(limit.unwrap_or(usize::MAX)).collect();

    let mut ok = 0;
    for (i, l) in sel.iter().enumerate() {
        println!("\n=== [{}/{}] {} ===", begin + i + 1, all.len(), l.label());
        let mut p = SilentProgress;
        match run_lesson(
            &cdp, &cfg, &player, &l.lesson_id, &l.course_id,
            &RunOptions { resume, ..Default::default() }, &mut p,
        ).await {
            Ok(r) => { print_report(&r); ok += 1; }
            Err(e) => println!("  ✗ 失敗：{e}"),
        }
    }
    println!("\n全部批次完成：{ok}/{} 課", sel.len());
    Ok(())
}




