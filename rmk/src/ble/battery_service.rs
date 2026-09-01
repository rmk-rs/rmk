use core::sync::atomic::Ordering;

use embassy_futures::join::join;
use embassy_futures::select::{Either, select};
use embassy_sync::pubsub::Subscriber;
use embassy_time::{Duration, Instant, Timer, with_timeout};
use rmk_types::battery::BatteryStatus;
use trouble_host::prelude::*;

use super::ble_server::Server;
use crate::ble::sleep::SLEEPING_STATE;
use crate::core_traits::Runnable;
#[cfg(feature = "split")]
use crate::event::PeripheralBatteryEvent;
use crate::event::{BatteryStatusEvent, SubscribableEvent};
use crate::keyboard::LAST_KEY_TIMESTAMP;

const CHARACTERISTIC_PRESENTATION_FORMAT_UINT8: u8 = 0x04;
const CHARACTERISTIC_PRESENTATION_FORMAT_EXPONENT_ZERO: u8 = 0x00;
const CHARACTERISTIC_PRESENTATION_FORMAT_UNIT_PERCENTAGE: u16 = 0x27AD;
const CHARACTERISTIC_PRESENTATION_FORMAT_NAMESPACE_BLUETOOTH_SIG: u8 = 0x01;
const CHARACTERISTIC_PRESENTATION_FORMAT_DESCRIPTION_MAIN: u16 = 0x0106;

const fn battery_presentation_format(description: u16) -> [u8; 7] {
    let [unit_low, unit_high] = CHARACTERISTIC_PRESENTATION_FORMAT_UNIT_PERCENTAGE.to_le_bytes();
    let [description_low, description_high] = description.to_le_bytes();
    [
        CHARACTERISTIC_PRESENTATION_FORMAT_UINT8,
        CHARACTERISTIC_PRESENTATION_FORMAT_EXPONENT_ZERO,
        unit_low,
        unit_high,
        CHARACTERISTIC_PRESENTATION_FORMAT_NAMESPACE_BLUETOOTH_SIG,
        description_low,
        description_high,
    ]
}

const MAIN_BATTERY_PRESENTATION_FORMAT: [u8; 7] =
    battery_presentation_format(CHARACTERISTIC_PRESENTATION_FORMAT_DESCRIPTION_MAIN);

/// Battery service
#[gatt_service(uuid = service::BATTERY)]
pub(crate) struct BatteryService {
    /// Battery Level
    #[descriptor(
        uuid = descriptors::CHARACTERISTIC_PRESENTATION_FORMAT,
        read,
        value = MAIN_BATTERY_PRESENTATION_FORMAT
    )]
    #[descriptor(
        uuid = descriptors::CHARACTERISTIC_USER_DESCRIPTION,
        read,
        value = crate::CENTRAL_BATTERY_USER_DESCRIPTION
    )]
    #[descriptor(uuid = descriptors::VALID_RANGE, read, value = [0, 100])]
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify)]
    pub(crate) level: u8,
}

#[cfg(feature = "split")]
pub(crate) struct PeripheralBatteryServices {
    pub(crate) levels: [Characteristic<u8>; crate::SPLIT_BATTERY_PERIPHERALS_NUM],
}

#[cfg(feature = "split")]
impl PeripheralBatteryServices {
    pub(crate) const ATTRIBUTE_COUNT: usize = 7 * crate::SPLIT_BATTERY_PERIPHERALS_NUM;
    pub(crate) const CCCD_COUNT: usize = crate::SPLIT_BATTERY_PERIPHERALS_NUM;

    pub(crate) fn new<M: embassy_sync::blocking_mutex::raw::RawMutex, const MAX: usize>(
        table: &mut AttributeTable<'_, M, MAX>,
    ) -> Self {
        Self {
            levels: core::array::from_fn(|slot| {
                add_peripheral_battery_level(
                    table,
                    crate::SPLIT_BATTERY_PERIPHERAL_IDS[slot],
                    crate::SPLIT_BATTERY_PERIPHERAL_USER_DESCRIPTIONS[slot],
                )
            }),
        }
    }
}

