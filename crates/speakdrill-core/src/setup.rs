//! 「自動把 per-app 音訊設到虛擬迴路」的共用邏輯。
//!
//! 優先序：registry 已綁定 → COM API 直接寫入 → 失敗則 UI Automation 代替
//! 使用者操作 Windows「設定 → 音量混合器」。全程驗證、失敗即停、不亂點其他 App。
//!
//! CLI 與 GUI 共用這支；在終端機用 `speakdrill setup`，GUI 按鈕也是同一段程式。

use anyhow::Result;

/// UIA helper（scripts/audio-uia.ps1）隨 binary 一起發布，執行時解到 cache 目錄
const AUDIO_UIA_PS1: &str = include_str!("../../../scripts/audio-uia.ps1");

#[derive(Debug, Default, Clone)]
pub struct SetupOptions {
    /// 同時設定 App 的輸出裝置
    pub set_output: Option<String>,
    /// 只預覽、不實際寫入
    pub dry_run: bool,
    /// 不使用 UI Automation（COM 失敗時直接印手動步驟）
    pub no_uia: bool,
    /// 即使 registry 顯示已綁定也重設
    pub force: bool,
    /// 「設定」視窗沒有回應時，先殺掉再重新打開（你自己按下去的明確同意）
    pub restart_settings: bool,
}

/// 列出指定 exe 的執行程序（透過 tasklist，避免引入額外依賴）
pub fn process_pids(name: &str) -> Vec<u32> {
    let exe = if name.to_lowercase().ends_with(".exe") {
        name.to_string()
    } else {
        format!("{name}.exe")
    };
    let Ok(out) = std::process::Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {exe}"), "/FO", "CSV", "/NH"])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split(',');
            parts.next()?;
            parts.next()?.trim().trim_matches('"').parse::<u32>().ok()
        })
        .collect()
}

/// 把內嵌的 UIA helper 寫到 cache 目錄，回傳路徑
fn uia_script_path() -> Result<std::path::PathBuf> {
    let dir = crate::config::cache_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("audio-uia.ps1");
    std::fs::write(&path, AUDIO_UIA_PS1)?;
    Ok(path)
}

/// 執行 UIA helper，回傳 SPEAKDRILL_RESULT 的 JSON（ok/code/plan…）
/// 最多等 45 秒，逾時就砍掉（UIA 可能卡在彈出視窗上）。
fn run_audio_uia(
    app_name: &str,
    pids: &[u32],
    mode: &str,
    endpoint_name: &str,
    dry_run: bool,
) -> Result<serde_json::Value> {
    let script = uia_script_path()?;
    let ps_candidates = [
        r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
        "powershell",
    ];
    let exe = ps_candidates
        .iter()
        .find(|p| std::path::Path::new(p).exists() || **p == "powershell")
        .unwrap()
        .to_string();

    let mut cmd = std::process::Command::new(&exe);
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ])
    .arg(&script)
    .args([
        "-AppName",
        app_name,
        "-Mode",
        mode,
        "-EndpointName",
        endpoint_name,
        "-Pids",
        &pids.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(","),
    ]);
    // 沒開 Settings 時自動開 ms-settings:apps-volume
    cmd.arg("-AutoOpenSettings");
    if dry_run {
        cmd.arg("-DryRun");
    }
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn()?;
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait()? {
            Some(s) => break s,
            None => {
                if start.elapsed() > std::time::Duration::from_secs(45) {
                    let _ = child.kill();
                    anyhow::bail!("UIA helper 逾時（45 秒），已終止");
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    };

    let mut stdout = String::new();
    if let Some(mut out) = child.stdout.take() {
        use std::io::Read;
        out.read_to_string(&mut stdout).ok();
    }

    let result_line = stdout
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("SPEAKDRILL_RESULT="))
        .ok_or_else(|| {
            let mut stderr = String::new();
            if let Some(mut err) = child.stderr.take() {
                use std::io::Read;
                err.read_to_string(&mut stderr).ok();
            }
            let tail = stderr.lines().rev().take(3).collect::<Vec<_>>().join(" / ");
            anyhow::anyhow!("UIA helper 沒有回報結果（exit {status}）：{tail}")
        })?;
    Ok(serde_json::from_str(result_line)?)
}

