//! Manage BLE profiles and bonding information

#[cfg(feature = "_ble")]
use bt_hci::{cmd::le::LeSetPhy, controller::ControllerCmdAsync};
use embassy_futures::select::{Either, select, select_array};
use embassy_sync::signal::Signal;
use trouble_host::prelude::*;
use trouble_host::{BondInformation, LongTermKey};

use super::ble_server::CCCD_TABLE_SIZE;
use crate::NUM_BLE_PROFILE;
use crate::channel::BLE_PROFILE_CHANNEL;
use crate::state::{current_profile, set_ble_bonded, set_ble_profile};
#[cfg(feature = "storage")]
use crate::storage::{StorageItem, StorageKey, StorageValue, read, store};

pub(crate) static UPDATED_PROFILES: [Signal<crate::RawMutex, ProfileInfo>; BOND_SLOTS] =
    [const { Signal::new() }; BOND_SLOTS];

/// The dedicated dongle bond slot: the slot number after the normal profiles,
/// so pairing a dongle never touches a host bond. Selected only via the
/// `SwitchToDongle` key, never by profile cycling.
#[cfg(feature = "dongle")]
pub(crate) const DONGLE_PROFILE: u8 = NUM_BLE_PROFILE as u8;

/// Bond slots kept by the profile manager: the host profiles, plus the
/// dedicated dongle slot on `dongle` builds.
pub(crate) const BOND_SLOTS: usize = NUM_BLE_PROFILE + cfg!(feature = "dongle") as usize;

/// A fixed profile address that preserves the base address's random-address type bits.
pub(crate) fn profile_address(base: [u8; 6], profile: u8) -> [u8; 6] {
    let mut addr = base;
    addr[3] = addr[3].wrapping_add(profile);
    addr
}

/// BLE profile info
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ProfileInfo {
    pub(crate) slot_num: u8,
    pub(crate) removed: bool,
    pub(crate) info: BondInformation,
    /// Raw bytes of the trouble-host `ClientAttTable` for this peer.
    /// Reconstructed via `ClientAttTableView::try_from_raw` when applied to the stack.
    pub(crate) cccd_table: heapless::Vec<u8, CCCD_TABLE_SIZE>,
}

/// Returns the maximum number of bytes required to encode T.
pub const fn varint_max<T: Sized>() -> usize {
    const BITS_PER_BYTE: usize = 8;
    const BITS_PER_VARINT_BYTE: usize = 7;

    // How many data bits do we need for this type?
    let bits = core::mem::size_of::<T>() * BITS_PER_BYTE;

    // We add (BITS_PER_VARINT_BYTE - 1), to ensure any integer divisions
    // with a remainder will always add exactly one full byte, but
    // an evenly divided number of bits will be the same
    let roundup_bits = bits + (BITS_PER_VARINT_BYTE - 1);

    // Apply division, using normal "round down" integer division
    roundup_bits / BITS_PER_VARINT_BYTE
}

// Manual MaxSize implementation
impl postcard::experimental::max_size::MaxSize for ProfileInfo {
    const POSTCARD_MAX_SIZE: usize = varint_max::<Self>();
}

impl Default for ProfileInfo {
    fn default() -> Self {
        Self {
            slot_num: 0,
            removed: false,
            info: BondInformation::new(
                Identity {
                    addr: Address::default(),
                    irk: None,
                },
                LongTermKey(0),
                SecurityLevel::NoEncryption,
                false,
            ),
            cccd_table: heapless::Vec::new(),
        }
    }
}

/// Live bonding information for a profile slot, skipping cleared entries.
fn bond_info_of(bonded_devices: &[ProfileInfo], slot_num: u8) -> Option<&ProfileInfo> {
    bonded_devices
        .iter()
        .find(|info| !info.removed && info.slot_num == slot_num)
}

/// BLE profile switch action
#[derive(Debug)]
pub(crate) enum BleProfileAction {
    Switch(u8),
    Previous,
    Next,
    ClearBond,
    /// Clear the bond for an explicit slot, regardless of which slot is
    /// currently active. Rynk's `Cmd::ClearBleProfile` issues this so a host
    /// tool can wipe any bond without first switching to it.
    ClearSlot(u8),
}

