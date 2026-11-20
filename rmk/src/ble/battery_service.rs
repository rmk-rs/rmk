use core::cell::Cell;

#[cfg(feature = "split")]
use embassy_futures::select::select;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_time::Timer;
use rmk_types::battery::BatteryStatus;
use trouble_host::prelude::*;

use super::ble_server::Server;
#[cfg(feature = "split")]
use crate::event::PeripheralBatteryEvent;
use crate::event::{BatteryStatusEvent, SubscribableEvent};

#[cfg(feature = "split")]
const PERIPHERAL_BATTERY_SERVICE_UUID: u128 = 0x061686b6_6a06_4d02_8a8f_c514225bbdb3;
#[cfg(feature = "split")]
const PERIPHERAL_BATTERY_LEVEL_UUID: u128 = 0xcd3259e5_a755_4bf0_9f36_690cddccdb51;

/// Battery services in handle order, starting with the central on every profile.
pub(super) struct BatteryServices {
    pub(super) central_notifications: bool,
    central: BatteryLevel,
    #[cfg(feature = "split")]
    peripherals: [BatteryLevel; crate::NUM_SPLIT_BATTERY_PERIPHERALS],
}

pub(super) struct BatteryLevel {
    value: Characteristic<u8>,
    source: Source,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Central,
    #[cfg(feature = "split")]
    Peripheral(usize),
}

impl BatteryLevel {
    fn new<const MAX: usize>(
        table: &mut AttributeTable<'_, NoopRawMutex, MAX>,
        source: Source,
        presentation: &'static [u8; 7],
        name: &'static str,
    ) -> Self {
        let (service_uuid, level_uuid): (Uuid, Uuid) = match source {
            #[cfg(feature = "split")]
            Source::Peripheral(_) if !crate::SPLIT_BATTERY_HAND_PAIR => (
                PERIPHERAL_BATTERY_SERVICE_UUID.into(),
                PERIPHERAL_BATTERY_LEVEL_UUID.into(),
            ),
            _ => (service::BATTERY.into(), characteristic::BATTERY_LEVEL.into()),
        };
        let mut service = table.add_service(Service::new(service_uuid));
        let mut level = service
            .add_characteristic_small(level_uuid, [CharacteristicProp::Read, CharacteristicProp::Notify], 0u8)
            .read_permission(PermissionLevel::EncryptionRequired);
        level.add_descriptor_ro(
            descriptors::CHARACTERISTIC_PRESENTATION_FORMAT,
            PermissionLevel::Allowed,
            presentation,
        );
        level.add_descriptor_ro(
            descriptors::CHARACTERISTIC_USER_DESCRIPTION,
            PermissionLevel::Allowed,
            name,
        );
        level.add_descriptor_ro(descriptors::VALID_RANGE, PermissionLevel::Allowed, &[0u8, 100]);
        Self {
            value: level.build(),
            source,
        }
    }

    fn percent(&self) -> Option<u8> {
        let status = match self.source {
            Source::Central => crate::input_device::battery::current_battery_status(),
            #[cfg(feature = "split")]
            Source::Peripheral(id) => crate::split::driver::current_peripheral_battery_status(id)?,
        };
        match status {
            BatteryStatus::Available { level, .. } => level,
            BatteryStatus::Unavailable => None,
        }
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
        Self {
            central_notifications: true,
            central: BatteryLevel::new(
                table,
                Source::Central,
                &crate::CENTRAL_BATTERY_PRESENTATION,
                crate::CENTRAL_BATTERY_USER_DESCRIPTION,
            ),
            #[cfg(feature = "split")]
            peripherals: core::array::from_fn(|slot| {
                BatteryLevel::new(
                    table,
                    Source::Peripheral(crate::SPLIT_BATTERY_PERIPHERAL_IDS[slot]),
                    &crate::SPLIT_BATTERY_PERIPHERAL_PRESENTATIONS[slot],
                    crate::SPLIT_BATTERY_PERIPHERAL_USER_DESCRIPTIONS[slot],
                )
            }),
        }
    }

    fn levels(&self) -> impl Iterator<Item = &BatteryLevel> {
        let levels = core::iter::once(&self.central);
        #[cfg(feature = "split")]
        let levels = levels.chain(&self.peripherals);
        levels
    }

    pub(super) fn by_cccd(&self, handle: u16) -> Option<&BatteryLevel> {
        self.levels().find(|battery| battery.value.cccd_handle == Some(handle))
    }

    /// Keep one readable value per battery, including while notifications are disabled.
    pub(super) async fn update_values(&self, server: &Server<'_>) -> ! {
        let mut central = BatteryStatusEvent::subscriber();
        #[cfg(feature = "split")]
        let mut peripherals = (!self.peripherals.is_empty()).then(PeripheralBatteryEvent::subscriber);
        loop {
            while central.try_next_message_pure().is_some() {}
            #[cfg(feature = "split")]
            if let Some(changes) = &mut peripherals {
                while changes.try_next_message_pure().is_some() {}
            }
            for battery in self.levels() {
                if let Some(level) = battery.percent()
                    && let Err(e) = server.set(&battery.value, &level)
                {
                    warn!("Failed to update battery handle {}: {:?}", battery.value.handle, e);
                }
            }
            #[cfg(feature = "split")]
            if let Some(changes) = &mut peripherals {
                select(central.next_message_pure(), changes.next_message_pure()).await;
                continue;
            }
            central.next_message_pure().await;
        }
    }

