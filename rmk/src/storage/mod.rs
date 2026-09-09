use core::fmt::Debug;

use embassy_embedded_hal::adapter::BlockingAsync;
use embassy_sync::signal::Signal;
use embassy_time::Duration;
use embedded_storage::nor_flash::NorFlash;
use embedded_storage_async::nor_flash::NorFlash as AsyncNorFlash;
use postcard::experimental::max_size::MaxSize;
use rmk_types::connection::ConnectionType;
use rmk_types::morse::MorseProfile;
#[cfg(all(feature = "lighting", feature = "rynk"))]
use rmk_types::protocol::rynk::{
    LIGHTING_CONDITIONAL_SCENE_CHUNK_SIZE, LIGHTING_EXTENDED_CONDITIONAL_SCENE_CHUNK_SIZE,
    LIGHTING_EXTENSION_PARAM_CHUNK, LIGHTING_SCENE_CHUNK_SIZE, LightingConditionalSceneCell,
    LightingExtendedConditionalSceneCell, LightingLayerPolicy, LightingSceneCell,
};
use sequential_storage::Error as SSError;
use sequential_storage::cache::{Cache, Uncached};
use sequential_storage::map::{Key, MapConfig, MapStorage, PostcardValue, SerializationError};
#[cfg(feature = "host")]
use {
    crate::{MACRO_SPACE_SIZE, keyboard::combo::ComboConfig},
    rmk_types::action::{EncoderAction, KeyAction},
    rmk_types::fork::Fork,
    rmk_types::morse::Morse,
};

#[cfg(feature = "_ble")]
use crate::ble::profile::ProfileInfo;
use crate::boot::reboot_keyboard;
use crate::channel::FLASH_CHANNEL;
use crate::config::StorageConfig;
#[cfg(all(feature = "_ble", feature = "split"))]
use crate::split::ble::PeerAddress;
use crate::{BUILD_HASH, config};

/// Reply to a `Flush` request: `false` if a write failed since the previous flush.
static FLUSHED: Signal<crate::RawMutex, bool> = Signal::new();

/// Wait until every write queued before this call has been processed.
/// Returns `false` if any write failed since the previous flush.
/// `FLUSHED` has a single waiter slot, so calls must not overlap.
pub(crate) async fn flush() -> bool {
    FLUSHED.reset();
    FLASH_CHANNEL.send(FlashOperationMessage::Flush).await;
    FLUSHED.wait().await
}

// Request/response over `FLASH_CHANNEL`. One `Signal` per read variant; the
// storage task fires the matching one once it has the result.
#[cfg(feature = "_ble")]
static BOND_INFO_RESPONSE: Signal<crate::RawMutex, Option<ProfileInfo>> = Signal::new();
#[cfg(all(feature = "_ble", feature = "split"))]
static PEER_ADDRESS_RESPONSE: Signal<crate::RawMutex, Option<PeerAddress>> = Signal::new();
#[cfg(feature = "_ble")]
static CONNECTION_TYPE_RESPONSE: Signal<crate::RawMutex, Option<ConnectionType>> = Signal::new();
#[cfg(feature = "_ble")]
static ACTIVE_BLE_PROFILE_RESPONSE: Signal<crate::RawMutex, Option<u8>> = Signal::new();

#[cfg(feature = "_ble")]
async fn request_read<T: Send>(msg: FlashOperationMessage, response: &Signal<crate::RawMutex, T>) -> T {
    response.reset();
    FLASH_CHANNEL.send(msg).await;
    response.wait().await
}

#[cfg(feature = "_ble")]
pub(crate) async fn read_bond_info(slot_num: u8) -> Option<ProfileInfo> {
    request_read(FlashOperationMessage::ReadBleBondInfo(slot_num), &BOND_INFO_RESPONSE).await
}

#[cfg(all(feature = "_ble", feature = "split"))]
pub(crate) async fn read_peer_address(peer_id: u8) -> Option<PeerAddress> {
    request_read(FlashOperationMessage::ReadPeerAddress(peer_id), &PEER_ADDRESS_RESPONSE).await
}

#[cfg(feature = "_ble")]
pub(crate) async fn read_connection_type() -> Option<ConnectionType> {
    request_read(FlashOperationMessage::ReadConnectionType, &CONNECTION_TYPE_RESPONSE).await
}

#[cfg(feature = "_ble")]
pub(crate) async fn read_active_ble_profile() -> Option<u8> {
    request_read(
        FlashOperationMessage::ReadActiveBleProfile,
        &ACTIVE_BLE_PROFILE_RESPONSE,
    )
    .await
}

/// Persist a peer address and wait for it to land.
/// Returns `true` if the write completed successfully.
#[cfg(all(feature = "_ble", feature = "split"))]
pub(crate) async fn write_peer_address(addr: PeerAddress) -> bool {
    FLASH_CHANNEL.send(FlashOperationMessage::PeerAddress(addr)).await;
    flush().await
}

