use std::{
    fs::{self, DirBuilder},
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
};

use thiserror::Error;

/// Creates or tightens a private directory using normal operating-system path resolution.
///
/// `fs::metadata` intentionally follows symbolic links. evtap does not impose a special
/// no-symlink policy; callers still receive normal type and I/O errors.
pub fn ensure_private_directory(path: &Path) -> Result<(), PrivatePathError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(PrivatePathError::NotDirectory(path.to_path_buf())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder
                .create(path)
                .map_err(|source| PrivatePathError::CreateDirectory {
                    path: path.to_path_buf(),
                    source,
                })?;
        }
        Err(source) => {
            return Err(PrivatePathError::ReadMetadata {
                path: path.to_path_buf(),
                source,
            });
        }
    }
    set_private_permissions(path, 0o700)
}

pub fn set_private_permissions(path: &Path, mode: u32) -> Result<(), PrivatePathError> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|source| {
        PrivatePathError::SetPermissions {
            path: path.to_path_buf(),
            mode,
            source,
        }
    })
}

#[derive(Debug, Error)]
pub enum PrivatePathError {
    #[error("private path is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("failed to read private path metadata: {path}")]
    ReadMetadata {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create private directory: {path}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to set private path permissions to {mode:#o}: {path}")]
    SetPermissions {
        path: PathBuf,
        mode: u32,
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

    use super::ensure_private_directory;

    #[test]
    fn private_directories_follow_normal_symlink_resolution() {
        let temporary = tempfile::tempdir().unwrap();
        let target = temporary.path().join("target");
        let link = temporary.path().join("link");
        fs::create_dir(&target).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        symlink(&target, &link).unwrap();

        ensure_private_directory(&link).unwrap();

        assert_eq!(
            fs::metadata(target).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}
