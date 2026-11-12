//! Consumer page keycodes.

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use super::hid::HidKeyCode;

/// Keys in consumer page
/// Ref: <https://www.usb.org/sites/default/files/documents/hut1_12v2.pdf#page=75>
#[non_exhaustive]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord, MaxSize)]
#[cfg_attr(feature = "_codegen", derive(strum::EnumIter))]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub enum ConsumerKey {
    No,
    // 15.5 Display Controls
    SnapShot,
    /// <https://www.usb.org/sites/default/files/hutrr41_0.pdf>
    BrightnessUp,
    BrightnessDown,
    // 15.7 Transport Controls
    Play,
    Pause,
    Record,
    FastForward,
    Rewind,
    NextTrack,
    PrevTrack,
    StopPlay,
    Eject,
    RandomPlay,
    Repeat,
    StopEject,
    PlayPause,
    // 15.9.1 Audio Controls - Volume
    Mute,
    VolumeIncrement,
    VolumeDecrement,
    Reserved,
    // 15.15 Application Launch Buttons
    Email,
    Calculator,
    LocalBrowser,
    Lock,
    ControlPanel,
    Assistant,
    // 15.16 Generic GUI Application Controls
    New,
    Open,
    Close,
    Exit,
    Maximize,
    Minimize,
    Save,
    Print,
    Properties,
    Undo,
    Copy,
    Cut,
    Paste,
    SelectAll,
    Find,
    Search,
    Home,
    Back,
    Forward,
    Stop,
    Refresh,
    Bookmarks,
    NextKeyboardLayoutSelect,
    DesktopShowAllWindows,
    AcSoftKeyLeft,
    Redo,
}

impl From<u16> for ConsumerKey {
    fn from(value: u16) -> Self {
        match value {
            0x00 => Self::No,
            0x65 => Self::SnapShot,
            0x6F => Self::BrightnessUp,
            0x70 => Self::BrightnessDown,
            0xB0 => Self::Play,
            0xB1 => Self::Pause,
            0xB2 => Self::Record,
            0xB3 => Self::FastForward,
            0xB4 => Self::Rewind,
            0xB5 => Self::NextTrack,
            0xB6 => Self::PrevTrack,
            0xB7 => Self::StopPlay,
            0xB8 => Self::Eject,
            0xB9 => Self::RandomPlay,
            0xBC => Self::Repeat,
            0xCC => Self::StopEject,
            0xCD => Self::PlayPause,
            0xE2 => Self::Mute,
            0xE9 => Self::VolumeIncrement,
            0xEA => Self::VolumeDecrement,
            0xEB => Self::Reserved,
            0x18A => Self::Email,
            0x192 => Self::Calculator,
            0x194 => Self::LocalBrowser,
            0x19E => Self::Lock,
            0x19F => Self::ControlPanel,
            0x1CB => Self::Assistant,
            0x201 => Self::New,
            0x202 => Self::Open,
            0x203 => Self::Close,
            0x204 => Self::Exit,
            0x205 => Self::Maximize,
            0x206 => Self::Minimize,
            0x207 => Self::Save,
            0x208 => Self::Print,
            0x209 => Self::Properties,
            0x21A => Self::Undo,
            0x21B => Self::Copy,
            0x21C => Self::Cut,
            0x21D => Self::Paste,
            0x21E => Self::SelectAll,
            0x21F => Self::Find,
            0x221 => Self::Search,
            0x223 => Self::Home,
            0x224 => Self::Back,
            0x225 => Self::Forward,
            0x226 => Self::Stop,
            0x227 => Self::Refresh,
            0x22A => Self::Bookmarks,
            0x279 => Self::Redo,
            0x29D => Self::NextKeyboardLayoutSelect,
            0x29F => Self::DesktopShowAllWindows,
            0x2A0 => Self::AcSoftKeyLeft,
            _ => Self::No,
        }
    }
}

