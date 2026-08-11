use super::pack::{
    LOG_OUTPUT, PLUGINS_FOLDER, RUN_SCRIPT, SMM_LAUNCH_SCRIPT,
};
#[cfg(not(target_os = "macos"))]
use super::pack::STEAM_LAUNCH_OPTIONS;
use crate::detection::{candidate_steam_roots, BuildKind, SilksongInstall, SILKSONG_APP_ID};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const PROTON_WINEDLL_OVERRIDE: &str = r#"winhttp=n,b"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchOptionsPlan {
    pub required_launch_options: String,
}

impl LaunchOptionsPlan {
    pub fn for_install(install: &SilksongInstall) -> Self {
        Self {
            required_launch_options: required_launch_options(install),
        }
    }
}

pub fn required_launch_options(install: &SilksongInstall) -> String {
    #[cfg(target_os = "macos")]
    {
        format!("\"{}\" %command%", launch_script_path(&install.install_folder).display())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = install;
        STEAM_LAUNCH_OPTIONS.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchScriptAction {
    Created { path: PathBuf },
    AlreadyPresent { path: PathBuf },
    Reset { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchOptionsOutcome {
    AlreadyConfigured {
        launch_options: String,
        script: LaunchScriptAction,
    },
    Updated {
        localconfig_path: PathBuf,
        launch_options: String,
        script: LaunchScriptAction,
        steam_restart_recommended: bool,
    },
    ManualPasteRequired {
        reason: String,
        launch_options: String,
        script: LaunchScriptAction,
    },
    DryRun {
        launch_options: String,
    },
}

impl LaunchOptionsOutcome {
    pub fn alert_message(&self) -> Option<String> {
        match self {
            Self::ManualPasteRequired {
                reason,
                launch_options,
                script,
            } => Some(format!(
                "launch script ready at {}, but steam launch options were NOT set automatically — {reason}. paste this into steam → silksong → properties → launch options:\n{launch_options}",
                script_path(script).display()
            )),
            Self::Updated {
                steam_restart_recommended: true,
                launch_options,
                script,
                ..
            } => Some(format!(
                "steam launch options were written to use {}, but steam was running — fully restart steam once so they stick:\n{launch_options}",
                script_path(script).display()
            )),
            _ => None,
        }
    }
}

fn script_path(action: &LaunchScriptAction) -> PathBuf {
    match action {
        LaunchScriptAction::Created { path }
        | LaunchScriptAction::AlreadyPresent { path }
        | LaunchScriptAction::Reset { path } => path.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InjectionState {
    NotConfigured {
        current_launch_options: Option<String>,
        required_launch_options: String,
        reason: String,
        launch_script: Option<PathBuf>,
    },
    ConfiguredAwaitingLaunch {
        launch_options: String,
        launch_script: PathBuf,
        steam_restart_recommended: bool,
    },
    Injected {
        launch_options: String,
        launch_script: PathBuf,
        log_path: PathBuf,
    },
}

impl InjectionState {
    pub fn label(&self) -> &'static str {
        match self {
            Self::NotConfigured { .. } => "not configured",
            Self::ConfiguredAwaitingLaunch {
                steam_restart_recommended: true,
                ..
            } => "launch options written — restart steam, then launch once",
            Self::ConfiguredAwaitingLaunch { .. } => "launch options set — launch game once",
            Self::Injected { .. } => "injected (log present)",
        }
    }

    pub fn is_configured(&self) -> bool {
        !matches!(self, Self::NotConfigured { .. })
    }
}

pub fn launch_script_path(install_folder: &Path) -> PathBuf {
    install_folder.join(SMM_LAUNCH_SCRIPT)
}

pub fn launch_script_body(build_kind: BuildKind) -> String {
    match build_kind {
        BuildKind::NativeLinux => format!(
            "#!/bin/sh\nset -eu\ncd \"$(dirname \"$0\")\"\nexec ./{RUN_SCRIPT} \"$@\"\n"
        ),
        BuildKind::NativeMacOS => format!(
            "#!/bin/sh\nset -eu\ncd \"$(dirname \"$0\")\"\nexec ./{RUN_SCRIPT} \"$@\"\n"
        ),
        BuildKind::Proton => format!(
            "#!/bin/sh\nset -eu\ncd \"$(dirname \"$0\")\"\nexport WINEDLLOVERRIDES=\"{PROTON_WINEDLL_OVERRIDE}\"\nexec \"$@\"\n"
        ),
    }
}

pub fn ensure_launch_script(
    install: &SilksongInstall,
    reset: bool,
) -> Result<LaunchScriptAction, std::io::Error> {
    let path = launch_script_path(&install.install_folder);
    if path.is_file() && !reset {
        return Ok(LaunchScriptAction::AlreadyPresent { path });
    }

    let body = launch_script_body(install.build_kind);
    {
        let mut file = fs::File::create(&path)?;
        file.write_all(body.as_bytes())?;
        file.flush()?;
    }
    set_executable(&path)?;

    if reset {
        Ok(LaunchScriptAction::Reset { path })
    } else {
        Ok(LaunchScriptAction::Created { path })
    }
}

pub fn open_launch_script(install: &SilksongInstall) -> Result<(), std::io::Error> {
    let path = launch_script_path(&install.install_folder);
    if !path.is_file() {
        ensure_launch_script(install, false)?;
    }
    crate::platform::open_path(&path)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchGameReport {
    pub method: String,
    pub injection_warning: Option<String>,
}

#[derive(Debug)]
pub enum LaunchGameError {
    BepinexMissing,
    Io(std::io::Error),
    SteamUnavailable { detail: String },
}

impl std::fmt::Display for LaunchGameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BepinexMissing => write!(
                formatter,
                "bepinex is not installed — install it in settings before playing modded"
            ),
            Self::Io(error) => write!(formatter, "could not prepare launch: {error}"),
            Self::SteamUnavailable { detail } => write!(
                formatter,
                "could not ask steam to start silksong: {detail}"
            ),
        }
    }
}

impl std::error::Error for LaunchGameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for LaunchGameError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn launch_silksong(install: &SilksongInstall) -> Result<LaunchGameReport, LaunchGameError> {
    use super::status::inspect_bepinex;
    use super::status::BepinexStatus;

    if matches!(
        inspect_bepinex(&install.install_folder),
        BepinexStatus::NotInstalled
    ) {
        return Err(LaunchGameError::BepinexMissing);
    }

    ensure_launch_script(install, false)?;

    let injection_warning = match inspect_injection(install) {
        InjectionState::NotConfigured { reason, .. } => Some(format!(
            "steam injection is not configured ({reason}). the game may start without mods until you fix launch options in settings"
        )),
        InjectionState::ConfiguredAwaitingLaunch {
            steam_restart_recommended: true,
            ..
        } => Some(
            "steam may still need a full restart before launch options stick".to_string(),
        ),
        _ => None,
    };

    let method = ask_steam_to_run_silksong()?;
    Ok(LaunchGameReport {
        method,
        injection_warning,
    })
}

fn ask_steam_to_run_silksong() -> Result<String, LaunchGameError> {
    let uri = format!("steam://rungameid/{SILKSONG_APP_ID}");

    match crate::platform::launch_steam_uri(&uri) {
        Ok(method) => Ok(method),
        Err(error) => Err(LaunchGameError::SteamUnavailable {
            detail: error.to_string(),
        }),
    }
}

pub fn inspect_injection(install: &SilksongInstall) -> InjectionState {
    let required = required_launch_options(install);
    let current = read_launch_options(SILKSONG_APP_ID).ok().flatten();
    let script = launch_script_path(&install.install_folder);
    let script_present = script.is_file();
    let log_path = install.install_folder.join(LOG_OUTPUT);
    let plugins_path = install.install_folder.join(PLUGINS_FOLDER);
    let runtime_evidence = log_path.is_file() || plugins_path.is_dir();

    match &current {
        Some(existing) if launch_options_satisfy(existing, install.build_kind) && script_present => {
            if runtime_evidence {
                InjectionState::Injected {
                    launch_options: existing.clone(),
                    launch_script: script,
                    log_path,
                }
            } else {
                InjectionState::ConfiguredAwaitingLaunch {
                    launch_options: existing.clone(),
                    launch_script: script,
                    steam_restart_recommended: false,
                }
            }
        }
        Some(existing) if launch_options_satisfy(existing, install.build_kind) && !script_present => {
            InjectionState::NotConfigured {
                current_launch_options: Some(existing.clone()),
                required_launch_options: required,
                reason: format!(
                    "steam points at {SMM_LAUNCH_SCRIPT}, but that script is missing from the game folder"
                ),
                launch_script: None,
            }
        }
        Some(existing) => {
            let reason = wrong_launch_options_reason(existing, &required);
            InjectionState::NotConfigured {
                current_launch_options: Some(existing.clone()),
                required_launch_options: required,
                reason,
                launch_script: script_present.then_some(script),
            }
        }
        None => InjectionState::NotConfigured {
            current_launch_options: None,
            required_launch_options: required,
            reason: "steam launch options are empty".to_string(),
            launch_script: script_present.then_some(script),
        },
    }
}

fn wrong_launch_options_reason(existing: &str, required: &str) -> String {
    format!(
        "current steam launch options do not use {SMM_LAUNCH_SCRIPT} (have: {existing}). needed: {required}"
    )
}

pub fn ensure_launch_options(
    install: &SilksongInstall,
    plan: &LaunchOptionsPlan,
) -> Result<LaunchOptionsOutcome, std::io::Error> {
    let script = ensure_launch_script(install, false)?;
    let required = &plan.required_launch_options;

    if let Some(existing) = read_launch_options(SILKSONG_APP_ID)? {
        if launch_options_satisfy(&existing, install.build_kind)
            && launch_script_path(&install.install_folder).is_file()
        {
            return Ok(LaunchOptionsOutcome::AlreadyConfigured {
                launch_options: existing,
                script,
            });
        }
    }

    let steam_was_running = steam_process_running();
    match write_launch_options(SILKSONG_APP_ID, required) {
        Ok(localconfig_path) => Ok(LaunchOptionsOutcome::Updated {
            localconfig_path,
            launch_options: required.to_string(),
            script,
            steam_restart_recommended: steam_was_running,
        }),
        Err(error) => Ok(LaunchOptionsOutcome::ManualPasteRequired {
            reason: format!("could not update steam localconfig.vdf ({error})"),
            launch_options: required.clone(),
            script,
        }),
    }
}

pub fn launch_options_satisfy(existing: &str, build_kind: BuildKind) -> bool {
    if existing.contains(SMM_LAUNCH_SCRIPT) {
        return true;
    }
    match build_kind {
        BuildKind::Proton => {
            existing.contains("WINEDLLOVERRIDES")
                && (existing.contains("winhttp=n,b") || existing.contains("winhttp.dll=n,b"))
        }
        BuildKind::NativeLinux | BuildKind::NativeMacOS => existing.contains("run_bepinex.sh"),
    }
}

fn set_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(permissions.mode() | 0o111);
    fs::set_permissions(path, permissions)
}

pub fn read_launch_options(app_id: u32) -> Result<Option<String>, std::io::Error> {
    for localconfig in discover_localconfig_paths()? {
        if let Some(value) = launch_options_from_localconfig(&localconfig, app_id)? {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

pub fn write_launch_options(app_id: u32, launch_options: &str) -> Result<PathBuf, std::io::Error> {
    let localconfig = discover_localconfig_paths()?
        .into_iter()
        .next()
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no steam userdata localconfig.vdf found",
            )
        })?;

    let original = fs::read_to_string(&localconfig)?;
    let updated = upsert_launch_options(&original, app_id, launch_options).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("could not locate apps/{app_id} block in localconfig.vdf"),
        )
    })?;

    let backup_path = localconfig.with_extension("vdf.silksong-backup");
    fs::copy(&localconfig, &backup_path)?;
    fs::write(&localconfig, updated)?;
    Ok(localconfig)
}

