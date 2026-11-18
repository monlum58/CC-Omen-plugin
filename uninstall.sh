#!/usr/bin/env bash
# Removes the plugin and its SELinux rule, hands the fans back to the firmware and restarts
# coolercontrold.   ./uninstall.sh
set -euo pipefail

ID=cc_omen
[ "$(id -u)" -eq 0 ] || exec sudo "$0" "$@"

PLUGINS="$(readlink -f /var/lib/coolercontrol/plugins 2>/dev/null || echo /var/lib/coolercontrol/plugins)"
DEST="$PLUGINS/$ID"

systemctl stop "cc-plugin-$ID" 2>/dev/null || true
pkill -x "$ID" 2>/dev/null || true

if command -v selinuxenabled >/dev/null && selinuxenabled; then
  semanage fcontext -d "$DEST/$ID" 2>/dev/null || true
fi
if [ -d "$DEST" ]; then
  find "$DEST" -mindepth 1 -delete
  rmdir "$DEST"
  echo "removed $DEST"
fi
SOCK=/run/coolercontrol-plugin-$ID.sock
[ -S "$SOCK" ] && unlink "$SOCK"

# Firmware auto on the hp fan device, found by name.
for d in /sys/class/hwmon/hwmon*; do
  if [ "$(cat "$d/name" 2>/dev/null)" = hp ] && [ -w "$d/pwm1_enable" ]; then
    echo 2 > "$d/pwm1_enable"
    echo "fans handed back to firmware auto ($d)"
  fi
done

systemctl restart coolercontrold
if pgrep -x "$ID" >/dev/null; then
  echo "warning: $ID is still running" >&2
  exit 1
fi
echo "uninstalled"
