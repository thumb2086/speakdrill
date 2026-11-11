//! 檢查「App 的 per-app 麥克風綁定」是否符合我們設定的迴路。
//!
//! Windows 把 per-app 裝置存在
//! `HKCU\Software\Microsoft\Multimedia\Audio\DefaultEndpoint\<hash>_0`
//! 的 `001_000`（錄音）與 `000_000`（播放）。
//!
//! 這裡用 `reg query` 讀（避免多拉一個 crate），只比對 `{0.0.1.00000000}.{guid}` 這段。

use std::process::Command;

pub struct AppBinding {
    /// 例如 `{0.0.1.00000000}.{a70fcea2-...}`
    pub mic_guid: Option<String>,
    pub key: String,
}

/// 找出某支執行檔的 per-app 錄音裝置
pub fn app_mic_binding(exe_name: &str) -> Option<AppBinding> {
    let out = Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Multimedia\Audio\DefaultEndpoint",
            "/s",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);

    let mut current_key = String::new();
    let mut matched_key: Option<String> = None;
    let mut mic_by_key: std::collections::HashMap<String, String> = Default::default();

    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("HKEY_") {
            current_key = t.to_string();
            continue;
        }
        // 值行格式：<name>    REG_SZ    <data>
        let mut parts = t.splitn(3, "    ");
        let name = match parts.next() {
            Some(n) => n.trim(),
            None => continue,
        };
        let rest = parts.next();
        let data = parts.next().map(|d| d.trim()).unwrap_or("");
        if rest.is_none() || data.is_empty() {
            continue;
        }

        if name == "001_000" {
            mic_by_key.insert(current_key.clone(), data.to_string());
        } else if data.to_lowercase().contains(&exe_name.to_lowercase()) {
            // 這是該 app 的 (預設值) → exe 路徑
            matched_key = Some(current_key.clone());
        }
    }

    let key = matched_key?;
    let raw = mic_by_key.get(&key).cloned()?;
    Some(AppBinding {
        mic_guid: extract_guid(&raw),
        key,
    })
}

/// 從 `\\?\SWD#MMDEVAPI#{0.0.1.00000000}.{guid}#{iface}` 取出 `{0.0.1.00000000}.{guid}`
fn extract_guid(path: &str) -> Option<String> {
    let start = path.find("{0.0.")?;
    let rest = &path[start..];
    // 找第二個 '}'（`{0.0.1.00000000}` 與 `{guid}`）
    let mut seen = 0;
    for (i, c) in rest.char_indices() {
        if c == '}' {
            seen += 1;
            if seen == 2 {
                return Some(rest[..=i].to_string());
            }
        }
    }
    None
}