fn discover_localconfig_paths() -> Result<Vec<PathBuf>, std::io::Error> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "HOME unset"))?;

    let mut paths = Vec::new();
    for steam_root in candidate_steam_roots(&home) {
        let userdata = steam_root.join("userdata");
        if !userdata.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&userdata)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let localconfig = entry.path().join("config").join("localconfig.vdf");
            if localconfig.is_file() {
                paths.push(localconfig);
            }
        }
    }
    Ok(paths)
}

fn launch_options_from_localconfig(
    localconfig: &Path,
    app_id: u32,
) -> Result<Option<String>, std::io::Error> {
    let contents = fs::read_to_string(localconfig)?;
    Ok(extract_launch_options(&contents, app_id))
}

fn line_is_app_id_key(line: &str, app_id: u32) -> bool {
    line.trim() == format!("\"{app_id}\"")
}

fn next_nonempty_line<'a>(lines: &[&'a str], start: usize) -> Option<(usize, &'a str)> {
    let mut index = start;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if !trimmed.is_empty() {
            return Some((index, trimmed));
        }
        index += 1;
    }
    None
}

fn app_block_starts_at(lines: &[&str], app_line_index: usize, app_id: u32) -> bool {
    if !line_is_app_id_key(lines[app_line_index], app_id) {
        return false;
    }
    matches!(
        next_nonempty_line(lines, app_line_index + 1),
        Some((_, "{"))
    )
}

