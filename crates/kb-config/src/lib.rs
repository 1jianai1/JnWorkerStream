//! Settings persistence for JnWorkerStream.
//!
//! P1 scope (see `docs/interfaces.md` §6.1): schema-versioned JSON, defaults on
//! missing fields, backup-and-default on corruption, and an atomic-ish save.
//! Public command names and the Tauri layer are intentionally absent until P2a.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;
pub const CONFIG_FILE: &str = "config.json";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid settings JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported settings schema version {found} (expected {expected})")]
    UnsupportedSchema { found: u32, expected: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub schema_version: u32,
    pub link: LinkSettings,
    pub audio: AudioSettings,
    pub voice: VoiceSettings,
    pub logging: LoggingSettings,
    pub window: WindowSettings,
    pub ui: UiSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LinkSettings {
    pub kind: String,
    pub auto_reconnect: bool,
    pub serial: SerialSettings,
    pub ble: BleSettings,
    pub sim: SimSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SerialSettings {
    pub port: String,
    pub vid: u16,
    pub pid: u16,
    pub serial_number: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BleSettings {
    pub name: String,
    pub address: String,
    pub paired_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SimSettings {
    pub rate_hz: u32,
    pub size: u32,
    pub pattern: String,
    pub anomaly: String,
    pub burst: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AudioSettings {
    pub receive: bool,
    pub save_dir: String,
    pub playback: bool,
    pub buf_ms: u32,
    pub buffers: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VoiceSettings {
    pub enabled: bool,
    pub inject_mode: String,
    pub per_char_ms: u32,
    pub asr: AsrSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AsrSettings {
    pub model_path: String,
    pub device: String,
    pub compute_type: String,
    pub language: String,
    pub beam_size: u32,
    pub vad_filter: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LoggingSettings {
    pub level_min: String,
    pub max_entries: u32,
    pub domains: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WindowSettings {
    pub width: u32,
    pub height: u32,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub maximized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiSettings {
    pub theme: String,
    pub locale: String,
    pub keypad: KeypadSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct KeypadSettings {
    pub renderer: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            link: LinkSettings::default(),
            audio: AudioSettings::default(),
            voice: VoiceSettings::default(),
            logging: LoggingSettings::default(),
            window: WindowSettings::default(),
            ui: UiSettings::default(),
        }
    }
}

impl Default for LinkSettings {
    fn default() -> Self {
        Self {
            kind: "serial".into(),
            auto_reconnect: true,
            serial: SerialSettings::default(),
            ble: BleSettings::default(),
            sim: SimSettings::default(),
        }
    }
}

impl Default for SerialSettings {
    fn default() -> Self {
        Self {
            port: "COM3".into(),
            vid: 0,
            pid: 0,
            serial_number: String::new(),
            description: String::new(),
        }
    }
}

impl Default for BleSettings {
    fn default() -> Self {
        Self {
            name: "SF32".into(),
            address: String::new(),
            paired_only: false,
        }
    }
}

impl Default for SimSettings {
    fn default() -> Self {
        Self {
            rate_hz: 20,
            size: 64,
            pattern: "inc".into(),
            anomaly: "none".into(),
            burst: 0,
        }
    }
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            receive: false,
            save_dir: String::new(),
            playback: true,
            buf_ms: 20,
            buffers: 3,
        }
    }
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            inject_mode: "auto".into(),
            per_char_ms: 6,
            asr: AsrSettings::default(),
        }
    }
}

impl Default for AsrSettings {
    fn default() -> Self {
        Self {
            model_path: String::new(),
            device: "cpu".into(),
            compute_type: "int8".into(),
            language: "zh".into(),
            beam_size: 1,
            vad_filter: true,
        }
    }
}

impl Default for LoggingSettings {
    fn default() -> Self {
        Self {
            level_min: "info".into(),
            max_entries: 5000,
            domains: Vec::new(),
        }
    }
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 800,
            x: None,
            y: None,
            maximized: false,
        }
    }
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: "light".into(),
            locale: "zh-CN".into(),
            keypad: KeypadSettings::default(),
        }
    }
}

impl Default for KeypadSettings {
    fn default() -> Self {
        Self {
            renderer: "auto".into(),
        }
    }
}

impl Settings {
    /// Clamp values that the UI or a hand-edited file could put out of range.
    /// Invalid enum-like values fall back to documented defaults (ADR-016, ADR-015).
    pub fn normalize(&mut self) {
        if self.schema_version == 0 {
            self.schema_version = SCHEMA_VERSION;
        }
        if !matches!(self.ui.locale.as_str(), "zh-CN" | "en-US") {
            self.ui.locale = "zh-CN".into();
        }
        if !matches!(self.ui.keypad.renderer.as_str(), "auto" | "3d" | "2d") {
            self.ui.keypad.renderer = "auto".into();
        }
        if !matches!(self.link.kind.as_str(), "serial" | "ble" | "sim") {
            self.link.kind = "serial".into();
        }
        if !matches!(self.link.sim.pattern.as_str(), "inc" | "ascii" | "zero") {
            self.link.sim.pattern = "inc".into();
        }
        if !matches!(
            self.voice.inject_mode.as_str(),
            "auto" | "sendinput" | "clipboard" | "recognize_only"
        ) {
            self.voice.inject_mode = "auto".into();
        }
        if !matches!(
            self.logging.level_min.as_str(),
            "trace" | "debug" | "info" | "warn" | "error"
        ) {
            self.logging.level_min = "info".into();
        }
        self.link.sim.rate_hz = self.link.sim.rate_hz.clamp(1, 200);
        self.link.sim.size = self.link.sim.size.clamp(8, 1013);
        self.audio.buf_ms = self.audio.buf_ms.clamp(10, 60);
        self.logging.max_entries = self.logging.max_entries.clamp(100, 20_000);
        self.window.width = self.window.width.max(1040);
        self.window.height = self.window.height.max(680);
    }
}

#[derive(Debug, Clone)]
pub struct LoadOutcome {
    pub settings: Settings,
    /// Set when the original file could not be parsed and was backed up.
    pub corrupt_backup: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<LoadOutcome, Error> {
        if !self.path.exists() {
            return Ok(LoadOutcome {
                settings: Settings::default(),
                corrupt_backup: None,
            });
        }
        let text = fs::read_to_string(&self.path).map_err(|source| Error::Io {
            path: self.path.clone(),
            source,
        })?;
        match serde_json::from_str::<Settings>(&text) {
            Ok(mut settings) => {
                if settings.schema_version > SCHEMA_VERSION {
                    return Err(Error::UnsupportedSchema {
                        found: settings.schema_version,
                        expected: SCHEMA_VERSION,
                    });
                }
                settings.normalize();
                Ok(LoadOutcome {
                    settings,
                    corrupt_backup: None,
                })
            }
            Err(_) => {
                let backup = self.backup_corrupt()?;
                Ok(LoadOutcome {
                    settings: Settings::default(),
                    corrupt_backup: Some(backup),
                })
            }
        }
    }

    fn backup_corrupt(&self) -> Result<PathBuf, Error> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_millis())
            .unwrap_or(0);
        let backup = self
            .path
            .with_file_name(format!("config.corrupt-{stamp}.json"));
        fs::copy(&self.path, &backup).map_err(|source| Error::Io {
            path: backup.clone(),
            source,
        })?;
        Ok(backup)
    }

    pub fn save(&self, settings: &Settings) -> Result<(), Error> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let body = serde_json::to_string_pretty(settings)?;
        let temp = self.path.with_extension("json.tmp");
        fs::write(&temp, body).map_err(|source| Error::Io {
            path: temp.clone(),
            source,
        })?;
        #[cfg(windows)]
        if self.path.exists() {
            fs::remove_file(&self.path).map_err(|source| Error::Io {
                path: self.path.clone(),
                source,
            })?;
        }
        fs::rename(&temp, &self.path).map_err(|source| Error::Io {
            path: self.path.clone(),
            source,
        })
    }
}

