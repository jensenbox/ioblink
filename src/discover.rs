use std::fs;
use std::io::{self, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::led;

const LEDS_ROOT: &str = "/sys/class/leds";
// _IOW('E', 0x90, int) -- see <linux/input.h>. Grabbing (value=1) an
// already-grabbed device fails with EBUSY; grabbing an ungrabbed one
// succeeds, and we immediately release it (value=0).
const EVIOCGRAB: libc::c_ulong = 0x4004_4590;

/// Preference order to suggest in the generated config, matching the
/// "nothing else uses it" reasoning: Scroll Lock, then Num Lock, then Caps
/// Lock.
const PREFERENCE: [&str; 3] = ["scrolllock", "numlock", "capslock"];

struct Candidate {
    led_path: PathBuf,
    lock_name: String,
    input_name: String,
    vendor: Option<String>,
    product: Option<String>,
    event_device: Option<PathBuf>,
}

pub fn run() -> io::Result<()> {
    println!("ioblink discover -- find which keyboard LED (if any) has a physical lamp\n");

    let candidates = list_candidates()?;
    if candidates.is_empty() {
        println!("No capslock/numlock/scrolllock LED nodes found under {LEDS_ROOT}.");
        println!("This usually means no attached keyboard exposes the standard input LEDs.");
        return Ok(());
    }

    println!("Found {} candidate LED node(s):\n", candidates.len());
    for c in &candidates {
        println!(
            "  {:<28} keyboard: {:<28} usb: {}",
            format!("{}", c.led_path.display()),
            c.input_name,
            match (&c.vendor, &c.product) {
                (Some(v), Some(p)) => format!("{v}:{p}"),
                _ => "unknown".to_string(),
            }
        );
    }
    println!();

    let mut confirmed: Vec<&Candidate> = Vec::new();
    let mut skipped_grabbed: Vec<&Candidate> = Vec::new();

    for c in &candidates {
        println!("--- {} ({}) ---", c.led_path.display(), c.lock_name);

        if let Some(event) = &c.event_device {
            if is_grabbed(event)? {
                println!(
                    "  SKIPPED: {} is held by an exclusive grab (EVIOCGRAB) right now.",
                    event.display()
                );
                println!(
                    "  A light test here would be misleading: writes to brightness report \
                     success but silently do nothing while a grab is held (see the README's \
                     EVIOCGRAB gotcha)."
                );
                let openers = find_openers(event);
                if openers.is_empty() {
                    println!("  Could not identify the holder (need root to read /proc/*/fd).");
                } else {
                    println!("  Processes with this device open (one likely holds the grab):");
                    for (pid, comm) in &openers {
                        println!("    pid {pid}: {comm}");
                    }
                }
                println!("  Close/stop that process and re-run `ioblink discover` to test it.\n");
                skipped_grabbed.push(c);
                continue;
            }
        }

        let led = match led::Led::acquire(c.led_path.clone()) {
            Ok(led) => led,
            Err(e) => {
                println!("  SKIPPED: couldn't take control of this LED: {e}\n");
                continue;
            }
        };

        print!(
            "  Look at the keyboard now. Press Enter when you're watching it, and I'll blink \
             {} three times: ",
            c.lock_name
        );
        io::stdout().flush()?;
        let mut discard = String::new();
        io::stdin().read_line(&mut discard)?;

        for _ in 0..3 {
            let _ = led.set_on(true);
            std::thread::sleep(Duration::from_millis(400));
            let _ = led.set_on(false);
            std::thread::sleep(Duration::from_millis(400));
        }

        let lit = prompt_yes_no("  Did a physical LED just blink 3 times? [y/N]: ")?;
        let _ = led.restore();

        if lit {
            println!("  -> confirmed working.\n");
            confirmed.push(c);
        } else {
            println!("  -> no light on this node.\n");
        }
    }

    print_summary(&confirmed, &skipped_grabbed);
    Ok(())
}

fn print_summary(confirmed: &[&Candidate], skipped_grabbed: &[&Candidate]) {
    println!("=== Summary ===\n");

    if confirmed.is_empty() {
        println!("No candidate lit up.");
        if !skipped_grabbed.is_empty() {
            println!(
                "{} node(s) were skipped because they were grabbed -- resolve that and re-run \
                 before concluding this keyboard has no usable LED.",
                skipped_grabbed.len()
            );
        } else {
            println!(
                "This keyboard likely has no usable LED for this technique. See the README's \
                 fallback note (a dedicated USB indicator)."
            );
        }
        return;
    }

    let best = PREFERENCE
        .iter()
        .find_map(|name| confirmed.iter().find(|c| c.lock_name == *name))
        .unwrap_or(&confirmed[0]);

    println!(
        "Recommended: {} on {} (usb {}:{})\n",
        best.lock_name,
        best.input_name,
        best.vendor.as_deref().unwrap_or("?"),
        best.product.as_deref().unwrap_or("?")
    );
    println!("Paste this into your config:\n");
    println!("[led.selector]");
    println!(
        "vendor = \"{}\"",
        best.vendor.as_deref().unwrap_or("REPLACE_ME")
    );
    println!(
        "product = \"{}\"",
        best.product.as_deref().unwrap_or("REPLACE_ME")
    );
    println!("name = \"{}\"", best.lock_name);

    if confirmed.len() > 1 {
        println!(
            "\n({} other node(s) also lit up; scrolllock is preferred since nothing else \
             usually depends on it.)",
            confirmed.len() - 1
        );
    }
}

fn prompt_yes_no(prompt: &str) -> io::Result<bool> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}

