//! Lightguide output backend.
//!
//! Translates Neothesia's per-event `midi_event(channel, MidiMessage)` stream
//! into per-key lighting commands on a `lightguide::Driver`. Mirrors the shape
//! of `midi_backend.rs` so it slots into the existing `OutputManager`
//! abstraction the same way the external-MIDI-out backend does.
//!
//! For Phase 3b: hardcoded device (KK MK2 via `lightguide::drivers::kk_mk2`),
//! hardcoded `track_color_id → palette` mapping, hardcoded S61 MK2 MIDI-note
//! offset. Phase 4 will add device autodetection across MK2 variants and a
//! user-configurable color schema.
//!
//! ### Notes-active state and `stop_all`
//!
//! The backend tracks every (channel, note) pair it has lit. On `stop_all`,
//! the driver's `set_off()` clears all keys in a single wire write — cleaner
//! than per-note `set_one(_, OFF)` calls and resilient to the file emitting
//! `NoteOff` events without matching `NoteOn` (Opus's flagged stuck-light
//! failure mode).
//!
//! ### Channel-as-track-color-id assumption
//!
//! Hand-color distinction reuses Neothesia's `track_color_id`. When the user
//! has `separate_channels = true` in config, `MidiPlayer` rewrites the channel
//! field to `event.track_color_id` before dispatching to outputs (see
//! `midi_player.rs`). With `separate_channels = false`, all events arrive with
//! the original MIDI channel and hand colors collapse to a single hue. This is
//! a known Phase 3b limitation — Phase 4 will surface a "lighting: use track
//! color always" override.

use std::{cell::RefCell, collections::HashSet, rc::Rc};

use midi_file::midly::{MidiMessage, num::u4};

use lightguide::Driver;
use lightguide::drivers::kk_mk2::{KkMk2, palette};

use crate::output_manager::OutputDescriptor;

/// Track-color-id → KK MK2 palette index.
///
/// First two entries are the Phase-2 hardware-validated values (light green +
/// SynthesiaKontrol-canonical blue) that won't surprise on Will's specific
/// firmware. Entries 2-5 are from the SynthesiaKontrol palette doc but their
/// on-device appearance is firmware-dependent — Phase 4 will calibrate.
const COLORS: &[u8] = &[
    0x19, // track 0: LIGHT GREEN  (Phase-2 hardware-validated)
    0x2d, // track 1: BLUE         (SynthesiaKontrol-canonical left-hand)
    0x1d, // track 2: GREEN saturated
    0x09, // track 3: ORANGE
    0x11, // track 4: YELLOW
    0x32, // track 5: PURPLE       (Phase-2 hardware-validated)
];

fn palette_for_track(track_color_id: u8) -> u8 {
    COLORS[(track_color_id as usize) % COLORS.len()]
}

/// MIDI note number of the lowest key on an S61 MK2 (C2 = 36).
/// Phase 4 will detect MK2 variant (S49 / S88 differ).
const S61_MK2_MIDI_OFFSET: u8 = 36;
const S61_MK2_LED_COUNT: u8 = 61;

/// Translate a MIDI note number to a `set_one(key, …)` buffer position.
/// Returns `None` if the note falls outside the physical keyboard range.
fn midi_note_to_key(note: u8) -> Option<u8> {
    let key = note.checked_sub(S61_MK2_MIDI_OFFSET)?;
    if key >= S61_MK2_LED_COUNT {
        None
    } else {
        Some(key)
    }
}

struct LightguideOutputConnectionInner {
    driver: Box<dyn Driver>,
    /// All (channel, midi_note) pairs currently lit. Used to bound `stop_all`
    /// behavior and to reason about Drop ordering.
    active_notes: HashSet<(u8, u8)>,
}

#[derive(Clone)]
pub struct LightguideOutputConnection {
    inner: Rc<RefCell<LightguideOutputConnectionInner>>,
}