#[cfg(feature = "split")]
fn add_peripheral_battery_level<M: embassy_sync::blocking_mutex::raw::RawMutex, const MAX: usize>(
    table: &mut AttributeTable<'_, M, MAX>,
    peripheral_id: usize,
    user_description: &'static str,
) -> Characteristic<u8> {
    let permissions = AttPermissions {
        read: PermissionLevel::Allowed,
        write: PermissionLevel::NotAllowed,
        ..Default::default()
    };
    let mut service = table.add_service(Service::new(service::BATTERY));
    let mut level = service.add_characteristic_small(
        characteristic::BATTERY_LEVEL,
        [CharacteristicProp::Read, CharacteristicProp::Notify],
        0u8,
    );
    level.add_descriptor_small(
        descriptors::CHARACTERISTIC_PRESENTATION_FORMAT,
        permissions,
        peripheral_battery_presentation_format(peripheral_id),
    );
    level.add_descriptor_ro(
        descriptors::CHARACTERISTIC_USER_DESCRIPTION,
        PermissionLevel::Allowed,
        user_description,
    );
    level.add_descriptor_small(descriptors::VALID_RANGE, permissions, [0u8, 100]);
    level.build()
}

#[cfg(feature = "split")]
fn peripheral_battery_presentation_format(peripheral_id: usize) -> [u8; 7] {
    let description = u16::try_from(peripheral_id + 1).expect("peripheral id exceeds GATT namespace range");
    battery_presentation_format(description)
}

pub(crate) struct BleBatteryServer<'stack, 'server, 'conn, P: PacketPool> {
    battery_level: Characteristic<u8>,
    conn: &'conn GattConnection<'stack, 'server, P>,
    sub: Subscriber<
        'static,
        crate::RawMutex,
        BatteryStatusEvent,
        { crate::BATTERY_STATUS_EVENT_CHANNEL_SIZE },
        { crate::BATTERY_STATUS_EVENT_SUB_SIZE },
        { crate::BATTERY_STATUS_EVENT_PUB_SIZE },
    >,
}

impl<'stack, 'server, 'conn, P: PacketPool> BleBatteryServer<'stack, 'server, 'conn, P> {
    pub(crate) fn new(server: &Server, conn: &'conn GattConnection<'stack, 'server, P>) -> Self {
        Self {
            battery_level: server.battery_service.level,
            conn,
            sub: BatteryStatusEvent::subscriber(),
        }
    }
}

impl<P: PacketPool> Runnable for BleBatteryServer<'_, '_, '_, P> {
    async fn run(&mut self) -> ! {
        // Wait 2 seconds, ensure that gatt server has been started
        Timer::after_secs(2).await;

        // First report after connected.
        //
        // Prefer the cached status from the processor — that way a host that
        // connects after the level has already stabilized (battery clamped at
        // 100%, no recent key activity, etc.) doesn't have to wait for a state
        // change to learn the level. If the cache is empty, fall through to
        // waiting on the event stream.
        let first_report = async {
            if let BatteryStatus::Available { level: Some(level), .. } =
                crate::input_device::battery::current_battery_status()
                && self.battery_level.notify(self.conn, &level, true).await.is_ok()
            {
                return;
            }
            loop {
                if let BatteryStatus::Available { level: Some(level), .. } = self.sub.next_message_pure().await.0 {
                    if let Err(e) = self.battery_level.notify(self.conn, &level, true).await {
                        error!("Failed to notify battery level: {:?}", e);
                    } else {
                        // The first report is sent, return to continue
                        return;
                    }
                }
                embassy_time::Timer::after_secs(2).await;
            }
        };

        // Try to do the first battery report in 30 seconds
        with_timeout(Duration::from_secs(30), first_report).await.ok();

        // Report the battery level.
        loop {
            let battery_status = self.wait_until_battery_status_available().await;

            // Check if there's a newer event, if not, use original battery status event
            let state = self.sub.try_next_message_pure().unwrap_or(battery_status);
            if let BatteryStatus::Available { level: Some(level), .. } = state.0
                && let Err(e) = self.battery_level.notify(self.conn, &level, true).await
            {
                error!("Failed to notify battery level: {:?}", e);
            }
        }
    }
}

