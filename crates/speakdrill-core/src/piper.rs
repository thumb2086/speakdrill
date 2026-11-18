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

/// 下載 / 既有的 piper 都要能「真的發出一句話」——模型下載不完整時
/// 檔案存在仍會讓 piper 以 0xC0000409 之類的 exit code 自行崩潰，
/// 而且什麼都不印。這裡做一次實際合成，讓問題在安裝 / 診斷階段就現形。
pub fn piper_self_test(p: &PiperPaths) -> anyhow::Result<()> {
    let tmp = std::env::temp_dir().join("speakdrill_piper_selftest.wav");
    let _ = std::fs::remove_file(&tmp);
    let cfg = crate::tts::TtsConfig {
        engine: crate::tts::TtsEngine::Piper {
            exe: p.exe.to_string_lossy().into_owned(),
            model: p.model.to_string_lossy().into_owned(),
        },
        length_scale: 1.0,
    };
    let result = cfg.synthesize("Hello.", &tmp);
    if result.is_ok() && std::fs::metadata(&tmp).map(|m| m.len() > 0).unwrap_or(false) {
        let _ = std::fs::remove_file(&tmp);
        Ok(())
    } else {
        let detail = result.err().map(|e| e.to_string()).unwrap_or_else(|| "未產出 wav".to_string());
        Err(anyhow::anyhow!(
            "Piper 自我測試失敗（{detail}）。\n模型或執行檔很可能損毀 / 被防毒擋下：請刪除 {dir} 後重新執行 speakdrill init",
            dir = crate::config::config_dir().join("piper").display()
        ))
    }
}

/// 下載並解開 piper + 聲音檔（約 82 MB）。已存在就直接回傳（但仍做自我測試）。
pub fn ensure_piper(log: &mut dyn FnMut(&str)) -> anyhow::Result<PiperPaths> {
    if let Some(p) = piper_paths() {
        log("Piper 已安裝，檢查能否合成…");
        return piper_self_test(&p).map(|_| p);
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
    log("Piper 安裝完成，自我測試…");
    piper_self_test(&p)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 模型下載中斷留下損毀 onnx 時，piper 會以 0xC0000409 之類的 exit code 直接崩潰，
    /// 且什麼都不印 —— 自我測試必須把它變成可讀的錯誤而不是「piper 失敗：」後面空白。
    /// 需要本機已安裝 piper（init 過）才跑得動；不會動到既有安裝。
    #[test]
    #[ignore]
    fn corrupt_model_reports_exit_code() {
        let Some(p) = piper_paths() else {
            eprintln!("skip：本機沒有 piper 安裝");
            return;
        };
        let tmp = std::env::temp_dir().join("speakdrill_corrupt_model.onnx");
        let src = std::fs::File::open(&p.model).unwrap();
        let mut limited = std::io::Read::take(src, 2 * 1024 * 1024);
        let mut dst = std::fs::File::create(&tmp).unwrap();
        std::io::copy(&mut limited, &mut dst).unwrap();
        drop(dst);
        let broken = PiperPaths { exe: p.exe.clone(), model: tmp.clone() };
        let err = piper_self_test(&broken).err().map(|e| e.to_string()).unwrap_or_else(|| "（自我測試竟然通過）".to_string());
        assert!(
            err.contains("自我測試失敗") && err.contains("0x"),
            "錯誤訊息應包含 exit code（例如 0xC0000409），實際：{err}"
        );
        println!("corrupt-model message: {err}");
        let _ = std::fs::remove_file(&tmp);
    }
}
