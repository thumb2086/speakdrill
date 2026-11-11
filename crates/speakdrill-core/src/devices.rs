//! 偵測可用的「虛擬音效迴路」，以及安裝 VB-CABLE 驅動。
//!
//! 兩種路線：
//!   - **VB-CABLE**（推薦）：零設定。播到 `CABLE Input`，`CABLE Output` 就有聲音。
//!   - **VoiceMeeter**：需要 strip → B1 路由，但有些人已經裝了。

use anyhow::{anyhow, Result};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct VirtualRoute {
    /// TTS 要播到哪個輸出裝置
    pub play_device: String,
    /// 目標 App 要選哪個錄音裝置
    pub mic_device: String,
    /// 需要 VoiceMeeter strip/bus 路由嗎
    pub needs_vm: bool,
}

impl VirtualRoute {
    pub fn kind(&self) -> &'static str {
        if self.needs_vm {
            "VoiceMeeter"
        } else {
            "VB-CABLE"
        }
    }
}

fn render_names() -> Vec<String> {
    crate::audio::list_outputs()
}

#[cfg(windows)]
fn capture_names() -> Vec<String> {
    crate::winvol::list_endpoints()
        .map(|v| {
            v.into_iter()
                .filter(|(k, _, _)| k == "capture")
                .map(|(_, n, _)| n)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(not(windows))]
fn capture_names() -> Vec<String> {
    Vec::new()
}

/// CABLE 的播放端點在 Windows 可能叫「CABLE Input」，
/// 也可能叫「喇叭 (VB-Audio Virtual Cable)」，兩種都要認。
fn pick_cable_input(names: &[String]) -> Option<String> {
    if let Some(n) = names.iter().find(|n| n.to_lowercase().contains("cable input")) {
        return Some(n.clone());
    }
    names
        .iter()
        .find(|n| {
            let l = n.to_lowercase();
            l.contains("vb-audio virtual cable") && !l.contains("16")
        })
        .cloned()
}

fn pick(names: &[String], needle: &str) -> Option<String> {
    names
        .iter()
        .find(|n| n.to_lowercase().contains(&needle.to_lowercase()))
        .cloned()
}

/// 是否已安裝 VB-CABLE
pub fn has_vb_cable() -> bool {
    let outs = render_names();
    let ins = capture_names();
    pick_cable_input(&outs).is_some() && pick(&ins, "CABLE Output").is_some()
}

/// 所有可用的迴路，優先序：VB-CABLE → VoiceMeeter
pub fn routes() -> Vec<VirtualRoute> {
    let outs = render_names();
    let ins = capture_names();
    let mut v = Vec::new();

    if let (Some(p), Some(m)) = (pick_cable_input(&outs), pick(&ins, "CABLE Output")) {
        v.push(VirtualRoute { play_device: p, mic_device: m, needs_vm: false });
    }
    if let (Some(p), Some(m)) = (pick(&outs, "Voicemeeter Input"), pick(&ins, "Voicemeeter Out B1")) {
        v.push(VirtualRoute { play_device: p, mic_device: m, needs_vm: true });
    }
    v
}

/// 偵測目前可用的虛擬迴路（優先 VB-CABLE，其次 VoiceMeeter）
pub fn detect_route() -> Option<VirtualRoute> {
    let outs = render_names();
    let ins = capture_names();

    if let (Some(p), Some(m)) = (pick_cable_input(&outs), pick(&ins, "CABLE Output")) {
        return Some(VirtualRoute {
            play_device: p,
            mic_device: m,
            needs_vm: false,
        });
    }

    if let (Some(p), Some(m)) = (pick(&outs, "Voicemeeter Input"), pick(&ins, "Voicemeeter Out B1")) {
        return Some(VirtualRoute {
            play_device: p,
            mic_device: m,
            needs_vm: true,
        });
    }

    None
}

/// 列出目前所有虛擬裝置（給 doctor 顯示）
pub fn describe() -> Vec<String> {
    let mut v = Vec::new();
    for n in render_names() {
        if n.to_lowercase().contains("cable input") || n.to_lowercase().contains("voicemeeter input") {
            v.push(format!("  輸出：{n}"));
        }
    }
    for n in capture_names() {
        let l = n.to_lowercase();
        if l.contains("cable output") || l.contains("voicemeeter out b1") {
            v.push(format!("  錄音：{n}"));
        }
    }
    v
}

// ---------------- VB-CABLE 安裝 ----------------

const VBCABLE_ZIP: &str = "https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack45.zip";

/// 下載並（以管理員權限）安裝 VB-CABLE。
///
/// 會跳出一次 UAC。安裝後裝置通常立即出現，不需重開機。
pub fn install_vb_cable(log: &mut dyn FnMut(&str)) -> Result<()> {
    let dir = crate::config::cache_dir().join("vbcable");
    std::fs::create_dir_all(&dir)?;
    let zip = dir.join("VBCABLE_Driver_Pack.zip");

    if !dir.join("VBCABLE_Setup_x64.exe").exists() {
        log("下載 VB-CABLE（約 1.3 MB）…");
        let st = std::process::Command::new("curl")
            .args(["-L", "-s", "-S", "-o"])
            .arg(&zip)
            .arg(VBCABLE_ZIP)
            .status()?;
        if !st.success() {
            return Err(anyhow!("下載 VB-CABLE 失敗"));
        }
        log("解開…");
        let st = std::process::Command::new("tar")
            .arg("-xf")
            .arg(&zip)
            .arg("-C")
            .arg(&dir)
            .status()?;
        if !st.success() {
            return Err(anyhow!("解壓縮 VB-CABLE 失敗"));
        }
        let _ = std::fs::remove_file(&zip);
    }

    let setup: PathBuf = dir.join("VBCABLE_Setup_x64.exe");
    if !setup.exists() {
        return Err(anyhow!("找不到 VBCABLE_Setup_x64.exe"));
    }

    log("接下來會跳出 UAC，請按「是」允許安裝音效驅動");
    log("注意：安裝任何虛擬音效驅動後，建議重開機再執行 init/doctor");
    let ps = format!(
        "Start-Process -FilePath '{}' -ArgumentList '-i','-h' -Verb RunAs -Wait",
        setup.display()
    );
    let st = std::process::Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &ps])
        .status()?;
    if !st.success() {
        return Err(anyhow!("安裝程式回報失敗（可能取消了 UAC）"));
    }

    log("等待裝置出現…");
    log("若安裝後迴路仍不可用，請重開機後再跑一次 init");
    for _ in 0..30 {
        if detect_route().is_some() {
            log("完成：偵測到虛擬音效迴路");
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    Err(anyhow!(
        "安裝完成但沒偵測到裝置。請重開機，或到「音效設定」確認 CABLE Input/Output 存在"
    ))
}