// Message send from other tasks, which will do saving or clearing operation
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum FlashOperationMessage {
    #[cfg(feature = "_ble")]
    // BLE profile info to be saved
    ProfileInfo(ProfileInfo),
    #[cfg(feature = "_ble")]
    // Current active BLE profile number
    ActiveBleProfile(u8),
    #[cfg(all(feature = "_ble", feature = "split"))]
    // Peer address
    PeerAddress(PeerAddress),
    // Clear the storage
    Reset,
    // Clear the layout info
    ResetLayout,
    #[cfg(feature = "_ble")]
    // Clear info of given slot number
    ClearSlot(u8),
    // Layout option
    LayoutOptions(u32),
    // Default layer number
    DefaultLayer(u8),
    #[cfg(feature = "host")]
    MacroData([u8; MACRO_SPACE_SIZE]),
    #[cfg(feature = "host")]
    KeymapKey {
        layer: u8,
        row: u8,
        col: u8,
        action: KeyAction,
    },
    #[cfg(feature = "host")]
    Encoder {
        layer: u8,
        idx: u8,
        action: EncoderAction,
    },
    #[cfg(feature = "host")]
    Combo {
        idx: u8,
        config: ComboConfig,
    },
    #[cfg(feature = "host")]
    Fork {
        idx: u8,
        fork: Fork,
    },
    #[cfg(feature = "host")]
    Morse {
        idx: u8,
        morse: Morse,
    },
    // Current saved connection type
    ConnectionType(ConnectionType),
    // Timeout time for combos
    ComboTimeout(u16),
    // Timeout time for one-shot keys
    OneShotTimeout(u16),
    // Interval for tap actions
    TapInterval(u16),
    // Interval for tapping capslock
    TapCapslockInterval(u16),
    // The prior-idle-time in ms used for in flow tap
    PriorIdleTime(u16),
    // Default morse profile containing all morse/tap-hold settings (mode, timeouts, unilateral_tap)
    MorseDefaultProfile(MorseProfile),
    #[cfg(feature = "rynk")]
    // The whole behavior config in one message (Rynk's SetBehaviorConfig carries
    // every field, so one store beats six read-modify-write cycles)
    BehaviorConfig(BehaviorConfig),
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    // Commit the lighting scene table written by the preceding shards:
    // authoritative cell count and layer policy
    LightingSceneCommit {
        len: u16,
        policy: LightingLayerPolicy,
    },
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    // One shard of the lighting scene table, in wire (stable LED id) form.
    // Index 0 starts a new generation.
    LightingSceneShard {
        index: u8,
        cells: heapless::Vec<LightingSceneCell, LIGHTING_SCENE_CHUNK_SIZE>,
    },
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingRuntimeConditionalSceneCommit {
        len: u16,
    },
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingRuntimeConditionalSceneShard {
        index: u8,
        cells: heapless::Vec<LightingExtendedConditionalSceneCell, LIGHTING_EXTENDED_CONDITIONAL_SCENE_CHUNK_SIZE>,
    },
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    // Animated extension-band selection and the selected effect's parameters
    LightingExtensionState(LightingExtensionRecord),
    #[cfg(feature = "_ble")]
    // Read bond info for the given slot; storage task replies via `BOND_INFO_RESPONSE`.
    ReadBleBondInfo(u8),
    #[cfg(all(feature = "_ble", feature = "split"))]
    // Read peer address for the given peer id; storage task replies via `PEER_ADDRESS_RESPONSE`.
    ReadPeerAddress(u8),
    #[cfg(feature = "_ble")]
    // Read the persisted `ConnectionType`; storage task replies via `CONNECTION_TYPE_RESPONSE`.
    ReadConnectionType,
    #[cfg(feature = "_ble")]
    // Read the persisted active BLE profile number; storage task replies via `ACTIVE_BLE_PROFILE_RESPONSE`.
    ReadActiveBleProfile,
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingExtensionOverlay(LightingExtensionOverlayRecord),
    // Barrier: storage task replies via `FLUSHED` once every earlier message is processed.
    Flush,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum StorageKey {
    StorageConfig,
    LayoutConfig,
    BehaviorConfig,
    ConnectionType,
    #[cfg(feature = "host")]
    MacroData,
    #[cfg(feature = "host")]
    Keymap {
        layer: u8,
        row: u8,
        col: u8,
    },
    #[cfg(feature = "host")]
    Encoder {
        layer: u8,
        idx: u8,
    },
    #[cfg(feature = "host")]
    Combo(u8),
    #[cfg(feature = "host")]
    Fork(u8),
    #[cfg(feature = "host")]
    Morse(u8),
    #[cfg(all(feature = "_ble", feature = "split"))]
    PeerAddress(u8),
    #[cfg(feature = "_ble")]
    ActiveBleProfile,
    #[cfg(feature = "_ble")]
    BondInfo(u8),
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingSceneTable,
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingSceneShard(u8),
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingRuntimeConditionalSceneTable,
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingRuntimeConditionalSceneShard(u8),
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingExtensionState,
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingExtensionOverlay,
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingRuntimeConditionalSceneTableV2,
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingRuntimeConditionalSceneShardV2(u8),
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingSceneCommit,
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingSceneShardB(u8),
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingRuntimeConditionalSceneCommit,
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingRuntimeConditionalSceneShardB(u8),
}

impl StorageKey {
    #[cfg(feature = "host")]
    pub(crate) const fn keymap(layer: u8, row: u8, col: u8) -> Self {
        Self::Keymap { layer, row, col }
    }

    #[cfg(feature = "_ble")]
    pub(crate) const fn bond_info(slot_num: u8) -> Self {
        Self::BondInfo(slot_num)
    }

    #[cfg(feature = "host")]
    pub(crate) const fn combo(idx: u8) -> Self {
        Self::Combo(idx)
    }

    #[cfg(feature = "host")]
    pub(crate) const fn encoder(idx: u8, layer: u8) -> Self {
        Self::Encoder { layer, idx }
    }

    #[cfg(feature = "host")]
    pub(crate) const fn fork(idx: u8) -> Self {
        Self::Fork(idx)
    }

    #[cfg(all(feature = "_ble", feature = "split"))]
    pub(crate) const fn peer_address(peer_id: u8) -> Self {
        Self::PeerAddress(peer_id)
    }

    #[cfg(feature = "host")]
    pub(crate) const fn morse(idx: u8) -> Self {
        Self::Morse(idx)
    }
}

impl Key for StorageKey {
    fn serialize_into(&self, buffer: &mut [u8]) -> Result<usize, SerializationError> {
        postcard::to_slice(self, buffer)
            .map(|used| used.len())
            .map_err(Into::into)
    }

    fn deserialize_from(buffer: &[u8]) -> Result<(Self, usize), SerializationError> {
        let (key, rest): (Self, &[u8]) = postcard::take_from_bytes(buffer).map_err(SerializationError::from)?;
        Ok((key, buffer.len() - rest.len()))
    }

    fn get_len(buffer: &[u8]) -> Result<usize, SerializationError> {
        Self::deserialize_from(buffer).map(|(_, len)| len)
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum StorageData {
    StorageConfig(LocalStorageConfig),
    LayoutConfig(LayoutConfig),
    BehaviorConfig(BehaviorConfig),
    ConnectionType(ConnectionType),
    #[cfg(feature = "host")]
    MacroData(#[serde(with = "crate::host::storage::macro_bytes_serde")] [u8; MACRO_SPACE_SIZE]),
    #[cfg(feature = "host")]
    KeyAction(KeyAction),
    #[cfg(feature = "host")]
    EncoderAction(EncoderAction),
    #[cfg(feature = "host")]
    Combo(ComboConfig),
    #[cfg(feature = "host")]
    Fork(Fork),
    #[cfg(feature = "host")]
    Morse(Morse),
    #[cfg(all(feature = "_ble", feature = "split"))]
    PeerAddress(PeerAddress),
    #[cfg(feature = "_ble")]
    BondInfo(ProfileInfo),
    #[cfg(feature = "_ble")]
    ActiveBleProfile(u8),
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingSceneTable(LightingSceneTableRecord),
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingSceneShard(heapless::Vec<LightingSceneCell, LIGHTING_SCENE_CHUNK_SIZE>),
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingRuntimeConditionalSceneTable(u16),
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingRuntimeConditionalSceneShard(
        heapless::Vec<LightingConditionalSceneCell, LIGHTING_CONDITIONAL_SCENE_CHUNK_SIZE>,
    ),
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingExtensionState(LightingExtensionRecord),
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingExtensionOverlay(LightingExtensionOverlayRecord),
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingRuntimeConditionalSceneTableV2(u16),
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingRuntimeConditionalSceneShardV2(
        heapless::Vec<LightingExtendedConditionalSceneCell, LIGHTING_EXTENDED_CONDITIONAL_SCENE_CHUNK_SIZE>,
    ),
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingSceneCommit(LightingSceneCommitRecord),
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    LightingRuntimeConditionalSceneCommit(LightingRuntimeConditionalSceneCommitRecord),
}

impl<'a> PostcardValue<'a> for StorageData {}

/// Persisted lighting scene-table header. Shards beyond `len` are stale
/// leftovers from a larger previous table and are ignored at load.
/// Persisted animated-extension selection, plus the parameter values of the
/// effect it names. Only the selected effect's parameters are kept: they are
/// what a reboot has to reproduce, and a fixed-size record keeps the write
/// cost of a single flash entry predictable.
///
/// Every index here is validated against the running firmware before it is
/// applied. Effect and palette lists are compiled in, so inserting one effect
/// shifts every later index; a record written by an older build would
/// otherwise resurrect a selection that now names something else.
#[cfg(all(feature = "lighting", feature = "rynk"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct LightingExtensionRecord {
    pub effect: u8,
    pub palette: u8,
    pub value: u8,
    pub speed: u8,
    /// Valid entries in `params`; the remainder is padding.
    pub param_len: u8,
    pub params: [u8; LIGHTING_EXTENSION_PARAM_CHUNK],
}

#[cfg(all(feature = "lighting", feature = "rynk"))]
impl LightingExtensionRecord {
    /// The parameter values that belong to [`Self::effect`].
    pub fn params(&self) -> &[u8] {
        &self.params[..(self.param_len as usize).min(LIGHTING_EXTENSION_PARAM_CHUNK)]
    }
}

/// Persisted optional second effect and the parameter row it uses. This is a
/// separate key so the established primary-extension record remains readable
/// across firmware upgrades.
#[cfg(all(feature = "lighting", feature = "rynk"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct LightingExtensionOverlayRecord {
    pub effect: Option<u8>,
    pub param_len: u8,
    pub params: [u8; LIGHTING_EXTENSION_PARAM_CHUNK],
}

#[cfg(all(feature = "lighting", feature = "rynk"))]
impl LightingExtensionOverlayRecord {
    pub fn params(&self) -> &[u8] {
        &self.params[..(self.param_len as usize).min(LIGHTING_EXTENSION_PARAM_CHUNK)]
    }
}

#[cfg(all(feature = "lighting", feature = "rynk"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct LightingSceneTableRecord {
    pub(crate) len: u16,
    pub(crate) policy: LightingLayerPolicy,
}

/// Which of two shard key sets a lighting table generation occupies.
///
/// A rewrite lands in the generation the commit record does not name, then
/// the commit record moves. Boot reads only the committed generation, so a
/// rewrite cut short by power loss leaves the previous table intact instead
/// of a mix of old and new shards. Generation A is the original key set, so
/// tables written before commit records existed load as A.
#[cfg(all(feature = "lighting", feature = "rynk"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum LightingGeneration {
    A,
    B,
}

#[cfg(all(feature = "lighting", feature = "rynk"))]
impl LightingGeneration {
    const fn alternate(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }

    const fn scene_shard_key(self, index: u8) -> StorageKey {
        match self {
            Self::A => StorageKey::LightingSceneShard(index),
            Self::B => StorageKey::LightingSceneShardB(index),
        }
    }

    const fn runtime_conditional_shard_key(self, index: u8) -> StorageKey {
        match self {
            Self::A => StorageKey::LightingRuntimeConditionalSceneShardV2(index),
            Self::B => StorageKey::LightingRuntimeConditionalSceneShardB(index),
        }
    }
}

#[cfg(all(feature = "lighting", feature = "rynk"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct LightingSceneCommitRecord {
    pub(crate) generation: LightingGeneration,
    pub(crate) len: u16,
    pub(crate) policy: LightingLayerPolicy,
    pub(crate) digest: u32,
}

#[cfg(all(feature = "lighting", feature = "rynk"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct LightingRuntimeConditionalSceneCommitRecord {
    pub(crate) generation: LightingGeneration,
    pub(crate) len: u16,
    pub(crate) digest: u32,
}

/// FNV-1a over the postcard encoding of each cell, in table order. It tells a
/// committed generation's shards from a stale or torn set without buffering.
#[cfg(all(feature = "lighting", feature = "rynk"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LightingTableDigest(u32);

#[cfg(all(feature = "lighting", feature = "rynk"))]
impl LightingTableDigest {
    const fn new() -> Self {
        Self(0x811c_9dc5)
    }

    fn fold<T: serde::Serialize>(&mut self, cell: &T) {
        if let Ok(digest) = postcard::serialize_with_flavor(cell, *self) {
            *self = digest;
        }
    }

    const fn value(self) -> u32 {
        self.0
    }
}

#[cfg(all(feature = "lighting", feature = "rynk"))]
impl postcard::ser_flavors::Flavor for LightingTableDigest {
    type Output = Self;

    fn try_push(&mut self, data: u8) -> postcard::Result<()> {
        self.0 = (self.0 ^ data as u32).wrapping_mul(0x0100_0193);
        Ok(())
    }

    fn finalize(self) -> postcard::Result<Self> {
        Ok(self)
    }
}

/// One in-progress table rewrite on the storage task: the generation its
/// shards go to and the digest of the cells written so far.
#[cfg(all(feature = "lighting", feature = "rynk"))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct LightingTableWrite {
    generation: Option<LightingGeneration>,
    digest: LightingTableDigest,
}

#[cfg(all(feature = "lighting", feature = "rynk"))]
impl LightingTableWrite {
    pub(crate) const fn new() -> Self {
        Self {
            generation: None,
            digest: LightingTableDigest::new(),
        }
    }

    fn begin(&mut self, generation: LightingGeneration) {
        self.generation = Some(generation);
        self.digest = LightingTableDigest::new();
    }

    fn fold<T: serde::Serialize>(&mut self, cells: &[T]) {
        for cell in cells {
            self.digest.fold(cell);
        }
    }

