use std::path::{Path, PathBuf};

use pcctl::preset::{self, PresetError};

fn temp_file(name: &str, contents: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pcctl-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn a_valid_preset_loads() {
    let path = temp_file("valid.toml", "openrgb_profile = \"default\"\n");
    assert_eq!(preset::load(&path).unwrap().openrgb_profile.as_deref(), Some("default"));
}

#[test]
fn a_missing_file_is_reported_with_its_path() {
    let err = preset::load(Path::new("C:/definitely/not/here.toml")).unwrap_err();
    assert!(matches!(err, PresetError::NotFound(_)), "{err}");
    assert!(err.to_string().contains("here.toml"));
}

#[test]
fn invalid_toml_is_reported() {
    let err = preset::parse("openrgb_profile = ", Path::new("bad.toml")).unwrap_err();
    assert!(matches!(err, PresetError::Invalid { .. }), "{err}");
}

#[test]
fn unknown_keys_are_rejected() {
    let err = preset::parse("openrgb_profile = \"a\"\nopenrgb_profle = \"b\"\n", Path::new("typo.toml")).unwrap_err();
    assert!(err.to_string().contains("openrgb_profle"), "{err}");
}

#[test]
fn an_empty_preset_is_rejected() {
    let err = preset::parse("", Path::new("empty.toml")).unwrap_err();
    assert!(matches!(err, PresetError::Empty(_)), "{err}");
}

#[test]
fn bare_names_map_into_the_presets_folder() {
    let path = preset::resolve_path("gaming").unwrap();
    assert!(path.ends_with("pc-control/presets/gaming.toml") || path.ends_with("pc-control\\presets\\gaming.toml"));
}

#[test]
fn paths_are_used_as_given() {
    assert_eq!(preset::resolve_path("C:\\x\\mine.toml").unwrap(), PathBuf::from("C:\\x\\mine.toml"));
    assert_eq!(preset::resolve_path("other.toml").unwrap(), PathBuf::from("other.toml"));
}

#[test]
fn names_cannot_escape_the_presets_folder() {
    for bad in ["..", "a b", "x:y", ""] {
        assert!(matches!(preset::resolve_path(bad), Err(PresetError::BadName(_))), "{bad:?}");
    }
}
