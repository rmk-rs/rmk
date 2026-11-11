//! Manage BLE profiles and bonding information

use core::cell::Cell;
use core::sync::atomic::Ordering;

use embassy_futures::select::{Either, select, select_array};
use embassy_sync::signal::Signal;
use embassy_sync::watch::Watch;
use rmk_types::connection::ConnectionType;
use trouble_host::prelude::*;
use trouble_host::{BondInformation, LongTermKey};

use super::ble_server::CCCD_TABLE_SIZE;
use crate::channel::{BLE_REPORT_CHANNEL, LED_SIGNAL};
use crate::event::{LedIndicatorEvent, publish_event};
use crate::keyboard::LOCK_LED_STATES;
use crate::state::{active_transport, current_profile, select_ble_profile, set_ble_bonded};
#[cfg(feature = "storage")]
use crate::storage::{StorageItem, StorageKey, StorageValue, read, store};

/// The dedicated dongle bond slot: the slot number after the normal profiles,
/// so pairing a dongle never touches a host bond. Selected only via the
/// `SwitchToDongle` key, never by profile cycling.
#[cfg(feature = "dongle")]
pub(crate) const DONGLE_PROFILE: u8 = crate::NUM_BLE_PROFILE as u8;

/// A fixed profile address that preserves the base address's random-address type bits.
pub(crate) fn profile_address(base: [u8; 6], profile: u8) -> [u8; 6] {
    let mut addr = base;
    addr[3] = addr[3].wrapping_add(profile);
    addr
}

/// Make `bond` the stack's only key: the stack holds just the active profile's bond.
pub(crate) fn load_stack_bond<C: Controller, P: PacketPool>(stack: &Stack<'_, C, P>, bond: Option<BondInformation>) {
    // The stack can also hold keys from an unfinished pairing.
    while let Some(identity) = stack.with_bond_information(|bonds| bonds.first().map(|b| b.identity)) {
        if let Err(e) = stack.remove_bond_information(identity) {
            debug!("Remove bond info error: {:?}", e);
            break;
        }
    }
    if let Some(bond) = bond {
        debug!("Loading bond for peer {:?}", bond.identity);
        if let Err(e) = stack.add_bond_information(bond) {
            debug!("Add bond info error: {:?}", e);
        }
    }
}

/// BLE profile info
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
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

/// A profile command from the keyboard or host.
#[derive(Debug)]
pub(crate) enum BleProfileAction {
    Switch(u8),
    /// Drop a slot's link; the selected slot then waits for a new host and keeps its bond until one pairs.
    Disconnect(u8),
    Previous,
    Next,
    /// Clear a slot's bond without selecting it; `None` is the slot selected when the action is applied.
    ClearBond(Option<u8>),
}

/// Stored bonds for host or dongle profiles.
///
/// `SLOTS` sizes the cache to the role: [`BOND_SLOTS`](crate::BOND_SLOTS) for a keyboard, a single
/// slot for a dongle.
pub(crate) struct ProfileManager<const SLOTS: usize> {
    profiles: [Watch<crate::RawMutex, ProfileInfo, 1>; SLOTS],
    updated_active_profile: Signal<crate::RawMutex, ()>,
    /// The slot whose stored bond stays hidden until a new host pairs with it.
    pairing: Cell<Option<u8>>,
}

impl<const SLOTS: usize> ProfileManager<SLOTS> {
    /// Create a new profile manager
    pub(crate) fn new() -> Self {
        Self {
            profiles: [const { Watch::new() }; SLOTS],
            updated_active_profile: Signal::new(),
            pairing: Cell::new(None),
        }
    }