    fn finish(&mut self, fallback: LightingGeneration) -> (LightingGeneration, u32) {
        let generation = self.generation.take().unwrap_or(fallback);
        let digest = self.digest.value();
        self.digest = LightingTableDigest::new();
        (generation, digest)
    }
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct LocalStorageConfig {
    enable: bool,
    build_hash: u32,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct LayoutConfig {
    pub(crate) default_layer: u8,
    pub(crate) layout_option: u32,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct BehaviorConfig {
    // The prior-idle-time in ms used for in flow tap
    pub(crate) prior_idle_time: u16,
    // Default morse profile containing mode, timeouts, and unilateral_tap settings
    pub(crate) morse_default_profile: MorseProfile,

    // Timeout time for combos
    pub(crate) combo_timeout: u16,
    // Timeout time for one-shot keys
    pub(crate) one_shot_timeout: u16,
    // Interval for tap actions
    pub(crate) tap_interval: u16,
    // Interval for tapping capslock.
    // macOS has special processing of capslock, when tapping capslock, the tap interval should be another value
    pub(crate) tap_capslock_interval: u16,
}

impl From<LocalStorageConfig> for StorageData {
    fn from(config: LocalStorageConfig) -> Self {
        Self::StorageConfig(config)
    }
}

impl From<LayoutConfig> for StorageData {
    fn from(config: LayoutConfig) -> Self {
        Self::LayoutConfig(config)
    }
}

impl From<&config::BehaviorConfig> for StorageData {
    fn from(behavior: &config::BehaviorConfig) -> Self {
        // Note: default_layer persists via LayoutConfig (restored in read_keymap), not this struct.
        Self::BehaviorConfig(BehaviorConfig {
            prior_idle_time: behavior.morse.prior_idle_time.as_millis() as u16,
            morse_default_profile: behavior.morse.default_profile,
            combo_timeout: behavior.combo.timeout.as_millis() as u16,
            one_shot_timeout: behavior.one_shot.timeout.as_millis() as u16,
            tap_interval: behavior.tap.tap_interval,
            tap_capslock_interval: behavior.tap.tap_capslock_interval,
        })
    }
}

pub fn async_flash_wrapper<F: NorFlash>(flash: F) -> BlockingAsync<F> {
    embassy_embedded_hal::adapter::BlockingAsync::new(flash)
}

/// Storage for the firmwares that hold no keymap of their own — a split
/// peripheral and a dongle. Both still persist their BLE bonds, which the
/// profile manager loads over `FLASH_CHANNEL`.
#[cfg(any(feature = "split", feature = "dongle"))]
pub async fn new_storage_without_keymap<F: AsyncNorFlash>(
    flash: F,
    storage_config: StorageConfig,
) -> Storage<F, 0, 0, 0, 0> {
    Storage::<F, 0, 0, 0, 0>::new(
        flash,
        #[cfg(feature = "host")]
        &[],
        #[cfg(feature = "host")]
        &None,
        &storage_config,
        &config::BehaviorConfig::default(),
    )
    .await
}

type StorageCache = Cache<Uncached, Uncached, Uncached, StorageKey>;

pub struct Storage<
    F: AsyncNorFlash,
    const ROW: usize,
    const COL: usize,
    const NUM_LAYER: usize,
    const NUM_ENCODER: usize = 0,
> {
    pub(crate) flash: MapStorage<StorageKey, F, StorageCache>,
    pub(crate) buffer: [u8; get_buffer_size()],
}

/// Read out storage config, update and then save back.
/// This macro applies to only some of the configs.
macro_rules! update_storage_field {
    ($f: expr, $buf: expr, $key:ident, $field:ident) => {{
        let key = StorageKey::$key;
        if let Ok(Some(StorageData::$key(mut saved))) = $f.fetch_item($buf, &key).await {
            saved.$field = $field;
            $f.store_item($buf, &key, &StorageData::$key(saved)).await
        } else {
            Ok(())
        }
    }};
}

impl<F: AsyncNorFlash, const ROW: usize, const COL: usize, const NUM_LAYER: usize, const NUM_ENCODER: usize>
    Storage<F, ROW, COL, NUM_LAYER, NUM_ENCODER>
{
    async fn fetch_data(&mut self, key: StorageKey) -> Option<StorageData> {
        match self.flash.fetch_item(&mut self.buffer, &key).await {
            Ok(data) => data,
            Err(e) => {
                print_storage_error::<F>(e);
                None
            }
        }
    }

    async fn store_data(&mut self, key: StorageKey, data: &StorageData) -> Result<(), SSError<F::Error>> {
        self.flash.store_item(&mut self.buffer, &key, data).await
    }

    pub async fn new(
        flash: F,
        #[cfg(feature = "host")] keymap: &[[[KeyAction; COL]; ROW]; NUM_LAYER],
        #[cfg(feature = "host")] encoder_map: &Option<&mut [[EncoderAction; NUM_ENCODER]; NUM_LAYER]>,
        storage_config: &StorageConfig,
        behavior_config: &config::BehaviorConfig,
    ) -> Self {
        // Check storage setting
        assert!(
            storage_config.num_sectors >= 2,
            "Number of used sector for storage must larger than 1"
        );

        // If config.start_addr == 0:
        // - For nRF chips: use sectors starting at 0x0006_0000
        // - For other chips: use the last `num_sectors` sectors
        // Otherwise, use storage config setting
        // When DFU is active the storage partition already sits at the correct
        // offset — the _nrf_ble special case (0x60000) only applies without DFU.
        #[cfg(all(feature = "_nrf_ble", not(any(feature = "dfu_rp", feature = "dfu_nrf"))))]
        let start_addr = if storage_config.start_addr == 0 {
            0x0006_0000
        } else {
            storage_config.start_addr
        };

        #[cfg(not(all(feature = "_nrf_ble", not(any(feature = "dfu_rp", feature = "dfu_nrf")))))]
        let start_addr = storage_config.start_addr;
        // Check storage setting
        info!(
            "Flash capacity {} KB, RMK use {} KB({} sectors) starting from 0x{:X} as storage",
            flash.capacity() / 1024,
            (F::ERASE_SIZE * storage_config.num_sectors as usize) / 1024,
            storage_config.num_sectors,
            storage_config.start_addr,
        );

        let storage_range = if start_addr == 0 {
            (flash.capacity() - storage_config.num_sectors as usize * F::ERASE_SIZE) as u32..flash.capacity() as u32
        } else {
            assert!(
                start_addr.is_multiple_of(F::ERASE_SIZE),
                "Storage's start addr MUST BE a multiplier of sector size"
            );
            start_addr as u32..(start_addr + storage_config.num_sectors as usize * F::ERASE_SIZE) as u32
        };

        let mut storage = Self {
            flash: MapStorage::new(flash, MapConfig::new(storage_range), Cache::new_uncached()),
            buffer: [0; get_buffer_size()],
        };

        // Check whether keymap and configs have been storaged in flash
        if !storage.check_enable().await || storage_config.clear_storage {
            // Clear storage first
            debug!("Clearing storage!");
            let _ = storage.flash.erase_all().await;

            // Initialize storage from keymap and config
            if storage
                .initialize_storage_with_config(
                    #[cfg(feature = "host")]
                    keymap,
                    #[cfg(feature = "host")]
                    encoder_map,
                    behavior_config,
                )
                .await
                .is_err()
            {
                // When there's an error, `enable: false` should be saved back to storage, preventing partial initialization of storage
                storage
                    .store_data(
                        StorageKey::StorageConfig,
                        &StorageData::from(LocalStorageConfig {
                            enable: false,
                            build_hash: BUILD_HASH,
                        }),
                    )
                    .await
                    .ok();
            }
        } else if storage_config.clear_layout {
            #[cfg(feature = "host")]
            {
                debug!("clear_layout=true; overwriting layout items without erase.");
                let encoder_map = encoder_map.as_ref().map(|m| &**m);
                let _ = storage.reset_layout_only(keymap, &encoder_map, behavior_config).await;
            }
        }

        storage
    }

    pub(crate) async fn read_behavior_config(
        &mut self,
        behavior_config: &mut config::BehaviorConfig,
    ) -> Result<(), ()> {
        let read_data = self
            .flash
            .fetch_item(&mut self.buffer, &StorageKey::BehaviorConfig)
            .await
            .map_err(|e| print_storage_error::<F>(e))?;

        if let Some(StorageData::BehaviorConfig(c)) = read_data {
            behavior_config.morse.prior_idle_time = Duration::from_millis(c.prior_idle_time as u64);
            behavior_config.morse.default_profile = c.morse_default_profile;

            behavior_config.combo.timeout = Duration::from_millis(c.combo_timeout as u64);
            behavior_config.one_shot.timeout = Duration::from_millis(c.one_shot_timeout as u64);
            behavior_config.tap.tap_interval = c.tap_interval;
            behavior_config.tap.tap_capslock_interval = c.tap_capslock_interval;
        }

        Ok(())
    }

    /// Read the persisted animated-extension selection at startup, before the
    /// storage task takes ownership. The caller validates every index against
    /// its own compiled effect/palette lists before applying it.
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    pub async fn read_lighting_extension_state(&mut self) -> Option<LightingExtensionRecord> {
        match self.fetch_data(StorageKey::LightingExtensionState).await {
            Some(StorageData::LightingExtensionState(record)) => Some(record),
            _ => None,
        }
    }

    #[cfg(all(feature = "lighting", feature = "rynk"))]
    pub async fn read_lighting_extension_overlay(&mut self) -> Option<LightingExtensionOverlayRecord> {
        match self.fetch_data(StorageKey::LightingExtensionOverlay).await {
            Some(StorageData::LightingExtensionOverlay(record)) => Some(record),
            _ => None,
        }
    }

    /// Read the persisted lighting scene configuration at startup, before the
    /// storage task takes ownership. Returns the persisted layer policy, if
    /// any, and appends up to `CAP` stored cells to `cells`.
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    pub async fn read_lighting_scenes<const CAP: usize>(
        &mut self,
        cells: &mut heapless::Vec<LightingSceneCell, CAP>,
    ) -> Option<LightingLayerPolicy> {
        if let Some(StorageData::LightingSceneCommit(commit)) = self.fetch_data(StorageKey::LightingSceneCommit).await {
            let start = cells.len();
            let mut digest = LightingTableDigest::new();
            let mut seen: u16 = 0;
            let mut index: u8 = 0;
            while seen < commit.len {
                let Some(StorageData::LightingSceneShard(shard)) =
                    self.fetch_data(commit.generation.scene_shard_key(index)).await
                else {
                    break;
                };
                if shard.is_empty() {
                    break;
                }
                for cell in shard {
                    if seen == commit.len {
                        break;
                    }
                    digest.fold(&cell);
                    seen += 1;
                    let _ = cells.push(cell);
                }
                let Some(next) = index.checked_add(1) else {
                    break;
                };
                index = next;
            }
            if seen != commit.len || digest.value() != commit.digest {
                cells.truncate(start);
                return None;
            }
            return Some(commit.policy);
        }

        let Some(StorageData::LightingSceneTable(record)) = self.fetch_data(StorageKey::LightingSceneTable).await
        else {
            return None;
        };
        let len = (record.len as usize).min(CAP);
        let mut index: u8 = 0;
        'shards: while cells.len() < len {
            let Some(StorageData::LightingSceneShard(shard)) =
                self.fetch_data(StorageKey::LightingSceneShard(index)).await
            else {
                break;
            };
            if shard.is_empty() {
                break;
            }
            for cell in shard {
                if cells.len() == len || cells.push(cell).is_err() {
                    break 'shards;
                }
            }
            let Some(next) = index.checked_add(1) else {
                break;
            };
            index = next;
        }
        Some(record.policy)
    }

    #[cfg(all(feature = "lighting", feature = "rynk"))]
    async fn committed_lighting_scene_generation(&mut self) -> Option<LightingGeneration> {
        match self.fetch_data(StorageKey::LightingSceneCommit).await {
            Some(StorageData::LightingSceneCommit(commit)) => Some(commit.generation),
            _ => None,
        }
    }

    /// Write one shard of a scene-table rewrite. Index 0 opens a generation:
    /// the one the current commit does not name, so the committed table stays
    /// readable until the rewrite commits.
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    pub(crate) async fn store_lighting_scene_shard(
        &mut self,
        write: &mut LightingTableWrite,
        index: u8,
        cells: heapless::Vec<LightingSceneCell, LIGHTING_SCENE_CHUNK_SIZE>,
    ) -> Result<(), SSError<F::Error>> {
        if index == 0 || write.generation.is_none() {
            let committed = self.committed_lighting_scene_generation().await;
            write.begin(committed.map_or(LightingGeneration::B, LightingGeneration::alternate));
        }
        let generation = write.generation.expect("begun above");
        write.fold(cells.as_slice());
        // Rewrites cover the whole table on every mutation; comparing before
        // writing keeps unchanged shards from consuming flash.
        let key = generation.scene_shard_key(index);
        match self.fetch_data(key).await {
            Some(StorageData::LightingSceneShard(saved)) if saved == cells => Ok(()),
            _ => self.store_data(key, &StorageData::LightingSceneShard(cells)).await,
        }
    }

    /// Commit the generation the preceding shards were written to. An
    /// identical table is already committed under the other generation when
    /// the length, policy, and digest match, so nothing moves.
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    pub(crate) async fn commit_lighting_scenes(
        &mut self,
        write: &mut LightingTableWrite,
        len: u16,
        policy: LightingLayerPolicy,
    ) -> Result<(), SSError<F::Error>> {
        let committed = self.committed_lighting_scene_generation().await;
        let (generation, digest) = write.finish(committed.map_or(LightingGeneration::B, LightingGeneration::alternate));
        let record = LightingSceneCommitRecord {
            generation,
            len,
            policy,
            digest,
        };
        match self.fetch_data(StorageKey::LightingSceneCommit).await {
            Some(StorageData::LightingSceneCommit(saved))
                if (saved.len, saved.policy, saved.digest) == (record.len, record.policy, record.digest) => {}
            _ => {
                self.store_data(
                    StorageKey::LightingSceneCommit,
                    &StorageData::LightingSceneCommit(record),
                )
                .await?
            }
        }
        // A downgrade reads the legacy header against generation A, which
        // later rewrites overwrite in place; leave it describing no cells.
        if let Some(StorageData::LightingSceneTable(saved)) = self.fetch_data(StorageKey::LightingSceneTable).await
            && saved.len != 0
        {
            self.store_data(
                StorageKey::LightingSceneTable,
                &StorageData::LightingSceneTable(LightingSceneTableRecord { len: 0, ..saved }),
            )
            .await?
        }
        Ok(())
    }

    #[cfg(all(feature = "lighting", feature = "rynk"))]
    async fn committed_lighting_runtime_conditional_generation(&mut self) -> Option<LightingGeneration> {
        match self.fetch_data(StorageKey::LightingRuntimeConditionalSceneCommit).await {
            Some(StorageData::LightingRuntimeConditionalSceneCommit(commit)) => Some(commit.generation),
            _ => None,
        }
    }

    #[cfg(all(feature = "lighting", feature = "rynk"))]
    pub(crate) async fn store_lighting_runtime_conditional_shard(
        &mut self,
        write: &mut LightingTableWrite,
        index: u8,
        cells: heapless::Vec<LightingExtendedConditionalSceneCell, LIGHTING_EXTENDED_CONDITIONAL_SCENE_CHUNK_SIZE>,
    ) -> Result<(), SSError<F::Error>> {
        if index == 0 || write.generation.is_none() {
            let committed = self.committed_lighting_runtime_conditional_generation().await;
            write.begin(committed.map_or(LightingGeneration::B, LightingGeneration::alternate));
        }
        let generation = write.generation.expect("begun above");
        write.fold(cells.as_slice());
        let key = generation.runtime_conditional_shard_key(index);
        match self.fetch_data(key).await {
            Some(StorageData::LightingRuntimeConditionalSceneShardV2(saved)) if saved == cells => Ok(()),
            _ => {
                self.store_data(key, &StorageData::LightingRuntimeConditionalSceneShardV2(cells))
                    .await
            }
        }
    }

    /// Commit the runtime conditional generation, and empty both legacy
    /// headers so a firmware downgrade sees no rules rather than stale cells
    /// that were edited or deleted since.
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    pub(crate) async fn commit_lighting_runtime_conditional_scenes(
        &mut self,
        write: &mut LightingTableWrite,
        len: u16,
    ) -> Result<(), SSError<F::Error>> {
        let committed = self.committed_lighting_runtime_conditional_generation().await;
        let (generation, digest) = write.finish(committed.map_or(LightingGeneration::B, LightingGeneration::alternate));
        let record = LightingRuntimeConditionalSceneCommitRecord {
            generation,
            len,
            digest,
        };
        match self.fetch_data(StorageKey::LightingRuntimeConditionalSceneCommit).await {
            Some(StorageData::LightingRuntimeConditionalSceneCommit(saved))
                if (saved.len, saved.digest) == (record.len, record.digest) => {}
            _ => {
                self.store_data(
                    StorageKey::LightingRuntimeConditionalSceneCommit,
                    &StorageData::LightingRuntimeConditionalSceneCommit(record),
                )
                .await?
            }
        }
        if let Some(StorageData::LightingRuntimeConditionalSceneTableV2(saved)) = self
            .fetch_data(StorageKey::LightingRuntimeConditionalSceneTableV2)
            .await
            && saved != 0
        {
            self.store_data(
                StorageKey::LightingRuntimeConditionalSceneTableV2,
                &StorageData::LightingRuntimeConditionalSceneTableV2(0),
            )
            .await?
        }
        if let Some(StorageData::LightingRuntimeConditionalSceneTable(saved)) =
            self.fetch_data(StorageKey::LightingRuntimeConditionalSceneTable).await
            && saved != 0
        {
            self.store_data(
                StorageKey::LightingRuntimeConditionalSceneTable,
                &StorageData::LightingRuntimeConditionalSceneTable(0),
            )
            .await?
        }
        Ok(())
    }

    /// Read the persisted ordered runtime conditional table at startup.
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    pub async fn read_lighting_runtime_conditional_scenes<const CAP: usize>(
        &mut self,
        cells: &mut heapless::Vec<LightingExtendedConditionalSceneCell, CAP>,
    ) {
        if let Some(StorageData::LightingRuntimeConditionalSceneCommit(commit)) =
            self.fetch_data(StorageKey::LightingRuntimeConditionalSceneCommit).await
        {
            let start = cells.len();
            let mut digest = LightingTableDigest::new();
            let mut seen: u16 = 0;
            let mut index: u8 = 0;
            while seen < commit.len {
                let Some(StorageData::LightingRuntimeConditionalSceneShardV2(shard)) = self
                    .fetch_data(commit.generation.runtime_conditional_shard_key(index))
                    .await
                else {
                    break;
                };
                if shard.is_empty() {
                    break;
                }
                for cell in shard {
                    if seen == commit.len {
                        break;
                    }
                    digest.fold(&cell);
                    seen += 1;
                    let _ = cells.push(cell);
                }
                let Some(next) = index.checked_add(1) else {
                    break;
                };
                index = next;
            }
            if seen != commit.len || digest.value() != commit.digest {
                cells.truncate(start);
            }
            return;
        }

        if let Some(StorageData::LightingRuntimeConditionalSceneTableV2(saved_len)) = self
            .fetch_data(StorageKey::LightingRuntimeConditionalSceneTableV2)
            .await
        {
            let len = (saved_len as usize).min(CAP);
            let mut index: u8 = 0;
            'shards: while cells.len() < len {
                let Some(StorageData::LightingRuntimeConditionalSceneShardV2(shard)) = self
                    .fetch_data(StorageKey::LightingRuntimeConditionalSceneShardV2(index))
                    .await
                else {
                    break;
                };
                if shard.is_empty() {
                    break;
                }
                for cell in shard {
                    if cells.len() == len || cells.push(cell).is_err() {
                        break 'shards;
                    }
                }
                let Some(next) = index.checked_add(1) else {
                    break;
                };
                index = next;
            }
            return;
        }

        let Some(StorageData::LightingRuntimeConditionalSceneTable(saved_len)) =
            self.fetch_data(StorageKey::LightingRuntimeConditionalSceneTable).await
        else {
            return;
        };
        let len = (saved_len as usize).min(CAP);
        let mut index: u8 = 0;
        'legacy_shards: while cells.len() < len {
            let Some(StorageData::LightingRuntimeConditionalSceneShard(shard)) = self
                .fetch_data(StorageKey::LightingRuntimeConditionalSceneShard(index))
                .await
            else {
                break;
            };
            if shard.is_empty() {
                break;
            }
            for cell in shard {
                if cells.len() == len
                    || cells
                        .push(LightingExtendedConditionalSceneCell {
                            cell,
                            connection: None,
                            effects: None,
                        })
                        .is_err()
                {
                    break 'legacy_shards;
                }
            }
            let Some(next) = index.checked_add(1) else {
                break;
            };
            index = next;
        }
    }

