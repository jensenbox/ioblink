use std::fs;
use std::path::{Path, PathBuf};

use log::{info, warn};

use crate::config::LedSelector;
use crate::error::Error;

const LEDS_ROOT: &str = "/sys/class/leds";

/// Resolve an LED's sysfs directory by walking /sys/class/leds and matching
/// the backing USB device's vendor/product ID against `selector`.
///
/// Matching by vendor/product (rather than trusting a fixed `inputN::name`
/// path) is what makes this survive USB re-enumeration: the `N` in `inputN`
/// is assigned in enumeration order and is not stable across reboots or
/// unplug/replug.
pub fn resolve_by_selector(selector: &LedSelector) -> Result<PathBuf, Error> {
    let suffix = format!("::{}", selector.name);
    let entries = fs::read_dir(LEDS_ROOT).map_err(|source| Error::Io {
        path: PathBuf::from(LEDS_ROOT),
        source,
    })?;

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(&suffix) {
            continue;
        }
        let Ok(real_path) = fs::canonicalize(entry.path()) else {
            continue;
        };
        if device_matches(&real_path, &selector.vendor, &selector.product) {
            return Ok(entry.path());
        }
    }

    Err(Error::LedSelectorNotFound {
        vendor: selector.vendor.clone(),
        product: selector.product.clone(),
        name: selector.name.clone(),
    })
}

/// Walk up from an LED's canonicalized sysfs path looking for the first
/// ancestor directory that exposes idVendor/idProduct (the USB device
/// level -- HID and input intermediate levels don't have these attrs).
fn device_matches(led_real_path: &Path, vendor: &str, product: &str) -> bool {
    let mut dir = led_real_path;
    while let Some(parent) = dir.parent() {
        let vendor_file = parent.join("idVendor");
        let product_file = parent.join("idProduct");
        if vendor_file.is_file() && product_file.is_file() {
            let v = fs::read_to_string(&vendor_file).unwrap_or_default();
            let p = fs::read_to_string(&product_file).unwrap_or_default();
            return v.trim().eq_ignore_ascii_case(vendor) && p.trim().eq_ignore_ascii_case(product);
        }
        dir = parent;
    }
    false
}

pub struct Led {
    dir: PathBuf,
    original_trigger: String,
}

impl Led {
    /// Take control of the LED at `dir`: detach whatever kernel trigger is
    /// currently driving it and remember the original so it can be restored
    /// on shutdown.
    pub fn acquire(dir: PathBuf) -> Result<Self, Error> {
        let brightness_path = dir.join("brightness");
        let trigger_path = dir.join("trigger");

        if !brightness_path.exists() {
            return Err(Error::LedNodeMissing(dir));
        }

        let original_trigger = read_active_trigger(&trigger_path)?;
        write_sysfs(&trigger_path, "none")?;
        info!(
            "acquired LED {} (was driven by trigger '{}')",
            dir.display(),
            original_trigger
        );

        Ok(Led {
            dir,
            original_trigger,
        })
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// Set brightness and verify the write actually took effect. A silent
    /// no-op write (reports success, value doesn't change) is exactly what
    /// happens when another process holds an exclusive EVIOCGRAB on the
    /// backing input device -- see error::Error::WriteVerifyMismatch.
    pub fn set_on(&self, on: bool) -> Result<(), Error> {
        let brightness_path = self.dir.join("brightness");
        let value = if on { "1" } else { "0" };
        write_sysfs(&brightness_path, value)?;

        let read_back = fs::read_to_string(&brightness_path)
            .map_err(|source| Error::Io {
                path: brightness_path.clone(),
                source,
            })?
            .trim()
            .to_string();

        if read_back != value {
            return Err(Error::WriteVerifyMismatch {
                path: brightness_path,
                wrote: value.to_string(),
                read_back,
            });
        }
        Ok(())
    }

    /// Restore the original kernel trigger so the key behaves normally
    /// again. Called on clean shutdown and from the resume hook.
    pub fn restore(&self) -> Result<(), Error> {
        let trigger_path = self.dir.join("trigger");
        write_sysfs(&trigger_path, &self.original_trigger)?;
        info!(
            "restored LED {} to trigger '{}'",
            self.dir.display(),
            self.original_trigger
        );
        Ok(())
    }

    /// Re-detach the trigger without forgetting the originally-saved value.
    /// Used after a suspend/resume cycle or a replug, in case the kernel
    /// silently re-attached a trigger while we weren't looking.
    pub fn reassert(&self) -> Result<(), Error> {
        let trigger_path = self.dir.join("trigger");
        let current = read_active_trigger(&trigger_path)?;
        if current != "none" {
            warn!(
                "LED {} trigger drifted to '{}' (expected 'none'), re-detaching",
                self.dir.display(),
                current
            );
            write_sysfs(&trigger_path, "none")?;
        }
        Ok(())
    }
}

fn read_active_trigger(trigger_path: &Path) -> Result<String, Error> {
    let content = fs::read_to_string(trigger_path).map_err(|source| Error::Io {
        path: trigger_path.to_path_buf(),
        source,
    })?;
    for token in content.split_whitespace() {
        if let Some(inner) = token.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            return Ok(inner.to_string());
        }
    }
    Ok("none".to_string())
}

fn write_sysfs(path: &Path, value: &str) -> Result<(), Error> {
    std::fs::write(path, value).map_err(|source| Error::LedNodeNotWritable {
        path: path.to_path_buf(),
        source,
    })
}
