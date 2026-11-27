use core::cell::Cell;

use embassy_futures::select::select;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_time::{Duration, Instant, Timer, with_timeout};
use rmk_types::battery::{BatteryStatus, BleBatteryId};
use trouble_host::prelude::*;

use super::ble_server::Server;
use crate::core_traits::Runnable;
#[cfg(feature = "split")]
use crate::event::PeripheralBatteryEvent;
use crate::event::{BatteryStatusEvent, SubscribableEvent};
use crate::keyboard::LAST_KEY_TIMESTAMP;

const PERIPHERAL_BATTERY_SERVICE_UUID: u128 = 0x061686b6_6a06_4d02_8a8f_c514225bbdb3;
const PERIPHERAL_BATTERY_LEVEL_UUID: u128 = 0xcd3259e5_a755_4bf0_9f36_690cddccdb51;

/// Battery services in handle order, starting with the central on every profile.
pub(super) struct BatteryServices {
    levels: [BatteryLevel; BatteryServices::CCCD_COUNT],
}

struct BatteryLevel {
    value: Characteristic<u8>,
    last_percent: Cell<Option<u8>>,
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
        Self { levels }
    }

    pub(super) fn by_cccd(&self, handle: u16) -> Option<usize> {
        self.levels
            .iter()
            .position(|battery| battery.value.cccd_handle == Some(handle))
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

    /// Send the configured split batteries' levels to this connection.
    #[cfg(feature = "split")]
    pub(super) async fn notify_peripherals<P: PacketPool>(&self, conn: &GattConnection<'_, '_, P>) {
        let ids = &crate::SPLIT_BATTERY_PERIPHERAL_IDS;
        if ids.is_empty() {
            return;
        }

        let mut changes = PeripheralBatteryEvent::subscriber();
        // Wait for the GATT server to be ready before sending the initial levels.
        Timer::after_secs(2).await;

        // Drain queued events before reading the latest levels from the cache.
        while changes.try_next_message_pure().is_some() {}
        for (battery, &id) in self.levels[1..].iter().zip(ids) {
            if let Some(BatteryStatus::Available { level: Some(level), .. }) =
                crate::split::driver::current_peripheral_battery_status(id)
                && let Err(e) = battery.value.to_raw().notify_raw(conn, &[level], false).await
            {
                error!("Failed to notify initial peripheral {} battery level: {:?}", id, e);
            }
        }

        loop {
            let id = changes.next_message_pure().await.id;
            if let Some(slot) = ids.iter().position(|&peripheral_id| peripheral_id == id)
                && let Some(BatteryStatus::Available { level: Some(level), .. }) =
                    crate::split::driver::current_peripheral_battery_status(id)
                && let Err(e) = self.levels[slot + 1]
                    .value
                    .to_raw()
                    .notify_raw(conn, &[level], false)
                    .await
            {
                error!("Failed to notify peripheral {} battery level: {:?}", id, e);
            }
        }
    }
}

pub(crate) struct BleBatteryServer<'stack, 'server, 'conn, P: PacketPool> {
    /// Raw, so its notifications share the HID reports' `notify_raw` instance.
    battery_level: Characteristic<[u8]>,
    conn: &'conn GattConnection<'stack, 'server, P>,
    sub: <BatteryStatusEvent as SubscribableEvent>::Subscriber,
}

impl<'stack, 'server, 'conn, P: PacketPool> BleBatteryServer<'stack, 'server, 'conn, P> {
    pub(crate) fn new(server: &Server, conn: &'conn GattConnection<'stack, 'server, P>) -> Self {
        Self {
            battery_level: server.batteries.levels[0].value.to_raw(),
            conn,
            sub: BatteryStatusEvent::subscriber(),
        }
    }
}

impl<P: PacketPool> Runnable for BleBatteryServer<'_, '_, '_, P> {
    async fn run(&mut self) -> ! {
        let mut last_key = LAST_KEY_TIMESTAMP
            .receiver()
            .expect("battery activity receiver slots exhausted");

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
                && self.battery_level.notify_raw(self.conn, &[level], false).await.is_ok()
            {
                return;
            }
            loop {
                let mut state = self.sub.next_message_pure().await;
                while let Some(newer) = self.sub.try_next_message_pure() {
                    state = newer;
                }
                if let BatteryStatus::Available { level: Some(level), .. } = state.0 {
                    if let Err(e) = self.battery_level.notify_raw(self.conn, &[level], false).await {
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

        // While idle, wait for the next key press or a retry while awake.
        loop {
            self.sub.next_message_pure().await;
            // `try_get` marks the latest press as seen, so `changed` waits for a new one.
            let idle = last_key
                .try_get()
                .is_none_or(|last_press| (Instant::now().as_secs() as u32).saturating_sub(last_press) >= 60);
            if idle {
                let awake_timeout = async {
                    let retry_secs = (2 * u64::from(crate::SPLIT_CENTRAL_SLEEP_TIMEOUT_SECONDS)).max(1800);
                    loop {
                        Timer::after_secs(retry_secs).await;
                        if !crate::state::current_sleeping() {
                            break;
                        }
                    }
                };
                select(awake_timeout, last_key.changed()).await;
            }
            while self.sub.try_next_message_pure().is_some() {}
            if let BatteryStatus::Available { level: Some(level), .. } =
                crate::input_device::battery::current_battery_status()
                && let Err(e) = self.battery_level.notify_raw(self.conn, &[level], false).await
            {
                error!("Failed to notify battery level: {:?}", e);
            }
        }
    }
}
