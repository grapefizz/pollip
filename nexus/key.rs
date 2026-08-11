use super::domain::KEY_FILE_NAME;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

#[derive(Debug)]
pub enum KeyError {
    HomeUnavailable,
    Io(io::Error),
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HomeUnavailable => {
                write!(formatter, "HOME is unset; cannot store nexus api key")
            }
            Self::Io(error) => write!(formatter, "nexus key io error: {error}"),
        }
    }
}

impl std::error::Error for KeyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for KeyError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn key_path() -> Result<PathBuf, KeyError> {
    Ok(crate::platform::data_directory()
        .map_err(|_| KeyError::HomeUnavailable)?
        .join(KEY_FILE_NAME))
}

pub fn load_api_key() -> Result<Option<String>, KeyError> {
    let path = key_path()?;
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path)?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_string()))
    }
}

pub fn save_api_key(key: &str) -> Result<(), KeyError> {
    let path = key_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("key.partial");
    {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(key.trim().as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    fs::rename(temporary, &path)?;
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    Ok(())
}

pub fn clear_api_key() -> Result<(), KeyError> {
    let path = key_path()?;
    if path.is_file() {
        fs::remove_file(path)?;
    }
    Ok(())
}