impl From<ConsumerKey> for u16 {
    fn from(value: ConsumerKey) -> Self {
        match value {
            ConsumerKey::No => 0x00,
            ConsumerKey::SnapShot => 0x65,
            ConsumerKey::BrightnessUp => 0x6F,
            ConsumerKey::BrightnessDown => 0x70,
            ConsumerKey::Play => 0xB0,
            ConsumerKey::Pause => 0xB1,
            ConsumerKey::Record => 0xB2,
            ConsumerKey::FastForward => 0xB3,
            ConsumerKey::Rewind => 0xB4,
            ConsumerKey::NextTrack => 0xB5,
            ConsumerKey::PrevTrack => 0xB6,
            ConsumerKey::StopPlay => 0xB7,
            ConsumerKey::Eject => 0xB8,
            ConsumerKey::RandomPlay => 0xB9,
            ConsumerKey::Repeat => 0xBC,
            ConsumerKey::StopEject => 0xCC,
            ConsumerKey::PlayPause => 0xCD,
            ConsumerKey::Mute => 0xE2,
            ConsumerKey::VolumeIncrement => 0xE9,
            ConsumerKey::VolumeDecrement => 0xEA,
            ConsumerKey::Reserved => 0xEB,
            ConsumerKey::Email => 0x18A,
            ConsumerKey::Calculator => 0x192,
            ConsumerKey::LocalBrowser => 0x194,
            ConsumerKey::Lock => 0x19E,
            ConsumerKey::ControlPanel => 0x19F,
            ConsumerKey::Assistant => 0x1CB,
            ConsumerKey::New => 0x201,
            ConsumerKey::Open => 0x202,
            ConsumerKey::Close => 0x203,
            ConsumerKey::Exit => 0x204,
            ConsumerKey::Maximize => 0x205,
            ConsumerKey::Minimize => 0x206,
            ConsumerKey::Save => 0x207,
            ConsumerKey::Print => 0x208,
            ConsumerKey::Properties => 0x209,
            ConsumerKey::Undo => 0x21A,
            ConsumerKey::Copy => 0x21B,
            ConsumerKey::Cut => 0x21C,
            ConsumerKey::Paste => 0x21D,
            ConsumerKey::SelectAll => 0x21E,
            ConsumerKey::Find => 0x21F,
            ConsumerKey::Search => 0x221,
            ConsumerKey::Home => 0x223,
            ConsumerKey::Back => 0x224,
            ConsumerKey::Forward => 0x225,
            ConsumerKey::Stop => 0x226,
            ConsumerKey::Refresh => 0x227,
            ConsumerKey::Bookmarks => 0x22A,
            ConsumerKey::Redo => 0x279,
            ConsumerKey::NextKeyboardLayoutSelect => 0x29D,
            ConsumerKey::DesktopShowAllWindows => 0x29F,
            ConsumerKey::AcSoftKeyLeft => 0x2A0,
        }
    }
}

impl ConsumerKey {
    /// Host-only: list all consumer-page keys, in declaration order.
    #[cfg(feature = "_codegen")]
    pub fn all() -> impl Iterator<Item = Self> {
        <Self as strum::IntoEnumIterator>::iter()
    }

    /// Convert ConsumerKey to the corresponding HidKeyCode
    pub fn to_hid_keycode(&self) -> Option<HidKeyCode> {
        match self {
            ConsumerKey::Mute => Some(HidKeyCode::AudioMute),
            ConsumerKey::VolumeIncrement => Some(HidKeyCode::AudioVolUp),
            ConsumerKey::VolumeDecrement => Some(HidKeyCode::AudioVolDown),
            ConsumerKey::NextTrack => Some(HidKeyCode::MediaNextTrack),
            ConsumerKey::PrevTrack => Some(HidKeyCode::MediaPrevTrack),
            ConsumerKey::StopPlay => Some(HidKeyCode::MediaStop),
            ConsumerKey::PlayPause => Some(HidKeyCode::MediaPlayPause),
            ConsumerKey::Record => Some(HidKeyCode::MediaSelect),
            ConsumerKey::Eject => Some(HidKeyCode::MediaEject),
            ConsumerKey::Email => Some(HidKeyCode::Mail),
            ConsumerKey::Calculator => Some(HidKeyCode::Calculator),
            ConsumerKey::LocalBrowser => Some(HidKeyCode::MyComputer),
            ConsumerKey::Search => Some(HidKeyCode::WwwSearch),
            ConsumerKey::Home => Some(HidKeyCode::WwwHome),
            ConsumerKey::Back => Some(HidKeyCode::WwwBack),
            ConsumerKey::Forward => Some(HidKeyCode::WwwForward),
            ConsumerKey::Stop => Some(HidKeyCode::WwwStop),
            ConsumerKey::Refresh => Some(HidKeyCode::WwwRefresh),
            ConsumerKey::Bookmarks => Some(HidKeyCode::WwwFavorites),
            ConsumerKey::FastForward => Some(HidKeyCode::MediaFastForward),
            ConsumerKey::Rewind => Some(HidKeyCode::MediaRewind),
            ConsumerKey::BrightnessUp => Some(HidKeyCode::BrightnessUp),
            ConsumerKey::BrightnessDown => Some(HidKeyCode::BrightnessDown),
            ConsumerKey::ControlPanel => Some(HidKeyCode::ControlPanel),
            ConsumerKey::Assistant => Some(HidKeyCode::Assistant),
            ConsumerKey::DesktopShowAllWindows => Some(HidKeyCode::MissionControl),
            ConsumerKey::AcSoftKeyLeft => Some(HidKeyCode::Launchpad),
            ConsumerKey::Redo => Some(HidKeyCode::AcRedo),
            ConsumerKey::SnapShot => Some(HidKeyCode::SnapShot),
            ConsumerKey::Play => Some(HidKeyCode::MediaPlay),
            ConsumerKey::Pause => Some(HidKeyCode::MediaPause),
            ConsumerKey::RandomPlay => Some(HidKeyCode::MediaRandomPlay),
            ConsumerKey::Repeat => Some(HidKeyCode::MediaRepeat),
            ConsumerKey::StopEject => Some(HidKeyCode::MediaStopEject),
            ConsumerKey::Lock => Some(HidKeyCode::ScreenLock),
            ConsumerKey::New => Some(HidKeyCode::AcNew),
            ConsumerKey::Open => Some(HidKeyCode::AcOpen),
            _ => None,
        }
    }
}
