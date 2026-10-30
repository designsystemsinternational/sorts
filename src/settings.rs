//! Personal settings and state. These live outside the Fonts folder and are never shared.

use anyhow::Context;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const CONFIG_DIR_ENV: &str = "SORTS_CONFIG_DIR";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub root: Option<PathBuf>,
    /// Folder entries and font entries (see `entry`).
    pub enabled: Vec<String>,
    /// Collection IDs.
    pub enabled_collections: Vec<String>,
    /// Preferred file extensions when several active files share a PostScript name.
    pub format_order: Vec<String>,
}

/// The scanned file formats, in the default order.
pub const FORMATS: [&str; 4] = ["otf", "ttf", "ttc", "otc"];

impl Default for Settings {
    fn default() -> Self {
        Self {
            root: None,
            enabled: vec![],
            enabled_collections: vec![],
            format_order: FORMATS.map(String::from).to_vec(),
        }
    }
}

impl Settings {
    /// All scanned formats: first the ones in `format_order`, then the missing ones in the
    /// default order. Unknown formats and repeats are left out.
    pub fn full_format_order(&self) -> Vec<String> {
        let listed = self.format_order.iter().filter_map(|f| FORMATS.into_iter().find(|k| f.eq_ignore_ascii_case(k)));
        let mut order: Vec<String> = vec![];
        for format in listed.chain(FORMATS) {
            if !order.iter().any(|f| f == format) {
                order.push(format.to_string());
            }
        }
        order
    }
}

/// What the CLI registered with the system. Used to never unregister fonts from other apps.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub registered: BTreeSet<PathBuf>,
}

/// The folder with `config.json` and `state.json`.
#[derive(Debug, Clone)]
pub struct ConfigDir {
    pub path: PathBuf,
}

impl ConfigDir {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The settings folder in a home folder.
    pub fn default_path(home: &Path) -> PathBuf {
        home.join("Library/Application Support/sorts")
    }

    /// `$SORTS_CONFIG_DIR`, or `~/Library/Application Support/sorts`.
    pub fn from_env() -> anyhow::Result<Self> {
        if let Some(dir) = std::env::var_os(CONFIG_DIR_ENV) {
            return Ok(Self::new(dir));
        }
        let home = std::env::var_os("HOME").context("HOME is not set")?;
        Ok(Self::new(Self::default_path(Path::new(&home))))
    }

    /// Missing file: default settings.
    pub fn load_settings(&self) -> anyhow::Result<Settings> {
        read_json(&self.path.join("config.json"))
    }

    pub fn save_settings(&self, settings: &Settings) -> anyhow::Result<()> {
        write_json(&self.path.join("config.json"), settings)
    }

    /// Missing file: empty state.
    pub fn load_state(&self) -> anyhow::Result<State> {
        read_json(&self.path.join("state.json"))
    }

    pub fn save_state(&self, state: &State) -> anyhow::Result<()> {
        write_json(&self.path.join("state.json"), state)
    }
}

fn read_json<T: DeserializeOwned + Default>(path: &Path) -> anyhow::Result<T> {
    match fs::read(path) {
        Ok(data) => serde_json::from_slice(&data).with_context(|| format!("Invalid JSON in {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e).with_context(|| format!("Cannot read {}", path.display())),
    }
}

/// Writes JSON through a temporary file, so a crash never leaves a half-written file.
pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let dir = path.parent().context("path has no parent folder")?;
    fs::create_dir_all(dir).with_context(|| format!("Cannot create {}", dir.display()))?;
    let mut json = serde_json::to_string_pretty(value)?;
    json.push('\n');
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).with_context(|| format!("Cannot write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("Cannot write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_folder_is_in_application_support() {
        assert_eq!(
            ConfigDir::default_path(Path::new("/Users/someone")),
            Path::new("/Users/someone/Library/Application Support/sorts")
        );
    }

    #[test]
    fn missing_files_give_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = ConfigDir::new(dir.path().join("does-not-exist"));
        assert_eq!(config.load_settings().unwrap(), Settings::default());
        assert_eq!(config.load_state().unwrap(), State::default());
    }

    #[test]
    fn default_format_order_prefers_otf() {
        assert_eq!(Settings::default().format_order, ["otf", "ttf", "ttc", "otc"]);
    }

    #[test]
    fn full_format_order_adds_the_missing_formats_in_the_default_order() {
        let settings = Settings { format_order: vec!["TTF".into(), "woff".into(), "ttf".into()], ..Default::default() };
        assert_eq!(settings.full_format_order(), ["ttf", "otf", "ttc", "otc"]);
        assert_eq!(Settings::default().full_format_order(), ["otf", "ttf", "ttc", "otc"]);
    }

    #[test]
    fn settings_round_trip_and_create_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let config = ConfigDir::new(dir.path().join("nested/sorts"));
        let settings = Settings {
            root: Some("/fonts".into()),
            enabled: vec!["A/".into(), "B/One.otf".into()],
            enabled_collections: vec!["5f0c2a8e".into()],
            format_order: vec!["ttf".into(), "otf".into()],
        };

        config.save_settings(&settings).unwrap();

        assert_eq!(config.load_settings().unwrap(), settings);
    }

    #[test]
    fn settings_file_uses_camel_case_keys() {
        let dir = tempfile::tempdir().unwrap();
        let config = ConfigDir::new(dir.path());
        config.save_settings(&Settings::default()).unwrap();

        let json = std::fs::read_to_string(dir.path().join("config.json")).unwrap();

        assert!(json.contains("\"enabledCollections\""));
        assert!(json.contains("\"formatOrder\""));
    }

    #[test]
    fn missing_keys_in_the_settings_file_use_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), r#"{"enabled": ["A/"]}"#).unwrap();

        let settings = ConfigDir::new(dir.path()).load_settings().unwrap();

        assert_eq!(settings.enabled, ["A/"]);
        assert_eq!(settings.format_order, Settings::default().format_order);
    }

    #[test]
    fn invalid_settings_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), "{ nope").unwrap();
        assert!(ConfigDir::new(dir.path()).load_settings().is_err());
    }

    #[test]
    fn state_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let config = ConfigDir::new(dir.path());
        let state = State { registered: ["/fonts/A.otf".into(), "/fonts/B.ttf".into()].into() };

        config.save_state(&state).unwrap();

        assert_eq!(config.load_state().unwrap(), state);
    }
}
