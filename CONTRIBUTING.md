# Contributing

Pull requests are welcome — bug fixes, additional LED-selector matching
strategies, support for other block device families, whatever's useful.

## Before opening a PR

- `cargo fmt`
- `cargo clippy --all-targets -- -D warnings`
- `cargo test`

Most of this project only makes sense to test against real hardware
(sysfs LEDs, real block devices), so unit tests focus on pure logic (device
name parsing, diskstats parsing). If you're changing behavior that can only
be verified on real hardware, say what you tested it against in the PR
description.

## Scope

This is intentionally small: a poll loop, a sysfs LED abstraction, and
systemd packaging. If a change adds a dependency or a new subsystem,
explain why the simpler option doesn't work in the PR description.

## Reporting a bug

Include your kernel version, the keyboard's `lsusb` line, the contents of
`/sys/class/leds/<node>/trigger`, and `journalctl -u ioblink` output around
the failure.
