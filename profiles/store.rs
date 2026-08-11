use super::model::{profile_mod_from_installed, ModOrigin, Profile, ProfileMod, ProfileSummary};
use crate::mods::{scan_installed_mods, InstalledMod};
use crate::thunderstore::RemotePackage;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const PROFILES_ENV: &str = "SILKSONG_PROFILES_DIR";

#[derive(Debug)]
pub enum StoreError {
    DirectoryUnavailable,
    InvalidName(String),
    AlreadyExists(String),
    Missing(String),
    Decode(String),
    Io(io::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DirectoryUnavailable => {
                write!(formatter, "could not resolve the profiles directory")
            }
            Self::InvalidName(name) => write!(formatter, "invalid profile name '{name}'"),
            Self::AlreadyExists(name) => write!(formatter, "profile '{name}' already exists"),
            Self::Missing(name) => write!(formatter, "profile '{name}' was not found"),
            Self::Decode(detail) => write!(formatter, "profile json error: {detail}"),
            Self::Io(error) => write!(formatter, "profile io error: {error}"),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for StoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActiveProfileFile {
    name: String,
}

pub fn profiles_directory() -> Result<PathBuf, StoreError> {
    if let Some(explicit) = std::env::var_os(PROFILES_ENV) {
        return Ok(PathBuf::from(explicit));
    }
    let home = std::env::var_os("HOME").ok_or(StoreError::DirectoryUnavailable)?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("pollip")
        .join("profiles"))
}

pub fn ensure_profiles_directory() -> Result<PathBuf, StoreError> {
    let directory = profiles_directory()?;
    fs::create_dir_all(&directory)?;
    Ok(directory)
}

pub fn slugify_name(name: &str) -> Result<String, StoreError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(StoreError::InvalidName(name.to_string()));
    }
    let mut slug = String::new();
    let mut previous_underscore = false;
    for character in trimmed.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            previous_underscore = false;
        } else if !previous_underscore {
            slug.push('_');
            previous_underscore = true;
        }
    }
    let slug = slug.trim_matches('_').to_string();
    if slug.is_empty() {
        return Err(StoreError::InvalidName(name.to_string()));
    }
    Ok(slug)
}

pub fn profile_json_path(slug: &str) -> Result<PathBuf, StoreError> {
    Ok(ensure_profiles_directory()?.join(format!("{slug}.json")))
}

pub fn profile_local_directory(slug: &str) -> Result<PathBuf, StoreError> {
    Ok(ensure_profiles_directory()?
        .join(slug)
        .join("local"))
}

pub fn list_profiles() -> Result<Vec<ProfileSummary>, StoreError> {
    let directory = ensure_profiles_directory()?;
    let mut summaries = Vec::new();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        if !extension.eq_ignore_ascii_case("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        if stem.starts_with('.') {
            continue;
        }
        match load_profile_from_path(&path) {
            Ok(profile) => summaries.push(ProfileSummary {
                name: profile.name,
                slug: stem.to_string(),
                mod_count: profile.mods.len(),
                path,
            }),
            Err(_) => continue,
        }
    }
    summaries.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.slug.cmp(&right.slug))
    });
    Ok(summaries)
}

pub fn load_profile(name: &str) -> Result<Profile, StoreError> {
    let slug = slugify_name(name)?;
    let path = profile_json_path(&slug)?;
    if !path.is_file() {
        return Err(StoreError::Missing(name.to_string()));
    }
    load_profile_from_path(&path)
}

pub fn load_profile_from_path(path: &Path) -> Result<Profile, StoreError> {
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(|error| StoreError::Decode(error.to_string()))
}

pub fn create_from_current(
    name: &str,
    install_folder: &Path,
    catalog: &[RemotePackage],
) -> Result<Profile, StoreError> {
    let slug = slugify_name(name)?;
    let path = profile_json_path(&slug)?;
    if path.exists() {
        return Err(StoreError::AlreadyExists(name.to_string()));
    }

    let installed = scan_installed_mods(install_folder)?;
    let now = unix_now();
    let mut mods = Vec::new();
    let local_root = profile_local_directory(&slug)?;
    fs::create_dir_all(&local_root)?;

    for entry in &installed {
        let profile_mod = profile_mod_from_installed(entry, catalog);
        if matches!(profile_mod.origin, ModOrigin::Local { .. }) {
            stash_local_mod(&local_root, entry)?;
        }
        mods.push(profile_mod);
    }

    let profile = Profile {
        name: name.trim().to_string(),
        created_at_unix: now,
        updated_at_unix: now,
        mods,
    };
    write_profile_atomic(&path, &profile)?;
    set_active_profile(&profile.name)?;
    Ok(profile)
}

