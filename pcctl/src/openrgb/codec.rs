//! OpenRGB SDK wire format for protocol versions 3–5 (all integers little-endian).
//!
//!   Controller ─┬─ modes[]   Mode ── colors[]
//!               ├─ zones[]   Zone ─┬─ matrix (optional)
//!               │                  └─ segments[]        (v4+)
//!               ├─ leds[]    Led
//!               ├─ colors[]  one per LED, 0x00BBGGRR
//!               └─ led_display_names[], flags           (v5+)
//!
//! Every count and length comes from the server, so each read is bounds-checked
//! against the received buffer before anything is allocated (PLAN.md, S3).

use std::fmt;

pub const MIN_VERSION: u32 = 3;
pub const MAX_VERSION: u32 = 5;

pub const MODE_FLAG_MANUAL_SAVE: u32 = 1 << 8;
pub const MODE_FLAG_AUTOMATIC_SAVE: u32 = 1 << 9;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    #[error("data ends early: {needed} more bytes needed at offset {offset}")]
    Truncated { offset: usize, needed: usize },
    #[error("{0} unexpected bytes after the end of the data")]
    Trailing(usize),
    #[error("declared size {declared} doesn't match the {actual} bytes received")]
    SizeMismatch { declared: usize, actual: usize },
    #[error("zone matrix of {len} bytes doesn't match {height}x{width}")]
    BadMatrix { len: usize, height: u32, width: u32 },
    #[error("protocol version {0} isn't supported (supported: {MIN_VERSION}-{MAX_VERSION})")]
    Version(u32),
}

type Result<T> = std::result::Result<T, DecodeError>;

pub fn check_version(version: u32) -> Result<()> {
    if (MIN_VERSION..=MAX_VERSION).contains(&version) {
        Ok(())
    } else {
        Err(DecodeError::Version(version))
    }
}

pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(DecodeError::Truncated { offset: self.pos, needed: n - self.remaining() });
        }
        let bytes = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(bytes)
    }

    pub fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }

    /// u16 length (including the NUL terminator), then the bytes.
    pub fn string(&mut self) -> Result<String> {
        let len = self.u16()? as usize;
        let bytes = self.take(len)?;
        let bytes = bytes.strip_suffix(&[0]).unwrap_or(bytes);
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }

    /// Caps a server-supplied count by what the remaining bytes could hold.
    fn capacity(&self, count: usize, min_item_size: usize) -> usize {
        count.min(self.remaining() / min_item_size)
    }

    fn finish(&self) -> Result<()> {
        match self.remaining() {
            0 => Ok(()),
            n => Err(DecodeError::Trailing(n)),
        }
    }

    /// Reads the leading `data_size` field and checks it against the buffer.
    fn sized(payload: &'a [u8]) -> Result<Self> {
        let mut r = Self::new(payload);
        let declared = r.u32()? as usize;
        if declared != payload.len() {
            return Err(DecodeError::SizeMismatch { declared, actual: payload.len() });
        }
        Ok(r)
    }
}

#[derive(Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn i32(&mut self, v: i32) {
        self.u32(v as u32);
    }

    pub fn string(&mut self, s: &str) {
        let bytes = &s.as_bytes()[..s.len().min(u16::MAX as usize - 1)];
        self.u16(bytes.len() as u16 + 1);
        self.buf.extend_from_slice(bytes);
        self.buf.push(0);
    }

    /// Builds a payload that starts with its own total size as a u32.
    fn sized(build: impl FnOnce(&mut Writer)) -> Vec<u8> {
        let mut w = Writer::default();
        w.u32(0);
        build(&mut w);
        let len = w.buf.len() as u32;
        w.buf[..4].copy_from_slice(&len.to_le_bytes());
        w.buf
    }
}

fn read_list<'a, T>(
    r: &mut Reader<'a>,
    min_item_size: usize,
    mut item: impl FnMut(&mut Reader<'a>) -> Result<T>,
) -> Result<Vec<T>> {
    let count = r.u16()? as usize;
    let mut items = Vec::with_capacity(r.capacity(count, min_item_size));
    for _ in 0..count {
        items.push(item(r)?);
    }
    Ok(items)
}

