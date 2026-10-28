#![allow(dead_code)]
//! In-process fake OpenRGB server for tests.
//!
//!   test ──pcctl Client──► 127.0.0.1:<random port> ──► server thread
//!                                                         ├─ answers from the Script
//!                                                         ├─ applies writes to its own controllers
//!                                                         └─ records every packet id (for asserts)

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use pcctl::openrgb::codec;
use pcctl::openrgb::{packet, Controller};

pub const V5: u32 = 5;

pub fn fixture_bytes(name: &str) -> &'static [u8] {
    match name {
        "dram" => include_bytes!("../fixtures/v5_dram.bin"),
        "gpu" => include_bytes!("../fixtures/v5_gpu.bin"),
        "motherboard" => include_bytes!("../fixtures/v5_motherboard.bin"),
        "profiles" => include_bytes!("../fixtures/v5_profiles.bin"),
        other => panic!("no fixture named {other}"),
    }
}

pub fn fixture(name: &str) -> Controller {
    Controller::decode(fixture_bytes(name), V5).expect("fixture decodes")
}

/// Same order as the real service reported them.
pub fn real_controllers() -> Vec<Controller> {
    vec![fixture("dram"), fixture("gpu"), fixture("motherboard")]
}

#[derive(Clone)]
pub struct Script {
    /// None: never answer REQUEST_PROTOCOL_VERSION (like a protocol-0 server).
    pub version_reply: Option<u32>,
    pub controllers: Vec<Controller>,
    pub profiles: Vec<String>,
    /// Accept writes but don't change anything (simulates a dropped write).
    pub ignore_writes: bool,
    /// After the first UpdateMode: send DEVICE_LIST_UPDATED and rotate the controller order.
    pub reorder_after_first_write: bool,
    /// Raw bytes to send instead of the controller-count reply.
    pub count_reply_override: Option<Vec<u8>>,
}

impl Default for Script {
    fn default() -> Self {
        Self {
            version_reply: Some(6),
            controllers: real_controllers(),
            profiles: vec!["default".into()],
            ignore_writes: false,
            reorder_after_first_write: false,
            count_reply_override: None,
        }
    }
}

#[derive(Default)]
struct State {
    controllers: Vec<Controller>,
    received: Vec<u32>,
    loaded_profiles: Vec<String>,
    version: u32,
    reordered: bool,
}

pub struct FakeServer {
    pub addr: String,
    state: Arc<Mutex<State>>,
}

impl FakeServer {
    pub fn start(script: Script) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let state = Arc::new(Mutex::new(State { controllers: script.controllers.clone(), ..State::default() }));
        let shared = Arc::clone(&state);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let _ = serve(stream, &script, &shared);
            }
        });
        Self { addr, state }
    }

    pub fn received(&self) -> Vec<u32> {
        self.state.lock().unwrap().received.clone()
    }

    pub fn loaded_profiles(&self) -> Vec<String> {
        self.state.lock().unwrap().loaded_profiles.clone()
    }

    pub fn controllers(&self) -> Vec<Controller> {
        self.state.lock().unwrap().controllers.clone()
    }
}

fn le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn reply(s: &mut TcpStream, id: u32, payload: &[u8]) -> std::io::Result<()> {
    let mut bytes = b"ORGB".to_vec();
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&id.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(payload);
    s.write_all(&bytes)
}

fn serve(mut s: TcpStream, script: &Script, state: &Mutex<State>) -> std::io::Result<()> {
    loop {
        let mut header = [0u8; 16];
        if s.read_exact(&mut header).is_err() {
            return Ok(());
        }
        let device = le(&header[4..8]) as usize;
        let id = le(&header[8..12]);
        let mut payload = vec![0; le(&header[12..16]) as usize];
        s.read_exact(&mut payload)?;

        let mut st = state.lock().unwrap();
        st.received.push(id);
        match id {
            packet::REQUEST_PROTOCOL_VERSION => {
                if let Some(server) = script.version_reply {
                    st.version = server.min(le(&payload));
                    reply(&mut s, id, &server.to_le_bytes())?;
                }
            }
            packet::REQUEST_CONTROLLER_COUNT => match &script.count_reply_override {
                Some(raw) => s.write_all(raw)?,
                None => reply(&mut s, id, &(st.controllers.len() as u32).to_le_bytes())?,
            },
            packet::REQUEST_CONTROLLER_DATA => {
                let data = st.controllers[device].encode(le(&payload));
                reply(&mut s, id, &data)?;
            }
            packet::GET_PROFILE_LIST => reply(&mut s, id, &codec::encode_profile_list(&script.profiles))?,
            packet::LOAD_PROFILE => {
                let name = payload.strip_suffix(&[0]).unwrap_or(&payload);
                st.loaded_profiles.push(String::from_utf8_lossy(name).into_owned());
            }
            packet::UPDATE_MODE => {
                if !script.ignore_writes {
                    let (index, mode) = codec::decode_update_mode(&payload).expect("valid UpdateMode");
                    let c = &mut st.controllers[device];
                    c.modes[index as usize] = mode;
                    c.active_mode = index;
                }
                if script.reorder_after_first_write && !st.reordered {
                    st.reordered = true;
                    st.controllers.rotate_left(1);
                    reply(&mut s, packet::DEVICE_LIST_UPDATED, &[])?;
                }
            }
            packet::UPDATE_LEDS if !script.ignore_writes => {
                st.controllers[device].colors = codec::decode_update_leds(&payload).expect("valid UpdateLeds");
            }
            _ => {}
        }
    }
}