    /// Load stored bonding information
    #[cfg(feature = "storage")]
    pub(crate) async fn load_bonded_devices(&mut self) {
        for (slot, state) in self.profiles.iter_mut().enumerate() {
            *state = match read(StorageKey::BondInfo(slot as u8)).await {
                Ok(Some(StorageValue::BondInfo(info))) if !info.removed => Watch::new_with(info),
                _ => Watch::new(),
            };
        }

        let profile = match read(StorageKey::ActiveBleProfile).await {
            // A build with more profiles may have saved a profile this build lacks.
            Ok(Some(StorageValue::ActiveBleProfile(profile))) if (profile as usize) < SLOTS => {
                debug!("Loaded active profile: {}", profile);
                profile
            }
            _ => {
                debug!("Loaded default active profile",);
                0
            }
        };
        for slot in 0..SLOTS as u8 {
            set_ble_bonded(slot, self.bond_info(slot).is_some());
        }
        select_ble_profile(profile);
    }

    /// Read the live bond stored in `profile`; `None` while the profile pairs a new host.
    pub(crate) fn bond_info(&self, profile: u8) -> Option<ProfileInfo> {
        if self.pairing.get() == Some(profile) {
            return None;
        }
        self.profiles.get(profile as usize)?.try_get_and(|info| !info.removed)
    }

    /// Hide `slot`'s stored bond until a new host pairs with it; `None` restores the hidden bond.
    pub(crate) fn set_pairing(&self, slot: Option<u8>) {
        for slot in [self.pairing.replace(slot), slot].into_iter().flatten() {
            set_ble_bonded(slot, self.bond_info(slot).is_some());
        }
    }

    /// Check if the `identity` is the bonded dongle's identity.
    ///
    /// This function is used when searching for BLE host,
    /// the connected dongle(in the DONGLE_PROFILE) should be excluded.
    #[cfg(feature = "dongle")]
    pub(crate) fn is_bonded_dongle(&self, identity: &Identity) -> bool {
        self.bond_info(DONGLE_PROFILE)
            .is_some_and(|info| info.info.identity.match_identity(identity))
    }

    /// Set a slot's bond and initial CCCD state.
    pub(crate) fn set_bond(&self, info: ProfileInfo) {
        let Some(state) = self.profiles.get(info.slot_num as usize) else {
            return;
        };
        if state.try_get().as_ref() == Some(&info) {
            return;
        }
        // A new bond ends pairing on its slot.
        self.pairing
            .set(self.pairing.get().filter(|&slot| slot != info.slot_num));
        set_ble_bonded(info.slot_num, !info.removed);
        state.sender().send(info);
    }

    pub(crate) fn update_profile_cccd_table(
        &self,
        slot: u8,
        peer: &Identity,
        table: heapless::Vec<u8, CCCD_TABLE_SIZE>,
    ) {
        if let Some(state) = self.profiles.get(slot as usize) {
            state.sender().send_if_modified(|info| {
                if let Some(info) = info
                    && !info.removed
                    && info.info.identity.match_identity(peer)
                    && info.cccd_table != table
                {
                    info.cccd_table = table.clone();
                    return true;
                }
                false
            });
        }
    }

    /// One persistence receiver per slot, retained across connection changes.
    pub(crate) async fn run(&self) -> ! {
        let mut receivers = self
            .profiles
            .each_ref()
            .map(|state| state.receiver().expect("one profile saver"));
        loop {
            let item = match select(
                select_array(receivers.each_mut().map(|receiver| receiver.changed())),
                self.updated_active_profile.wait(),
            )
            .await
            {
                Either::First((info, _)) => StorageItem::BondInfo(info),
                Either::Second(()) => StorageItem::ActiveBleProfile(current_profile()),
            };
            if store(item).await.is_err() {
                error!("Failed to save BLE profile");
            }
        }
    }

    /// Clear bonding information of the specified slot.
    pub(crate) fn clear_bond(&self, slot: u8) {
        if let Some(state) = self.profiles.get(slot as usize) {
            state.sender().send(ProfileInfo {
                slot_num: slot,
                removed: true,
                ..Default::default()
            });
            set_ble_bonded(slot, false);
        }
    }

