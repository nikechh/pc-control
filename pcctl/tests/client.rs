mod common;

use std::net::TcpListener;
use std::time::Duration;

use common::{fixture, real_controllers, FakeServer, Script};
use pcctl::openrgb::{self, packet, Change, Client, Color, DeviceType, Error, Target, Timeouts};
use pcctl::report::Outcome;

const BLUE: Color = Color::new(0, 0, 255);

fn timeouts() -> Timeouts {
    Timeouts { connect: Duration::from_secs(5), reply: Duration::from_millis(300) }
}

fn connect(server: &FakeServer) -> Client {
    match Client::connect_with(&server.addr, timeouts()) {
        Ok(client) => client,
        Err(e) => panic!("connect failed: {e}"),
    }
}

fn connect_err(script: Script) -> Error {
    let server = FakeServer::start(script);
    Client::connect_with(&server.addr, timeouts()).err().expect("connect should fail")
}

fn ram_of(server: &FakeServer) -> openrgb::Controller {
    server.controllers().into_iter().find(|c| c.device_type == DeviceType::Dram).expect("RAM in fake server")
}

#[test]
fn negotiates_version_5_with_a_version_6_server() {
    let server = FakeServer::start(Script::default());
    assert_eq!(connect(&server).version(), 5);
}

#[test]
fn rejects_servers_older_than_version_3() {
    let err = connect_err(Script { version_reply: Some(2), ..Script::default() });
    assert!(matches!(err, Error::ProtocolTooOld { server: 2 }), "{err}");
    assert!(err.to_string().contains("openrgb.org"));
}

#[test]
fn treats_a_silent_server_as_protocol_0() {
    let err = connect_err(Script { version_reply: None, ..Script::default() });
    assert!(matches!(err, Error::ProtocolTooOld { server: 0 }), "{err}");
}

#[test]
fn explains_a_stopped_service() {
    let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let err = Client::connect_with(&format!("127.0.0.1:{port}"), timeouts()).err().expect("nothing listens");
    assert!(matches!(err, Error::NotRunning(_)), "{err}");
    assert!(err.to_string().contains("Start-Service OpenRGB"));
}

#[test]
fn refuses_oversized_replies() {
    let mut header = b"ORGB".to_vec();
    header.extend_from_slice(&0u32.to_le_bytes());
    header.extend_from_slice(&packet::REQUEST_CONTROLLER_COUNT.to_le_bytes());
    header.extend_from_slice(&(2u32 << 20).to_le_bytes());
    let server = FakeServer::start(Script { count_reply_override: Some(header), ..Script::default() });
    let err = connect(&server).controller_count().unwrap_err();
    assert!(matches!(err, Error::TooLarge { size } if size == 2 << 20), "{err}");
}

#[test]
fn refuses_servers_that_are_not_openrgb() {
    let server = FakeServer::start(Script {
        count_reply_override: Some(b"HTTP/1.1 400 Bad Request\r\n\r\n".to_vec()),
        ..Script::default()
    });
    let err = connect(&server).controller_count().unwrap_err();
    assert!(matches!(err, Error::Protocol(_)), "{err}");
}

#[test]
fn refuses_absurd_controller_counts() {
    let mut raw = b"ORGB".to_vec();
    raw.extend_from_slice(&0u32.to_le_bytes());
    raw.extend_from_slice(&packet::REQUEST_CONTROLLER_COUNT.to_le_bytes());
    raw.extend_from_slice(&4u32.to_le_bytes());
    raw.extend_from_slice(&1_000_000u32.to_le_bytes());
    let server = FakeServer::start(Script { count_reply_override: Some(raw), ..Script::default() });
    assert!(matches!(connect(&server).controller_count(), Err(Error::Protocol(_))));
}

#[test]
fn lists_controllers_exactly_as_served() {
    let server = FakeServer::start(Script::default());
    assert_eq!(connect(&server).controllers().unwrap(), real_controllers());
}

#[test]
fn sets_a_solid_ram_color_and_verifies_it() {
    let server = FakeServer::start(Script::default());
    let outcome = openrgb::apply(&mut connect(&server), Target::Ram, &Change::Color(BLUE));
    assert_eq!(outcome, Outcome::Ok("Direct #0000FF".into()));
    let ram = ram_of(&server);
    assert_eq!(ram.active().unwrap().name, "Direct");
    assert!(ram.colors.iter().all(|c| *c == BLUE));
    let received = server.received();
    assert!(received.contains(&packet::UPDATE_MODE) && received.contains(&packet::UPDATE_LEDS));
}

