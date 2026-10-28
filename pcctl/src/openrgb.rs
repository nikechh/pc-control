//! OpenRGB SDK client (protocol 3–5, synchronous, std only) and the
//! resolve → write → verify flow behind `pcctl set` and `pcctl apply`.
//!
//!   connect ─► SET_CLIENT_NAME ─► REQUEST_PROTOCOL_VERSION (require ≥ 3, use ≤ 5)
//!                                          │
//!   ┌──────────────────────────────────────┘
//!   ▼
//!   list controllers ─► resolve by device type (+ location on retry)
//!         ▲                         │
//!         │                         ▼
//!         │              UpdateMode (+ UpdateLeds) ─► re-fetch that controller
//!         │                                                    │
//!         └──── packet 100 arrived meanwhile: retry once ◄─────┤
//!                                                              ▼
//!                                  compare with what was sent ─► Ok / Unverified
//!
//! OpenRGB doesn't acknowledge writes before protocol 6, so reading the controller
//! back is the only way to notice a write that was dropped (PLAN.md, 17A).

pub mod codec;

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

pub use codec::{Color, ColorMode, Controller, DeviceType, Mode};

use crate::report::Outcome;

pub const DEFAULT_ADDR: &str = "127.0.0.1:6742";

const MAGIC: &[u8; 4] = b"ORGB";
const MAX_PACKET_SIZE: usize = 1 << 20;
const MAX_CONTROLLERS: u32 = 256;
const MAX_NOTIFICATIONS_PER_REPLY: usize = 64;
const CLIENT_NAME: &str = "pc-control";

pub mod packet {
    pub const REQUEST_CONTROLLER_COUNT: u32 = 0;
    pub const REQUEST_CONTROLLER_DATA: u32 = 1;
    pub const REQUEST_PROTOCOL_VERSION: u32 = 40;
    pub const SET_CLIENT_NAME: u32 = 50;
    pub const DEVICE_LIST_UPDATED: u32 = 100;
    pub const GET_PROFILE_LIST: u32 = 150;
    pub const LOAD_PROFILE: u32 = 152;
    pub const UPDATE_LEDS: u32 = 1050;
    pub const UPDATE_MODE: u32 = 1101;
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("\"{0}\" isn't a valid address (expected e.g. 127.0.0.1:6742)")]
    BadAddress(String),
    #[error(
        "the OpenRGB service isn't running (nothing accepted a connection on {0}). \
         Start it from an admin terminal: Start-Service OpenRGB"
    )]
    NotRunning(SocketAddr),
    #[error("OpenRGB didn't accept a connection on {0} in time. Check the service: Get-Service OpenRGB")]
    ConnectTimeout(SocketAddr),
    #[error("can't connect to OpenRGB on {addr}: {source}")]
    Connect {
        addr: SocketAddr,
        #[source]
        source: io::Error,
    },
    #[error(
        "OpenRGB speaks SDK protocol {server}, but pc-control needs at least {min}. \
         Update OpenRGB from https://openrgb.org",
        min = codec::MIN_VERSION
    )]
    ProtocolTooOld { server: u32 },
    #[error("OpenRGB stopped answering (timed out waiting for a reply)")]
    Timeout,
    #[error("the connection to OpenRGB was closed")]
    Disconnected,
    #[error("the server sent a {size}-byte reply; refusing anything over {MAX_PACKET_SIZE} bytes")]
    TooLarge { size: usize },
    #[error("unexpected data from the server (is it really OpenRGB?): {0}")]
    Protocol(String),
    #[error("malformed reply from OpenRGB: {0}")]
    Decode(#[from] codec::DecodeError),
    #[error("connection error: {0}")]
    Io(io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    pub connect: Duration,
    pub reply: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        // Windows retries a refused localhost connection for ~2 s before reporting it.
        Self { connect: Duration::from_secs(5), reply: Duration::from_secs(3) }
    }
}

fn io_error(e: io::Error) -> Error {
    match e.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => Error::Timeout,
        io::ErrorKind::UnexpectedEof
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::BrokenPipe => Error::Disconnected,
        _ => Error::Io(e),
    }
}

fn le_u32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn u32_payload(payload: &[u8]) -> Result<u32> {
    match payload {
        [a, b, c, d] => Ok(u32::from_le_bytes([*a, *b, *c, *d])),
        _ => Err(Error::Protocol(format!("expected a 4-byte value, got {} bytes", payload.len()))),
    }
}