    async fn initialize_storage_with_config(
        &mut self,
        #[cfg(feature = "host")] keymap: &[[[KeyAction; COL]; ROW]; NUM_LAYER],
        #[cfg(feature = "host")] encoder_map: &Option<&mut [[EncoderAction; NUM_ENCODER]; NUM_LAYER]>,
        behavior: &config::BehaviorConfig,
    ) -> Result<(), ()> {
        // Save storage config
        self.store_data(
            StorageKey::StorageConfig,
            &StorageData::from(LocalStorageConfig {
                enable: true,
                build_hash: BUILD_HASH,
            }),
        )
        .await
        .map_err(|e| print_storage_error::<F>(e))?;

        // Save layout config
        self.store_data(
            StorageKey::LayoutConfig,
            &StorageData::from(LayoutConfig {
                default_layer: 0,
                layout_option: 0,
            }),
        )
        .await
        .map_err(|e| print_storage_error::<F>(e))?;

        // Save behavior config
        self.store_data(StorageKey::BehaviorConfig, &StorageData::from(behavior))
            .await
            .map_err(|e| print_storage_error::<F>(e))?;

        #[cfg(feature = "host")]
        for (layer, layer_data) in keymap.iter().enumerate() {
            for (row, row_data) in layer_data.iter().enumerate() {
                for (col, action) in row_data.iter().enumerate() {
                    self.store_data(
                        StorageKey::keymap(layer as u8, row as u8, col as u8),
                        &StorageData::KeyAction(*action),
                    )
                    .await
                    .map_err(|e| print_storage_error::<F>(e))?;
                }
            }
        }

        // Save encoder configurations
        #[cfg(feature = "host")]
        if let Some(encoder_map) = encoder_map {
            for (layer, layer_data) in encoder_map.iter().enumerate() {
                for (idx, action) in layer_data.iter().enumerate() {
                    self.store_data(
                        StorageKey::encoder(idx as u8, layer as u8),
                        &StorageData::EncoderAction(*action),
                    )
                    .await
                    .map_err(|e| print_storage_error::<F>(e))?;
                }
            }
        }

        Ok(())
    }

