# ioblink

Blink a keyboard LED in response to block device (NVMe/SATA) read and write
activity, as a system service.

## Why this exists

Plenty of desktops have no hard disk activity LED and no realistic path to
one in hardware: modern cases often drop the HDD LED header entirely, boards
that still expose a "Storage Device Activity LED" pin usually wire it to the
SATA controller (so it never fires for an NVMe-only build), and the kernel's
built-in `disk-activity` LED trigger is ATA-specific — the generic `blkdev`
trigger that would cover NVMe [stalled in review and never landed
upstream](https://lore.kernel.org/linux-leds/).

That leaves userspace: poll `/proc/diskstats`, drive an LED you already own.
Most keyboards expose unused `capslock`/`numlock`/`scrolllock` LED nodes in
`/sys/class/leds/`, and if the hardware has a lamp behind one of them, this
turns it into a disk activity light.

## How it works

- Polls `/proc/diskstats` on a timer (default 50ms), tracking sectors read
  and/or written per watched device.
- On any delta since the last poll, turns the LED on; keeps it on for at
  least a minimum on-time (default 30ms) after the last activity so a single
  short burst is actually visible instead of a sub-frame flicker.
- Detaches the kernel's own trigger from the LED on startup (`echo none >
  .../trigger`) so it can drive `brightness` directly, and restores the
  original trigger on clean shutdown so the key goes back to behaving
  normally.
- Resolves the LED by walking `/sys/class/leds` and matching the *backing
  USB device's* vendor/product ID rather than trusting a fixed `inputN::name`
  path — the `N` is enumeration order, not a stable identifier, and will
  change across reboots or a replug.
- Every write is read back and verified. See [the EVIOCGRAB
  gotcha](#the-eviocgrab-gotcha) below for why that matters.

## Commands

| Command | What it does |
|---|---|
| `ioblink --config <path>` | Run the poller (this is what the systemd unit invokes). Defaults to `/etc/ioblink/config.toml`. |
| `ioblink discover` | Interactively find which LED (if any) has a physical lamp, warn about grabbed devices, and print a config snippet. See below. |

## Before you install: find your LED

Not every `capslock`/`numlock`/`scrolllock` node the kernel exposes has a
physical lamp behind it — the kernel creates all three unconditionally
regardless of the hardware. You have to verify by eye, and it's easy to be
misled (see [the EVIOCGRAB gotcha](#the-eviocgrab-gotcha) below) into
thinking your keyboard has no lamp when the real problem is a grabby
background app.

```sh
cargo build --release
sudo ./target/release/ioblink discover
```

This walks every `capslock`/`numlock`/`scrolllock` node on the system,
checks each one for an active exclusive grab first (and tells you which
process to close if it finds one, instead of silently giving you a false
negative), then walks you through a blink test per candidate: press Enter
when you're watching the keyboard, it blinks 3 times, you answer y/N. At the
end it prints a ready-to-paste `[led.selector]` config block for whichever
node actually lit up, preferring Scroll Lock (nothing else uses it) over Num
Lock over Caps Lock.

If none of the candidate nodes light up, this approach doesn't work on your
hardware — the fallback is a dedicated USB indicator (e.g. a
[blink(1)](https://blink1.thingm.com/)), which is out of scope for this
project.

### The EVIOCGRAB gotcha

If a write to `brightness` reports success but the LED never lights **and**
the value reads back as unchanged, don't conclude the hardware has no lamp
yet. Check whether something holds an exclusive grab on the keyboard's event
device first:

```sh
sudo fuser -v /dev/input/eventN
```

Any userspace tool that reads global hotkeys or does input remapping by
grabbing a device exclusively (`EVIOCGRAB`) will cause the kernel's
`input_inject_event()` — which is what a direct `brightness` write on an
`inputN::*` LED goes through — to silently no-op. No error, no `dmesg`
output, the write just doesn't take effect. This cost real debugging time
building this tool; `ioblink` reads back every write it makes specifically
so this shows up as a loud, attributable error (`WriteVerifyMismatch`)
instead of looking like dead hardware.

## Installation

### Quick install

```sh
curl -fsSL https://raw.githubusercontent.com/jensenbox/ioblink/main/install.sh | bash
```

Prebuilt Linux binaries (x86_64 and aarch64) are also attached to each
[release](https://github.com/jensenbox/ioblink/releases) if you'd rather
skip the from-source build.

Builds from source (needs `cargo`/`git`; get Rust from [rustup.rs](https://rustup.rs)
if you don't have it) and installs the binary, the dedicated system user, the
systemd unit, the udev rule, and the suspend/resume hook. It does **not**
enable or start the service, and it never overwrites an existing
`/etc/ioblink/config.toml` or `/etc/udev/rules.d/99-ioblink.rules` — both
ship pointed at the author's keyboard as a worked example, not a default
that's correct for yours. The script's last lines tell you exactly what to
run next:

```sh
sudo ioblink discover                      # find your LED, get a config snippet
sudo $EDITOR /etc/ioblink/config.toml       # paste it in
sudo $EDITOR /etc/udev/rules.d/99-ioblink.rules   # same vendor/product here
sudo udevadm control --reload-rules && sudo udevadm trigger
sudo systemctl enable --now ioblink
```

Safe to re-run `install.sh` any time (e.g. to pick up an update) — it
rebuilds and reinstalls the binary/unit/hook but leaves your config and udev
rule alone.

### Uninstalling

```sh
curl -fsSL https://raw.githubusercontent.com/jensenbox/ioblink/main/uninstall.sh | bash
```

Stops and disables the service, and removes everything `install.sh` added
except `/etc/ioblink/config.toml` (it's the one file with your own settings
in it — the script prints the `rm -rf` command if you want it gone too).

### Manual install

Equivalent to what `install.sh` does, if you'd rather see every step or
you're packaging this for something else:

```sh
cargo build --release
sudo install -m 0755 target/release/ioblink /usr/local/bin/ioblink

sudo useradd --system --no-create-home --shell /usr/sbin/nologin ioblink

sudo mkdir -p /etc/ioblink
sudo install -m 0644 config/ioblink.toml /etc/ioblink/config.toml
# edit /etc/ioblink/config.toml: paste in the [led.selector] block that
# `ioblink discover` printed for you.

# edit systemd/99-ioblink.rules the same way (vendor/product), then:
sudo install -m 0644 systemd/99-ioblink.rules /etc/udev/rules.d/99-ioblink.rules
sudo udevadm control --reload-rules
sudo udevadm trigger

sudo install -m 0755 systemd/ioblink-resume.sh /usr/lib/systemd/system-sleep/ioblink

sudo install -m 0644 systemd/ioblink.service /etc/systemd/system/ioblink.service
sudo systemctl daemon-reload
sudo systemctl enable --now ioblink.service
```

### Verify

```sh
systemctl status ioblink.service
journalctl -u ioblink.service -f
dd if=/dev/nvme0n1 of=/dev/null bs=1M count=2000 iflag=direct   # LED should flicker
```

## Configuration

See [`config/ioblink.toml`](config/ioblink.toml) for the full annotated
example. Summary:

| Key | Default | Meaning |
|---|---|---|
| `led.selector.vendor` / `.product` | — | USB `idVendor`/`idProduct` of the keyboard (from `lsusb`) |
| `led.selector.name` | — | `capslock`, `numlock`, or `scrolllock` |
| `led.path` | — | Exact sysfs path instead of a selector. Not recommended — see above. |
| `watch.devices` | `[]` (autodetect) | Block devices to watch; empty autodetects every `nvme*n*` whole disk at startup |
| `watch.count` | `"both"` | `"read"`, `"write"`, or `"both"` |
| `watch.invert` | `false` | `false`: LED off at idle, on during activity. `true`: LED on at idle, off during activity. |
| `timing.poll_interval_ms` | `50` | Poll period. Lower = snappier, more CPU; higher = smoother but can miss short bursts |
| `timing.min_on_time_ms` | `30` | Minimum time the LED stays lit per burst, so it's visible at all |

`poll_interval_ms` is the tuning knob that decides whether this feels like a
real activity light or a lava lamp. 50ms is a starting point, not a
conclusion — tune it for your own eyes.

## Runtime behavior

- **Reboot**: the LED is re-resolved by USB vendor/product on every start,
  so `inputN` renumbering doesn't matter.
- **Unplug/replug**: a missing LED node degrades to "keep polling, retry
  once a second" rather than crashing or crash-looping. Once the node
  reappears, the trigger is re-detached and normal operation resumes with no
  manual restart.
- **Suspend/resume**: the bundled `systemd-sleep` hook sends `SIGHUP` to the
  service on resume, which re-verifies the trigger is still detached and
  re-detaches it if the kernel silently reattached one during the sleep
  cycle.
- **Stop**: `systemctl stop ioblink` restores the LED's original kernel
  trigger before exiting, so the key goes back to normal (Scroll/Num/Caps
  Lock indication) behavior.

## Contributing

Issues and PRs welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). This is a
small, focused tool; keep changes in that spirit.

## License

[MIT](LICENSE)
