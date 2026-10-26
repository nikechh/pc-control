# pc-control — Plan

A personal, low-memory Windows tool that replaces vendor peripheral software
(Logitech G HUB, HyperX NGENUITY, Kingston FURY CTRL) with one small Rust app.

## Goals
- Control RGB lighting of RAM (and, for free, GPU + motherboard), mouse settings, and headset settings from one app.
- **Primary constraint: minimal memory / CPU.** No Electron, no Tauri/webview.
- Target: 0 MB when closed, < 25 MB with GUI open. Baseline being replaced: G HUB ~280 MB + NGENUITY helper ~85 MB.

## Non-goals
- HyperX Alloy FPS keyboard lighting (firmware/Fn-key only, no host protocol).
- Cross-platform support (Windows 11 only for now).
- Distribution to other users.

## Verified environment (2026-10-26)
| Item | Detail |
|---|---|
| OS / CPU / board | Windows 11 Pro, i7-12700, ASUS PRIME Z690-P D4 (Intel i801 SMBus) |
| RAM | 2x32 GB Kingston FURY Beast RGB DDR4-3200 (box KF432C16BB2A) |
| Mouse | Logitech PRO X Superlight 2 via LIGHTSPEED receiver `046d:c54d`, device PID `40A9`, receiver slot 1 |
| Headset | HyperX Cloud III Wireless `03f0:05b7` |
| Keyboard | HyperX Alloy FPS `0951:16b7` (out of scope) |
| OpenRGB | v1.1.0, runs as LocalSystem service (~31 MB), SDK on `127.0.0.1:6742`, reachable without admin |
| PawnIO | 2.2.0 installed; OpenRGB detects "Kingston Fury DDR4 DRAM" (2 zones x 10 LEDs, SMBus 0x59/0x5B), plus GPU (1 LED) and ASUS board |
| Proven | Own SDK client (PowerShell prototype) set both RAM sticks to solid blue in Direct mode |
| Toolchain | Rust 1.99.1 stable MSVC, VS 2022 Build Tools |
| Running vendor software | G HUB (agent + tray + updater), NGenuity2Helper |

## Architecture
- **Language:** Rust, single Cargo workspace.
- **GUI:** Slint with software renderer (no GPU stack) for lowest RAM.
- **Model:** "apply settings to devices, then exit." Devices store settings themselves (mouse onboard profiles, RAM controller state, headset settings), so no resident process is needed. Optional tray mode later for headset battery.
- **RAM/GPU/board lighting:** talk to the existing OpenRGB service over its SDK using our own small, synchronous, std-only TCP client (no tokio, no GPL crate).
- **Mouse:** Logitech HID++ 2.0 over the receiver via the `hidapi` crate.
- **Headset:** HID commands to the Cloud III Wireless dongle via `hidapi`.
- **Profiles:** TOML files (e.g. `%APPDATA%\pc-control\profiles\gaming.toml`) describing the desired state of every device.

### Workspace layout
```
pc-control/
├── Cargo.toml              (workspace)
├── crates/
│   ├── core/               Device trait, Profile (serde + toml), errors
│   ├── openrgb-client/     sync SDK client: list controllers, modes, set LEDs, update mode
│   ├── hidpp/              Logitech HID++ 2.0: feature discovery, DPI, report rate, onboard profiles
│   └── hyperx/             Cloud III Wireless: status, sidetone, auto-off
├── pcctl/                  CLI: list / set / apply <profile>
└── app/                    Slint GUI
```

## Milestones
1. **Workspace + lighting (CLI).** Port the PowerShell SDK client to `openrgb-client`. `pcctl list`, `pcctl set ram <hex>`, `pcctl set ram --mode <name>`, `pcctl apply <profile>`. TOML profiles for RAM/GPU/board.
2. **GUI.** Slint window: device list, color picker per zone, mode dropdown, save/apply profile. Measure RSS against budget.
3. **Mouse.** HID++ feature discovery; DPI levels (0x2201), report rate (0x8060/0x8061), onboard profiles (0x8100). Requires quitting G HUB. References: Solaar, libratbag source.
4. **Headset.** Check whether HeadsetControl supports Cloud III Wireless; otherwise capture NGENUITY USB traffic (Wireshark + USBPcap) for sidetone / auto-off / battery. Then uninstall NGENUITY.
5. **Polish.** Autostart (`pcctl apply default` at login), global hotkeys for profiles, optional tray icon with headset battery, memory verification.

## Risks
- **OpenRGB service is still resident (~31 MB).** Possible mitigation: save mode to RAM controller if supported, then stop the service.
- **Vendor software conflicts:** G HUB / NGENUITY overwrite settings while running.
- **Undocumented protocols** (HID++ details for Superlight 2, Cloud III Wireless) may change with firmware updates.
- **SMBus safety:** only ever go through OpenRGB; never write raw SMBus ourselves (risk of corrupting SPD).
- **GPL:** porting code from OpenRGB/Solaar would make this GPL (acceptable for personal use).
- **Intel 12th-gen SMBus quirks** could cause intermittent RAM detection.

## Open questions
- Does the Kingston Fury controller support persisting a mode to the stick (MODE_FLAG_MANUAL_SAVE)?
- Does HeadsetControl already support `03f0:05b7`?
- Does the Superlight 2 accept onboard-profile writes when G HUB is uninstalled vs. just closed?
- Is Slint's royalty-free license fine (personal use → yes, confirm)?