/// 印出 UIA helper 的結果（人話版）
fn print_uia_outcome(app: &str, res: &serde_json::Value, log: &mut dyn FnMut(String)) {
    let code = res["code"].as_i64().unwrap_or(-1);
    if code == 0 {
        for entry in res["plan"].as_array().into_iter().flatten() {
            let r = &entry["result"];
            let role = entry["role"].as_str().unwrap_or("?");
            let already = r["already"].as_bool().unwrap_or(false);
            let after = r["after"]
                .as_str()
                .or(r["current"].as_str())
                .unwrap_or("?");
            if already {
                log(format!("  ✓ {role}：{app} 本來就已經是 {after}"));
            } else {
                let from = r["current"].as_str().unwrap_or("（預設）");
                log(format!("  ✓ {role}：{app}  {from} → {after}"));
            }
        }
        return;
    }
    if code == 16 {
        log(format!(
            "  （dry-run）{}",
            res["note"].as_str().unwrap_or("只預覽、不更改")
        ));
        return;
    }
    log(format!(
        "  ✗ 自動設定失敗（code {code}）：{}",
        res["err"].as_str().unwrap_or("未知錯誤")
    ));
    match code {
        10 => log("        → 該 App 還沒有 audio session，Volume mixer 不會列出它；請先啟動 MyET 用過麥克風再重跑。".into()),
        11 => log("        → 同名 App 列太多，拒絕自動選擇；請關閉多餘的設定視窗。".into()),
        12 => log("        → 找不到「音量混合器」頁面；請手動開 ms-settings:apps-volume 確認。".into()),
        13 => {
            if let Some(avail) = res["available"].as_array() {
                let names: Vec<&str> = avail.iter().filter_map(|v| v.as_str()).collect();
                log(format!("        → 下拉選單裡沒有這個裝置。可選：{}", names.join(" / ")));
            }
            log("        → 若是剛裝驅動，請重開機後再跑一次。".into());
        }
        14 => log("        → 選完之後讀回值對不上；請用 doctor 確認。".into()),
        _ => log("        → 請保留 log 回報問題。".into()),
    }
    log(format!(
        "        或改成手動：ms-settings:apps-volume → English-Chinese-Japanese → 輸入裝置 → {app}"
    ));
}

