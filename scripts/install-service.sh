#!/usr/bin/env bash
#
# Install fawnd as a systemd --user service that starts at login.
#
# Builds release binaries, installs them to ~/.local/bin, installs a udev rule
# for hidraw access (needs sudo), and enables the fawnd-daemon user service.
#
# Re-runnable: rebuilds and restarts the service with the latest binary.

set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="${HOME}/.local/bin"
unit_dir="${HOME}/.config/systemd/user"
udev_rule="/etc/udev/rules.d/99-drunkdeer.rules"

echo "==> Building daemon + CLI"
cargo build --release --manifest-path "${repo_root}/Cargo.toml" \
    --bin fawnd-daemon --bin fawnd

echo "==> Installing binaries to ${bin_dir}"
mkdir -p "${bin_dir}"
for bin in fawnd-daemon fawnd; do
    install -m 0755 "${repo_root}/target/release/${bin}" "${bin_dir}/${bin}"
done

# The GUI needs desktop libraries; build it best-effort so a headless box can
# still install the service.
echo "==> Building GUI (optional)"
if cargo build --release --manifest-path "${repo_root}/Cargo.toml" --bin fawnd-gui; then
    install -m 0755 "${repo_root}/target/release/fawnd-gui" "${bin_dir}/fawnd-gui"
else
    echo "!! GUI build failed (missing desktop libraries?); skipping fawnd-gui."
fi

# The daemon runs as your user, so the hidraw node must be user-accessible.
if [ -e "${udev_rule}" ]; then
    echo "==> udev rule already present (${udev_rule})"
elif command -v sudo >/dev/null 2>&1; then
    echo "==> Installing udev rule for DrunkDeer hidraw access (needs sudo)"
    printf 'SUBSYSTEM=="hidraw", ATTRS{idVendor}=="352d", MODE="0660", TAG+="uaccess"\n' \
        | sudo tee "${udev_rule}" >/dev/null
    sudo udevadm control --reload-rules
    sudo udevadm trigger
    echo "    If the keyboard was already connected, re-plug it to pick up the rule."
else
    echo "!! sudo not found; skipping udev rule."
    echo "   Without it the daemon may lack permission for the keyboard."
    echo "   Create ${udev_rule} manually (see README) then reload udev."
fi

echo "==> Installing systemd user unit"
mkdir -p "${unit_dir}"
install -m 0644 "${repo_root}/packaging/fawnd.service" "${unit_dir}/fawnd.service"

echo "==> Enabling and (re)starting the service"
systemctl --user daemon-reload
systemctl --user enable fawnd.service
systemctl --user restart fawnd.service

echo
echo "Installed. fawnd-daemon now starts automatically at login."
echo
echo "  Status:  systemctl --user status fawnd"
echo "  Logs:    journalctl --user -u fawnd -f"
echo "  Stop:    systemctl --user disable --now fawnd"
echo
echo "Auto profile switching: create ~/.config/fawnd/rules.toml (see rules.example.toml)"
echo "and put profiles in ~/.config/fawnd/profiles/."
