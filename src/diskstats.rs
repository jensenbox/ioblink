use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::error::Error;

/// Sector counters for one block device, as read from /proc/diskstats.
///
/// /proc/diskstats fields are documented in
/// Documentation/admin-guide/iostats.rst, numbered starting after
/// major/minor/name. Field 3 is sectors read, field 7 is sectors written.
#[derive(Debug, Clone, Copy, Default)]
pub struct Counters {
    pub sectors_read: u64,
    pub sectors_written: u64,
}

pub fn read_diskstats() -> Result<HashMap<String, Counters>, Error> {
    let path = Path::new("/proc/diskstats");
    let content = fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let mut map = HashMap::new();
    for line in content.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // major minor name <field1..field7+>
        if fields.len() < 10 {
            continue;
        }
        let name = fields[2].to_string();
        let sectors_read: u64 = fields[5].parse().unwrap_or(0);
        let sectors_written: u64 = fields[9].parse().unwrap_or(0);
        map.insert(
            name,
            Counters {
                sectors_read,
                sectors_written,
            },
        );
    }
    Ok(map)
}

/// Whole-disk NVMe device names present under /sys/block, e.g. "nvme0n1".
/// Excludes partitions (nvme0n1p1 never appears as a top-level /sys/block
/// entry -- it's nested under nvme0n1/).
pub fn autodetect_nvme_devices() -> Result<Vec<String>, Error> {
    let path = Path::new("/sys/block");
    let entries = fs::read_dir(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let mut devices = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_nvme_whole_disk(&name) {
            devices.push(name);
        }
    }
    devices.sort();
    Ok(devices)
}

fn is_nvme_whole_disk(name: &str) -> bool {
    // nvme<ctrl>n<ns>, e.g. nvme0n1. Reject partitions like nvme0n1p1.
    let Some(rest) = name.strip_prefix("nvme") else {
        return false;
    };
    let Some(n_pos) = rest.find('n') else {
        return false;
    };
    let (ctrl, tail) = rest.split_at(n_pos);
    if ctrl.is_empty() || !ctrl.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let ns = &tail[1..];
    !ns.is_empty() && ns.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_whole_disks() {
        assert!(is_nvme_whole_disk("nvme0n1"));
        assert!(is_nvme_whole_disk("nvme1n1"));
        assert!(is_nvme_whole_disk("nvme10n2"));
    }

    #[test]
    fn rejects_partitions_and_junk() {
        assert!(!is_nvme_whole_disk("nvme0n1p1"));
        assert!(!is_nvme_whole_disk("nvme0n1p12"));
        assert!(!is_nvme_whole_disk("sda"));
        assert!(!is_nvme_whole_disk("sda1"));
        assert!(!is_nvme_whole_disk("nvme"));
        assert!(!is_nvme_whole_disk("nvmeXnY"));
    }
}