impl<P: PacketPool> BleBatteryServer<'_, '_, '_, P> {
    /// Wait until the battery status is available.
    /// To avoid unexpected wakeup, before reporting battery level, all conditions should be satistied:
    ///
    /// 1. There's a battery status update
    /// 2. There's a key press in last 1 minute, or timeout(30 minutes)
    /// 3. The keyboard is not in the sleep mode
    async fn wait_until_battery_status_available(&mut self) -> BatteryStatusEvent {
        loop {
            // Calculate timeout when reporting battery level
            let timeout = async {
                loop {
                    embassy_time::Timer::after_secs(1800).await;
                    // 30 minutes passed and the keyboard isn't in sleep mode: timeout
                    if !SLEEPING_STATE.load(Ordering::Acquire) {
                        break;
                    }
                }
            };

            // Wait until there are both battery status update and key pressing or timeout
            let (battery_status, last_press) =
                join(self.sub.next_message_pure(), select(timeout, LAST_KEY_TIMESTAMP.wait())).await;

            // Then check the value last press time
            let last_press = match last_press {
                Either::First(_) => Instant::now().as_secs() as u32,
                Either::Second(last_press) => last_press,
            };

            // Only report battery status if the last key action is less than 60 seconds ago
            let current_time = Instant::now().as_secs() as u32;
            if current_time.saturating_sub(last_press) < 60 {
                return battery_status;
            }
        }
    }
}

/// GATT server task that exposes a peripheral's battery level over BLE.
///
/// Subscribes to [`PeripheralBatteryEvent`] (published by the split driver
/// when the peripheral reports its battery via the split BLE link) and
/// notifies the matching peripheral battery characteristic so the host can
/// read it the same way it reads the central's level.
#[cfg(feature = "split")]
pub(crate) struct BlePeripheralBatteryServer<'stack, 'server, 'conn, P: PacketPool> {
    battery_levels: [Characteristic<u8>; crate::SPLIT_BATTERY_PERIPHERALS_NUM],
    conn: &'conn GattConnection<'stack, 'server, P>,
    sub: Subscriber<
        'static,
        crate::RawMutex,
        PeripheralBatteryEvent,
        { crate::PERIPHERAL_BATTERY_EVENT_CHANNEL_SIZE },
        { crate::PERIPHERAL_BATTERY_EVENT_SUB_SIZE },
        { crate::PERIPHERAL_BATTERY_EVENT_PUB_SIZE },
    >,
}

#[cfg(feature = "split")]
fn find_peripheral_battery_slot(configured_ids: &[usize], peripheral_id: usize) -> Option<usize> {
    configured_ids
        .iter()
        .position(|configured_id| *configured_id == peripheral_id)
}

#[cfg(feature = "split")]
fn initialize_peripheral_battery_levels(server: &Server) {
    for (slot, peripheral_id) in crate::SPLIT_BATTERY_PERIPHERAL_IDS.iter().copied().enumerate() {
        if let Some(BatteryStatus::Available { level: Some(level), .. }) =
            crate::split::driver::current_peripheral_battery_status(peripheral_id)
            && let Err(e) = server.set(&server.peripheral_battery_services.levels[slot], &level)
        {
            error!(
                "Failed to initialize peripheral {} battery level: {:?}",
                peripheral_id, e
            );
        }
    }
}

