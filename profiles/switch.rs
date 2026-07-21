use super::model::{
    build_diff, find_installed_for_profile_mod, profile_mod_matches_installed,
    thunderstore_ref_from_profile_mod, DiffAction, ModOrigin, Profile, ProfileDiff, ProfileMod,
};
use super::store::{
    clear_active_profile, ensure_profiles_directory, load_profile, local_stash_present,
    profile_local_directory, restore_local_mod_files, set_active_profile, snapshot_current,
    StoreError,
};
use crate::mods::{
    disable_mod, disabled_folder, enable_mod, ensure_mod_folders, plugins_folder, remove_mod,
    scan_installed_mods, InstalledMod, ModError,
};
use crate::thunderstore::{install_from_download, load_catalog, InstallError, RemotePackage};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwitchPhase {
    Applying,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThunderstoreFallback {
    pub full_name: String,
    pub version: String,
    pub download_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SwitchStep {
    Remove {
        entry_name: String,
    },
    Disable {
        entry_name: String,
    },
    Enable {
        entry_name: String,
    },
    InstallThunderstore {
        entry_name: String,
        full_name: String,
        version: String,
        download_url: String,
        enabled: bool,
    },
    RestoreLocal {
        entry_name: String,
        enabled: bool,
        fallback: Option<ThunderstoreFallback>,
    },
    ReplaceThunderstore {
        entry_name: String,
        full_name: String,
        version: String,
        download_url: String,
        enabled: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchJournal {
    pub phase: SwitchPhase,
    pub install_folder: PathBuf,
    pub target_profile_name: String,
    pub rollback_profile: Profile,
    pub rollback_local_root: PathBuf,
    pub target_profile: Profile,
    pub target_local_root: PathBuf,
    pub steps: Vec<SwitchStep>,
    pub next_step_index: usize,
}

#[derive(Debug)]
pub enum SwitchError {
    Store(StoreError),
    Mods(ModError),
    Install(InstallError),
    Io(io::Error),
    Decode(String),
    MissingLocal { entry_name: String },
    MissingInstalled { entry_name: String },
}

impl std::fmt::Display for SwitchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Store(error) => write!(formatter, "{error}"),
            Self::Mods(error) => write!(formatter, "{error}"),
            Self::Install(error) => write!(formatter, "{error}"),
            Self::Io(error) => write!(formatter, "switch io error: {error}"),
            Self::Decode(detail) => write!(formatter, "switch journal error: {detail}"),
            Self::MissingLocal { entry_name } => write!(
                formatter,
                "could not restore '{entry_name}' — local stash missing and no thunderstore package matched"
            ),
            Self::MissingInstalled { entry_name } => {
                write!(formatter, "mod '{entry_name}' is not installed")
            }
        }
    }
}

impl std::error::Error for SwitchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            Self::Mods(error) => Some(error),
            Self::Install(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StoreError> for SwitchError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<ModError> for SwitchError {
    fn from(error: ModError) -> Self {
        Self::Mods(error)
    }
}

impl From<InstallError> for SwitchError {
    fn from(error: InstallError) -> Self {
        Self::Install(error)
    }
}

impl From<io::Error> for SwitchError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn journal_path() -> Result<PathBuf, SwitchError> {
    Ok(ensure_profiles_directory()?.join(".switch_journal.json"))
}

pub fn load_journal() -> Result<Option<SwitchJournal>, SwitchError> {
    let path = journal_path()?;
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)?;
    let mut journal: SwitchJournal =
        serde_json::from_str(&text).map_err(|error| SwitchError::Decode(error.to_string()))?;
    if journal.phase == SwitchPhase::Applying {
        journal.phase = SwitchPhase::Interrupted;
        save_journal(&journal)?;
    }
    Ok(Some(journal))
}

pub fn clear_journal() -> Result<(), SwitchError> {
    let path = journal_path()?;
    if path.is_file() {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub fn preview_switch(
    install_folder: &Path,
    target_name: &str,
    catalog: &[RemotePackage],
) -> Result<(Profile, ProfileDiff, Vec<String>), SwitchError> {
    let target = load_profile(target_name)?;
    let local_root = profile_local_directory(&slug_from_profile(&target)?)?;
    let catalog = ensure_catalog(catalog);
    let current = scan_installed_mods(install_folder)?;
    let diff = build_diff(&current, &target);
    let lines = describe_planned_actions(&diff, &target, &local_root, &catalog);
    Ok((target, diff, lines))
}

pub fn begin_switch(
    install_folder: &Path,
    target_name: &str,
    catalog: &[RemotePackage],
) -> Result<SwitchJournal, SwitchError> {
    let catalog = ensure_catalog(catalog);
    let target_profile = load_profile(target_name)?;
    let target_slug_local = profile_local_directory(&slug_from_profile(&target_profile)?)?;
    let (rollback_profile, rollback_local_root) =
        snapshot_current(".rollback", install_folder, &catalog)?;
    let current = scan_installed_mods(install_folder)?;
    let diff = build_diff(&current, &target_profile);
    let steps = steps_from_diff(&diff, &target_profile, &target_slug_local, &catalog);

    let journal = SwitchJournal {
        phase: SwitchPhase::Applying,
        install_folder: install_folder.to_path_buf(),
        target_profile_name: target_profile.name.clone(),
        rollback_profile,
        rollback_local_root,
        target_profile,
        target_local_root: target_slug_local,
        steps,
        next_step_index: 0,
    };
    save_journal(&journal)?;
    Ok(journal)
}

pub fn continue_switch(journal: &mut SwitchJournal) -> Result<(), SwitchError> {
    journal.phase = SwitchPhase::Applying;
    save_journal(journal)?;

    while journal.next_step_index < journal.steps.len() {
        let step = journal.steps[journal.next_step_index].clone();
        apply_step(journal, &step)?;
        journal.next_step_index += 1;
        save_journal(journal)?;
    }

    enforce_target_mod_set(journal)?;
    set_active_profile(&journal.target_profile_name)?;
    clear_journal()?;
    let _ = fs::remove_dir_all(&journal.rollback_local_root);
    Ok(())
}

pub fn rollback_switch(journal: &mut SwitchJournal) -> Result<(), SwitchError> {
    let catalog = ensure_catalog(&[]);
    let current = scan_installed_mods(&journal.install_folder)?;
    let diff = build_diff(&current, &journal.rollback_profile);
    let steps = steps_from_diff(
        &diff,
        &journal.rollback_profile,
        &journal.rollback_local_root,
        &catalog,
    );

    journal.phase = SwitchPhase::Applying;
    journal.target_profile_name = journal.rollback_profile.name.clone();
    journal.target_profile = journal.rollback_profile.clone();
    journal.target_local_root = journal.rollback_local_root.clone();
    journal.steps = steps;
    journal.next_step_index = 0;
    save_journal(journal)?;

    continue_switch(journal)?;
    clear_active_profile()?;
    Ok(())
}

fn ensure_catalog(catalog: &[RemotePackage]) -> Vec<RemotePackage> {
    if !catalog.is_empty() {
        return catalog.to_vec();
    }
    load_catalog(false)
        .map(|snapshot| snapshot.packages)
        .unwrap_or_default()
}

fn slug_from_profile(profile: &Profile) -> Result<String, SwitchError> {
    super::store::slugify_name(&profile.name).map_err(SwitchError::from)
}

fn describe_planned_actions(
    diff: &ProfileDiff,
    target: &Profile,
    local_root: &Path,
    catalog: &[RemotePackage],
) -> Vec<String> {
    let mut lines = Vec::new();
    for action in &diff.actions {
        match action {
            DiffAction::Install {
                entry_name,
                display_name,
                ..
            } => {
                if let Some(profile_mod) = find_mod(target, entry_name) {
                    lines.push(format!(
                        "install {display_name} ({})",
                        install_source_label(profile_mod, local_root, catalog)
                    ));
                }
            }
            DiffAction::ReplaceVersion {
                entry_name,
                display_name,
                from_version,
                to_version,
            } => {
                if let Some(profile_mod) = find_mod(target, entry_name) {
                    lines.push(format!(
                        "replace {display_name} {from_version} → {to_version} ({})",
                        install_source_label(profile_mod, local_root, catalog)
                    ));
                }
            }
            DiffAction::Remove { display_name, .. } => {
                lines.push(format!("remove {display_name}"));
            }
            DiffAction::Enable { display_name, .. } => {
                lines.push(format!("enable {display_name}"));
            }
            DiffAction::Disable { display_name, .. } => {
                lines.push(format!("disable {display_name}"));
            }
        }
    }
    lines
}

fn install_source_label(
    profile_mod: &ProfileMod,
    local_root: &Path,
    catalog: &[RemotePackage],
) -> String {
    match &profile_mod.origin {
        ModOrigin::Thunderstore { version, .. } => format!("thunderstore v{version}"),
        ModOrigin::Nexus {
            mod_id, version, ..
        } => {
            if local_stash_present(local_root, &profile_mod.entry_name) {
                format!("nexus #{mod_id} v{version} (local stash)")
            } else {
                format!("nexus #{mod_id} v{version}")
            }
        }
        ModOrigin::Local { .. } => {
            if local_stash_present(local_root, &profile_mod.entry_name) {
                "local stash".to_string()
            } else if let Some((full_name, version, _)) =
                thunderstore_ref_from_profile_mod(profile_mod, catalog)
            {
                format!("thunderstore download {full_name} v{version}")
            } else {
                "local stash (missing — no thunderstore match)".to_string()
            }
        }
    }
}

fn steps_from_diff(
    diff: &ProfileDiff,
    target: &Profile,
    local_root: &Path,
    catalog: &[RemotePackage],
) -> Vec<SwitchStep> {
    let mut steps = Vec::new();

    for action in &diff.actions {
        if let DiffAction::Remove { entry_name, .. } = action {
            steps.push(SwitchStep::Remove {
                entry_name: entry_name.clone(),
            });
        }
    }

    for action in &diff.actions {
        match action {
            DiffAction::ReplaceVersion { entry_name, .. }
            | DiffAction::Install { entry_name, .. } => {
                if let Some(profile_mod) = find_mod(target, entry_name) {
                    let replacing = matches!(action, DiffAction::ReplaceVersion { .. });
                    steps.push(plan_install_step(profile_mod, local_root, catalog, replacing));
                }
            }
            _ => {}
        }
    }

    for action in &diff.actions {
        match action {
            DiffAction::Enable { entry_name, .. } => steps.push(SwitchStep::Enable {
                entry_name: entry_name.clone(),
            }),
            DiffAction::Disable { entry_name, .. } => steps.push(SwitchStep::Disable {
                entry_name: entry_name.clone(),
            }),
            _ => {}
        }
    }

    steps
}

fn plan_install_step(
    profile_mod: &ProfileMod,
    local_root: &Path,
    catalog: &[RemotePackage],
    replacing: bool,
) -> SwitchStep {
    let fallback = thunderstore_ref_from_profile_mod(profile_mod, catalog).map(
        |(full_name, version, download_url)| ThunderstoreFallback {
            full_name,
            version,
            download_url,
        },
    );

    match &profile_mod.origin {
        ModOrigin::Thunderstore {
            full_name,
            version,
            download_url,
            ..
        } => {
            let (resolved_full_name, resolved_version, resolved_url) = fallback
                .map(|entry| (entry.full_name, entry.version, entry.download_url))
                .unwrap_or_else(|| {
                    (
                        full_name.clone(),
                        version.clone(),
                        download_url.clone(),
                    )
                });
            if replacing {
                SwitchStep::ReplaceThunderstore {
                    entry_name: profile_mod.entry_name.clone(),
                    full_name: resolved_full_name,
                    version: resolved_version,
                    download_url: resolved_url,
                    enabled: profile_mod.enabled,
                }
            } else {
                SwitchStep::InstallThunderstore {
                    entry_name: profile_mod.entry_name.clone(),
                    full_name: resolved_full_name,
                    version: resolved_version,
                    download_url: resolved_url,
                    enabled: profile_mod.enabled,
                }
            }
        }
        ModOrigin::Local { .. } | ModOrigin::Nexus { .. } => {
            let stash_ok = local_stash_present(local_root, &profile_mod.entry_name);
            if !stash_ok {
                if let Some(remote) = fallback {
                    if replacing {
                        return SwitchStep::ReplaceThunderstore {
                            entry_name: profile_mod.entry_name.clone(),
                            full_name: remote.full_name,
                            version: remote.version,
                            download_url: remote.download_url,
                            enabled: profile_mod.enabled,
                        };
                    }
                    return SwitchStep::InstallThunderstore {
                        entry_name: profile_mod.entry_name.clone(),
                        full_name: remote.full_name,
                        version: remote.version,
                        download_url: remote.download_url,
                        enabled: profile_mod.enabled,
                    };
                }
            }
            SwitchStep::RestoreLocal {
                entry_name: profile_mod.entry_name.clone(),
                enabled: profile_mod.enabled,
                fallback,
            }
        }
    }
}

fn find_mod<'a>(profile: &'a Profile, entry_name: &str) -> Option<&'a ProfileMod> {
    profile
        .mods
        .iter()
        .find(|entry| entry.entry_name == entry_name)
}

fn apply_step(journal: &SwitchJournal, step: &SwitchStep) -> Result<(), SwitchError> {
    ensure_mod_folders(&journal.install_folder)?;
    match step {
        SwitchStep::Remove { entry_name } => {
            remove_installed_entry(&journal.install_folder, entry_name)?;
        }
        SwitchStep::Disable { entry_name } => {
            let installed = find_installed(&journal.install_folder, entry_name)?.ok_or_else(|| {
                SwitchError::MissingInstalled {
                    entry_name: entry_name.clone(),
                }
            })?;
            disable_mod(&journal.install_folder, &installed)?;
        }
        SwitchStep::Enable { entry_name } => {
            let installed = find_installed(&journal.install_folder, entry_name)?.ok_or_else(|| {
                SwitchError::MissingInstalled {
                    entry_name: entry_name.clone(),
                }
            })?;
            enable_mod(&journal.install_folder, &installed)?;
        }
        SwitchStep::InstallThunderstore {
            entry_name,
            full_name,
            version,
            download_url,
            enabled,
        } => {
            install_package_as_entry(
                &journal.install_folder,
                entry_name,
                full_name,
                version,
                download_url,
                *enabled,
            )?;
        }
        SwitchStep::ReplaceThunderstore {
            entry_name,
            full_name,
            version,
            download_url,
            enabled,
        } => {
            remove_installed_entry(&journal.install_folder, entry_name)?;
            if entry_name != full_name {
                remove_installed_entry(&journal.install_folder, full_name)?;
            }
            install_package_as_entry(
                &journal.install_folder,
                entry_name,
                full_name,
                version,
                download_url,
                *enabled,
            )?;
        }
        SwitchStep::RestoreLocal {
            entry_name,
            enabled,
            fallback,
        } => {
            restore_local_or_download(journal, entry_name, *enabled, fallback.as_ref())?;
        }
    }
    Ok(())
}

fn enforce_target_mod_set(journal: &SwitchJournal) -> Result<(), SwitchError> {
    let catalog = ensure_catalog(&[]);
    let current = scan_installed_mods(&journal.install_folder)?;

    for installed in &current {
        let wanted = journal
            .target_profile
            .mods
            .iter()
            .any(|entry| profile_mod_matches_installed(entry, installed));
        if !wanted {
            remove_mod(&journal.install_folder, installed)?;
        }
    }

    let remaining = scan_installed_mods(&journal.install_folder)?;
    for profile_mod in &journal.target_profile.mods {
        if find_installed_for_profile_mod(profile_mod, &remaining).is_some() {
            continue;
        }
        let step = plan_install_step(
            profile_mod,
            &journal.target_local_root,
            &catalog,
            false,
        );
        apply_step(journal, &step)?;
    }

    let final_mods = scan_installed_mods(&journal.install_folder)?;
    for profile_mod in &journal.target_profile.mods {
        if let Some(installed) = find_installed_for_profile_mod(profile_mod, &final_mods) {
            if installed.enabled != profile_mod.enabled {
                if profile_mod.enabled {
                    enable_mod(&journal.install_folder, installed)?;
                } else {
                    disable_mod(&journal.install_folder, installed)?;
                }
            }
        }
    }
    Ok(())
}

fn remove_installed_entry(install_folder: &Path, entry_name: &str) -> Result<(), SwitchError> {
    if let Some(installed) = find_installed(install_folder, entry_name)? {
        remove_mod(install_folder, &installed)?;
    }
    Ok(())
}

fn restore_local_or_download(
    journal: &SwitchJournal,
    entry_name: &str,
    enabled: bool,
    fallback: Option<&ThunderstoreFallback>,
) -> Result<(), SwitchError> {
    if let Some(installed) = find_installed(&journal.install_folder, entry_name)? {
        remove_mod(&journal.install_folder, &installed)?;
    }

    let profile_mod = find_mod(&journal.target_profile, entry_name).ok_or_else(|| {
        SwitchError::MissingLocal {
            entry_name: entry_name.to_string(),
        }
    })?;
    let parent = if enabled {
        plugins_folder(&journal.install_folder)
    } else {
        disabled_folder(&journal.install_folder)
    };

    match restore_local_mod_files(&journal.target_local_root, profile_mod, &parent) {
        Ok(_) => Ok(()),
        Err(StoreError::Missing(_)) => {
            let remote = match fallback {
                Some(remote) => remote.clone(),
                None => live_thunderstore_fallback(profile_mod)?,
            };
            install_package_as_entry(
                &journal.install_folder,
                entry_name,
                &remote.full_name,
                &remote.version,
                &remote.download_url,
                enabled,
            )
        }
        Err(other) => Err(SwitchError::Store(other)),
    }
}

fn live_thunderstore_fallback(profile_mod: &ProfileMod) -> Result<ThunderstoreFallback, SwitchError> {
    let catalog = ensure_catalog(&[]);
    thunderstore_ref_from_profile_mod(profile_mod, &catalog)
        .map(|(full_name, version, download_url)| ThunderstoreFallback {
            full_name,
            version,
            download_url,
        })
        .ok_or_else(|| SwitchError::MissingLocal {
            entry_name: profile_mod.entry_name.clone(),
        })
}

fn install_package_as_entry(
    install_folder: &Path,
    entry_name: &str,
    full_name: &str,
    version: &str,
    download_url: &str,
    enabled: bool,
) -> Result<(), SwitchError> {
    let installed_path =
        install_from_download(install_folder, full_name, version, download_url)?;

    if entry_name != full_name {
        let destination = plugins_folder(install_folder).join(entry_name);
        if destination.exists() {
            if destination.is_dir() {
                fs::remove_dir_all(&destination)?;
            } else {
                fs::remove_file(&destination)?;
            }
        }
        fs::rename(&installed_path, &destination)?;
    }

    if !enabled {
        let installed = find_installed(install_folder, entry_name)?.ok_or_else(|| {
            SwitchError::MissingInstalled {
                entry_name: entry_name.to_string(),
            }
        })?;
        disable_mod(install_folder, &installed)?;
    }
    Ok(())
}

fn find_installed(
    install_folder: &Path,
    entry_name: &str,
) -> Result<Option<InstalledMod>, SwitchError> {
    let mods = scan_installed_mods(install_folder)?;
    Ok(mods
        .into_iter()
        .find(|entry| entry.entry_name == entry_name))
}

fn save_journal(journal: &SwitchJournal) -> Result<(), SwitchError> {
    let path = journal_path()?;
    let text = serde_json::to_string_pretty(journal)
        .map_err(|error| SwitchError::Decode(error.to_string()))?;
    let temporary = path.with_extension("json.partial");
    {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::model::{LocalModKind, ModOrigin};
    use super::super::store::{create_from_current, PROFILES_ENV, PROFILES_ENV_LOCK};
    use crate::mods::{ensure_mod_folders, plugins_folder};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("silksong-switch-{label}-{nanos}"));
        fs::create_dir_all(&directory).expect("scratch");
        directory
    }

    #[test]
    fn switch_between_local_only_profiles_is_resumable() {
        let _guard = PROFILES_ENV_LOCK.lock().expect("lock");
        let root = scratch("resume");
        let profiles_dir = root.join("profiles");
        let install = root.join("game");
        ensure_mod_folders(&install).expect("folders");
        fs::write(plugins_folder(&install).join("Alpha.dll"), b"a").expect("a");
        unsafe {
            std::env::set_var(PROFILES_ENV, &profiles_dir);
        }

        create_from_current("alpha", &install, &[]).expect("alpha");
        fs::remove_file(plugins_folder(&install).join("Alpha.dll")).expect("rm");
        fs::write(plugins_folder(&install).join("Beta.dll"), b"b").expect("b");
        create_from_current("beta", &install, &[]).expect("beta");

        let mut journal = begin_switch(&install, "alpha", &[]).expect("begin");
        assert!(!journal.steps.is_empty());
        journal.phase = SwitchPhase::Interrupted;
        save_journal(&journal).expect("save");

        let mut loaded = load_journal().expect("load").expect("present");
        assert_eq!(loaded.phase, SwitchPhase::Interrupted);
        continue_switch(&mut loaded).expect("finish");

        let listed = scan_installed_mods(&install).expect("scan");
        assert!(listed.iter().any(|entry| entry.entry_name == "Alpha.dll"));
        assert!(!listed.iter().any(|entry| entry.entry_name == "Beta.dll"));
        assert!(load_journal().expect("gone").is_none());

        unsafe {
            std::env::remove_var(PROFILES_ENV);
        }
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn switching_to_smaller_profile_removes_extra_mods() {
        let _guard = PROFILES_ENV_LOCK.lock().expect("lock");
        let root = scratch("shrink");
        let profiles_dir = root.join("profiles");
        let install = root.join("game");
        ensure_mod_folders(&install).expect("folders");
        fs::write(plugins_folder(&install).join("Keep.dll"), b"keep").expect("keep");
        fs::write(plugins_folder(&install).join("DropOne.dll"), b"drop1").expect("drop1");
        fs::write(plugins_folder(&install).join("DropTwo.dll"), b"drop2").expect("drop2");
        unsafe {
            std::env::set_var(PROFILES_ENV, &profiles_dir);
        }

        create_from_current("full", &install, &[]).expect("full");
        fs::remove_file(plugins_folder(&install).join("DropOne.dll")).expect("rm1");
        fs::remove_file(plugins_folder(&install).join("DropTwo.dll")).expect("rm2");
        create_from_current("minimal", &install, &[]).expect("minimal");

        fs::write(plugins_folder(&install).join("DropOne.dll"), b"drop1").expect("restore1");
        fs::write(plugins_folder(&install).join("DropTwo.dll"), b"drop2").expect("restore2");
        assert_eq!(scan_installed_mods(&install).expect("scan").len(), 3);

        let mut journal = begin_switch(&install, "minimal", &[]).expect("begin");
        let remove_count = journal
            .steps
            .iter()
            .filter(|step| matches!(step, SwitchStep::Remove { .. }))
            .count();
        assert_eq!(remove_count, 2);
        continue_switch(&mut journal).expect("switch");

        let listed = scan_installed_mods(&install).expect("after");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].entry_name, "Keep.dll");

        unsafe {
            std::env::remove_var(PROFILES_ENV);
        }
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn missing_local_stash_plans_thunderstore_download() {
        let remote = RemotePackage {
            name: "FsmUtil".into(),
            full_name: "silksong_modding-FsmUtil".into(),
            owner: "silksong_modding".into(),
            description: String::new(),
            version: "0.3.17".into(),
            version_full_name: "silksong_modding-FsmUtil-0.3.17".into(),
            downloads: 1,
            download_url: "https://example.invalid/fsmutil.zip".into(),
            icon_url: String::new(),
            dependencies: Vec::new(),
            date_updated: String::new(),
            is_deprecated: false,
        };
        let profile_mod = ProfileMod {
            entry_name: "silksong_modding-FsmUtil".into(),
            display_name: "FsmUtil".into(),
            enabled: true,
            origin: ModOrigin::Local {
                mod_kind: LocalModKind::Folder,
            },
        };
        let local_root = scratch("empty-local");
        let step = plan_install_step(&profile_mod, &local_root, &[remote], false);
        assert!(matches!(
            step,
            SwitchStep::InstallThunderstore {
                ref full_name,
                ref version,
                ..
            } if full_name == "silksong_modding-FsmUtil" && version == "0.3.17"
        ));
        fs::remove_dir_all(local_root).ok();
    }
}
