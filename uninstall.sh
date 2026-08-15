#!/bin/sh
# ioblink uninstaller.
#
#   curl -fsSL https://raw.githubusercontent.com/jensenbox/ioblink/main/uninstall.sh | bash
#
# Removes everything install.sh added: the running service, its systemd
# unit, the udev rule, the systemd-sleep resume hook, the binary, and the
# dedicated system user.
#
# Leaves /etc/ioblink/config.toml in place -- it's the one file with your
# hand-picked keyboard settings in it, and there's no good reason to
# destroy that just because the service is being removed. The path is
# printed at the end if you want it gone too.

set -eu

BINARY_PATH="/usr/local/bin/ioblink"
UDEV_RULE_PATH="/etc/udev/rules.d/99-ioblink.rules"
RESUME_HOOK_PATH="/usr/lib/systemd/system-sleep/ioblink"
UNIT_PATH="/etc/systemd/system/ioblink.service"
CONFIG_DIR="/etc/ioblink"
SERVICE_USER="ioblink"

log() { printf '\033[1m==>\033[0m %s\n' "$1"; }

if [ -f "$UNIT_PATH" ]; then
    log "Stopping and disabling ioblink.service"
    sudo systemctl disable --now ioblink.service 2>/dev/null || true
    log "Removing $UNIT_PATH"
    sudo rm -f "$UNIT_PATH"
    sudo systemctl daemon-reload
else
    log "ioblink.service not installed, skipping"
fi

if [ -f "$RESUME_HOOK_PATH" ]; then
    log "Removing $RESUME_HOOK_PATH"
    sudo rm -f "$RESUME_HOOK_PATH"
fi

if [ -f "$UDEV_RULE_PATH" ]; then
    log "Removing $UDEV_RULE_PATH"
    sudo rm -f "$UDEV_RULE_PATH"
    sudo udevadm control --reload-rules
    sudo udevadm trigger
fi

if [ -f "$BINARY_PATH" ]; then
    log "Removing $BINARY_PATH"
    sudo rm -f "$BINARY_PATH"
fi

if id "$SERVICE_USER" >/dev/null 2>&1; then
    log "Removing system user '$SERVICE_USER'"
    sudo userdel "$SERVICE_USER" 2>/dev/null || true
fi

cat <<EOF

Uninstalled.

$CONFIG_DIR was left in place (it only contains your keyboard settings,
nothing this script created that needs cleaning up on its own). Remove it
yourself if you're done with ioblink for good:

  sudo rm -rf $CONFIG_DIR

EOF
