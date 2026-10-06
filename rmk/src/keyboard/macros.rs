//! Macro playback and, with a host, the buffer the macros live in.
//!
//! Every macro sits in one `MACRO_SPACE_SIZE`-byte buffer, slot after slot, in the
//! encoding of the host that edits it: Vial's own with `vial`, so Vial reads and
//! writes the buffer as it is, otherwise the postcard encoding of the ops, which
//! is what Rynk carries. A keyboard without a host has no buffer and plays the
//! compiled-in defaults from rodata; with `vial` the defaults stay there too, as
//! Vial's encoding cannot spell every op, and a slot still equal to its default's
//! rendering plays the default.
//!
//! Playback never copies a macro: a queued [`Cursor`] walks its source, a
//! default's op slice or a byte range of the buffer, one op per fire.

#[cfg(feature = "host")]
use core::ops::Range;

use heapless::Deque;
#[cfg(feature = "host")]
use rmk_types::constants::MACRO_CHUNK_SIZE;
use rmk_types::keyboard_macros::MacroOp;

/// Where a queued macro's ops come from.
#[derive(Clone, Copy)]
enum Source {
    /// Built-in macro, defined in compile-time and compiled into the firmware.
    Default(&'static [MacroOp]),
    /// The macro from run-time buffer.
    #[cfg(feature = "host")]
    Buffer,
}

/// A macro half on its way through playback: ops `pos..end` of its source, op
/// indices for a default, byte offsets into the buffer otherwise.
#[derive(Clone, Copy)]
struct Cursor {
    source: Source,
    pos: usize,
    end: usize,
    /// A wait before the first op, the release half's 20ms.
    lead_delay: u16,
}

impl Cursor {
    /// The next op, or `None` at the end of the half.
    #[cfg_attr(not(feature = "host"), allow(unused_variables))]
    fn next(&mut self, buf: &[u8]) -> Option<MacroOp> {
        if self.lead_delay > 0 {
            return Some(MacroOp::Delay(core::mem::take(&mut self.lead_delay)));
        }
        if self.pos >= self.end {
            return None;
        }
        match self.source {
            Source::Default(ops) => {
                self.pos += 1;
                Some(ops[self.pos - 1])
            }
            #[cfg(feature = "host")]
            Source::Buffer => {
                let (op, len) = codec::decode(&buf[self.pos..self.end])?;
                self.pos += len;
                Some(op)
            }
        }
    }
}

/// The macros: the compiled-in defaults, the buffer a host edits, and the
/// halves queued to run, front first; a nested `Macro(n)` queues behind.
pub(crate) struct Macros<'a> {
    defaults: &'static [&'static [MacroOp]],
    /// Every macro's bytes, slot after slot; empty without a host.
    buf: &'a mut [u8],
    /// Whether flash has the macros yet. If not, the next save writes all of them.
    #[cfg(all(feature = "host", feature = "storage"))]
    stored: bool,
    queue: Deque<Cursor, 4>,
}