fn list_candidates() -> io::Result<Vec<Candidate>> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(LEDS_ROOT) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e),
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(lock_name) = ["capslock", "numlock", "scrolllock"]
            .iter()
            .find(|n| name.ends_with(&format!("::{n}")))
        else {
            continue;
        };

        let Ok(real_path) = fs::canonicalize(entry.path()) else {
            continue;
        };
        let ids = led::find_usb_ids(&real_path);
        let input_dir = find_input_dir(&real_path);
        let input_name = input_dir
            .as_ref()
            .and_then(|d| fs::read_to_string(d.join("name")).ok())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown input device".to_string());
        let event_device = input_dir.as_ref().and_then(find_event_device);

        out.push(Candidate {
            led_path: entry.path(),
            lock_name: lock_name.to_string(),
            input_name,
            vendor: ids.as_ref().map(|(v, _)| v.clone()),
            product: ids.as_ref().map(|(_, p)| p.clone()),
            event_device,
        });
    }

    out.sort_by(|a, b| a.led_path.cmp(&b.led_path));
    Ok(out)
}

/// From an LED's canonicalized path (.../input/inputN/inputN::name), find
/// the inputN directory.
fn find_input_dir(led_real_path: &Path) -> Option<PathBuf> {
    let mut dir = led_real_path.parent();
    while let Some(d) = dir {
        if d.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("input"))
            && d.join("name").is_file()
        {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

/// Find the /dev/input/eventN node under an inputN sysfs directory.
fn find_event_device(input_dir: &PathBuf) -> Option<PathBuf> {
    fs::read_dir(input_dir).ok()?.flatten().find_map(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        name.starts_with("event")
            .then(|| PathBuf::from("/dev/input").join(name))
    })
}

/// True if something else currently holds an exclusive EVIOCGRAB on this
/// device: we try to grab it ourselves and immediately release it. Requires
/// read-write access to the device node (root, typically).
fn is_grabbed(event_path: &Path) -> io::Result<bool> {
    let file = match fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(event_path)
    {
        Ok(f) => f,
        Err(e) => {
            println!(
                "  (couldn't open {} to test for a grab: {e})",
                event_path.display()
            );
            return Ok(false);
        }
    };
    let fd = file.as_raw_fd();
    let value: libc::c_int = 1;
    let ret = unsafe { libc::ioctl(fd, EVIOCGRAB, &value as *const libc::c_int) };
    if ret == 0 {
        let release: libc::c_int = 0;
        unsafe { libc::ioctl(fd, EVIOCGRAB, &release as *const libc::c_int) };
        Ok(false)
    } else {
        Ok(true)
    }
}

/// Best-effort: walk /proc/*/fd looking for open file descriptors pointing
/// at `event_path`. Not proof any one of them holds the grab (a plain open
/// doesn't require it), but it's the same information `fuser -v` shows and
/// is exactly what we'd otherwise have to explain to a confused user.
fn find_openers(event_path: &Path) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let Ok(target) = fs::canonicalize(event_path) else {
        return out;
    };
    let Ok(procs) = fs::read_dir("/proc") else {
        return out;
    };

    for proc_entry in procs.flatten() {
        let Ok(pid) = proc_entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let fd_dir = proc_entry.path().join("fd");
        let Ok(fds) = fs::read_dir(&fd_dir) else {
            continue;
        };
        for fd_entry in fds.flatten() {
            if fs::canonicalize(fd_entry.path()).ok().as_deref() == Some(target.as_path()) {
                let comm = fs::read_to_string(proc_entry.path().join("comm"))
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                out.push((pid, comm));
                break;
            }
        }
    }
    out
}
