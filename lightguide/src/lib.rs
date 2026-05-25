//! lightguide — lighting output for MIDI visualizers.
//!
//! Driver-agnostic interface at the crate root. Concrete drivers (USB HID,
//! virtual MIDI, future LED strips) live under `drivers::`. First driver
//! is `drivers::kk_mk2` for Native Instruments Komplete Kontrol S-series MK2.
//!
//! Designed to be upstreamable to PolyMeilex/Neothesia as a generic lighting
//! output capability, with KK MK2 as the first concrete driver.

use anyhow::Result;

pub mod drivers;

/// A lightguide driver: a sink for "color this key" / "all keys this color" /
/// "all off" commands. Implementations wrap a hardware connection.
pub trait Driver {
    /// Initialize the device (send init handshake, prepare for commands).
    /// Called once after construction and before any `set_*` call.
    fn init(&mut self) -> Result<()>;

    /// Light every key on the keyboard with the given palette index.
    /// Palette is driver-specific — see the driver's documentation.
    fn set_all(&mut self, color: u8) -> Result<()>;

    /// Turn off every key.
    fn set_off(&mut self) -> Result<()>;
}
