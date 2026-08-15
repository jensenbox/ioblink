use serde::Deserialize;
use std::path::{Path, PathBuf};

use crate::error::Error;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CountMode {
    Read,
    Write,
    #[default]
    Both,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LedSelector {
    /// USB idVendor, hex, no 0x prefix (e.g. "03f0").
    pub vendor: String,
    /// USB idProduct, hex, no 0x prefix (e.g. "344a").
    pub product: String,
    /// LED name suffix as it appears after `::` in /sys/class/leds
    /// (e.g. "scrolllock", "numlock", "capslock").
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LedConfig {
    /// Exact sysfs LED directory, e.g. /sys/class/leds/input4::scrolllock.
    /// Takes precedence over `selector` when set. Not recommended as the
    /// sole configuration: the `inputN` numbering is not stable across
    /// reboots or USB re-enumeration.
    pub path: Option<PathBuf>,
    /// Resolve the LED dynamically by walking /sys/class/leds and matching
    /// the backing USB device's vendor/product ID. Survives renumbering.
    pub selector: Option<LedSelector>,
}

fn default_poll_interval_ms() -> u64 {
    50
}

fn default_min_on_time_ms() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize)]
pub struct TimingConfig {
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
    #[serde(default = "default_min_on_time_ms")]
    pub min_on_time_ms: u64,
}

impl Default for TimingConfig {
    fn default() -> Self {
        TimingConfig {
            poll_interval_ms: default_poll_interval_ms(),
            min_on_time_ms: default_min_on_time_ms(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WatchConfig {
    /// Explicit block devices to watch, e.g. ["nvme0n1"]. Empty means
    /// autodetect every nvme*n* whole disk present at startup.
    #[serde(default)]
    pub devices: Vec<String>,
    #[serde(default)]
    pub count: CountMode,
    /// false (default): LED off at idle, on during activity (classic HDD
    /// LED behavior). true: LED on at idle, off during activity.
    #[serde(default)]
    pub invert: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub led: LedConfig,
    #[serde(default)]
    pub watch: WatchConfig,
    #[serde(default)]
    pub timing: TimingConfig,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, Error> {
        let text = std::fs::read_to_string(path).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        toml::from_str(&text).map_err(|source| Error::Config {
            path: path.to_path_buf(),
            source: Box::new(source),
        })
    }
}
