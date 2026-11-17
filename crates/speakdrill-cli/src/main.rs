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
#[command(name = "speakdrill", version, about = "MyET 口說練習自動化：TTS 驅動、虛擬音效迴路、分數報告")]
struct Cli {
    /// 不給子命令時直接進 TUI
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// 一次性設定：偵測裝置、寫設定、安裝 VB-CABLE 虛擬迴路、拉雙端音量、自動綁 per-app 麥克風
    Init {
        #[arg(long)] device: Option<String>,
        #[arg(long)] server: Option<String>,
        #[arg(long)] code: Option<String>,
        /// 找不到虛擬音效裝置時，自動下載並安裝 VB-CABLE（會跳 UAC）
        #[arg(long)] install_driver: bool,
    },
    /// 檢查 npm 上的最新版，並顯示更新指令（Windows 無法從程式內就地覆寫執行中的 exe，故只提示）
    Update,
    /// 自動下載並安裝最新版（優先下載 Inno Setup 安裝程式，會關閉 app 並覆蓋安裝）
    SelfUpdate,
    /// 檢查環境（port / 登入 / 裝置 / VoiceMeeter / TTS / 麥克風綁定）
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
        Cmd::Update => rt.block_on(cmd_update()),
        Cmd::SelfUpdate => rt.block_on(cmd_self_update()),
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

    println!("\n[1/6] 輸出裝置");
    let outs = list_outputs();
    for o in &outs {
        let mark = if o.to_lowercase().contains(&cfg.play_device.to_lowercase()) { " <== 選用" } else { "" };
        println!("  {o}{mark}");
    }
    if !outs.iter().any(|o| o.to_lowercase().contains(&cfg.play_device.to_lowercase())) {
        println!("  ⚠ 找不到「{}」，請用 --device 指定", cfg.play_device);
    }

    println!("\n[2/6] TTS 引擎");
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

    println!("\n[3/6] 虛擬音效迴路");
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

    let mic_device_hint = match &chosen {
        Some(r) => r.mic_device.clone(),
        None => "Voicemeeter Out B1".to_string(),
    };

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
            println!("  程式需要一個虛擬音效迴路（VB-CABLE）把 TTS 播放聲音變成麥克風輸入。");
            println!("  Windows 沒有 user-mode 建立麥克風的方法，必須裝虛擬音效驅動。");
            println!();
            println!("  VB-CABLE（1.3 MB，零設定，必裝）");
            println!();
            print!("  要現在自動安裝 VB-CABLE 嗎？需要管理員權限（UAC），安裝後必須重開機 [Y/n] ");
            use std::io::{self, Write};
            io::stdout().flush().ok();
            let mut input = String::new();
            io::stdin().read_line(&mut input).ok();
            let input = input.trim().to_lowercase();
            if input.is_empty() || input == "y" || input == "yes" {
                println!("  正在下載並安裝 VB-CABLE…");
                if let Err(e) = speakdrill_core::devices::install_vb_cable(&mut |m| println!("  {m}")) {
                    println!("  ⚠ 安裝失敗：{e}");
                    println!("  請自行從 https://vb-audio.com/Cable/ 下載並安裝");
                } else {
                    println!("  ✓ VB-CABLE 安裝完成");
                    println!("  ⚠ 安裝後必須重開機才能生效（Windows 核心驅動限制）");
                    println!("  重開機後請再跑一次 `speakdrill init`");
                    return Ok(());
                }
            } else {
                println!("  已取消。請從 https://vb-audio.com/Cable/ 下載並安裝 VB-CABLE，裝完重開機後再跑 init");
                return Ok(());
            }
        }
    }

    println!("\n[4/6] Windows 裝置音量");
    #[cfg(windows)]
    {
        // 不管哪條路線，都把「模擬麥克風」端拉滿（VB-CABLE 是 CABLE Output，VoiceMeeter 是 Voicemeeter Out B1）
        let routes = speakdrill_core::devices::routes();
        match routes.iter().find(|r| r.play_device == cfg.play_device) {
            Some(r) => match speakdrill_core::winvol::set_volume(&r.mic_device, true, 1.0) {
                Ok(n) => println!("  錄音裝置 {}：{n} 個已設 100%", r.mic_device),
                Err(e) => println!("  ⚠ 錄音裝置 {} 音量：{e}", r.mic_device),
            },
            None => println!("  （找不到對應的錄音迴路，略過錄音裝置音量）"),
        }
        match speakdrill_core::winvol::set_volume(&cfg.play_device, false, 1.0) {
            Ok(n) => println!("  播放裝置：{n} 個已設 100%"),
            Err(e) => println!("  ⚠ 播放音量：{e}"),
        }
    }
    #[cfg(not(windows))]
    println!("  非 Windows，略過");

    cfg.save()?;
    println!("\n[5/6] 設定已寫入 {}", config_path().display());

    println!("\n[6/6] 自動設定 per-app 麥克風（registry → COM → UIA）");
    #[cfg(windows)]
    {
        match speakdrill_core::setup::run_setup(
            &speakdrill_core::setup::SetupOptions::default(),
            &mut |s| println!("  {s}"),
        ) {
            Ok(()) => println!("  （如上方顯示「已綁定」即完成；否則請先開過 MyET 再跑 `speakdrill setup`）"),
            Err(e) => {
                println!("  ⚠ 自動設定失敗：{e}");
                println!("{}", "─".repeat(64));
                println!("手動步驟（或先開過一次 MyET 再跑 `speakdrill setup`）：");
                println!("  1. Win+R → ms-settings:apps-volume");
                println!("  2. 找到「English-Chinese-Japanese」（就是 MyET）並展開");
                println!("  3. 輸入裝置 → 選「{}」", mic_device_hint);
                println!("{}", "─".repeat(64));
            }
        }
    }
    #[cfg(not(windows))]
    println!("  非 Windows，略過");
    println!("\n✓ init 完成。下一步：speakdrill doctor（檢查環境）→ speakdrill（開始練習）");
    Ok(())
}

