use crate::detection::DetectionError;
use std::fs;
use std::path::{Path, PathBuf};

pub fn default_steam_common_folders() -> Result<Vec<PathBuf>, DetectionError> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(DetectionError::HomeUnavailable)?;

    let mut steam_roots = Vec::new();
    for root in candidate_steam_roots(&home) {
        if root.is_dir() {
            steam_roots.push(root);
        }
    }

    common_folders_from_steam_roots(&steam_roots)
}

pub fn candidate_steam_roots(home: &Path) -> Vec<PathBuf> {
    vec![
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
    ]
}

pub fn common_folders_from_steam_roots(
    steam_roots: &[PathBuf],
) -> Result<Vec<PathBuf>, DetectionError> {
    let mut common_folders = Vec::new();

    for steam_root in steam_roots {
        let vdf_path = steam_root.join("steamapps").join("libraryfolders.vdf");
        if vdf_path.is_file() {
            match fs::read_to_string(&vdf_path) {
                Ok(contents) => {
                    for library_path in library_paths_from_vdf(&contents) {
                        push_unique(
                            &mut common_folders,
                            library_path.join("steamapps").join("common"),
                        );
                    }
                }
                Err(error) => return Err(DetectionError::Io(error)),
            }
        }

        push_unique(
            &mut common_folders,
            steam_root.join("steamapps").join("common"),
        );
    }

    Ok(common_folders)
}

fn library_paths_from_vdf(contents: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    for line in contents.lines() {
        let trimmed = line.trim();
        let Some(after_key) = trimmed.strip_prefix("\"path\"") else {
            continue;
        };
        let value = after_key.trim().trim_matches('"');
        if value.is_empty() {
            continue;
        }
        paths.push(PathBuf::from(value));
    }

    paths
}

fn push_unique(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths.iter().any(|existing| existing == &candidate) {
        paths.push(candidate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch_directory(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("silksong-steam-{label}-{nanos}"));
        fs::create_dir_all(&directory).expect("create scratch");
        directory
    }

    #[test]
    fn reads_library_paths_from_vdf() {
        let contents = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"/home/player/.local/share/Steam"
		"apps"
		{
			"1030300"		"12000"
		}
	}
	"1"
	{
		"path"		"/mnt/games/SteamLibrary"
	}
}
"#;

        let paths = library_paths_from_vdf(contents);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/home/player/.local/share/Steam"),
                PathBuf::from("/mnt/games/SteamLibrary"),
            ]
        );
    }

    #[test]
    fn builds_common_folders_from_steam_root_and_vdf() {
        let root = scratch_directory("lib");
        let steamapps = root.join("steamapps");
        fs::create_dir_all(steamapps.join("common")).expect("common");

        let extra_library = root.join("extra_library");
        fs::create_dir_all(extra_library.join("steamapps").join("common")).expect("extra");

        let vdf = format!(
            "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
            root.display(),
            extra_library.display()
        );
        fs::write(steamapps.join("libraryfolders.vdf"), vdf).expect("write vdf");

        let folders = common_folders_from_steam_roots(&[root.clone()]).expect("common folders");
        assert!(folders.contains(&steamapps.join("common")));
        assert!(folders.contains(&extra_library.join("steamapps").join("common")));

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn candidate_roots_include_flatpak_and_snap() {
        let home = PathBuf::from("/home/player");
        let roots = candidate_steam_roots(&home);
        assert!(roots.contains(&home.join(".local").join("share").join("Steam")));
        assert!(roots.contains(
            &home
                .join(".var")
                .join("app")
                .join("com.valvesoftware.Steam")
                .join(".local")
                .join("share")
                .join("Steam")
        ));
        assert!(roots.contains(
            &home
                .join("snap")
                .join("steam")
                .join("common")
                .join(".local")
                .join("share")
                .join("Steam")
        ));
    }
}
