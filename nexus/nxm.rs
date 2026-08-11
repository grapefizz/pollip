#[cfg(not(target_os = "macos"))]
use super::domain::{NXM_DESKTOP_FILE, NXM_MIME};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NxmLink {
    pub game_domain: String,
    pub mod_id: u64,
    pub file_id: u64,
    pub key: Option<String>,
    pub expires: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Debug)]
pub enum NxmError {
    InvalidUrl(String),
    HomeUnavailable,
    Io(io::Error),
    Handler(String),
}

impl std::fmt::Display for NxmError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUrl(detail) => write!(formatter, "invalid nxm url: {detail}"),
            Self::HomeUnavailable => write!(formatter, "HOME is unset; cannot register nxm handler"),
            Self::Io(error) => write!(formatter, "nxm io error: {error}"),
            Self::Handler(detail) => write!(formatter, "nxm handler error: {detail}"),
        }
    }
}

impl std::error::Error for NxmError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for NxmError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn parse_nxm_url(raw: &str) -> Result<NxmLink, NxmError> {
    let trimmed = raw.trim();
    let without_scheme = trimmed
        .strip_prefix("nxm://")
        .or_else(|| trimmed.strip_prefix("NXM://"))
        .ok_or_else(|| NxmError::InvalidUrl("missing nxm:// scheme".to_string()))?;

    let (path_part, query_part) = match without_scheme.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (without_scheme, None),
    };

    let segments: Vec<&str> = path_part.split('/').filter(|part| !part.is_empty()).collect();
    if segments.len() < 5 || segments[1] != "mods" || segments[3] != "files" {
        return Err(NxmError::InvalidUrl(format!(
            "expected nxm://{{game}}/mods/{{id}}/files/{{fileId}}, got {trimmed}"
        )));
    }

    let game_domain = segments[0].to_string();
    let mod_id: u64 = segments[2]
        .parse()
        .map_err(|_| NxmError::InvalidUrl(format!("bad mod id '{}'", segments[2])))?;
    let file_id: u64 = segments[4]
        .parse()
        .map_err(|_| NxmError::InvalidUrl(format!("bad file id '{}'", segments[4])))?;

    let mut key = None;
    let mut expires = None;
    let mut user_id = None;
    if let Some(query) = query_part {
        for pair in query.split('&') {
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            match name {
                "key" => key = Some(value.to_string()),
                "expires" => expires = Some(value.to_string()),
                "user_id" => user_id = Some(value.to_string()),
                _ => {}
            }
        }
    }

    Ok(NxmLink {
        game_domain,
        mod_id,
        file_id,
        key,
        expires,
        user_id,
    })
}

pub fn enqueue_nxm_url(url: &str) -> Result<(), NxmError> {
    let _ = parse_nxm_url(url)?;
    let path = pending_file_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(file, "{url}")?;
    file.sync_all()?;
    Ok(())
}

pub fn pending_nxm_urls() -> Result<Vec<String>, NxmError> {
    let path = pending_file_path()?;
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path)?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

pub fn take_pending_nxm_urls() -> Result<Vec<String>, NxmError> {
    let path = pending_file_path()?;
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let urls = pending_nxm_urls()?;
    let _ = fs::remove_file(path);
    Ok(urls)
}

pub fn register_nxm_handler() -> Result<PathBuf, NxmError> {
    let exe = std::env::current_exe().map_err(|error| {
        NxmError::Handler(format!("could not resolve current executable: {error}"))
    })?;
    #[cfg(target_os = "macos")]
    return register_macos_nxm_handler(&exe);

    #[cfg(not(target_os = "macos"))]
    {
    let exe_display = exe.display().to_string();
    let applications = applications_directory()?;
    fs::create_dir_all(&applications)?;
    let desktop_path = applications.join(NXM_DESKTOP_FILE);
    let contents = format!(
        "[Desktop Entry]\n\
Type=Application\n\
Name=pollip (Nexus Downloads)\n\
Exec=\"{exe_display}\" %u\n\
MimeType={NXM_MIME};\n\
NoDisplay=true\n\
Terminal=false\n\
StartupNotify=false\n\
Categories=Game;\n"
    );
    {
        let mut file = fs::File::create(&desktop_path)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
    }

    let mimeapps = mimeapps_path()?;
    if let Some(parent) = mimeapps.parent() {
        fs::create_dir_all(parent)?;
    }
    upsert_mime_default(&mimeapps, NXM_MIME, NXM_DESKTOP_FILE)?;

    let _ = Command::new("xdg-mime")
        .args(["default", NXM_DESKTOP_FILE, NXM_MIME])
        .status();
    let _ = Command::new("update-desktop-database")
        .arg(&applications)
        .status();

    Ok(desktop_path)
    }
}

pub fn handler_is_registered() -> bool {
    #[cfg(target_os = "macos")]
    return macos_nxm_bundle_path()
        .map(|path| path.join("Contents/Info.plist").is_file())
        .unwrap_or(false);

    #[cfg(not(target_os = "macos"))]
    {
    let Ok(applications) = applications_directory() else {
        return false;
    };
    if !applications.join(NXM_DESKTOP_FILE).is_file() {
        return false;
    }
    let Ok(mimeapps) = mimeapps_path() else {
        return false;
    };
    let Ok(text) = fs::read_to_string(mimeapps) else {
        return false;
    };
    text.lines().any(|line| {
        let trimmed = line.trim();
        trimmed.starts_with(NXM_MIME)
            && trimmed.contains('=')
            && trimmed.contains(NXM_DESKTOP_FILE)
    })
    }
}