fn nul_terminated(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}

pub struct Client {
    stream: TcpStream,
    version: u32,
    list_changed: bool,
}

impl Client {
    pub fn connect(addr: &str) -> Result<Self> {
        Self::connect_with(addr, Timeouts::default())
    }

    pub fn connect_with(addr: &str, timeouts: Timeouts) -> Result<Self> {
        let sock: SocketAddr = addr.parse().map_err(|_| Error::BadAddress(addr.to_string()))?;
        let stream = TcpStream::connect_timeout(&sock, timeouts.connect).map_err(|e| match e.kind() {
            io::ErrorKind::ConnectionRefused => Error::NotRunning(sock),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => Error::ConnectTimeout(sock),
            _ => Error::Connect { addr: sock, source: e },
        })?;
        stream.set_read_timeout(Some(timeouts.reply)).map_err(Error::Io)?;
        stream.set_write_timeout(Some(timeouts.reply)).map_err(Error::Io)?;
        stream.set_nodelay(true).map_err(Error::Io)?;

        let mut client = Self { stream, version: 0, list_changed: false };
        client.send(0, packet::SET_CLIENT_NAME, &nul_terminated(CLIENT_NAME))?;
        client.send(0, packet::REQUEST_PROTOCOL_VERSION, &codec::MAX_VERSION.to_le_bytes())?;
        let server = match client.recv_reply(packet::REQUEST_PROTOCOL_VERSION) {
            Ok(payload) => u32_payload(&payload)?,
            // Protocol-0 servers never answer this request.
            Err(Error::Timeout) => 0,
            Err(e) => return Err(e),
        };
        if server < codec::MIN_VERSION {
            return Err(Error::ProtocolTooOld { server });
        }
        client.version = server.min(codec::MAX_VERSION);
        Ok(client)
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    /// True if OpenRGB announced a device-list change since the last call.
    pub fn take_list_changed(&mut self) -> bool {
        std::mem::take(&mut self.list_changed)
    }

    fn send(&mut self, device: u32, id: u32, payload: &[u8]) -> Result<()> {
        let mut bytes = Vec::with_capacity(16 + payload.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&device.to_le_bytes());
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        self.stream.write_all(&bytes).map_err(io_error)
    }

    fn recv(&mut self) -> Result<(u32, Vec<u8>)> {
        let mut header = [0u8; 16];
        self.stream.read_exact(&mut header).map_err(io_error)?;
        if &header[..4] != MAGIC {
            return Err(Error::Protocol(format!("bad packet header {:02X?}", &header[..4])));
        }
        let id = le_u32(&header[8..12]);
        let size = le_u32(&header[12..16]) as usize;
        if size > MAX_PACKET_SIZE {
            return Err(Error::TooLarge { size });
        }
        let mut payload = vec![0; size];
        self.stream.read_exact(&mut payload).map_err(io_error)?;
        Ok((id, payload))
    }

    fn recv_reply(&mut self, want: u32) -> Result<Vec<u8>> {
        for _ in 0..=MAX_NOTIFICATIONS_PER_REPLY {
            let (id, payload) = self.recv()?;
            match id {
                _ if id == want => return Ok(payload),
                packet::DEVICE_LIST_UPDATED => self.list_changed = true,
                other => return Err(Error::Protocol(format!("expected packet {want}, got packet {other}"))),
            }
        }
        Err(Error::Protocol("too many notifications before the reply".into()))
    }

    pub fn controller_count(&mut self) -> Result<u32> {
        self.send(0, packet::REQUEST_CONTROLLER_COUNT, &[])?;
        let count = u32_payload(&self.recv_reply(packet::REQUEST_CONTROLLER_COUNT)?)?;
        if count > MAX_CONTROLLERS {
            return Err(Error::Protocol(format!("server claims {count} controllers")));
        }
        Ok(count)
    }

    /// Raw controller data payload, as sent by the server.
    pub fn controller_raw(&mut self, index: u32) -> Result<Vec<u8>> {
        let version = self.version.to_le_bytes();
        self.send(index, packet::REQUEST_CONTROLLER_DATA, &version)?;
        self.recv_reply(packet::REQUEST_CONTROLLER_DATA)
    }

    pub fn controller(&mut self, index: u32) -> Result<Controller> {
        let raw = self.controller_raw(index)?;
        Ok(Controller::decode(&raw, self.version)?)
    }

    pub fn controllers(&mut self) -> Result<Vec<Controller>> {
        let count = self.controller_count()?;
        (0..count).map(|i| self.controller(i)).collect()
    }

    /// Raw profile list payload, as sent by the server.
    pub fn profiles_raw(&mut self) -> Result<Vec<u8>> {
        self.send(0, packet::GET_PROFILE_LIST, &[])?;
        self.recv_reply(packet::GET_PROFILE_LIST)
    }

    pub fn profiles(&mut self) -> Result<Vec<String>> {
        Ok(codec::decode_profile_list(&self.profiles_raw()?)?)
    }

    pub fn load_profile(&mut self, name: &str) -> Result<()> {
        self.send(0, packet::LOAD_PROFILE, &nul_terminated(name))
    }

    pub fn update_mode(&mut self, index: u32, mode_index: usize, mode: &Mode) -> Result<()> {
        self.send(index, packet::UPDATE_MODE, &codec::encode_update_mode(mode_index as i32, mode))
    }

    pub fn update_leds(&mut self, index: u32, colors: &[Color]) -> Result<()> {
        self.send(index, packet::UPDATE_LEDS, &codec::encode_update_leds(colors))
    }

    /// A round trip, so everything sent before it has been processed by the server.
    pub fn sync(&mut self) -> Result<()> {
        self.controller_count().map(|_| ())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Ram,
    Gpu,
    Board,
}

impl Target {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "ram" | "dram" => Some(Self::Ram),
            "gpu" => Some(Self::Gpu),
            "board" | "motherboard" | "mb" => Some(Self::Board),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ram => "ram",
            Self::Gpu => "gpu",
            Self::Board => "board",
        }
    }

