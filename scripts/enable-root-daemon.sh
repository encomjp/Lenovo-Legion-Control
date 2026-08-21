#!/bin/bash
# One-shot: install root daemon so fans/profile/battery writes work.
set -e
cd "$(dirname "$0")/.."
cargo build --release
sudo install -Dm755 target/release/legion-daemon /usr/local/bin/legion-daemon
sudo install -Dm755 target/release/legion-cli /usr/bin/legion-cli
sudo install -Dm755 target/release/legion-settings /usr/bin/legion-settings
sudo install -Dm644 data/systemd/legion-control.system.service /etc/systemd/system/legion-control.service
sudo install -Dm644 data/udev/99-legion.rules /etc/udev/rules.d/99-legion.rules
sudo udevadm control --reload-rules
sudo udevadm trigger -s hidraw || true
# stop any stray user daemon
systemctl --user disable --now legion-control 2>/dev/null || true
pkill -x legion-daemon 2>/dev/null || true
sudo systemctl daemon-reload
sudo systemctl enable --now legion-control
sleep 0.5
systemctl --no-pager --full status legion-control || true
echo
echo "Test:"
legion-cli set-profile balanced && legion-cli profile
legion-cli fan-auto
legion-cli rgb 200 16 46
echo "GUI: legion-settings"
