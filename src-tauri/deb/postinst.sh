#!/bin/sh
# Runs after apt installs or upgrades puru-nucleus. Never fails the install.

# Upgrade from the pre-rename "puru-dc" package (≤ 0.12.6): keep its config
# and licence, and swap its systemd unit (it points at the removed
# /usr/bin/puru-dc) for the puru-nucleus one.
if [ -d /etc/puru-dc ] && [ ! -e /etc/puru-nucleus ]; then
    mv /etc/puru-dc /etc/puru-nucleus || true
fi
if [ -f /etc/systemd/system/puru-dc.service ]; then
    systemctl disable --now puru-dc >/dev/null 2>&1 || true
    rm -f /etc/systemd/system/puru-dc.service
    /usr/bin/puru-nucleus service install >/dev/null 2>&1 || true
    systemctl start puru-nucleus >/dev/null 2>&1 || true
fi

# Pick up the new binary if the daemon was already running.
systemctl try-restart puru-nucleus >/dev/null 2>&1 || true
exit 0