    pub fn device_type(self) -> DeviceType {
        match self {
            Self::Ram => DeviceType::Dram,
            Self::Gpu => DeviceType::Gpu,
            Self::Board => DeviceType::Motherboard,
        }
    }

    fn missing_hint(self) -> &'static str {
        match self {
            Self::Ram => {
                "If PawnIO failed to load, OpenRGB can't see the RAM: look for \"PawnIO\" in the newest log \
                 under C:\\Program Files\\OpenRGB\\service_config\\logs, then restart the service from an \
                 admin terminal (Restart-Service OpenRGB)."
            }
            Self::Gpu | Self::Board => "Check what OpenRGB detects with: pcctl list",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Solid color, using the device's Direct mode (or Static if it has none).
    Color(Color),
    Mode { name: String, color: Option<Color> },
}

#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error(transparent)]
    Client(#[from] Error),
    #[error("OpenRGB has no {target} device. {hint}")]
    NotFound { target: &'static str, hint: &'static str },
    #[error("OpenRGB reports {count} {target} devices ({locations}); pc-control expects exactly one")]
    Ambiguous { target: &'static str, count: usize, locations: String },
    #[error("{device} has no mode named \"{name}\". Available: {available}")]
    UnknownMode { device: String, name: String, available: String },
    #[error("{device} has no Direct or Static mode to set a color with. Available: {available}")]
    NoColorMode { device: String, available: String },
    #[error("mode \"{mode}\" doesn't take a color")]
    ModeHasNoColor { mode: String },
}

/// Finds the single controller of the target's type (and location, when known).
pub fn resolve(
    controllers: &[Controller],
    target: Target,
    location: Option<&str>,
) -> std::result::Result<usize, ApplyError> {
    let found: Vec<usize> = controllers
        .iter()
        .enumerate()
        .filter(|(_, c)| c.device_type == target.device_type() && location.map_or(true, |l| c.location == l))
        .map(|(i, _)| i)
        .collect();
    match found.as_slice() {
        [index] => Ok(*index),
        [] => Err(ApplyError::NotFound { target: target.label(), hint: target.missing_hint() }),
        many => Err(ApplyError::Ambiguous {
            target: target.label(),
            count: many.len(),
            locations: many.iter().map(|&i| controllers[i].location.as_str()).collect::<Vec<_>>().join("; "),
        }),
    }
}

struct Plan {
    mode_index: usize,
    mode: Mode,
    led_colors: Option<Vec<Color>>,
    summary: String,
}

fn plan(controller: &Controller, change: &Change) -> std::result::Result<Plan, ApplyError> {
    let available = || controller.modes.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", ");
    let (mode_index, color) = match change {
        Change::Color(color) => {
            let index = controller
                .find_mode("Direct")
                .or_else(|| controller.find_mode("Static"))
                .ok_or_else(|| ApplyError::NoColorMode { device: controller.name.clone(), available: available() })?;
            (index, Some(*color))
        }
        Change::Mode { name, color } => {
            let index = controller.find_mode(name).ok_or_else(|| ApplyError::UnknownMode {
                device: controller.name.clone(),
                name: name.clone(),
                available: available(),
            })?;
            (index, *color)
        }
    };

    let mut mode = controller.modes[mode_index].clone();
    let mut led_colors = None;
    if let Some(color) = color {
        match mode.color_kind() {
            ColorMode::PerLed => led_colors = Some(vec![color; controller.leds.len()]),
            ColorMode::ModeSpecific => {
                let n = mode.colors.len().max(mode.colors_min as usize).max(1);
                mode.colors = vec![color; n];
            }
            _ => return Err(ApplyError::ModeHasNoColor { mode: mode.name.clone() }),
        }
    }
    let summary = match color {
        Some(c) => format!("{} {c}", mode.name),
        None => mode.name.clone(),
    };
    Ok(Plan { mode_index, mode, led_colors, summary })
}

fn verify(after: &Controller, location: &str, plan: &Plan) -> Outcome {
    if after.location != location {
        return Outcome::Unverified("a different device answered at this index after the write".into());
    }
    if after.active_mode != plan.mode_index as i32 {
        let current = after.active().map_or("?", |m| m.name.as_str());
        return Outcome::Unverified(format!(
            "OpenRGB still reports mode \"{current}\" instead of \"{}\"",
            plan.mode.name
        ));
    }
    if let Some(colors) = &plan.led_colors {
        if &after.colors != colors {
            return Outcome::Unverified("OpenRGB didn't take the new LED colors".into());
        }
    }
    if after.modes.get(plan.mode_index).map(|m| &m.colors) != Some(&plan.mode.colors) {
        return Outcome::Unverified("OpenRGB didn't take the new mode colors".into());
    }
    Outcome::Ok(plan.summary.clone())
}

fn try_apply(client: &mut Client, target: Target, change: &Change) -> std::result::Result<Outcome, ApplyError> {
    let mut location: Option<String> = None;
    for _ in 0..2 {
        client.take_list_changed();
        let controllers = client.controllers()?;
        let index = resolve(&controllers, target, location.as_deref())?;
        let controller = &controllers[index];
        location = Some(controller.location.clone());
        let plan = plan(controller, change)?;

        let index = index as u32;
        client.update_mode(index, plan.mode_index, &plan.mode)?;
        if let Some(colors) = &plan.led_colors {
            client.update_leds(index, colors)?;
        }
        let after = client.controller(index)?;
        if client.take_list_changed() {
            // Indexes may have shifted under us; redo against the fresh list.
            continue;
        }
        return Ok(verify(&after, &controller.location, &plan));
    }
    Ok(Outcome::Unverified("OpenRGB's device list changed twice while applying; run the command again".into()))
}

/// Sets a color and/or mode on one device and reads it back to confirm.
pub fn apply(client: &mut Client, target: Target, change: &Change) -> Outcome {
    try_apply(client, target, change).unwrap_or_else(|e| Outcome::Failed(e.to_string()))
}

fn try_apply_profile(client: &mut Client, name: &str) -> Result<Outcome> {
    let profiles = client.profiles()?;
    if !profiles.iter().any(|p| p == name) {
        let available = if profiles.is_empty() { "none saved yet".to_string() } else { profiles.join(", ") };
        return Ok(Outcome::Failed(format!("OpenRGB has no profile named \"{name}\" (available: {available})")));
    }
    client.load_profile(name)?;
    client.sync()?;
    Ok(Outcome::Ok(format!(
        "profile \"{name}\" loaded (OpenRGB can't confirm profile loads before SDK protocol 6)"
    )))
}

/// Loads a profile saved in OpenRGB, after checking that it exists.
pub fn apply_profile(client: &mut Client, name: &str) -> Outcome {
    try_apply_profile(client, name).unwrap_or_else(|e| Outcome::Failed(e.to_string()))
}