/// 比較兩個 x.y.z 版本字串（a > b 回傳 true）
fn version_gt(a: &str, b: &str) -> bool {
    let parse = |s: &str| {
        s.trim()
            .trim_start_matches('v')
            .split('.')
            .map(|p| p.trim().parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let (a, b) = (parse(a), parse(b));
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

/// `speakdrill update`：比對目前版本與 npm 上的最新版，印出更新指令
/// （Windows 無法從程式內就地覆寫執行中的 exe，所以更新本身在程式外跑）
async fn cmd_update() -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!("目前版本：v{current}");
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let out = std::process::Command::new(npm).args(["view", "speakdrill", "version"]).output();
    match out {
        Ok(o) if o.status.success() => {
            let latest = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if latest.is_empty() {
                println!("⚠ 解析不到 npm 上的最新版本");
                println!("手動更新：npm i -g speakdrill@latest");
            } else if version_gt(&latest, current) {
                println!("新版本可用：v{latest}");
                println!();
                println!("更新指令：");
                println!("  npm i -g speakdrill@latest");
                println!("（先關閉所有 speakdrill 視窗，避免 speakdrill.exe 被鎖住導致 EBUSY）");
            } else {
                println!("✓ 已是最新版");
            }
        }
        _ => {
            println!("⚠ 找不到 npm（或網路不通）");
            println!("手動更新：npm i -g speakdrill@latest");
        }
    }
    Ok(())
}

/// `speakdrill self-update`：自動下載並安裝最新版
/// 優先使用 Inno Setup 安裝程式（speakdrill-setup-*.exe，會關閉 app、裝好覆蓋舊版）
/// 沒有 setup 程式時退回 zip（解壓覆蓋安裝目錄裡的舊 exe，需先關閉 speakdrill）
async fn cmd_self_update() -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!("目前版本：v{current}");

    // 從 GitHub Releases API 取得最新版本資訊
    let client = reqwest::Client::new();
    let url = "https://api.github.com/repos/thumb2086/speakdrill/releases/latest";
    let resp = client
        .get(url)
        .header("User-Agent", "speakdrill")
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await?;

    if !resp.status().is_success() {
        println!("⚠ 無法取得 GitHub Releases 資訊");
        println!("請手動從 https://github.com/thumb2086/speakdrill/releases 下載最新版");
        return Ok(());
    }

    let release: serde_json::Value = resp.json().await?;
    let tag_name = release["tag_name"].as_str().unwrap_or("").trim_start_matches('v');
    let empty_vec = vec![];
    let assets = release["assets"].as_array().unwrap_or(&empty_vec);

    // 優先找 Inno Setup 安裝程式
    let setup_asset = assets.iter().find(|a| {
        let name = a["name"].as_str().unwrap_or("");
        name.starts_with("speakdrill-setup-") && name.ends_with(".exe")
    });
    let zip_asset = assets.iter().find(|a| {
        a["name"].as_str().unwrap_or("").ends_with(".zip")
    });

    let latest_version = tag_name;
    let (download_url, tmp_name, use_installer) = if let Some(a) = setup_asset {
        (
            a["browser_download_url"].as_str().unwrap_or("").to_string(),
            a["name"].as_str().unwrap_or("speakdrill-setup.exe").to_string(),
            true,
        )
    } else if let Some(a) = zip_asset {
        (
            a["browser_download_url"].as_str().unwrap_or("").to_string(),
            a["name"].as_str().unwrap_or("speakdrill.zip").to_string(),
            false,
        )
    } else {
        println!("⚠ 找不到可下載的發佈檔（setup exe / zip）");
        println!("請手動從 https://github.com/thumb2086/speakdrill/releases 下載最新版");
        return Ok(());
    };

    if !version_gt(latest_version, env!("CARGO_PKG_VERSION")) {
        println!("✓ 已是最新版 (v{current})");
        return Ok(());
    }

    println!("發現新版本：v{latest_version}");
    println!("下載中...");

    let tmp_dir = std::env::temp_dir().join("speakdrill-update");
    std::fs::create_dir_all(&tmp_dir)?;
    let tmp_path = tmp_dir.join(&tmp_name);

    let mut resp = client.get(&download_url).send().await?;
    let mut file = std::fs::File::create(&tmp_path)?;
    let mut stream = resp.bytes_stream();
    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        use std::io::Write;
        file.write_all(&chunk?)?;
    }
    println!("✓ 下載完成：{}", tmp_path.display());

    if use_installer {
        // Inno Setup 安裝程式會自動關閉正在執行的 speakdrill、覆蓋安裝
        println!("安裝中（Inno Setup 會關閉 app 並覆蓋舊版，請稍候）...");
        let status = std::process::Command::new(&tmp_path)
            .arg("/VERYSILENT")
            .arg("/CLOSEAPPLICATIONS")
            .arg("/NORESTART")
            .status()?;

        if status.success() {
            println!("✓ 安裝完成！請重新開啟 speakdrill");
        } else {
            println!("⚠ 安裝程式回傳非零（{status}），請手動執行：{}", tmp_path.display());
        }
    } else {
        // zip：需要先解壓再覆蓋安裝目錄
        println!("解壓縮中...");
        let extract_dir = tmp_dir.join("extracted");
        let extract_str = extract_dir.to_string_lossy().to_string();
        let zip_str = tmp_path.to_string_lossy().to_string();
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command",
                &format!("Expand-Archive -Path '{}' -DestinationPath '{}' -Force", zip_str, extract_str)])
            .status()?;
        if !status.success() {
            println!("⚠ 解壓失敗，請手動從 https://github.com/thumb2086/speakdrill/releases 下載最新版");
            return Ok(());
        }
        // 找安裝目錄（目前程式所在位置）
        let exe_path = std::env::current_exe()?;
        let install_dir = exe_path.parent().unwrap_or(std::path::Path::new("."));
        // 覆蓋可執行檔（正在執行的 exe 會鎖住，覆蓋會失敗）
        for name in ["speakdrill.exe", "speakdrill-gui.exe"] {
            let src = extract_dir.join(name);
            let dst = install_dir.join(name);
            if src.exists() {
                match std::fs::copy(&src, &dst) {
                    Ok(_) => println!("✓ 已更新 {}", dst.display()),
                    Err(e) => println!("⚠ 無法覆蓋 {}（{e}）：請先關閉 speakdrill，或以管理員執行", dst.display()),
                }
            }
        }
        println!("✓ 更新完成（如上方有警告，請關閉 speakdrill 後重開）");
    }

    Ok(())
}

