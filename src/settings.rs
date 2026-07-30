use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    private_fs::{PrivatePathError, ensure_private_directory, set_private_permissions},
    session::SessionId,
};

const SETTINGS_SCHEMA_VERSION: u32 = 2;
const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;
const MAX_KEYBOARD_VALUE_BYTES: usize = 256;
const MAX_TEMP_FILE_ATTEMPTS: usize = 100;
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Settings {
    schema_version: u32,
    storage: StorageSettings,
    keyboard: KeyboardSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            storage: StorageSettings::default(),
            keyboard: KeyboardSettings::default(),
        }
    }
}

impl Settings {
    pub fn storage_disclosure_acknowledged(&self) -> bool {
        self.storage.disclosure_acknowledged
    }

    pub fn acknowledge_storage_disclosure(&mut self) {
        self.storage.disclosure_acknowledged = true;
    }

    pub fn autosave_enabled(&self) -> bool {
        self.storage.autosave_enabled
    }

    pub fn set_autosave_enabled(&mut self, enabled: bool) {
        self.storage.autosave_enabled = enabled;
    }

    pub fn last_session_id(&self) -> Option<SessionId> {
        self.storage.last_session_id.and_then(SessionId::new)
    }

    pub fn set_last_session_id(&mut self, session_id: Option<SessionId>) {
        self.storage.last_session_id = session_id.map(SessionId::get);
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
        if self.storage.last_session_id.is_some_and(|id| id <= 0) {
            return Err(SettingsError::InvalidSessionId);
        }
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

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
struct StorageSettings {
    disclosure_acknowledged: bool,
    autosave_enabled: bool,
    last_session_id: Option<i64>,
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
        let metadata = match fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Settings::default());
            }
            Err(source) => return Err(SettingsError::ReadMetadata { source }),
        };
        if !metadata.is_file() {
            return Err(SettingsError::NotRegularFile);
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

        let file_name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(SettingsError::InvalidFileName)?;
        let (temporary, mut file) = create_temporary_file(parent, file_name, &TEMP_FILE_COUNTER)?;

        let result = (|| {
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

fn create_temporary_file(
    parent: &Path,
    file_name: &str,
    counter: &AtomicU64,
) -> Result<(PathBuf, File), SettingsError> {
    for _ in 0..MAX_TEMP_FILE_ATTEMPTS {
        let sequence = counter.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            ".{file_name}.tmp-{}-{sequence}",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(SettingsError::CreateTemporary { source }),
        }
    }
    Err(SettingsError::TemporaryNameExhausted)
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("settings path has no parent directory")]
    MissingParent,
    #[error("settings path has an invalid file name")]
    InvalidFileName,
    #[error("settings path is not a regular file")]
    NotRegularFile,
    #[error("settings file exceeds the size limit")]
    FileTooLarge,
    #[error("settings schema version {actual} is unsupported; expected {supported}")]
    UnsupportedSchemaVersion { supported: u32, actual: u32 },
    #[error("last session ID is invalid")]
    InvalidSessionId,
    #[error("keyboard setting exceeds the size limit")]
    KeyboardValueTooLong,
    #[error("failed to read settings metadata")]
    ReadMetadata {
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    PrivatePath(#[from] PrivatePathError),
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
    #[error("could not find an unused temporary settings file name")]
    TemporaryNameExhausted,
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
    use std::{fs, os::unix::fs::PermissionsExt as _, sync::atomic::AtomicU64};

    use tempfile::tempdir;

    use crate::session::SessionId;

    use super::{
        MAX_TEMP_FILE_ATTEMPTS, Settings, SettingsError, SettingsStore, create_temporary_file,
    };

    #[test]
    fn missing_settings_use_defaults_without_creating_files() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("config/evtap/settings.json");
        let store = SettingsStore::new(path.clone());

        let settings = store.load().unwrap();

        assert!(!settings.storage_disclosure_acknowledged());
        assert!(!settings.autosave_enabled());
        assert_eq!(settings.last_session_id(), None);
        assert!(!path.exists());
    }

    #[test]
    fn settings_round_trip_atomically_with_private_permissions() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("config/evtap/settings.json");
        let store = SettingsStore::new(path.clone());
        let mut settings = Settings::default();
        settings.acknowledge_storage_disclosure();
        settings.set_autosave_enabled(true);
        settings.set_last_session_id(SessionId::new(7));
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
    fn temporary_file_creation_retries_collisions_and_is_bounded() {
        let temporary = tempdir().unwrap();
        let parent = temporary.path();
        let process_id = std::process::id();
        fs::write(
            parent.join(format!(".settings.json.tmp-{process_id}-0")),
            [],
        )
        .unwrap();
        let counter = AtomicU64::new(0);

        let (path, file) = create_temporary_file(parent, "settings.json", &counter).unwrap();

        drop(file);
        assert!(path.ends_with(format!(".settings.json.tmp-{process_id}-1")));

        let exhausted = tempdir().unwrap();
        for sequence in 0..MAX_TEMP_FILE_ATTEMPTS {
            fs::write(
                exhausted
                    .path()
                    .join(format!(".settings.json.tmp-{process_id}-{sequence}")),
                [],
            )
            .unwrap();
        }
        let counter = AtomicU64::new(0);
        assert!(matches!(
            create_temporary_file(exhausted.path(), "settings.json", &counter),
            Err(SettingsError::TemporaryNameExhausted)
        ));
    }

    #[test]
    fn unsupported_schema_is_not_silently_replaced() {
        let temporary = tempdir().unwrap();
        let path = temporary.path().join("settings.json");
        fs::write(&path, r#"{"schema_version":1}"#).unwrap();
        let original = fs::read(&path).unwrap();
        let store = SettingsStore::new(path.clone());

        assert!(matches!(
            store.load(),
            Err(SettingsError::UnsupportedSchemaVersion { actual: 1, .. })
        ));
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn follows_symlinked_settings_and_rejects_oversized_files() {
        let temporary = tempdir().unwrap();
        let target = temporary.path().join("target.json");
        let path = temporary.path().join("settings.json");
        fs::write(&target, serde_json::to_vec(&Settings::default()).unwrap()).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();

        assert_eq!(
            SettingsStore::new(path).load().unwrap(),
            Settings::default()
        );

        let oversized = temporary.path().join("oversized.json");
        fs::write(&oversized, vec![b' '; 1024 * 1024 + 1]).unwrap();
        assert!(matches!(
            SettingsStore::new(oversized).load(),
            Err(SettingsError::FileTooLarge)
        ));
    }
}