fn write_list<T>(w: &mut Writer, items: &[T], mut item: impl FnMut(&mut Writer, &T)) {
    w.u16(items.len() as u16);
    for i in items {
        item(w, i);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Accepts `RRGGBB` or `#RRGGBB`.
    pub fn from_hex(s: &str) -> Option<Self> {
        let hex = s.strip_prefix('#').unwrap_or(s);
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let v = u32::from_str_radix(hex, 16).ok()?;
        Some(Self::new((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let b = r.take(4)?;
        Ok(Self::new(b[0], b[1], b[2]))
    }

    fn write(&self, w: &mut Writer) {
        w.buf.extend_from_slice(&[self.r, self.g, self.b, 0]);
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    Motherboard,
    Dram,
    Gpu,
    Cooler,
    LedStrip,
    Keyboard,
    Mouse,
    Mousemat,
    Headset,
    HeadsetStand,
    Gamepad,
    Light,
    Speaker,
    Virtual,
    Storage,
    Case,
    Microphone,
    Accessory,
    Keypad,
    Laptop,
    Monitor,
    Unknown,
    Other(i32),
}

/// Position in this table is the raw value (OpenRGB's DEVICE_TYPE_* enum order).
const DEVICE_TYPES: [(DeviceType, &str); 22] = [
    (DeviceType::Motherboard, "Motherboard"),
    (DeviceType::Dram, "DRAM"),
    (DeviceType::Gpu, "GPU"),
    (DeviceType::Cooler, "Cooler"),
    (DeviceType::LedStrip, "LED strip"),
    (DeviceType::Keyboard, "Keyboard"),
    (DeviceType::Mouse, "Mouse"),
    (DeviceType::Mousemat, "Mousemat"),
    (DeviceType::Headset, "Headset"),
    (DeviceType::HeadsetStand, "Headset stand"),
    (DeviceType::Gamepad, "Gamepad"),
    (DeviceType::Light, "Light"),
    (DeviceType::Speaker, "Speaker"),
    (DeviceType::Virtual, "Virtual"),
    (DeviceType::Storage, "Storage"),
    (DeviceType::Case, "Case"),
    (DeviceType::Microphone, "Microphone"),
    (DeviceType::Accessory, "Accessory"),
    (DeviceType::Keypad, "Keypad"),
    (DeviceType::Laptop, "Laptop"),
    (DeviceType::Monitor, "Monitor"),
    (DeviceType::Unknown, "Unknown"),
];

impl DeviceType {
    pub fn from_raw(v: i32) -> Self {
        usize::try_from(v)
            .ok()
            .and_then(|i| DEVICE_TYPES.get(i))
            .map_or(Self::Other(v), |(t, _)| *t)
    }

    pub fn raw(self) -> i32 {
        match self {
            Self::Other(v) => v,
            t => DEVICE_TYPES.iter().position(|(x, _)| *x == t).map_or(-1, |i| i as i32),
        }
    }

    pub fn label(self) -> &'static str {
        DEVICE_TYPES.iter().find(|(x, _)| *x == self).map_or("Other", |(_, label)| label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    NoColor,
    PerLed,
    ModeSpecific,
    Random,
    Unknown(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mode {
    pub name: String,
    pub value: i32,
    pub flags: u32,
    pub speed_min: u32,
    pub speed_max: u32,
    pub brightness_min: u32,
    pub brightness_max: u32,
    pub colors_min: u32,
    pub colors_max: u32,
    pub speed: u32,
    pub brightness: u32,
    pub direction: u32,
    pub color_mode: u32,
    pub colors: Vec<Color>,
}

impl Mode {
    const MIN_SIZE: usize = 52;

    pub fn color_kind(&self) -> ColorMode {
        match self.color_mode {
            0 => ColorMode::NoColor,
            1 => ColorMode::PerLed,
            2 => ColorMode::ModeSpecific,
            3 => ColorMode::Random,
            other => ColorMode::Unknown(other),
        }
    }

    /// Whether OpenRGB can store this mode in the device's own memory.
    pub fn can_save(&self) -> bool {
        self.flags & (MODE_FLAG_MANUAL_SAVE | MODE_FLAG_AUTOMATIC_SAVE) != 0
    }

    // Brightness fields exist from v3 on, which is the minimum supported version.
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            name: r.string()?,
            value: r.i32()?,
            flags: r.u32()?,
            speed_min: r.u32()?,
            speed_max: r.u32()?,
            brightness_min: r.u32()?,
            brightness_max: r.u32()?,
            colors_min: r.u32()?,
            colors_max: r.u32()?,
            speed: r.u32()?,
            brightness: r.u32()?,
            direction: r.u32()?,
            color_mode: r.u32()?,
            colors: read_list(r, 4, Color::read)?,
        })
    }

    fn write(&self, w: &mut Writer) {
        w.string(&self.name);
        w.i32(self.value);
        for v in [
            self.flags,
            self.speed_min,
            self.speed_max,
            self.brightness_min,
            self.brightness_max,
            self.colors_min,
            self.colors_max,
            self.speed,
            self.brightness,
            self.direction,
            self.color_mode,
        ] {
            w.u32(v);
        }
        write_list(w, &self.colors, |w, c| c.write(w));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matrix {
    pub height: u32,
    pub width: u32,
    pub map: Vec<u32>,
}

impl Matrix {
    /// `len` covers height + width + map, so a present matrix is at least 8 bytes.
    fn read(r: &mut Reader<'_>) -> Result<Option<Self>> {
        let len = r.u16()? as usize;
        if len == 0 {
            return Ok(None);
        }
        if len < 8 {
            return Err(DecodeError::BadMatrix { len, height: 0, width: 0 });
        }
        let height = r.u32()?;
        let width = r.u32()?;
        let data_len = len - 8;
        if data_len % 4 != 0 || u64::from(height) * u64::from(width) * 4 != data_len as u64 {
            return Err(DecodeError::BadMatrix { len, height, width });
        }
        let mut map = Vec::with_capacity(r.capacity(data_len / 4, 4));
        for _ in 0..data_len / 4 {
            map.push(r.u32()?);
        }
        Ok(Some(Self { height, width, map }))
    }

    fn write(matrix: Option<&Self>, w: &mut Writer) {
        match matrix {
            None => w.u16(0),
            Some(m) => {
                w.u16((8 + m.map.len() * 4) as u16);
                w.u32(m.height);
                w.u32(m.width);
                for v in &m.map {
                    w.u32(*v);
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub name: String,
    pub segment_type: i32,
    pub start_idx: u32,
    pub leds_count: u32,
}

impl Segment {
    const MIN_SIZE: usize = 14;

    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self { name: r.string()?, segment_type: r.i32()?, start_idx: r.u32()?, leds_count: r.u32()? })
    }

    fn write(&self, w: &mut Writer) {
        w.string(&self.name);
        w.i32(self.segment_type);
        w.u32(self.start_idx);
        w.u32(self.leds_count);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    pub name: String,
    pub zone_type: i32,
    pub leds_min: u32,
    pub leds_max: u32,
    pub leds_count: u32,
    pub matrix: Option<Matrix>,
    pub segments: Vec<Segment>,
    pub flags: u32,
}

impl Zone {
    const MIN_SIZE: usize = 20;

    fn read(r: &mut Reader<'_>, version: u32) -> Result<Self> {
        Ok(Self {
            name: r.string()?,
            zone_type: r.i32()?,
            leds_min: r.u32()?,
            leds_max: r.u32()?,
            leds_count: r.u32()?,
            matrix: Matrix::read(r)?,
            segments: if version >= 4 { read_list(r, Segment::MIN_SIZE, Segment::read)? } else { Vec::new() },
            flags: if version >= 5 { r.u32()? } else { 0 },
        })
    }

    fn write(&self, w: &mut Writer, version: u32) {
        w.string(&self.name);
        w.i32(self.zone_type);
        w.u32(self.leds_min);
        w.u32(self.leds_max);
        w.u32(self.leds_count);
        Matrix::write(self.matrix.as_ref(), w);
        if version >= 4 {
            write_list(w, &self.segments, |w, s| s.write(w));
        }
        if version >= 5 {
            w.u32(self.flags);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Led {
    pub name: String,
    pub value: u32,
}

impl Led {
    const MIN_SIZE: usize = 6;

    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self { name: r.string()?, value: r.u32()? })
    }

    fn write(&self, w: &mut Writer) {
        w.string(&self.name);
        w.u32(self.value);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Controller {
    pub device_type: DeviceType,
    pub name: String,
    pub vendor: String,
    pub description: String,
    pub version: String,
    pub serial: String,
    pub location: String,
    pub modes: Vec<Mode>,
    pub active_mode: i32,
    pub zones: Vec<Zone>,
    pub leds: Vec<Led>,
    pub colors: Vec<Color>,
    pub led_display_names: Vec<String>,
    pub flags: u32,
}

impl Controller {
    pub fn decode(payload: &[u8], version: u32) -> Result<Self> {
        check_version(version)?;
        let mut r = Reader::sized(payload)?;
        let device_type = DeviceType::from_raw(r.i32()?);
        let name = r.string()?;
        let vendor = r.string()?;
        let description = r.string()?;
        let version_str = r.string()?;
        let serial = r.string()?;
        let location = r.string()?;
        // The mode count and the active mode come before the modes themselves.
        let num_modes = r.u16()? as usize;
        let active_mode = r.i32()?;
        let mut modes = Vec::with_capacity(r.capacity(num_modes, Mode::MIN_SIZE));
        for _ in 0..num_modes {
            modes.push(Mode::read(&mut r)?);
        }
        let zones = read_list(&mut r, Zone::MIN_SIZE, |r| Zone::read(r, version))?;
        let leds = read_list(&mut r, Led::MIN_SIZE, Led::read)?;
        let colors = read_list(&mut r, 4, Color::read)?;
        let (led_display_names, flags) = if version >= 5 {
            (read_list(&mut r, 2, Reader::string)?, r.u32()?)
        } else {
            (Vec::new(), 0)
        };
        r.finish()?;
        Ok(Self {
            device_type,
            name,
            vendor,
            description,
            version: version_str,
            serial,
            location,
            modes,
            active_mode,
            zones,
            leds,
            colors,
            led_display_names,
            flags,
        })
    }

    pub fn encode(&self, version: u32) -> Vec<u8> {
        Writer::sized(|w| {
            w.i32(self.device_type.raw());
            for s in [&self.name, &self.vendor, &self.description, &self.version, &self.serial, &self.location] {
                w.string(s);
            }
            w.u16(self.modes.len() as u16);
            w.i32(self.active_mode);
            for m in &self.modes {
                m.write(w);
            }
            write_list(w, &self.zones, |w, z| z.write(w, version));
            write_list(w, &self.leds, |w, l| l.write(w));
            write_list(w, &self.colors, |w, c| c.write(w));
            if version >= 5 {
                write_list(w, &self.led_display_names, |w, s| w.string(s));
                w.u32(self.flags);
            }
        })
    }

    pub fn active(&self) -> Option<&Mode> {
        usize::try_from(self.active_mode).ok().and_then(|i| self.modes.get(i))
    }

    pub fn find_mode(&self, name: &str) -> Option<usize> {
        self.modes.iter().position(|m| m.name.eq_ignore_ascii_case(name))
    }
}

/// UPDATEMODE / SAVEMODE payload: data_size, mode index, mode block.
pub fn encode_update_mode(mode_index: i32, mode: &Mode) -> Vec<u8> {
    Writer::sized(|w| {
        w.i32(mode_index);
        mode.write(w);
    })
}

pub fn decode_update_mode(payload: &[u8]) -> Result<(i32, Mode)> {
    let mut r = Reader::sized(payload)?;
    let index = r.i32()?;
    let mode = Mode::read(&mut r)?;
    r.finish()?;
    Ok((index, mode))
}

/// UPDATELEDS payload: data_size, color count, colors.
pub fn encode_update_leds(colors: &[Color]) -> Vec<u8> {
    Writer::sized(|w| write_list(w, colors, |w, c| c.write(w)))
}

pub fn decode_update_leds(payload: &[u8]) -> Result<Vec<Color>> {
    let mut r = Reader::sized(payload)?;
    let colors = read_list(&mut r, 4, Color::read)?;
    r.finish()?;
    Ok(colors)
}

/// PROFILEMANAGER_GET_PROFILE_LIST reply: data_size, count, names.
pub fn encode_profile_list(names: &[String]) -> Vec<u8> {
    Writer::sized(|w| write_list(w, names, |w, n| w.string(n)))
}

pub fn decode_profile_list(payload: &[u8]) -> Result<Vec<String>> {
    let mut r = Reader::sized(payload)?;
    let names = read_list(&mut r, 2, Reader::string)?;
    r.finish()?;
    Ok(names)
}
