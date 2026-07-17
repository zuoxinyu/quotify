/// Strip leading 'v'/'V' and trailing pre-release suffix from a version string.
pub(crate) fn normalize_version(v: &str) -> String {
    let v = v.trim().trim_start_matches('v').trim_start_matches('V');
    if let Some((main, _)) = v.split_once('-') {
        main.to_string()
    } else {
        v.to_string()
    }
}

/// Compare two semver-ish version strings; returns true if `latest` > `current`.
pub(crate) fn is_newer(current: &str, latest: &str) -> bool {
    let current_norm = normalize_version(current);
    let latest_norm = normalize_version(latest);

    let current_parts: Vec<u32> = current_norm
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect();
    let latest_parts: Vec<u32> = latest_norm
        .split('.')
        .filter_map(|s| s.parse().ok())
        .collect();

    for i in 0..std::cmp::max(current_parts.len(), latest_parts.len()) {
        let curr = current_parts.get(i).cloned().unwrap_or(0);
        let lat = latest_parts.get(i).cloned().unwrap_or(0);
        if lat > curr {
            return true;
        } else if curr > lat {
            return false;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_version() {
        assert_eq!(normalize_version("v0.1.0-1-gae62f96"), "0.1.0");
        assert_eq!(normalize_version("V1.2.3"), "1.2.3");
        assert_eq!(normalize_version("2.0.0"), "2.0.0");
    }

    #[test]
    fn test_is_newer() {
        assert!(is_newer("v0.1.0-1-gae62f96", "v0.2.0"));
        assert!(is_newer("0.1.0", "v0.1.1"));
        assert!(!is_newer("v0.2.0", "v0.1.0"));
        assert!(!is_newer("v0.1.0", "v0.1.0"));
        assert!(is_newer("v0.1.0", "1.0.0"));
    }
}
