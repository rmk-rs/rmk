//! Bulk-page capacities: how many elements one bulk frame carries, derived
//! from the physical frame buffer. They bound the `heapless::Vec` fields of the
//! sibling bulk payload types and are advertised to hosts via `GetCapabilities`.

use postcard::experimental::max_size::MaxSize;

use crate::action::KeyAction;
use crate::combo::{Combo, ComboDefinition};
use crate::constants::RYNK_BUFFER_SIZE;
use crate::morse::Morse;
use crate::protocol::rynk::message::{RYNK_HEADER_SIZE, max_frame_size};
use crate::varint_max_size;

/// Reported bulk counts are `u8`, so a single message never carries more
/// than this many elements regardless of how large the buffer is.
const BULK_COUNT_CEILING: usize = u8::MAX as usize;

/// Elements a physical buffer holds after COBS framing, header, fixed bytes,
/// and worst-case count prefix.
const fn calculate_item_capacity(physical: usize, item_size: usize, fixed: usize) -> usize {
    let budget = max_frame_size(physical);
    let overhead = RYNK_HEADER_SIZE + fixed + varint_max_size(BULK_COUNT_CEILING);
    let cap = budget.saturating_sub(overhead) / item_size;
    if cap < BULK_COUNT_CEILING {
        cap
    } else {
        BULK_COUNT_CEILING
    }
}

/// Calculate the number of Combos/morses one bulk frame can carry;
/// `physical` is the buffer size the frame must COBS-encode into.
/// Sized by the largest of `ComboDefinition`/`Combo`/`Morse` so every behavior
/// bulk endpoint fits;
/// the one fixed byte is `start_index` on the request / the `Result` tag on the response.
pub const fn bulk_item_capacity(physical: usize) -> usize {
    let mut item = Combo::POSTCARD_MAX_SIZE;
    if ComboDefinition::POSTCARD_MAX_SIZE > item {
        item = ComboDefinition::POSTCARD_MAX_SIZE;
    }
    if Morse::POSTCARD_MAX_SIZE > item {
        item = Morse::POSTCARD_MAX_SIZE;
    }
    calculate_item_capacity(physical, item, 1)
}

/// Calculate the number of keymap keys one bulk frame can carry.
/// The three fixed bytes are `layer`/`start_row`/`start_col`.
pub const fn bulk_key_capacity(physical: usize) -> usize {
    calculate_item_capacity(physical, KeyAction::POSTCARD_MAX_SIZE, 3)
}

/// Worst-case number of combos/morses in the response of `GetMorseBulk`/`GetComboBulk`.
/// The value is advertised as `max_bulk_items`.
pub const MAX_BULK_ITEMS: usize = bulk_item_capacity(RYNK_BUFFER_SIZE);

/// Worst-case number of keymap keys in the response of `GetKeymapBulk`.
/// The value is advertised as `max_bulk_keys`.
pub const MAX_BULK_KEYS: usize = bulk_key_capacity(RYNK_BUFFER_SIZE);

const _: () = assert!(
    MAX_BULK_ITEMS >= 1,
    "rynk_buffer_size is too small to hold one combo/morse in a bulk message; increase it"
);
const _: () = assert!(
    MAX_BULK_KEYS >= 1,
    "rynk_buffer_size is too small to hold one key in a bulk keymap message; increase it"
);

#[cfg(test)]
mod tests {
    use postcard::experimental::max_size::MaxSize;

    use super::{bulk_item_capacity, bulk_key_capacity};
    use crate::action::KeyAction;
    use crate::combo::{Combo, ComboDefinition};
    use crate::morse::Morse;
    use crate::protocol::rynk::message::{RYNK_HEADER_SIZE, max_frame_size};
    use crate::varint_max_size;

    /// The buffer-derived bulk counts stay within `[1, u8::MAX]`, grow with the
    /// buffer, and — crucially — their worst-case encoded frame fits the
    /// logical budget of the physical buffer they were derived from. That fit
    /// is what lets the firmware serve a full bulk message out of its buffer.
    #[test]
    fn bulk_counts_derive_from_buffer_and_fit() {
        const U8_MAX: usize = u8::MAX as usize;
        let combo_item = Combo::POSTCARD_MAX_SIZE
            .max(ComboDefinition::POSTCARD_MAX_SIZE)
            .max(Morse::POSTCARD_MAX_SIZE);

        // Clamp to the u8 report width once the buffer holds 255 of the item
        // (plus slack for framing); 0 for a buffer too small to hold even one
        // element (the `MAX_BULK_ITEMS >= 1` build assert rejects that).
        assert_eq!(bulk_item_capacity(U8_MAX * combo_item + 1024), U8_MAX);
        assert_eq!(bulk_key_capacity(U8_MAX * KeyAction::POSTCARD_MAX_SIZE + 1024), U8_MAX);
        assert_eq!(bulk_item_capacity(0), 0);
        assert_eq!(bulk_key_capacity(0), 0);

        // Sweep small physical sizes one by one — this crosses several COBS
        // 254-byte block boundaries, where the logical budget stalls.
        let (mut prev_c, mut prev_k) = (0, 0);
        for physical in 0..=4096 {
            let c = bulk_item_capacity(physical);
            let k = bulk_key_capacity(physical);
            assert!(c >= prev_c && k >= prev_k, "counts must not shrink as the buffer grows");
            (prev_c, prev_k) = (c, k);

            // Worst-case frames must fit the logical budget of the buffer that
            // produced the count.
            let budget = max_frame_size(physical);
            if c >= 1 {
                let frame = RYNK_HEADER_SIZE + 1 + varint_max_size(c) + c * combo_item;
                assert!(frame <= budget, "combo/morse bulk frame {frame} > budget {budget}");
            }
            if k >= 1 {
                let frame = RYNK_HEADER_SIZE + 3 + varint_max_size(k) + k * KeyAction::POSTCARD_MAX_SIZE;
                assert!(frame <= budget, "keymap bulk frame {frame} > budget {budget}");
            }
        }
    }
}
