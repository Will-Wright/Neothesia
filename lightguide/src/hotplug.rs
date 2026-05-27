//! Hot-plug detection for lightguide-supported devices.
//!
//! macOS-friendly: runs on a dedicated worker thread, never inside winit's
//! event handler. `hidapi::HidApi::refresh_devices()` pumps the calling
//! thread's CFRunLoop on macOS, which is fine on a worker thread but
//! reentrant-panics if called from inside winit's event handler on the
//! main thread (the Phase 3b bug class).
//!
//! Pattern: spawn a thread, refresh `hidapi`'s device list every
//! `POLL_INTERVAL`, compare current device-path state to last-known
//! state, fire a callback when state changes. The callback typically
//! wraps a `winit::event_loop::EventLoopProxy` and posts a user event
//! that the main thread handles in a non-reentrant context.
//!
//! IOKit notification-based discovery (zero polling) is the structural
//! ideal but adds substantial FFI for marginal UX gain at v0.1 cadences.
//! Polling at 2s is below human plug/unplug → expected-effect tolerance
//! for piano-learning UX. Promote to IOKit later if a concrete need
//! surfaces.

use std::ffi::CString;
use std::thread;
use std::time::Duration;

use hidapi::HidApi;

use crate::drivers::kk_mk2;

/// How often the watcher thread refreshes the device list.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Topology event observed by the watcher. The watcher fires exactly one
/// of these per state transition.
#[derive(Debug, Clone)]
pub enum HotplugEvent {
    /// A KK MK2 became reachable. Includes the device path, suitable for
    /// passing to `HidApi::open_path` from the main thread later.
    KkMk2Connected { path: CString },
    /// The previously connected KK MK2 became unreachable (unplug, USB
    /// hub power cycle, etc.).
    KkMk2Disconnected,
}

/// Spawn a background watcher thread. The callback is invoked on the
/// watcher thread (NOT the calling thread) whenever device topology
/// changes for a supported device. Returns immediately; thread runs for
/// the process lifetime.
///
/// The callback should be cheap and non-blocking — typical implementation
/// is forwarding to a winit `EventLoopProxy` which queues the event for
/// the main thread.
pub fn start_watcher<F>(callback: F)
where
    F: Fn(HotplugEvent) + Send + 'static,
{
    let result = thread::Builder::new()
        .name("lightguide-hotplug".into())
        .spawn(move || run_watcher(callback));

    if let Err(e) = result {
        log::warn!("lightguide hotplug: failed to spawn watcher thread: {e:?}");
    }
}

fn run_watcher<F>(callback: F)
where
    F: Fn(HotplugEvent),
{
    let mut api = match HidApi::new() {
        Ok(a) => a,
        Err(e) => {
            log::warn!("lightguide hotplug: HidApi::new failed on watcher thread: {e:?}");
            return;
        }
    };

    let mut last_path: Option<CString> = current_kk_mk2_path(&api);
    log::debug!(
        "lightguide hotplug: initial state — kk_mk2 {}",
        if last_path.is_some() { "present" } else { "absent" }
    );

    loop {
        thread::sleep(POLL_INTERVAL);

        if let Err(e) = api.refresh_devices() {
            log::warn!("lightguide hotplug: refresh_devices failed: {e:?}");
            continue;
        }

        let current_path = current_kk_mk2_path(&api);

        if current_path == last_path {
            continue;
        }

        match (&last_path, &current_path) {
            (None, Some(p)) => {
                log::info!("lightguide hotplug: KK MK2 connected");
                callback(HotplugEvent::KkMk2Connected { path: p.clone() });
            }
            (Some(_), None) => {
                log::info!("lightguide hotplug: KK MK2 disconnected");
                callback(HotplugEvent::KkMk2Disconnected);
            }
            (Some(_), Some(p)) => {
                // Path changed — probably a reconnect that landed on a
                // different USB topology slot. Fire disconnect + connect
                // so downstream consumers can rebuild their handle.
                log::info!("lightguide hotplug: KK MK2 reconnected on different path");
                callback(HotplugEvent::KkMk2Disconnected);
                callback(HotplugEvent::KkMk2Connected { path: p.clone() });
            }
            (None, None) => unreachable!("inequality but both None"),
        }

        last_path = current_path;
    }
}

fn current_kk_mk2_path(api: &HidApi) -> Option<CString> {
    api.device_list()
        .find(|d| {
            d.vendor_id() == kk_mk2::VID && kk_mk2::MK2_PIDS.contains(&d.product_id())
        })
        .map(|d| d.path().to_owned())
}