    #[cfg(feature = "host")]
    async fn reset_layout_only(
        &mut self,
        keymap: &[[[KeyAction; COL]; ROW]; NUM_LAYER],
        encoder_map: &Option<&[[EncoderAction; NUM_ENCODER]; NUM_LAYER]>,
        behavior: &config::BehaviorConfig,
    ) -> Result<(), SSError<F::Error>> {
        self.store_data(
            StorageKey::LayoutConfig,
            &StorageData::from(LayoutConfig {
                default_layer: 0,
                layout_option: 0,
            }),
        )
        .await?;
        self.store_data(StorageKey::BehaviorConfig, &StorageData::from(behavior))
            .await?;

        // TODO: Generic reset for vial and other hosts
        for (layer, layer_data) in keymap.iter().enumerate() {
            for (row, row_data) in layer_data.iter().enumerate() {
                for (col, action) in row_data.iter().enumerate() {
                    self.store_data(
                        StorageKey::keymap(layer as u8, row as u8, col as u8),
                        &StorageData::KeyAction(*action),
                    )
                    .await?;
                }
            }
        }

        // TODO: Generic reset for vial and other hosts
        if let Some(encoder_map) = encoder_map {
            for (layer, layer_data) in encoder_map.iter().enumerate() {
                for (idx, action) in layer_data.iter().enumerate() {
                    self.store_data(
                        StorageKey::encoder(idx as u8, layer as u8),
                        &StorageData::EncoderAction(*action),
                    )
                    .await?;
                }
            }
        }

        Ok(())
    }

    async fn check_enable(&mut self) -> bool {
        if let Some(StorageData::StorageConfig(config)) = self.fetch_data(StorageKey::StorageConfig).await
            && config.enable
            && config.build_hash == BUILD_HASH
        {
            return true;
        }
        false
    }
}

impl<F: AsyncNorFlash, const ROW: usize, const COL: usize, const NUM_LAYER: usize, const NUM_ENCODER: usize>
    crate::core_traits::Runnable for Storage<F, ROW, COL, NUM_LAYER, NUM_ENCODER>
{
    async fn run(&mut self) -> ! {
        let mut failed = false;
        #[cfg(all(feature = "lighting", feature = "rynk"))]
        let mut scene_write = LightingTableWrite::new();
        #[cfg(all(feature = "lighting", feature = "rynk"))]
        let mut runtime_conditional_write = LightingTableWrite::new();
        loop {
            let info: FlashOperationMessage = FLASH_CHANNEL.receive().await;
            debug!("Flash operation: {:?}", info);

            let write_result: Result<(), SSError<F::Error>> = match info {
                FlashOperationMessage::Flush => {
                    FLUSHED.signal(!failed);
                    failed = false;
                    continue;
                }
                #[cfg(feature = "_ble")]
                FlashOperationMessage::ReadBleBondInfo(slot_num) => {
                    let resp = match self.fetch_data(StorageKey::bond_info(slot_num)).await {
                        Some(StorageData::BondInfo(info)) => Some(info),
                        _ => None,
                    };
                    BOND_INFO_RESPONSE.signal(resp);
                    continue;
                }
                #[cfg(all(feature = "_ble", feature = "split"))]
                FlashOperationMessage::ReadPeerAddress(peer_id) => {
                    let resp = match self.fetch_data(StorageKey::peer_address(peer_id)).await {
                        Some(StorageData::PeerAddress(addr)) => Some(addr),
                        _ => None,
                    };
                    PEER_ADDRESS_RESPONSE.signal(resp);
                    continue;
                }
                #[cfg(feature = "_ble")]
                FlashOperationMessage::ReadConnectionType => {
                    let resp = match self.fetch_data(StorageKey::ConnectionType).await {
                        Some(StorageData::ConnectionType(v)) => Some(v),
                        _ => None,
                    };
                    CONNECTION_TYPE_RESPONSE.signal(resp);
                    continue;
                }
                #[cfg(feature = "_ble")]
                FlashOperationMessage::ReadActiveBleProfile => {
                    let resp = match self.fetch_data(StorageKey::ActiveBleProfile).await {
                        Some(StorageData::ActiveBleProfile(v)) => Some(v),
                        _ => None,
                    };
                    ACTIVE_BLE_PROFILE_RESPONSE.signal(resp);
                    continue;
                }

                FlashOperationMessage::LayoutOptions(layout_option) => {
                    update_storage_field!(&mut self.flash, &mut self.buffer, LayoutConfig, layout_option)
                }
                FlashOperationMessage::Reset => {
                    let result = self.flash.erase_all().await;
                    reboot_keyboard();
                    result
                }
                FlashOperationMessage::ResetLayout => {
                    info!("Ignoring ResetLayout at runtime (handled at startup via clear_layout).");
                    Ok(())
                }
                FlashOperationMessage::DefaultLayer(default_layer) => {
                    update_storage_field!(&mut self.flash, &mut self.buffer, LayoutConfig, default_layer)
                }
                #[cfg(feature = "host")]
                FlashOperationMessage::MacroData(data) => {
                    self.store_data(StorageKey::MacroData, &StorageData::MacroData(data))
                        .await
                }
                #[cfg(feature = "host")]
                FlashOperationMessage::KeymapKey {
                    layer,
                    row,
                    col,
                    action,
                } => {
                    self.store_data(StorageKey::keymap(layer, row, col), &StorageData::KeyAction(action))
                        .await
                }
                #[cfg(feature = "host")]
                FlashOperationMessage::Encoder { layer, idx, action } => {
                    self.store_data(StorageKey::encoder(idx, layer), &StorageData::EncoderAction(action))
                        .await
                }
                #[cfg(feature = "host")]
                FlashOperationMessage::Combo { idx, config } => {
                    self.store_data(StorageKey::combo(idx), &StorageData::Combo(config))
                        .await
                }
                #[cfg(feature = "host")]
                FlashOperationMessage::Fork { idx, fork } => {
                    self.store_data(StorageKey::fork(idx), &StorageData::Fork(fork)).await
                }
                #[cfg(feature = "host")]
                FlashOperationMessage::Morse { idx, morse } => {
                    self.store_data(StorageKey::morse(idx), &StorageData::Morse(morse))
                        .await
                }
                FlashOperationMessage::ConnectionType(ty) => {
                    self.store_data(StorageKey::ConnectionType, &StorageData::ConnectionType(ty))
                        .await
                }
                #[cfg(all(feature = "_ble", feature = "split"))]
                FlashOperationMessage::PeerAddress(peer) => {
                    self.store_data(StorageKey::peer_address(peer.peer_id), &StorageData::PeerAddress(peer))
                        .await
                }
                #[cfg(feature = "_ble")]
                FlashOperationMessage::ActiveBleProfile(profile) => {
                    self.store_data(StorageKey::ActiveBleProfile, &StorageData::ActiveBleProfile(profile))
                        .await
                }
                #[cfg(feature = "_ble")]
                FlashOperationMessage::ClearSlot(slot_num) => {
                    use trouble_host::prelude::SecurityLevel;
                    use trouble_host::{Address, BondInformation, Identity, LongTermKey};

                    info!("Clearing bond info slot_num: {}", slot_num);
                    // Remove item in `sequential-storage` is quite expensive, so just override the item with `removed = true`
                    let empty = ProfileInfo {
                        removed: true,
                        slot_num,
                        info: BondInformation::new(
                            Identity {
                                addr: Address::default(),
                                irk: None,
                            },
                            LongTermKey::from_le_bytes([0; 16]),
                            SecurityLevel::NoEncryption,
                            false,
                        ),
                        cccd_table: heapless::Vec::new(),
                    };
                    self.store_data(StorageKey::bond_info(slot_num), &StorageData::BondInfo(empty))
                        .await
                }
                #[cfg(feature = "_ble")]
                FlashOperationMessage::ProfileInfo(b) => {
                    debug!("Saving profile info: {:?}", b);
                    self.store_data(StorageKey::bond_info(b.slot_num), &StorageData::BondInfo(b))
                        .await
                }
                FlashOperationMessage::ComboTimeout(combo_timeout) => {
                    update_storage_field!(&mut self.flash, &mut self.buffer, BehaviorConfig, combo_timeout)
                }
                FlashOperationMessage::OneShotTimeout(one_shot_timeout) => {
                    update_storage_field!(&mut self.flash, &mut self.buffer, BehaviorConfig, one_shot_timeout)
                }
                FlashOperationMessage::TapInterval(tap_interval) => {
                    update_storage_field!(&mut self.flash, &mut self.buffer, BehaviorConfig, tap_interval)
                }
                FlashOperationMessage::TapCapslockInterval(tap_capslock_interval) => {
                    update_storage_field!(&mut self.flash, &mut self.buffer, BehaviorConfig, tap_capslock_interval)
                }
                FlashOperationMessage::PriorIdleTime(prior_idle_time) => {
                    update_storage_field!(&mut self.flash, &mut self.buffer, BehaviorConfig, prior_idle_time)
                }
                FlashOperationMessage::MorseDefaultProfile(morse_default_profile) => {
                    update_storage_field!(&mut self.flash, &mut self.buffer, BehaviorConfig, morse_default_profile)
                }
                #[cfg(feature = "rynk")]
                FlashOperationMessage::BehaviorConfig(behavior_config) => {
                    self.store_data(
                        StorageKey::BehaviorConfig,
                        &StorageData::BehaviorConfig(behavior_config),
                    )
                    .await
                }
                #[cfg(all(feature = "lighting", feature = "rynk"))]
                FlashOperationMessage::LightingExtensionState(record) => {
                    // The selection changes on every RGB key press and every
                    // host edit, so skip writes that would store what is
                    // already there rather than spending a flash entry.
                    match self.fetch_data(StorageKey::LightingExtensionState).await {
                        Some(StorageData::LightingExtensionState(saved)) if saved == record => Ok(()),
                        _ => {
                            self.store_data(
                                StorageKey::LightingExtensionState,
                                &StorageData::LightingExtensionState(record),
                            )
                            .await
                        }
                    }
                }
                #[cfg(all(feature = "lighting", feature = "rynk"))]
                FlashOperationMessage::LightingExtensionOverlay(record) => {
                    match self.fetch_data(StorageKey::LightingExtensionOverlay).await {
                        Some(StorageData::LightingExtensionOverlay(saved)) if saved == record => Ok(()),
                        _ => {
                            self.store_data(
                                StorageKey::LightingExtensionOverlay,
                                &StorageData::LightingExtensionOverlay(record),
                            )
                            .await
                        }
                    }
                }
                #[cfg(all(feature = "lighting", feature = "rynk"))]
                FlashOperationMessage::LightingSceneCommit { len, policy } => {
                    self.commit_lighting_scenes(&mut scene_write, len, policy).await
                }
                #[cfg(all(feature = "lighting", feature = "rynk"))]
                FlashOperationMessage::LightingSceneShard { index, cells } => {
                    self.store_lighting_scene_shard(&mut scene_write, index, cells).await
                }
                #[cfg(all(feature = "lighting", feature = "rynk"))]
                FlashOperationMessage::LightingRuntimeConditionalSceneCommit { len } => {
                    self.commit_lighting_runtime_conditional_scenes(&mut runtime_conditional_write, len)
                        .await
                }
                #[cfg(all(feature = "lighting", feature = "rynk"))]
                FlashOperationMessage::LightingRuntimeConditionalSceneShard { index, cells } => {
                    self.store_lighting_runtime_conditional_shard(&mut runtime_conditional_write, index, cells)
                        .await
                }
            };

            if let Err(e) = write_result {
                print_storage_error::<F>(e);
                failed = true;
            }
        }
    }
}

