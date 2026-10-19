//! 常駐で使う既定の置き場（純粋）。design 05「常駐と配布」、Windows は design 07。
//! 同梱 plist は絶対パスを書けないので、ログは midistaged 自身が書く。
use std::path::{Path, PathBuf};

/// 既定の置き場の根を持つ環境変数。macOS は HOME、Windows は LOCALAPPDATA
#[cfg(not(windows))]
pub const ROOT_VAR: &str = "HOME";
#[cfg(windows)]
pub const ROOT_VAR: &str = "LOCALAPPDATA";

#[cfg(not(windows))]
const STATE: &[&str] = &["Library/Application Support/Midistage"];
#[cfg(not(windows))]
const LOGS: &[&str] = &["Library/Logs/midistaged"];
#[cfg(windows)]
const STATE: &[&str] = &["Midistage"];
#[cfg(windows)]
const LOGS: &[&str] = &["Midistage", "logs"];

/// 状態の置き場。`--state-dir` が無ければ `~/Library/Application Support/Midistage`
/// （Windows は `%LOCALAPPDATA%\Midistage`）
pub fn state_dir(root: Option<&Path>, override_: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    under_root(root, override_, STATE, "--state-dir")
}

/// ログの置き場。`--log-dir` が無ければ `~/Library/Logs/midistaged`
/// （Windows は `%LOCALAPPDATA%\Midistage\logs`）
pub fn log_dir(root: Option<&Path>, override_: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    under_root(root, override_, LOGS, "--log-dir")
}

fn under_root(
    root: Option<&Path>,
    override_: Option<PathBuf>,
    segments: &[&str],
    flag: &str,
) -> anyhow::Result<PathBuf> {
    if let Some(dir) = override_ {
        return Ok(dir);
    }
    let root = root.ok_or_else(|| anyhow::anyhow!("{ROOT_VAR} is unavailable; pass {flag}"))?;
    Ok(segments.iter().fold(root.to_path_buf(), |p, s| p.join(s)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
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

    #[cfg(windows)]
    #[test]
    fn defaults_live_under_local_app_data_and_overrides_win() {
        let root = Path::new(r"C:\Users\x\AppData\Local");
        assert_eq!(ROOT_VAR, "LOCALAPPDATA");
        assert_eq!(
            state_dir(Some(root), None).unwrap(),
            PathBuf::from(r"C:\Users\x\AppData\Local\Midistage")
        );
        assert_eq!(
            log_dir(Some(root), None).unwrap(),
            PathBuf::from(r"C:\Users\x\AppData\Local\Midistage\logs")
        );
        assert_eq!(
            log_dir(Some(root), Some(r"D:\l".into())).unwrap(),
            PathBuf::from(r"D:\l")
        );
    }

    #[test]
    fn without_root_only_overrides_work() {
        assert!(state_dir(None, None).is_err());
        assert!(log_dir(None, None).is_err());
        assert_eq!(
            log_dir(None, Some("/tmp/l".into())).unwrap(),
            PathBuf::from("/tmp/l")
        );
    }
}
