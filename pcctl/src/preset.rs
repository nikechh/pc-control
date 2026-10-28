//! Presets: small TOML files naming what to apply, e.g.
//!
//! ```toml
//! # %APPDATA%\pc-control\presets\gaming.toml
//! openrgb_profile = "gaming"
//! ```
//!
//! Mouse settings join this file in milestone 2.

use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    /// Name of a profile saved in OpenRGB.
    pub openrgb_profile: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum PresetError {
    #[error("preset name \"{0}\" may only contain letters, digits, '-' and '_' (or pass a path to a .toml file)")]
    BadName(String),
    #[error("APPDATA isn't set, so the presets folder can't be found; pass a path to the .toml file instead")]
    NoAppData,
    #[error("preset file not found: {}", .0.display())]
    NotFound(PathBuf),
    #[error("can't read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{}: {message}", path.display())]
    Invalid { path: PathBuf, message: String },
    #[error("{}: the preset doesn't set anything (for example: openrgb_profile = \"default\")", .0.display())]
    Empty(PathBuf),
}

pub fn presets_dir() -> Result<PathBuf, PresetError> {
    let appdata = std::env::var_os("APPDATA").ok_or(PresetError::NoAppData)?;
    Ok(PathBuf::from(appdata).join("pc-control").join("presets"))
}

/// A path (anything ending in `.toml` or containing a separator) is used as is;
/// a bare name maps to `<presets_dir>/<name>.toml`.
pub fn resolve_path(arg: &str) -> Result<PathBuf, PresetError> {
    if arg.ends_with(".toml") || arg.contains(['/', '\\']) {
        return Ok(PathBuf::from(arg));
    }
    if arg.is_empty() || !arg.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(PresetError::BadName(arg.to_string()));
    }
    Ok(presets_dir()?.join(format!("{arg}.toml")))
}

pub fn load(path: &Path) -> Result<Preset, PresetError> {
    let text = std::fs::read_to_string(path).map_err(|source| match source.kind() {
        io::ErrorKind::NotFound => PresetError::NotFound(path.to_path_buf()),
        _ => PresetError::Read { path: path.to_path_buf(), source },
    })?;
    parse(&text, path)
}

pub fn parse(text: &str, path: &Path) -> Result<Preset, PresetError> {
    let preset: Preset = toml::from_str(text)
        .map_err(|e| PresetError::Invalid { path: path.to_path_buf(), message: e.message().to_string() })?;
    if preset.openrgb_profile.is_none() {
        return Err(PresetError::Empty(path.to_path_buf()));
    }
    Ok(preset)
}