    /// Apply a selected profile outside the cancellable action wait.
    pub(crate) fn switch_profile(&self, profile: u8) {
        let usb_was_active = active_transport() == Some(ConnectionType::Usb);
        select_ble_profile(profile);
        // Reports and lock LEDs of the previous host must not reach the new one.
        BLE_REPORT_CHANNEL.clear();
        LED_SIGNAL.reset();
        if !usb_was_active || active_transport() != Some(ConnectionType::Usb) {
            LOCK_LED_STATES.store(0, Ordering::Relaxed);
            publish_event(LedIndicatorEvent::new(Default::default()));
        }
        self.updated_active_profile.signal(());
        info!("Switched to BLE profile: {}", profile);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DefaultPacketPool, HostResources, LongTermKey, ProfileInfo, ProfileManager, load_stack_bond, profile_address,
    };
    use crate::ble::fake_hci::FakeController;
    use crate::test_support::test_block_on;

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

    #[test]
    fn bond_and_cccd_updates_target_the_requested_profile() {
        let manager = ProfileManager::<2>::new();
        let first = ProfileInfo::default();
        let mut second = ProfileInfo {
            slot_num: 1,
            ..Default::default()
        };
        second.info.ltk = LongTermKey(1);
        manager.set_bond(first.clone());
        manager.set_bond(second.clone());

        second.cccd_table = heapless::Vec::from_slice(&[1, 2]).unwrap();
        manager.update_profile_cccd_table(1, &second.info.identity, second.cccd_table.clone());
        assert_eq!(manager.bond_info(0), Some(first));
        assert_eq!(manager.bond_info(1), Some(second));
    }

    #[test]
    fn notification_settings_belong_to_the_bonded_peer() {
        let manager = ProfileManager::<1>::new();
        let old = ProfileInfo {
            cccd_table: heapless::Vec::from_slice(&[1, 2]).unwrap(),
            ..Default::default()
        };
        let mut replacement = old.clone();
        replacement.info.identity.addr = trouble_host::prelude::Address::random([1, 2, 3, 4, 5, 0xC6]);
        replacement.info.ltk = LongTermKey(42);
        replacement.cccd_table = heapless::Vec::from_slice(&[3, 4]).unwrap();
        manager.set_bond(old.clone());

        manager.update_profile_cccd_table(0, &replacement.info.identity, replacement.cccd_table.clone());
        assert_eq!(manager.bond_info(0), Some(old.clone()));

        manager.set_bond(replacement.clone());
        manager.update_profile_cccd_table(0, &old.info.identity, old.cccd_table);
        assert_eq!(manager.bond_info(0), Some(replacement));
    }

    #[test]
    fn a_new_bond_ends_pairing_on_its_slot() {
        use crate::state::current_connection_status;

        let manager = ProfileManager::<1>::new();
        let old = ProfileInfo::default();
        manager.set_bond(old.clone());
        manager.set_pairing(Some(0));
        assert!(manager.bond_info(0).is_none());
        assert!(!current_connection_status().ble[0].bonded);

        let mut new = old;
        new.info.ltk = LongTermKey(42);
        manager.set_bond(new.clone());
        manager.set_pairing(None);
        assert_eq!(manager.bond_info(0), Some(new));
        assert!(current_connection_status().ble[0].bonded);
    }

    #[test]
    fn profile_switches_update_selection_and_stack_bonds() {
        use crate::channel::BLE_REPORT_CHANNEL;
        use crate::state::{current_ble_status, current_profile};

        let manager = ProfileManager::<2>::new();
        crate::ble::rng::seed([7; 32]);
        let mut resources = HostResources::<DefaultPacketPool, 1, 1>::new();
        let stack = trouble_host::new(FakeController::default(), &mut resources).build();
        for slot in 0..2 {
            let mut info = ProfileInfo {
                slot_num: slot,
                ..Default::default()
            };
            info.info.ltk = LongTermKey(slot.into());
            manager.set_bond(info);
        }
        test_block_on(async {
            for selected in [1, 0, 1] {
                // Reports queued for the previous host must not reach the new one.
                BLE_REPORT_CHANNEL
                    .try_send(crate::hid::Report::KeyboardReport(Default::default()))
                    .unwrap();
                manager.switch_profile(selected);
                assert!(BLE_REPORT_CHANNEL.is_empty());
                load_stack_bond(&stack, manager.bond_info(selected).map(|bond| bond.info));
                assert_eq!(current_profile(), selected);
                assert!(current_ble_status().bonded);
                stack.with_bond_information(|bonds| {
                    assert_eq!(bonds.len(), 1);
                    assert_eq!(bonds[0].ltk, LongTermKey(selected.into()));
                });
            }
            manager.clear_bond(1);
            load_stack_bond(&stack, None);
        });
        assert_eq!(current_profile(), 1);
        assert!(!current_ble_status().bonded);
        assert!(manager.bond_info(1).is_none());
        assert!(manager.bond_info(0).is_some());
        stack.with_bond_information(|bonds| assert!(bonds.is_empty()));
    }

