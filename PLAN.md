# pc-control — Plan

A personal, low-memory Windows tool that replaces vendor peripheral software
(Logitech G HUB, HyperX NGENUITY, Kingston FURY CTRL) with a small Rust CLI + GUI.

Reviewed 2026-10-26 (scope reduction chosen). Decision numbers (1A, 2A, …) refer to
the review; security items are S1–S8. Deferred work lives in [TODOS.md](TODOS.md).

## Goals
- Switch lighting (RAM, GPU, motherboard) and mouse settings together via presets, without vendor software.
- **Primary constraint: minimal memory / CPU.** No Electron, no Tauri/webview, no async runtime.
- Budget: 0 MB of our own when closed; < 25 MB with the GUI open (measured, 15A).
  Accepted resident cost: OpenRGB service ~31 MB (see TODOS T1).
- Baseline being replaced: G HUB ~280 MB + NGENUITY helper ~85 MB.

## Non-goals
- HyperX Alloy FPS keyboard lighting (firmware/Fn-key only, no host protocol).
- Raw SMBus/PawnIO access from our code (risk of corrupting SPD; OpenRGB owns this).
- Our own lighting format (OpenRGB profiles are used instead, 1A).
- Cross-platform support; tray/hotkeys/headset/onboard flash writes in the MVP (see TODOS).

## Verified environment (2026-10-26)
| Item | Detail |
|---|---|
| OS / CPU / board | Windows 11 Pro, i7-12700, ASUS PRIME Z690-P D4 (Intel i801 SMBus) |
| RAM | 2x32 GB Kingston FURY Beast RGB DDR4-3200 (box KF432C16BB2A) |
| Mouse | Logitech PRO X Superlight 2 via LIGHTSPEED receiver `046d:c54d`, device PID `40A9` |
| Headset | HyperX Cloud III Wireless `03f0:05b7` |
| Keyboard | HyperX Alloy FPS `0951:16b7` (out of scope) |
| OpenRGB | 1.1 (`8e16de2`), LocalSystem service, SDK on `127.0.0.1:6742`, reachable without admin. Installer hash matches the official release (S1). |
| PawnIO | 2.2.0, driver signed by Microsoft WHCP, installer signed by namazso. RAM detected: 2 zones x 10 LEDs, SMBus 0x59/0x5B. GPU (1 LED) and ASUS board also exposed. |
| Proven | Own SDK client prototype set both RAM sticks to solid blue |
| Toolchain | Rust 1.99.1 stable MSVC, VS 2022 Build Tools |

## Step 0 — no code
1. ~~Save lighting profiles in the OpenRGB GUI; enable `service_startup_profile`~~ **Done 2026-10-28:** profile `default` saved by the service in `service_config\profiles`, startup profile enabled, log confirms it loads at service start.
2. ~~Set the mouse's onboard profile once in G HUB~~ **Done 2026-10-28:** mouse reports onboard mode. **Still to do:** uninstall G HUB (its SYSTEM updater service is still running).
3. ~~Check whether HeadsetControl supports `03f0:05b7`~~ **Done 2026-10-28:** HeadsetControl doesn't, but HyperHeadset (Rust, MIT) documents the protocol, and a read-only probe on this headset worked (battery, charging, sidetone, auto-shutdown). No USB capture needed; details in TODOS T2.

## Architecture

### Layout (2 packages)
```
pc-control/
├── Cargo.toml          workspace; Cargo.lock committed (S4)
├── deny.toml           cargo-deny: advisories, licenses, sources (S4)
├── pcctl/              library + console binary  → pcctl.exe          (4A)
│   ├── src/
│   │   ├── lib.rs
│   │   ├── openrgb.rs  sync std-only SDK client (5A codec, 2A, 6A, 17A)
│   │   ├── hidpp.rs    Logitech HID++ 2.0 over a Transport trait (3A, 10A)
│   │   ├── preset.rs   TOML preset: OpenRGB profile name + mouse settings
│   │   ├── report.rs   per-device outcome + exit code (3A, 7A)
│   │   └── main.rs     list / set / apply <preset>
│   └── tests/
│       ├── fixtures/   bytes captured from the real devices (serials stripped, S8)
│       └── *.rs        fake OpenRGB server, scripted HID transport
├── app/                Slint GUI binary → pc-control.exe (thin, 12A)
│   └── TESTING.md      manual visual checklist
└── scripts/
    └── measure-mem.ps1 peak + idle working set (15A)
```

