use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn data_directory() -> io::Result<PathBuf> {
    let home = home_directory()?;
    #[cfg(target_os = "macos")]
    {
        Ok(home.join("Library").join("Application Support").join("pollip"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(home.join(".local").join("share").join("pollip"))
    }
}

pub fn cache_directory() -> io::Result<PathBuf> {
    let home = home_directory()?;
    #[cfg(target_os = "macos")]
    {
        Ok(home.join("Library").join("Caches").join("pollip"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(home.join(".cache").join("pollip"))
    }
}

pub fn steam_root_candidates(home: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(target_os = "macos")]
    roots.push(home.join("Library").join("Application Support").join("Steam"));
    #[cfg(not(target_os = "macos"))]
    roots.extend([
        home.join(".steam").join("steam"),
        home.join(".local").join("share").join("Steam"),
        home.join(".var")
            .join("app")
            .join("com.valvesoftware.Steam")
            .join(".local")
            .join("share")
            .join("Steam"),
        home.join(".var")
            .join("app")
            .join("com.valvesoftware.Steam")
            .join(".steam")
            .join("steam"),
        home.join("snap")
            .join("steam")
            .join("common")
            .join(".local")
            .join("share")
            .join("Steam"),
        home.join("snap")
            .join("steam")
            .join("common")
            .join(".steam")
            .join("steam"),
    ]);
    roots
}

pub fn open_path(path: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(not(target_os = "macos"))]
    let mut command = Command::new("xdg-open");
    command.arg(path).spawn().map(|_| ())
}

pub fn open_url(url: &str) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(not(target_os = "macos"))]
    let mut command = Command::new("xdg-open");
    command.arg(url).spawn().map(|_| ())
}

pub fn launch_steam_uri(uri: &str) -> io::Result<String> {
    #[cfg(target_os = "macos")]
    {
        open_url(uri)?;
        Ok(format!("open {uri}"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        if Command::new("steam").arg(uri).spawn().is_ok() {
            return Ok(format!("steam {uri}"));
        }
        open_url(uri)?;
        Ok(format!("xdg-open {uri}"))
    }
}

pub fn process_named_running(names: &[&str]) -> bool {
    names.iter().any(|name| {
        Command::new("pgrep")
            .args(["-x", name])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    })
}

pub fn process_is_running(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn home_directory() -> io::Result<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "HOME is unset")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_roots_include_the_current_platform_default() {
        let home = PathBuf::from("/home/player");
        let roots = steam_root_candidates(&home);
        #[cfg(target_os = "macos")]
        assert!(roots.contains(&home.join("Library/Application Support/Steam")));
        #[cfg(not(target_os = "macos"))]
        assert!(roots.contains(&home.join(".local/share/Steam")));
    }
}
