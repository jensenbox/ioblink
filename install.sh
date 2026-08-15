#!/bin/sh
# ioblink installer.
#
#   curl -fsSL https://raw.githubusercontent.com/jensenbox/ioblink/main/install.sh | bash
#
# Builds from source, installs the binary/service/udev rule, but deliberately
# does NOT enable or start the service and does NOT touch an existing
# /etc/ioblink/config.toml or /etc/udev/rules.d/99-ioblink.rules -- both
# contain your keyboard's specific USB vendor/product ID, which this script
# has no way to know. Run `sudo ioblink discover` after installing to find
# yours.
#
# Safe to re-run: rebuilds and reinstalls the binary/unit/hook, but never
# overwrites a config or udev rule that's already there.

set -eu

REPO_URL="https://github.com/jensenbox/ioblink.git"
REF="${IOBLINK_REF:-main}"
BINARY_PATH="/usr/local/bin/ioblink"
CONFIG_DIR="/etc/ioblink"
CONFIG_PATH="$CONFIG_DIR/config.toml"
UDEV_RULE_PATH="/etc/udev/rules.d/99-ioblink.rules"
RESUME_HOOK_PATH="/usr/lib/systemd/system-sleep/ioblink"
UNIT_PATH="/etc/systemd/system/ioblink.service"
SERVICE_USER="ioblink"

log() { printf '\033[1m==>\033[0m %s\n' "$1"; }

if [ "$(uname -s)" != "Linux" ]; then
    echo "ioblink only runs on Linux (needs /sys/class/leds and /proc/diskstats)." >&2
    exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo not found. Install Rust first: https://rustup.rs" >&2
    exit 1
fi

if ! command -v git >/dev/null 2>&1; then
    echo "git not found. Install it first." >&2
    exit 1
fi

WORKDIR=$(mktemp -d)
trap 'rm -rf "$WORKDIR"' EXIT

log "Cloning jensenbox/ioblink@$REF"
git clone --quiet --depth 1 --branch "$REF" "$REPO_URL" "$WORKDIR/ioblink"

log "Building release binary (this takes a minute)"
(cd "$WORKDIR/ioblink" && cargo build --quiet --release)

log "Installing binary to $BINARY_PATH"
sudo install -m 0755 "$WORKDIR/ioblink/target/release/ioblink" "$BINARY_PATH"

if id "$SERVICE_USER" >/dev/null 2>&1; then
    log "Service user '$SERVICE_USER' already exists"
else
    log "Creating system user '$SERVICE_USER'"
    sudo useradd --system --no-create-home --shell /usr/sbin/nologin "$SERVICE_USER"
fi

sudo mkdir -p "$CONFIG_DIR"
if [ -f "$CONFIG_PATH" ]; then
    log "Leaving existing $CONFIG_PATH alone"
else
    log "Installing example config to $CONFIG_PATH (edit this before starting the service)"
    sudo install -m 0644 "$WORKDIR/ioblink/config/ioblink.toml" "$CONFIG_PATH"
fi

if [ -f "$UDEV_RULE_PATH" ]; then
    log "Leaving existing $UDEV_RULE_PATH alone"
else
    log "Installing udev rule to $UDEV_RULE_PATH (edit the vendor/product IDs before it takes effect for you)"
    sudo install -m 0644 "$WORKDIR/ioblink/systemd/99-ioblink.rules" "$UDEV_RULE_PATH"
fi
sudo udevadm control --reload-rules
sudo udevadm trigger

log "Installing systemd-sleep resume hook to $RESUME_HOOK_PATH"
sudo mkdir -p "$(dirname "$RESUME_HOOK_PATH")"
sudo install -m 0755 "$WORKDIR/ioblink/systemd/ioblink-resume.sh" "$RESUME_HOOK_PATH"

log "Installing systemd unit to $UNIT_PATH"
sudo install -m 0644 "$WORKDIR/ioblink/systemd/ioblink.service" "$UNIT_PATH"
sudo systemctl daemon-reload

cat <<EOF

Installed. Not started yet -- $CONFIG_PATH and $UDEV_RULE_PATH both need
your keyboard's actual USB vendor/product ID (they ship pointed at the
author's HP KBAR211 as a worked example, not a default that's correct for
your hardware).

Next steps:

  1. sudo ioblink discover
       Finds which LED (if any) actually has a physical lamp, and prints
       a [led.selector] block to paste into $CONFIG_PATH.

  2. Edit $UDEV_RULE_PATH: set ATTRS{idVendor}/ATTRS{idProduct} to match
     (same values \`discover\` just printed), then:
       sudo udevadm control --reload-rules && sudo udevadm trigger

  3. sudo systemctl enable --now ioblink

EOF
