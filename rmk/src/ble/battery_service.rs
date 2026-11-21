use core::cell::Cell;

#[cfg(feature = "split")]
use embassy_futures::select::select;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_time::Timer;
use rmk_types::battery::BatteryStatus;
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

fn add_battery<const MAX: usize>(
    table: &mut AttributeTable<'_, NoopRawMutex, MAX>,
    peripheral_id: Option<usize>,
    is_left: Option<bool>,
    name: &'static str,
) -> BatteryLevel {
    let (service_uuid, level_uuid, description): (Uuid, Uuid, u16) = match (peripheral_id, is_left) {
        (Some(id), None) => (
            PERIPHERAL_BATTERY_SERVICE_UUID.into(),
            PERIPHERAL_BATTERY_LEVEL_UUID.into(),
            (id + 1) as u16,
        ),
        (_, side) => (
            service::BATTERY.into(),
            characteristic::BATTERY_LEVEL.into(),
            match side {
                Some(true) => 0x010D,  // Left
                Some(false) => 0x010E, // Right
                None => 0x0106,        // Main
            },
        ),
    };
    let mut service = table.add_service(Service::new(service_uuid));
    let mut level = service
        .add_characteristic_small(level_uuid, [CharacteristicProp::Read, CharacteristicProp::Notify], 0u8)
        .read_permission(PermissionLevel::EncryptionRequired);
    let [lo, hi] = description.to_le_bytes();
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
        name,
    );
    level.add_descriptor_ro(descriptors::VALID_RANGE, PermissionLevel::Allowed, &[0u8, 100]);
    BatteryLevel {
        value: level.build(),
        last_percent: Cell::new(None),
    }
}

impl BatteryServices {
    pub(super) const CCCD_COUNT: usize = if cfg!(feature = "split") {
        1 + crate::NUM_SPLIT_BATTERY_PERIPHERALS
    } else {
        1
    };
    pub(super) const ATTRIBUTE_COUNT: usize = 7 * Self::CCCD_COUNT;

    pub(super) fn new<const MAX: usize>(table: &mut AttributeTable<'_, NoopRawMutex, MAX>) -> Self {
        let left = crate::CENTRAL_BATTERY_IS_LEFT;
        let levels = core::array::from_fn(|slot| match slot.checked_sub(1) {
            None => add_battery(table, None, left, crate::CENTRAL_BATTERY_USER_DESCRIPTION),
            Some(peripheral) => add_battery(
                table,
                Some(crate::SPLIT_BATTERY_PERIPHERAL_IDS[peripheral]),
                left.map(|left| !left),
                crate::SPLIT_BATTERY_PERIPHERAL_USER_DESCRIPTIONS[peripheral],
            ),
        });
        Self {
            levels,
            central_notifications: true,
        }
    }

    pub(super) fn by_cccd(&self, handle: u16) -> Option<usize> {
        self.levels
            .iter()
            .position(|battery| battery.value.cccd_handle == Some(handle))
    }

    /// Keep one readable value per battery, including while notifications are disabled.
    pub(super) async fn update_values<const MAX: usize>(&self, table: &AttributeTable<'_, NoopRawMutex, MAX>) -> ! {
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
        let mut central = BatteryStatusEvent::subscriber();
        #[cfg(feature = "split")]
        let mut peripherals =
            (!crate::SPLIT_BATTERY_PERIPHERAL_IDS.is_empty()).then(PeripheralBatteryEvent::subscriber);
        loop {
            while central.try_next_message_pure().is_some() {}
            update(&self.levels[0], crate::input_device::battery::current_battery_status());
            #[cfg(feature = "split")]
            if let Some(changes) = &mut peripherals {
                while changes.try_next_message_pure().is_some() {}
                for (battery, &id) in self.levels[1..].iter().zip(&crate::SPLIT_BATTERY_PERIPHERAL_IDS) {
                    if let Some(status) = crate::split::driver::current_peripheral_battery_status(id) {
                        update(battery, status);
                    }
                }
                select(central.next_message_pure(), changes.next_message_pure()).await;
                continue;
            }
            central.next_message_pure().await;
        }
    }

    pub(super) async fn notify(&self, conn: &GattConnection<'_, '_, impl PacketPool>, slot: usize) -> bool {
        let battery = &self.levels[slot];
        if (slot != 0 || self.central_notifications)
            && let Some(level) = battery.last_percent.get()
            && let Err(e) = battery.value.notify(conn, &level, false).await
        {
            warn!("Failed to notify battery handle {}: {:?}", battery.value.handle, e);
            return false;
        }
        true
    }

