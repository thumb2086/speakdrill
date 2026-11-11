use std::path::PathBuf;

/// 找 piper 安裝位置（`init` 會下載到這裡）
pub fn piper_paths() -> Option<PiperPaths> {
    let dir = crate::config::config_dir().join("piper");
    let exe = [dir.join("piper").join("piper.exe"), dir.join("piper.exe")]
        .into_iter()
        .find(|p| p.exists())?;
    let model = dir.join("en_US-lessac-medium.onnx");
    if !model.exists() {
        return None;
    }
    Some(PiperPaths { exe, model })
}

pub struct PiperPaths {
    pub exe: PathBuf,
    pub model: PathBuf,
}

const PIPER_ZIP: &str =
    "https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_windows_amd64.zip";
const VOICE_BASE: &str =
    "https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/en/en_US/lessac/medium";

/// 下載並解開 piper + 聲音檔（約 82 MB）。已存在就直接回傳。
pub fn ensure_piper(log: &mut dyn FnMut(&str)) -> anyhow::Result<PiperPaths> {
    if let Some(p) = piper_paths() {
        log("Piper 已安裝");
        return Ok(p);
    }

    let dir = crate::config::config_dir().join("piper");
    std::fs::create_dir_all(&dir)?;

    let zip = dir.join("piper_windows_amd64.zip");
    log("下載 Piper（約 21 MB）…");
    curl(PIPER_ZIP, &zip)?;

    log("解開 Piper…");
    let status = std::process::Command::new("tar")
        .arg("-xf")
        .arg(&zip)
        .arg("-C")
        .arg(&dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("解壓縮 Piper 失敗（tar）");
    }
    let _ = std::fs::remove_file(&zip);

    let model = dir.join("en_US-lessac-medium.onnx");
    if !model.exists() {
        log("下載英文聲音檔（約 60 MB）…");
        curl(&format!("{VOICE_BASE}/en_US-lessac-medium.onnx"), &model)?;
        curl(
            &format!("{VOICE_BASE}/en_US-lessac-medium.onnx.json"),
            &dir.join("en_US-lessac-medium.onnx.json"),
        )?;
    }

    let p = piper_paths().ok_or_else(|| anyhow::anyhow!("Piper 安裝後仍找不到執行檔"))?;
    log("Piper 安裝完成");
    Ok(p)
}

fn curl(url: &str, dest: &std::path::Path) -> anyhow::Result<()> {
    let status = std::process::Command::new("curl")
        .args(["-L", "-s", "-S", "-o"])
        .arg(dest)
        .arg(url)
        .status()?;
    if !status.success() {
        anyhow::bail!("下載失敗：{url}");
    }
    if !dest.exists() || std::fs::metadata(dest)?.len() < 1024 {
        anyhow::bail!("下載的檔案不完整：{}", dest.display());
    }
    Ok(())
}