pub(crate) fn print_storage_error<F: AsyncNorFlash>(e: SSError<F::Error>) {
    match e {
        #[cfg(feature = "defmt")]
        SSError::Storage { value: e } => error!("Flash error: {:?}", defmt::Debug2Format(&e)),
        #[cfg(not(feature = "defmt"))]
        SSError::Storage { value: _e } => error!("Flash error"),
        SSError::FullStorage => error!("Storage is full"),
        SSError::Corrupted {} => error!("Storage is corrupted"),
        SSError::BufferTooBig => error!("Buffer too big"),
        SSError::BufferTooSmall(x) => error!("Buffer too small, needs {} bytes", x),
        SSError::SerializationError(e) => error!("Map value error: {}", e),
        SSError::ItemTooBig => error!("Item too big"),
        _ => error!("Unknown storage error"),
    }
}

const fn get_buffer_size() -> usize {
    #[cfg(feature = "host")]
    {
        // The buffer size needed = size_of(StorageData) = MACRO_SPACE_SIZE + 8(generally)
        // According to doc of `sequential-storage`, for some flashes it should be aligned in 32 bytes
        // To make sure the buffer works, do this alignment always
        let buffer_size = if crate::MACRO_SPACE_SIZE < 248 {
            256
        } else {
            crate::MACRO_SPACE_SIZE + 8
        };

        // Efficiently round up to the nearest multiple of 32 using bit manipulation.
        (buffer_size + 31) & !31
    }

    #[cfg(not(feature = "host"))]
    256
}

#[cfg(test)]
mod tests {
    use sequential_storage::cache::Cache;
    use sequential_storage::map::{MapConfig, MapStorage};

    use super::*;
    use crate::config::{BehaviorConfig as RuntimeBehaviorConfig, StorageConfig as RuntimeStorageConfig};
    use crate::test_support::test_block_on as block_on;

    #[derive(Debug, Clone, Copy)]
    struct TestFlashError;

    impl embedded_storage_async::nor_flash::NorFlashError for TestFlashError {
        fn kind(&self) -> embedded_storage_async::nor_flash::NorFlashErrorKind {
            embedded_storage_async::nor_flash::NorFlashErrorKind::Other
        }
    }

    struct TestFlash<const SIZE: usize, const ERASE_SIZE: usize, const WRITE_SIZE: usize> {
        bytes: [u8; SIZE],
    }

    impl<const SIZE: usize, const ERASE_SIZE: usize, const WRITE_SIZE: usize> TestFlash<SIZE, ERASE_SIZE, WRITE_SIZE> {
        fn new() -> Self {
            Self { bytes: [0xFF; SIZE] }
        }
    }

    impl<const SIZE: usize, const ERASE_SIZE: usize, const WRITE_SIZE: usize> embedded_storage::nor_flash::ErrorType
        for TestFlash<SIZE, ERASE_SIZE, WRITE_SIZE>
    {
        type Error = TestFlashError;
    }

    impl<const SIZE: usize, const ERASE_SIZE: usize, const WRITE_SIZE: usize> embedded_storage::nor_flash::ReadNorFlash
        for TestFlash<SIZE, ERASE_SIZE, WRITE_SIZE>
    {
        const READ_SIZE: usize = 1;

        fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
            let start = offset as usize;
            let end = start + bytes.len();
            bytes.copy_from_slice(&self.bytes[start..end]);
            Ok(())
        }

