mod common;

use common::{fixture, fixture_bytes, V5};
use pcctl::openrgb::codec::{self, DecodeError};
use pcctl::openrgb::{Color, ColorMode, Controller, DeviceType};

const FIXTURES: [&str; 3] = ["dram", "gpu", "motherboard"];

#[test]
fn real_fixtures_round_trip_byte_for_byte() {
    for name in FIXTURES {
        let bytes = fixture_bytes(name);
        let controller = Controller::decode(bytes, V5).unwrap();
        assert_eq!(controller.encode(V5), bytes, "{name} re-encodes differently");
    }
}

#[test]
fn ram_fixture_matches_what_openrgb_reported() {
    let ram = fixture("dram");
    assert_eq!(ram.device_type, DeviceType::Dram);
    assert_eq!(ram.name, "Kingston Fury DDR4 DRAM");
    assert_eq!(ram.zones.len(), 2);
    assert!(ram.zones.iter().all(|z| z.leds_count == 10));
    assert_eq!(ram.leds.len(), 20);
    assert_eq!(ram.colors.len(), 20);
    let direct = ram.find_mode("direct").expect("Direct mode");
    assert_eq!(ram.modes[direct].color_kind(), ColorMode::PerLed);
    assert!(ram.modes.iter().all(|m| !m.can_save()), "Kingston Fury can't save modes to the sticks");
}

#[test]
fn older_protocol_versions_round_trip_without_newer_fields() {
    for name in FIXTURES {
        let c = fixture(name);
        for version in [3, 4] {
            let mut expected = c.clone();
            expected.led_display_names.clear();
            expected.flags = 0;
            for z in &mut expected.zones {
                z.flags = 0;
                if version < 4 {
                    z.segments.clear();
                }
            }
            let decoded = Controller::decode(&c.encode(version), version).unwrap();
            assert_eq!(decoded, expected, "{name} at v{version}");
        }
    }
}

#[test]
fn unsupported_versions_are_rejected() {
    let bytes = fixture_bytes("dram");
    assert_eq!(Controller::decode(bytes, 2), Err(DecodeError::Version(2)));
    assert_eq!(Controller::decode(bytes, 6), Err(DecodeError::Version(6)));
}

#[test]
fn every_truncation_is_an_error_not_a_panic() {
    for name in FIXTURES {
        let bytes = fixture_bytes(name);
        for len in 0..bytes.len() {
            assert!(Controller::decode(&bytes[..len], V5).is_err(), "{name} truncated to {len} decoded");
        }
    }
}

#[test]
fn declared_size_must_match_received_bytes() {
    let mut bytes = fixture_bytes("gpu").to_vec();
    bytes.push(0);
    assert!(matches!(Controller::decode(&bytes, V5), Err(DecodeError::SizeMismatch { .. })));
}

#[test]
fn random_corruption_never_panics() {
    // Deterministic xorshift so failures are reproducible.
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for name in FIXTURES {
        let original = fixture_bytes(name);
        for _ in 0..3000 {
            let mut bytes = original.to_vec();
            for _ in 0..(next() % 4 + 1) {
                let i = (next() as usize) % bytes.len();
                bytes[i] = next() as u8;
            }
            let _ = Controller::decode(&bytes, V5);
        }
    }
}

#[test]
fn huge_counts_do_not_allocate_beyond_the_buffer() {
    // data_size, type, 6 empty strings, then num_modes = 0xFFFF with nothing after it.
    let mut bytes = vec![0u8; 4];
    bytes.extend_from_slice(&1i32.to_le_bytes());
    for _ in 0..6 {
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.push(0);
    }
    bytes.extend_from_slice(&u16::MAX.to_le_bytes());
    bytes.extend_from_slice(&0i32.to_le_bytes());
    let len = bytes.len() as u32;
    bytes[..4].copy_from_slice(&len.to_le_bytes());
    assert!(matches!(Controller::decode(&bytes, V5), Err(DecodeError::Truncated { .. })));
}

#[test]
fn profile_list_fixture_round_trips() {
    let bytes = fixture_bytes("profiles");
    let names = codec::decode_profile_list(bytes).unwrap();
    assert_eq!(names, vec!["default".to_string()]);
    assert_eq!(codec::encode_profile_list(&names), bytes);
}

#[test]
fn update_payloads_round_trip() {
    let ram = fixture("dram");
    let (index, mode) = codec::decode_update_mode(&codec::encode_update_mode(3, &ram.modes[3])).unwrap();
    assert_eq!((index, &mode), (3, &ram.modes[3]));
    let colors = vec![Color::new(1, 2, 3); 20];
    assert_eq!(codec::decode_update_leds(&codec::encode_update_leds(&colors)).unwrap(), colors);
}

#[test]
fn colors_use_openrgb_byte_order() {
    let payload = codec::encode_update_leds(&[Color::new(0x11, 0x22, 0x33)]);
    assert_eq!(&payload[6..], &[0x11, 0x22, 0x33, 0x00]);
}

#[test]
fn hex_colors_parse_and_print() {
    assert_eq!(Color::from_hex("#0000FF"), Some(Color::new(0, 0, 255)));
    assert_eq!(Color::from_hex("ff8000"), Some(Color::new(255, 128, 0)));
    assert_eq!(Color::from_hex("#00F"), None);
    assert_eq!(Color::from_hex("#GG0000"), None);
    assert_eq!(Color::from_hex("+12345"), None);
    assert_eq!(Color::new(255, 128, 0).to_string(), "#FF8000");
}
