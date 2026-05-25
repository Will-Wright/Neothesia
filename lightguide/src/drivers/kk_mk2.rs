//! Native Instruments Komplete Kontrol S-series MK2 USB HID lightguide driver.
//!
//! USB device identity (VID 0x17CC = Native Instruments):
//!   PID 0x1610  Komplete Kontrol S49 MK2
//!   PID 0x1620  Komplete Kontrol S61 MK2
//!   PID 0x1630  Komplete Kontrol S88 MK2
//!
//! Protocol — reverse-engineered, all open-source:
//!   - madebycm/LightKontrol (Python, minimal HID REPL)
//!   - ojacques/SynthesiaKontrol (Python, canonical reference, MK1 + MK2)
//!   - tillt/KompleteSynthesia (Swift, most polished — drives LCD screens too)
//!
//! Wire protocol (MK2):
//!   - Init: write a single 3-byte HID report `[0xa0, 0x00, 0x00]`.
//!     The first byte (0xa0) is the init report ID; the trailing zeros
//!     prepare the device to accept light-update reports.
//!   - Light update: write a 250-byte HID report `[0x81, c0, c1, ..., c248]`
//!     where 0x81 is the light-update report ID and c0..c248 are palette
//!     indices for keys 0..248. Keys beyond the keyboard's physical range
//!     are ignored.
//!
//! Palette (partial; see color_scan.py in SynthesiaKontrol for the full sweep):
//!     Low   Medium High  Saturated
//!     0x04  0x05   0x06  0x07   RED
//!     0x08  0x09   0x0a  0x0b   ORANGE
//!     0x0c  0x0d   0x0e  0x0f   LIGHT ORANGE
//!     0x10  0x11   0x12  0x13   YELLOW
//!     0x14  0x15   0x16  0x17   GOLD
//!     0x18  0x19   0x1a  0x1b   LIGHT GREEN
//!     0x1c  0x1d   0x1e  0x1f   GREEN
//!     ...  (palette continues through blues, purples, whites to 0xff)
//!     0x2c  0x2d   0x2e  0x2f   BLUE
//!
//! Convention from Synthesia's finger-based-channel protocol:
//!   - right hand = green (0x1d), right thumb = light green (0x1f)
//!   - left hand  = blue (0x2d), left thumb  = light blue (0x2f)

use anyhow::{Context, Result};
use hidapi::{HidApi, HidDevice};

use crate::Driver;

/// Native Instruments USB vendor ID.
pub const VID: u16 = 0x17CC;

/// All known Komplete Kontrol MK2 product IDs — the driver opens the first one found.
pub const MK2_PIDS: &[u16] = &[0x1610, 0x1620, 0x1630];

/// Light-update payload length, in bytes (does not include the report ID byte).
/// Per the SynthesiaKontrol canonical implementation, the device accepts a
/// fixed-width payload that comfortably covers all MK2 variants up to S88.
const LIGHT_PAYLOAD_LEN: usize = 249;

/// Report ID for light-update writes (MK2).
const REPORT_ID_LIGHT: u8 = 0x81;

/// Init handshake bytes — written once after open, before any light-update.
const INIT_HANDSHAKE: [u8; 3] = [0xa0, 0x00, 0x00];

/// Common palette indices, named after Synthesia's finger-based channel convention.
pub mod palette {
    pub const OFF: u8 = 0x00;
    pub const LIGHT_GREEN: u8 = 0x19;
    pub const GREEN: u8 = 0x1d;
    pub const RIGHT_THUMB: u8 = 0x1f;
    pub const BLUE: u8 = 0x2d;
    pub const LEFT_THUMB: u8 = 0x2f;
}

pub struct KkMk2 {
    device: HidDevice,
}

impl KkMk2 {
    /// Open the first connected Komplete Kontrol S25/S49/S61/S88 MK2 device.
    /// Returns an error if no device is found or if HID open fails.
    pub fn open() -> Result<Self> {
        let api = HidApi::new().context("HidApi::new failed")?;

        let mut last_err: Option<anyhow::Error> = None;
        for &pid in MK2_PIDS {
            match api.open(VID, pid) {
                Ok(device) => {
                    log::info!("opened Komplete Kontrol MK2 (VID 0x{VID:04X} PID 0x{pid:04X})");
                    return Ok(Self { device });
                }
                Err(e) => {
                    last_err = Some(anyhow::anyhow!(e));
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            anyhow::anyhow!(
                "no Komplete Kontrol MK2 device found (looked for VID 0x{VID:04X}, PIDs {MK2_PIDS:?})"
            )
        }))
    }
}

impl Driver for KkMk2 {
    fn init(&mut self) -> Result<()> {
        self.device
            .write(&INIT_HANDSHAKE)
            .context("KK MK2 init handshake (0xa0 0x00 0x00) write failed")?;
        log::debug!("KK MK2 init handshake sent");
        Ok(())
    }

    fn set_all(&mut self, color: u8) -> Result<()> {
        // [report_id, color*LIGHT_PAYLOAD_LEN]
        let mut packet = [0u8; 1 + LIGHT_PAYLOAD_LEN];
        packet[0] = REPORT_ID_LIGHT;
        packet[1..].fill(color);
        self.device
            .write(&packet)
            .with_context(|| format!("KK MK2 set_all(0x{color:02X}) write failed"))?;
        log::debug!("KK MK2 set_all palette=0x{color:02X}");
        Ok(())
    }

    fn set_off(&mut self) -> Result<()> {
        self.set_all(palette::OFF)
    }
}
