use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const SETTINGS_SCHEMA_VERSION: u32 = 1;
const DEFAULT_RETENTION_DAYS: u32 = 90;
const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;
const MAX_KEYBOARD_VALUE_BYTES: usize = 256;
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetentionPolicy {
    Days(u32),
    Forever,
}

impl RetentionPolicy {
    pub fn days(self) -> Option<u32> {
        match self {
            Self::Days(days) => Some(days),
            Self::Forever => None,
        }
    }

    fn validate(self) -> Result<(), SettingsError> {
        match self {
            Self::Days(30 | 90 | 365) | Self::Forever => Ok(()),
            Self::Days(_) => Err(SettingsError::InvalidRetention),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Settings {
    schema_version: u32,
    persistence: PersistenceSettings,
    keyboard: KeyboardSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            persistence: PersistenceSettings::default(),
            keyboard: KeyboardSettings::default(),
        }
    }
}

impl Settings {
    pub fn persistence_enabled(&self) -> bool {
        self.persistence.enabled
    }

    pub fn set_persistence_enabled(&mut self, enabled: bool) {
        self.persistence.enabled = enabled;
    }

    pub fn retention(&self) -> RetentionPolicy {
        self.persistence
            .retention_days
            .map_or(RetentionPolicy::Forever, RetentionPolicy::Days)
    }

    pub fn set_retention(&mut self, retention: RetentionPolicy) -> Result<(), SettingsError> {
        retention.validate()?;
        self.persistence.retention_days = retention.days();
        Ok(())
    }

    pub fn keyboard_model(&self) -> &str {
        &self.keyboard.model
    }

    pub fn keyboard_layout(&self) -> &str {
        &self.keyboard.layout
    }

    pub fn keyboard_variant(&self) -> &str {
        &self.keyboard.variant
    }

    pub fn set_keyboard(&mut self, model: String, layout: String, variant: String) {
        self.keyboard = KeyboardSettings {
            model,
            layout,
            variant,
        };
    }

    fn validate(&self) -> Result<(), SettingsError> {
        if self.schema_version != SETTINGS_SCHEMA_VERSION {
            return Err(SettingsError::UnsupportedSchemaVersion {
                supported: SETTINGS_SCHEMA_VERSION,
                actual: self.schema_version,
            });
        }
        self.retention().validate()?;
        for value in [
            &self.keyboard.model,
            &self.keyboard.layout,
            &self.keyboard.variant,
        ] {
            if value.len() > MAX_KEYBOARD_VALUE_BYTES {
                return Err(SettingsError::KeyboardValueTooLong);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct PersistenceSettings {
    enabled: bool,
    retention_days: Option<u32>,
}

impl Default for PersistenceSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            retention_days: Some(DEFAULT_RETENTION_DAYS),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct KeyboardSettings {
    model: String,
    layout: String,
    variant: String,
}

#[derive(Clone, Debug)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn load(&self) -> Result<Settings, SettingsError> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Settings::default());
            }
            Err(source) => return Err(SettingsError::ReadMetadata { source }),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(SettingsError::UnsafePath);
        }
        if metadata.len() > MAX_SETTINGS_BYTES {
            return Err(SettingsError::FileTooLarge);
        }
        let parent = self.path.parent().ok_or(SettingsError::MissingParent)?;
        ensure_private_directory(parent)?;
        set_private_permissions(&self.path, 0o600)?;

        let file = File::open(&self.path).map_err(|source| SettingsError::Open { source })?;
        let mut contents = String::new();
        file.take(MAX_SETTINGS_BYTES + 1)
            .read_to_string(&mut contents)
            .map_err(|source| SettingsError::Read { source })?;
        if contents.len() as u64 > MAX_SETTINGS_BYTES {
            return Err(SettingsError::FileTooLarge);
        }

        let settings: Settings =
            serde_json::from_str(&contents).map_err(|source| SettingsError::Decode { source })?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn save(&self, settings: &Settings) -> Result<(), SettingsError> {
        settings.validate()?;
        let mut encoded = serde_json::to_vec_pretty(settings)
            .map_err(|source| SettingsError::Encode { source })?;
        encoded.push(b'\n');
        if encoded.len() as u64 > MAX_SETTINGS_BYTES {
            return Err(SettingsError::FileTooLarge);
        }

        let parent = self.path.parent().ok_or(SettingsError::MissingParent)?;
        ensure_private_directory(parent)?;
        reject_existing_symlink(&self.path)?;

        let file_name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(SettingsError::InvalidFileName)?;
        let sequence = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".{file_name}.tmp-{}-{sequence}",
            std::process::id()
        ));

        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)
                .map_err(|source| SettingsError::CreateTemporary { source })?;
            file.write_all(&encoded)
                .map_err(|source| SettingsError::Write { source })?;
            file.sync_all()
                .map_err(|source| SettingsError::Sync { source })?;
            drop(file);
            fs::rename(&temporary, &self.path)
                .map_err(|source| SettingsError::Replace { source })?;
            set_private_permissions(&self.path, 0o600)?;
            File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|source| SettingsError::SyncDirectory { source })?;
            Ok(())
        })();

        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