#[test]
fn a_dropped_write_is_reported_as_unverified() {
    let server = FakeServer::start(Script { ignore_writes: true, ..Script::default() });
    let mut client = connect(&server);
    // Start from a non-Direct mode so an ignored write is visible.
    let rainbow = Change::Mode { name: "Rainbow".into(), color: None };
    let outcome = openrgb::apply(&mut client, Target::Ram, &rainbow);
    assert!(matches!(outcome, Outcome::Unverified(_)), "{outcome:?}");
}

#[test]
fn retries_against_the_new_list_when_devices_are_reordered() {
    let server = FakeServer::start(Script { reorder_after_first_write: true, ..Script::default() });
    let outcome = openrgb::apply(&mut connect(&server), Target::Ram, &Change::Color(BLUE));
    assert!(outcome.is_ok(), "{outcome:?}");
    let controllers = server.controllers();
    assert_ne!(controllers[0].device_type, DeviceType::Dram, "the fake server should have reordered");
    assert!(ram_of(&server).colors.iter().all(|c| *c == BLUE));
}

#[test]
fn missing_ram_points_at_pawnio() {
    let server = FakeServer::start(Script { controllers: vec![fixture("gpu"), fixture("motherboard")], ..Script::default() });
    let outcome = openrgb::apply(&mut connect(&server), Target::Ram, &Change::Color(BLUE));
    assert!(matches!(&outcome, Outcome::Failed(m) if m.contains("PawnIO")), "{outcome:?}");
}

#[test]
fn two_devices_of_the_same_type_are_refused() {
    let mut second = fixture("dram");
    second.location = "I2C: somewhere else".into();
    let server = FakeServer::start(Script { controllers: vec![fixture("dram"), second], ..Script::default() });
    let outcome = openrgb::apply(&mut connect(&server), Target::Ram, &Change::Color(BLUE));
    assert!(matches!(&outcome, Outcome::Failed(m) if m.contains("expects exactly one")), "{outcome:?}");
    assert!(!server.received().contains(&packet::UPDATE_MODE), "nothing may be written");
}

#[test]
fn switches_to_a_named_mode_case_insensitively() {
    let server = FakeServer::start(Script::default());
    let outcome =
        openrgb::apply(&mut connect(&server), Target::Ram, &Change::Mode { name: "rainbow".into(), color: None });
    assert_eq!(outcome, Outcome::Ok("Rainbow".into()));
    assert_eq!(ram_of(&server).active().unwrap().name, "Rainbow");
}

#[test]
fn unknown_modes_list_the_available_ones() {
    let server = FakeServer::start(Script::default());
    let outcome =
        openrgb::apply(&mut connect(&server), Target::Ram, &Change::Mode { name: "Disco".into(), color: None });
    assert!(matches!(&outcome, Outcome::Failed(m) if m.contains("Available: Direct, Static")), "{outcome:?}");
}

#[test]
fn a_color_for_a_colorless_mode_is_refused() {
    let mut ram = fixture("dram");
    let rainbow = ram.find_mode("Rainbow").unwrap();
    ram.modes[rainbow].color_mode = 0;
    let server = FakeServer::start(Script { controllers: vec![ram], ..Script::default() });
    let change = Change::Mode { name: "Rainbow".into(), color: Some(BLUE) };
    let outcome = openrgb::apply(&mut connect(&server), Target::Ram, &change);
    assert!(matches!(&outcome, Outcome::Failed(m) if m.contains("doesn't take a color")), "{outcome:?}");
}

#[test]
fn mode_specific_colors_are_written_into_the_mode() {
    let mut ram = fixture("dram");
    let stat = ram.find_mode("Static").unwrap();
    ram.modes[stat].color_mode = 2;
    ram.modes[stat].colors = vec![Color::new(255, 0, 0)];
    let server = FakeServer::start(Script { controllers: vec![ram], ..Script::default() });
    let change = Change::Mode { name: "Static".into(), color: Some(BLUE) };
    let outcome = openrgb::apply(&mut connect(&server), Target::Ram, &change);
    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(ram_of(&server).modes[stat].colors, vec![BLUE]);
    assert!(!server.received().contains(&packet::UPDATE_LEDS));
}

#[test]
fn loads_an_existing_profile() {
    let server = FakeServer::start(Script::default());
    let outcome = openrgb::apply_profile(&mut connect(&server), "default");
    assert!(outcome.is_ok(), "{outcome:?}");
    assert_eq!(server.loaded_profiles(), vec!["default".to_string()]);
}

#[test]
fn unknown_profiles_are_not_sent() {
    let server = FakeServer::start(Script::default());
    let outcome = openrgb::apply_profile(&mut connect(&server), "gaming");
    assert!(matches!(&outcome, Outcome::Failed(m) if m.contains("available: default")), "{outcome:?}");
    assert!(server.loaded_profiles().is_empty());
}