impl<'a> Macros<'a> {
    /// Fills `buf` from the defaults unless flash has the macros.
    #[cfg_attr(not(feature = "host"), allow(unused_variables))]
    pub(crate) fn new(defaults: &'static [&'static [MacroOp]], buf: &'a mut [u8], stored: bool) -> Self {
        #[cfg(feature = "host")]
        if !stored {
            encode_defaults(buf, defaults);
        }
        Self {
            defaults,
            buf,
            #[cfg(all(feature = "host", feature = "storage"))]
            stored,
            queue: Deque::new(),
        }
    }

    /// Queue the press or release half of the macro at `slot`, `false` when the
    /// queue is full.
    pub(crate) fn queue(&mut self, slot: u8, pressed: bool) -> bool {
        // Past the last slot the buffer holds whatever a longer save left behind.
        if slot as usize >= crate::MACRO_MAX_NUM {
            return true;
        }
        let buf = &*self.buf;
        // Without Vial a host's buffer is lossless, so the defaults only seed it.
        let default = self
            .defaults
            .get(slot as usize)
            .copied()
            .filter(|_| !cfg!(all(feature = "host", not(feature = "vial"))));
        #[cfg(feature = "host")]
        let range = codec::segment(buf, slot as usize);
        // With Vial a slot still equal to its default's rendering plays the default,
        // whose ops Vial could not spell included.
        #[cfg(feature = "vial")]
        let default = default.filter(|ops| codec::render(ops).eq(buf[range.clone()].iter().copied()));
        let whole = match default {
            Some(ops) => Cursor {
                source: Source::Default(ops),
                pos: 0,
                end: ops.len(),
                lead_delay: 0,
            },
            #[cfg(feature = "host")]
            None => Cursor {
                source: Source::Buffer,
                pos: range.start,
                end: range.end,
                lead_delay: 0,
            },
            #[cfg(not(feature = "host"))]
            None => return true,
        };
        // Without a `PauseForRelease` the press queues the whole macro; with one, the
        // press queues the ops before it and the release the ops after it.
        let mut scan = whole;
        let pause = loop {
            let at = scan.pos;
            match scan.next(buf) {
                Some(MacroOp::PauseForRelease) => break Some((at, scan.pos)),
                Some(_) => {}
                None => break None,
            }
        };
        let mut half = whole;
        match (pressed, pause) {
            (true, Some((at, _))) => half.end = at,
            (true, None) => {}
            // The release half starts 20ms late so it doesn't toggle the host's IME.
            (false, Some((_, after))) => {
                half.pos = after;
                half.lead_delay = 20;
            }
            (false, None) => return true,
        }
        half.pos >= half.end || self.queue.push_back(half).is_ok()
    }

    /// Pop the next queued op, dropping halves that ran out.
    pub(crate) fn next_op(&mut self) -> Option<MacroOp> {
        loop {
            if let Some(op) = self.queue.front_mut()?.next(self.buf) {
                return Some(op);
            }
            self.queue.pop_front();
        }
    }

    /// The op `next_op` would pop, without popping it.
    pub(crate) fn peek_op(&mut self) -> Option<MacroOp> {
        loop {
            let mut front = *self.queue.front()?;
            if let Some(op) = front.next(self.buf) {
                return Some(op);
            }
            self.queue.pop_front();
        }
    }

    pub(crate) fn is_playing(&self) -> bool {
        !self.queue.is_empty()
    }

    #[cfg(feature = "host")]
    pub(crate) fn bytes(&self) -> &[u8] {
        self.buf
    }

    /// Overwrite the buffer from `offset` with `data`, what falls past its end
    /// dropped, and return the chunks to persist. A change ends the macros playing.
    #[cfg(feature = "host")]
    pub(crate) fn write(&mut self, offset: usize, data: &[u8]) -> Range<usize> {
        let end = (offset + data.len()).min(self.buf.len());
        if offset >= end || self.buf[offset..end] == data[..end - offset] {
            return 0..0;
        }
        self.buf[offset..end].copy_from_slice(&data[..end - offset]);
        self.queue.clear();
        offset / MACRO_CHUNK_SIZE..end.div_ceil(MACRO_CHUNK_SIZE)
    }

    /// Empty every macro, all zero in both encodings, and return the chunks to persist.
    #[cfg(feature = "host")]
    pub(crate) fn clear(&mut self) -> Range<usize> {
        if self.buf.iter().all(|&b| b == 0) {
            return 0..0;
        }
        self.buf.fill(0);
        self.queue.clear();
        0..self.buf.len() / MACRO_CHUNK_SIZE
    }

    /// The op bytes of macro `slot`.
    #[cfg(feature = "rynk")]
    pub(crate) fn slot(&self, slot: u8) -> &[u8] {
        &self.buf[codec::segment(self.buf, slot as usize)]
    }

    /// Replace macro `slot` with the op `bytes` and return the chunks to persist,
    /// `None` when the buffer cannot hold them. A change ends the macros playing.
    #[cfg(feature = "rynk")]
    pub(crate) fn set_slot(&mut self, slot: u8, bytes: &[u8]) -> Option<Range<usize>> {
        let range = codec::splice(self.buf, slot as usize, bytes)?;
        self.queue.clear();
        Some(range.start / MACRO_CHUNK_SIZE..range.end.div_ceil(MACRO_CHUNK_SIZE))
    }
}

