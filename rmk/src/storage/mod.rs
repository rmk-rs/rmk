use core::fmt::Debug;
use core::future::{Future, poll_fn};

use embassy_embedded_hal::adapter::BlockingAsync;
use embassy_sync::channel::{Channel, TrySendError};
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embedded_storage::nor_flash::NorFlash;
use embedded_storage_async::nor_flash::NorFlash as AsyncNorFlash;
use rmk_types::connection::ConnectionType;
use rmk_types::morse::MorseProfile;
use sequential_storage::Error as SSError;
use sequential_storage::cache::Cache;
use sequential_storage::cache::key_pointers::ArrayKeyPointers;
use sequential_storage::cache::page_pointers::ArrayPagePointers;
use sequential_storage::cache::page_states::CalculatedPageStates;
use sequential_storage::map::{Key as MapKey, MapConfig, MapStorage, PostcardValue, SerializationError};
#[cfg(feature = "host")]
use {
    crate::config::HoldTriggerPositions,
    crate::keyboard::combo::ComboConfig,
    rmk_types::action::{EncoderAction, KeyAction},
    rmk_types::constants::MACRO_CHUNK_SIZE,
    rmk_types::fork::Fork,
    rmk_types::morse::Morse,
};

#[cfg(feature = "_ble")]
use crate::ble::profile::ProfileInfo;
use crate::boot::reboot_keyboard;
use crate::config;
use crate::config::StorageConfig;
#[cfg(all(feature = "_ble", feature = "split"))]
use crate::split::ble::PeerAddress;

/// An operation request to the `Storage` task.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub(crate) enum FlashOperationMessage {
    /// Save a [`StorageItem`] to the storage.
    Store(StorageItem),
    /// Read a stored value by [`StorageKey`].
    Read(StorageKey),
    /// Fully erase and reset the storage.
    Reset,
}

// Requests to the storage task, handled in order (FIFO). Each carries the id `REPLY` answers it
// under, `None` when nobody waits. Send through `store`/`store_unchecked`/`read`/`reset`.
static FLASH_CHANNEL: Channel<crate::RawMutex, (FlashOperationMessage, Option<u8>), { crate::FLASH_CHANNEL_SIZE }> =
    Channel::new();
/// The in-flight request's id: the lock hands it out and holds it until the reply arrives,
/// so only one request waits on `REPLY` at a time.
static REQUEST_ID: Mutex<crate::RawMutex, u8> = Mutex::new(0);
/// Storage's reply of a request, `(request id, reply)`.
static REPLY: Signal<crate::RawMutex, (u8, Result<Option<StorageValue>, ()>)> = Signal::new();

/// Send `message` to the storage task and wait for its reply.
/// Not an `async fn`, and no `send` future: either would hold a second copy of `message`.
#[allow(clippy::manual_async_fn)]
fn request(mut message: FlashOperationMessage) -> impl Future<Output = Result<Option<StorageValue>, ()>> {
    async move {
        let mut id = REQUEST_ID.lock().await;
        *id = id.wrapping_add(1);
        loop {
            poll_fn(|cx| FLASH_CHANNEL.poll_ready_to_send(cx)).await;
            match FLASH_CHANNEL.try_send((message, Some(*id))) {
                Ok(()) => break,
                Err(TrySendError::Full((rejected, _))) => message = rejected,
            }
        }
        // A predecessor cancelled after sending leaves its reply in the slot first: skip it by id.
        loop {
            let (replied, reply) = REPLY.wait().await;
            if replied == *id {
                return reply;
            }
        }
    }
}

/// Write `item`, resolving once it has landed on flash.
pub(crate) fn store(item: StorageItem) -> impl Future<Output = Result<Option<StorageValue>, ()>> {
    request(FlashOperationMessage::Store(item))
}

/// Write `item` without waiting for the result.
pub(crate) fn store_unchecked(item: StorageItem) -> impl Future<Output = ()> {
    FLASH_CHANNEL.send((FlashOperationMessage::Store(item), None))
}

/// Read a stored item.
pub(crate) fn read(key: StorageKey) -> impl Future<Output = Result<Option<StorageValue>, ()>> {
    request(FlashOperationMessage::Read(key))
}

/// Erase everything and reboot. Fire and forget.
pub(crate) fn reset() -> impl Future<Output = ()> {
    FLASH_CHANNEL.send((FlashOperationMessage::Reset, None))
}

/// The most one user slot holds. Changing it reframes stored values, but the new
/// commit also changes [`SCHEMA_HASH`], so the next boot reinitializes on its own.
pub const USER_DATA_MAX_SIZE: usize = 16;

/// Persist user-defined `bytes` in board-defined slot `slot`. RMK never looks inside one.
///
/// `Err` when `bytes` is longer than [`USER_DATA_MAX_SIZE`].
pub async fn store_user_data(slot: u8, bytes: &[u8]) -> Result<(), heapless::CapacityError> {
    let data = heapless::Vec::from_slice(bytes)?;
    store_unchecked(StorageItem::UserData { slot, data }).await;
    Ok(())
}