/// 「設定」視窗（SystemSettings.exe）是否有回應；沒有這個行程回傳 None
pub fn settings_responding() -> Option<bool> {
    let out = std::process::Command::new(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            "(Get-Process SystemSettings -ErrorAction SilentlyContinue).Responding",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let t = text.trim().to_lowercase();
    if t.starts_with("true") {
        Some(true)
    } else if t.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

/// 跑完整套設定流程；所有輸出都走 `log` 閉包（CLI 印出來、GUI 送到 log 區）
pub fn run_setup(opts: &SetupOptions, log: &mut dyn FnMut(String)) -> Result<()> {
    let cfg = crate::config::Config::load_or_default();
    let routes = crate::devices::routes();
    let route = routes
        .iter()
        .find(|r| r.play_device == cfg.play_device)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("目前設定的迴路不可用，請先跑 speakdrill init"))?;

    log("== speakdrill setup ==".to_string());
    log(format!(
        "麥克風：{} → {}（{}）",
        cfg.app_exe,
        route.mic_device,
        route.kind()
    ));
    if opts.dry_run {
        log("（dry-run 模式：不會實際更改任何設定）".to_string());
    }

    let pids = process_pids(&cfg.app_exe);
    if pids.is_empty() {
        log(format!("⚠ 找不到 {} 的執行程序", cfg.app_exe));
        log("  要讓程式自動設定，MyET 至少要啟動一次並用過麥克風（Windows 才會列在 mixer 裡）".to_string());
        log("  已開啟 ms-settings:apps-volume，啟動 MyET 後再跑一次 setup".to_string());
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "ms-settings:apps-volume"])
            .status();
        return Ok(());
    }

    let expected_mic = crate::winvol::find_endpoint_id(&route.mic_device, true);
    let mic_ok = || -> bool {
        match (&expected_mic, crate::binding::app_mic_binding(&cfg.app_exe)) {
            (Some(exp), Some(b)) => {
                // 檢查 Multimedia role (001_001) 是否已綁定，這才是 MyET 實際使用的 role
                if let Some((_, v)) = b.mic_roles.iter().find(|(k, _)| k == "001_001") {
                    if let Some(g) = crate::binding::extract_guid(v) {
                        return g.eq_ignore_ascii_case(exp);
                    }
                }
                // 如果 001_001 不存在，fallback 到 any_matches
                b.any_matches(exp)
            }
            _ => false,
        }
    };
    // registry 已綁定且沒有強制/輸出/dry-run 需求時直接結束
    if !opts.force && !opts.dry_run && opts.set_output.is_none() && mic_ok() {
        log(format!("✓ {} 已綁定 {}", cfg.app_exe, route.mic_device));
        log("（要強制重設請加 --force；要設定輸出裝置請加 --set-output）".to_string());
        return Ok(());
    }

    log(format!(
        "找到程序：{}",
        pids.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ")
    ));

    // ---- 第 1 層：COM API 直接設定 -------------------------------------
    if !opts.dry_run {
        for pid in &pids {
            match crate::per_app::configure(&route.mic_device, *pid, true) {
                Ok(()) => log(format!("  ✓ pid {pid}：已設成 {}", route.mic_device)),
                Err(e) => log(format!("  ✗ pid {pid}：{e}")),
            }
        }
        if let Some(out_name) = &opts.set_output {
            for pid in &pids {
                match crate::per_app::configure(out_name, *pid, false) {
                    Ok(()) => log(format!("  ✓ pid {pid}（輸出）：已設成 {out_name}")),
                    Err(e) => log(format!("  ✗ pid {pid}（輸出）：{e}")),
                }
            }
        }

        // registry 確認；成功就結束
        if mic_ok() {
            log(format!(
                "\n✓ 驗證成功：{} 的麥克風已綁定 {}",
                cfg.app_exe, route.mic_device
            ));
            return Ok(());
        }
        log(format!(
            "⚠ COM API 已呼叫但 registry 回讀不一致：{}",
            route.mic_device
        ));
    }

    if opts.no_uia {
        log("\n依你指定 --no-uia，不自動操作設定視窗。手動步驟：".to_string());
        log(format!(
            "  ms-settings:apps-volume → English-Chinese-Japanese → 輸入裝置 → {}",
            route.mic_device
        ));
        return Ok(());
    }

    // ---- 第 2 層：UI Automation 操作 Windows「設定」---------------------
    // 「設定」視窗沒回應時 UIA 會直接卡死：先快速檢查，有問題就用明確旗標重開
    match settings_responding() {
        Some(false) => {
            if opts.restart_settings {
                log("\n「設定」視窗沒有回應，依 --restart-settings 先關閉再重開…".to_string());
                let _ = std::process::Command::new("taskkill")
                    .args(["/F", "/IM", "SystemSettings.exe", "/T"])
                    .status();
                std::thread::sleep(std::time::Duration::from_millis(800));
                let _ = std::process::Command::new("cmd")
                    .args(["/C", "start", "ms-settings:apps-volume"])
                    .status();
                std::thread::sleep(std::time::Duration::from_secs(2));
            } else {
                log("\n✗「設定」視窗沒有回應（可能已經卡死），UI Automation 會卡住。".to_string());
                log("  請關閉所有「設定」視窗後再跑一次；".to_string());
                log("  或在資料沒問題的前提下用 speakdrill setup --restart-settings（會自動關閉並重開設定視窗）".to_string());
                return Ok(());
            }
        }
        _ => {}
    }
    log("\n改用 UI Automation 操作「設定 → 音量混合器」…".to_string());
    match run_audio_uia(&cfg.app_exe, &pids, "input", &route.mic_device, opts.dry_run) {
        Ok(res) => print_uia_outcome(&cfg.app_exe, &res, log),
        Err(e) => log(format!("  ✗ UIA helper 執行失敗：{e}")),
    }
    if let Some(out_name) = &opts.set_output {
        match run_audio_uia(&cfg.app_exe, &pids, "output", out_name, opts.dry_run) {
            Ok(res) => print_uia_outcome(&cfg.app_exe, &res, log),
            Err(e) => log(format!("  ✗ UIA helper 執行失敗：{e}")),
        }
    }

    if opts.dry_run {
        return Ok(());
    }

    // ---- 第 3 層：最後再驗一次 registry ----------------------------------
    match crate::binding::app_mic_binding(&cfg.app_exe) {
        Some(b) => match &expected_mic {
            Some(exp) if b.any_matches(exp) => log(format!(
                "\n✓ 驗證成功：{} 的麥克風已綁定 {}",
                cfg.app_exe, route.mic_device
            )),
            _ => log(
                "\n⚠ registry 回讀與預期不符；請用 doctor 確認（registry 可能有同步延遲）"
                    .to_string(),
            ),
        },
        None => log("\n⚠ 讀不到 per-app 設定；請用 doctor 確認".to_string()),
    }

    Ok(())
}