fn ensure_private_directory(path: &Path) -> Result<(), SettingsError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(SettingsError::UnsafeDirectory);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder
                .create(path)
                .map_err(|source| SettingsError::CreateDirectory { source })?;
        }
        Err(source) => return Err(SettingsError::ReadDirectoryMetadata { source }),
    }
    set_private_permissions(path, 0o700)
}

fn reject_existing_symlink(path: &Path) -> Result<(), SettingsError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(SettingsError::UnsafePath)
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(SettingsError::ReadMetadata { source }),
    }
}

fn set_private_permissions(path: &Path, mode: u32) -> Result<(), SettingsError> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|source| SettingsError::SetPermissions { source })
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("settings path has no parent directory")]
    MissingParent,
    #[error("settings path has an invalid file name")]
    InvalidFileName,
    #[error("settings path is a symbolic link or not a regular file")]
    UnsafePath,
    #[error("settings directory is a symbolic link or not a directory")]
    UnsafeDirectory,
    #[error("settings file exceeds the size limit")]
    FileTooLarge,
    #[error("settings schema version {actual} is unsupported; expected {supported}")]
    UnsupportedSchemaVersion { supported: u32, actual: u32 },
    #[error("settings retention must be 30, 90, 365 days, or forever")]
    InvalidRetention,
    #[error("keyboard setting exceeds the size limit")]
    KeyboardValueTooLong,
    #[error("failed to read settings metadata")]
    ReadMetadata {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read settings directory metadata")]
    ReadDirectoryMetadata {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create settings directory")]
    CreateDirectory {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to set private settings permissions")]
    SetPermissions {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to open settings")]
    Open {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read settings")]
    Read {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to decode settings")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to encode settings")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to create temporary settings file")]
    CreateTemporary {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write settings")]
    Write {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to synchronize settings")]
    Sync {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to replace settings")]
    Replace {
        #[source]
        source: std::io::Error,
    },
    #[error("failed to synchronize settings directory")]
    SyncDirectory {
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt as _, symlink},
    };

    use tempfile::tempdir;

    use super::{RetentionPolicy, Settings, SettingsError, SettingsStore};

    #[test]
    fn missing_settings_use_defaults_without_creating_files() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("config/evtap/settings.json");
        let store = SettingsStore::new(path.clone());

        let settings = store.load().unwrap();

        assert!(!settings.persistence_enabled());
        assert_eq!(settings.retention(), RetentionPolicy::Days(90));
        assert!(!path.exists());
        assert!(!path.parent().unwrap().exists());
    }

    #[test]
    fn settings_round_trip_atomically_with_private_permissions() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("config/evtap/settings.json");
        let store = SettingsStore::new(path.clone());
        let mut settings = Settings::default();
        settings.set_persistence_enabled(true);
        settings.set_retention(RetentionPolicy::Forever).unwrap();
        settings.set_keyboard("pc105".to_owned(), "de".to_owned(), "nodeadkeys".to_owned());

        store.save(&settings).unwrap();

        assert_eq!(store.load().unwrap(), settings);
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn unsupported_schema_is_not_silently_replaced() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("settings.json");
        fs::write(
            &path,
            r#"{"schema_version":2,"persistence":{"enabled":true,"retention_days":90}}"#,
        )
        .unwrap();
        let original = fs::read(&path).unwrap();
        let store = SettingsStore::new(path.clone());

        assert!(matches!(
            store.load(),
            Err(SettingsError::UnsupportedSchemaVersion { actual: 2, .. })
        ));
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn rejects_symlinked_and_oversized_settings_without_modifying_them() {
        let temporary = tempdir().unwrap();
        let target = temporary.path().join("target.json");
        let path = temporary.path().join("settings.json");
        fs::write(&target, b"sensitive target").unwrap();
        symlink(&target, &path).unwrap();
        let store = SettingsStore::new(path);

        assert!(matches!(store.load(), Err(SettingsError::UnsafePath)));
        assert!(matches!(
            store.save(&Settings::default()),
            Err(SettingsError::UnsafePath)
        ));
        assert_eq!(fs::read(&target).unwrap(), b"sensitive target");

        let oversized = temporary.path().join("oversized.json");
        fs::write(&oversized, vec![b' '; 1024 * 1024 + 1]).unwrap();
        assert!(matches!(
            SettingsStore::new(oversized).load(),
            Err(SettingsError::FileTooLarge)
        ));
    }

    #[test]
    fn rejects_unsupported_retention() {
        let mut settings = Settings::default();
        assert!(matches!(
            settings.set_retention(RetentionPolicy::Days(7)),
            Err(SettingsError::InvalidRetention)
        ));
    }
}
