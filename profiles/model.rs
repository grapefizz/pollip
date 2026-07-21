use crate::mods::{InstalledMod, ModKind, ModSource};
use crate::nexus::read_meta;
use crate::thunderstore::{match_installed_package, RemotePackage};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalModKind {
    Folder,
    Assembly,
}

impl From<ModKind> for LocalModKind {
    fn from(kind: ModKind) -> Self {
        match kind {
            ModKind::Folder => Self::Folder,
            ModKind::Assembly => Self::Assembly,
        }
    }
}

impl From<LocalModKind> for ModKind {
    fn from(kind: LocalModKind) -> Self {
        match kind {
            LocalModKind::Folder => Self::Folder,
            LocalModKind::Assembly => Self::Assembly,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModOrigin {
    Thunderstore {
        owner: String,
        package_name: String,
        full_name: String,
        version: String,
        download_url: String,
    },
    Nexus {
        mod_id: u64,
        file_id: u64,
        game_domain: String,
        version: String,
        mod_name: String,
    },
    Local {
        mod_kind: LocalModKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileMod {
    pub entry_name: String,
    pub display_name: String,
    pub enabled: bool,
    pub origin: ModOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub mods: Vec<ProfileMod>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileSummary {
    pub name: String,
    pub slug: String,
    pub mod_count: usize,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffAction {
    Install {
        entry_name: String,
        display_name: String,
        detail: String,
    },
    Remove {
        entry_name: String,
        display_name: String,
    },
    Enable {
        entry_name: String,
        display_name: String,
    },
    Disable {
        entry_name: String,
        display_name: String,
    },
    ReplaceVersion {
        entry_name: String,
        display_name: String,
        from_version: String,
        to_version: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProfileDiff {
    pub actions: Vec<DiffAction>,
}

pub fn profile_mod_from_installed(
    installed: &InstalledMod,
    catalog: &[RemotePackage],
) -> ProfileMod {
    if installed.source == ModSource::Nexus {
        if let Some(meta) = read_meta(&installed.path) {
            return ProfileMod {
                entry_name: installed.entry_name.clone(),
                display_name: if meta.mod_name.is_empty() {
                    installed.display_name.clone()
                } else {
                    meta.mod_name.clone()
                },
                enabled: installed.enabled,
                origin: ModOrigin::Nexus {
                    mod_id: meta.mod_id,
                    file_id: meta.file_id,
                    game_domain: meta.game_domain,
                    version: meta.version,
                    mod_name: meta.mod_name,
                },
            };
        }
        return ProfileMod {
            entry_name: installed.entry_name.clone(),
            display_name: installed.display_name.clone(),
            enabled: installed.enabled,
            origin: ModOrigin::Local {
                mod_kind: installed.kind.into(),
            },
        };
    }

    if let Some(remote) = match_installed_package(installed, catalog) {
        let version = installed
            .version
            .clone()
            .unwrap_or_else(|| remote.version.clone());
        return ProfileMod {
            entry_name: installed.entry_name.clone(),
            display_name: remote.name.clone(),
            enabled: installed.enabled,
            origin: ModOrigin::Thunderstore {
                owner: remote.owner.clone(),
                package_name: remote.name.clone(),
                full_name: remote.full_name.clone(),
                version: version.clone(),
                download_url: thunderstore_download_url(&remote.owner, &remote.name, &version),
            },
        };
    }

    if let Some((owner, package_name)) = split_owner_and_name(&installed.entry_name) {
        if let Some(version) = &installed.version {
            return ProfileMod {
                entry_name: installed.entry_name.clone(),
                display_name: installed.display_name.clone(),
                enabled: installed.enabled,
                origin: ModOrigin::Thunderstore {
                    owner: owner.clone(),
                    package_name: package_name.clone(),
                    full_name: installed.entry_name.clone(),
                    version: version.clone(),
                    download_url: thunderstore_download_url(&owner, &package_name, version),
                },
            };
        }
    }

    ProfileMod {
        entry_name: installed.entry_name.clone(),
        display_name: installed.display_name.clone(),
        enabled: installed.enabled,
        origin: ModOrigin::Local {
            mod_kind: installed.kind.into(),
        },
    }
}

pub fn profile_mod_matches_installed(profile_mod: &ProfileMod, installed: &InstalledMod) -> bool {
    if profile_mod.entry_name == installed.entry_name {
        return true;
    }
    if let ModOrigin::Thunderstore { full_name, .. } = &profile_mod.origin {
        if full_name == &installed.entry_name {
            return true;
        }
    }
    if let ModOrigin::Nexus { mod_id, .. } = &profile_mod.origin {
        if installed.entry_name == format!("nexus-{mod_id}") {
            return true;
        }
    }
    false
}

pub fn find_installed_for_profile_mod<'a>(
    profile_mod: &ProfileMod,
    installed_mods: &'a [InstalledMod],
) -> Option<&'a InstalledMod> {
    installed_mods
        .iter()
        .find(|installed| profile_mod_matches_installed(profile_mod, installed))
}

pub fn thunderstore_download_url(owner: &str, package_name: &str, version: &str) -> String {
    format!("https://thunderstore.io/package/download/{owner}/{package_name}/{version}/")
}

pub fn split_owner_and_name(full_name: &str) -> Option<(String, String)> {
    let (owner, package_name) = full_name.split_once('-')?;
    if owner.is_empty() || package_name.is_empty() {
        return None;
    }
    Some((owner.to_string(), package_name.to_string()))
}

pub fn looks_like_assembly_entry(entry_name: &str) -> bool {
    Path::new(entry_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
}

pub fn resolve_thunderstore_package<'a>(
    profile_mod: &ProfileMod,
    catalog: &'a [RemotePackage],
) -> Option<&'a RemotePackage> {
    if looks_like_assembly_entry(&profile_mod.entry_name) {
        return None;
    }

    if let ModOrigin::Thunderstore {
        full_name,
        version,
        ..
    } = &profile_mod.origin
    {
        if let Some(exact) = catalog.iter().find(|package| {
            package.full_name == *full_name && package.version == *version
        }) {
            return Some(exact);
        }
        if let Some(same_name) = catalog
            .iter()
            .find(|package| package.full_name == *full_name)
        {
            return Some(same_name);
        }
    }

    catalog
        .iter()
        .find(|package| package.full_name == profile_mod.entry_name)
        .or_else(|| {
            catalog.iter().find(|package| {
                package
                    .full_name
                    .eq_ignore_ascii_case(&profile_mod.entry_name)
            })
        })
        .or_else(|| {
            if split_owner_and_name(&profile_mod.entry_name).is_none() {
                return None;
            }
            catalog.iter().find(|package| {
                package.name.eq_ignore_ascii_case(&profile_mod.display_name)
                    || package.name.eq_ignore_ascii_case(&profile_mod.entry_name)
            })
        })
}

pub fn thunderstore_ref_from_profile_mod(
    profile_mod: &ProfileMod,
    catalog: &[RemotePackage],
) -> Option<(String, String, String)> {
    match &profile_mod.origin {
        ModOrigin::Thunderstore {
            full_name,
            version,
            download_url,
            ..
        } => {
            if let Some(remote) = resolve_thunderstore_package(profile_mod, catalog) {
                let url = if remote.version == *version {
                    download_url.clone()
                } else {
                    thunderstore_download_url(&remote.owner, &remote.name, &remote.version)
                };
                Some((remote.full_name.clone(), remote.version.clone(), url))
            } else {
                Some((full_name.clone(), version.clone(), download_url.clone()))
            }
        }
        ModOrigin::Local { .. } | ModOrigin::Nexus { .. } => {
            let remote = resolve_thunderstore_package(profile_mod, catalog)?;
            Some((
                remote.full_name.clone(),
                remote.version.clone(),
                thunderstore_download_url(&remote.owner, &remote.name, &remote.version),
            ))
        }
    }
}

pub fn build_diff(current: &[InstalledMod], target: &Profile) -> ProfileDiff {
    let mut actions = Vec::new();

    for target_mod in &target.mods {
        let current_match = find_installed_for_profile_mod(target_mod, current);

        match current_match {
            None => {
                let detail = match &target_mod.origin {
                    ModOrigin::Thunderstore { version, .. } => format!("thunderstore v{version}"),
                    ModOrigin::Nexus {
                        mod_id, version, ..
                    } => format!("nexus #{mod_id} v{version}"),
                    ModOrigin::Local { .. } => "local stash or thunderstore".to_string(),
                };
                actions.push(DiffAction::Install {
                    entry_name: target_mod.entry_name.clone(),
                    display_name: target_mod.display_name.clone(),
                    detail,
                });
            }
            Some(installed) => {
                let current_version = installed.version.clone().unwrap_or_else(|| "?".to_string());
                let target_version = match &target_mod.origin {
                    ModOrigin::Thunderstore { version, .. } => Some(version.clone()),
                    ModOrigin::Nexus { version, .. } => {
                        if version.is_empty() {
                            installed.version.clone()
                        } else {
                            Some(version.clone())
                        }
                    }
                    ModOrigin::Local { .. } => installed.version.clone(),
                };
                if let Some(wanted) = target_version {
                    if installed.version.as_deref() != Some(wanted.as_str()) {
                        actions.push(DiffAction::ReplaceVersion {
                            entry_name: installed.entry_name.clone(),
                            display_name: target_mod.display_name.clone(),
                            from_version: current_version,
                            to_version: wanted,
                        });
                    }
                }
                if installed.enabled != target_mod.enabled {
                    if target_mod.enabled {
                        actions.push(DiffAction::Enable {
                            entry_name: installed.entry_name.clone(),
                            display_name: target_mod.display_name.clone(),
                        });
                    } else {
                        actions.push(DiffAction::Disable {
                            entry_name: installed.entry_name.clone(),
                            display_name: target_mod.display_name.clone(),
                        });
                    }
                }
            }
        }
    }

    for installed in current {
        let still_wanted = target
            .mods
            .iter()
            .any(|entry| profile_mod_matches_installed(entry, installed));
        if !still_wanted {
            actions.push(DiffAction::Remove {
                entry_name: installed.entry_name.clone(),
                display_name: installed.display_name.clone(),
            });
        }
    }

    ProfileDiff { actions }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn installed(
        entry_name: &str,
        display_name: &str,
        version: Option<&str>,
        enabled: bool,
    ) -> InstalledMod {
        InstalledMod {
            entry_name: entry_name.to_string(),
            display_name: display_name.to_string(),
            version: version.map(str::to_string),
            enabled,
            path: PathBuf::from(format!("/tmp/{entry_name}")),
            kind: ModKind::Folder,
            source: ModSource::Thunderstore,
        }
    }

    #[test]
    fn diff_covers_install_remove_and_toggle() {
        let current = vec![
            installed("keep-Mod", "Keep", Some("1.0.0"), true),
            installed("gone-Mod", "Gone", Some("1.0.0"), true),
            installed("toggle-Mod", "Toggle", Some("1.0.0"), true),
        ];
        let target = Profile {
            name: "demo".into(),
            created_at_unix: 1,
            updated_at_unix: 1,
            mods: vec![
                ProfileMod {
                    entry_name: "keep-Mod".into(),
                    display_name: "Keep".into(),
                    enabled: true,
                    origin: ModOrigin::Thunderstore {
                        owner: "keep".into(),
                        package_name: "Mod".into(),
                        full_name: "keep-Mod".into(),
                        version: "1.0.0".into(),
                        download_url: thunderstore_download_url("keep", "Mod", "1.0.0"),
                    },
                },
                ProfileMod {
                    entry_name: "toggle-Mod".into(),
                    display_name: "Toggle".into(),
                    enabled: false,
                    origin: ModOrigin::Thunderstore {
                        owner: "toggle".into(),
                        package_name: "Mod".into(),
                        full_name: "toggle-Mod".into(),
                        version: "1.0.0".into(),
                        download_url: thunderstore_download_url("toggle", "Mod", "1.0.0"),
                    },
                },
                ProfileMod {
                    entry_name: "new-Mod".into(),
                    display_name: "New".into(),
                    enabled: true,
                    origin: ModOrigin::Thunderstore {
                        owner: "new".into(),
                        package_name: "Mod".into(),
                        full_name: "new-Mod".into(),
                        version: "2.0.0".into(),
                        download_url: thunderstore_download_url("new", "Mod", "2.0.0"),
                    },
                },
            ],
        };

        let diff = build_diff(&current, &target);
        assert!(diff.actions.iter().any(|action| matches!(
            action,
            DiffAction::Install {
                entry_name,
                ..
            } if entry_name == "new-Mod"
        )));
        assert!(diff.actions.iter().any(|action| matches!(
            action,
            DiffAction::Remove {
                entry_name,
                ..
            } if entry_name == "gone-Mod"
        )));
        assert!(diff.actions.iter().any(|action| matches!(
            action,
            DiffAction::Disable {
                entry_name,
                ..
            } if entry_name == "toggle-Mod"
        )));
    }
}
