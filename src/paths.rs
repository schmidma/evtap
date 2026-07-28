#![allow(
    dead_code,
    reason = "application paths are consumed by the storage worker milestone"
)]

use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use thiserror::Error;

const APPLICATION_NAME: &str = "evtap";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppPaths {
    config_dir: PathBuf,
    data_dir: PathBuf,
}

impl AppPaths {
    pub fn discover() -> Result<Self, AppPathsError> {
        let project = ProjectDirs::from("", "", APPLICATION_NAME)
            .ok_or(AppPathsError::DirectoriesUnavailable)?;
        Ok(Self {
            config_dir: project.config_dir().to_path_buf(),
            data_dir: project.data_dir().to_path_buf(),
        })
    }

    #[cfg(test)]
    pub fn with_roots(config_dir: PathBuf, data_dir: PathBuf) -> Self {
        Self {
            config_dir,
            data_dir,
        }
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }

    pub fn eframe_file(&self) -> PathBuf {
        self.data_dir.join("app.ron")
    }

    pub fn database_file(&self) -> PathBuf {
        self.data_dir.join("evtap.sqlite3")
    }
}

#[derive(Debug, Error)]
pub enum AppPathsError {
    #[error("could not determine configuration and data directories")]
    DirectoriesUnavailable,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::AppPaths;

    #[test]
    fn constructs_separate_settings_and_data_paths() {
        let paths =
            AppPaths::with_roots(PathBuf::from("/config/evtap"), PathBuf::from("/data/evtap"));

        assert_eq!(
            paths.settings_file(),
            PathBuf::from("/config/evtap/settings.json")
        );
        assert_eq!(paths.eframe_file(), PathBuf::from("/data/evtap/app.ron"));
        assert_eq!(
            paths.database_file(),
            PathBuf::from("/data/evtap/evtap.sqlite3")
        );
    }
}