    pub(super) async fn notify(&self, conn: &GattConnection<'_, '_, impl PacketPool>, battery: &BatteryLevel) {
        if (self.central_notifications || battery.source != Source::Central)
            && let Some(level) = battery.percent()
            && let Err(e) = battery.value.notify(conn, &level, false).await
        {
            warn!("Failed to notify battery handle {}: {:?}", battery.value.handle, e);
        }
    }

    pub(super) async fn replay(&self, conn: &GattConnection<'_, '_, impl PacketPool>) {
        for battery in self.levels() {
            self.notify(conn, battery).await;
        }
    }

    /// Starts once this connection is encrypted and its subscriptions have been restored.
    pub(super) async fn run(&self, conn: &GattConnection<'_, '_, impl PacketPool>, suspended: &Cell<bool>) {
        let reports = self.report_central(conn, suspended);
        #[cfg(feature = "split")]
        let reports = embassy_futures::join::join(reports, self.report_peripherals(conn, suspended));
        reports.await;
    }

    async fn report_central(&self, conn: &GattConnection<'_, '_, impl PacketPool>, suspended: &Cell<bool>) {
        if !self.central_notifications {
            return core::future::pending().await;
        }
        let mut changes = BatteryStatusEvent::subscriber();
        loop {
            while changes.try_next_message_pure().is_some() {}
            if !suspended.get()
                && let Some(level) = self.central.percent()
                && let Err(e) = self.central.value.notify(conn, &level, false).await
            {
                warn!("Failed to notify central battery level: {:?}", e);
                Timer::after_secs(2).await;
                continue;
            }
            changes.next_message_pure().await;
        }
    }

    #[cfg(feature = "split")]
    async fn report_peripherals(&self, conn: &GattConnection<'_, '_, impl PacketPool>, suspended: &Cell<bool>) {
        if self.peripherals.is_empty() {
            return;
        }
        let mut changes = PeripheralBatteryEvent::subscriber();
        for battery in &self.peripherals {
            if !suspended.get() {
                self.notify(conn, battery).await;
            }
        }
        loop {
            let event = changes.next_message_pure().await;
            if !suspended.get()
                && let Some(battery) = self
                    .peripherals
                    .iter()
                    .find(|battery| battery.source == Source::Peripheral(event.id))
            {
                self.notify(conn, battery).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use embassy_sync::blocking_mutex::raw::NoopRawMutex;
    use trouble_host::prelude::*;

    use super::{BatteryLevel, Source};

    #[test]
    fn each_battery_has_its_own_percentage_descriptors_and_subscription() {
        let mut table: AttributeTable<'_, NoopRawMutex, 21> = AttributeTable::new();
        let boards = [
            (Source::Central, &[4, 0, 0xAD, 0x27, 1, 6, 1], "Central"),
            #[cfg(feature = "split")]
            (Source::Peripheral(0), &[4, 0, 0xAD, 0x27, 1, 1, 0], "Left"),
            #[cfg(feature = "split")]
            (Source::Peripheral(254), &[4, 0, 0xAD, 0x27, 1, 255, 0], "Right"),
        ];
        let levels = boards.map(|(source, description, name)| BatteryLevel::new(&mut table, source, description, name));
        assert_eq!(table.len(), 7 * boards.len());
        for (level, (source, description, name)) in levels.iter().zip(boards) {
            let value = level.value;
            let standard = source == Source::Central || crate::SPLIT_BATTERY_HAND_PAIR;
            let mut bytes = [0; 32];
            let service_handle = value.handle - 2;
            assert_eq!(table.uuid(service_handle), Some(declarations::PRIMARY_SERVICE.into()));
            let len = table.read(service_handle, 0, &mut bytes).unwrap();
            assert_eq!(bytes[..len] == service::BATTERY.to_le_bytes(), standard);
            assert_eq!(value.uuid == Uuid::from(characteristic::BATTERY_LEVEL), standard);
            assert_eq!(
                table.permissions(value.handle).unwrap().read,
                PermissionLevel::EncryptionRequired
            );
            assert!(value.props.any(&[CharacteristicProp::Read]));
            assert!(value.props.any(&[CharacteristicProp::Notify]));
            for (uuid, expected) in [
                (descriptors::CHARACTERISTIC_PRESENTATION_FORMAT, &description[..]),
                (descriptors::CHARACTERISTIC_USER_DESCRIPTION, name.as_bytes()),
                (descriptors::VALID_RANGE, &[0, 100][..]),
            ] {
                let handle = (value.handle + 1..=value.end_handle)
                    .find(|&h| table.uuid(h) == Some(uuid.into()))
                    .unwrap();
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