#[cfg(feature = "split")]
impl<'stack, 'server, 'conn, P: PacketPool> BlePeripheralBatteryServer<'stack, 'server, 'conn, P> {
    pub(crate) fn new(server: &Server, conn: &'conn GattConnection<'stack, 'server, P>) -> Self {
        let sub = PeripheralBatteryEvent::subscriber();
        initialize_peripheral_battery_levels(server);

        Self {
            battery_levels: server.peripheral_battery_services.levels,
            conn,
            sub,
        }
    }
}

#[cfg(feature = "split")]
impl<P: PacketPool> Runnable for BlePeripheralBatteryServer<'_, '_, '_, P> {
    async fn run(&mut self) -> ! {
        // Wait for the GATT server to be ready before pushing notifications.
        Timer::after_secs(2).await;

        // The subscriber is created before this snapshot, so discarding queued
        // events and then reading the cache cannot miss a newer value.
        while self.sub.try_next_message_pure().is_some() {}
        for (slot, peripheral_id) in crate::SPLIT_BATTERY_PERIPHERAL_IDS.iter().copied().enumerate() {
            if let Some(BatteryStatus::Available { level: Some(level), .. }) =
                crate::split::driver::current_peripheral_battery_status(peripheral_id)
                && let Err(e) = self.battery_levels[slot].notify(self.conn, &level, true).await
            {
                error!(
                    "Failed to set initial peripheral {} battery level: {:?}",
                    peripheral_id, e
                );
            }
        }

        loop {
            let event = self.sub.next_message_pure().await;
            if let Some(slot) = find_peripheral_battery_slot(&crate::SPLIT_BATTERY_PERIPHERAL_IDS, event.id)
                && let BatteryStatus::Available { level: Some(level), .. } = event.state.0
                && let Err(e) = self.battery_levels[slot].notify(self.conn, &level, true).await
            {
                error!("Failed to notify peripheral {} battery level: {:?}", event.id, e);
            }
        }
    }
}

#[cfg(test)]
mod cpf_tests {
    use super::{CHARACTERISTIC_PRESENTATION_FORMAT_DESCRIPTION_MAIN, battery_presentation_format};

    #[test]
    fn battery_presentation_format_uses_assigned_numbers() {
        assert_eq!(
            battery_presentation_format(CHARACTERISTIC_PRESENTATION_FORMAT_DESCRIPTION_MAIN),
            [0x04, 0x00, 0xAD, 0x27, 0x01, 0x06, 0x01]
        );
        assert_eq!(
            battery_presentation_format(0x0001),
            [0x04, 0x00, 0xAD, 0x27, 0x01, 0x01, 0x00]
        );
        assert_eq!(
            battery_presentation_format(0x0002),
            [0x04, 0x00, 0xAD, 0x27, 0x01, 0x02, 0x00]
        );
        assert_eq!(
            battery_presentation_format(0x0003),
            [0x04, 0x00, 0xAD, 0x27, 0x01, 0x03, 0x00]
        );
        assert_eq!(
            battery_presentation_format(0x000F),
            [0x04, 0x00, 0xAD, 0x27, 0x01, 0x0F, 0x00]
        );
    }
}

#[cfg(all(test, feature = "split"))]
mod tests {
    use embassy_sync::blocking_mutex::raw::{NoopRawMutex, RawMutex};
    use trouble_host::prelude::{AttributeTable, Characteristic, CharacteristicProp, characteristic};

    use super::{
        BatteryService, MAIN_BATTERY_PRESENTATION_FORMAT, add_peripheral_battery_level, find_peripheral_battery_slot,
        peripheral_battery_presentation_format,
    };

    fn descriptor_value<M: RawMutex, const N: usize>(
        table: &AttributeTable<'_, M, N>,
        characteristic: Characteristic<u8>,
        uuid: trouble_host::prelude::Uuid,
    ) -> heapless::Vec<u8, 32> {
        for handle in characteristic.handle + 1..=characteristic.end_handle {
            if table.uuid(handle) == Some(uuid) {
                let mut value = [0; 32];
                let len = table.read(handle, 0, &mut value).unwrap();
                return heapless::Vec::from_slice(&value[..len]).unwrap();
            }
        }
        panic!("Battery Level descriptor not found");
    }

