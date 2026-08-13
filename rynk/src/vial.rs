//! Host-side Vial/Via protocol engine.
//!
//! Speaks the firmware's `VialService` wire format (`rmk/src/host/via/`): one
//! 32-byte request report, one 32-byte response report, strict lockstep. The
//! typed operations mirror that service's handlers byte-for-byte and reuse the
//! shared [`vial_keycode`](rmk_types::protocol::vial_keycode) mapping, so a
//! value written here reads back exactly as the firmware stores it.
//!
//! Transport contract is the Rynk one ([`crate`] docs): reads return arbitrary
//! chunk boundaries and `Ok(0)` only at EOF; reads and writes are cancel-safe.
//! Reports are fixed-size, so the reader reassembles by counting bytes.
//!
//! Concurrency mirrors `rynk-wasm`'s driver-lock pattern: every in-flight call
//! races its own future against the reader lock, and the lock winner pumps
//! reports for whoever waits. [`VialSession::run_until_disconnect`] parks a
//! caller on that pump so link death surfaces somewhere even while no request
//! is in flight — the wasm client's `next_topic` sits on it.

use alloc::vec::Vec;
use core::pin::pin;
use core::sync::atomic::{AtomicBool, Ordering};

use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use rmk_types::action::{EncoderAction, KeyAction};
use rmk_types::combo::Combo;
use rmk_types::morse::{DOUBLE_TAP, HOLD, HOLD_AFTER_TAP, Morse, MorseProfile, TAP};
use rmk_types::protocol::rynk::RynkError;
use rmk_types::protocol::vial::{VIAL_EP_SIZE, ViaCommand, ViaKeyboardInfo, VialCommand, VialDynamic};
use rmk_types::protocol::vial_keycode::{from_via_keycode, to_via_keycode};

use crate::RynkHostError;
use crate::io::{Read, Write};

type CS = CriticalSectionRawMutex;

/// Every Vial exchange is one fixed-size report each way.
pub const REPORT_SIZE: usize = 32;
pub type Report = [u8; REPORT_SIZE];

/// Trigger slots in a Vial combo entry. This is the *wire* width — the
/// firmware's `COMBO_MAX_LENGTH` with its default value, which the Vial GUI
/// hardcodes — not this build's `COMBO_MAX_LENGTH`, which floats with
/// `keyboard.toml`.
const VIAL_COMBO_KEYS: usize = 4;

/// Largest payload a keymap/macro buffer command carries (`size <= 28`
/// upstream; the 4 header bytes fill the report).
const BUFFER_CHUNK: usize = 28;

/// The four fixed tap-dance patterns of a Vial entry, in wire order.
const VIAL_MORSE_PATTERNS: [rmk_types::morse::MorsePattern; 4] = [TAP, HOLD, DOUBLE_TAP, HOLD_AFTER_TAP];

fn report(bytes: &[u8]) -> Report {
    let mut out = [0u8; REPORT_SIZE];
    out[..bytes.len()].copy_from_slice(bytes);
    out
}

fn rejected(code: RynkError) -> RynkHostError {
    RynkHostError::Rejected(code)
}

/// Incremental report reassembly. Partial fill survives a cancelled pump —
/// the next lock holder resumes mid-report instead of desyncing the stream.
struct Rx<R> {
    inner: R,
    buf: Report,
    filled: usize,
}

pub struct VialSession<R, W> {
    reader: Mutex<CS, Rx<R>>,
    writer: Mutex<CS, W>,
    /// Vial is strict request→response; this serializes requests.
    lockstep: Mutex<CS, ()>,
    /// Reply hand-off from the pump to the one awaiting request.
    /// `Err(())` is link death.
    reply: Signal<CS, Result<Report, ()>>,
    /// A request is parked on `reply`; anything received otherwise is a stale
    /// report from a desynced earlier session and is dropped.
    awaiting: AtomicBool,
    dead: AtomicBool,
}

