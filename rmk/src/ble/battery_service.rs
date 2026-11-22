use core::cell::Cell;
use core::sync::atomic::Ordering;

use embassy_futures::select::select;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_time::{Duration, Instant, Timer, with_timeout};
use rmk_types::battery::BatteryStatus;
use trouble_host::prelude::*;

use super::ble_server::Server;
use crate::core_traits::Runnable;
#[cfg(feature = "split")]
use crate::event::PeripheralBatteryEvent;
use crate::event::{BatteryStatusEvent, SubscribableEvent};
use crate::keyboard::LAST_KEY_TIMESTAMP;
use crate::sleep::SLEEPING_STATE;

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
        Self { levels }
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
        let mut central_status = crate::input_device::battery::current_battery_status();
        #[cfg(feature = "split")]
        let mut peripherals =
            (!crate::SPLIT_BATTERY_PERIPHERAL_IDS.is_empty()).then(PeripheralBatteryEvent::subscriber);
        loop {
            update(&self.levels[0], central_status);
            #[cfg(feature = "split")]
            if let Some(changes) = &mut peripherals {
                while changes.try_next_message_pure().is_some() {}
                for (battery, &id) in self.levels[1..].iter().zip(&crate::SPLIT_BATTERY_PERIPHERAL_IDS) {
                    if let Some(status) = crate::split::driver::current_peripheral_battery_status(id) {
                        update(battery, status);
                    }
                }
                if let embassy_futures::select::Either::First(event) =
                    select(central.next_message_pure(), changes.next_message_pure()).await
                {
                    central_status = event.0;
                }
                continue;
            }
            central_status = central.next_message_pure().await.0;
        }
    }
}

pub(crate) struct BleBatteryServer<'stack, 'server, 'conn, P: PacketPool> {
    battery_level: Characteristic<u8>,
    conn: &'conn GattConnection<'stack, 'server, P>,
    sub: <BatteryStatusEvent as SubscribableEvent>::Subscriber,
}