fn extract_launch_options(contents: &str, app_id: u32) -> Option<String> {
    let lines: Vec<&str> = contents.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        if app_block_starts_at(&lines, index, app_id) {
            if let Some(value) = scan_app_block_for_launch_options(&lines, index + 1) {
                return Some(value);
            }
        }
        index += 1;
    }
    None
}

fn scan_app_block_for_launch_options(lines: &[&str], mut index: usize) -> Option<String> {
    let mut depth: usize = 0;
    let mut saw_open = false;
    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed == "{" {
            depth += 1;
            saw_open = true;
        } else if trimmed == "}" {
            depth = depth.saturating_sub(1);
            if saw_open && depth == 0 {
                return None;
            }
        } else if depth == 1 {
            if let Some(value) = quoted_value_after_key(trimmed, "LaunchOptions") {
                return Some(value);
            }
        }
        index += 1;
    }
    None
}

fn upsert_launch_options(contents: &str, app_id: u32, launch_options: &str) -> Option<String> {
    let lines: Vec<&str> = contents.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        if app_block_starts_at(&lines, index, app_id) {
            return rewrite_app_block(&lines, index, launch_options);
        }
        index += 1;
    }
    None
}

fn rewrite_app_block(lines: &[&str], app_line_index: usize, launch_options: &str) -> Option<String> {
    let mut depth: usize = 0;
    let mut saw_open = false;
    let mut end_index = None;
    let mut launch_line_index = None;
    let mut index = app_line_index + 1;

    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed == "{" {
            depth += 1;
            saw_open = true;
        } else if trimmed == "}" {
            depth = depth.saturating_sub(1);
            if saw_open && depth == 0 {
                end_index = Some(index);
                break;
            }
        } else if depth == 1 && trimmed.starts_with("\"LaunchOptions\"") {
            launch_line_index = Some(index);
        }
        index += 1;
    }

    let end_index = end_index?;
    let indent = detect_launch_options_indent(lines, app_line_index, end_index);
    let encoded = escape_vdf_value(launch_options);
    let new_line = format!("{indent}\"LaunchOptions\"\t\t\"{encoded}\"");

    let mut output: Vec<String> = lines.iter().map(|line| (*line).to_string()).collect();
    if let Some(existing_index) = launch_line_index {
        output[existing_index] = new_line;
    } else {
        output.insert(end_index, new_line);
    }
    Some(output.join("\n"))
}