impl<R: Read, W: Write> VialSession<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader: Mutex::new(Rx {
                inner: reader,
                buf: [0; REPORT_SIZE],
                filled: 0,
            }),
            writer: Mutex::new(writer),
            lockstep: Mutex::new(()),
            reply: Signal::new(),
            awaiting: AtomicBool::new(false),
            dead: AtomicBool::new(false),
        }
    }

    fn die(&self) -> RynkHostError {
        self.dead.store(true, Ordering::Release);
        self.reply.signal(Err(()));
        RynkHostError::Disconnected
    }

    /// Read reports and route them until the link dies. Runs under the reader
    /// lock; only returns on link death.
    async fn pump(&self, rx: &mut Rx<R>) -> RynkHostError {
        loop {
            while rx.filled < REPORT_SIZE {
                match rx.inner.read(&mut rx.buf[rx.filled..]).await {
                    Ok(0) | Err(_) => return self.die(),
                    Ok(n) => rx.filled += n,
                }
            }
            let received = rx.buf;
            rx.filled = 0;
            if self.awaiting.load(Ordering::Acquire) {
                self.reply.signal(Ok(received));
            }
        }
    }

    /// Park on the pump until the link dies. The wasm client's `next_topic`
    /// loop sits here so a dead link is noticed while no request is running.
    pub async fn run_until_disconnect(&self) -> RynkHostError {
        if self.dead.load(Ordering::Acquire) {
            return RynkHostError::Disconnected;
        }
        let mut rx = self.reader.lock().await;
        self.pump(&mut rx).await
    }

    /// Run one future full-duplex with the pump: whoever wins the reader lock
    /// pumps for everyone parked (the `rynk-wasm` drive pattern).
    async fn drive<T>(&self, fut: impl Future<Output = Result<T, RynkHostError>>) -> Result<T, RynkHostError> {
        let mut fut = pin!(fut);
        match select(self.reader.lock(), &mut fut).await {
            Either::Second(result) => result,
            Either::First(mut rx) => match select(self.pump(&mut rx), &mut fut).await {
                Either::First(err) => Err(err),
                Either::Second(result) => result,
            },
        }
    }

    /// One lockstep exchange: send `request`, hand back the matching response.
    pub async fn exchange(&self, request: Report) -> Result<Report, RynkHostError> {
        let _turn = self.lockstep.lock().await;
        if self.dead.load(Ordering::Acquire) {
            return Err(RynkHostError::Disconnected);
        }
        self.reply.reset();
        self.awaiting.store(true, Ordering::Release);
        let sent = { self.writer.lock().await.write_all(&request).await };
        let result = match sent {
            Err(_) => Err(self.die()),
            Ok(()) => {
                self.drive(async {
                    loop {
                        match self.reply.wait().await {
                            Err(()) => break Err(RynkHostError::Disconnected),
                            // The service seeds the response with the request, so
                            // Via-level replies echo the command byte (`Unhandled`
                            // 0xFF is the reject marker). A mismatch is a stale
                            // report from a desynced session — drop it and keep
                            // waiting. Vial subcommand replies overwrite byte 0,
                            // so they can't be validated this way.
                            Ok(rep)
                                if request[0] != ViaCommand::Vial as u8
                                    && rep[0] != request[0]
                                    && rep[0] != ViaCommand::Unhandled as u8 => {}
                            Ok(rep) => break Ok(rep),
                        }
                    }
                })
                .await
            }
        };
        self.awaiting.store(false, Ordering::Release);
        result
    }

    /// Fire-and-forget: the device resets before it can answer, so nothing is
    /// awaited. A reply that does arrive is dropped as stale.
    pub async fn send_only(&self, request: Report) -> Result<(), RynkHostError> {
        let _turn = self.lockstep.lock().await;
        if self.dead.load(Ordering::Acquire) {
            return Err(RynkHostError::Disconnected);
        }
        let sent = { self.writer.lock().await.write_all(&request).await };
        sent.map_err(|_| self.die())
    }

    // -- handshake / identity --

    pub async fn via_protocol_version(&self) -> Result<u16, RynkHostError> {
        let rep = self.exchange(report(&[ViaCommand::GetProtocolVersion as u8])).await?;
        Ok(u16::from_be_bytes([rep[1], rep[2]]))
    }

    /// Vial protocol version + keyboard uid. A relay with no keyboard behind it
    /// answers the unhandled-command echo, which surfaces as `NotReady`.
    pub async fn vial_keyboard_id(&self) -> Result<(u32, [u8; 8]), RynkHostError> {
        let rep = self
            .exchange(report(&[ViaCommand::Vial as u8, VialCommand::GetKeyboardId as u8]))
            .await?;
        if rep[0] == ViaCommand::Unhandled as u8 {
            return Err(rejected(RynkError::NotReady));
        }
        let version = u32::from_le_bytes([rep[0], rep[1], rep[2], rep[3]]);
        let mut uid = [0u8; 8];
        uid.copy_from_slice(&rep[4..12]);
        Ok((version, uid))
    }

    /// The xz-compressed `vial.json`, reassembled from its 32-byte pages.
    pub async fn keyboard_def(&self) -> Result<Vec<u8>, RynkHostError> {
        let rep = self
            .exchange(report(&[ViaCommand::Vial as u8, VialCommand::GetSize as u8]))
            .await?;
        let size = u32::from_le_bytes([rep[0], rep[1], rep[2], rep[3]]) as usize;
        // A megabyte of definition is not a keyboard; don't loop on garbage.
        if size == 0 || size > 1 << 20 {
            return Err(RynkHostError::Transport(
                "vial",
                alloc::format!("definition size {size}"),
            ));
        }
        let mut def = Vec::with_capacity(size);
        let mut page: u16 = 0;
        while def.len() < size {
            let rep = self
                .exchange(report(&[
                    ViaCommand::Vial as u8,
                    VialCommand::GetKeyboardDef as u8,
                    page as u8,
                    (page >> 8) as u8,
                ]))
                .await?;
            let take = VIAL_EP_SIZE.min(size - def.len());
            def.extend_from_slice(&rep[..take]);
            page += 1;
        }
        Ok(def)
    }

    pub async fn layer_count(&self) -> Result<u8, RynkHostError> {
        let rep = self
            .exchange(report(&[ViaCommand::DynamicKeymapGetLayerCount as u8]))
            .await?;
        Ok(rep[1])
    }

    /// (tap dance entries, combo entries, key override entries).
    pub async fn dynamic_entry_counts(&self) -> Result<(u8, u8, u8), RynkHostError> {
        let rep = self
            .exchange(report(&[
                ViaCommand::Vial as u8,
                VialCommand::DynamicEntryOp as u8,
                VialDynamic::DynamicVialGetNumberOfEntries as u8,
            ]))
            .await?;
        Ok((rep[0], rep[1], rep[2]))
    }

    // -- keymap --

    /// One `DynamicKeymapGetBuffer` page. `offset`/`len` are in bytes over the
    /// flat big-endian u16 keymap (layer-major, then row, then column).
    pub async fn keymap_page(&self, offset: u16, len: u8) -> Result<Vec<KeyAction>, RynkHostError> {
        let rep = self
            .exchange(report(&[
                ViaCommand::DynamicKeymapGetBuffer as u8,
                (offset >> 8) as u8,
                offset as u8,
                len,
            ]))
            .await?;
        Ok((0..len as usize / 2)
            .map(|i| from_via_keycode(u16::from_be_bytes([rep[4 + 2 * i], rep[5 + 2 * i]])))
            .collect())
    }

    pub async fn read_all_keymap(&self, layers: u8, rows: u8, cols: u8) -> Result<Vec<KeyAction>, RynkHostError> {
        let total = layers as usize * rows as usize * cols as usize * 2;
        let mut actions = Vec::with_capacity(total / 2);
        let mut offset = 0usize;
        while offset < total {
            let len = BUFFER_CHUNK.min(total - offset) as u8;
            actions.extend(self.keymap_page(offset as u16, len).await?);
            offset += len as usize;
        }
        Ok(actions)
    }

    pub async fn set_keycode(&self, layer: u8, row: u8, col: u8, action: KeyAction) -> Result<(), RynkHostError> {
        let keycode = to_via_keycode(action);
        self.exchange(report(&[
            ViaCommand::DynamicKeymapSetKeyCode as u8,
            layer,
            row,
            col,
            (keycode >> 8) as u8,
            keycode as u8,
        ]))
        .await?;
        Ok(())
    }

    /// Whole-keymap write, `read_all_keymap` order. Key-at-a-time on purpose:
    /// the firmware parses `DynamicKeymapSetBuffer` little-endian while stock
    /// QMK parses it big-endian, so the per-key command is the one write path
    /// every Vial keyboard agrees on.
    pub async fn write_all_keymap(&self, actions: &[KeyAction], rows: u8, cols: u8) -> Result<(), RynkHostError> {
        let per_layer = rows as usize * cols as usize;
        for (i, action) in actions.iter().enumerate() {
            let layer = (i / per_layer) as u8;
            let row = (i % per_layer / cols as usize) as u8;
            let col = (i % cols as usize) as u8;
            self.set_keycode(layer, row, col, *action).await?;
        }
        Ok(())
    }

    // -- encoders --

    pub async fn encoder(&self, encoder_id: u8, layer: u8) -> Result<EncoderAction, RynkHostError> {
        let rep = self
            .exchange(report(&[
                ViaCommand::Vial as u8,
                VialCommand::GetEncoder as u8,
                layer,
                encoder_id,
            ]))
            .await?;
        // Wire order is counter-clockwise first.
        Ok(EncoderAction {
            counter_clockwise: from_via_keycode(u16::from_be_bytes([rep[0], rep[1]])),
            clockwise: from_via_keycode(u16::from_be_bytes([rep[2], rep[3]])),
        })
    }

    pub async fn set_encoder(&self, encoder_id: u8, layer: u8, action: EncoderAction) -> Result<(), RynkHostError> {
        for (clockwise, key_action) in [(1u8, action.clockwise), (0u8, action.counter_clockwise)] {
            let keycode = to_via_keycode(key_action);
            self.exchange(report(&[
                ViaCommand::Vial as u8,
                VialCommand::SetEncoder as u8,
                layer,
                encoder_id,
                clockwise,
                (keycode >> 8) as u8,
                keycode as u8,
            ]))
            .await?;
        }
        Ok(())
    }

    // -- combos --

    pub async fn combo(&self, index: u8) -> Result<Combo, RynkHostError> {
        let rep = self
            .exchange(report(&[
                ViaCommand::Vial as u8,
                VialCommand::DynamicEntryOp as u8,
                VialDynamic::DynamicVialComboGet as u8,
                index,
            ]))
            .await?;
        let actions = (0..VIAL_COMBO_KEYS)
            .map(|i| from_via_keycode(u16::from_le_bytes([rep[1 + 2 * i], rep[2 + 2 * i]])))
            .filter(|action| !action.is_empty());
        let output = from_via_keycode(u16::from_le_bytes([
            rep[1 + 2 * VIAL_COMBO_KEYS],
            rep[2 + 2 * VIAL_COMBO_KEYS],
        ]));
        Ok(Combo::new(actions, output, None))
    }

    pub async fn set_combo(&self, index: u8, combo: &Combo) -> Result<(), RynkHostError> {
        // The wire has no layer field and exactly four trigger slots.
        if combo.layer.is_some() || combo.actions.len() > VIAL_COMBO_KEYS {
            return Err(rejected(RynkError::Invalid));
        }
        let mut req = report(&[
            ViaCommand::Vial as u8,
            VialCommand::DynamicEntryOp as u8,
            VialDynamic::DynamicVialComboSet as u8,
            index,
        ]);
        for (i, action) in combo.actions.iter().enumerate() {
            let keycode = to_via_keycode(*action);
            req[4 + 2 * i..6 + 2 * i].copy_from_slice(&keycode.to_le_bytes());
        }
        let output = to_via_keycode(combo.output);
        req[4 + 2 * VIAL_COMBO_KEYS..6 + 2 * VIAL_COMBO_KEYS].copy_from_slice(&output.to_le_bytes());
        self.exchange(req).await?;
        Ok(())
    }

    // -- morses (Vial tap dance) --

    pub async fn morse(&self, index: u8) -> Result<Morse, RynkHostError> {
        let rep = self
            .exchange(report(&[
                ViaCommand::Vial as u8,
                VialCommand::DynamicEntryOp as u8,
                VialDynamic::DynamicVialMorseGet as u8,
                index,
            ]))
            .await?;
        let action = |off: usize| from_via_keycode(u16::from_le_bytes([rep[off], rep[off + 1]])).to_action();
        let timeout = u16::from_le_bytes([rep[9], rep[10]]);
        // The service writes hold and gap timeout from the same wire field.
        let profile = MorseProfile::const_default()
            .with_hold_timeout_ms(Some(timeout))
            .with_gap_timeout_ms(Some(timeout));
        Ok(Morse::new_from_vial(
            action(1),
            action(3),
            action(7),
            action(5),
            profile,
        ))
    }

    pub async fn set_morse(&self, index: u8, morse: &Morse) -> Result<(), RynkHostError> {
        // The wire carries exactly the four fixed patterns.
        if morse
            .actions
            .iter()
            .any(|(pattern, _)| !VIAL_MORSE_PATTERNS.contains(pattern))
        {
            return Err(rejected(RynkError::Invalid));
        }
        let mut req = report(&[
            ViaCommand::Vial as u8,
            VialCommand::DynamicEntryOp as u8,
            VialDynamic::DynamicVialMorseSet as u8,
            index,
        ]);
        for (i, pattern) in VIAL_MORSE_PATTERNS.iter().enumerate() {
            let keycode = morse
                .get(*pattern)
                .map_or(0, |action| to_via_keycode(KeyAction::Single(action)));
            req[4 + 2 * i..6 + 2 * i].copy_from_slice(&keycode.to_le_bytes());
        }
        let timeout = morse.profile.hold_timeout_ms().unwrap_or(250);
        req[12..14].copy_from_slice(&timeout.to_le_bytes());
        self.exchange(req).await?;
        Ok(())
    }

    // -- behavior settings --

    /// Setting keys the device supports, paged until the 0xFFFF terminator.
    pub async fn behavior_setting_keys(&self) -> Result<Vec<u16>, RynkHostError> {
        let mut keys = Vec::new();
        loop {
            let offset = keys.len() as u16;
            let rep = self
                .exchange(report(&[
                    ViaCommand::Vial as u8,
                    VialCommand::BehaviorSettingQuery as u8,
                    offset as u8,
                    (offset >> 8) as u8,
                ]))
                .await?;
            let mut terminated = false;
            for pair in rep.chunks_exact(2) {
                let key = u16::from_le_bytes([pair[0], pair[1]]);
                if key == 0xFFFF {
                    terminated = true;
                    break;
                }
                keys.push(key);
            }
            // A full page might continue; a bounded id space keeps a
            // terminator-less device from looping us forever.
            if terminated || keys.len() >= 256 {
                return Ok(keys);
            }
        }
    }

    pub async fn behavior_setting(&self, key: u16) -> Result<Option<u16>, RynkHostError> {
        let rep = self
            .exchange(report(&[
                ViaCommand::Vial as u8,
                VialCommand::GetBehaviorSetting as u8,
                key as u8,
                (key >> 8) as u8,
            ]))
            .await?;
        if rep[0] != 0 {
            return Ok(None);
        }
        Ok(Some(u16::from_le_bytes([rep[1], rep[2]])))
    }

    pub async fn set_behavior_setting(&self, key: u16, value: u16) -> Result<(), RynkHostError> {
        self.exchange(report(&[
            ViaCommand::Vial as u8,
            VialCommand::SetBehaviorSetting as u8,
            key as u8,
            (key >> 8) as u8,
            value as u8,
            (value >> 8) as u8,
        ]))
        .await?;
        Ok(())
    }

    // -- lock gate --

    /// `(unlocked, unlock in progress, challenge key positions)`.
    #[allow(clippy::type_complexity)]
    pub async fn unlock_status(&self) -> Result<(bool, bool, Vec<(u8, u8)>), RynkHostError> {
        let rep = self
            .exchange(report(&[ViaCommand::Vial as u8, VialCommand::GetUnlockStatus as u8]))
            .await?;
        let mut keys = Vec::new();
        // Pairs from byte 2 up to the 0xFF fill.
        for pair in rep[2..].chunks_exact(2) {
            if pair[0] == 0xFF {
                break;
            }
            keys.push((pair[0], pair[1]));
        }
        Ok((rep[0] == 1, rep[1] == 1, keys))
    }

    pub async fn unlock_start(&self) -> Result<(), RynkHostError> {
        self.exchange(report(&[ViaCommand::Vial as u8, VialCommand::UnlockStart as u8]))
            .await?;
        Ok(())
    }

    /// `(unlocked, unlock in progress, challenge keys still up)`.
    pub async fn unlock_poll(&self) -> Result<(bool, bool, u8), RynkHostError> {
        let rep = self
            .exchange(report(&[ViaCommand::Vial as u8, VialCommand::UnlockPoll as u8]))
            .await?;
        Ok((rep[0] == 1, rep[1] == 1, rep[2]))
    }

    pub async fn lock(&self) -> Result<(), RynkHostError> {
        self.exchange(report(&[ViaCommand::Vial as u8, VialCommand::Lock as u8]))
            .await?;
        Ok(())
    }

    // -- status --

    /// Pressed-key bitmap in Rynk order: row-major, `ceil(cols / 8)` bytes per
    /// row, bit 0 of a row's first byte = column 0. The Vial wire carries each
    /// row's bytes big-endian, so they get reversed here. Rows past what fits
    /// in one report read as released. Unlock-gated upstream: while locked the
    /// reply is an echo, which decodes as all-released.
    pub async fn matrix_state(&self, rows: u8, cols: u8) -> Result<Vec<u8>, RynkHostError> {
        let rep = self
            .exchange(report(&[
                ViaCommand::GetKeyboardValue as u8,
                ViaKeyboardInfo::SwitchMatrixState as u8,
            ]))
            .await?;
        let row_len = (cols as usize).div_ceil(8);
        let total = rows as usize * row_len;
        let mut bitmap = alloc::vec![0u8; total];
        let available = total.min(REPORT_SIZE - 2);
        bitmap[..available].copy_from_slice(&rep[2..2 + available]);
        if row_len > 1 {
            for row in bitmap[..available].chunks_mut(row_len) {
                row.reverse();
            }
        }
        Ok(bitmap)
    }

    // -- macros --

    pub async fn macro_count(&self) -> Result<u8, RynkHostError> {
        let rep = self
            .exchange(report(&[ViaCommand::DynamicKeymapMacroGetCount as u8]))
            .await?;
        Ok(rep[1])
    }

    pub async fn macro_buffer_size(&self) -> Result<u16, RynkHostError> {
        let rep = self
            .exchange(report(&[ViaCommand::DynamicKeymapMacroGetBufferSize as u8]))
            .await?;
        Ok(u16::from_be_bytes([rep[1], rep[2]]))
    }

    pub async fn macro_buffer_page(&self, offset: u16, len: u8) -> Result<Vec<u8>, RynkHostError> {
        if len as usize > BUFFER_CHUNK {
            return Err(rejected(RynkError::Invalid));
        }
        let rep = self
            .exchange(report(&[
                ViaCommand::DynamicKeymapMacroGetBuffer as u8,
                (offset >> 8) as u8,
                offset as u8,
                len,
            ]))
            .await?;
        if rep[0] == 0xFF {
            return Err(rejected(RynkError::Invalid));
        }
        Ok(rep[4..4 + len as usize].to_vec())
    }

    pub async fn write_macro_buffer(&self, offset: u16, data: &[u8]) -> Result<(), RynkHostError> {
        if data.len() > BUFFER_CHUNK {
            return Err(rejected(RynkError::Invalid));
        }
        let mut req = report(&[
            ViaCommand::DynamicKeymapMacroSetBuffer as u8,
            (offset >> 8) as u8,
            offset as u8,
            data.len() as u8,
        ]);
        req[4..4 + data.len()].copy_from_slice(data);
        let rep = self.exchange(req).await?;
        if rep[0] == 0xFF {
            return Err(rejected(RynkError::Invalid));
        }
        Ok(())
    }

    // -- system --

    pub async fn eeprom_reset(&self) -> Result<(), RynkHostError> {
        self.exchange(report(&[ViaCommand::EepromReset as u8])).await?;
        Ok(())
    }

    /// Fire-and-forget: the device jumps before it can answer.
    pub async fn bootloader_jump(&self) -> Result<(), RynkHostError> {
        self.send_only(report(&[ViaCommand::BootloaderJump as u8])).await
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use embassy_futures::block_on;
    use embassy_futures::select::{Either, select};
    use rmk_types::action::Action;
    use rmk_types::keycode::{HidKeyCode, KeyCode};

    use super::*;
    use crate::io::{ErrorKind, ErrorType};

    /// Scripted device: each expected request is answered with its canned
    /// replies, queued into the read side the moment the request lands.
    #[derive(Default)]
    struct Fake {
        script: VecDeque<(Report, Vec<Report>)>,
        rx: VecDeque<u8>,
        closed: bool,
    }

    struct FakeReader(Rc<RefCell<Fake>>);
    struct FakeWriter(Rc<RefCell<Fake>>);

    impl ErrorType for FakeReader {
        type Error = ErrorKind;
    }
    impl ErrorType for FakeWriter {
        type Error = ErrorKind;
    }

    impl Read for FakeReader {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, ErrorKind> {
            // Empty-but-open yields, so a parked pump lets the racing request
            // run and queue its reply — the lockstep flow of a real device.
            loop {
                {
                    let mut fake = self.0.borrow_mut();
                    if !fake.rx.is_empty() {
                        let n = buf.len().min(fake.rx.len());
                        for slot in buf.iter_mut().take(n) {
                            *slot = fake.rx.pop_front().unwrap();
                        }
                        return Ok(n);
                    }
                    if fake.closed {
                        return Ok(0);
                    }
                }
                embassy_futures::yield_now().await;
            }
        }
    }

    impl Write for FakeWriter {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, ErrorKind> {
            let mut fake = self.0.borrow_mut();
            let (expected, replies) = fake.script.pop_front().expect("unexpected request");
            assert_eq!(buf, expected, "request bytes");
            for reply in replies {
                fake.rx.extend(reply);
            }
            Ok(buf.len())
        }

        async fn flush(&mut self) -> Result<(), ErrorKind> {
            Ok(())
        }
    }

    fn session(fake: &Rc<RefCell<Fake>>) -> VialSession<FakeReader, FakeWriter> {
        VialSession::new(FakeReader(fake.clone()), FakeWriter(fake.clone()))
    }

    fn expect(fake: &Rc<RefCell<Fake>>, request: &[u8], replies: &[Report]) {
        fake.borrow_mut().script.push_back((report(request), replies.to_vec()));
    }

    #[test]
    fn via_version_parses_big_endian() {
        let fake = Rc::new(RefCell::new(Fake::default()));
        let mut reply = report(&[ViaCommand::GetProtocolVersion as u8]);
        reply[1..3].copy_from_slice(&0x0009u16.to_be_bytes());
        expect(&fake, &[0x01], &[reply]);
        let s = session(&fake);
        assert_eq!(block_on(s.via_protocol_version()).unwrap(), 9);
    }

    #[test]
    fn keyboard_id_parses_and_relay_echo_is_not_ready() {
        let fake = Rc::new(RefCell::new(Fake::default()));
        let mut reply = [0u8; 32];
        reply[0..4].copy_from_slice(&6u32.to_le_bytes());
        reply[4..12].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        expect(&fake, &[0xFE, 0x00], &[reply]);
        let s = session(&fake);
        assert_eq!(block_on(s.vial_keyboard_id()).unwrap(), (6, [1, 2, 3, 4, 5, 6, 7, 8]));

        // Dongle with no keyboard: unhandled echo, 0xFF in byte 0.
        let mut echo = report(&[0xFE, 0x00]);
        echo[0] = 0xFF;
        expect(&fake, &[0xFE, 0x00], &[echo]);
        let s = session(&fake);
        assert!(matches!(
            block_on(s.vial_keyboard_id()),
            Err(RynkHostError::Rejected(RynkError::NotReady))
        ));
    }

    #[test]
    fn keyboard_def_reassembles_pages() {
        let fake = Rc::new(RefCell::new(Fake::default()));
        let def: Vec<u8> = (0u8..40).collect();
        let mut size_reply = [0u8; 32];
        size_reply[0..4].copy_from_slice(&40u32.to_le_bytes());
        expect(&fake, &[0xFE, 0x01], &[size_reply]);
        let mut page0 = [0u8; 32];
        page0.copy_from_slice(&def[..32]);
        expect(&fake, &[0xFE, 0x02, 0, 0], &[page0]);
        let mut page1 = [0u8; 32];
        page1[..8].copy_from_slice(&def[32..]);
        expect(&fake, &[0xFE, 0x02, 1, 0], &[page1]);
        let s = session(&fake);
        assert_eq!(block_on(s.keyboard_def()).unwrap(), def);
    }

    #[test]
    fn keymap_reads_big_endian_and_writes_per_key() {
        let fake = Rc::new(RefCell::new(Fake::default()));
        // 1 layer × 1 row × 2 cols → one 4-byte page.
        let mut reply = report(&[0x12, 0, 0, 4]);
        reply[4..6].copy_from_slice(&0x0004u16.to_be_bytes()); // A
        reply[6..8].copy_from_slice(&0x0005u16.to_be_bytes()); // B
        expect(&fake, &[0x12, 0, 0, 4], &[reply]);
        let s = session(&fake);
        let keymap = block_on(s.read_all_keymap(1, 1, 2)).unwrap();
        assert_eq!(
            keymap,
            vec![
                KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::A))),
                KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::B))),
            ]
        );

        expect(
            &fake,
            &[0x05, 0, 0, 0, 0x00, 0x04],
            &[report(&[0x05, 0, 0, 0, 0x00, 0x04])],
        );
        expect(
            &fake,
            &[0x05, 0, 0, 1, 0x00, 0x05],
            &[report(&[0x05, 0, 0, 1, 0x00, 0x05])],
        );
        let s = session(&fake);
        block_on(s.write_all_keymap(&keymap, 1, 2)).unwrap();
        assert!(fake.borrow().script.is_empty());
    }

    #[test]
    fn combo_round_trips_the_vial_wire_layout() {
        let fake = Rc::new(RefCell::new(Fake::default()));
        let mut reply = [0u8; 32];
        reply[1..3].copy_from_slice(&0x0004u16.to_le_bytes()); // A
        reply[3..5].copy_from_slice(&0x0005u16.to_le_bytes()); // B
        // slots 2, 3 empty
        reply[9..11].copy_from_slice(&0x0006u16.to_le_bytes()); // C output
        expect(&fake, &[0xFE, 0x0D, 0x03, 2], &[reply]);
        let s = session(&fake);
        let combo = block_on(s.combo(2)).unwrap();
        assert_eq!(combo.actions.len(), 2);
        assert_eq!(combo.layer, None);

        let mut set_req = report(&[0xFE, 0x0D, 0x04, 2]);
        set_req[4..6].copy_from_slice(&0x0004u16.to_le_bytes());
        set_req[6..8].copy_from_slice(&0x0005u16.to_le_bytes());
        set_req[12..14].copy_from_slice(&0x0006u16.to_le_bytes());
        expect(&fake, &set_req.clone(), &[[0u8; 32]]);
        let s = session(&fake);
        block_on(s.set_combo(2, &combo)).unwrap();
        assert!(fake.borrow().script.is_empty());
    }

    #[test]
    fn combo_with_layer_is_rejected_without_touching_the_wire() {
        let fake = Rc::new(RefCell::new(Fake::default()));
        let s = session(&fake);
        let combo = Combo::new([], KeyAction::No, Some(1));
        assert!(matches!(
            block_on(s.set_combo(0, &combo)),
            Err(RynkHostError::Rejected(RynkError::Invalid))
        ));
    }

    #[test]
    fn morse_maps_the_four_patterns_and_timeout() {
        let fake = Rc::new(RefCell::new(Fake::default()));
        let mut reply = [0u8; 32];
        reply[1..3].copy_from_slice(&0x0004u16.to_le_bytes()); // tap: A
        reply[3..5].copy_from_slice(&0x0005u16.to_le_bytes()); // hold: B
        reply[9..11].copy_from_slice(&200u16.to_le_bytes());
        expect(&fake, &[0xFE, 0x0D, 0x01, 1], &[reply]);
        let s = session(&fake);
        let morse = block_on(s.morse(1)).unwrap();
        assert_eq!(morse.get(TAP), Some(Action::Key(KeyCode::Hid(HidKeyCode::A))));
        assert_eq!(morse.get(HOLD), Some(Action::Key(KeyCode::Hid(HidKeyCode::B))));
        assert_eq!(morse.get(DOUBLE_TAP), None);
        assert_eq!(morse.profile.hold_timeout_ms(), Some(200));

        let mut set_req = report(&[0xFE, 0x0D, 0x02, 1]);
        set_req[4..6].copy_from_slice(&0x0004u16.to_le_bytes());
        set_req[6..8].copy_from_slice(&0x0005u16.to_le_bytes());
        set_req[12..14].copy_from_slice(&200u16.to_le_bytes());
        expect(&fake, &set_req.clone(), &[[0u8; 32]]);
        let s = session(&fake);
        block_on(s.set_morse(1, &morse)).unwrap();
        assert!(fake.borrow().script.is_empty());
    }

    #[test]
    fn matrix_state_reverses_row_bytes() {
        let fake = Rc::new(RefCell::new(Fake::default()));
        // 2 rows × 9 cols → 2 bytes per row, big-endian on the wire. The
        // service echoes the request header, and the echo check relies on it.
        let mut reply = report(&[0x02, 0x03]);
        reply[2..6].copy_from_slice(&[0x01, 0x80, 0x00, 0x01]);
        expect(&fake, &[0x02, 0x03], &[reply]);
        let s = session(&fake);
        let bitmap = block_on(s.matrix_state(2, 9)).unwrap();
        // Row 0: wire [0x01, 0x80] → col 0 lives in the low byte → [0x80, 0x01].
        assert_eq!(bitmap, vec![0x80, 0x01, 0x01, 0x00]);
    }

    #[test]
    fn behavior_keys_stop_at_terminator() {
        let fake = Rc::new(RefCell::new(Fake::default()));
        let mut reply = [0xFFu8; 32];
        reply[0..2].copy_from_slice(&0x0002u16.to_le_bytes());
        reply[2..4].copy_from_slice(&0x0006u16.to_le_bytes());
        expect(&fake, &[0xFE, 0x09, 0, 0], &[reply]);
        let s = session(&fake);
        assert_eq!(block_on(s.behavior_setting_keys()).unwrap(), vec![0x0002, 0x0006]);
    }

    #[test]
    fn stale_report_with_wrong_echo_is_skipped() {
        let fake = Rc::new(RefCell::new(Fake::default()));
        let stale = report(&[0x77, 1, 2, 3]);
        let mut good = report(&[ViaCommand::DynamicKeymapGetLayerCount as u8]);
        good[1] = 4;
        expect(&fake, &[0x11], &[stale, good]);
        let s = session(&fake);
        assert_eq!(block_on(s.layer_count()).unwrap(), 4);
    }

    #[test]
    fn eof_kills_the_session_for_good() {
        let fake = Rc::new(RefCell::new(Fake::default()));
        fake.borrow_mut().closed = true;
        expect(&fake, &[0x11], &[]);
        let s = session(&fake);
        assert!(matches!(block_on(s.layer_count()), Err(RynkHostError::Disconnected)));
        // Dead flag latched: the next call fails without a request.
        assert!(matches!(block_on(s.layer_count()), Err(RynkHostError::Disconnected)));
    }

    #[test]
    fn parked_pump_routes_the_reply() {
        let fake = Rc::new(RefCell::new(Fake::default()));
        let mut reply = report(&[0x11]);
        reply[1] = 2;
        expect(&fake, &[0x11], &[reply]);
        let s = session(&fake);
        // run_until_disconnect wins the reader lock first; the request's reply
        // must flow through its pump.
        let result = block_on(async {
            match select(s.run_until_disconnect(), s.layer_count()).await {
                Either::First(err) => Err(err),
                Either::Second(result) => result,
            }
        });
        assert_eq!(result.unwrap(), 2);
    }
}