/// `%APPDATA%\JnWorkerStream\config.json` on Windows; a local fallback elsewhere.
pub fn default_config_path() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("JnWorkerStream").join(CONFIG_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        let dir = base.join(format!("jnws-{label}-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_roundtrip() {
        let dir = temp_dir("roundtrip");
        let store = SettingsStore::new(dir.join("config.json"));
        let settings = Settings::default();
        store.save(&settings).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.settings, settings);
        assert!(loaded.corrupt_backup.is_none());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn partial_json_uses_defaults_and_normalizes() {
        let dir = temp_dir("partial");
        let path = dir.join("config.json");
        fs::write(
            &path,
            r#"{ "ui": { "locale": "fr-FR", "keypad": { "renderer": "vr" } }, "logging": { "max_entries": 999999 } }"#,
        )
        .unwrap();
        let loaded = SettingsStore::new(&path).load().unwrap();
        assert_eq!(loaded.settings.ui.locale, "zh-CN");
        assert_eq!(loaded.settings.ui.keypad.renderer, "auto");
        assert_eq!(loaded.settings.logging.max_entries, 20_000);
        assert_eq!(loaded.settings.link.sim.rate_hz, 20);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_file_is_backed_up_and_defaults_are_returned() {
        let dir = temp_dir("corrupt");
        let path = dir.join("config.json");
        fs::write(&path, "{ not json").unwrap();
        let loaded = SettingsStore::new(&path).load().unwrap();
        assert!(loaded.corrupt_backup.is_some());
        assert_eq!(loaded.settings, Settings::default());
        assert!(loaded.corrupt_backup.unwrap().exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn future_schema_is_rejected() {
        let dir = temp_dir("schema");
        let path = dir.join("config.json");
        fs::write(&path, r#"{ "schema_version": 99 }"#).unwrap();
        let error = SettingsStore::new(&path).load().unwrap_err();
        assert!(matches!(error, Error::UnsupportedSchema { found: 99, .. }));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn config_path_is_under_appdata_when_present() {
        let path = default_config_path();
        assert!(path.ends_with(Path::new("JnWorkerStream").join(CONFIG_FILE)));
    }
}
