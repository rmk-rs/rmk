//! Manage BLE profiles and bonding information

#[cfg(feature = "_ble")]
use bt_hci::{cmd::le::LeSetPhy, controller::ControllerCmdAsync};
use embassy_futures::select::{Either3, select3};
use embassy_sync::signal::Signal;
use trouble_host::prelude::*;
use trouble_host::{BondInformation, LongTermKey};

use super::ble_server::CCCD_TABLE_SIZE;
use crate::NUM_BLE_PROFILE;
use crate::channel::BLE_PROFILE_CHANNEL;
#[cfg(feature = "storage")]
use crate::channel::FLASH_CHANNEL;
use crate::state::{current_profile, set_ble_bonded, set_ble_profile};

pub(crate) static UPDATED_PROFILE: Signal<crate::RawMutex, ProfileInfo> = Signal::new();
pub(crate) static UPDATED_CCCD_TABLE: Signal<crate::RawMutex, heapless::Vec<u8, CCCD_TABLE_SIZE>> = Signal::new();

/// The dedicated dongle bond slot: the slot number after the normal profiles,
/// so pairing a dongle never touches a host bond. Selected only via the
/// `SwitchToDongle` key, never by profile cycling.
#[cfg(feature = "dongle")]
pub(crate) const DONGLE_PROFILE: u8 = NUM_BLE_PROFILE as u8;

/// Bond slots kept by the profile manager: the host profiles, plus the
/// dedicated dongle slot on `dongle` builds.
pub(crate) const BOND_SLOTS: usize = NUM_BLE_PROFILE + cfg!(feature = "dongle") as usize;

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