No shared `Device` trait: the devices have no common shape. The only new abstraction is
`Transport` (`write` / `read_timeout`), which exists so HID++ logic can be tested (10A).

### Data flow
```
            ┌───────────────── pc-control.exe (Slint, software renderer) ─┐
            │                                                              │
pcctl.exe ──┤                    pcctl (lib)                               │
            │  preset.rs ──► openrgb.rs ──TCP 127.0.0.1:6742──► OpenRGB service (LocalSystem)
            │      │                                             ├─ PawnIO ─► SMBus ─► RAM x2
            │      │                                             ├─ NvAPI ──► GPU
            │      │                                             └─ HID ────► ASUS Aura
            │      └────► hidpp.rs ──Transport(hidapi)──► receiver 046d:c54d ─(slot)─► Superlight 2
            │                                                              │
            │  report.rs ◄── per-device results (ok │ skipped │ unverified │ error)
            └──────────────────────────────────────────────────────────────┘
```

### OpenRGB client (`openrgb.rs`)
```
connect ─► negotiate version (packet 40; require ≥ 3, else clear error)        2A
   │
   ▼
list controllers ─► decode Controller/Mode/Zone (version-gated codec)           5A
   │
   ▼
resolve target by type (DRAM/GPU/Motherboard) + location, never by index       6A
   │
   ▼
write: load profile (packets 150-152) │ UpdateMode │ UpdateLeds (one per device) 1A
   │
   ▼
verify: re-fetch that controller, compare mode + colors ─► mismatch = unverified 17A
   ▲
   └── packet 100 (device list changed) ─► invalidate cache, re-resolve          14A
```
- Profiles are listed/saved/loaded **only through the SDK**, never via files (needs no admin; the service owns storage).
- CLI fetches the controller list once per run; the GUI caches it and invalidates on packet 100.
- The client treats every server reply as untrusted (S3): max message size 1 MB, every length/count bounds-checked, no panics on malformed input.

### Mouse (`hidpp.rs`)
- Every run: open only receiver `046d:c54d` on the HID++ vendor interface (S7), ask which slot the mouse is in, ping it.
- "Asleep/unreachable" is its own outcome (`mouse: asleep, skipped`), never a silent success (3A).
- Features via HID++ root/feature lookup. **Verified on this mouse (2026-10-28):** Extended Adjustable DPI `0x2202` (index 9), Extended Report Rate `0x8061` (12), Onboard Profiles `0x8100` (13, currently in onboard mode), Unified Battery `0x1004` (6). It does **not** have `0x2201` or `0x8060`. Always look indexes up at runtime; never hard-code them.
- **Before any mouse code ships, run a spike (8A):** does a host-mode DPI change survive sleep/power-cycle on this mouse?
  - Survives → simple host-mode writes, apply-and-exit holds.
  - Reverts → choose between onboard-profile writes (TODOS T4) or a tiny watcher (TODOS T3). Not decided until the spike runs.

### Presets (`preset.rs`)
```toml
# %APPDATA%\pc-control\presets\gaming.toml
openrgb_profile = "gaming"
[mouse]
dpi = 1600
report_rate = 2000
```
Parsed with `deny_unknown_fields`; unknown OpenRGB profile names are reported, not ignored.

### GUI (`app/`)
- Slint with default features **off**: software renderer + Windows backend only, one embedded font (15A).
- Every action calls a `pcctl` lib function; no logic in the UI layer (12A).
- Color picker changes are coalesced: latest value wins, max ~20 updates/s per device, one `UpdateLeds` per device (13A).

### Errors and reporting
- One `thiserror` error type per module; each variant maps to an actionable message, e.g. connection refused → "OpenRGB service isn't running → `Start-Service OpenRGB` (admin)" (7A).
- Covered cases: service down, protocol too old, RAM missing (PawnIO broke), unknown profile, invalid preset, receiver busy (G HUB), mouse unpaired/asleep, HID++ error codes, write not verified.
- `pcctl` exit codes: `0` all devices ok, `2` partial, `1` nothing applied.