    #[test]
    fn profile_state_is_restored_from_storage() {
        use embassy_futures::select::{select, select3};

        use crate::config::StorageConfig;
        use crate::core_traits::Runnable;
        use crate::storage::{Storage, StorageKey, StorageValue, async_flash_wrapper, read};
        use crate::test_support::{InMemoryFlash, clear_flash_channel, reset_connection_status};

        let manager = ProfileManager::<2>::new();
        clear_flash_channel();
        let mut host = ProfileInfo {
            slot_num: 1,
            ..Default::default()
        };
        host.info.ltk = LongTermKey(42);
        host.cccd_table = heapless::Vec::from_slice(&[1, 2]).unwrap();
        manager.set_bond(ProfileInfo::default());
        manager.set_bond(host.clone());
        manager.clear_bond(0);
        test_block_on(async {
            manager.switch_profile(1);
            let flash = InMemoryFlash::<16_384, 4_096, 1>::new();
            let reboot = flash.clone();
            let mut storage = Box::pin(Storage::new(async_flash_wrapper(flash), &StorageConfig::default())).await;
            let saved = async {
                for slot in 0..2 {
                    loop {
                        if let Some(StorageValue::BondInfo(info)) = read(StorageKey::BondInfo(slot)).await.unwrap() {
                            if slot == 0 {
                                assert!(info.removed);
                            } else {
                                assert_eq!(info, host);
                            }
                            break;
                        }
                    }
                }
                loop {
                    if let Some(StorageValue::ActiveBleProfile(active)) =
                        read(StorageKey::ActiveBleProfile).await.unwrap()
                    {
                        assert_eq!(active, 1);
                        break;
                    }
                }
            };
            select3(manager.run(), storage.run(), saved).await;
            drop(storage);

            clear_flash_channel();
            reset_connection_status();
            let mut restored = ProfileManager::<2>::new();
            let mut storage = Box::pin(Storage::new(async_flash_wrapper(reboot), &StorageConfig::default())).await;
            select(restored.load_bonded_devices(), storage.run()).await;
            assert!(restored.bond_info(0).is_none());
            assert_eq!(restored.bond_info(1), Some(host));
            assert_eq!(crate::state::current_profile(), 1);
            assert!(crate::state::current_ble_status().bonded);
        });
    }

    #[test]
    fn a_saved_profile_this_build_lacks_selects_the_first() {
        use embassy_futures::select::select;

        use crate::config::StorageConfig;
        use crate::core_traits::Runnable;
        use crate::storage::{Storage, StorageItem, async_flash_wrapper, store};
        use crate::test_support::{InMemoryFlash, clear_flash_channel, reset_connection_status};

        clear_flash_channel();
        reset_connection_status();
        test_block_on(async {
            let flash = async_flash_wrapper(InMemoryFlash::<16_384, 4_096, 1>::new());
            let mut storage = Box::pin(Storage::new(flash, &StorageConfig::default())).await;
            select(storage.run(), async {
                store(StorageItem::ActiveBleProfile(2)).await.unwrap();
                ProfileManager::<2>::new().load_bonded_devices().await;
            })
            .await;
        });
        assert_eq!(crate::state::current_profile(), 0);
    }
}
