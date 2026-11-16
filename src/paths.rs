//! Platform directories. Only XDG-style locations for now (Linux and macOS).

use std::path::PathBuf;

fn xdg(var: &str, home_relative: &str) -> Option<PathBuf> {
    match std::env::var_os(var) {
        Some(p) if !p.is_empty() => Some(PathBuf::from(p)),
        _ => {
            let home = std::env::var_os("HOME").filter(|h| !h.is_empty())?;
            Some(PathBuf::from(home).join(home_relative))
        }
    }
}

/// Directory for user configuration (`config.toml`, `keymap.toml`, `themes/`).
pub fn config_dir() -> Option<PathBuf> {
    Some(xdg("XDG_CONFIG_HOME", ".config")?.join("oxpecker"))
}

/// Directory for logs and other state.
pub fn state_dir() -> Option<PathBuf> {
    Some(xdg("XDG_STATE_HOME", ".local/state")?.join("oxpecker"))
}