#[cfg(target_os = "macos")]
fn register_macos_nxm_handler(exe: &Path) -> Result<PathBuf, NxmError> {
    use std::os::unix::fs::PermissionsExt;

    let bundle = macos_nxm_bundle_path()?;
    let contents = bundle.join("Contents");
    let macos = contents.join("MacOS");
    fs::create_dir_all(&macos)?;

    let info = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleDevelopmentRegion</key><string>en</string>
<key>CFBundleExecutable</key><string>pollip-nxm-handler</string>
<key>CFBundleIdentifier</key><string>io.pollip.nxm-handler</string>
<key>CFBundleName</key><string>pollip Nexus Downloads</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleURLTypes</key><array><dict>
<key>CFBundleURLName</key><string>Nexus Mod Manager Link</string>
<key>CFBundleURLSchemes</key><array><string>nxm</string></array>
</dict></array>
</dict></plist>
"#;
    let info_path = contents.join("Info.plist");
    fs::write(&info_path, info)?;

    let launcher = macos.join("pollip-nxm-handler");
    fs::write(
        &launcher,
        format!("#!/bin/sh\nexec {} \"$@\"\n", shell_quote(exe)),
    )?;
    let mut permissions = fs::metadata(&launcher)?.permissions();
    permissions.set_mode(permissions.mode() | 0o111);
    fs::set_permissions(&launcher, permissions)?;

    let registration = Command::new("/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister")
        .args(["-f", &bundle.display().to_string()])
        .status()
        .map_err(|error| NxmError::Handler(format!("could not register macOS URL handler: {error}")))?;
    if !registration.success() {
        return Err(NxmError::Handler(format!(
            "macOS rejected the URL handler bundle at {}",
            bundle.display()
        )));
    }
    Ok(bundle)
}

#[cfg(target_os = "macos")]
fn macos_nxm_bundle_path() -> Result<PathBuf, NxmError> {
    let home = std::env::var_os("HOME").ok_or(NxmError::HomeUnavailable)?;
    Ok(PathBuf::from(home)
        .join("Applications")
        .join("pollip NXM.app"))
}

#[cfg(target_os = "macos")]
fn shell_quote(path: &Path) -> String {
    let text = path.to_string_lossy();
    format!("'{}'", text.replace('\'', "'\\\"'\\\"'"))
}

#[cfg(not(target_os = "macos"))]
fn upsert_mime_default(path: &Path, mime: &str, desktop: &str) -> Result<(), NxmError> {
    let existing = if path.is_file() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };
    let mut lines: Vec<String> = existing.lines().map(str::to_string).collect();
    let prefix = format!("{mime}=");
    let replacement = format!("{mime}={desktop}");
    let mut replaced = false;
    for line in &mut lines {
        if line.trim_start().starts_with(&prefix) {
            *line = replacement.clone();
            replaced = true;
        }
    }
    if !replaced {
        if !lines.iter().any(|line| line.trim() == "[Default Applications]") {
            if !lines.is_empty() && !lines.last().is_some_and(|line| line.is_empty()) {
                lines.push(String::new());
            }
            lines.push("[Default Applications]".to_string());
        }
        lines.push(replacement);
    }
    let mut file = fs::File::create(path)?;
    for line in lines {
        writeln!(file, "{line}")?;
    }
    file.sync_all()?;
    Ok(())
}

fn data_directory() -> Result<PathBuf, NxmError> {
    crate::platform::data_directory().map_err(|_| NxmError::HomeUnavailable)
}

fn pending_file_path() -> Result<PathBuf, NxmError> {
    Ok(data_directory()?.join("nxm-pending.txt"))
}

#[cfg(not(target_os = "macos"))]
fn applications_directory() -> Result<PathBuf, NxmError> {
    let home = std::env::var_os("HOME").ok_or(NxmError::HomeUnavailable)?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("applications"))
}

#[cfg(not(target_os = "macos"))]
fn mimeapps_path() -> Result<PathBuf, NxmError> {
    let home = std::env::var_os("HOME").ok_or(NxmError::HomeUnavailable)?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("mimeapps.list"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_premium_style_nxm_without_query() {
        let link = parse_nxm_url("nxm://hollowknightsilksong/mods/12/files/34").expect("parse");
        assert_eq!(link.game_domain, "hollowknightsilksong");
        assert_eq!(link.mod_id, 12);
        assert_eq!(link.file_id, 34);
        assert!(link.key.is_none());
    }

    #[test]
    fn parses_free_user_nxm_with_key() {
        let link = parse_nxm_url(
            "nxm://hollowknightsilksong/mods/12/files/34?key=abc&expires=999&user_id=1",
        )
        .expect("parse");
        assert_eq!(link.key.as_deref(), Some("abc"));
        assert_eq!(link.expires.as_deref(), Some("999"));
        assert_eq!(link.user_id.as_deref(), Some("1"));
    }
}