pub fn delete_profile(name: &str) -> Result<(), StoreError> {
    let slug = slugify_name(name)?;
    let path = profile_json_path(&slug)?;
    if !path.is_file() {
        return Err(StoreError::Missing(name.to_string()));
    }
    fs::remove_file(path)?;
    let local_root = ensure_profiles_directory()?.join(&slug);
    if local_root.is_dir() {
        fs::remove_dir_all(local_root)?;
    }
    if active_profile_name()?.as_deref() == Some(name.trim()) {
        clear_active_profile()?;
    }
    Ok(())
}

pub fn duplicate_profile(source_name: &str, new_name: &str) -> Result<Profile, StoreError> {
    let source = load_profile(source_name)?;
    let new_slug = slugify_name(new_name)?;
    let destination = profile_json_path(&new_slug)?;
    if destination.exists() {
        return Err(StoreError::AlreadyExists(new_name.to_string()));
    }

    let now = unix_now();
    let duplicated = Profile {
        name: new_name.trim().to_string(),
        created_at_unix: now,
        updated_at_unix: now,
        mods: source.mods.clone(),
    };
    write_profile_atomic(&destination, &duplicated)?;

    let source_slug = slugify_name(source_name)?;
    let source_local = profile_local_directory(&source_slug)?;
    if source_local.is_dir() {
        let destination_local = profile_local_directory(&new_slug)?;
        copy_tree(&source_local, &destination_local)?;
    }
    Ok(duplicated)
}

pub fn export_profile(name: &str, destination: &Path) -> Result<(), StoreError> {
    let profile = load_profile(name)?;
    write_profile_atomic(destination, &profile)
}

pub fn import_profile(source: &Path) -> Result<Profile, StoreError> {
    let mut profile = load_profile_from_path(source)?;
    let slug = slugify_name(&profile.name)?;
    let destination = profile_json_path(&slug)?;
    if destination.exists() {
        return Err(StoreError::AlreadyExists(profile.name.clone()));
    }
    let now = unix_now();
    profile.created_at_unix = now;
    profile.updated_at_unix = now;
    write_profile_atomic(&destination, &profile)?;
    Ok(profile)
}

pub fn active_profile_name() -> Result<Option<String>, StoreError> {
    let path = active_profile_path()?;
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)?;
    let file: ActiveProfileFile =
        serde_json::from_str(&text).map_err(|error| StoreError::Decode(error.to_string()))?;
    Ok(Some(file.name))
}

pub fn set_active_profile(name: &str) -> Result<(), StoreError> {
    let path = active_profile_path()?;
    let file = ActiveProfileFile {
        name: name.trim().to_string(),
    };
    let text =
        serde_json::to_string_pretty(&file).map_err(|error| StoreError::Decode(error.to_string()))?;
    atomic_write(&path, text.as_bytes())
}