fn detect_launch_options_indent(lines: &[&str], app_line_index: usize, end_index: usize) -> String {
    for line in &lines[app_line_index + 1..end_index] {
        let trimmed = line.trim_start();
        if trimmed.starts_with('"') {
            let indent_len = line.len() - trimmed.len();
            return line[..indent_len].to_string();
        }
    }
    "\t\t\t\t".to_string()
}

fn quoted_value_after_key(line: &str, key: &str) -> Option<String> {
    let key_token = format!("\"{key}\"");
    let rest = line.trim().strip_prefix(&key_token)?.trim_start();
    let value = rest.trim().trim_matches('"');
    if value.is_empty() {
        None
    } else {
        Some(unescape_vdf_value(value))
    }
}

fn escape_vdf_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn unescape_vdf_value(value: &str) -> String {
    value.replace("\\\"", "\"").replace("\\\\", "\\")
}

pub fn steam_process_running() -> bool {
    crate::platform::process_named_running(&["steam", "Steam"])
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY_PROTON_LAUNCH_OPTIONS: &str =
        r#"WINEDLLOVERRIDES="winhttp=n,b" %command%"#;

    #[test]
    fn detects_proton_override() {
        assert!(launch_options_satisfy(
            r#"WINEDLLOVERRIDES="winhttp=n,b" %command%"#,
            BuildKind::Proton
        ));
        assert!(launch_options_satisfy(
            r#"WINEDLLOVERRIDES="winhttp.dll=n,b" %command%"#,
            BuildKind::Proton
        ));
        assert!(!launch_options_satisfy("%command%", BuildKind::Proton));
    }

    #[test]
    fn detects_native_runner() {
        assert!(launch_options_satisfy(
            "./run_bepinex.sh %command%",
            BuildKind::NativeLinux
        ));
        assert!(!launch_options_satisfy("%command%", BuildKind::NativeLinux));
        assert!(!launch_options_satisfy(
            r#"WINEDLLOVERRIDES="winhttp=n,b" %command%"#,
            BuildKind::NativeLinux
        ));
    }

    #[test]
    fn extracts_and_upserts_launch_options() {
        let original = r#"
"UserLocalConfigStore"
{
	"Software"
	{
		"Valve"
		{
			"Steam"
			{
				"apps"
				{
					"1030300"
					{
						"LaunchOptions"		"-windowed"
					}
				}
			}
		}
	}
}
"#;
        assert_eq!(
            extract_launch_options(original, 1_030_300).as_deref(),
            Some("-windowed")
        );
        let updated = upsert_launch_options(original, 1_030_300, LEGACY_PROTON_LAUNCH_OPTIONS)
            .expect("upsert");
        assert_eq!(
            extract_launch_options(&updated, 1_030_300).as_deref(),
            Some(LEGACY_PROTON_LAUNCH_OPTIONS)
        );
    }

    #[test]
    fn ignores_single_line_app_id_entries() {
        let contents = r#"
"Something"
{
	"1030300"		"deadbeefblob"
	"apps"
	{
		"1030300"
		{
			"LaunchOptions"		"./run_bepinex.sh %command%"
		}
	}
}
"#;
        assert_eq!(
            extract_launch_options(contents, 1_030_300).as_deref(),
            Some("./run_bepinex.sh %command%")
        );
    }

    #[test]
    fn accepts_smm_launch_script_for_any_build() {
        assert!(launch_options_satisfy(
            "./smm_launch.sh %command%",
            BuildKind::NativeLinux
        ));
        assert!(launch_options_satisfy(
            "./smm_launch.sh %command%",
            BuildKind::Proton
        ));
        assert!(launch_options_satisfy(
            "\"/Applications/Hollow Knight Silksong/smm_launch.sh\" %command%",
            BuildKind::NativeMacOS
        ));
    }

    #[test]
    fn native_launch_script_runs_bepinex_wrapper() {
        let body = launch_script_body(BuildKind::NativeLinux);
        assert!(body.contains("exec ./run_bepinex.sh"));
        assert!(!body.contains("WINEDLLOVERRIDES"));
    }

    #[test]
    fn macos_launch_script_runs_bepinex_wrapper() {
        let body = launch_script_body(BuildKind::NativeMacOS);
        assert!(body.contains("exec ./run_bepinex.sh"));
        assert!(!body.contains("WINEDLLOVERRIDES"));
    }

    #[test]
    fn proton_launch_script_sets_winhttp_override() {
        let body = launch_script_body(BuildKind::Proton);
        assert!(body.contains("WINEDLLOVERRIDES"));
        assert!(body.contains("winhttp=n,b"));
    }

    #[test]
    fn manual_paste_alert_is_unmistakable() {
        let launch_options = "./smm_launch.sh %command%";
        let outcome = LaunchOptionsOutcome::ManualPasteRequired {
            reason: "could not update steam localconfig.vdf".to_string(),
            launch_options: launch_options.to_string(),
            script: LaunchScriptAction::Created {
                path: PathBuf::from("/tmp/game/smm_launch.sh"),
            },
        };
        let alert = outcome.alert_message().expect("alert");
        assert!(alert.contains("NOT set automatically"));
        assert!(alert.contains(launch_options));
        assert!(alert.contains("smm_launch.sh"));
    }

    #[test]
    fn preserves_existing_launch_script_unless_reset() {
        let root = std::env::temp_dir().join(format!(
            "silksong-launch-script-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let install = SilksongInstall {
            install_folder: root.clone(),
            executable: root.join("Hollow Knight Silksong"),
            build_kind: BuildKind::NativeLinux,
            proton_prefix: None,
        };
        let path = launch_script_path(&root);
        fs::write(&path, b"#!/bin/sh\necho custom\n").unwrap();

        let kept = ensure_launch_script(&install, false).unwrap();
        assert!(matches!(kept, LaunchScriptAction::AlreadyPresent { .. }));
        assert_eq!(fs::read_to_string(&path).unwrap(), "#!/bin/sh\necho custom\n");

        let reset = ensure_launch_script(&install, true).unwrap();
        assert!(matches!(reset, LaunchScriptAction::Reset { .. }));
        assert!(fs::read_to_string(&path).unwrap().contains("run_bepinex.sh"));

        fs::remove_dir_all(root).ok();
    }
}
