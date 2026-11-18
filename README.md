# CC-Omen-plugin

A [CoolerControl](https://gitlab.com/coolercontrol/coolercontrol) device plugin that lets fan curves **stop** the fans of the HP OMEN 16 (board 8BCA) on Linux.

> **Status: planning.** Nothing here is built yet. The plan is [`ISA.md`](ISA.md) and the issues.

## The problem

Since Linux 7.3 the in-tree `hp-wmi` driver gives the OMEN 16 separate CPU and GPU fan control (`pwm1`, `pwm2`, shared `pwm1_enable`). In manual mode the driver clamps every speed to the lowest step of the BIOS fan table (about 2000 RPM), so CoolerControl can never stop the fans. Firmware auto mode does stop them when the machine is cool, but it reacts too late under load.

## The idea

The plugin shows CoolerControl one device with two channels, *CPU fan* and *GPU fan*:

| Curves ask for | The plugin sets | Result |
|---|---|---|
| 0 % on both channels | `pwm1_enable = 2` (firmware auto) | firmware stops the fans while cool |
| more than 0 % on either | `pwm1_enable = 1` and both speeds | your curve, from the driver's floor upwards |

If anything goes wrong (CoolerControl stops, the plugin dies or goes silent), the fans go back to firmware auto. Above an emergency CPU temperature they run at full speed.

No kernel module is involved, so it works with Secure Boot and survives kernel updates.

## Requirements

- HP OMEN 16 with board 8BCA (others with the same `hp` hwmon interface may work)
- Linux 7.3 or newer with the in-tree `hp-wmi` driver
- CoolerControl 5.1 or newer
- "Fan Always On" disabled in the BIOS

## Licence

GPL-3.0, following CoolerControl's plugin template.
