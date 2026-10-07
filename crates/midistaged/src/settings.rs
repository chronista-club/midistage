use midistage_protocol::Assignment;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedSettings {
    pub version: u32,
    pub devices: BTreeMap<String, SavedDevice>,
    #[serde(default)]
    pub initialized_clients: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedDevice {
    pub profile_id: String,
    pub name: String,
    pub assignment: Assignment,
}

pub struct SettingsStore {
    pub directory: PathBuf,
}
pub struct RuntimeLock {
    _file: std::fs::File,
}
impl RuntimeLock {
    pub fn acquire(directory: &Path) -> anyhow::Result<Self> {
        use std::os::{
            fd::AsRawFd,
            unix::fs::{OpenOptionsExt, PermissionsExt},
        };
        std::fs::create_dir_all(directory)?;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(directory.join("runtime.lock"))?;
        // inode は削除しない。close で lock を解放し、全起動が同じ inode を使う。
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            anyhow::bail!(
                "midistaged already running or lock unavailable: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(Self { _file: file })
    }
}
impl SettingsStore {
    pub fn new(directory: &Path) -> Self {
        Self {
            directory: directory.into(),
        }
    }
    pub fn load(&self) -> anyhow::Result<SavedSettings> {
        let path = self.directory.join("settings.json");
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SavedSettings {
                    version: 1,
                    ..Default::default()
                });
            }
            Err(error) => return Err(error.into()),
        };
        let settings: SavedSettings = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            settings.version == 1,
            "unsupported settings version {}",
            settings.version
        );
        Ok(settings)
    }
    pub fn save(&self, settings: &SavedSettings) -> anyhow::Result<()> {
        anyhow::ensure!(settings.version == 1, "unsupported settings version");
        atomic_write(
            &self.directory,
            "settings.json",
            &serde_json::to_vec_pretty(settings)?,
        )
    }
}

/// rename の対象と別 inode を flock する。サービス単一起動 lock は runtime.lock。
pub fn atomic_write(directory: &Path, name: &str, bytes: &[u8]) -> anyhow::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    std::fs::create_dir_all(directory)?;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    let temporary = directory.join(format!(".{name}.{}", uuid::Uuid::new_v4()));
    let result = (|| -> anyhow::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, directory.join(name))?;
        std::fs::File::open(directory)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_one_runtime_can_hold_a_directory_and_exit_releases_it() {
        let dir = tempfile::tempdir().unwrap();
        let first = RuntimeLock::acquire(dir.path()).unwrap();
        assert!(RuntimeLock::acquire(dir.path()).is_err());
        drop(first);
        assert!(RuntimeLock::acquire(dir.path()).is_ok());
    }
    #[test]
    fn saved_off_survives_reopening_without_runtime_lease() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        let settings = SavedSettings {
            version: 1,
            devices: BTreeMap::from([(
                "nano-1".into(),
                SavedDevice {
                    profile_id: "nanokontrol".into(),
                    name: "nanoKONTROL2".into(),
                    assignment: Assignment {
                        client_id: None,
                        revision: 7,
                        expected: true,
                    },
                },
            )]),
            initialized_clients: BTreeSet::new(),
        };
        store.save(&settings).unwrap();
        assert_eq!(SettingsStore::new(dir.path()).load().unwrap(), settings);
    }
    #[test]
    fn corrupt_or_future_settings_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        let file = dir.path().join("settings.json");
        std::fs::write(&file, "{broken").unwrap();
        assert!(store.load().is_err());
        std::fs::write(&file, r#"{"version":99,"devices":{}}"#).unwrap();
        assert!(store.load().is_err());
    }
}
