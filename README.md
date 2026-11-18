# CC-Omen-plugin

A [CoolerControl](https://gitlab.com/coolercontrol/coolercontrol) device plugin that lets fan curves **stop** the fans of the HP OMEN 16 (board 8BCA) on Linux, and still start them as early as your curves say.

## The problem

Since Linux 7.3 the in-tree `hp-wmi` driver gives the OMEN 16 separate CPU and GPU fan control (`pwm1`, `pwm2`) with one shared mode switch (`pwm1_enable`: 0 full speed, 1 manual, 2 firmware auto). In manual mode the driver clamps every speed to the lowest step of the BIOS fan table, about 2000 RPM, so CoolerControl can never stop the fans. Firmware auto mode does stop them when the machine is cool, but it reacts too late under load.

## What the plugin does

It shows CoolerControl one device, **HP OMEN fans**, with two channels, **CPU fan** and **GPU fan**:

| Your curves ask for | The plugin sets | Result |
|---|---|---|
| 0 % on both channels | firmware auto (`pwm1_enable = 2`) | the firmware stops the fans while the machine is cool |
| more than 0 % on either | manual (`pwm1_enable = 1`) and both speeds | your curves, from the driver's floor upwards |

- Going back from manual to firmware auto waits until both channels have been at 0 % for 30 seconds, so the fans don't flap at the threshold.
- Because the mode switch is shared, a channel at 0 % runs at the driver floor (about 2000 RPM) while the other channel is above 0 %.
- No kernel module is involved, so it works with Secure Boot and survives kernel updates.

### Safety

- On every normal way out (CoolerControl stops the plugin, SIGTERM/SIGINT, a panic in any thread) the fans go back to firmware auto.
- **Watchdog:** if CoolerControl sends nothing for 15 s while the fans are manual, the plugin hands them back to firmware auto.
- **Emergency:** at 95 °C CPU the fans run at full speed until the CPU is down to 85 °C, whatever CoolerControl asks. If the CPU temperature cannot be read three times in a row, the fans also run at full speed.
- On start the plugin always writes firmware auto first, so a crashed earlier instance never leaves the fans in a stale state.
- `SIGKILL` (`kill -9`) cannot be caught by any program. After one, the driver keeps the last manual speed until CoolerControl starts the plugin again, which then writes firmware auto first.
- It refuses to start without the `hp` fan interface, without a CPU temperature sensor (k10temp or coretemp), or on a board other than 8BCA (override: `--allow-any-board` in the manifest's `args`).

## Requirements

- HP OMEN 16 with board 8BCA (`cat /sys/class/dmi/id/board_name`)
- Linux 7.3 or newer with the in-tree `hp-wmi` driver
- CoolerControl 5.1 or newer
- "Fan Always On" disabled in the BIOS (otherwise the firmware never stops the fans)
- To build: Rust 1.86+, `protoc`

## Install

```bash
./install.sh
```

It builds as your user, then uses `sudo` to copy the binary and `manifest.toml` to `/var/lib/coolercontrol/plugins/cc_omen/`, adds the SELinux `bin_t` label (with `semanage fcontext` and `restorecon`, so it survives a relabel), and restarts `coolercontrold`.

## Set up CoolerControl

1. **Disable the stock `hp` device** in CoolerControl (in its device settings, like any other sensor you hide). Only the plugin may write to the fans; if something else changes the mode, the plugin restores it and logs a warning.
2. Assign your curves to **HP OMEN fans → CPU fan** and **GPU fan**. Use 0 % below the temperature where the fans should stop, for example a CPU curve that stays at 0 % up to 50 °C.
3. Calibration is optional. Uncalibrated, a channel shows the driver's pwm as a percent, so the floor of about 2000 RPM reads as 33 %, the same as other system monitors. Curve values between 1 % and 33 % all run at that floor.

## Settings

Optional `config.json` next to the binary (`/var/lib/coolercontrol/plugins/cc_omen/config.json`), owned by root and not writable by group or others. Otherwise it is ignored:

```json
{ "dwell_seconds": 30, "watchdog_seconds": 15, "emergency_celsius": 95, "emergency_hysteresis_celsius": 10 }
```

| Key | Default | Allowed |
|---|---|---|
| `dwell_seconds` | 30 | 0–600 |
| `watchdog_seconds` | 15 | 3–60 |
| `emergency_celsius` | 95 | 70–100 |
| `emergency_hysteresis_celsius` | 10 | 3–20 |

Values outside the range are clamped. Unknown keys or broken JSON fall back to all defaults. Each case is logged. Restart `coolercontrold` after a change.

## Logs

```bash
journalctl -u cc-plugin-cc_omen -b
```

Set CoolerControl's log level to debug to see every request and write.

## Uninstall

```bash
./uninstall.sh
```

Removes the plugin folder and the SELinux rule, hands the fans back to firmware auto, and restarts `coolercontrold`.

## Development

```bash
cargo test                 # unit, property and process tests against a fake sysfs
bun scripts/mutate.ts      # breaks each safety guard and requires a test to catch it
bun scripts/privacy-scan.ts
```

`git config core.hooksPath .githooks` enables the pre-commit hook that raises the build number in `version.properties` on every code change.

## Licence

GPL-3.0-or-later, following CoolerControl's plugin template.
