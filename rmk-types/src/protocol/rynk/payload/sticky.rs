//! Sticky profile endpoint types.

#[cfg(not(feature = "host"))]
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use crate::sticky::StickyProfile;

/// Replace one complete profile. Index 255 selects the default profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(not(feature = "host"), derive(MaxSize))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct SetStickyProfileRequest {
    pub index: u8,
    pub config: StickyProfile,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keycode::{HidKeyCode, KeyCode};
    #[cfg(not(feature = "host"))]
    use crate::protocol::rynk::tests::{assert_max_size_bound, round_trip};
    use crate::sticky::StickyReleaseConditions;

    #[test]
    fn sticky_profile_round_trip() {
        let req = SetStickyProfileRequest {
            index: 255,
            config: StickyProfile {
                release_on: StickyReleaseConditions::new()
                    .with_before_next_press(true)
                    .with_layer_deactivate(true),
                ignore: core::iter::repeat_n(KeyCode::Hid(HidKeyCode::RGui), crate::constants::STICKY_IGNORE_MAX)
                    .collect(),
                wait_timeout_ms: u16::MAX,
                hold_timeout_ms: u16::MAX,
            },
        };
        #[cfg(not(feature = "host"))]
        {
            round_trip(&req);
            assert_max_size_bound(&req);
        }
        #[cfg(feature = "host")]
        {
            let mut req = req;
            req.config.ignore.push(KeyCode::Hid(HidKeyCode::A));
            let mut buffer = alloc::vec![0; 32 + 5 * req.config.ignore.len()];
            let bytes = postcard::to_slice(&req, &mut buffer).unwrap();
            assert_eq!(postcard::from_bytes::<SetStickyProfileRequest>(bytes).unwrap(), req);
        }
    }
}
