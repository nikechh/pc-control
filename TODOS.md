# TODOS

Deferred work from the plan review (2026-10-26). None of these are in the MVP
(MVP = Step 0 + pcctl/OpenRGB + mouse spike/module + GUI).

---

## T1. Persist RAM lighting on the DIMMs and stop the OpenRGB service

- **What:** Save the chosen lighting mode into the Kingston FURY controllers so the OpenRGB service no longer has to run all the time.
- **Why:** Once G HUB and NGENUITY are uninstalled, the OpenRGB service (~31 MB working set, LocalSystem) is the largest always-running process left. The project's main goal is minimal resident memory.
- **Context:** OpenRGB reports per-mode `flags`; a manual-save flag (`MODE_FLAG_MANUAL_SAVE`) means the device supports SDK `SaveMode` (packet 1102). The 5A codec already parses `flags`. Check them on the Kingston Fury DDR4 controller (zones "Fury Slot 1/2", SMBus 0x59/0x5B). If saving is supported:
  1. Save the mode to the sticks.
  2. Confirm it survives a cold boot (full power-off, not just a restart).
  3. Switch the `OpenRGB` service to Manual start.
  4. Decide how `pcctl` starts it when needed. Starting a service needs admin unless its permissions are changed with `sc sdset`, which weakens security, so that needs a deliberate decision.
  If saving is not supported, write down that the 31 MB is the accepted cost.
- **Depends on / blocked by:** the 5A codec (Milestone 1). It's a cheap check right after the codec lands.

---

## T2. Headset support: HyperX Cloud III Wireless (`03f0:05b7`)

- **What:** Read battery, mute and charging status, and set sidetone and auto-off, without NGENUITY. Then uninstall NGENUITY.
- **Why:** The NGenuity2Helper process uses ~85 MB all the time just to expose a handful of headset settings.
- **Context:** No public reverse-engineering was found for this exact model. The Cloud Flight projects (srn/hyperx-cloud-flight-wireless, flightdeck) target a different device (`0951:16c4`/`0951:1723`) and don't apply. Steps:
  1. Check whether HeadsetControl (Sapd/HeadsetControl, C, supports Windows) lists `03f0:05b7`. If it does, call it or port its commands.
  2. If not, record NGENUITY's USB traffic with Wireshark + USBPcap while changing each setting, then replay the reports through the existing `Transport`/hidapi path.
  Implement it as `pcctl/src/hyperx.rs` with the same tests as the mouse: scripted fake transport, with hardware tests opt-in (`PCCTL_HW=1`).
- **Depends on / blocked by:** HeadsetControl check (Step 0). The `Transport` trait from 10A.

---

## T3. Tray icon and global hotkeys for presets

- **What:** An optional resident mode: a tray icon showing headset battery, and global hotkeys that apply presets (e.g. Ctrl+Alt+1 = "gaming").
- **Why:** Switching presets without opening the GUI or a terminal is the daily-use convenience that the vendor tools provide.
- **Context:** Both features need a process that keeps running, which conflicts with the "apply then exit, 0 MB when closed" design. Make it opt-in and keep it small (target < 3 MB, no Slint window loaded until opened; use the `tray-icon` crate and the Win32 `RegisterHotKey`). If the 8A mouse test shows that host-mode settings revert on sleep, this process should **also** be the mouse watcher that reapplies settings on reconnect, so there is one resident process rather than two. Measure it with `scripts/measure-mem.ps1` (15A).
- **Depends on / blocked by:** result of the 8A test, the GUI milestone, and T2 for battery display.

---

## T4. Write Superlight 2 onboard profiles (HID++ `0x8100`)

- **What:** Write DPI, report rate and button settings directly into the mouse's onboard profile memory, so they survive sleep and power cycles with no running process.
- **Why:** This is the only way to get persistent mouse settings with **zero** resident memory if the 8A test shows that host-mode changes (`0x2201` DPI, `0x8061` report rate) are lost when the mouse sleeps.
- **Context:** This is the riskiest code in the project: memory-page reads and writes, per-sector checksums, and flash wear. Required safeguards:
  1. Read and back up the full onboard profile area to a file before the first write.
  2. Provide `pcctl mouse restore-backup`.
  3. Verify after each write by reading back and comparing checksums.
  4. Never run it from automated tests (11A).
  References: Solaar and libratbag source (GPL; read them, don't copy). Mouse PID `40A9` via receiver `046d:c54d`. The device must not be in use by G HUB.
- **Depends on / blocked by:** the 8A test result (only needed if host mode is volatile **and** T3's watcher isn't wanted), plus the HID++ base module (Milestone 3).