/// Fill `buf` with the defaults, slot after slot, the rest zero.
#[cfg(feature = "host")]
pub(crate) fn encode_defaults(buf: &mut [u8], defaults: &[&[MacroOp]]) {
    buf.fill(0);
    let mut pos = 0;
    for slot in 0..crate::MACRO_MAX_NUM {
        let ops = defaults.get(slot).copied().unwrap_or(&[]);
        pos += codec::encode_slot(&mut buf[pos..], ops);
    }
}

/// Save the macro chunks a host `changed`.
///
/// Before the first save, boot loads the default macros. After it, boot loads only
/// what is in flash, so the first save writes the whole buffer.
#[cfg(all(feature = "host", feature = "storage"))]
pub(crate) async fn persist(keymap: &crate::keymap::KeyMap<'_>, changed: Range<usize>) -> Result<(), ()> {
    let stored = keymap.macros(|m| m.stored);
    // If macros have been stored in flash, persist changed range only.
    // Otherwise persist the full macro space.
    let chunks = if stored {
        changed
    } else {
        0..crate::MACRO_SPACE_SIZE / MACRO_CHUNK_SIZE
    };
    for idx in chunks {
        let bytes = keymap.macros(|m| m.buf.as_chunks::<MACRO_CHUNK_SIZE>().0[idx]);
        // A chunk missing from flash boots as zeros, so skip empty ones. But write
        // chunk 0 anyway, or clearing every macro would bring the defaults back.
        if !stored && idx > 0 && bytes == [0; MACRO_CHUNK_SIZE] {
            continue;
        }
        crate::storage::store(crate::storage::StorageItem::MacroChunk { idx: idx as u8, bytes }).await?;
    }
    keymap.macros(|m| m.stored = true);
    Ok(())
}

/// Vial's macro encoding, the buffer's with `vial`: a text character is its
/// byte, a key step `01 kind kc` or `01 kind+4 lo hi`, a delay `01 04 lo hi`
/// and every slot ends with `0x00`.
#[cfg(feature = "vial")]
pub(crate) mod codec {
    use core::ops::Range;

    use rmk_types::action::{Action, KeyAction};
    use rmk_types::keyboard_macros::MacroOp;

    use crate::host::via::keycode_convert::{from_via_keycode, to_via_keycode};

    /// The longest delay one Vial delay op holds: two 1-based bytes.
    const VIAL_DELAY_MAX: u16 = 254 * 255 + 254;

    /// The bytes of slot `k`, its terminator excluded.
    pub(crate) fn segment(buf: &[u8], k: usize) -> Range<usize> {
        let mut start = 0;
        for _ in 0..k {
            start = buf[start..]
                .iter()
                .position(|&b| b == 0)
                .map_or(buf.len(), |p| start + p + 1);
        }
        let end = buf[start..]
            .iter()
            .position(|&b| b == 0)
            .map_or(buf.len(), |p| start + p);
        start..end
    }

    /// The first op in `bytes` and the bytes it took, skipping what types
    /// nothing and, like vial-gui, an unknown `01 xx` pair.
    pub(crate) fn decode(bytes: &[u8]) -> Option<(MacroOp, usize)> {
        let mut i = 0;
        while i < bytes.len() {
            let (op, len) = if bytes[i] == 0x01 {
                let at = |offset: usize| bytes.get(i + offset).copied();
                match at(1)? {
                    kind @ 1..=3 => (key_op(kind, at(2)? as u16), 3),
                    4 => {
                        let ms = (at(2)?.max(1) - 1) as u16 + (at(3)?.max(1) - 1) as u16 * 255;
                        (Some(MacroOp::Delay(ms)), 4)
                    }
                    kind @ 5..=7 => {
                        let raw = u16::from_le_bytes([at(2)?, at(3)?]);
                        let keycode = if raw & 0xFF00 == 0xFF00 { (raw & 0xFF) << 8 } else { raw };
                        (key_op(kind - 4, keycode), 4)
                    }
                    _ => (None, 2),
                }
            } else {
                // The bytes `from_ascii` types: printable ASCII and a few controls.
                let c = bytes[i];
                let typed = matches!(c, 0x20..=0x7E | b'\t' | b'\n' | 0x08 | 0x1B | 0x7F);
                (typed.then_some(MacroOp::Char(c)), 1)
            };
            i += len;
            if let Some(op) = op {
                return Some((op, i));
            }
        }
        None
    }

