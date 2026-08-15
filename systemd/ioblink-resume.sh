#!/bin/sh
# systemd-sleep hook. Install to /usr/lib/systemd/system-sleep/ioblink
# (executable, no extension) and systemd-logind will call it automatically
# around every suspend/hibernate cycle as "<script> pre|post suspend|...".
#
# On resume we ask the running service to re-verify its LED trigger is
# still detached rather than trusting it came back cleanly on its own --
# the ticket that spawned this project called out LED state after resume
# as "the thing most likely to be wrong", and this is cheap insurance
# against it. No-op if the service isn't running.
#
# Install: sudo install -m 0755 ioblink-resume.sh /usr/lib/systemd/system-sleep/ioblink

set -eu

case "${1:-}" in
    post)
        systemctl kill -s HUP ioblink.service 2>/dev/null || true
        ;;
esac