/// Manage BLE profiles and bonding information
///
/// ProfileManager is responsible for:
/// 1. Managing multiple BLE profiles, allowing users to switch between multiple devices
/// 2. Storing and loading bonding information for each profile
/// 3. Updating the bonding information of the active profile to the BLE stack
/// 4. Handling profile switch, clear, and save operations
///
/// `SLOTS` sizes the cache to the role: [`BOND_SLOTS`] for a keyboard, a single
/// slot for a dongle.
#[cfg(feature = "_ble")]
pub(crate) struct ProfileManager<
    'b,
    's,
    C: Controller + ControllerCmdAsync<LeSetPhy>,
    P: PacketPool,
    const SLOTS: usize,
> where
    's: 'b,
{
    /// List of bonded devices
    bonded_devices: heapless::Vec<ProfileInfo, SLOTS>,
    /// Retained when the connection loop cancels a flash write.
    pending_save: Option<ProfileInfo>,
    /// BLE stack
    stack: &'b Stack<'s, C, P>,
}

#[cfg(feature = "_ble")]
impl<'b, 's, C: Controller + ControllerCmdAsync<LeSetPhy>, P: PacketPool, const SLOTS: usize>
    ProfileManager<'b, 's, C, P, SLOTS>
where
    's: 'b,
{
    /// Create a new profile manager
    pub(crate) fn new(stack: &'b Stack<'s, C, P>) -> Self {
        Self {
            bonded_devices: heapless::Vec::new(),
            pending_save: None,
            stack,
        }
    }

    /// Load stored bonding information
    #[cfg(feature = "storage")]
    pub(crate) async fn load_bonded_devices(&mut self) {
        self.bonded_devices.clear();
        for slot_num in 0..SLOTS {
            if let Ok(Some(StorageValue::BondInfo(info))) = read(StorageKey::BondInfo(slot_num as u8)).await
                && !info.removed
                && let Err(e) = self.bonded_devices.push(info)
            {
                error!("Failed to add bond info: {:?}", e);
            }
        }
        debug!("Loaded {} bond info", self.bonded_devices.len());

        let profile =
            if let Ok(Some(StorageValue::ActiveBleProfile(profile))) = read(StorageKey::ActiveBleProfile).await {
                debug!("Loaded active profile: {}", profile);
                profile
            } else {
                debug!("Loaded default active profile",);
                0
            };
        set_ble_profile(profile, self.is_bonded(profile));
    }

    fn is_bonded(&self, slot_num: u8) -> bool {
        bond_info_of(&self.bonded_devices, slot_num).is_some()
    }

    /// Cached bond info for `profile`, cloned to free the caller from borrow
    /// conflicts with concurrent `update_profile()`.
    pub(crate) fn bond_info(&self, profile: u8) -> Option<ProfileInfo> {
        bond_info_of(&self.bonded_devices, profile).cloned()
    }

    /// Check if the `identity` is the bonded dongle's identity.
    ///
    /// This function is used when searching for BLE host,
    /// the connected dongle(in the DONGLE_PROFILE) should be excluded.
    #[cfg(feature = "dongle")]
    pub(crate) fn is_bonded_dongle(&self, identity: &Identity) -> bool {
        self.bonded_devices.iter().any(|bond_info| {
            !bond_info.removed
                && bond_info.slot_num == DONGLE_PROFILE
                && bond_info.info.identity.match_identity(identity)
        })
    }

    /// Update bonding information in the stack according to the current active profile
    ///
    /// Also republishes `BleStatus::bonded`. Every change to `bonded_devices` that can
    /// alter bond presence is followed by this call, so the flag is derived here once
    /// instead of at each mutation site.
    pub(crate) fn update_stack_bonds(&self) {
        // Drain one at a time rather than collecting: the stack holds bonds this
        // manager has no slot for — a fresh pairing lands there before we prune —
        // and a `heapless::Vec` collect panics on the overflow.
        while let Some(identity) = self
            .stack
            .with_bond_information(|bonds| bonds.first().map(|b| b.identity))
        {
            if let Err(e) = self.stack.remove_bond_information(identity) {
                debug!("Remove bond info error: {:?}", e);
                break; // a bond that won't come off would spin here forever
            }
        }

        let active = self.bond_info(current_profile());
        set_ble_bonded(active.is_some());

        if let Some(info) = active {
            debug!("Add bond info of profile {}: {:?}", info.slot_num, info);
            if let Err(e) = self.stack.add_bond_information(info.info) {
                debug!("Add bond info error: {:?}", e);
            }
        }
    }

    /// Add/update bonding information
    pub(crate) async fn add_profile_info(&mut self, profile_info: ProfileInfo) {
        let bond_changed = if let Some(info) = self
            .bonded_devices
            .iter_mut()
            .find(|info| info.slot_num == profile_info.slot_num)
        {
            let bond_changed = info.removed || info.info != profile_info.info;
            if !bond_changed && info.cccd_table == profile_info.cccd_table {
                debug!("Skip saving unchanged profile");
                return;
            }
            *info = profile_info.clone();
            bond_changed
        } else if self.bonded_devices.push(profile_info.clone()).is_err() {
            // Nothing entered the cache, so skip the flash write too: persisting a
            // bond the cache rejected would leave flash holding an entry RAM lacks.
            error!(
                "Failed to add bond info for profile {}: cache is full",
                profile_info.slot_num
            );
            return;
        } else {
            true
        };

        if bond_changed {
            self.update_stack_bonds();
        }
        self.pending_save = Some(profile_info);
        self.save_pending().await;
    }

    /// Persist the latest snapshot.
    ///
    /// Called before every wait, so a write cancelled by a profile switch or a
    /// new connection is resumed by the next call. A storage error is not
    /// retried: repeating the same write cannot make it succeed, and retrying
    /// would keep this future from ever handling the next profile action.
    async fn save_pending(&mut self) {
        let Some(info) = self.pending_save.clone() else {
            return;
        };
        let result = store(StorageItem::BondInfo(info)).await;
        self.pending_save = None;
        if result.is_err() {
            error!("Failed to save BLE profile");
        }
    }

    /// Clear bonding information of the specified slot
    pub(crate) async fn clear_bond(&mut self, slot_num: u8) {
        info!("Clearing bonding information on profile: {}", slot_num);

        if let Some(signal) = UPDATED_PROFILES.get(slot_num as usize) {
            signal.reset();
        }

        // Update bonding information in memory
        for bond_info in self.bonded_devices.iter_mut() {
            if bond_info.slot_num == slot_num {
                bond_info.removed = true;
            }
        }

        // Update the active bonding information in the stack
        self.update_stack_bonds();

        // Mark the profile removed, instead of deleting it from storage
        self.pending_save = Some(ProfileInfo {
            slot_num,
            removed: true,
            ..Default::default()
        });
        self.save_pending().await;
    }

    /// Save snapshots and switch profiles; return a slot to clear after the connection stops.
    pub(crate) async fn update_profile(&mut self) -> Option<u8> {
        loop {
            self.save_pending().await;
            match select(
                BLE_PROFILE_CHANNEL.receive(),
                select_array(UPDATED_PROFILES.each_ref().map(|signal| signal.wait())),
            )
            .await
            {
                Either::First(action) => {
                    let current = current_profile();
                    let profile = match action {
                        BleProfileAction::Switch(profile) if profile == current => continue,
                        BleProfileAction::Switch(profile) => profile,
                        BleProfileAction::Previous => {
                            if current == 0 {
                                NUM_BLE_PROFILE as u8 - 1
                            } else {
                                current - 1
                            }
                        }
                        BleProfileAction::Next => {
                            // Cycling stays within the host profiles. The dongle slot sits past
                            // the last one, so wrap there too instead of landing on profile 1.
                            let next = current + 1;
                            if next >= NUM_BLE_PROFILE as u8 { 0 } else { next }
                        }
                        BleProfileAction::ClearBond => return Some(current),
                        BleProfileAction::ClearSlot(slot) => return Some(slot),
                    };
                    if profile != current {
                        set_ble_profile(profile, self.is_bonded(profile));
                        self.update_stack_bonds();
                        #[cfg(feature = "storage")]
                        if store(StorageItem::ActiveBleProfile(profile)).await.is_err() {
                            error!("Failed to save active profile");
                        }
                        info!("Switched to BLE profile: {}", profile);
                    }
                    info!("Update profile done");
                    return None;
                }
                Either::Second((profile_info, _)) => {
                    self.add_profile_info(profile_info).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bt_hci::controller::ExternalController;
    use bt_hci::transport::{PacketToController, PacketToHost, Transport};
    use embassy_futures::select::select;

    use super::{
        DefaultPacketPool, HostResources, LongTermKey, ProfileInfo, ProfileManager, UPDATED_PROFILES, bond_info_of,
        profile_address,
    };
    use crate::test_support::{drain_flash_channel, test_block_on};

    #[test]
    fn profile_addresses_are_stable_and_distinct() {
        let base = [1, 2, 3, 4, 5, 0xC6];
        assert_eq!(profile_address(base, 0), base);
        assert_eq!(profile_address(base, 2), [1, 2, 3, 6, 5, 0xC6]);
        for byte in [0, 0xFF] {
            let base = [1, 2, 3, byte, 5, 0xC6];
            for profile in 0..=crate::NUM_BLE_PROFILE as u8 {
                for other in 0..profile {
                    assert_ne!(profile_address(base, profile), profile_address(base, other));
                }
            }
        }
        assert_eq!(profile_address([1, 2, 3, 0xFF, 5, 0xC6], 1), [1, 2, 3, 0, 5, 0xC6]);
    }

    struct TestTransport;

    impl embedded_io_async::ErrorType for TestTransport {
        type Error = bt_hci::ReadHciError<core::convert::Infallible>;
    }

    impl Transport for TestTransport {
        async fn read<'a, P: PacketToHost<'a>>(&self, _: &'a mut [u8]) -> Result<P, Self::Error> {
            panic!("unexpected controller read");
        }

        async fn write<P: PacketToController>(&self, _: &P) -> Result<(), Self::Error> {
            panic!("unexpected controller write");
        }
    }

    type TestManager<'b, 's, const SLOTS: usize> =
        ProfileManager<'b, 's, ExternalController<TestTransport, 1>, DefaultPacketPool, SLOTS>;

    fn with_manager<const SLOTS: usize>(test: impl FnOnce(&mut TestManager<'_, '_, SLOTS>)) {
        crate::ble::rng::seed([7; 32]);
        let mut resources = HostResources::<DefaultPacketPool, 1, 1>::new();
        let stack = trouble_host::new(ExternalController::<_, 1>::new(TestTransport), &mut resources).build();
        let mut manager = ProfileManager::new(&stack);
        test(&mut manager);
    }

    fn add_profile<const SLOTS: usize>(manager: &mut TestManager<'_, '_, SLOTS>, info: ProfileInfo) {
        test_block_on(select(manager.add_profile_info(info), drain_flash_channel()));
    }

    #[test]
    fn cleared_profile_can_be_paired_again_with_same_bond_information() {
        with_manager::<1>(|manager| {
            let profile_info = ProfileInfo::default();
            add_profile(manager, profile_info.clone());
            test_block_on(manager.add_profile_info(profile_info.clone()));

            UPDATED_PROFILES[0].signal(profile_info.clone());
            test_block_on(select(manager.clear_bond(0), drain_flash_channel()));
            assert!(!UPDATED_PROFILES[0].signaled());
            add_profile(manager, profile_info);
            assert!(!manager.bonded_devices[0].removed);
            assert_eq!(manager.bonded_devices.len(), 1);
        });
    }

    #[test]
    fn full_cache_rejects_new_profile_without_saving_or_evicting() {
        with_manager::<1>(|manager| {
            add_profile(manager, ProfileInfo::default());
            let other_slot = ProfileInfo {
                slot_num: 1,
                ..Default::default()
            };
            test_block_on(manager.add_profile_info(other_slot));
            assert_eq!(manager.bonded_devices.len(), 1);
            assert_eq!(manager.bonded_devices[0].slot_num, 0);
        });
    }

    #[test]
    fn cleared_slot_has_no_bond_info() {
        let mut bonded_devices = heapless::Vec::<ProfileInfo, 1>::new();
        bonded_devices.push(ProfileInfo::default()).unwrap();
        assert!(bond_info_of(&bonded_devices, 0).is_some());

        bonded_devices[0].removed = true;
        assert!(bond_info_of(&bonded_devices, 0).is_none());
    }

    #[test]
    fn bonding_one_slot_leaves_the_others_unbonded() {
        let mut bonded_devices = heapless::Vec::<ProfileInfo, 2>::new();
        bonded_devices.push(ProfileInfo::default()).unwrap();

        assert!(bond_info_of(&bonded_devices, 0).is_some());
        assert!(bond_info_of(&bonded_devices, 1).is_none());
    }

    #[test]
    fn re_pairing_a_slot_with_different_bond_information_replaces_it() {
        with_manager::<1>(|manager| {
            add_profile(manager, ProfileInfo::default());
            let mut new_host = ProfileInfo::default();
            new_host.info.ltk = LongTermKey(1);
            add_profile(manager, new_host);
            assert_eq!(manager.bonded_devices[0].info.ltk, LongTermKey(1));
            assert_eq!(manager.bonded_devices.len(), 1);
        });
    }

    #[test]
    fn latest_profile_snapshots_survive_cancelled_save() {
        use core::future::Future;
        use core::pin::pin;
        use core::task::{Context, Waker};

        use embassy_futures::select::{Either3, select3};

        use crate::config::StorageConfig;
        use crate::core_traits::Runnable;
        use crate::storage::{Storage, StorageKey, StorageValue, async_flash_wrapper, read};
        use crate::test_support::{InMemoryFlash, clear_flash_channel};

        with_manager::<2>(|manager| {
            clear_flash_channel();
            assert_eq!(crate::state::current_profile(), 0);
            let mut existing = ProfileInfo {
                slot_num: 1,
                ..Default::default()
            };
            existing.info.ltk = LongTermKey(1);
            add_profile(manager, existing);
            for (profile, value) in [(1, 1), (0, 2)] {
                let mut info = ProfileInfo {
                    slot_num: profile,
                    ..Default::default()
                };
                info.info.ltk = LongTermKey(value as u128);
                UPDATED_PROFILES[profile as usize].signal(info.clone());
                info.cccd_table = heapless::Vec::from_slice(&[value]).unwrap();
                UPDATED_PROFILES[profile as usize].signal(info);
            }

            {
                let mut read = pin!(read(StorageKey::StorageConfig));
                let mut cx = Context::from_waker(Waker::noop());
                assert!(read.as_mut().poll(&mut cx).is_pending());
                let mut update = pin!(manager.update_profile());
                assert!(update.as_mut().poll(&mut cx).is_pending());
            }
            let mut latest = manager.bond_info(1).unwrap();
            latest.cccd_table = heapless::Vec::from_slice(&[3]).unwrap();
            UPDATED_PROFILES[1].signal(latest);

            test_block_on(async {
                let flash = async_flash_wrapper(InMemoryFlash::<16_384, 4_096, 1>::new());
                let mut storage = Box::pin(Storage::new(flash, &StorageConfig::default())).await;
                let verify = async {
                    loop {
                        if let Some(StorageValue::BondInfo(info)) = read(StorageKey::BondInfo(1)).await.unwrap() {
                            assert_eq!(info.info.ltk, LongTermKey(1));
                            assert_eq!(info.cccd_table.as_slice(), &[3]);
                            break;
                        }
                    }
                    let Some(StorageValue::BondInfo(info)) = read(StorageKey::BondInfo(0)).await.unwrap() else {
                        panic!("the cancelled save must be resumed");
                    };
                    assert_eq!(info.info.ltk, LongTermKey(2));
                    assert_eq!(info.cccd_table.as_slice(), &[2]);
                };
                assert!(matches!(
                    select3(manager.update_profile(), storage.run(), verify).await,
                    Either3::Third(())
                ));
            });

            assert_eq!(crate::state::current_profile(), 0);
            assert_eq!(manager.bond_info(0).unwrap().cccd_table.as_slice(), &[2]);
            assert_eq!(manager.bond_info(1).unwrap().cccd_table.as_slice(), &[3]);
        });
    }

    #[test]
    fn failed_save_is_dropped_instead_of_retried() {
        use embassy_futures::select::{Either3, select3};
        use embassy_time::Timer;

        use crate::config::StorageConfig;
        use crate::core_traits::Runnable;
        use crate::storage::{Storage, async_flash_wrapper};
        use crate::test_support::{InMemoryFlash, clear_flash_channel};

        with_manager::<1>(|manager| {
            clear_flash_channel();
            test_block_on(async {
                let flash = InMemoryFlash::<16_384, 4_096, 1>::new();
                let control = flash.clone();
                let mut storage = Box::pin(Storage::new(async_flash_wrapper(flash), &StorageConfig::default())).await;
                // The schema write above landed; reject only the bond write below.
                control.fail_writes(true);
                let save = manager.add_profile_info(ProfileInfo::default());
                assert!(
                    matches!(
                        select3(save, storage.run(), Timer::after_millis(1_000)).await,
                        Either3::First(())
                    ),
                    "a rejected flash write must not keep save_pending retrying"
                );
            });
            assert!(manager.pending_save.is_none());
        });
    }
}
