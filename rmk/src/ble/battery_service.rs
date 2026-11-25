use core::cell::Cell;

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_time::Timer;
use rmk_types::battery::{BatteryStatus, BleBatteryId};
use trouble_host::prelude::*;

#[cfg(feature = "split")]
use crate::event::PeripheralBatteryEvent;
use crate::event::{BatteryStatusEvent, SubscribableEvent};

const PERIPHERAL_BATTERY_SERVICE_UUID: u128 = 0x061686b6_6a06_4d02_8a8f_c514225bbdb3;
const PERIPHERAL_BATTERY_LEVEL_UUID: u128 = 0xcd3259e5_a755_4bf0_9f36_690cddccdb51;

/// Battery services in handle order, starting with the central on every profile.
pub(super) struct BatteryServices {
    pub(super) central_notifications: bool,
    levels: [BatteryLevel; BatteryServices::CCCD_COUNT],
}

struct BatteryLevel {
    value: Characteristic<u8>,
    last_percent: Cell<Option<u8>>,
}

pub(super) enum LinkChange {
    SubscriptionWritten(usize),
    Resumed,
}

impl BatteryServices {
    pub(super) const CCCD_COUNT: usize = crate::BATTERY_BLE_DESCRIPTIONS.len();
    pub(super) const ATTRIBUTE_COUNT: usize = 7 * Self::CCCD_COUNT;

    pub(super) fn new<const MAX: usize>(table: &mut AttributeTable<'_, NoopRawMutex, MAX>) -> Self {
        let levels = crate::BATTERY_BLE_DESCRIPTIONS.map(|(battery_id, user_description)| {
            let mut service_uuid: Uuid = service::BATTERY.into();
            let mut characteristic_uuid: Uuid = characteristic::BATTERY_LEVEL.into();
            // For peripherals, use custom service & characteristic UUID
            if matches!(battery_id, BleBatteryId::Peripheral(_)) {
                service_uuid = PERIPHERAL_BATTERY_SERVICE_UUID.into();
                characteristic_uuid = PERIPHERAL_BATTERY_LEVEL_UUID.into();
            }
            let mut service = table.add_service(Service::new(service_uuid));
            let mut level = service
                .add_characteristic_small(
                    characteristic_uuid,
                    [CharacteristicProp::Read, CharacteristicProp::Notify],
                    0u8,
                )
                .read_permission(PermissionLevel::EncryptionRequired);

            // Service description bytes
            let [lo, hi] = match battery_id {
                BleBatteryId::Central => 0x0106u16.to_le_bytes(),
                BleBatteryId::Left => 0x010Du16.to_le_bytes(),
                BleBatteryId::Right => 0x010Eu16.to_le_bytes(),
                BleBatteryId::Peripheral(id) => (u16::from(id) + 1).to_le_bytes(),
            };
            level.add_descriptor_small(
                descriptors::CHARACTERISTIC_PRESENTATION_FORMAT,
                AttPermissions {
                    write: PermissionLevel::NotAllowed,
                    ..Default::default()
                },
                [0x04, 0, 0xAD, 0x27, 1, lo, hi], // uint8 percentage, Bluetooth SIG namespace
            );
            level.add_descriptor_ro(
                descriptors::CHARACTERISTIC_USER_DESCRIPTION,
                PermissionLevel::Allowed,
                user_description,
            );
            level.add_descriptor_ro(descriptors::VALID_RANGE, PermissionLevel::Allowed, &[0u8, 100]);
            BatteryLevel {
                value: level.build(),
                last_percent: Cell::new(None),
            }
        });
        Self {
            levels,
            central_notifications: true,
        }
    }

    pub(super) fn subscription_write(&self, handle: u16) -> Option<LinkChange> {
        self.levels
            .iter()
            .position(|battery| battery.value.cccd_handle == Some(handle))
            .map(LinkChange::SubscriptionWritten)
    }