/// Upserts bonding information for a profile slot.
///
/// Returns `Ok(true)` if the entry was inserted, changed, or revived from
/// `removed`, `Ok(false)` if a live entry already held the same info, and
/// `Err(())` if the cache is full.
fn upsert_bond_info<const SLOTS: usize>(
    bonded_devices: &mut heapless::Vec<ProfileInfo, SLOTS>,
    profile_info: &ProfileInfo,
) -> Result<bool, ()> {
    if let Some(index) = bonded_devices
        .iter()
        .position(|info| info.slot_num == profile_info.slot_num)
    {
        if !bonded_devices[index].removed && bonded_devices[index].info == profile_info.info {
            return Ok(false);
        }
        bonded_devices[index] = profile_info.clone();
    } else {
        bonded_devices.push(profile_info.clone()).map_err(|_| ())?;
    }
    Ok(true)
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
            stack,
        }
    }

    /// Load stored bonding information
    #[cfg(feature = "storage")]
    pub(crate) async fn load_bonded_devices(&mut self) {
        use crate::storage::{read_active_ble_profile, read_bond_info};

        self.bonded_devices.clear();
        for slot_num in 0..SLOTS {
            if let Some(info) = read_bond_info(slot_num as u8).await
                && !info.removed
                && let Err(e) = self.bonded_devices.push(info)
            {
                error!("Failed to add bond info: {:?}", e);
            }
        }
        debug!("Loaded {} bond info", self.bonded_devices.len());

        let profile = if let Some(profile) = read_active_ble_profile().await {
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

    /// Cached bond info for the currently active profile, cloned to free the
    /// caller from borrow conflicts with concurrent `update_profile()`.
    pub(crate) fn active_bond_info(&self) -> Option<ProfileInfo> {
        bond_info_of(&self.bonded_devices, current_profile()).cloned()
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

        let active = self.active_bond_info();
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
        match upsert_bond_info(&mut self.bonded_devices, &profile_info) {
            Ok(false) => {
                info!("Skip saving same bonding info");
                return;
            }
            Ok(true) => {}
            Err(()) => {
                // Nothing entered the cache, so skip the flash write too: persisting a
                // bond the cache rejected would leave flash holding an entry RAM lacks.
                error!(
                    "Failed to add bond info for profile {}: cache is full",
                    profile_info.slot_num
                );
                return;
            }
        }

        self.update_stack_bonds();

        #[cfg(feature = "storage")]
        // Send bonding information to the flash task for saving
        FLASH_CHANNEL
            .send(crate::storage::FlashOperationMessage::ProfileInfo(profile_info))
            .await;
    }

    /// Update CCCD table in the stack
    pub(crate) async fn update_profile_cccd_table(&mut self, table: heapless::Vec<u8, CCCD_TABLE_SIZE>) {
        // Get current active profile
        let active_profile = current_profile();

        // Update profile information in memory
        if let Some(index) = self
            .bonded_devices
            .iter()
            .position(|info| info.slot_num == active_profile)
        {
            if self.bonded_devices[index].cccd_table == table {
                debug!("Skip updating same CCCD table");
                return;
            }

            debug!("Updating profile {} CCCD table: {:?}", active_profile, table);
            self.bonded_devices[index].cccd_table = table;

            #[cfg(feature = "storage")]
            FLASH_CHANNEL
                .send(crate::storage::FlashOperationMessage::ProfileInfo(
                    self.bonded_devices[index].clone(),
                ))
                .await;
        } else {
            error!("Failed to update profile CCCD table: profile not found");
        }
    }

    /// Clear bonding information of the specified slot
    pub(crate) async fn clear_bond(&mut self, slot_num: u8) {
        info!("Clearing bonding information on profile: {}", slot_num);

        // Update bonding information in memory
        for bond_info in self.bonded_devices.iter_mut() {
            if bond_info.slot_num == slot_num {
                bond_info.removed = true;
            }
        }

        // Update the active bonding information in the stack
        self.update_stack_bonds();

        #[cfg(feature = "storage")]
        // Send the clear slot message to the flash task
        FLASH_CHANNEL
            .send(crate::storage::FlashOperationMessage::ClearSlot(slot_num))
            .await;
    }

    /// Switch to the specified profile, return true if the profile is switched
    pub(crate) async fn switch_profile(&mut self, profile: u8) -> bool {
        let current = current_profile();
        if profile == current {
            return false;
        }

        set_ble_profile(profile, self.is_bonded(profile));

        // Update the active bonding information in the stack
        self.update_stack_bonds();

        #[cfg(feature = "storage")]
        FLASH_CHANNEL
            .send(crate::storage::FlashOperationMessage::ActiveBleProfile(profile))
            .await;

        info!("Switched to BLE profile: {}", profile);

        true
    }

    /// Wait for profile switch event and update active profile
    ///
    /// This function will wait for profile switch operation, then update the active profile
    /// based on the operation type, and return once the profile write has landed.
    pub(crate) async fn update_profile(&mut self) {
        // Wait for profile switch or updated profile event
        loop {
            match select3(
                BLE_PROFILE_CHANNEL.receive(),
                UPDATED_PROFILE.wait(),
                UPDATED_CCCD_TABLE.wait(),
            )
            .await
            {
                Either3::First(action) => {
                    match action {
                        BleProfileAction::Switch(profile) => {
                            if !self.switch_profile(profile).await {
                                // If the profile is the same as the current profile, do nothing
                                continue;
                            }
                        }
                        BleProfileAction::Previous => {
                            let mut profile = current_profile();
                            profile = if profile == 0 {
                                NUM_BLE_PROFILE as u8 - 1
                            } else {
                                profile - 1
                            };

                            self.switch_profile(profile).await;
                        }
                        BleProfileAction::Next => {
                            // Cycling stays within the host profiles. The dongle slot sits past
                            // the last one, so wrap there too instead of landing on profile 1.
                            let next = current_profile() + 1;
                            let profile = if next >= NUM_BLE_PROFILE as u8 { 0 } else { next };

                            self.switch_profile(profile).await;
                        }
                        BleProfileAction::ClearBond => {
                            self.clear_bond(current_profile()).await;
                        }
                        BleProfileAction::ClearSlot(slot) => {
                            self.clear_bond(slot).await;
                        }
                    }
                    #[cfg(feature = "storage")]
                    crate::storage::flush().await;
                    info!("Update profile done");
                    break;
                }
                Either3::Second(profile_info) => {
                    self.add_profile_info(profile_info).await;
                }
                Either3::Third(table) => {
                    self.update_profile_cccd_table(table).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LongTermKey, ProfileInfo, bond_info_of, upsert_bond_info};

    #[test]
    fn cleared_profile_can_be_paired_again_with_same_bond_information() {
        let profile_info = ProfileInfo::default();
        let mut bonded_devices = heapless::Vec::<ProfileInfo, 1>::new();

        assert_eq!(upsert_bond_info(&mut bonded_devices, &profile_info), Ok(true));
        assert_eq!(upsert_bond_info(&mut bonded_devices, &profile_info), Ok(false));

        bonded_devices[0].removed = true;
        assert_eq!(upsert_bond_info(&mut bonded_devices, &profile_info), Ok(true));
        assert!(!bonded_devices[0].removed);
    }

    #[test]
    fn upsert_fails_without_evicting_when_the_cache_is_full() {
        let mut bonded_devices = heapless::Vec::<ProfileInfo, 1>::new();
        assert_eq!(upsert_bond_info(&mut bonded_devices, &ProfileInfo::default()), Ok(true));

        let other_slot = ProfileInfo {
            slot_num: 1,
            ..Default::default()
        };
        assert_eq!(upsert_bond_info(&mut bonded_devices, &other_slot), Err(()));
        assert_eq!(bonded_devices.len(), 1);
        assert_eq!(bonded_devices[0].slot_num, 0);
    }

    #[test]
    fn cleared_slot_has_no_bond_info() {
        let mut bonded_devices = heapless::Vec::<ProfileInfo, 1>::new();
        assert_eq!(upsert_bond_info(&mut bonded_devices, &ProfileInfo::default()), Ok(true));
        assert!(bond_info_of(&bonded_devices, 0).is_some());

        bonded_devices[0].removed = true;
        assert!(bond_info_of(&bonded_devices, 0).is_none());
    }

    #[test]
    fn bonding_one_slot_leaves_the_others_unbonded() {
        let mut bonded_devices = heapless::Vec::<ProfileInfo, 2>::new();
        assert_eq!(upsert_bond_info(&mut bonded_devices, &ProfileInfo::default()), Ok(true));

        assert!(bond_info_of(&bonded_devices, 0).is_some());
        assert!(bond_info_of(&bonded_devices, 1).is_none());
    }

    #[test]
    fn re_pairing_a_slot_with_different_bond_information_replaces_it() {
        let mut bonded_devices = heapless::Vec::<ProfileInfo, 1>::new();
        assert_eq!(upsert_bond_info(&mut bonded_devices, &ProfileInfo::default()), Ok(true));

        let mut new_host = ProfileInfo::default();
        new_host.info.ltk = LongTermKey(1);
        assert_eq!(upsert_bond_info(&mut bonded_devices, &new_host), Ok(true));
        assert_eq!(bonded_devices[0].info.ltk, LongTermKey(1));
    }
}