    /// `None` for a keycode with no action here.
    fn key_op(kind: u8, keycode: u16) -> Option<MacroOp> {
        match from_via_keycode(keycode).to_action() {
            Action::No => None,
            action => Some(match kind {
                1 => MacroOp::Tap(action),
                2 => MacroOp::Press(action),
                _ => MacroOp::Release(action),
            }),
        }
    }

    /// Vial's bytes for `ops`, without the terminator; an op Vial cannot spell
    /// renders to nothing.
    pub(crate) fn render(ops: &[MacroOp]) -> impl Iterator<Item = u8> + '_ {
        ops.iter().flat_map(|op| {
            let mut out = heapless::Vec::<u8, 8>::new();
            match *op {
                MacroOp::Tap(action) => render_key(&mut out, 1, action),
                MacroOp::Press(action) => render_key(&mut out, 2, action),
                MacroOp::Release(action) => render_key(&mut out, 3, action),
                MacroOp::Delay(mut ms) => loop {
                    let chunk = ms.min(VIAL_DELAY_MAX);
                    let _ = out.extend_from_slice(&[0x01, 0x04, (chunk % 255) as u8 + 1, (chunk / 255) as u8 + 1]);
                    ms -= chunk;
                    if ms == 0 {
                        break;
                    }
                },
                MacroOp::Char(c) => {
                    if c > 0x01 {
                        let _ = out.push(c);
                    }
                }
                MacroOp::PauseForRelease => {}
            }
            out
        })
    }

    /// `01 kind kc` for a keycode Vial keeps in one byte, `01 kind+4 lo hi` for
    /// the rest; nothing for an action Vial has no keycode for.
    fn render_key(out: &mut heapless::Vec<u8, 8>, kind: u8, action: Action) {
        let keycode = to_via_keycode(KeyAction::Single(action));
        if keycode == 0 {
            return;
        }
        if keycode < 0x100 {
            let _ = out.extend_from_slice(&[0x01, kind, keycode as u8]);
        } else {
            // Vial escapes a zero low byte so the payload never holds a terminator.
            let word = if keycode & 0xFF == 0 {
                0xFF00 | (keycode >> 8)
            } else {
                keycode
            };
            let [lo, hi] = word.to_le_bytes();
            let _ = out.extend_from_slice(&[0x01, kind + 4, lo, hi]);
        }
    }

    /// Write `ops` as one terminated slot at the start of `buf`, returning the
    /// bytes it took; what doesn't fit is dropped, though the defaults were
    /// checked to fit at compile time.
    pub(crate) fn encode_slot(buf: &mut [u8], ops: &[MacroOp]) -> usize {
        let Some(room) = buf.len().checked_sub(1) else {
            return 0;
        };
        let mut pos = 0;
        for byte in render(ops).take(room) {
            buf[pos] = byte;
            pos += 1;
        }
        buf[pos] = 0;
        pos + 1
    }

    #[cfg(test)]
    mod tests {
        use rmk_types::keycode::{HidKeyCode, KeyCode};
        use rmk_types::modifier::ModifierCombination;

        use super::*;

        fn bytes(ops: &[MacroOp]) -> std::vec::Vec<u8> {
            render(ops).collect()
        }

        fn ops(mut bytes: &[u8]) -> std::vec::Vec<MacroOp> {
            let mut ops = std::vec::Vec::new();
            while let Some((op, len)) = decode(bytes) {
                ops.push(op);
                bytes = &bytes[len..];
            }
            ops
        }

        #[test]
        fn every_op_shape_renders_and_decodes_back() {
            let ops_in = [
                MacroOp::Tap(Action::Key(KeyCode::Hid(HidKeyCode::A))),
                MacroOp::Press(Action::Key(KeyCode::Hid(HidKeyCode::LShift))),
                MacroOp::Release(Action::Key(KeyCode::Hid(HidKeyCode::LShift))),
                MacroOp::Delay(100),
                MacroOp::Char(b'x'),
                MacroOp::Tap(Action::KeyWithModifier(HidKeyCode::A, ModifierCombination::LCTRL)),
                // 0x7E00: the zero low byte is escaped.
                MacroOp::Tap(Action::User(0)),
                MacroOp::Press(Action::PersistentDefaultLayer(1)),
            ];
            let rendered = bytes(&ops_in);
            assert_eq!(
                rendered,
                [
                    0x01, 0x01, 0x04, // tap A
                    0x01, 0x02, 0xE1, // press LShift
                    0x01, 0x03, 0xE1, // release LShift
                    0x01, 0x04, 0x65, 0x01, // 100 ms
                    b'x', // a character is its own byte
                    0x01, 0x05, 0x04, 0x01, // WM(A, LCtrl) = 0x0104, little-endian
                    0x01, 0x05, 0x7E, 0xFF, // User(0) = 0x7E00 escaped
                    0x01, 0x06, 0xE1, 0x52, // PDF(1) = 0x52E1
                ]
            );
            assert_eq!(ops(&rendered), ops_in);
        }

        #[test]
        fn delays_split_at_the_encoding_limit_and_round_trip() {
            for ms in [0, 1, 254, 255, 65024, 65025, u16::MAX] {
                let decoded = ops(&bytes(&[MacroOp::Delay(ms)]));
                let total: u32 = decoded
                    .iter()
                    .map(|op| match op {
                        MacroOp::Delay(ms) => *ms as u32,
                        _ => panic!("not a delay"),
                    })
                    .sum();
                assert_eq!(total, ms as u32, "{ms}ms");
                assert!(decoded.len() <= 2);
            }
        }

        #[test]
        fn ops_vial_cannot_spell_are_left_out() {
            let ops = [
                MacroOp::PauseForRelease,
                MacroOp::Char(0x00),
                MacroOp::Char(0x01),
                MacroOp::Tap(Action::Key(KeyCode::Hid(HidKeyCode::B))),
            ];
            assert_eq!(bytes(&ops), [0x01, 0x01, 0x05]);
        }

        #[test]
        fn decode_drops_what_it_cannot_run_and_skips_bad_bytes() {
            // A macro trigger is kept; an unmapped keycode and a control character are dropped.
            assert_eq!(
                ops(&[0x01, 0x05, 0x02, 0x77, 0x01, 0x01, 0x00, 0x07, b'a']),
                [MacroOp::Tap(Action::TriggerMacro(2)), MacroOp::Char(b'a')]
            );
            // An unknown kind skips its two bytes, as vial-gui does.
            assert_eq!(ops(&[0x01, 0x09, b'b']), [MacroOp::Char(b'b')]);
            assert_eq!(ops(&[0x01, 0x01]), [], "truncated");
            assert_eq!(ops(&[0x01, 0x04, 0x01]), [], "truncated delay");
        }

        #[test]
        fn segments_follow_the_terminators() {
            let buf = [b'a', 0, 0, b'b', b'c', 0, 0, 0];
            assert_eq!(segment(&buf, 0), 0..1);
            assert_eq!(segment(&buf, 1), 2..2);
            assert_eq!(segment(&buf, 2), 3..5);
            assert_eq!(segment(&buf, 3), 6..6);
            // Past the last terminator every slot is the empty tail.
            assert_eq!(segment(&buf, 9), 8..8);
            let mut out = [0xFF; 8];
            assert_eq!(encode_slot(&mut out, &[MacroOp::Char(b'a'), MacroOp::Char(b'b')]), 3);
            assert_eq!(out[..3], [b'a', b'b', 0]);
        }
    }
}