        fn capacity(&self) -> usize {
            SIZE
        }
    }

    impl<const SIZE: usize, const ERASE_SIZE: usize, const WRITE_SIZE: usize> embedded_storage::nor_flash::NorFlash
        for TestFlash<SIZE, ERASE_SIZE, WRITE_SIZE>
    {
        const WRITE_SIZE: usize = WRITE_SIZE;
        const ERASE_SIZE: usize = ERASE_SIZE;

        fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
            self.bytes[from as usize..to as usize].fill(0xFF);
            Ok(())
        }

        fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
            let start = offset as usize;
            let end = start + bytes.len();
            for (dst, src) in self.bytes[start..end].iter_mut().zip(bytes.iter()) {
                *dst &= *src;
            }
            Ok(())
        }
    }

    impl<const SIZE: usize, const ERASE_SIZE: usize, const WRITE_SIZE: usize>
        embedded_storage_async::nor_flash::ReadNorFlash for TestFlash<SIZE, ERASE_SIZE, WRITE_SIZE>
    {
        const READ_SIZE: usize = 1;

        async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
            embedded_storage::nor_flash::ReadNorFlash::read(self, offset, bytes)
        }

        fn capacity(&self) -> usize {
            SIZE
        }
    }

    impl<const SIZE: usize, const ERASE_SIZE: usize, const WRITE_SIZE: usize>
        embedded_storage_async::nor_flash::NorFlash for TestFlash<SIZE, ERASE_SIZE, WRITE_SIZE>
    {
        const WRITE_SIZE: usize = WRITE_SIZE;
        const ERASE_SIZE: usize = ERASE_SIZE;

        async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
            embedded_storage::nor_flash::NorFlash::erase(self, from, to)
        }

        async fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
            embedded_storage::nor_flash::NorFlash::write(self, offset, bytes)
        }
    }

    #[test]
    fn storage_key_round_trip() {
        let cases = [
            StorageKey::StorageConfig,
            StorageKey::LayoutConfig,
            StorageKey::BehaviorConfig,
            StorageKey::ConnectionType,
            #[cfg(feature = "host")]
            StorageKey::MacroData,
            #[cfg(feature = "host")]
            StorageKey::Keymap {
                layer: 2,
                row: 3,
                col: 4,
            },
            #[cfg(feature = "host")]
            StorageKey::Encoder { layer: 1, idx: 5 },
            #[cfg(feature = "host")]
            StorageKey::Combo(6),
            #[cfg(feature = "host")]
            StorageKey::Fork(7),
            #[cfg(feature = "host")]
            StorageKey::Morse(8),
            #[cfg(all(feature = "_ble", feature = "split"))]
            StorageKey::PeerAddress(0),
            #[cfg(feature = "_ble")]
            StorageKey::ActiveBleProfile,
            #[cfg(feature = "_ble")]
            StorageKey::BondInfo(0),
            #[cfg(all(feature = "lighting", feature = "rynk"))]
            StorageKey::LightingSceneTable,
            #[cfg(all(feature = "lighting", feature = "rynk"))]
            StorageKey::LightingSceneShard(0),
            #[cfg(all(feature = "lighting", feature = "rynk"))]
            StorageKey::LightingRuntimeConditionalSceneTable,
            #[cfg(all(feature = "lighting", feature = "rynk"))]
            StorageKey::LightingRuntimeConditionalSceneShard(0),
            #[cfg(all(feature = "lighting", feature = "rynk"))]
            StorageKey::LightingExtensionState,
            #[cfg(all(feature = "lighting", feature = "rynk"))]
            StorageKey::LightingExtensionOverlay,
            #[cfg(all(feature = "lighting", feature = "rynk"))]
            StorageKey::LightingRuntimeConditionalSceneTableV2,
            #[cfg(all(feature = "lighting", feature = "rynk"))]
            StorageKey::LightingRuntimeConditionalSceneShardV2(0),
            #[cfg(all(feature = "lighting", feature = "rynk"))]
            StorageKey::LightingSceneCommit,
            #[cfg(all(feature = "lighting", feature = "rynk"))]
            StorageKey::LightingSceneShardB(0),
            #[cfg(all(feature = "lighting", feature = "rynk"))]
            StorageKey::LightingRuntimeConditionalSceneCommit,
            #[cfg(all(feature = "lighting", feature = "rynk"))]
            StorageKey::LightingRuntimeConditionalSceneShardB(0),
        ];

        let mut buffer = [0u8; 64];
        for key in cases {
            let size = <StorageKey as Key>::serialize_into(&key, &mut buffer).unwrap();
            let (decoded, used) = <StorageKey as Key>::deserialize_from(&buffer[..size]).unwrap();
            assert_eq!(decoded, key);
            assert_eq!(used, size);
        }
    }

    #[cfg(all(feature = "_ble", feature = "split"))]
    #[test]
    fn peer_address_write_waits_for_its_own_flush() {
        use core::future::Future;
        use core::pin::pin;
        use core::task::{Context, Poll, Waker};

        let mut cx = Context::from_waker(Waker::noop());
        FLASH_CHANNEL.clear();
        FLUSHED.reset();
        FLASH_CHANNEL
            .try_send(FlashOperationMessage::LayoutOptions(42))
            .unwrap();

        let mut write = pin!(write_peer_address(PeerAddress::new(0, true, [1; 6])));
        assert!(matches!(write.as_mut().poll(&mut cx), Poll::Pending));

        // The storage task sees the older write, the peer address, then the barrier.
        assert!(matches!(
            FLASH_CHANNEL.try_receive(),
            Ok(FlashOperationMessage::LayoutOptions(42))
        ));
        assert!(matches!(
            FLASH_CHANNEL.try_receive(),
            Ok(FlashOperationMessage::PeerAddress(_))
        ));
        assert!(matches!(write.as_mut().poll(&mut cx), Poll::Pending));
        assert!(matches!(FLASH_CHANNEL.try_receive(), Ok(FlashOperationMessage::Flush)));
        FLUSHED.signal(true);
        assert!(matches!(write.as_mut().poll(&mut cx), Poll::Ready(true)));
    }

    #[cfg(all(feature = "lighting", feature = "rynk"))]
    #[test]
    fn legacy_runtime_conditional_records_load_without_connection_condition() {
        block_on(async {
            type Flash = TestFlash<16_384, 4_096, 1>;

            let storage_range = (16_384 - 2 * 4_096) as u32..16_384u32;
            let mut map =
                MapStorage::<StorageKey, _, _>::new(Flash::new(), MapConfig::new(storage_range), Cache::new_uncached());
            let mut buffer = [0u8; get_buffer_size()];
            let legacy = LightingConditionalSceneCell {
                conditions: rmk_types::protocol::rynk::LightingConditionSet {
                    layer: Some(rmk_types::protocol::rynk::LightingLayerCondition { layer: 2, active: true }),
                    battery: None,
                    output_mode: None,
                },
                led_id: rmk_types::protocol::rynk::LightingLedId(42),
                effect: rmk_types::protocol::rynk::LightingEffect::Solid {
                    color: rmk_types::protocol::rynk::LightingRgb8 { r: 1, g: 2, b: 3 },
                },
            };
            let mut shard = heapless::Vec::<LightingConditionalSceneCell, LIGHTING_CONDITIONAL_SCENE_CHUNK_SIZE>::new();
            shard.push(legacy).unwrap();
            map.store_item(
                &mut buffer,
                &StorageKey::LightingRuntimeConditionalSceneTable,
                &StorageData::LightingRuntimeConditionalSceneTable(1),
            )
            .await
            .unwrap();
            map.store_item(
                &mut buffer,
                &StorageKey::LightingRuntimeConditionalSceneShard(0),
                &StorageData::LightingRuntimeConditionalSceneShard(shard),
            )
            .await
            .unwrap();

            let mut storage = Storage::<Flash, 0, 0, 0, 0> { flash: map, buffer };
            let mut loaded = heapless::Vec::<LightingExtendedConditionalSceneCell, 4>::new();
            storage.read_lighting_runtime_conditional_scenes(&mut loaded).await;

            assert_eq!(loaded.len(), 1);
            assert_eq!(loaded[0].cell, legacy);
            assert_eq!(loaded[0].connection, None);
        });
    }

    #[cfg(all(feature = "lighting", feature = "rynk"))]
    #[test]
    fn runtime_conditional_commit_empties_the_legacy_table() {
        block_on(async {
            type Flash = TestFlash<16_384, 4_096, 1>;

            let storage_range = (16_384 - 2 * 4_096) as u32..16_384u32;
            let mut map =
                MapStorage::<StorageKey, _, _>::new(Flash::new(), MapConfig::new(storage_range), Cache::new_uncached());
            let mut buffer = [0u8; get_buffer_size()];
            map.store_item(
                &mut buffer,
                &StorageKey::LightingRuntimeConditionalSceneTable,
                &StorageData::LightingRuntimeConditionalSceneTable(3),
            )
            .await
            .unwrap();

            let mut storage = Storage::<Flash, 0, 0, 0, 0> { flash: map, buffer };
            let mut write = LightingTableWrite::new();
            storage
                .commit_lighting_runtime_conditional_scenes(&mut write, 1)
                .await
                .unwrap();

            assert!(matches!(
                storage
                    .fetch_data(StorageKey::LightingRuntimeConditionalSceneCommit)
                    .await,
                Some(StorageData::LightingRuntimeConditionalSceneCommit(
                    LightingRuntimeConditionalSceneCommitRecord { len: 1, .. }
                ))
            ));
            assert!(
                matches!(
                    storage
                        .fetch_data(StorageKey::LightingRuntimeConditionalSceneTable)
                        .await,
                    Some(StorageData::LightingRuntimeConditionalSceneTable(0))
                ),
                "a downgraded firmware must see an empty legacy table"
            );
        });
    }

    #[cfg(all(feature = "lighting", feature = "rynk"))]
    #[test]
    fn extended_runtime_conditional_records_preserve_connection_condition() {
        block_on(async {
            type Flash = TestFlash<16_384, 4_096, 1>;

            let storage_range = (16_384 - 2 * 4_096) as u32..16_384u32;
            let mut map =
                MapStorage::<StorageKey, _, _>::new(Flash::new(), MapConfig::new(storage_range), Cache::new_uncached());
            let mut buffer = [0u8; get_buffer_size()];
            let cell = LightingExtendedConditionalSceneCell {
                cell: LightingConditionalSceneCell {
                    conditions: rmk_types::protocol::rynk::LightingConditionSet {
                        layer: None,
                        battery: None,
                        output_mode: None,
                    },
                    led_id: rmk_types::protocol::rynk::LightingLedId(7),
                    effect: rmk_types::protocol::rynk::LightingEffect::Solid {
                        color: rmk_types::protocol::rynk::LightingRgb8 { r: 4, g: 5, b: 6 },
                    },
                },
                connection: Some(rmk_types::protocol::rynk::LightingConnectionCondition {
                    transport: Some(rmk_types::protocol::rynk::LightingActiveTransport::Ble),
                    profile: Some(4),
                    ble_state: Some(rmk_types::ble::BleState::Advertising),
                    bonded: None,
                    usb_connected: None,
                }),
                effects: None,
            };
            let mut shard = heapless::Vec::<
                LightingExtendedConditionalSceneCell,
                LIGHTING_EXTENDED_CONDITIONAL_SCENE_CHUNK_SIZE,
            >::new();
            shard.push(cell).unwrap();
            map.store_item(
                &mut buffer,
                &StorageKey::LightingRuntimeConditionalSceneTableV2,
                &StorageData::LightingRuntimeConditionalSceneTableV2(1),
            )
            .await
            .unwrap();
            map.store_item(
                &mut buffer,
                &StorageKey::LightingRuntimeConditionalSceneShardV2(0),
                &StorageData::LightingRuntimeConditionalSceneShardV2(shard),
            )
            .await
            .unwrap();

            let mut storage = Storage::<Flash, 0, 0, 0, 0> { flash: map, buffer };
            let mut loaded = heapless::Vec::<LightingExtendedConditionalSceneCell, 4>::new();
            storage.read_lighting_runtime_conditional_scenes(&mut loaded).await;

            assert_eq!(loaded.as_slice(), &[cell]);
        });
    }

    #[cfg(all(feature = "lighting", feature = "rynk"))]
    type LightingFlash = TestFlash<16_384, 4_096, 1>;

    #[cfg(all(feature = "lighting", feature = "rynk"))]
    fn lighting_storage() -> Storage<LightingFlash, 0, 0, 0, 0> {
        let storage_range = (16_384 - 2 * 4_096) as u32..16_384u32;
        Storage {
            flash: MapStorage::<StorageKey, _, _>::new(
                LightingFlash::new(),
                MapConfig::new(storage_range),
                Cache::new_uncached(),
            ),
            buffer: [0u8; get_buffer_size()],
        }
    }

    #[cfg(all(feature = "lighting", feature = "rynk"))]
    fn scene_shard(index: u8, value: u8, len: usize) -> heapless::Vec<LightingSceneCell, LIGHTING_SCENE_CHUNK_SIZE> {
        use rmk_types::protocol::rynk::{LightingEffect, LightingLedId, LightingRgb8};
        (0..len)
            .map(|id| LightingSceneCell {
                layer: 0,
                led_id: LightingLedId(index as u16 * LIGHTING_SCENE_CHUNK_SIZE as u16 + id as u16),
                effect: LightingEffect::Solid {
                    color: LightingRgb8 { r: value, g: 0, b: 0 },
                },
            })
            .collect()
    }

    /// Loaded scene cells as `(policy, red values)`, the value marking which
    /// rewrite a cell came from.
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    async fn loaded_scenes(
        storage: &mut Storage<LightingFlash, 0, 0, 0, 0>,
    ) -> (Option<LightingLayerPolicy>, std::vec::Vec<u8>) {
        use rmk_types::protocol::rynk::LightingEffect;
        let mut cells = heapless::Vec::<LightingSceneCell, 32>::new();
        let policy = storage.read_lighting_scenes(&mut cells).await;
        let values = cells
            .iter()
            .map(|cell| match cell.effect {
                LightingEffect::Solid { color } => color.r,
                _ => 0,
            })
            .collect();
        (policy, values)
    }

    /// Power loss between the shard writes of a rewrite must restore the
    /// previous table whole, whether that table predates commit records or
    /// was committed by one.
    #[cfg(all(feature = "lighting", feature = "rynk"))]
    #[test]
    fn scene_rewrite_commits_a_generation_and_survives_interruption() {
        block_on(async {
            let mut storage = lighting_storage();
            // Written by firmware without commit records.
            for index in 0..2 {
                storage
                    .store_data(
                        StorageKey::LightingSceneShard(index),
                        &StorageData::LightingSceneShard(scene_shard(index, 1, LIGHTING_SCENE_CHUNK_SIZE)),
                    )
                    .await
                    .unwrap();
            }
            storage
                .store_data(
                    StorageKey::LightingSceneTable,
                    &StorageData::LightingSceneTable(LightingSceneTableRecord {
                        len: 2 * LIGHTING_SCENE_CHUNK_SIZE as u16,
                        policy: LightingLayerPolicy::EffectiveOnly,
                    }),
                )
                .await
                .unwrap();
            assert_eq!(
                loaded_scenes(&mut storage).await,
                (Some(LightingLayerPolicy::EffectiveOnly), vec![1; 16])
            );

            // The first rewrite stops after one shard.
            let mut write = LightingTableWrite::new();
            storage
                .store_lighting_scene_shard(&mut write, 0, scene_shard(0, 2, LIGHTING_SCENE_CHUNK_SIZE))
                .await
                .unwrap();
            assert_eq!(
                loaded_scenes(&mut storage).await,
                (Some(LightingLayerPolicy::EffectiveOnly), vec![1; 16])
            );

            let mut write = LightingTableWrite::new();
            for index in 0..2 {
                storage
                    .store_lighting_scene_shard(&mut write, index, scene_shard(index, 2, LIGHTING_SCENE_CHUNK_SIZE))
                    .await
                    .unwrap();
            }
            storage
                .commit_lighting_scenes(&mut write, 16, LightingLayerPolicy::ActiveStack)
                .await
                .unwrap();
            assert_eq!(
                loaded_scenes(&mut storage).await,
                (Some(LightingLayerPolicy::ActiveStack), vec![2; 16])
            );
            assert!(matches!(
                storage.fetch_data(StorageKey::LightingSceneTable).await,
                Some(StorageData::LightingSceneTable(LightingSceneTableRecord { len: 0, .. }))
            ));

            // The next rewrite goes back to the first generation's keys and
            // is again cut short: the committed generation is untouched.
            let mut write = LightingTableWrite::new();
            storage
                .store_lighting_scene_shard(&mut write, 0, scene_shard(0, 3, LIGHTING_SCENE_CHUNK_SIZE))
                .await
                .unwrap();
            assert_eq!(
                loaded_scenes(&mut storage).await,
                (Some(LightingLayerPolicy::ActiveStack), vec![2; 16])
            );

            // A shorter table commits over the leftover shard.
            let mut write = LightingTableWrite::new();
            storage
                .store_lighting_scene_shard(&mut write, 0, scene_shard(0, 4, 5))
                .await
                .unwrap();
            storage
                .commit_lighting_scenes(&mut write, 5, LightingLayerPolicy::ActiveStack)
                .await
                .unwrap();
            assert_eq!(
                loaded_scenes(&mut storage).await,
                (Some(LightingLayerPolicy::ActiveStack), vec![4; 5])
            );
        });
    }

    #[cfg(all(feature = "lighting", feature = "rynk"))]
    #[test]
    fn runtime_conditional_rewrite_reads_only_the_committed_generation() {
        block_on(async {
            let cell = |led: u16| LightingExtendedConditionalSceneCell {
                cell: LightingConditionalSceneCell {
                    conditions: rmk_types::protocol::rynk::LightingConditionSet {
                        layer: None,
                        battery: None,
                        output_mode: None,
                    },
                    led_id: rmk_types::protocol::rynk::LightingLedId(led),
                    effect: rmk_types::protocol::rynk::LightingEffect::Solid {
                        color: rmk_types::protocol::rynk::LightingRgb8 { r: 1, g: 2, b: 3 },
                    },
                },
                connection: None,
                effects: None,
            };
            let shard = |led: u16| {
                let mut shard = heapless::Vec::<
                    LightingExtendedConditionalSceneCell,
                    LIGHTING_EXTENDED_CONDITIONAL_SCENE_CHUNK_SIZE,
                >::new();
                shard.push(cell(led)).unwrap();
                shard
            };
            let mut storage = lighting_storage();
            storage
                .store_data(
                    StorageKey::LightingRuntimeConditionalSceneTableV2,
                    &StorageData::LightingRuntimeConditionalSceneTableV2(1),
                )
                .await
                .unwrap();
            storage
                .store_data(
                    StorageKey::LightingRuntimeConditionalSceneShardV2(0),
                    &StorageData::LightingRuntimeConditionalSceneShardV2(shard(7)),
                )
                .await
                .unwrap();
            let mut loaded = heapless::Vec::<LightingExtendedConditionalSceneCell, 4>::new();
            storage.read_lighting_runtime_conditional_scenes(&mut loaded).await;
            assert_eq!(loaded.as_slice(), &[cell(7)]);

            let mut write = LightingTableWrite::new();
            storage
                .store_lighting_runtime_conditional_shard(&mut write, 0, shard(8))
                .await
                .unwrap();
            let mut loaded = heapless::Vec::<LightingExtendedConditionalSceneCell, 4>::new();
            storage.read_lighting_runtime_conditional_scenes(&mut loaded).await;
            assert_eq!(loaded.as_slice(), &[cell(7)]);

            storage
                .commit_lighting_runtime_conditional_scenes(&mut write, 1)
                .await
                .unwrap();
            let mut loaded = heapless::Vec::<LightingExtendedConditionalSceneCell, 4>::new();
            storage.read_lighting_runtime_conditional_scenes(&mut loaded).await;
            assert_eq!(loaded.as_slice(), &[cell(8)]);
            assert!(matches!(
                storage
                    .fetch_data(StorageKey::LightingRuntimeConditionalSceneTableV2)
                    .await,
                Some(StorageData::LightingRuntimeConditionalSceneTableV2(0))
            ));
        });
    }

    #[test]
    fn build_hash_mismatch_reinitializes_storage() {
        block_on(async {
            type Flash = TestFlash<16_384, 4_096, 1>;

            let storage_range = (16_384 - 2 * 4_096) as u32..16_384u32;
            let mut map =
                MapStorage::<StorageKey, _, _>::new(Flash::new(), MapConfig::new(storage_range), Cache::new_uncached());
            let mut buffer = [0u8; 256];

            map.store_item(
                &mut buffer,
                &StorageKey::StorageConfig,
                &StorageData::StorageConfig(LocalStorageConfig {
                    enable: true,
                    build_hash: BUILD_HASH.wrapping_sub(1),
                }),
            )
            .await
            .unwrap();
            map.store_item(
                &mut buffer,
                &StorageKey::LayoutConfig,
                &StorageData::LayoutConfig(LayoutConfig {
                    default_layer: 7,
                    layout_option: 42,
                }),
            )
            .await
            .unwrap();

            let (flash, _) = map.destroy();
            #[cfg(feature = "host")]
            let keymap = [[[KeyAction::No; 1]; 1]; 1];
            #[cfg(feature = "host")]
            let encoder_map: Option<&mut [[EncoderAction; 0]; 1]> = None;

            let mut storage = Storage::<Flash, 1, 1, 1, 0>::new(
                flash,
                #[cfg(feature = "host")]
                &keymap,
                #[cfg(feature = "host")]
                &encoder_map,
                &RuntimeStorageConfig::default(),
                &RuntimeBehaviorConfig::default(),
            )
            .await;

            let stored_layout = storage.fetch_data(StorageKey::LayoutConfig).await.unwrap();
            let stored_config = storage.fetch_data(StorageKey::StorageConfig).await.unwrap();

            assert!(matches!(
                stored_layout,
                StorageData::LayoutConfig(LayoutConfig {
                    default_layer: 0,
                    layout_option: 0,
                })
            ));
            assert!(matches!(
                stored_config,
                StorageData::StorageConfig(LocalStorageConfig {
                    enable: true,
                    build_hash: BUILD_HASH,
                })
            ));
        });
    }

    // A stored LayoutConfig must reach the Vial GUI again after a power
    // cycle: read_keymap restores layout_option into KeymapData and
    // KeyMap::build copies it into the runtime state that
    // GetKeyboardValue(LayoutOptions) answers from (the GET wiring itself is
    // covered by host::via::tests::layout_options_set_then_get_roundtrip).
    // Deleting the restore in read_keymap (or the copy in KeyMap::build)
    // leaves the runtime value at 0 and fails this test.
    #[cfg(feature = "vial")]
    #[test]
    fn layout_option_restored_from_storage() {
        use crate::config::BehaviorConfig;
        use crate::keymap::{KeyMap, KeymapData};

        block_on(async {
            type Flash = TestFlash<16_384, 4_096, 1>;

            let storage_range = (16_384 - 2 * 4_096) as u32..16_384u32;
            let mut map =
                MapStorage::<StorageKey, _, _>::new(Flash::new(), MapConfig::new(storage_range), Cache::new_uncached());
            let mut buffer = [0u8; 256];

            // A matching build hash keeps the stored records across the boot.
            map.store_item(
                &mut buffer,
                &StorageKey::StorageConfig,
                &StorageData::StorageConfig(LocalStorageConfig {
                    enable: true,
                    build_hash: BUILD_HASH,
                }),
            )
            .await
            .unwrap();
            map.store_item(
                &mut buffer,
                &StorageKey::LayoutConfig,
                &StorageData::LayoutConfig(LayoutConfig {
                    default_layer: 0,
                    layout_option: 42,
                }),
            )
            .await
            .unwrap();

            let (flash, _) = map.destroy();
            let keymap_init = [[[KeyAction::No; 1]; 1]; 1];
            let encoder_map_init: Option<&mut [[EncoderAction; 0]; 1]> = None;

            let mut storage = Storage::<Flash, 1, 1, 1, 0>::new(
                flash,
                &keymap_init,
                &encoder_map_init,
                &RuntimeStorageConfig::default(),
                &RuntimeBehaviorConfig::default(),
            )
            .await;

            // Boot-time restore path: storage -> KeymapData -> KeyMap::build.
            let mut data = KeymapData::new([[[KeyAction::No]]]);
            let mut behavior = BehaviorConfig::default();
            storage.read_keymap(&mut data, &mut behavior).await.unwrap();

            let positional = crate::config::PositionalConfig::<1, 1>::default();
            let keymap = KeyMap::new(&mut data, &mut behavior, &positional).await;

            // The freshly built keymap exposes the stored value to the via
            // GET handler.
            assert_eq!(keymap.layout_option(), 42);
        });
    }
}