## Testing
Built-in `cargo test`, no mocking framework (9A).
- **Codec:** read → write → compare round-trips against fixture bytes captured from this PC (5A).
- **Fake OpenRGB server:** in-test `TcpListener` replaying fixtures and simulating every failure branch: refused, version < 3, RAM missing, duplicate match, unknown profile, packet 100 mid-apply, write not reflected (17A), malformed/oversized replies (S3).
- **HID++:** scripted fake `Transport` covering found / unpaired / asleep / timeout / busy / unsupported feature / error code (10A).
- **Presets:** valid, missing file, invalid TOML, unknown key, unknown profile.
- **Reporting:** exit code for all-ok / partial / none.
- **Hardware tests:** `#[ignore]` **and** `PCCTL_HW=1`; snapshot device state first and restore after; mouse flash writes are never run automatically (11A).
- **GUI:** manual checklist in `app/TESTING.md` (12A).

## Failure modes
| Code path | Failure | Test | Handling | Visible |
|---|---|---|---|---|
| connect / negotiate | service down, old version | fake server | 7A | clear message |
| resolve | RAM missing after Windows update | fake server | 7A | clear message |
| profile load | name typo | fake server | 7A | clear message |
| any OpenRGB write | SDK has no ACK; write dropped/ignored | fake server | 17A read-back | `unverified` |
| packet 100 mid-apply | indexes shift | fake server | 6A + 17A | `unverified` |
| hidpp set | mouse asleep | scripted transport | 3A | `skipped` |
| hidpp host mode | DPI reverts after sleep | 8A spike | 8A decision | resolved before ship |
| GUI throttle | final value dropped | manual + lib tests | 13A | visible mismatch |
| malicious/fake server on 6742 | oversized/malformed reply | fake server | S3 bounds checks | clear error, no crash |

## Performance
- Memory budget enforced by `scripts/measure-mem.ps1`, run at every milestone (15A).
- No tokio/async, no GPL crates (`openrgb2` rejected).
- No per-LED SDK calls; one `UpdateLeds` per device.
- Check the Kingston mode `flags` for a save-to-device capability as soon as the codec lands (16A → TODOS T1).

## Security
Threat model: a personal tool on one PC. The main risks are third-party code running as SYSTEM, supply-chain compromise at build time, and data leaking through captures or the public repo.

| # | Rule |
|---|---|
| S1 | Install OpenRGB/PawnIO only from official sources and verify (done 2026-10-26: OpenRGB MSI hash matches official; PawnIO signed). Re-verify on every update. |
| S2 | USB captures (headset work) record only the target device, never while typing passwords; deleted after use; `*.pcap*` is gitignored. |
| S3 | OpenRGB SDK is unauthenticated: never change `default_host` from `127.0.0.1`; our client treats replies as untrusted (size caps, bounds checks, fuzz tests). |
| S4 | `Cargo.lock` committed; minimal dependencies; `cargo deny check` (advisories, licenses incl. GPL, sources) before each milestone; verify crate names before adding. |
| S5 | Build from source. No prebuilt downloads until releases have checksums + artifact attestations (TODOS T7). |
| S6 | Our binaries never run elevated; autostart via Startup folder or non-elevated scheduled task. |
| S7 | Open only allow-listed HID devices/interfaces (`046d:c54d` HID++ vendor interface, later `03f0:05b7`); never implement firmware-update commands. |
| S8 | `.gitignore` for build output and captures; serials stripped from fixtures; repo/account hardening in TODOS T6. |

## Milestones
0. **Step 0** (no code, above).
1. **Workspace + lighting CLI — done 2026-10-28.** `openrgb.rs` + `openrgb/codec.rs` (protocol 3–5; the service speaks 6, see TODOS T9), `report.rs`, `preset.rs`, `pcctl list | profiles | set <ram|gpu|board> ... | apply <preset>`.
   - Tests: 43 automated (byte-exact round trips of fixtures captured from this PC, truncation and corruption fuzzing, fake server for every failure branch) + 2 opt-in hardware tests, both passing on the real RAM. Clippy clean with `-D warnings`.
   - Release `pcctl.exe`: 374 KB; peak working set 4.9 MB, peak private 0.84 MB (`scripts/measure-mem.ps1`).
   - 16A answered: Kingston Fury modes can't be saved to the sticks (GPU and board can). See TODOS T1.
   - Confirmed on hardware: loading a profile through the SDK re-applies it.
2. **Mouse:** 8A spike first, then `hidpp.rs` with discovery, DPI, report rate; scripted-transport tests.
3. **GUI:** Slint window: device list, color picker per zone (throttled), mode dropdown, preset apply; `measure-mem.ps1` within budget.
4. **Follow-ups** from TODOS.md, in order of value.

## Open questions
- Does a host-mode DPI change survive sleep on the Superlight 2? (8A spike)
- Slint royalty-free license: confirm it covers this personal use.