/// The postcard encoding of the ops, the buffer's without `vial`: each slot is
/// its byte length as a postcard `u16`, then the ops back to back.
#[cfg(all(feature = "host", not(feature = "vial")))]
pub(crate) mod codec {
    use core::ops::Range;

    use postcard::experimental::max_size::MaxSize;
    use rmk_types::keyboard_macros::MacroOp;

    /// The op bytes of slot `k`, its length prefix excluded; the empty tail
    /// once the buffer runs out.
    pub(crate) fn segment(buf: &[u8], k: usize) -> Range<usize> {
        let mut start = 0;
        for slot in 0..=k {
            let Ok((len, rest)) = postcard::take_from_bytes::<u16>(&buf[start..]) else {
                return buf.len()..buf.len();
            };
            start = buf.len() - rest.len();
            let end = (start + len as usize).min(buf.len());
            if slot == k {
                return start..end;
            }
            start = end;
        }
        unreachable!()
    }

    /// The first op in `bytes` and the bytes it took.
    pub(crate) fn decode(bytes: &[u8]) -> Option<(MacroOp, usize)> {
        let (op, rest) = postcard::take_from_bytes(bytes).ok()?;
        Some((op, bytes.len() - rest.len()))
    }

    /// Write `ops` as one length-prefixed slot at the start of `buf`, returning
    /// the bytes it took. The defaults were checked to fit at compile time.
    pub(crate) fn encode_slot(buf: &mut [u8], ops: &[MacroOp]) -> usize {
        let len: usize = ops
            .iter()
            .map(|op| postcard::to_slice(op, &mut [0; MacroOp::POSTCARD_MAX_SIZE]).map_or(0, |b| b.len()))
            .sum();
        let Ok(prefix) = postcard::to_slice(&(len as u16), &mut *buf) else {
            return 0;
        };
        let mut pos = prefix.len();
        for op in ops {
            pos += postcard::to_slice(op, &mut buf[pos..]).map_or(0, |b| b.len());
        }
        pos
    }

