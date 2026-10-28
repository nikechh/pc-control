//! Records raw OpenRGB SDK replies from the running service as test fixtures.
//! Read-only: it only requests data, it never changes lighting.
//!
//!     cargo run -p pcctl --example capture_fixtures -- pcctl/tests/fixtures
//!
//! Serial numbers are overwritten with 'X' (same length, so the byte layout is unchanged).

use std::collections::HashMap;
use std::path::PathBuf;

use pcctl::openrgb::{Client, Controller, DEFAULT_ADDR};

fn mask_serial(raw: &mut [u8], serial: &str) {
    if serial.is_empty() {
        return;
    }
    let mut needle = (serial.len() as u16 + 1).to_le_bytes().to_vec();
    needle.extend_from_slice(serial.as_bytes());
    needle.push(0);
    if let Some(pos) = raw.windows(needle.len()).position(|w| w == needle.as_slice()) {
        raw[pos + 2..pos + 2 + serial.len()].fill(b'X');
    }
}

fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("usage: capture_fixtures <output dir>"));
    std::fs::create_dir_all(&dir).expect("create output dir");
    let mut client = Client::connect(DEFAULT_ADDR).expect("connect to OpenRGB");
    let version = client.version();
    let mut seen: HashMap<String, usize> = HashMap::new();

    for index in 0..client.controller_count().expect("controller count") {
        let mut raw = client.controller_raw(index).expect("controller data");
        let controller = Controller::decode(&raw, version).expect("decode");
        mask_serial(&mut raw, &controller.serial);
        let label = controller.device_type.label().to_ascii_lowercase().replace(' ', "_");
        let n = seen.entry(label.clone()).or_default();
        let name = if *n == 0 { format!("v{version}_{label}.bin") } else { format!("v{version}_{label}_{n}.bin") };
        *n += 1;
        std::fs::write(dir.join(&name), &raw).expect("write fixture");
        println!("{name}: {} ({} bytes)", controller.name, raw.len());
    }

    let profiles = client.profiles_raw().expect("profile list");
    std::fs::write(dir.join(format!("v{version}_profiles.bin")), &profiles).expect("write fixture");
    println!("v{version}_profiles.bin ({} bytes)", profiles.len());
}
