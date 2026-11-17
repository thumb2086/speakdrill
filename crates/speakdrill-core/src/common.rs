//! 共用工具函數與型別

/// 比較兩個 x.y.z 版本字串（a > b 回傳 true）
pub fn version_gt(a: &str, b: &str) -> bool {
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