// ---------------- version test ----------------
#[cfg(test)]
mod tests {
    use super::version_gt;

    #[test]
    fn version_gt_works() {
        assert!(version_gt("0.2.35", "0.2.34"));
        assert!(version_gt("0.3.0", "0.2.99"));
        assert!(version_gt("1.0.0", "0.99.9"));
        assert!(version_gt("0.2.100", "0.2.99"));
        assert!(!version_gt("0.2.35", "0.2.35"));
        assert!(!version_gt("0.2.9", "0.2.35"));
        assert!(!version_gt("0.2.35", "0.3.0"));
    }
}

// ---------------- doctor ----------------

async fn cmd_doctor() -> Result<()> {
    let cfg = Config::load_or_default();
    println!("== speakdrill doctor ==\n設定檔：{}", config_path().display());

    let mut bad = 0;

    print!("[CDP] port {} … ", cfg.cdp_port);
    // MyET 沒開就自動啟動（與 TUI 相同行為），否則永遠噴錯
    if !Cdp::check_myet_running("127.0.0.1", cfg.cdp_port).await {
        let myet_path = std::env::var("SPEAKDRILL_MYET_PATH")
            .unwrap_or_else(|_| r"C:\Program Files (x86)\LLabs\MyET10\MyET10.exe".to_string());
        if std::path::Path::new(&myet_path).exists() {
            println!("\n  MyET 未啟動 → 自動啟動中…");
            let port_str = cfg.cdp_port.to_string();
            let _ = std::process::Command::new(&myet_path)
                .args([format!("--remote-debugging-port={}", port_str), "--remote-allow-origins=*".to_string()])
                .spawn();
            for _ in 0..30 {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                if Cdp::check_myet_running("127.0.0.1", cfg.cdp_port).await {
                    break;
                }
            }
        } else {
            println!("（找不到 MyET 執行檔）");
        }
    }
    match Cdp::connect("127.0.0.1", cfg.cdp_port).await {
        Ok(cdp) => {
            println!("OK  ({})", cdp.target_url);
            print!("[登入] … ");
            match ensure_logged_in(&cdp, &cfg.server).await {
                Ok(_) => println!("OK"),
                Err(e) => { println!("✗ {e}"); bad += 1; }
            }
        }
        Err(e) => {
            println!("✗ MyET 未連上（{e}）");
            println!("  請先手動打開 MyET，或確認它已用 --remote-debugging-port=9222 啟動");
            println!("  若已啟動，請確認沒有多個 speakdrill 視窗鎖住 port 9222");
            bad += 1;
        }
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
                                "      修正：speakdrill setup 自動綁定（或手動 ms-settings:apps-volume → {} → 輸入裝置選「{}」）",
                                cfg.app_exe, r.mic_device
                            );
                            bad += 1;
                        }
                    }
                    None => {
                        println!("✗ 找不到 {} 的 per-app 設定", cfg.app_exe);
                        println!(
                            "      請跑 speakdrill setup 自動綁定（或手動 ms-settings:apps-volume → 輸入裝置選「{}」）",
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
    // CodeName／ContentType 必須與目錄一致，否則 MyET 頁面對不上課程。
    // 先在使用者設定的 CodeName 裡找；找不到就掃所有目錄定位這一課屬於哪本。
    let (code, content_type) = match speakdrill_core::lesson::fetch_catalog(&cdp, &cfg.server, &cfg.code, &cfg.content_type).await {
        Ok(list) if list.iter().any(|l| l.lesson_id == lesson) => (cfg.code.clone(), cfg.content_type.clone()),
        _ => match speakdrill_core::lesson::fetch_all_catalogs(&cdp, &cfg.server, &cfg.content_type, false).await {
            Ok(all) => all.iter().find(|l| l.lesson_id == lesson).map(|l| (l.code.clone(), l.contype.clone())).unwrap_or_else(|| {
                println!("⚠ 在目錄中找不到 {lesson}，退回設定的 CodeName {}", cfg.code);
                (cfg.code.clone(), cfg.content_type.clone())
            }),
            Err(_) => (cfg.code.clone(), cfg.content_type.clone()),
        },
    };
    if code != cfg.code {
        println!("使用課程所屬的 CodeName：{code}");
    }
    let mut p = SilentProgress;
    let report = run_lesson(
        &cdp, &cfg, &player, &lesson, &course, &code, &content_type,
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
            &cdp, &cfg, &player, &l.lesson_id, &l.course_id, &l.code, &l.contype,
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
            &cdp, &cfg, &player, &l.lesson_id, &l.course_id, &l.code, &l.contype,
            &RunOptions { resume, ..Default::default() }, &mut p,
        ).await {
            Ok(r) => { print_report(&r); ok += 1; }
            Err(e) => println!("  ✗ 失敗：{e}"),
        }
    }
    println!("\n全部批次完成：{ok}/{} 課", sel.len());
    Ok(())
}




