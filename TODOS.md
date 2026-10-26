# TODOS

Deferred work from the plan review (2026-10-26). None of these are in the MVP
(MVP = Step 0 + lighting CLI + mouse spike/module + GUI; see PLAN.md milestones).
T5–T8 come from the security review (PLAN.md, section "Security").

---

## T1. Persist RAM lighting on the DIMMs and stop the OpenRGB service

- **What:** Save the chosen lighting mode into the Kingston FURY controllers so the OpenRGB service no longer has to run all the time.
- **Why:** Once G HUB and NGENUITY are uninstalled, the OpenRGB service (~31 MB working set, LocalSystem) is the largest always-running process left. The project's main goal is minimal resident memory.
- **Context:** OpenRGB reports per-mode `flags`; a manual-save flag (`MODE_FLAG_MANUAL_SAVE`) means the device supports SDK `SaveMode` (packet 1102). The 5A codec already parses `flags`. Check them on the Kingston Fury DDR4 controller (zones "Fury Slot 1/2", SMBus 0x59/0x5B). If saving is supported:
  1. Save the mode to the sticks.
  2. Confirm it survives a cold boot (full power-off, not just a restart).
  3. Switch the `OpenRGB` service to Manual start.
  4. Decide how `pcctl` starts it when needed. Starting a service needs admin unless its permissions are changed with `sc sdset`. If that route is taken, grant **start-only** rights to **your user SID only**, never to Everyone/Users, and never stop/config rights (S6): anything more lets any program control a SYSTEM service.
  If saving is not supported, write down that the 31 MB is the accepted cost.
- **Depends on / blocked by:** the 5A codec (Milestone 1). It's a cheap check right after the codec lands.

---

## T2. Headset support: HyperX Cloud III Wireless (`03f0:05b7`)

- **What:** Read battery, mute and charging status, and set sidetone and auto-off, without NGENUITY. Then uninstall NGENUITY.
- **Why:** The NGenuity2Helper process uses ~85 MB all the time just to expose a handful of headset settings.
- **Context:** No public reverse-engineering was found for this exact model. The Cloud Flight projects (srn/hyperx-cloud-flight-wireless, flightdeck) target a different device (`0951:16c4`/`0951:1723`) and don't apply. Steps:
  1. Check whether HeadsetControl (Sapd/HeadsetControl, C, supports Windows) lists `03f0:05b7`. If it does, call it or port its commands.
  2. If not, record NGENUITY's USB traffic with Wireshark + USBPcap while changing each setting, then replay the reports through the existing `Transport`/hidapi path.
     **Capture safety (S2):** USBPcap records every device on the same root hub, including the keyboard. Capture only the headset's device address (or move the headset to a root hub without the keyboard), don't type passwords while capturing, save captures under the gitignored `captures/`, and delete them once the commands are documented.
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
- **Depends on / blocked by:** the 8A test result (only needed if host mode is volatile **and** T3's watcher isn't wanted), plus the HID++ base module (Milestone 2).

---

## T5. Firewall rule blocking inbound TCP 6742

- **What:** Add a Windows Firewall rule that blocks inbound connections to TCP port 6742 on all profiles.
- **Why:** The OpenRGB SDK has no authentication and runs as SYSTEM (S3). Today it only listens on `127.0.0.1`, so it's not reachable from the network, but one config change (`default_host` → `0.0.0.0`) or a future default change would expose it to the LAN. The rule is a backstop.
- **Context:** Verified 2026-10-26: listener bound to `127.0.0.1` only, no firewall rules mention OpenRGB. Run in an admin terminal:
  `New-NetFirewallRule -DisplayName "Block OpenRGB SDK inbound" -Direction Inbound -Protocol TCP -LocalPort 6742 -Action Block`
  Loopback traffic isn't filtered by this rule, so `pcctl` keeps working.
- **Depends on / blocked by:** nothing; a one-time manual step.

---

## T6. GitHub account and repo hardening

- **What:** Turn on 2FA for the account; on `nikechh/pc-control` enable secret scanning with push protection, Dependabot alerts, and branch protection for `main` (no force-push, no deletion).
- **Why:** The repo is public, and later CI/releases (T7) will produce binaries meant to run on this PC. A compromised account or a force-pushed `main` is a direct route to running attacker code.
- **Context:** Commits use the author email nikitakirichevskiy@gmail.com, which is public in the history (chosen deliberately). Settings live under the account's Settings → Password and authentication, and the repo's Settings → Code security / Branches.
- **Depends on / blocked by:** nothing; do before T7.

---

## T7. CI, then releases with checksums and artifact attestations

- **What:** A GitHub Actions workflow running `cargo test`, `cargo clippy` and `cargo deny check` on every push. Later, a release workflow that builds `pcctl.exe` / `pc-control.exe`, publishes SHA256 checksums, and creates GitHub artifact attestations.
- **Why:** S4 and S5. CI catches vulnerable or wrongly-licensed dependencies automatically, and attestations let you prove a downloaded binary was built from this repo by this workflow before running it.
- **Context:** Until this exists, the rule is to build from source only (S5). Verify a release with `gh attestation verify <file> --repo nikechh/pc-control`. Pin third-party actions to full commit SHAs, give workflows minimal `permissions:`, and never run release jobs on pull requests from forks.
- **Depends on / blocked by:** Milestone 1 (there must be code and a `deny.toml`), T6.

---

## T8. Keep OpenRGB and PawnIO updated and re-verify each update

- **What:** Check for OpenRGB and PawnIO updates periodically; verify every new installer before running it.
- **Why:** Both run with the highest privileges on the machine (SYSTEM service and kernel driver). OpenRGB isn't code-signed and its SDK parser accepts input from any local process, so bug fixes there are security fixes.
- **Context:** Verified 2026-10-26: OpenRGB 1.1 MSI SHA256 `A4CE48A5E1532750AD4F5248424D42F7F2C710775CEE88DB070CCDB63C549CAA` matches the official download from openrgb.org (which links to codeberg.org/OpenRGB releases; the site publishes no checksums, so compare against a fresh download from that link). PawnIO installer signed by namazso.eu, driver signed by Microsoft WHCP. For updates: download only via openrgb.org / pawnio.eu, check `Get-AuthenticodeSignature` for PawnIO, and compare OpenRGB hashes against a second official download.
- **Depends on / blocked by:** nothing; recurring.
