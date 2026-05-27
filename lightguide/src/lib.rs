//! lightguide — lighting output for MIDI visualizers.
//!
//! Driver-agnostic interface at the crate root. Concrete drivers (USB HID,
//! virtual MIDI, future LED strips) live under `drivers::`. First driver
//! is `drivers::kk_mk2` for Native Instruments Komplete Kontrol S-series MK2.
//!
//! Designed to be upstreamable to PolyMeilex/Neothesia as a generic lighting
//! output capability, with KK MK2 as the first concrete driver.
//!
//! ## Initialization discipline (macOS)
//!
//! `hidapi`'s `HidApi::new` triggers IOHIDManager device enumeration on
//! macOS, which dispatches CFRunLoop notifications. When that call happens
//! from inside winit's event handler (e.g. per-frame menu update or scene
//! transition), the notifications re-enter winit's dispatch and panic with
//! "tried to handle event while another event is currently being handled."
//!
//! Fix: `lightguide` keeps a process-global `HidApi` handle behind a
//! `OnceLock`. Consumers running inside an event loop (Neothesia) must call
//! [`init_hidapi`] once at app startup BEFORE the event loop begins, so the
//! enumeration completes in a non-reentrant context. Standalone consumers
//! (the `lightguide-test` binary, integration tests) get lazy init for free
//! — `KkMk2::open` calls `ensure_hidapi` which initializes on first access.

use std::ffi::CString;
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};
use hidapi::HidApi;

pub mod drivers;

/// Process-global lightguide runtime state. Holds the `HidApi` handle plus
/// per-driver device paths cached at enumeration time. Subsequent device
/// opens use `HidApi::open_path` against the cached paths, which does NOT
/// re-enumerate — critical for avoiding CFRunLoop dispatch from inside
/// winit's event handler on macOS.
struct LightguideRuntime {
    api: Mutex<HidApi>,
    /// Path of the first detected Komplete Kontrol S25/S49/S61/S88 MK2
    /// device, if any. None if the device was unplugged at init time.
    kk_mk2_path: Option<CString>,
}

static RUNTIME: OnceLock<LightguideRuntime> = OnceLock::new();

/// Eagerly initialize the process-global `HidApi` handle and enumerate
/// supported devices. Call this once at app startup, BEFORE entering an
/// event loop, on macOS / any platform where `HidApi::new` may dispatch
/// run-loop notifications.
///
/// Safe to call multiple times — subsequent calls are no-ops. Returns
/// `Err` if `HidApi::new` itself fails; absence of a target device is NOT
/// an error (the driver-specific `open` calls handle that).
pub fn init_hidapi() -> Result<()> {
    ensure_runtime().map(|_| ())
}

pub(crate) fn ensure_runtime() -> Result<&'static LightguideRuntime> {
    if let Some(rt) = RUNTIME.get() {
        return Ok(rt);
    }

    let api = HidApi::new().context("HidApi::new failed")?;

    // Cache the KK MK2 device path. Matching device-info iteration is read-only
    // against the in-memory enumeration result HidApi already populated during
    // `new()`, so this does NOT trigger another `hid_enumerate` round.
    let kk_mk2_path = api
        .device_list()
        .find(|d| {
            d.vendor_id() == drivers::kk_mk2::VID
                && drivers::kk_mk2::MK2_PIDS.contains(&d.product_id())
        })
        .map(|d| d.path().to_owned());

    if let Some(p) = &kk_mk2_path {
        log::info!("lightguide: cached KK MK2 device path ({} bytes)", p.as_bytes().len());
    } else {
        log::info!("lightguide: no KK MK2 device found at init time");
    }

    let runtime = LightguideRuntime {
        api: Mutex::new(api),
        kk_mk2_path,
    };

    // Racing setters both end up with equivalent runtimes; losing handle drops.
    let _ = RUNTIME.set(runtime);
    Ok(RUNTIME.get().expect("RUNTIME set but get returned None"))
}

pub(crate) fn kk_mk2_path() -> Result<&'static CString> {
    let rt = ensure_runtime()?;
    rt.kk_mk2_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!(
            "no Komplete Kontrol MK2 device cached — was the keyboard plugged in at app startup?"
        ))
}

pub(crate) fn with_hidapi<R>(f: impl FnOnce(&HidApi) -> R) -> Result<R> {
    let rt = ensure_runtime()?;
    let api = rt.api.lock().expect("lightguide HidApi mutex poisoned");
    Ok(f(&api))
}

/// A lightguide driver: a sink for "color this key" / "all keys this color" /
/// "all off" commands. Implementations wrap a hardware connection.
///
/// Cost model: every method is a single wire write to the hardware. The KK MK2
/// always sends a fixed-width report; per-key and full-bus updates have
/// identical wire cost. The trait reflects this — `set_one` is not built on
/// top of `set_all`, both are first-class.
pub trait Driver {
    /// Initialize the device (send init handshake, prepare for commands).
    /// Called once after construction and before any `set_*` call.
    fn init(&mut self) -> Result<()>;

    /// Set a single key to the given palette index without disturbing the
    /// other keys' current state. Drivers maintain internal per-key state
    /// to support this. Out-of-range key indices are silently ignored
    /// (callers may pass MIDI notes that don't fit a smaller keyboard).
    /// Palette is driver-specific — see the driver's documentation.
    fn set_one(&mut self, key: u8, color: u8) -> Result<()>;

    /// Light every key on the keyboard with the given palette index.
    /// Palette is driver-specific — see the driver's documentation.
    fn set_all(&mut self, color: u8) -> Result<()>;

    /// Turn off every key.
    fn set_off(&mut self) -> Result<()>;
}