    #[test]
    fn peripheral_battery_services_use_ids_as_unique_descriptions() {
        let mut table: AttributeTable<'_, NoopRawMutex, 21> = AttributeTable::new();
        let main = BatteryService::new(&mut table);
        let peripherals = [
            add_peripheral_battery_level(&mut table, 0, "Left"),
            add_peripheral_battery_level(&mut table, 2, "Right"),
        ];

        assert_eq!(table.len(), 21);
        assert_eq!(main.level.uuid, characteristic::BATTERY_LEVEL.into());
        assert!(main.level.props.any(&[CharacteristicProp::Read]));
        assert!(main.level.props.any(&[CharacteristicProp::Notify]));
        assert!(main.level.cccd_handle.is_some());
        for level in &peripherals {
            assert_eq!(level.uuid, characteristic::BATTERY_LEVEL.into());
            assert!(level.props.any(&[CharacteristicProp::Read]));
            assert!(level.props.any(&[CharacteristicProp::Notify]));
            assert!(level.cccd_handle.is_some());
        }

        assert_eq!(
            descriptor_value(
                &table,
                main.level,
                trouble_host::prelude::descriptors::CHARACTERISTIC_PRESENTATION_FORMAT.into()
            ),
            MAIN_BATTERY_PRESENTATION_FORMAT.as_slice()
        );
        assert_eq!(
            descriptor_value(
                &table,
                peripherals[0],
                trouble_host::prelude::descriptors::CHARACTERISTIC_PRESENTATION_FORMAT.into()
            ),
            [0x04, 0x00, 0xAD, 0x27, 0x01, 0x01, 0x00].as_slice()
        );
        assert_eq!(
            descriptor_value(
                &table,
                peripherals[1],
                trouble_host::prelude::descriptors::CHARACTERISTIC_PRESENTATION_FORMAT.into()
            ),
            [0x04, 0x00, 0xAD, 0x27, 0x01, 0x03, 0x00].as_slice()
        );
        for level in core::iter::once(&main.level).chain(peripherals.iter()) {
            assert_eq!(
                descriptor_value(&table, *level, trouble_host::prelude::descriptors::VALID_RANGE.into()),
                [0, 100].as_slice()
            );
        }
        assert_eq!(
            descriptor_value(
                &table,
                main.level,
                trouble_host::prelude::descriptors::CHARACTERISTIC_USER_DESCRIPTION.into()
            ),
            crate::CENTRAL_BATTERY_USER_DESCRIPTION.as_bytes()
        );
        assert_eq!(
            descriptor_value(
                &table,
                peripherals[0],
                trouble_host::prelude::descriptors::CHARACTERISTIC_USER_DESCRIPTION.into()
            ),
            b"Left".as_slice()
        );
        assert_eq!(
            descriptor_value(
                &table,
                peripherals[1],
                trouble_host::prelude::descriptors::CHARACTERISTIC_USER_DESCRIPTION.into()
            ),
            b"Right".as_slice()
        );
        assert_ne!(peripherals[0].handle, peripherals[1].handle);
    }

    #[test]
    fn peripheral_battery_presentation_format_maps_id_254_to_assigned_description() {
        assert_eq!(
            peripheral_battery_presentation_format(254),
            [0x04, 0x00, 0xAD, 0x27, 0x01, 0xFF, 0x00]
        );
    }

    #[test]
    fn peripheral_events_map_to_configured_battery_slots() {
        let configured_ids = [0, 2];

        assert_eq!(find_peripheral_battery_slot(&configured_ids, 0), Some(0));
        assert_eq!(find_peripheral_battery_slot(&configured_ids, 1), None);
        assert_eq!(find_peripheral_battery_slot(&configured_ids, 2), Some(1));
    }
}
