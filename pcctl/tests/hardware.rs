//! Real-hardware tests. They change the RAM lighting briefly, then restore it.
//! Opt-in only (PLAN.md, 11A):
//!
//!     $env:PCCTL_HW = "1"; cargo test -p pcctl --test hardware -- --ignored --test-threads=1

use pcctl::openrgb::{self, Change, Client, Color, ColorMode, Controller, Target, DEFAULT_ADDR};

fn enabled() -> bool {
    let on = std::env::var("PCCTL_HW").as_deref() == Ok("1");
    if !on {
        eprintln!("skipped: set PCCTL_HW=1 to run hardware tests");
    }
    on
}

fn connect() -> Client {
    Client::connect(DEFAULT_ADDR).unwrap_or_else(|e| panic!("{e}"))
}

fn ram(client: &mut Client) -> (u32, Controller) {
    let controllers = client.controllers().unwrap();
    let index = openrgb::resolve(&controllers, Target::Ram, None).unwrap_or_else(|e| panic!("{e}"));
    (index as u32, controllers[index].clone())
}

fn restore(client: &mut Client, index: u32, before: &Controller) {
    let mode = usize::try_from(before.active_mode).unwrap();
    client.update_mode(index, mode, &before.modes[mode]).unwrap();
    if before.modes[mode].color_kind() == ColorMode::PerLed {
        client.update_leds(index, &before.colors).unwrap();
    }
    let after = client.controller(index).unwrap();
    assert_eq!(after.active_mode, before.active_mode, "RAM mode not restored");
    assert_eq!(after.colors, before.colors, "RAM colors not restored");
}

#[test]
#[ignore = "changes the real RAM lighting; run with PCCTL_HW=1"]
fn ram_color_is_applied_verified_and_restored() {
    if !enabled() {
        return;
    }
    let mut client = connect();
    let (index, before) = ram(&mut client);
    let outcome = openrgb::apply(&mut client, Target::Ram, &Change::Color(Color::new(0, 255, 0)));
    std::thread::sleep(std::time::Duration::from_millis(700));
    restore(&mut client, index, &before);
    assert!(outcome.is_ok(), "{outcome:?}");
}

#[test]
#[ignore = "changes the real RAM lighting; run with PCCTL_HW=1"]
fn loading_a_profile_through_the_sdk_changes_the_ram() {
    if !enabled() {
        return;
    }
    let mut client = connect();
    let (index, before) = ram(&mut client);
    if !client.profiles().unwrap().iter().any(|p| p == "default") {
        eprintln!("skipped: no OpenRGB profile named \"default\"");
        return;
    }
    let magenta = openrgb::apply(&mut client, Target::Ram, &Change::Color(Color::new(255, 0, 255)));
    assert!(magenta.is_ok(), "{magenta:?}");
    let while_magenta = client.controller(index).unwrap();
    let outcome = openrgb::apply_profile(&mut client, "default");
    std::thread::sleep(std::time::Duration::from_millis(700));
    let after_profile = client.controller(index).unwrap();
    restore(&mut client, index, &before);
    assert!(outcome.is_ok(), "{outcome:?}");
    assert_ne!(
        (after_profile.active_mode, &after_profile.colors),
        (while_magenta.active_mode, &while_magenta.colors),
        "loading the profile through the SDK didn't change the RAM"
    );
}