pub fn clear_active_profile() -> Result<(), StoreError> {
    let path = active_profile_path()?;
    if path.is_file() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn snapshot_current(
    name: &str,
    install_folder: &Path,
    catalog: &[RemotePackage],
) -> Result<(Profile, PathBuf), StoreError> {
    let slug = slugify_name(name)?;
    let installed = scan_installed_mods(install_folder)?;
    let now = unix_now();
    let local_root = ensure_profiles_directory()?
        .join(".snapshots")
        .join(&slug)
        .join("local");
    if local_root.exists() {
        fs::remove_dir_all(&local_root)?;
    }
    fs::create_dir_all(&local_root)?;

    let mut mods = Vec::new();
    for entry in &installed {
        let profile_mod = profile_mod_from_installed(entry, catalog);
        if matches!(profile_mod.origin, ModOrigin::Local { .. }) {
            stash_local_mod(&local_root, entry)?;
        }
        mods.push(profile_mod);
    }

    Ok((
        Profile {
            name: name.trim().to_string(),
            created_at_unix: now,
            updated_at_unix: now,
            mods,
        },
        local_root,
    ))
}

pub fn local_mod_stash_path(local_root: &Path, entry_name: &str) -> PathBuf {
    local_root.join(entry_name)
}

pub fn local_stash_present(local_root: &Path, entry_name: &str) -> bool {
    local_mod_stash_path(local_root, entry_name).exists()
}

pub fn restore_local_mod_files(
    local_root: &Path,
    profile_mod: &ProfileMod,
    destination_parent: &Path,
) -> Result<PathBuf, StoreError> {
    let source = local_mod_stash_path(local_root, &profile_mod.entry_name);
    if !source.exists() {
        return Err(StoreError::Missing(format!(
            "local files for '{}'",
            profile_mod.entry_name
        )));
    }
    let destination = destination_parent.join(&profile_mod.entry_name);
    if destination.exists() {
        if destination.is_dir() {
            fs::remove_dir_all(&destination)?;
        } else {
            fs::remove_file(&destination)?;
        }
    }
    copy_tree(&source, &destination)?;
    Ok(destination)
}

fn stash_local_mod(local_root: &Path, installed: &InstalledMod) -> Result<(), StoreError> {
    let destination = local_mod_stash_path(local_root, &installed.entry_name);
    if destination.exists() {
        if destination.is_dir() {
            fs::remove_dir_all(&destination)?;
        } else {
            fs::remove_file(&destination)?;
        }
    }
    copy_tree(&installed.path, &destination)?;
    Ok(())
}

fn active_profile_path() -> Result<PathBuf, StoreError> {
    Ok(ensure_profiles_directory()?.join(".active.json"))
}

fn write_profile_atomic(path: &Path, profile: &Profile) -> Result<(), StoreError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(profile)
        .map_err(|error| StoreError::Decode(error.to_string()))?;
    atomic_write(path, text.as_bytes())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let temporary = path.with_extension("json.partial");
    {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub fn copy_tree(source: &Path, destination: &Path) -> Result<(), StoreError> {
    if source.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
        return Ok(());
    }
    if !source.is_dir() {
        return Err(StoreError::Io(io::Error::other(format!(
            "expected file or directory at {}",
            source.display()
        ))));
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if file_type.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) static PROFILES_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::{ensure_mod_folders, plugins_folder};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("silksong-profiles-{label}-{nanos}"));
        fs::create_dir_all(&directory).expect("scratch");
        directory
    }

    #[test]
    fn create_list_duplicate_and_delete_profile() {
        let _guard = PROFILES_ENV_LOCK.lock().expect("lock");
        let root = scratch("store");
        let profiles_dir = root.join("profiles");
        let install = root.join("game");
        ensure_mod_folders(&install).expect("folders");
        fs::write(plugins_folder(&install).join("Loose.dll"), b"dll").expect("dll");
        fs::create_dir_all(plugins_folder(&install).join("demo_mod")).expect("dir");
        fs::write(
            plugins_folder(&install).join("demo_mod").join("manifest.json"),
            br#"{ "name": "Demo", "version_number": "1.0.0" }"#,
        )
        .expect("manifest");

        unsafe {
            std::env::set_var(PROFILES_ENV, &profiles_dir);
        }

        let created = create_from_current("My Run", &install, &[]).expect("create");
        assert_eq!(created.name, "My Run");
        assert!(!created.mods.is_empty());

        let listed = list_profiles().expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].slug, "my_run");

        let duplicated = duplicate_profile("My Run", "My Run Copy").expect("dup");
        assert_eq!(duplicated.name, "My Run Copy");
        assert_eq!(list_profiles().expect("list2").len(), 2);

        delete_profile("My Run").expect("delete");
        assert_eq!(list_profiles().expect("list3").len(), 1);

        unsafe {
            std::env::remove_var(PROFILES_ENV);
        }
        fs::remove_dir_all(root).ok();
    }
}
