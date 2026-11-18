use std::path::PathBuf;

/// 聲音模型 / 設定檔的**確切大小與 SHA-256**（比對自 HuggingFace LFS oid 與本機實測可用模型）。
/// 下載中斷時 curl 只會留下部分檔案，這是「壞模型靜默存在」的根本解法。
pub const MODEL_SHA256: &str = "5efe09e69902187827af646e1a6e9d269dee769f9877d17b16b1b46eeaaf019f";
pub const MODEL_SIZE: u64 = 63_201_294;
pub const MODEL_JSON_SHA256: &str =
    "efe19c417bed055f2d69908248c6ba650fa135bc868b0e6abb3da181dab690a0";
pub const MODEL_JSON_SIZE: u64 = 4_885;

fn sha256_file(path: &std::path::Path) -> anyhow::Result<String> {
    use sha2::Digest;
    let mut f = std::fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = [0u8; 1024 * 1024];
    loop {
        let n = std::io::Read::read(&mut f, &mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn json_path(model: &std::path::Path) -> PathBuf {
    let mut s = model.as_os_str().to_os_string();
    s.push(".json");
    PathBuf::from(s)
}

fn check_one(path: &std::path::Path, size: u64, sha256: &str, label: &str) -> anyhow::Result<()> {
    let meta = std::fs::metadata(path)
        .map_err(|e| anyhow::anyhow!("{label} 不存在（{}）：{e}", path.display()))?;
    if meta.len() != size {
        anyhow::bail!("{label} 大小不符（{} bytes，應為 {size} bytes）— 下載中斷 / 檔案損毀", meta.len());
    }
    let got = sha256_file(path)?;
    if !got.eq_ignore_ascii_case(sha256) {
        anyhow::bail!("{label} SHA-256 不符（got {got}，應為 {sha256}）— 檔案損毀");
    }
    Ok(())
}

/// 驗證聲音模型與設定檔完整（大小 + SHA-256）。
pub fn model_integrity(p: &PiperPaths) -> anyhow::Result<()> {
    check_one(&p.model, MODEL_SIZE, MODEL_SHA256, "聲音模型 onnx")?;
    check_one(
        &json_path(&p.model),
        MODEL_JSON_SIZE,
        MODEL_JSON_SHA256,
        "模型設定檔 onnx.json",
    )
}

/// 下載到 dest 並驗證大小 + SHA-256；不符就刪掉，避免壞檔案被當成已安裝。
fn download_verified(
    url: &str,
    dest: &std::path::Path,
    size: u64,
    sha256: &str,
    label: &str,
) -> anyhow::Result<()> {
    curl(url, dest)?;
    let meta = std::fs::metadata(dest)?;
    if meta.len() != size {
        let _ = std::fs::remove_file(dest);
        anyhow::bail!("{label} 下載不完整（{} bytes，應為 {size} bytes）— 已刪除，請重新執行 speakdrill init", meta.len());
    }
    let got = sha256_file(dest)?;
    if !got.eq_ignore_ascii_case(sha256) {
        let _ = std::fs::remove_file(dest);
        anyhow::bail!("{label} SHA-256 不符 — 已刪除，請重新執行 speakdrill init");
    }
    Ok(())
}

/// 下載聲音模型與設定檔（含驗證），不完整就會報錯並留下乾淨狀態。
fn download_model(dir: &std::path::Path, log: &mut dyn FnMut(&str)) -> anyhow::Result<()> {
    log("下載英文聲音檔（約 63 MB）…");
    download_verified(
        &format!("{VOICE_BASE}/en_US-lessac-medium.onnx"),
        &dir.join("en_US-lessac-medium.onnx"),
        MODEL_SIZE,
        MODEL_SHA256,
        "聲音模型 onnx",
    )?;
    log("下載模型設定檔…");
    download_verified(
        &format!("{VOICE_BASE}/en_US-lessac-medium.onnx.json"),
        &dir.join("en_US-lessac-medium.onnx.json"),
        MODEL_JSON_SIZE,
        MODEL_JSON_SHA256,
        "模型設定檔 onnx.json",
    )?;
    Ok(())
}

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

/// 下載 / 既有的 piper 都會先驗證模型完整性，再跑一次「真的合成一句」——模型下載不完整時
/// 檔案存在仍會讓 piper 以 0xC0000409 之類的 exit code 自行崩潰，而且什麼都不印。
pub fn ensure_piper(log: &mut dyn FnMut(&str)) -> anyhow::Result<PiperPaths> {
    let dir = crate::config::config_dir().join("piper");

    if let Some(p) = piper_paths() {
        log("Piper 已安裝，檢查模型完整性…");
        if model_integrity(&p).is_err() {
            log("模型損毀 / 下載中斷，重新下載聲音檔…");
            download_model(&dir, log)?;
            model_integrity(&p)
                .map_err(|e| anyhow::anyhow!("模型重新下載後驗證仍失敗：{e}"))?;
            log("模型完整性驗證通過");
        }
        return piper_self_test(&p).map(|_| p);
    }

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

    download_model(&dir, log)?;

    let p = piper_paths().ok_or_else(|| anyhow::anyhow!("Piper 安裝後仍找不到執行檔"))?;
    model_integrity(&p)?;
    log("Piper 安裝完成，自我測試…");
    piper_self_test(&p)?;
    Ok(p)
}

/// 確認 piper 能真的發出一句話——模型完整性通過後，實際合成一次。
// （注意：piper 崩潰時 stdout/stderr 常為空、非 0 exit，見 LESSONS.md #12）
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
    /// 模型完整性檢查：健康安裝通過；截斷 2 MB 的模型必須被擋下且訊息指向「大小不符」。
    #[test]
    #[ignore]
    fn model_integrity_detects_truncated_model() {
        let Some(p) = piper_paths() else {
            eprintln!("skip：本機沒有 piper 安裝");
            return;
        };
        assert!(model_integrity(&p).is_ok(), "健康模型應通過驗證");
        let tmp = std::env::temp_dir().join("speakdrill_integrity_truncated.onnx");
        let src = std::fs::File::open(&p.model).unwrap();
        let mut limited = std::io::Read::take(src, 2 * 1024 * 1024);
        let mut dst = std::fs::File::create(&tmp).unwrap();
        std::io::copy(&mut limited, &mut dst).unwrap();
        drop(dst);
        let broken = PiperPaths { exe: p.exe.clone(), model: tmp.clone() };
        let err = model_integrity(&broken).map(|_| "（竟然通過）".to_string()).unwrap_err().to_string();
        assert!(err.contains("大小不符"), "錯誤訊息應指出大小不符，實際：{err}");
        println!("integrity-truncated message: {err}");
        let _ = std::fs::remove_file(&tmp);
    }

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
