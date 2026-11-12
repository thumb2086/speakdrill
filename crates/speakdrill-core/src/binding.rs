//! 讀取「App 的 per-app 麥克風綁定」。
//!
//! Windows 把不同 role 存在 `DefaultEndpoint\<hash>_0` 的 `001_<role>`。
//! Audio apps normally use the Multimedia role (role = 1); Windows settings often
//! writes only that row. 不能只看 001_000，否則 UI 設定成 CABLE Output 時會誤判。

use std::process::Command;

#[derive(Debug, Clone)]
pub struct AppBinding {
    /// 例如 `001_000 = raw MMDevice path`, `001_001 = raw MMDevice path`
    pub mic_roles: Vec<(String, String)>,
    pub key: String,
}

impl AppBinding {
    /// 從 role 1(Multimedia) 優先，其次 role 0(Console)、role 2(Communications)。
    pub fn best_guid(&self) -> Option<String> {
        for r in ["001_001", "001_000", "001_002"] {
            if let Some((_, v)) = self.mic_roles.iter().find(|(k, _)| k == r) {
                if let Some(g) = extract_guid(v) {
                    return Some(g);
                }
            }
        }
        self.mic_roles.iter().find_map(|(_, v)| extract_guid(v))
    }

    /// 只要任一 role 已對齊就視為通過。
    pub fn any_matches(&self, expected: &str) -> bool {
        self.mic_roles.iter().any(|(_, v)| {
            extract_guid(v)
                .map(|g| g.eq_ignore_ascii_case(expected))
                .unwrap_or(false)
        })
    }
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
    // rg 的 stdout 可能含本地化字碼頁，from_utf8_lossy 不能逐行解析，
    // 但只需要判斷是否含特定 ascii 子字串，這樣就安全。
    let text = String::from_utf8_lossy(&out.stdout);
    let needle = exe_name.to_lowercase();

    let mut current_key = String::new();
    let mut current = String::new();

    for raw_line in text.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.trim_start().starts_with("HKEY_") {
            if let Some(binding) = inspect_chunk(&needle, &current_key, &current) {
                return Some(binding);
            }
            current_key = line.trim_start().to_string();
            current.clear();
            continue;
        }
        current.push_str(line);
        current.push('\n');
    }

    inspect_chunk(&needle, &current_key, &current)
}

fn inspect_chunk(needle: &str, key: &str, chunk: &str) -> Option<AppBinding> {
    if !chunk.to_lowercase().contains(needle) {
        return None;
    }

    let mut mic_roles = Vec::new();
    for line in chunk.lines() {
        let t = line.trim();
        for role in ["001_000", "001_001", "001_002"] {
            if let Some(rest) = t.strip_prefix(role) {
                let rd = rest.trim();
                if let Some(value) = rd.strip_prefix("REG_SZ") {
                    let v = value.trim();
                    if !v.is_empty() {
                        mic_roles.push((role.to_string(), v.to_string()));
                    }
                }
            }
        }
    }

    if mic_roles.is_empty() {
        return None;
    }

    Some(AppBinding {
        mic_roles,
        key: key.to_string(),
    })
}

/// 從 `\\?\SWD#MMDEVAPI#{0.0.0.00000000}.{guid}#{iface}` 取出 `{0.0.0...}.{guid}`
fn extract_guid(path: &str) -> Option<String> {
    let start = path.find("{0.0.")?;
    let after_dot = &path[start..];

    // 取第一個 }} 包住的 flow guid 與第二個 }} 包住的 endpoint guid
    let mut seen = 0;
    let mut end = 0;
    for (i, c) in after_dot.char_indices() {
        if c == '}' {
            seen += 1;
            if seen == 2 {
                end = i;
                break;
            }
        }
    }

    if end == 0 {
        None
    } else {
        Some(after_dot[..=end].to_string())
    }
}