    pub(super) async fn replay(&self, conn: &GattConnection<'_, '_, impl PacketPool>) {
        for slot in 0..self.levels.len() {
            self.notify(conn, slot).await;
        }
    }

    pub(super) async fn report_changes(&self, conn: &GattConnection<'_, '_, impl PacketPool>, suspended: &Cell<bool>) {
        let central = async {
            if !self.central_notifications {
                return;
            }
            let mut changes = BatteryStatusEvent::subscriber();
            loop {
                while changes.try_next_message_pure().is_some() {}
                if !suspended.get() && !self.notify(conn, 0).await {
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
            for slot in 1..self.levels.len() {
                if !suspended.get() {
                    self.notify(conn, slot).await;
                }
            }
            loop {
                let event = changes.next_message_pure().await;
                if !suspended.get()
                    && matches!(event.state.0, BatteryStatus::Available { level: Some(_), .. })
                    && let Some(slot) = ids.iter().position(|&id| id == event.id)
                {
                    self.notify(conn, slot + 1).await;
                }
            }
        });
        central.await;
    }
}

#[cfg(test)]
mod tests {
    use embassy_sync::blocking_mutex::raw::NoopRawMutex;
    use trouble_host::prelude::*;

    use super::add_battery;

    #[test]
    fn each_battery_has_its_own_percentage_descriptors_and_subscription() {
        let mut table: AttributeTable<'_, NoopRawMutex, 28> = AttributeTable::new();
        let boards = [
            (None, None, 0x0106u16, "Central"),
            #[cfg(feature = "split")]
            (Some(0), Some(true), 0x010D, "Peripheral 0"),
            #[cfg(feature = "split")]
            (Some(1), Some(false), 0x010E, "Peripheral 1"),
            #[cfg(feature = "split")]
            (Some(254), None, 255, "Peripheral 254"),
        ];
        let levels = boards.map(|(peripheral, side, _, name)| add_battery(&mut table, peripheral, side, name).value);
        assert_eq!(table.len(), 7 * boards.len());
        for (value, (peripheral, side, description, name)) in levels.iter().zip(boards) {
            let (service_uuid, level_uuid): (Uuid, Uuid) = if peripheral.is_none() || side.is_some() {
                (service::BATTERY.into(), characteristic::BATTERY_LEVEL.into())
            } else {
                (
                    0x061686b6_6a06_4d02_8a8f_c514225bbdb3u128.into(),
                    0xcd3259e5_a755_4bf0_9f36_690cddccdb51u128.into(),
                )
            };
            let mut bytes = [0; 32];
            let service_handle = value.handle - 2;
            assert_eq!(table.uuid(service_handle), Some(declarations::PRIMARY_SERVICE.into()));
            let len = table.read(service_handle, 0, &mut bytes).unwrap();
            assert_eq!(&bytes[..len], service_uuid.as_raw());
            assert_eq!(table.uuid(value.handle), Some(level_uuid));
            assert_eq!(
                table.permissions(value.handle).unwrap().read,
                PermissionLevel::EncryptionRequired
            );
            assert!(value.props.any(&[CharacteristicProp::Read]));
            assert!(value.props.any(&[CharacteristicProp::Notify]));
            let [lo, hi] = description.to_le_bytes();
            let presentation = [4, 0, 0xAD, 0x27, 1, lo, hi];
            for (uuid, expected) in [
                (descriptors::CHARACTERISTIC_PRESENTATION_FORMAT, &presentation[..]),
                (descriptors::CHARACTERISTIC_USER_DESCRIPTION, name.as_bytes()),
                (descriptors::VALID_RANGE, &[0, 100][..]),
            ] {
                let handle = (value.handle + 1..=value.end_handle)
                    .find(|&h| table.uuid(h) == Some(uuid.into()))
                    .unwrap();
                let permissions = table.permissions(handle).unwrap();
                assert_eq!(permissions.read, PermissionLevel::Allowed);
                assert_eq!(permissions.write, PermissionLevel::NotAllowed);
                let len = table.read(handle, 0, &mut bytes).unwrap();
                assert_eq!(&bytes[..len], expected);
            }
            assert_eq!(
                table.uuid(value.cccd_handle.unwrap()),
                Some(descriptors::CLIENT_CHARACTERISTIC_CONFIGURATION.into())
            );
        }
    }
}
