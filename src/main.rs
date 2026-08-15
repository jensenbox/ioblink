mod config;
mod discover;
mod diskstats;
mod error;
mod led;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use clap::Parser;
use log::{error, info, warn};

use config::{Config, CountMode};
use error::Error;
use led::Led;

/// Blink a keyboard (or other input-device) LED in response to block device
/// read/write activity.
#[derive(Parser, Debug)]
#[command(name = "ioblink", version, about)]
struct Args {
    #[command(subcommand)]
    command: Option<SubCommand>,

    /// Path to the TOML config file. Only used when running the poller
    /// (i.e. not `ioblink discover`).
    #[arg(short, long, default_value = "/etc/ioblink/config.toml")]
    config: PathBuf,
}

#[derive(clap::Subcommand, Debug)]
enum SubCommand {
    /// Interactively find which keyboard LED (if any) has a physical lamp
    /// behind it, and print a ready-to-paste config snippet. Detects and
    /// flags LEDs whose backing input device is currently held by an
    /// exclusive grab, so a dead app doesn't get misread as dead hardware.
    Discover,
}

/// How long to keep retrying to acquire the LED at startup before giving
/// up, to ride out boot-time USB enumeration races. systemd's
/// Restart=always is the backstop past this.
const ACQUIRE_RETRY_INTERVAL: Duration = Duration::from_secs(1);
const ACQUIRE_MAX_ATTEMPTS: u32 = 30;

/// Once acquired, if the LED node vanishes at runtime (unplug), how often
/// to check whether it has come back (replug) rather than crash-looping.
const REACQUIRE_RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// Set by the SIGHUP handler (also invoked by the systemd-sleep resume
/// hook) to make the main loop re-verify the LED's trigger is still
/// detached. This is the belt-and-suspenders fix for "LED state after
/// resume is the thing most likely to be wrong."
static RESYNC_REQUESTED: AtomicBool = AtomicBool::new(false);
static SHOULD_EXIT: AtomicUsize = AtomicUsize::new(0);

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();

    if let Some(SubCommand::Discover) = args.command {
        return match discover::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                error!("{e}");
                ExitCode::FAILURE
            }
        };
    }

    if let Err(e) = install_signal_handlers() {
        error!("failed to install signal handlers: {e}");
        return ExitCode::FAILURE;
    }

    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn install_signal_handlers() -> Result<(), std::io::Error> {
    // SIGTERM/SIGINT: clean shutdown (restore original trigger, then exit).
    unsafe {
        signal_hook::low_level::register(signal_hook::consts::SIGTERM, || {
            SHOULD_EXIT.store(1, Ordering::SeqCst);
        })?;
        signal_hook::low_level::register(signal_hook::consts::SIGINT, || {
            SHOULD_EXIT.store(1, Ordering::SeqCst);
        })?;
        // SIGHUP: sent by the systemd-sleep resume hook (and available for
        // manual `systemctl kill -s HUP`) to request a trigger re-sync
        // without restarting the whole process.
        signal_hook::low_level::register(signal_hook::consts::SIGHUP, || {
            RESYNC_REQUESTED.store(true, Ordering::SeqCst);
        })?;
    }
    Ok(())
}

fn run(args: &Args) -> Result<(), Error> {
    let config = Config::load(&args.config)?;
    info!("loaded config from {}", args.config.display());

    let led = acquire_led_with_retry(&config)?;
    let devices = resolve_devices(&config)?;
    info!(
        "watching {} device(s): {} (mode: {:?}, poll: {}ms, min-on: {}ms)",
        devices.len(),
        devices.join(", "),
        config.watch.count,
        config.timing.poll_interval_ms,
        config.timing.min_on_time_ms
    );

    let final_led = poll_loop(&config, led, &devices);

    if let Some(led) = &final_led {
        if let Err(e) = led.restore() {
            warn!("failed to restore original trigger on shutdown: {e}");
        }
    }

    Ok(())
}

fn acquire_led_with_retry(config: &Config) -> Result<Led, Error> {
    let mut last_err = None;
    for attempt in 1..=ACQUIRE_MAX_ATTEMPTS {
        match resolve_led_path(config).and_then(Led::acquire) {
            Ok(led) => return Ok(led),
            Err(e) => {
                if attempt == 1 {
                    warn!(
                        "LED not ready yet ({e}), retrying for up to {}s",
                        ACQUIRE_MAX_ATTEMPTS as u64 * ACQUIRE_RETRY_INTERVAL.as_secs()
                    );
                }
                last_err = Some(e);
                std::thread::sleep(ACQUIRE_RETRY_INTERVAL);
            }
        }
    }
    Err(last_err.expect("loop runs at least once"))
}

fn resolve_led_path(config: &Config) -> Result<PathBuf, Error> {
    if let Some(path) = &config.led.path {
        return Ok(path.clone());
    }
    if let Some(selector) = &config.led.selector {
        return led::resolve_by_selector(selector);
    }
    Err(Error::Config {
        path: PathBuf::from("<config>"),
        source: "config.led must set either `path` or `selector`".into(),
    })
}

