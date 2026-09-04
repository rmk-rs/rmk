use postcard::experimental::max_size::MaxSize;
use rmk_macro::event;
use rmk_types::action::Action;
use serde::{Deserialize, Serialize};

use crate::event::KeyboardEvent;

#[event(
    channel_size = crate::ACTION_EVENT_CHANNEL_SIZE,
    pubs = crate::ACTION_EVENT_PUB_SIZE,
    subs = crate::ACTION_EVENT_SUB_SIZE
)]
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ActionEvent {
    pub action: Action,
    pub keyboard_event: KeyboardEvent,
    /// Whether a Sticky key produced this resolved action.
    ///
    /// This local classification flag is omitted from the dongle event wire
    /// format. A deserialized event therefore has this set to `false`.
    #[serde(skip)]
    pub is_sticky: bool,
}

// `is_sticky` is skipped by serde, so it must not enlarge the dongle event buffer.
impl MaxSize for ActionEvent {
    const POSTCARD_MAX_SIZE: usize = Action::POSTCARD_MAX_SIZE + KeyboardEvent::POSTCARD_MAX_SIZE;
}

impl ActionEvent {
    pub const fn new(action: Action, keyboard_event: KeyboardEvent) -> Self {
        Self {
            action,
            keyboard_event,
            is_sticky: false,
        }
    }

    pub(crate) const fn new_sticky(action: Action, keyboard_event: KeyboardEvent) -> Self {
        Self {
            action,
            keyboard_event,
            is_sticky: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use rmk_types::keycode::{HidKeyCode, KeyCode};

    use super::*;
    use crate::event::KeyboardEvent;

    #[test]
    fn sticky_flag_does_not_change_dongle_encoding() {
        let keyboard_event = KeyboardEvent::key(1, 2, true);
        let action = Action::Key(KeyCode::Hid(HidKeyCode::A));
        let direct = ActionEvent::new(action, keyboard_event);
        let sticky = ActionEvent::new_sticky(action, keyboard_event);
        let mut direct_buf = [0; ActionEvent::POSTCARD_MAX_SIZE];
        let mut sticky_buf = [0; ActionEvent::POSTCARD_MAX_SIZE];

        let direct_bytes = postcard::to_slice(&direct, &mut direct_buf).unwrap();
        let sticky_bytes = postcard::to_slice(&sticky, &mut sticky_buf).unwrap();

        assert_eq!(direct_bytes, sticky_bytes);
        let decoded: ActionEvent = postcard::from_bytes(sticky_bytes).unwrap();
        assert_eq!(decoded.action, action);
        assert_eq!(decoded.keyboard_event, keyboard_event);
        assert!(!decoded.is_sticky);
    }
}
