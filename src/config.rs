use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub pm_defaults: HashMap<String, String>, // family -> box_name
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub first_run_done: bool,
}

pub fn config_dir() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(format!("{home}/.config"))
    }).join("pkgbridge")
}

pub fn state_dir() -> PathBuf {
    std::env::var("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(format!("{home}/.local/state"))
    }).join("pkgbridge")
}

pub fn load_config() -> Config {
    let path = config_dir().join("config.toml");
    match fs::read_to_string(&path) {
        Ok(s) => toml::from_str(&s).unwrap_or_default(),
        Err(_) => Config::default(),
    }
}

pub fn save_config(cfg: &Config) -> Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir).ok();
    let path = dir.join("config.toml");
    let s = toml::to_string_pretty(cfg).unwrap_or_default();
    fs::write(&path, s).with_context(|| format!("writing {}", path.display()))
}

pub fn load_state() -> State {
    let path = state_dir().join("state.toml");
    match fs::read_to_string(&path) {
        Ok(s) => toml::from_str(&s).unwrap_or_default(),
        Err(_) => State::default(),
    }
}

pub fn save_state(st: &State) -> Result<()> {
    let dir = state_dir();
    fs::create_dir_all(&dir).ok();
    let path = dir.join("state.toml");
    let s = toml::to_string_pretty(st).unwrap_or_default();
    fs::write(&path, s).with_context(|| format!("writing {}", path.display()))
}

pub fn snapshot_dir() -> PathBuf { state_dir().join("snapshots") }

pub fn snapshot_path(container: &str) -> PathBuf { snapshot_dir().join(format!("{}.txt", container)) }
