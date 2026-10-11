//! 常駐で使う既定の置き場（純粋）。design 05「常駐と配布」。
//! 同梱 plist は絶対パスを書けないので、ログは midistaged 自身が書く。
use std::path::{Path, PathBuf};

/// 状態の置き場。`--state-dir` が無ければ `~/Library/Application Support/Midistage`
pub fn state_dir(home: Option<&Path>, override_: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(dir) = override_ {
        return Ok(dir);
    }
    Ok(home
        .ok_or_else(|| anyhow::anyhow!("HOME is unavailable; pass --state-dir"))?
        .join("Library/Application Support/Midistage"))
}

/// ログの置き場。`--log-dir` が無ければ `~/Library/Logs/midistaged`
pub fn log_dir(home: Option<&Path>, override_: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(dir) = override_ {
        return Ok(dir);
    }
    Ok(home
        .ok_or_else(|| anyhow::anyhow!("HOME is unavailable; pass --log-dir"))?
        .join("Library/Logs/midistaged"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_live_under_home_and_overrides_win() {
        let home = Path::new("/Users/x");
        assert_eq!(
            state_dir(Some(home), None).unwrap(),
            PathBuf::from("/Users/x/Library/Application Support/Midistage")
        );
        assert_eq!(
            log_dir(Some(home), None).unwrap(),
            PathBuf::from("/Users/x/Library/Logs/midistaged")
        );
        assert_eq!(
            log_dir(Some(home), Some("/tmp/l".into())).unwrap(),
            PathBuf::from("/tmp/l")
        );
    }

    #[test]
    fn without_home_only_overrides_work() {
        assert!(state_dir(None, None).is_err());
        assert!(log_dir(None, None).is_err());
        assert_eq!(
            log_dir(None, Some("/tmp/l".into())).unwrap(),
            PathBuf::from("/tmp/l")
        );
    }
}