    /// Replace slot `k`'s ops with `bytes`, moving the slots after it; `None`
    /// when the buffer cannot hold the result. Returns the changed byte range.
    pub(crate) fn splice(buf: &mut [u8], k: usize, bytes: &[u8]) -> Option<Range<usize>> {
        let last = crate::MACRO_MAX_NUM.checked_sub(1)?;
        let old = segment(buf, k);
        let start = if k == 0 { 0 } else { segment(buf, k - 1).end };
        let used = segment(buf, last).end;
        let mut prefix = [0; 3];
        let prefix = postcard::to_slice(&(bytes.len() as u16), &mut prefix).ok()?;
        let new_end = start + prefix.len() + bytes.len();
        let used_after = new_end + (used - old.end);
        if used_after > buf.len() {
            return None;
        }
        buf.copy_within(old.end..used, new_end);
        buf[start..start + prefix.len()].copy_from_slice(prefix);
        buf[new_end - bytes.len()..new_end].copy_from_slice(bytes);
        if used_after < used {
            buf[used_after..used].fill(0);
        }
        Some(start..used.max(used_after))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const CHAR_A: MacroOp = MacroOp::Char(b'a');

        #[test]
        fn slots_are_length_prefixed_and_splice_moves_the_rest() {
            // Room for the three slots and the empty ones up to `MACRO_MAX_NUM`.
            let mut buf = [0u8; 16 + crate::MACRO_MAX_NUM];
            let mut pos = encode_slot(&mut buf, &[CHAR_A, CHAR_A]);
            pos += encode_slot(&mut buf[pos..], &[]);
            pos += encode_slot(&mut buf[pos..], &[CHAR_A]);
            assert_eq!(buf[..pos], [4, 4, b'a', 4, b'a', 0, 2, 4, b'a']);
            assert!(buf[pos..].iter().all(|&b| b == 0), "nothing is left past the slots");
            assert_eq!(segment(&buf, 0), 1..5);
            assert_eq!(segment(&buf, 1), 6..6);
            assert_eq!(segment(&buf, 2), 7..9);
            assert_eq!(decode(&buf[7..9]), Some((CHAR_A, 2)));
            // The last slot: its prefix sits behind the 28 empty ones before it.
            const LAST: usize = crate::MACRO_MAX_NUM - 1;
            assert_eq!(splice(&mut buf, LAST, &[4, b'b']), Some(9 + LAST - 3..12 + LAST - 3));

            // The tail moves down behind a shorter slot 0 and up behind a longer one.
            assert_eq!(splice(&mut buf, 0, &[]), Some(0..12 + LAST - 3));
            assert_eq!(buf[..6], [0, 0, 2, 4, b'a', 0]);
            assert_eq!(segment(&buf, 2), 3..5);
            assert_eq!(
                splice(&mut buf, 0, &[4, b'a', 4, b'a', 4, b'a']),
                Some(0..14 + LAST - 3)
            );
            assert_eq!(segment(&buf, 0), 1..7);
            assert_eq!(segment(&buf, 2), 9..11);
            assert_eq!(decode(&buf[segment(&buf, LAST)]), Some((MacroOp::Char(b'b'), 2)));
            // Past the buffer is refused and changes nothing.
            let before = buf;
            assert!(splice(&mut buf, 1, &[4; 64]).is_none());
            assert_eq!(buf, before);
        }
    }
}