impl<'stack, 'server, 'conn, P: PacketPool> BleBatteryServer<'stack, 'server, 'conn, P> {
    pub(crate) fn new(server: &Server, conn: &'conn GattConnection<'stack, 'server, P>) -> Self {
        Self {
            battery_level: server.batteries.levels[0].value,
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
                && self.battery_level.notify(self.conn, &level, false).await.is_ok()
            {
                return;
            }
            loop {
                let mut state = self.sub.next_message_pure().await;
                while let Some(newer) = self.sub.try_next_message_pure() {
                    state = newer;
                }
                if let BatteryStatus::Available { level: Some(level), .. } = state.0 {
                    if let Err(e) = self.battery_level.notify(self.conn, &level, false).await {
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

        // Hold each battery change until the keyboard is in use: a key press within the last
        // minute, the next key press, or 30 awake minutes. Every link wakes on the same change
        // and sends the same cached level, so hosts on different profiles agree.
        loop {
            self.sub.next_message_pure().await;
            // `try_get` marks the latest press as seen, so `changed` waits for a new one.
            let idle = last_key
                .try_get()
                .is_none_or(|last_press| (Instant::now().as_secs() as u32).saturating_sub(last_press) >= 60);
            if idle {
                let awake_timeout = async {
                    loop {
                        Timer::after_secs(1800).await;
                        if !SLEEPING_STATE.load(Ordering::Acquire) {
                            break;
                        }
                    }
                };
                select(awake_timeout, last_key.changed()).await;
            }
            while self.sub.try_next_message_pure().is_some() {}
            if let BatteryStatus::Available { level: Some(level), .. } =
                crate::input_device::battery::current_battery_status()
                && let Err(e) = self.battery_level.notify(self.conn, &level, false).await
            {
                error!("Failed to notify battery level: {:?}", e);
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
    battery_levels: [Characteristic<u8>; crate::NUM_SPLIT_BATTERY_PERIPHERALS],
    conn: &'conn GattConnection<'stack, 'server, P>,
    sub: <PeripheralBatteryEvent as SubscribableEvent>::Subscriber,
}

#[cfg(feature = "split")]
fn find_peripheral_battery_slot(configured_ids: &[usize], peripheral_id: usize) -> Option<usize> {
    configured_ids
        .iter()
        .position(|configured_id| *configured_id == peripheral_id)
}

#[cfg(feature = "split")]
impl<'stack, 'server, 'conn, P: PacketPool> BlePeripheralBatteryServer<'stack, 'server, 'conn, P> {
    pub(crate) fn new(server: &Server, conn: &'conn GattConnection<'stack, 'server, P>) -> Self {
        let sub = PeripheralBatteryEvent::subscriber();

        Self {
            battery_levels: core::array::from_fn(|slot| server.batteries.levels[slot + 1].value),
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
                && let Err(e) = self.battery_levels[slot].notify(self.conn, &level, false).await
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
                && let Some(BatteryStatus::Available { level: Some(level), .. }) =
                    crate::split::driver::current_peripheral_battery_status(event.id)
                && let Err(e) = self.battery_levels[slot].notify(self.conn, &level, false).await
            {
                error!("Failed to notify peripheral {} battery level: {:?}", event.id, e);
            }
        }
    }
}

#[cfg(test)]
mod report_tests {
    use embassy_futures::join::join;
    use embassy_futures::select::select;
    use embassy_time::{Duration, Instant, MockDriver, Timer, with_timeout};
    use rmk_types::battery::{BatteryStatus, ChargeState};
    use trouble_host::prelude::*;

    use super::BleBatteryServer;
    use crate::ble::adv::{Adv, advertise};
    use crate::ble::ble_server::Server;
    use crate::ble::fake_hci::FakeController;
    use crate::ble::{NoopHandler, ble_task};
    use crate::core_traits::Runnable;
    use crate::event::{BatteryStatusEvent, publish_event};
    use crate::keyboard::LAST_KEY_TIMESTAMP;
    use crate::test_support::test_block_on;

    fn set_level(level: u8) {
        let status = BatteryStatus::Available {
            charge_state: ChargeState::Discharging,
            level: Some(level),
        };
        crate::input_device::battery::BATTERY_STATUS.lock(|cache| cache.set(status));
        publish_event(BatteryStatusEvent(status));
    }

    #[test]
    fn a_change_late_in_continuous_typing_reaches_the_host() {
        crate::ble::rng::seed([7; 32]);
        let hci = FakeController::default();
        let mut resources = HostResources::<DefaultPacketPool, 1, 0, 1, 1>::new();
        let stack = trouble_host::new(hci.clone(), &mut resources).build();
        let mut peripheral = stack.peripheral();
        let Ok(server) = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
            name: "test",
            appearance: &appearance::human_interface_device::KEYBOARD,
        })) else {
            panic!("the GATT server must build");
        };
        let level = server.batteries.levels[0].value;

        test_block_on(select(ble_task(stack.runner(), &NoopHandler), async {
            let (conn, ()) = join(
                advertise(
                    &mut peripheral,
                    &server.server,
                    Adv::Host {
                        name: "test",
                        fast: false,
                    },
                    cfg!(feature = "ext_adv").then(|| BdAddr::new([1, 2, 3, 4, 5, 0xC6])),
                    Duration::from_secs(1),
                ),
                async {
                    while hci.advertising_starts() == 0 {
                        Timer::after_millis(1).await;
                    }
                    hci.connect(1, [0xA1, 0, 0, 0, 0, 0xC1]);
                },
            )
            .await;
            let conn = conn.expect("the host must connect");
            // The host subscribed to battery notifications.
            let cccd = level.cccd_handle.expect("battery level must be notifiable");
            let mut table = ClientAttTable::builder();
            table.push(cccd, 2, false);
            let mut table = table.build();
            table.write(cccd, 0, &[1, 0]).unwrap();
            server.set_client_att_table(conn.raw(), &table.view());

            set_level(80);
            let mut battery = BleBatteryServer::new(&server, &conn);
            select(battery.run(), async {
                // The connect report goes out, then typing runs past a minute before the level drops.
                for _ in 0..4 {
                    LAST_KEY_TIMESTAMP.sender().send(Instant::now().as_secs() as u32);
                    MockDriver::get().advance(Duration::from_secs(30));
                    Timer::after_millis(1).await;
                }
                set_level(79);
                with_timeout(Duration::from_secs(1), async {
                    while hci.notifications(1, level.handle).len() < 2 {
                        Timer::after_millis(1).await;
                    }
                })
                .await
                .expect("the new level must reach the host");
                assert_eq!(hci.notifications(1, level.handle), [[80], [79]]);
            })
            .await;
        }));
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

    #[cfg(feature = "split")]
    #[test]
    fn peripheral_events_map_to_configured_battery_slots() {
        let configured_ids = [0, 2];

        assert_eq!(super::find_peripheral_battery_slot(&configured_ids, 0), Some(0));
        assert_eq!(super::find_peripheral_battery_slot(&configured_ids, 1), None);
        assert_eq!(super::find_peripheral_battery_slot(&configured_ids, 2), Some(1));
    }
}