    /// Keep one readable value per battery, including while notifications are disabled.
    pub(super) async fn update_values<const MAX: usize>(&self, table: &AttributeTable<'_, NoopRawMutex, MAX>) {
        let update = |battery: &BatteryLevel, status| {
            if let BatteryStatus::Available { level: Some(level), .. } = status
                && battery.last_percent.get() != Some(level)
            {
                match table.set(&battery.value, &level) {
                    Ok(()) => battery.last_percent.set(Some(level)),
                    Err(e) => warn!("Failed to update battery handle {}: {:?}", battery.value.handle, e),
                }
            }
        };
        let central = async {
            let mut changes = BatteryStatusEvent::subscriber();
            update(&self.levels[0], crate::input_device::battery::current_battery_status());
            loop {
                update(&self.levels[0], changes.next_message_pure().await.0);
            }
        };
        #[cfg(feature = "split")]
        let central = embassy_futures::join::join(central, async {
            let ids = &crate::SPLIT_BATTERY_PERIPHERAL_IDS;
            if ids.is_empty() {
                return;
            }
            let mut changes = PeripheralBatteryEvent::subscriber();
            loop {
                for (battery, &id) in self.levels[1..].iter().zip(ids) {
                    if let Some(status) = crate::split::driver::current_peripheral_battery_status(id) {
                        update(battery, status);
                    }
                }
                changes.next_message_pure().await;
                while changes.try_next_message_pure().is_some() {}
            }
        });
        central.await;
    }

    async fn notify_current(
        &self,
        conn: &GattConnection<'_, '_, impl PacketPool>,
        profile: u8,
        slot: usize,
    ) -> Result<(), Error> {
        let battery = &self.levels[slot];
        if crate::state::ble_hid_active(profile)
            && (slot != 0 || self.central_notifications)
            && let Some(level) = battery.last_percent.get()
            && let Err(e) = battery.value.notify(conn, &level, false).await
        {
            warn!("Failed to notify battery handle {}: {:?}", battery.value.handle, e);
            return Err(e);
        }
        Ok(())
    }

    pub(super) async fn on_link_change(
        &self,
        conn: &GattConnection<'_, '_, impl PacketPool>,
        profile: u8,
        change: LinkChange,
    ) {
        let slots = match change {
            LinkChange::SubscriptionWritten(slot) => slot..slot + 1,
            LinkChange::Resumed => 0..self.levels.len(),
        };
        for slot in slots {
            let _ = self.notify_current(conn, profile, slot).await;
        }
    }

    pub(super) async fn report_changes(&self, conn: &GattConnection<'_, '_, impl PacketPool>, profile: u8) {
        let central = async {
            if !self.central_notifications {
                return;
            }
            let mut changes = BatteryStatusEvent::subscriber();
            loop {
                while changes.try_next_message_pure().is_some() {}
                if self.notify_current(conn, profile, 0).await.is_err() {
                    Timer::after_secs(2).await;
                    continue;
                }
                while !matches!(
                    changes.next_message_pure().await.0,
                    BatteryStatus::Available { level: Some(_), .. }
                ) {}
            }
        };
        #[cfg(feature = "split")]
        let central = embassy_futures::join::join(central, async {
            let ids = &crate::SPLIT_BATTERY_PERIPHERAL_IDS;
            if ids.is_empty() {
                return;
            }
            let mut changes = PeripheralBatteryEvent::subscriber();
            loop {
                for slot in 1..self.levels.len() {
                    let _ = self.notify_current(conn, profile, slot).await;
                }
                while let embassy_sync::pubsub::WaitResult::Message(event) = changes.next_message().await {
                    if matches!(event.state.0, BatteryStatus::Available { level: Some(_), .. })
                        && let Some(slot) = ids.iter().position(|&id| id == event.id)
                    {
                        let _ = self.notify_current(conn, profile, slot + 1).await;
                    }
                }
                // A lagged subscriber has lost board IDs. Replay the current values for this link.
                while changes.try_next_message_pure().is_some() {}
            }
        });
        central.await;
    }
}
