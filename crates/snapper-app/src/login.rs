//! Apply [`snapper_core::Settings::start_at_login`].
//!
//! The settings window that edits the flag is a later issue. This only
//! enables or disables the OS login entry for the current executable.

use auto_launch::AutoLaunchBuilder;

/// Enable or disable launching this executable at login.
///
/// A `false` flag removes an entry this app created earlier. When nothing is
/// registered, it leaves the OS setting alone.
pub fn apply(enabled: bool) -> Result<(), String> {
    let path = std::env::current_exe().map_err(|err| err.to_string())?;
    let path = path
        .to_str()
        .ok_or_else(|| "executable path is not valid Unicode".to_owned())?;
    let auto = AutoLaunchBuilder::new()
        .set_app_name("Snapper")
        .set_app_path(path)
        .set_use_launch_agent(true)
        .set_args(&[] as &[&str])
        .build()
        .map_err(|err| err.to_string())?;
    if enabled {
        auto.enable().map_err(|err| err.to_string())?;
        return Ok(());
    }
    match auto.is_enabled() {
        Ok(true) => auto.disable().map_err(|err| err.to_string()),
        Ok(false) => Ok(()),
        Err(err) => Err(err.to_string()),
    }
}
