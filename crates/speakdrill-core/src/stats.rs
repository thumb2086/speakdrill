//! 讀 MyET 官方統計（學習顧問室 Dashboard.aspx）
//!
//! 策略：直接連「目前這一頁」（不開新分頁 — CEF 新分頁不會導航），
//! 用 JS 把它導航到 Dashboard.aspx，等內容載入，讀 innerText 抓
//! 「學習時間 / 證書 / LV」，完了導回原頁（盡力而為）。
//!
//! 注意：呼叫時會短暫把 MyET 切到 Dashboard.aspx。

use anyhow::{anyhow, Result};

/// MyET 官方累計統計
#[derive(Debug, Clone)]
pub struct MyetStats {
    /// 累計學習時數（小時）
    pub hours: f64,
    /// 經驗值等級（例如 LV 44）
    pub level: Option<String>,
    /// 證書數
    pub certs: Option<u64>,
}

/// Dashboard.aspx 網址
pub fn dashboard_url(server: &str) -> String {
    format!("{}/AudreyWeb/Dashboard.aspx", server.trim_end_matches('/'))
}

/// 讀取 MyET 官方累計學習時數；失敗回傳 Err（呼叫端可退回本機累計）
pub async fn fetch_myet_stats(server: &str, port: u16) -> Result<MyetStats> {
    let host = "127.0.0.1";
    let cdp = crate::cdp::Cdp::connect(host, port).await?;
    let original_url = cdp.target_url.clone();

    // 導航到 Dashboard.aspx（此頁面顯示 累積 時數 / LV / 證書）
    cdp.navigate(&dashboard_url(server)).await?;

    // 等 innerText 填上「學習時間」（非同步內容，最多 8 秒）
    let mut text = String::new();
    for _ in 0..16 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if let Ok(t) = cdp.body_text().await {
            text = t;
            if text.contains("學習時間") {
                break;
            }
        }
    }

    // 導回原頁（盡力而為；about:blank 之類的就不用導回）
    if !original_url.is_empty() && !original_url.starts_with("about:blank") {
        let _ = cdp.navigate(&original_url).await;
    }

    if text.is_empty() {
        return Err(anyhow!("讀不到 Dashboard.aspx 內容"));
    }
    parse_myet_stats(&text)
}

fn parse_myet_stats(text: &str) -> Result<MyetStats> {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    let mut hours: Option<f64> = None;
    let mut level: Option<String> = None;
    let mut certs: Option<u64> = None;

    let re = regex::Regex::new(r"([0-9]+(?:\.[0-9]+)?)\s*(時|小時|分)").unwrap();
    for (i, l) in lines.iter().enumerate() {
        match *l {
            "學習時間" => {
                if let Some(next) = lines.get(i + 1) {
                    if let Some(c) = re.captures(next) {
                        let v: f64 = c[1].parse().unwrap_or(0.0);
                        hours = Some(if &c[2] == "分" { v / 60.0 } else { v });
                    }
                }
            }
            "證書" => {
                if let Some(n) = lines.get(i + 1).and_then(|s| s.parse::<u64>().ok()) {
                    certs = Some(n);
                }
            }
            _ => {
                if let Some(rest) = l.strip_prefix("LV") {
                    let lv = rest.trim();
                    if !lv.is_empty() {
                        level = Some(format!("LV {lv}"));
                    }
                }
            }
        }
    }

    let hours = hours.ok_or_else(|| anyhow!("找不到『學習時間』欄位"))?;
    Ok(MyetStats { hours, level, certs })
}

#[cfg(test)]
mod live_tests {
    #[test]
    #[ignore = "requires local MyET running with CDP 9222"]
    fn fetch_live() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cfg = crate::config::Config::load_or_default();
        let r = rt.block_on(crate::fetch_myet_stats(&cfg.server, cfg.cdp_port));
        eprintln!("[live] result: {:?}", r);
        assert!(r.is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::parse_myet_stats;

    #[test]
    fn parses_hours_minutes_level_certs() {
        let text = "學習顧問室\nB11511029\n國立雲林科技大學\n證書 \n16\n \n經驗值\nLV 44\n \n學習時間\n15.2 時\n";
        let s = parse_myet_stats(text).unwrap();
        assert!((s.hours - 15.2).abs() < 1e-6);
        assert_eq!(s.level.as_deref(), Some("LV 44"));
        assert_eq!(s.certs, Some(16));
    }

    #[test]
    fn parses_minutes_as_hours() {
        let text = "證書\n16\nLV 44\n學習時間\n90 分\n";
        let s = parse_myet_stats(text).unwrap();
        assert!((s.hours - 1.5).abs() < 1e-6);
    }
}