impl LightguideOutputConnection {
    pub fn new(driver: Box<dyn Driver>) -> Self {
        Self {
            inner: Rc::new(RefCell::new(LightguideOutputConnectionInner {
                driver,
                active_notes: Default::default(),
            })),
        }
    }

    pub fn midi_event(&self, channel: u4, message: MidiMessage) {
        let inner = &mut *self.inner.borrow_mut();
        let ch = channel.as_int();

        match message {
            MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                let note = key.as_int();
                let Some(buf_key) = midi_note_to_key(note) else { return };
                let color = palette_for_track(ch);
                if let Err(e) = inner.driver.set_one(buf_key, color) {
                    log::warn!("lightguide: set_one note-on failed: {e:?}");
                    return;
                }
                inner.active_notes.insert((ch, note));
            }
            // velocity-0 NoteOn is semantically a NoteOff
            MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                let note = key.as_int();
                let Some(buf_key) = midi_note_to_key(note) else { return };
                if let Err(e) = inner.driver.set_one(buf_key, palette::OFF) {
                    log::warn!("lightguide: set_one note-off failed: {e:?}");
                    return;
                }
                inner.active_notes.remove(&(ch, note));
            }
            // Aftertouch, Controller, ProgramChange, etc. — no lighting impact.
            _ => {}
        }
    }

    pub fn stop_all(&self) {
        let inner = &mut *self.inner.borrow_mut();
        // Single full-clear is cheaper and more resilient than per-note
        // off-events. Also handles the case where the file dropped NoteOffs.
        if let Err(e) = inner.driver.set_off() {
            log::warn!("lightguide: set_off on stop_all failed: {e:?}");
        }
        inner.active_notes.clear();
    }
}

impl Drop for LightguideOutputConnection {
    fn drop(&mut self) {
        // Mirror MidiOutputConnection's discipline — explicit cleanup before
        // the inner handle releases. The Rc means the inner only drops when
        // the last clone drops, at which point this Drop fires and clears the
        // device state before the HID handle inside the Driver is closed.
        self.stop_all();
    }
}

pub struct LightguideBackend {
    // Phase 3b: stateless backend. Discovery happens at output enumeration time.
    // Phase 4 will track connected devices across hot-plug events.
    _private: (),
}

impl LightguideBackend {
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Lightguide does NOT appear in the user-facing output picker for Phase 3b.
    /// The device is auto-attached in parallel to whatever primary output the
    /// user has selected (see `OutputManager::open_lightguide_connection`).
    ///
    /// Rationale: `KkMk2::open` calls `HidApi::new`, which on macOS dispatches
    /// CFRunLoop notifications during device enumeration. When called from
    /// inside winit's event handler (e.g. per-frame `MenuScene::update` →
    /// `OutputManager::outputs`) those callbacks reentrantly re-enter winit's
    /// dispatch and panic with "tried to handle event while another event is
    /// currently being handled" (observed 2026-05-25 during E2E test).
    ///
    /// Phase 4 will add a UI entry once we can do HID enumeration off the
    /// main thread / out of the event handler context.
    pub fn get_outputs(&self) -> Vec<OutputDescriptor> {
        Vec::new()
    }

    /// Open a fresh KK MK2 driver, init the device, and wrap it in an
    /// OutputConnection-compatible adapter. Returns None on any failure.
    pub fn new_output_connection() -> Option<LightguideOutputConnection> {
        let mut driver = match KkMk2::open() {
            Ok(d) => d,
            Err(e) => {
                log::warn!("lightguide: KK MK2 open failed: {e:?}");
                return None;
            }
        };
        if let Err(e) = driver.init() {
            log::warn!("lightguide: KK MK2 init failed: {e:?}");
            return None;
        }
        Some(LightguideOutputConnection::new(Box::new(driver)))
    }
}

impl Default for LightguideBackend {
    fn default() -> Self {
        Self::new()
    }
}
