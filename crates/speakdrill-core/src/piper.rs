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

/// 判斷整條路徑是否全為 ASCII 字元。
/// **piper 無法載入含非 ASCII 字元的路徑**（實測 spawn → 0xC0000409 fastfail，無任何輸出）。
pub fn any_ascii(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii())
}

fn legacy_piper_dir() -> PathBuf {
    crate::config::config_dir().join("piper")
}

/// piper 安裝根目錄（PiperPaths 的模型/執行檔都在這）。
/// 使用者名稱是中文（例如 `C:\Users\還是的\...`）時，既有安裝一定是壞的 —
/// 尊重既有 ASCII 安裝；否則改用 machine-wide ASCII 位置。
pub fn piper_root() -> PathBuf {
    let legacy = legacy_piper_dir();
    if legacy.exists() && any_ascii(&legacy.to_string_lossy()) {
        return legacy;
    }
    let public = PathBuf::from(r"C:\Users\Public");
    if public.exists() {
        return public.join("speakdrill").join("piper");
    }
    // 萬一連 Public 都沒有（極少數企業鎖死），退回 config dir — piper 可能還是會崩潰，
    // 但至少 init 不會直接放棄
    legacy
}

fn paths_in(dir: &std::path::Path) -> Option<PiperPaths> {
    let exe = [dir.join("piper").join("piper.exe"), dir.join("piper.exe")]
        .into_iter()
        .find(|p| p.exists())?;
    let model = dir.join("en_US-lessac-medium.onnx");
    if !model.exists() {
        return None;
    }
    Some(PiperPaths { exe, model })
}

/// 遞迴複製資料夾（用來遷移完整的 piper 安裝，避免重抓 63 MB 模型）
fn copy_dir_recursive(src: &std::path::Path, dest: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dest.join(entry.file_name());
        let ft = entry.file_type()?;
        if ft.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// 既有安裝在非 ASCII 路徑且模型 intact → 複製到 ASCII 路徑。
/// 只有這種情況才能免重抓；模型壞了還是走下載。
fn migrate_from_legacy(dir: &std::path::Path, log: &mut dyn FnMut(&str)) -> anyhow::Result<bool> {
    let legacy = legacy_piper_dir();
    if !legacy.exists() || any_ascii(&legacy.to_string_lossy()) {
        return Ok(false);
    }
    let Some(p) = paths_in(&legacy) else {
        log("偵測到非 ASCII 的舊安裝，但 exe/模型殘缺 — 改為重新下載");
        return Ok(false);
    };
    if model_integrity(&p).is_err() {
        log("舊安裝的模型也損毀 — 改為重新下載");
        return Ok(false);
    }
    log(&format!(
        "piper 不支援非 ASCII 路徑（{}）— 複製到 ASCII 路徑（{}）…",
        legacy.display(),
        dir.display()
    ));
    copy_dir_recursive(&legacy, dir)?;
    Ok(true)
}

/// 找 piper 安裝位置（`init` 會下載到這裡）
pub fn piper_paths() -> Option<PiperPaths> {
    paths_in(&piper_root())
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
    let dir = piper_root();

    // A. ASCII 位置已有完整安裝 → 驗證完整性 + 自我測試
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

    // B. 沒有可用安裝，但 legacy 在非 ASCII 路徑且模型完整 → 本地複製遷移（不重抓 84 MB）
    if migrate_from_legacy(&dir, log)? {
        let p = piper_paths().ok_or_else(|| anyhow::anyhow!("遷移後仍找不到 piper"))?;
        model_integrity(&p)?;
        log("遷移完成，自我測試…");
        return piper_self_test(&p).map(|_| p);
    }

    // C. 全新安裝：下載 + 驗證 + 自我測試
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
            dir = piper_root().display()
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

    /// 非 ASCII 安裝路徑（中文使用者名稱）是 piper 必崩潰的環境 — 驗證偵測邏輯
    #[test]
    fn piper_root_ascii_detection() {
        assert!(any_ascii("C:\\Users\\Public\\speakdrill"));
        assert!(any_ascii("C:\\Users\\CPXru"));
        assert!(!any_ascii("C:\\Users\\還是的\\AppData"));
        assert!(!any_ascii("C:\\Users\\CPXru\\測試"));
    }

    #[test]
    fn piper_paths_in_empty_dir() {
        let d = std::env::temp_dir().join("speakdrill_empty_piper_dir");
        let _ = std::fs::create_dir_all(&d);
        assert!(paths_in(&d).is_none());
    }

    #[test]
    fn any_ascii_handles_mixed() {
        assert!(any_ascii(""));
        assert!(any_ascii("hello world"));
        assert!(any_ascii("123-abc_def.ghi"));
        assert!(!any_ascii("h\u{4f60}o"));
    }

    #[test]
    #[ignore]
    fn migrate_copies_complete_tree() {
        let src = std::env::temp_dir().join("speakdrill_legacy_fake");
        let dst = std::env::temp_dir().join("speakdrill_migrate_dst");
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dst);
        std::fs::create_dir_all(src.join("piper").join("sub")).unwrap();
        std::fs::write(src.join("piper").join("piper.exe"), b"fake-exe").unwrap();
        std::fs::write(src.join("en_US-lessac-medium.onnx"), b"fake-model").unwrap();
        std::fs::write(src.join("piper").join("sub").join("data.bin"), b"nested").unwrap();
        copy_dir_recursive(&src, &dst).unwrap();
        assert!(dst.join("piper").join("piper.exe").exists());
        assert!(dst.join("en_US-lessac-medium.onnx").exists());
        assert!(dst.join("piper").join("sub").join("data.bin").exists());
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dst);
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