/// Read back slot `slot`, `None` if nothing was ever stored there.
///
/// Answered by the storage task, so it only works once `Storage` is running.
pub async fn read_user_data(slot: u8) -> Option<heapless::Vec<u8, USER_DATA_MAX_SIZE>> {
    match read(StorageKey::UserData(slot)).await {
        Ok(Some(StorageValue::UserData(data))) => Some(data),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum StorageKey {
    StorageConfig,
    DefaultLayer,
    LayoutOption,
    BehaviorConfig,
    ConnectionType,
    /// One `MACRO_CHUNK_SIZE`-byte piece of the macro buffer, keyed by its index.
    #[cfg(feature = "host")]
    MacroChunk(u8),
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
    /// A slot the board defines, see [`store_user_data`].
    UserData(u8),
    #[cfg(feature = "host")]
    MorseHoldTriggerPositions,
}

/// A Storage item is actually a storage (key, value) pair.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub(crate) enum StorageItem {
    StorageConfig(u32),
    DefaultLayer(u8),
    LayoutOption(u32),
    BehaviorConfig(BehaviorConfig),
    ConnectionType(ConnectionType),
    #[cfg(feature = "host")]
    MacroChunk {
        idx: u8,
        bytes: [u8; MACRO_CHUNK_SIZE],
    },
    #[cfg(feature = "host")]
    Keymap {
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
    #[cfg(all(feature = "_ble", feature = "split"))]
    PeerAddress(PeerAddress),
    #[cfg(feature = "_ble")]
    BondInfo(ProfileInfo),
    #[cfg(feature = "_ble")]
    ActiveBleProfile(u8),
    UserData {
        slot: u8,
        data: heapless::Vec<u8, USER_DATA_MAX_SIZE>,
    },
    #[cfg(feature = "host")]
    MorseHoldTriggerPositions(HoldTriggerPositions),
}

impl StorageItem {
    fn split(self) -> (StorageKey, StorageValue) {
        match self {
            Self::StorageConfig(v) => (StorageKey::StorageConfig, StorageValue::StorageConfig(v)),
            Self::DefaultLayer(v) => (StorageKey::DefaultLayer, StorageValue::DefaultLayer(v)),
            Self::LayoutOption(v) => (StorageKey::LayoutOption, StorageValue::LayoutOption(v)),
            Self::BehaviorConfig(v) => (StorageKey::BehaviorConfig, StorageValue::BehaviorConfig(v)),
            Self::ConnectionType(v) => (StorageKey::ConnectionType, StorageValue::ConnectionType(v)),
            #[cfg(feature = "host")]
            Self::MacroChunk { idx, bytes } => (StorageKey::MacroChunk(idx), StorageValue::MacroChunk(bytes)),
            #[cfg(feature = "host")]
            Self::Keymap {
                layer,
                row,
                col,
                action,
            } => (StorageKey::Keymap { layer, row, col }, StorageValue::KeyAction(action)),
            #[cfg(feature = "host")]
            Self::Encoder { layer, idx, action } => {
                (StorageKey::Encoder { layer, idx }, StorageValue::EncoderAction(action))
            }
            #[cfg(feature = "host")]
            Self::Combo { idx, config } => (StorageKey::Combo(idx), StorageValue::Combo(config)),
            #[cfg(feature = "host")]
            Self::Fork { idx, fork } => (StorageKey::Fork(idx), StorageValue::Fork(fork)),
            #[cfg(feature = "host")]
            Self::Morse { idx, morse } => (StorageKey::Morse(idx), StorageValue::Morse(morse)),
            #[cfg(all(feature = "_ble", feature = "split"))]
            Self::PeerAddress(v) => (StorageKey::PeerAddress(v.peer_id), StorageValue::PeerAddress(v)),
            #[cfg(feature = "_ble")]
            Self::BondInfo(v) => (StorageKey::BondInfo(v.slot_num), StorageValue::BondInfo(v)),
            #[cfg(feature = "_ble")]
            Self::ActiveBleProfile(v) => (StorageKey::ActiveBleProfile, StorageValue::ActiveBleProfile(v)),
            Self::UserData { slot, data } => (StorageKey::UserData(slot), StorageValue::UserData(data)),
            #[cfg(feature = "host")]
            Self::MorseHoldTriggerPositions(v) => (
                StorageKey::MorseHoldTriggerPositions,
                StorageValue::MorseHoldTriggerPositions(v),
            ),
        }
    }
}

impl MapKey for StorageKey {
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
pub(crate) enum StorageValue {
    /// The [`SCHEMA_HASH`] of the firmware that wrote this storage.
    StorageConfig(u32),
    DefaultLayer(u8),
    LayoutOption(u32),
    BehaviorConfig(BehaviorConfig),
    ConnectionType(ConnectionType),
    #[cfg(feature = "host")]
    MacroChunk([u8; MACRO_CHUNK_SIZE]),
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
    UserData(heapless::Vec<u8, USER_DATA_MAX_SIZE>),
    #[cfg(feature = "host")]
    MorseHoldTriggerPositions(HoldTriggerPositions),
}

impl<'a> PostcardValue<'a> for StorageValue {}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct BehaviorConfig {
    // Timeouts and intervals are stored as milliseconds.
    pub(crate) prior_idle_time: u16,
    pub(crate) morse_default_profile: MorseProfile,
    pub(crate) combo_timeout: u16,
    pub(crate) one_shot_timeout: u16,
    pub(crate) tap_interval: u16,
    // macOS treats capslock specially, so tapping it needs its own interval
    pub(crate) tap_capslock_interval: u16,
}

impl From<&config::BehaviorConfig> for BehaviorConfig {
    fn from(behavior: &config::BehaviorConfig) -> Self {
        // default_layer persists under its own key (restored in `read_keymap`), not here.
        Self {
            prior_idle_time: behavior.morse.prior_idle_time.as_millis() as u16,
            morse_default_profile: behavior.morse.default_profile,
            combo_timeout: behavior.combo.timeout.as_millis() as u16,
            one_shot_timeout: behavior.one_shot.timeout.as_millis() as u16,
            tap_interval: behavior.tap.tap_interval,
            tap_capslock_interval: behavior.tap.tap_capslock_interval,
        }
    }
}

pub fn async_flash_wrapper<F: NorFlash>(flash: F) -> BlockingAsync<F> {
    embassy_embedded_hal::adapter::BlockingAsync::new(flash)
}

/// Storage for the firmwares that hold no keymap of their own, a split peripheral and a dongle.
#[cfg(any(feature = "split", feature = "dongle"))]
pub async fn new_storage_without_keymap<F: AsyncNorFlash>(
    flash: F,
    storage_config: StorageConfig,
) -> Storage<F, 0, 0, 0, 0> {
    Storage::<F, 0, 0, 0, 0>::new(flash, &storage_config).await
}

/// The FNV-1a offset basis, the seed [`SCHEMA_HASH`] folds its byte runs into.
const FNV_OFFSET: u32 = 0x811c_9dc5;

/// FNV-1a over `bytes`, continuing `hash`.
const fn fnv_hash(mut hash: u32, bytes: &[u8]) -> u32 {
    let mut i = 0;
    while i < bytes.len() {
        hash = (hash ^ bytes[i] as u32).wrapping_mul(0x0100_0193);
        i += 1;
    }
    hash
}

/// FNV-1a over everything that frames stored bytes: rmk version, commit and features.
/// A mismatch could decode an item as the wrong variant, so the storage is erased.
pub(crate) const SCHEMA_HASH: u32 = {
    let mut hash = fnv_hash(FNV_OFFSET, env!("CARGO_PKG_VERSION").as_bytes());
    hash = fnv_hash(hash, env!("RMK_COMMIT").as_bytes());
    // Features gate variants of the two enums, shifting their postcard tags.
    hash = fnv_hash(hash, env!("RMK_FEATURES").as_bytes());
    // `keyboard.toml` sizes decide how a stored value is framed.
    #[cfg(feature = "host")]
    {
        hash = fnv_hash(hash, &(crate::MACRO_SPACE_SIZE as u32).to_le_bytes());
        hash = fnv_hash(hash, &(crate::COMBO_SIZE as u32).to_le_bytes());
        hash = fnv_hash(hash, &(crate::MORSE_SIZE as u32).to_le_bytes());
        hash = fnv_hash(hash, &(crate::HOLD_TRIGGER_KEY_POSITION_MAX_NUM as u32).to_le_bytes());
    }
    hash
};

pub struct Storage<
    F: AsyncNorFlash,
    const ROW: usize,
    const COL: usize,
    const NUM_LAYER: usize,
    const NUM_ENCODER: usize = 0,
> {
    pub(crate) flash: MapStorage<
        StorageKey,
        F,
        Cache<CalculatedPageStates, ArrayPagePointers<32>, ArrayKeyPointers<StorageKey, 32>, StorageKey>,
    >,
    pub(crate) buffer: [u8; BUFFER_SIZE],
    #[cfg(feature = "host")]
    pub(crate) clear_layout: bool,
}

impl<F: AsyncNorFlash, const ROW: usize, const COL: usize, const NUM_LAYER: usize, const NUM_ENCODER: usize>
    Storage<F, ROW, COL, NUM_LAYER, NUM_ENCODER>
{
    pub(crate) async fn fetch(&mut self, key: StorageKey) -> Result<Option<StorageValue>, ()> {
        self.flash
            .fetch_item(&mut self.buffer, &key)
            .await
            .map_err(|e| print_storage_error::<F>(e))
    }

    // Like `store`: split first so the future holds the pair, not the pair and `item`.
    fn put(&mut self, item: StorageItem) -> impl Future<Output = Result<(), SSError<F::Error>>> {
        let (key, value) = item.split();
        async move {
            self.flash
                .store_item(&mut self.buffer, &key, &value)
                .await
                .inspect_err(|_| error!("Failed to store {:?}", key))
        }
    }

    pub async fn new(flash: F, storage_config: &StorageConfig) -> Self {
        assert!(
            storage_config.num_sectors >= 2,
            "Number of used sector for storage must larger than 1"
        );

        // `start_addr == 0` means the last `num_sectors` sectors, except on nRF BLE builds without
        // DFU, which keep the historical 0x6_0000; with DFU rmk-boot places the partition.
        #[cfg(all(feature = "_nrf_ble", not(feature = "_dfu")))]
        let start_addr = if storage_config.start_addr == 0 {
            0x0006_0000
        } else {
            storage_config.start_addr
        };
        #[cfg(not(all(feature = "_nrf_ble", not(feature = "_dfu"))))]
        let start_addr = storage_config.start_addr;

        let storage_range = if start_addr == 0 {
            (flash.capacity() - storage_config.num_sectors as usize * F::ERASE_SIZE) as u32..flash.capacity() as u32
        } else {
            assert!(
                start_addr.is_multiple_of(F::ERASE_SIZE),
                "Storage's start addr MUST BE a multiplier of sector size"
            );
            start_addr as u32..(start_addr + storage_config.num_sectors as usize * F::ERASE_SIZE) as u32
        };
        info!(
            "Flash capacity {} KB, RMK use {} KB({} sectors) starting from 0x{:X} as storage",
            flash.capacity() / 1024,
            (F::ERASE_SIZE * storage_config.num_sectors as usize) / 1024,
            storage_config.num_sectors,
            storage_range.start,
        );

        let cache = || {
            Cache::new(
                CalculatedPageStates::new(storage_config.num_sectors as usize),
                ArrayPagePointers::new(),
                ArrayKeyPointers::new(),
            )
        };
        let mut storage = Self {
            flash: MapStorage::new(flash, MapConfig::new(storage_range.clone()), cache()),
            buffer: [0; BUFFER_SIZE],
            #[cfg(feature = "host")]
            clear_layout: false,
        };

        let stored = storage.fetch(StorageKey::StorageConfig).await;
        if storage_config.clear_storage
            || !matches!(stored, Ok(Some(StorageValue::StorageConfig(schema_hash))) if schema_hash == SCHEMA_HASH)
        {
            debug!("Clearing storage!");
            // An erase never invalidates the cache the probing `fetch` filled, so rebuild the map
            // with a fresh one. Nothing else is written back: reads now miss and the RAM defaults stand.
            let (mut raw, _) = storage.flash.destroy();
            let _ = raw.erase(storage_range.start, storage_range.end).await;
            storage.flash = MapStorage::new(raw, MapConfig::new(storage_range), cache());
            let _ = storage.put(StorageItem::StorageConfig(SCHEMA_HASH)).await;
            // The erase already handed the compiled-in layout the win: reads miss and the RAM
            // defaults stand, so there is nothing for `clear_layout` to overwrite.
        } else {
            #[cfg(feature = "host")]
            {
                storage.clear_layout = storage_config.clear_layout;
            }
        }

        storage
    }

    /// Overwrite every item the layout owns with the compiled-in defaults, so a value a host
    /// wrote earlier stops shadowing what was flashed. Only `clear_layout` reaches here.
    #[cfg(feature = "host")]
    pub(crate) async fn write_layout(
        &mut self,
        data: &mut crate::keymap::KeymapData<ROW, COL, NUM_LAYER, NUM_ENCODER>,
        behavior: &config::BehaviorConfig,
    ) {
        let mut put = async |item| {
            if let Err(e) = self.put(item).await {
                print_storage_error::<F>(e);
            }
        };
        put(StorageItem::BehaviorConfig(behavior.into())).await;
        put(StorageItem::MorseHoldTriggerPositions(
            behavior.morse.hold_trigger_positions.clone(),
        ))
        .await;
        put(StorageItem::DefaultLayer(0)).await;
        put(StorageItem::LayoutOption(0)).await;

        for (layer, layer_data) in data.keymap.iter().enumerate() {
            for (row, row_data) in layer_data.iter().enumerate() {
                for (col, action) in row_data.iter().enumerate() {
                    put(StorageItem::Keymap {
                        layer: layer as u8,
                        row: row as u8,
                        col: col as u8,
                        action: *action,
                    })
                    .await;
                }
            }
        }
        // `NUM_ENCODER == 0` leaves every inner array empty, so a keyboard without encoders
        // writes nothing here.
        for (layer, layer_data) in data.encoder_map.iter().enumerate() {
            for (idx, action) in layer_data.iter().enumerate() {
                put(StorageItem::Encoder {
                    layer: layer as u8,
                    idx: idx as u8,
                    action: *action,
                })
                .await;
            }
        }
        // An empty slot is written as an empty config, so a combo the user added over the host
        // protocol is cleared, not left behind.
        for (idx, combo) in behavior.combo.combos.iter().enumerate() {
            let config = combo.as_ref().map_or_else(ComboConfig::empty, |c| c.config.clone());
            put(StorageItem::Combo { idx: idx as u8, config }).await;
        }
        for (idx, fork) in behavior.fork.forks.iter().enumerate() {
            put(StorageItem::Fork {
                idx: idx as u8,
                fork: *fork,
            })
            .await;
        }
        for (idx, morse) in behavior.morse.morses.iter().enumerate() {
            put(StorageItem::Morse {
                idx: idx as u8,
                morse: morse.clone(),
            })
            .await;
        }
        // The whole buffer, so a macro the user wrote over the host protocol is
        // replaced by its default or cleared.
        crate::keyboard::macros::encode_defaults(&mut data.macros, behavior.keyboard_macros);
        for (idx, bytes) in data.macros.as_chunks::<MACRO_CHUNK_SIZE>().0.iter().enumerate() {
            put(StorageItem::MacroChunk {
                idx: idx as u8,
                bytes: *bytes,
            })
            .await;
        }
        // Flash now has every chunk, so the next save writes only what changed, zeros included.
        data.macros_stored = true;
    }
}

impl<F: AsyncNorFlash, const ROW: usize, const COL: usize, const NUM_LAYER: usize, const NUM_ENCODER: usize>
    crate::core_traits::Runnable for Storage<F, ROW, COL, NUM_LAYER, NUM_ENCODER>
{
    async fn run(&mut self) -> ! {
        loop {
            let (message, reply_id) = FLASH_CHANNEL.receive().await;
            let result = match message {
                FlashOperationMessage::Store(item) => {
                    self.put(item).await.map(|_| None).map_err(print_storage_error::<F>)
                }
                FlashOperationMessage::Read(key) => self.fetch(key).await,
                FlashOperationMessage::Reset => {
                    let _ = self.flash.erase_all().await;
                    reboot_keyboard();
                    // Only `std` returns from reboot; the cache no longer describes the flash, so serve nothing.
                    core::future::pending().await
                }
            };
            if let Some(id) = reply_id {
                REPLY.signal((id, result));
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

/// Holds any one item with its key and framing; a multiple of 32 because
/// `sequential-storage` wants that alignment on some flashes.
const BUFFER_SIZE: usize = 256;

/// Test-only: forget queued requests and any pending reply, so a test starts clean.
#[cfg(any(test, feature = "std"))]
pub(crate) fn clear_flash_channel() {
    FLASH_CHANNEL.clear();
    REPLY.reset();
}

/// Test-only stand-in for the storage task when a simulation has no flash: every
/// write lands, every read is absent, so nothing blocks on a never-serviced queue.
#[cfg(any(test, feature = "std"))]
pub(crate) async fn drain_flash_channel() {
    loop {
        if let (_, Some(id)) = FLASH_CHANNEL.receive().await {
            REPLY.signal((id, Ok(None)))
        }
    }
}

#[cfg(test)]
mod tests {
    use core::future::Future;
    use core::pin::pin;
    use core::task::{Context, Poll, Waker};

    use embassy_futures::select::{Either, select};
    use sequential_storage::cache::Cache;
    use sequential_storage::map::{MapConfig, MapStorage};

    use super::*;
    use crate::config::{BehaviorConfig as RuntimeBehaviorConfig, StorageConfig as RuntimeStorageConfig};
    use crate::test_support::test_block_on as block_on;

    /// 16 KB of byte-writable flash in 4 KB sectors, the geometry `STORAGE_RANGE` is cut from.
    type Part = crate::test_support::InMemoryFlash<16_384, 4_096, 1>;
    type TestFlash = BlockingAsync<Part>;

    // The primitive tests below poll by hand: `test_block_on` re-polls with a noop waker
    // every step, so it would also pass a primitive that loses wake-ups.
    fn take_request_id() -> u8 {
        match FLASH_CHANNEL.try_receive() {
            Ok((_, Some(id))) => id,
            other => panic!("expected a request that wants a reply, got {other:?}"),
        }
    }

    #[test]
    fn request_skips_a_cancelled_predecessors_reply() {
        let mut cx = Context::from_waker(Waker::noop());
        crate::test_support::clear_flash_channel();

        let stale = {
            let mut first = pin!(read(StorageKey::StorageConfig));
            assert!(first.as_mut().poll(&mut cx).is_pending());
            take_request_id()
            // Dropped after `send`: the lock is released, the reply still arrives.
        };

        let mut second = pin!(read(StorageKey::StorageConfig));
        assert!(second.as_mut().poll(&mut cx).is_pending());
        let live = take_request_id();
        assert_ne!(stale, live);

        REPLY.signal((stale, Ok(None)));
        assert!(second.as_mut().poll(&mut cx).is_pending());
        REPLY.signal((live, Ok(None)));
        assert!(matches!(second.as_mut().poll(&mut cx), Poll::Ready(Ok(None))));
    }

    #[test]
    fn second_requester_waits_for_the_first_reply() {
        let mut cx = Context::from_waker(Waker::noop());
        crate::test_support::clear_flash_channel();

        let mut first = pin!(read(StorageKey::StorageConfig));
        let mut second = pin!(store(StorageItem::LayoutOption(1)));
        assert!(first.as_mut().poll(&mut cx).is_pending());
        assert!(second.as_mut().poll(&mut cx).is_pending());
        // Only the lock holder's message is in flight.
        let id = take_request_id();
        assert!(FLASH_CHANNEL.try_receive().is_err());

        REPLY.signal((id, Ok(None)));
        assert!(matches!(first.as_mut().poll(&mut cx), Poll::Ready(Ok(None))));
        assert!(second.as_mut().poll(&mut cx).is_pending());
        assert!(matches!(
            FLASH_CHANNEL.try_receive(),
            Ok((FlashOperationMessage::Store(_), Some(_)))
        ));
    }

    #[cfg(all(feature = "_ble", feature = "split"))]
    #[test]
    fn peer_address_write_waits_for_its_own_reply() {
        let mut cx = Context::from_waker(Waker::noop());
        crate::test_support::clear_flash_channel();
        FLASH_CHANNEL
            .try_send((FlashOperationMessage::Store(StorageItem::LayoutOption(42)), None))
            .unwrap();

        let mut write = pin!(store(StorageItem::PeerAddress(PeerAddress::new(0, true, [1; 6]))));
        assert!(write.as_mut().poll(&mut cx).is_pending());

        // The storage task sees the older fire-and-forget write first, then this one.
        assert!(matches!(
            FLASH_CHANNEL.try_receive(),
            Ok((FlashOperationMessage::Store(StorageItem::LayoutOption(42)), None))
        ));
        let id = match FLASH_CHANNEL.try_receive() {
            Ok((FlashOperationMessage::Store(StorageItem::PeerAddress(_)), Some(id))) => id,
            other => panic!("expected the peer address write, got {other:?}"),
        };
        assert!(write.as_mut().poll(&mut cx).is_pending());
        REPLY.signal((id, Ok(None)));
        assert!(matches!(write.as_mut().poll(&mut cx), Poll::Ready(Ok(None))));
    }

    // Boxed: the flash part is 16 KB by value and the `new` future copies it several times.
    async fn new_storage(flash: TestFlash) -> Storage<TestFlash, 1, 1, 1, 0> {
        new_storage_configured(flash, &RuntimeStorageConfig::default()).await
    }

    async fn new_storage_configured(
        flash: TestFlash,
        storage_config: &RuntimeStorageConfig,
    ) -> Storage<TestFlash, 1, 1, 1, 0> {
        Box::pin(Storage::<TestFlash, 1, 1, 1, 0>::new(flash, storage_config)).await
    }

    /// A config item written by some other firmware.
    const STALE_CONFIG: StorageValue = StorageValue::StorageConfig(0);

    const STORAGE_RANGE: core::ops::Range<u32> = (16_384 - 2 * 4_096) as u32..16_384u32;

    /// A flash holding `items`, written by an uncached map so `Storage::new` boots over them.
    async fn seeded(items: &[(StorageKey, StorageValue)]) -> TestFlash {
        let mut map = MapStorage::<StorageKey, _, _>::new(
            async_flash_wrapper(Part::new()),
            MapConfig::new(STORAGE_RANGE),
            Cache::new_uncached(),
        );
        let mut buffer = [0u8; 256];
        for (key, data) in items {
            map.store_item(&mut buffer, key, data).await.unwrap();
        }
        map.destroy().0
    }

    /// Run `body` against a live storage task over `part`. A clone is enough:
    /// every clone shares the same bytes.
    fn with_storage_task<T>(part: Part, body: impl Future<Output = T>) -> T {
        use crate::core_traits::Runnable;

        crate::test_support::clear_flash_channel();
        block_on(async {
            let mut storage = new_storage(async_flash_wrapper(part)).await;
            match select(storage.run(), body).await {
                Either::First(never) => never,
                Either::Second(out) => out,
            }
        })
    }

    #[test]
    fn read_sees_write_queued_before_it() {
        with_storage_task(Part::new(), async {
            store_unchecked(StorageItem::ConnectionType(ConnectionType::Usb)).await;
            assert!(matches!(
                read(StorageKey::ConnectionType).await,
                Ok(Some(StorageValue::ConnectionType(ConnectionType::Usb)))
            ));
            store_unchecked(StorageItem::ConnectionType(ConnectionType::Ble)).await;
            assert!(matches!(
                read(StorageKey::ConnectionType).await,
                Ok(Some(StorageValue::ConnectionType(ConnectionType::Ble)))
            ));
        });
    }

    #[test]
    fn user_data_round_trips_through_its_slot() {
        with_storage_task(Part::new(), async {
            assert_eq!(read_user_data(3).await, None, "an untouched slot reads back empty");

            store_user_data(3, &[0xAA, 0x55]).await.unwrap();
            assert_eq!(read_user_data(3).await.as_deref(), Some(&[0xAA, 0x55][..]));

            // Slots are independent, and a second store replaces the first.
            store_user_data(4, &[1]).await.unwrap();
            store_user_data(3, &[9, 8, 7]).await.unwrap();
            assert_eq!(read_user_data(3).await.as_deref(), Some(&[9, 8, 7][..]));
            assert_eq!(read_user_data(4).await.as_deref(), Some(&[1][..]));

            assert!(
                store_user_data(3, &[0; USER_DATA_MAX_SIZE + 1]).await.is_err(),
                "too long for a slot, and nothing is written"
            );
        });
    }

    #[test]
    fn store_reports_its_own_failure() {
        let part = Part::new();
        with_storage_task(part.clone(), async {
            part.fail_writes(true);
            assert!(store(StorageItem::ConnectionType(ConnectionType::Usb)).await.is_err());
            // A failed write is not sticky: the next one answers for itself.
            part.fail_writes(false);
            assert!(store(StorageItem::ConnectionType(ConnectionType::Usb)).await.is_ok());
            assert!(read(StorageKey::ConnectionType).await.is_ok());
        });
    }

    // Without the rebuild in `Storage::new`, the cache still describes the pre-erase page layout
    // and the first store lands in a page whose marker is gone, invisible to an uncached map.
    #[test]
    fn reinit_writes_survive_a_fresh_map() {
        block_on(async {
            let flash = seeded(&[(StorageKey::StorageConfig, STALE_CONFIG)]).await;
            let mut storage = new_storage(flash).await;
            storage
                .put(StorageItem::ConnectionType(ConnectionType::Ble))
                .await
                .unwrap();

            let (flash, _) = storage.flash.destroy();
            let mut fresh =
                MapStorage::<StorageKey, _, _>::new(flash, MapConfig::new(STORAGE_RANGE), Cache::new_uncached());
            let mut buffer = [0u8; 256];
            assert!(matches!(
                fresh
                    .fetch_item::<StorageValue>(&mut buffer, &StorageKey::ConnectionType)
                    .await,
                Ok(Some(StorageValue::ConnectionType(ConnectionType::Ble)))
            ));
        });
    }

    #[test]
    fn firmware_mismatch_reinitializes_storage() {
        block_on(async {
            let flash = seeded(&[
                (StorageKey::StorageConfig, STALE_CONFIG),
                (StorageKey::DefaultLayer, StorageValue::DefaultLayer(7)),
                (StorageKey::LayoutOption, StorageValue::LayoutOption(42)),
            ])
            .await;
            let mut storage = new_storage(flash).await;

            // The mismatch wiped the layout items; the config item was rewritten for this firmware.
            assert!(matches!(storage.fetch(StorageKey::DefaultLayer).await, Ok(None)));
            assert!(matches!(storage.fetch(StorageKey::LayoutOption).await, Ok(None)));
            assert!(matches!(
                storage.fetch(StorageKey::StorageConfig).await,
                Ok(Some(StorageValue::StorageConfig(schema_hash))) if schema_hash == SCHEMA_HASH
            ));
        });
    }

    /// A schema mismatch is the only thing that erases. It has to take the pairings with
    /// it, because an item written under another schema can decode as the wrong variant.
    #[test]
    fn schema_mismatch_drops_even_the_pairings() {
        block_on(async {
            let flash = seeded(&[
                (StorageKey::StorageConfig, STALE_CONFIG),
                (
                    StorageKey::ConnectionType,
                    StorageValue::ConnectionType(ConnectionType::Ble),
                ),
            ])
            .await;
            let mut storage = new_storage(flash).await;
            assert!(matches!(storage.fetch(StorageKey::ConnectionType).await, Ok(None)));
        });
    }

    /// Storage outranks the compiled-in keymap: an edit made over the host protocol survives
    /// both a reboot and a firmware built from another `keyboard.toml`. `clear_layout` is the
    /// only way to hand the compiled-in layout back the win, and it keeps the pairings.
    #[cfg(feature = "host")]
    #[test]
    fn stored_keymap_outranks_the_compiled_one_until_clear_layout() {
        use rmk_types::action::Action;
        use rmk_types::keycode::{HidKeyCode, KeyCode};

        use crate::keymap::KeymapData;

        let a = KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::A)));
        let b = KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::B)));
        let mut behavior = RuntimeBehaviorConfig::default();

        block_on(async {
            let mut storage = new_storage(async_flash_wrapper(Part::new())).await;
            storage
                .put(StorageItem::Keymap {
                    layer: 0,
                    row: 0,
                    col: 0,
                    action: a,
                })
                .await
                .unwrap();
            storage
                .put(StorageItem::ConnectionType(ConnectionType::Ble))
                .await
                .unwrap();

            // A reboot into a firmware compiled with the other keymap: the stored edit still wins.
            let (flash, _) = storage.flash.destroy();
            let mut storage = new_storage(flash).await;
            let mut data = KeymapData::new([[[b]]]);
            storage.read_keymap(&mut data, &mut behavior).await.unwrap();
            assert_eq!(data.keymap[0][0][0], a);

            // `clear_layout` hands the compiled-in layout the win, and keeps the pairing.
            let clear_layout = RuntimeStorageConfig {
                clear_layout: true,
                ..RuntimeStorageConfig::default()
            };
            let (flash, _) = storage.flash.destroy();
            let mut storage = new_storage_configured(flash, &clear_layout).await;
            assert!(storage.clear_layout, "a matching schema arms the layout rewrite");
            storage.write_layout(&mut KeymapData::new([[[b]]]), &behavior).await;

            let mut data = KeymapData::new([[[a]]]);
            storage.read_keymap(&mut data, &mut behavior).await.unwrap();
            assert_eq!(data.keymap[0][0][0], b);
            assert!(matches!(
                storage.fetch(StorageKey::ConnectionType).await,
                Ok(Some(StorageValue::ConnectionType(ConnectionType::Ble)))
            ));
        });
    }

    /// A `clear_layout` boot leaves every macro chunk in flash, so a later save must also
    /// write the chunks it empties, or the next boot loads the defaults' bytes back.
    #[cfg(feature = "host")]
    #[test]
    fn macro_edit_after_clear_layout_survives_the_next_boot() {
        use rmk_types::keyboard_macros::MacroOp;

        use crate::config::PositionalConfig;
        use crate::core_traits::Runnable;
        use crate::keymap::{KeyMap, KeymapData};

        // 40 characters, so the default spills into chunk 1.
        static LONG: [MacroOp; 40] = [MacroOp::Char(b'a'); 40];
        static DEFAULTS: [&[MacroOp]; 1] = [&LONG];
        let behavior = || RuntimeBehaviorConfig {
            keyboard_macros: &DEFAULTS,
            ..RuntimeBehaviorConfig::default()
        };
        let positional = PositionalConfig::<1, 1>::default();
        let clear_layout = RuntimeStorageConfig {
            clear_layout: true,
            ..RuntimeStorageConfig::default()
        };

        block_on(async {
            // The first boot writes this firmware's schema, so the second honours `clear_layout`.
            let storage = new_storage(async_flash_wrapper(Part::new())).await;
            let (flash, _) = storage.flash.destroy();
            let mut storage = new_storage_configured(flash, &clear_layout).await;
            let mut data = KeymapData::new([[[KeyAction::No]]]);
            let mut b = behavior();
            let keymap = KeyMap::new_from_storage(&mut data, Some(&mut storage), &mut b, &positional).await;

            // The host shortens macro 0, emptying chunk 1.
            let mut shorter = [0u8; 2 * MACRO_CHUNK_SIZE];
            shorter[0] = b'b';
            crate::test_support::clear_flash_channel();
            let save = async {
                let changed = keymap.macros(|m| m.write(0, &shorter));
                crate::keyboard::macros::persist(&keymap, changed).await.unwrap();
                keymap.macros(|m| m.bytes().to_vec())
            };
            let saved = match select(storage.run(), save).await {
                Either::First(never) => never,
                Either::Second(bytes) => bytes,
            };

            let (flash, _) = storage.flash.destroy();
            let mut storage = new_storage(flash).await;
            let mut data = KeymapData::new([[[KeyAction::No]]]);
            let mut b = behavior();
            let keymap = KeyMap::new_from_storage(&mut data, Some(&mut storage), &mut b, &positional).await;
            assert_eq!(keymap.macros(|m| m.bytes().to_vec()), saved);
        });
    }

    // Postcard tags are declaration positions: inserting or reordering a variant shifts every
    // later tag and misreads storage written by the same commit. Both enums are append-only.
    #[test]
    fn storage_variant_order_is_pinned() {
        use sequential_storage::map::Value;

        let keys = [
            StorageKey::StorageConfig,
            StorageKey::DefaultLayer,
            StorageKey::LayoutOption,
            StorageKey::BehaviorConfig,
            StorageKey::ConnectionType,
            #[cfg(feature = "host")]
            StorageKey::MacroChunk(5),
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
            StorageKey::PeerAddress(9),
            #[cfg(feature = "_ble")]
            StorageKey::ActiveBleProfile,
            #[cfg(feature = "_ble")]
            StorageKey::BondInfo(10),
            StorageKey::UserData(11),
            #[cfg(feature = "host")]
            StorageKey::MorseHoldTriggerPositions,
        ];
        let mut buffer = [0u8; 64];
        for (tag, key) in keys.iter().enumerate() {
            let size = MapKey::serialize_into(key, &mut buffer).unwrap();
            assert_eq!(buffer[0], tag as u8, "{key:?}");
            assert_eq!(MapKey::deserialize_from(&buffer[..size]).unwrap(), (*key, size));
        }

        let data = [
            STALE_CONFIG,
            StorageValue::DefaultLayer(0),
            StorageValue::LayoutOption(0),
            StorageValue::BehaviorConfig((&RuntimeBehaviorConfig::default()).into()),
            StorageValue::ConnectionType(ConnectionType::Usb),
            #[cfg(feature = "host")]
            StorageValue::MacroChunk([0; MACRO_CHUNK_SIZE]),
            #[cfg(feature = "host")]
            StorageValue::KeyAction(KeyAction::No),
            #[cfg(feature = "host")]
            StorageValue::EncoderAction(EncoderAction::default()),
            #[cfg(feature = "host")]
            StorageValue::Combo(ComboConfig::empty()),
            #[cfg(feature = "host")]
            StorageValue::Fork(Fork::default()),
            #[cfg(feature = "host")]
            StorageValue::Morse(Morse::default()),
            #[cfg(all(feature = "_ble", feature = "split"))]
            StorageValue::PeerAddress(PeerAddress::new(0, false, [0; 6])),
            #[cfg(feature = "_ble")]
            StorageValue::BondInfo(ProfileInfo::default()),
            #[cfg(feature = "_ble")]
            StorageValue::ActiveBleProfile(0),
            StorageValue::UserData(heapless::Vec::new()),
            #[cfg(feature = "host")]
            StorageValue::MorseHoldTriggerPositions(HoldTriggerPositions::new()),
        ];
        let mut buffer = [0u8; BUFFER_SIZE];
        for (tag, item) in data.iter().enumerate() {
            Value::serialize_into(item, &mut buffer).unwrap();
            assert_eq!(buffer[0], tag as u8, "{item:?}");
        }
    }

    // A stored LayoutOption must reach the Vial GUI after a power cycle: `read_keymap` restores it
    // into `KeymapData`, `KeyMap::new` copies it where `GetKeyboardValue` reads. Drop either, get 0.
    #[cfg(feature = "vial")]
    #[test]
    fn layout_option_restored_from_storage() {
        use crate::config::BehaviorConfig;
        use crate::keymap::{KeyMap, KeymapData};

        block_on(async {
            // A matching config item keeps the stored records across the boot.
            let flash = seeded(&[
                (StorageKey::StorageConfig, StorageValue::StorageConfig(SCHEMA_HASH)),
                (StorageKey::LayoutOption, StorageValue::LayoutOption(42)),
            ])
            .await;
            let mut storage = new_storage(flash).await;

            let mut data = KeymapData::new([[[KeyAction::No]]]);
            let mut behavior = BehaviorConfig::default();
            storage.read_keymap(&mut data, &mut behavior).await.unwrap();

            let positional = crate::config::PositionalConfig::<1, 1>::default();
            let keymap = KeyMap::new(&mut data, &mut behavior, &positional).await;

            assert_eq!(keymap.layout_option(), 42);
        });
    }
}
