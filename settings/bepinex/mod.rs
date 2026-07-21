mod install;
mod launch;
mod pack;
mod status;

pub use install::{
    apply_install_plan, build_install_plan, download_recommended_pack, InstallError, InstallSummary,
};
pub use launch::{
    ensure_launch_options, ensure_launch_script, inspect_injection, launch_silksong,
    open_launch_script, InjectionState, LaunchOptionsOutcome, LaunchOptionsPlan,
    LaunchScriptAction,
};
pub use pack::{RECOMMENDED_PACK_FULL_NAME, SMM_LAUNCH_SCRIPT, STEAM_LAUNCH_OPTIONS};
pub use status::{inspect_bepinex, BepinexStatus};

use crate::detection::SilksongInstall;
use std::path::Path;

pub fn recommended_status_for(install: &SilksongInstall) -> BepinexStatus {
    inspect_bepinex(&install.install_folder)
}

pub fn prepare_install(
    install: &SilksongInstall,
    pack_archive: &Path,
    dry_run: bool,
) -> Result<InstallSummary, InstallError> {
    let plan = build_install_plan(&install.install_folder, pack_archive)?;
    let launch_plan = LaunchOptionsPlan::for_build(install.build_kind);
    if dry_run {
        return Ok(InstallSummary {
            dry_run: true,
            events: plan.events.clone(),
            launch: LaunchOptionsOutcome::DryRun {
                launch_options: launch_plan.required_launch_options.to_string(),
            },
            backup_root: plan.backup_root.clone(),
        });
    }

    let events = apply_install_plan(&plan)?;
    let launch = ensure_launch_options(install, &launch_plan)?;
    Ok(InstallSummary {
        dry_run: false,
        events,
        launch,
        backup_root: plan.backup_root.clone(),
    })
}

pub fn install_recommended(
    install: &SilksongInstall,
    dry_run: bool,
) -> Result<InstallSummary, InstallError> {
    let archive = download_recommended_pack()?;
    prepare_install(install, &archive, dry_run)
}

pub fn configure_injection(install: &SilksongInstall) -> Result<LaunchOptionsOutcome, InstallError> {
    let launch_plan = LaunchOptionsPlan::for_build(install.build_kind);
    Ok(ensure_launch_options(install, &launch_plan)?)
}

pub fn reset_launch_script(install: &SilksongInstall) -> Result<String, InstallError> {
    let action = ensure_launch_script(install, true)?;
    let path = match action {
        launch::LaunchScriptAction::Created { path }
        | launch::LaunchScriptAction::AlreadyPresent { path }
        | launch::LaunchScriptAction::Reset { path } => path,
    };
    Ok(format!(
        "reset editable launch script at {}\nsteam should use: {STEAM_LAUNCH_OPTIONS}",
        path.display()
    ))
}

pub fn status_label(status: &BepinexStatus) -> &'static str {
    match status {
        BepinexStatus::NotInstalled => "not installed",
        BepinexStatus::InstalledCurrent { .. } => "installed and current",
        BepinexStatus::NeedsUpdate { .. } => "needs update",
    }
}

pub fn build_kind_launch_hint() -> &'static str {
    STEAM_LAUNCH_OPTIONS
}