fn resolve_devices(config: &Config) -> Result<Vec<String>, Error> {
    if !config.watch.devices.is_empty() {
        return Ok(config.watch.devices.clone());
    }
    let devices = diskstats::autodetect_nvme_devices()?;
    if devices.is_empty() {
        return Err(Error::NoBlockDevicesFound {
            configured: config.watch.devices.clone(),
        });
    }
    Ok(devices)
}

/// State of our relationship with the LED: either we hold it and can drive
/// it, or it has vanished (unplug) and we're waiting for it to come back.
enum LedState {
    Acquired(Led),
    Missing { next_retry: Instant },
}

/// Runs until SHOULD_EXIT is set. Returns the last-acquired Led (if any) so
/// the caller can restore its trigger on the way out.
fn poll_loop(config: &Config, initial_led: Led, devices: &[String]) -> Option<Led> {
    let poll_interval = Duration::from_millis(config.timing.poll_interval_ms);
    let min_on_time = Duration::from_millis(config.timing.min_on_time_ms);
    // Normal: idle -> off, activity -> on. Inverted: idle -> on, activity -> off.
    let idle_state = config.watch.invert;
    let active_state = !idle_state;

    let mut last = diskstats::read_diskstats().unwrap_or_default();
    let mut displayed = idle_state;
    let mut activity_since: Option<Instant> = None;
    let mut state = LedState::Acquired(initial_led);
    if let LedState::Acquired(led) = &state {
        if let Err(e) = led.set_on(idle_state) {
            warn!("failed to set initial LED state: {e}");
        }
    }

    while SHOULD_EXIT.load(Ordering::SeqCst) == 0 {
        std::thread::sleep(poll_interval);

        if RESYNC_REQUESTED.swap(false, Ordering::SeqCst) {
            if let LedState::Acquired(led) = &state {
                if let Err(e) = led.reassert() {
                    warn!("post-resume resync failed: {e}");
                }
            }
        }

        // Try to come back from a vanished LED (unplug -> replug) before
        // evaluating activity for this tick.
        if let LedState::Missing { next_retry } = &state {
            if Instant::now() >= *next_retry {
                match resolve_led_path(config).and_then(Led::acquire) {
                    Ok(led) => {
                        info!(
                            "LED reappeared, resumed control of {}",
                            led.path().display()
                        );
                        if let Err(e) = led.set_on(idle_state) {
                            warn!("failed to set initial LED state after reacquire: {e}");
                        }
                        state = LedState::Acquired(led);
                        displayed = idle_state;
                        activity_since = None;
                    }
                    Err(_) => {
                        state = LedState::Missing {
                            next_retry: Instant::now() + REACQUIRE_RETRY_INTERVAL,
                        };
                    }
                }
            }
        }

        let current = match diskstats::read_diskstats() {
            Ok(c) => c,
            Err(e) => {
                warn!("failed to read /proc/diskstats, skipping this tick: {e}");
                continue;
            }
        };
        let activity = devices.iter().any(|dev| {
            let before = last.get(dev.as_str()).copied().unwrap_or_default();
            let after = current.get(dev.as_str()).copied().unwrap_or_default();
            match config.watch.count {
                CountMode::Read => after.sectors_read > before.sectors_read,
                CountMode::Write => after.sectors_written > before.sectors_written,
                CountMode::Both => {
                    after.sectors_read > before.sectors_read
                        || after.sectors_written > before.sectors_written
                }
            }
        });
        last = current;

        let LedState::Acquired(led) = &state else {
            continue;
        };

        // The debounce always protects the *activity-indicating* state
        // (active_state) so a single short burst is visible regardless of
        // whether that state is physically "on" or "off" under `invert`.
        let write_result = if activity {
            activity_since = Some(Instant::now());
            if displayed != active_state {
                Some(led.set_on(active_state))
            } else {
                None
            }
        } else if displayed == active_state
            && activity_since
                .map(|t| t.elapsed() >= min_on_time)
                .unwrap_or(true)
        {
            Some(led.set_on(idle_state))
        } else {
            None
        };

        match write_result {
            Some(Ok(())) => displayed = if activity { active_state } else { idle_state },
            Some(Err(Error::LedNodeMissing(_) | Error::LedNodeNotWritable { .. })) => {
                warn!("LED node vanished, will keep polling and resume when it returns");
                state = LedState::Missing {
                    next_retry: Instant::now() + REACQUIRE_RETRY_INTERVAL,
                };
                displayed = idle_state;
                activity_since = None;
            }
            Some(Err(e)) => warn!("LED write failed: {e}"),
            None => {}
        }
    }

    match state {
        LedState::Acquired(led) => Some(led),
        LedState::Missing { .. } => None,
    }
}
