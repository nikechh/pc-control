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
- **Result (2026-10-28, `pcctl list`):** none of the Kingston Fury modes has a save flag, so lighting **can't** be stored on the sticks. The GPU and ASUS board modes can. Remaining idea: the RAM controller keeps its state until power is cut, so a boot-time SYSTEM scheduled task could start the service, wait for `Loading service_startup_profile` in its log, then stop it. Before trying that, check whether sleep/hibernate resets the RAM LEDs (the service's `resume_profile` exists for that case). Until then the 31 MB service is the accepted cost.
- **Depends on / blocked by:** nothing (codec check done).

---

## T2. Headset support: HyperX Cloud III Wireless (`03f0:05b7`)

- **What:** Read battery, mute and charging status, and set sidetone and auto-off, without NGENUITY. Then uninstall NGENUITY.
- **Why:** The NGenuity2Helper process uses ~85 MB all the time just to expose a handful of headset settings.
- **Context (Step 0 result, 2026-10-28):** HeadsetControl does **not** support this model (its "HyperX Cloud 3" is the wired `03f0:089d`/`03cc`, sidetone on/off only). The protocol is already documented in **LennardKittner/HyperHeadset** (Rust, MIT), `src/devices/cloud_iii_wireless.rs`, so no USB capture is needed.
  - Packets: 62-byte output reports on **interface 3, usage page `0xFF13`**. Byte 0 = `0x66`, byte 1 = command, byte 2 = parameter. Replies start with `0x66` + the same command.
  - Get: wireless `0x82`, battery `0x89` (level in byte 4), charging `0x8A`, sidetone on `0x84`, sidetone volume `0x88`, auto-shutdown `0x85` (minutes), mute `0x86`, silent mode `0x87`.
  - Set: sidetone on `0x01`, auto-shutdown `0x02` (minutes), mute `0x03`, silent mode `0x04`, sidetone volume `0x05`.
  - **Verified on this headset** with a read-only probe: connected, battery 54%, not charging, sidetone off/0, auto-shutdown 20 min. Plain output reports worked; HaloBattery PR #53 notes some Windows dongles need a feature-report fallback, so keep one.
  - Set commands are not verified yet; test them with snapshot/restore (11A).
  Implement it as `pcctl/src/hyperx.rs` on the `Transport` trait, porting the command table (MIT: keep the attribution notice). Same tests as the mouse: scripted fake transport, with hardware tests opt-in (`PCCTL_HW=1`). If a capture is ever needed for an undocumented setting, follow S2: capture only the headset's device address, never type passwords while capturing, use the gitignored `captures/`, and delete afterwards.
- **Depends on / blocked by:** the `Transport` trait from 10A. (Step 0 check done.)

---

## T3. Tray icon and global hotkeys for presets

- **What:** An optional resident mode: a tray icon showing headset battery, and global hotkeys that apply presets (e.g. Ctrl+Alt+1 = "gaming").
- **Why:** Switching presets without opening the GUI or a terminal is the daily-use convenience that the vendor tools provide.
- **Context:** Both features need a process that keeps running, which conflicts with the "apply then exit, 0 MB when closed" design. Make it opt-in and keep it small (target < 3 MB, no Slint window loaded until opened; use the `tray-icon` crate and the Win32 `RegisterHotKey`). If the 8A mouse test shows that host-mode settings revert on sleep, this process should **also** be the mouse watcher that reapplies settings on reconnect, so there is one resident process rather than two. Measure it with `scripts/measure-mem.ps1` (15A).
- **Depends on / blocked by:** result of the 8A test, the GUI milestone, and T2 for battery display.

---

## T4. Write Superlight 2 onboard profiles (HID++ `0x8100`)

- **What:** Write DPI, report rate and button settings directly into the mouse's onboard profile memory, so they survive sleep and power cycles with no running process.
- **Why:** This is the only way to get persistent mouse settings with **zero** resident memory if the 8A test shows that host-mode changes (`0x2202` DPI, `0x8061` report rate; this mouse has no `0x2201`) are lost when the mouse sleeps.
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

---

## T9. Support OpenRGB SDK protocol 6

- **What:** Add protocol 6 to the `openrgb` client, alongside 3–5.
- **Why:** The installed OpenRGB 1.1 speaks protocol 6, which adds an `ACK` packet (10) for writes, `GET_ACTIVE_PROFILE` (156) and `PROFILE_LOADED` (158), and stable unique controller IDs in the packet header. Today `pcctl apply` can't confirm a profile load (it reports "OpenRGB can't confirm profile loads before SDK protocol 6"), and write confirmation relies on reading the controller back (17A).
- **Context:** Milestone 1 deliberately caps the client at protocol 5 (`codec::MAX_VERSION`), because OpenRGB's SDK documentation still marks protocol 6 as unreleased and its layout changes a lot: `mode_value` and `led_value` are removed, zones gain per-zone modes, controllers gain `display_name` and a configuration blob. Capture v6 fixtures with `cargo run -p pcctl --example capture_fixtures` after raising the version, and keep the round-trip tests byte-exact for both 5 and 6. The hardware test `loading_a_profile_through_the_sdk_changes_the_ram` confirmed that profile loads through the SDK do work today.
- **Depends on / blocked by:** nothing; nice to have before the GUI shows profile state.
