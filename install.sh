#!/usr/bin/env bash
# Builds the plugin (as you) and installs it into CoolerControl's plugin folder (with sudo).
#   ./install.sh
# Writes only the plugin folder and one SELinux file-context rule, then restarts coolercontrold.
set -euo pipefail

ID=cc_omen
HERE="$(cd "$(dirname "$0")" && pwd)"
BIN="$HERE/target/release/$ID"

if [ "$(id -u)" -ne 0 ]; then
  (cd "$HERE" && cargo build --release --locked)
  exec sudo "$0" "$@"
fi

[ -x "$BIN" ] || { echo "no $BIN: run ./install.sh as your user so it can build first" >&2; exit 1; }
command -v coolercontrold >/dev/null || { echo "coolercontrold is not installed" >&2; exit 1; }

PLUGINS="$(readlink -f /var/lib/coolercontrol/plugins 2>/dev/null || echo /var/lib/coolercontrol/plugins)"
DEST="$PLUGINS/$ID"

install -d -m 755 "$DEST"
install -m 755 "$BIN" "$DEST/$ID"
install -m 644 "$HERE/manifest.toml" "$DEST/manifest.toml"
echo "installed $DEST"

# systemd may only execute bin_t files. semanage makes the label survive a relabel;
# restorecon applies it now. SELinux itself is never switched off.
if command -v selinuxenabled >/dev/null && selinuxenabled; then
  semanage fcontext -a -t bin_t "$DEST/$ID" 2>/dev/null || semanage fcontext -m -t bin_t "$DEST/$ID"
  restorecon -v "$DEST/$ID"
fi

systemctl restart coolercontrold
echo "restarted coolercontrold; waiting for the plugin to start"
for _ in $(seq 1 30); do
  if journalctl -u "cc-plugin-$ID" --since "-60s" --no-pager -q 2>/dev/null | grep -q "firmware auto set at start"; then
    echo "plugin running. Next: in CoolerControl, set the stock 'hp' device to Default Profile (or disable it)"
    echo "and assign your curves to 'HP OMEN fans' > CPU fan / GPU fan."
    exit 0
  fi
  sleep 1
done
echo "the plugin did not report a start within 30 s; see: journalctl -u cc-plugin-$ID -b" >&2
exit 1
