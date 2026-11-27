//! Vial protocol

use strum::FromRepr;

/// VIA Protocol 11 uses channel-qualified custom values, including the
/// built-in RGB Matrix channel.
pub const VIA_PROTOCOL_VERSION: u16 = 0x000B;
pub const VIA_FIRMWARE_VERSION: u32 = 0x0001;

pub const VIAL_PROTOCOL_VERSION: u32 = 6;
pub const VIAL_EP_SIZE: usize = 32;

/// Via communication commands.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, FromRepr)]
#[repr(u8)]
pub enum ViaCommand {
    GetProtocolVersion = 0x01, // always 0x01
    GetKeyboardValue = 0x02,
    SetKeyboardValue = 0x03,
    DynamicKeymapGetKeyCode = 0x04,
    DynamicKeymapSetKeyCode = 0x05,
    DynamicKeymapReset = 0x06,
    CustomSetValue = 0x07,
    CustomGetValue = 0x08,
    CustomSave = 0x09,
    EepromReset = 0x0A,
    BootloaderJump = 0x0B,
    DynamicKeymapMacroGetCount = 0x0C,
    DynamicKeymapMacroGetBufferSize = 0x0D,
    DynamicKeymapMacroGetBuffer = 0x0E,
    DynamicKeymapMacroSetBuffer = 0x0F,
    DynamicKeymapMacroReset = 0x10,
    DynamicKeymapGetLayerCount = 0x11,
    DynamicKeymapGetBuffer = 0x12,
    DynamicKeymapSetBuffer = 0x13,
    DynamicKeymapGetEncoder = 0x14,
    DynamicKeymapSetEncoder = 0x15,
    Vial = 0xFE,
    Unhandled = 0xFF,
}

impl From<u8> for ViaCommand {
    fn from(value: u8) -> Self {
        Self::from_repr(value).unwrap_or(Self::Unhandled)
    }
}

/// Information of a via keyboard.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, FromRepr)]
#[repr(u8)]
pub enum ViaKeyboardInfo {
    Uptime = 0x01,
    LayoutOptions = 0x02,
    SwitchMatrixState = 0x03,
    FirmwareVersion = 0x04,
    DeviceIndication = 0x05,
}

impl TryFrom<u8> for ViaKeyboardInfo {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        // Return original value when there's an error
        Self::from_repr(value).ok_or(value)
    }
}

/// Vial communication commands.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, FromRepr)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u8)]
pub enum VialCommand {
    GetKeyboardId = 0x00,
    GetSize = 0x01,
    GetKeyboardDef = 0x02,
    GetEncoder = 0x03,
    SetEncoder = 0x04,
    GetUnlockStatus = 0x05,
    UnlockStart = 0x06,
    UnlockPoll = 0x07,
    Lock = 0x08,
    BehaviorSettingQuery = 0x09,
    GetBehaviorSetting = 0x0A,
    SetBehaviorSetting = 0x0B,
    QmkSettingsReset = 0x0C,
    // Operate on tapdance, combos, etc
    DynamicEntryOp = 0x0D,
    Unhandled = 0xFF,
}

impl From<u8> for VialCommand {
    fn from(value: u8) -> Self {
        Self::from_repr(value).unwrap_or(Self::Unhandled)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, FromRepr)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[repr(u16)]
pub enum SettingKey {
    None,
    ComboTimeout = 0x02,
    OneShotTimeout = 0x06,
    MorseTimeout = 0x07,
    TapInterval = 0x12,
    TapCapslockInterval = 0x13,
    PermissiveHold = 0x16,
    HoldOnOtherKeyPress = 0x17,
    QuickTapTerm = 0x19,
    UnilateralTap = 0x1A,
    PriorIdleTime = 0x1B,
}

impl From<u16> for SettingKey {
    fn from(value: u16) -> Self {
        Self::from_repr(value).unwrap_or(Self::None)
    }
}

/// Vial dynamic commands.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, FromRepr)]
#[repr(u8)]
pub enum VialDynamic {
    DynamicVialGetNumberOfEntries = 0x00,
    DynamicVialMorseGet = 0x01,
    DynamicVialMorseSet = 0x02,
    DynamicVialComboGet = 0x03,
    DynamicVialComboSet = 0x04,
    DynamicVialKeyOverrideGet = 0x05,
    DynamicVialKeyOverrideSet = 0x06,
    Unhandled = 0xFF,
}

impl From<u8> for VialDynamic {
    fn from(value: u8) -> Self {
        Self::from_repr(value).unwrap_or(Self::Unhandled)
    }
}
