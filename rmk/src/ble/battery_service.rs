use core::sync::atomic::Ordering;

use embassy_futures::select::{Either, select};
use embassy_sync::pubsub::Subscriber;
use embassy_sync::watch::Receiver as WatchReceiver;
use embassy_time::{Duration, Instant, Timer};
use rmk_types::battery::BatteryStatus;
use trouble_host::prelude::*;

use super::ble_server::Server;
use crate::ble::sleep::{LAST_ACTIVITY_TIMESTAMP, SLEEPING_STATE};
use crate::core_traits::Runnable;
#[cfg(feature = "split")]
use crate::event::PeripheralBatteryEvent;
use crate::event::{BatteryStatusEvent, SubscribableEvent};

const CHARACTERISTIC_PRESENTATION_FORMAT_UINT8: u8 = 0x04;
const CHARACTERISTIC_PRESENTATION_FORMAT_EXPONENT_ZERO: u8 = 0x00;
const CHARACTERISTIC_PRESENTATION_FORMAT_UNIT_PERCENTAGE: u16 = 0x27AD;
const CHARACTERISTIC_PRESENTATION_FORMAT_NAMESPACE_BLUETOOTH_SIG: u8 = 0x01;
const CHARACTERISTIC_PRESENTATION_FORMAT_DESCRIPTION_MAIN: u16 = 0x0106;
const GATT_SERVER_START_DELAY_SECS: u64 = 2;
const FIRST_BATTERY_REPORT_TIMEOUT_SECS: u64 = 30;
const BATTERY_REPORT_INTERVAL_SECS: u64 = 30 * 60;
const RECENT_ACTIVITY_THRESHOLD_SECS: u32 = 60;

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

pub(crate) struct BleBatteryServer<'stack, 'server, 'conn, 'values, P: PacketPool> {
    battery_level: Characteristic<u8>,
    server: &'conn Server<'values>,
    conn: &'conn GattConnection<'stack, 'server, P>,
    last_activity_timestamp: WatchReceiver<'static, crate::RawMutex, u32, 2>,
    sub: Subscriber<
        'static,
        crate::RawMutex,
        BatteryStatusEvent,
        { crate::BATTERY_STATUS_EVENT_CHANNEL_SIZE },
        { crate::BATTERY_STATUS_EVENT_SUB_SIZE },
        { crate::BATTERY_STATUS_EVENT_PUB_SIZE },
    >,
}

impl<'stack, 'server, 'conn, 'values, P: PacketPool> BleBatteryServer<'stack, 'server, 'conn, 'values, P> {
    pub(crate) fn new(server: &'conn Server<'values>, conn: &'conn GattConnection<'stack, 'server, P>) -> Self {
        Self {
            battery_level: server.battery_service.level,
            server,
            conn,
            last_activity_timestamp: LAST_ACTIVITY_TIMESTAMP
                .receiver()
                .expect("battery activity timestamp receiver limit reached"),
            sub: BatteryStatusEvent::subscriber(),
        }
    }
}

fn update_battery_value(server: &Server, battery_level: &Characteristic<u8>, status: &BatteryStatus) {
    if let BatteryStatus::Available { level: Some(level), .. } = status
        && let Err(e) = server.set(battery_level, level)
    {
        error!("Failed to update battery level: {:?}", e);
    }
}

impl<P: PacketPool> BleBatteryServer<'_, '_, '_, '_, P> {
    async fn notify_latest_battery_status(&mut self, pending_status: &mut Option<BatteryStatusEvent>) {
        while let Some(status) = self.sub.try_next_message_pure() {
            update_battery_value(self.server, &self.battery_level, &status.0);
            *pending_status = Some(status);
        }

        let status = pending_status.take().expect("battery status pending before notify");
        if let BatteryStatus::Available { level: Some(level), .. } = status.0
            && let Err(e) = self.battery_level.notify(self.conn, &level, false).await
        {
            error!("Failed to notify battery level: {:?}", e);
        }
    }
}

impl<P: PacketPool> Runnable for BleBatteryServer<'_, '_, '_, '_, P> {
    async fn run(&mut self) -> ! {
        // Wait 2 seconds, ensure that gatt server has been started
        Timer::after_secs(GATT_SERVER_START_DELAY_SECS).await;

        let mut pending_status = None;
        let cached_status = crate::input_device::battery::current_battery_status();
        if let BatteryStatus::Available { level: Some(_), .. } = cached_status {
            update_battery_value(self.server, &self.battery_level, &cached_status);
            pending_status = Some(BatteryStatusEvent::from(cached_status));
        }

        // The first report may wait for a battery event until its deadline.
        // Once a value is available, keep updating GATT storage while asleep
        // and defer only the notification until the keyboard wakes.
        let mut first_report_deadline = Some(Instant::now() + Duration::from_secs(FIRST_BATTERY_REPORT_TIMEOUT_SECS));
        let mut next_timeout = Instant::now() + Duration::from_secs(BATTERY_REPORT_INTERVAL_SECS);
        loop {
            if first_report_deadline.is_some() {
                if pending_status.is_none() {
                    let deadline = first_report_deadline.expect("first report deadline is set");
                    match select(Timer::at(deadline), self.sub.next_message_pure()).await {
                        Either::First(_) => {
                            first_report_deadline = None;
                            continue;
                        }
                        Either::Second(status) => {
                            update_battery_value(self.server, &self.battery_level, &status.0);
                            if matches!(status.0, BatteryStatus::Available { level: Some(_), .. }) {
                                pending_status = Some(status);
                            }
                        }
                    }
                    continue;
                }

                match select(
                    wait_until_awake(&mut self.last_activity_timestamp),
                    self.sub.next_message_pure(),
                )
                .await
                {
                    Either::First(_) => {
                        self.notify_latest_battery_status(&mut pending_status).await;
                        first_report_deadline = None;
                        next_timeout = Instant::now() + Duration::from_secs(BATTERY_REPORT_INTERVAL_SECS);
                    }
                    Either::Second(status) => {
                        update_battery_value(self.server, &self.battery_level, &status.0);
                        pending_status = Some(status);
                    }
                }
                continue;
            }

            // Report the battery level.
            if pending_status.is_none() {
                let status = self.sub.next_message_pure().await;
                update_battery_value(self.server, &self.battery_level, &status.0);
                pending_status = Some(status);
                continue;
            }

            match select(
                wait_for_central_battery_report_decision(&mut self.last_activity_timestamp, next_timeout),
                self.sub.next_message_pure(),
            )
            .await
            {
                Either::First(decision) => match decision {
                    BatteryReportDecision::Notify => {
                        self.notify_latest_battery_status(&mut pending_status).await;
                        next_timeout = Instant::now() + Duration::from_secs(BATTERY_REPORT_INTERVAL_SECS);
                    }
                    BatteryReportDecision::Drop => pending_status = None,
                },
                Either::Second(status) => {
                    update_battery_value(self.server, &self.battery_level, &status.0);
                    pending_status = Some(status);
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BatteryReportDecision {
    Notify,
    Drop,
}

/// Wait until a central battery notification is allowed or the current event must be dropped.
async fn wait_for_central_battery_report_decision<M: embassy_sync::blocking_mutex::raw::RawMutex>(
    last_activity_timestamp: &mut WatchReceiver<'_, M, u32, 2>,
    timeout_at: Instant,
) -> BatteryReportDecision {
    loop {
        let deadline_passed = Instant::now() >= timeout_at;
        if !SLEEPING_STATE.load(Ordering::Acquire) && deadline_passed {
            return BatteryReportDecision::Notify;
        }

        let activity = if let Some(activity) = last_activity_timestamp.try_changed() {
            activity
        } else if deadline_passed {
            last_activity_timestamp.changed().await
        } else {
            match select(Timer::at(timeout_at), last_activity_timestamp.changed()).await {
                Either::First(_) => continue,
                Either::Second(activity) => activity,
            }
        };

        if SLEEPING_STATE.load(Ordering::Acquire) {
            continue;
        }

        let current_time = Instant::now().as_secs() as u32;
        return if current_time.saturating_sub(activity) < RECENT_ACTIVITY_THRESHOLD_SECS {
            BatteryReportDecision::Notify
        } else {
            BatteryReportDecision::Drop
        };
    }
}

async fn wait_until_awake<M: embassy_sync::blocking_mutex::raw::RawMutex>(
    last_activity_timestamp: &mut WatchReceiver<'_, M, u32, 2>,
) {
    while SLEEPING_STATE.load(Ordering::Acquire) {
        last_activity_timestamp.changed().await;
    }
}

/// GATT server task that exposes a peripheral's battery level over BLE.
///
/// Subscribes to [`PeripheralBatteryEvent`] (published by the split driver
/// when the peripheral reports its battery via the split BLE link) and
/// notifies the matching peripheral battery characteristic so the host can
/// read it the same way it reads the central's level.
#[cfg(feature = "split")]
pub(crate) struct BlePeripheralBatteryServer<'stack, 'server, 'conn, 'values, P: PacketPool> {
    battery_levels: [Characteristic<u8>; crate::SPLIT_BATTERY_PERIPHERALS_NUM],
    server: &'conn Server<'values>,
    conn: &'conn GattConnection<'stack, 'server, P>,
    last_activity_timestamp: WatchReceiver<'static, crate::RawMutex, u32, 2>,
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
fn handle_peripheral_battery_event(
    server: &Server,
    battery_levels: &[Characteristic<u8>],
    configured_ids: &[usize],
    pending_slots: &mut [bool],
    result: embassy_sync::pubsub::WaitResult<PeripheralBatteryEvent>,
) {
    match result {
        embassy_sync::pubsub::WaitResult::Message(event) => {
            let Some(slot) = find_peripheral_battery_slot(configured_ids, event.id) else {
                return;
            };

            if let BatteryStatus::Available { level: Some(level), .. } = event.state.0
                && let Err(e) = server.set(&battery_levels[slot], &level)
            {
                error!("Failed to update peripheral {} battery level: {:?}", event.id, e);
            }
            pending_slots[slot] = true;
        }
        embassy_sync::pubsub::WaitResult::Lagged(_) => {
            initialize_peripheral_battery_levels(server);
            pending_slots.fill(true);
        }
    }
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
impl<'stack, 'server, 'conn, 'values, P: PacketPool> BlePeripheralBatteryServer<'stack, 'server, 'conn, 'values, P> {
    pub(crate) fn new(server: &'conn Server<'values>, conn: &'conn GattConnection<'stack, 'server, P>) -> Self {
        let sub = PeripheralBatteryEvent::subscriber();
        initialize_peripheral_battery_levels(server);

        Self {
            battery_levels: server.peripheral_battery_services.levels,
            server,
            conn,
            last_activity_timestamp: LAST_ACTIVITY_TIMESTAMP
                .receiver()
                .expect("battery activity timestamp receiver limit reached"),
            sub,
        }
    }
}

#[cfg(feature = "split")]
impl<P: PacketPool> Runnable for BlePeripheralBatteryServer<'_, '_, '_, '_, P> {
    async fn run(&mut self) -> ! {
        // Wait for the GATT server to be ready before pushing notifications.
        Timer::after_secs(GATT_SERVER_START_DELAY_SECS).await;

        // The subscriber is created before this snapshot, so the cache contains
        // the latest value after queued startup events have been consumed.
        while self.sub.try_next_message_pure().is_some() {}
        initialize_peripheral_battery_levels(self.server);

        let mut pending_slots = [false; crate::SPLIT_BATTERY_PERIPHERALS_NUM];
        for (slot, peripheral_id) in crate::SPLIT_BATTERY_PERIPHERAL_IDS.iter().copied().enumerate() {
            if matches!(
                crate::split::driver::current_peripheral_battery_status(peripheral_id),
                Some(BatteryStatus::Available { level: Some(_), .. })
            ) {
                pending_slots[slot] = true;
            }
        }

        loop {
            if !pending_slots.contains(&true) {
                let result = self.sub.next_message().await;
                handle_peripheral_battery_event(
                    self.server,
                    &self.battery_levels,
                    &crate::SPLIT_BATTERY_PERIPHERAL_IDS,
                    &mut pending_slots,
                    result,
                );
                if !pending_slots.contains(&true) {
                    continue;
                }
            }

            match select(
                wait_until_awake(&mut self.last_activity_timestamp),
                self.sub.next_message(),
            )
            .await
            {
                Either::First(_) => {
                    while let Some(result) = self.sub.try_next_message() {
                        handle_peripheral_battery_event(
                            self.server,
                            &self.battery_levels,
                            &crate::SPLIT_BATTERY_PERIPHERAL_IDS,
                            &mut pending_slots,
                            result,
                        );
                    }

                    for (slot, peripheral_id) in crate::SPLIT_BATTERY_PERIPHERAL_IDS.iter().copied().enumerate() {
                        if !pending_slots[slot] {
                            continue;
                        }
                        if SLEEPING_STATE.load(Ordering::Acquire) {
                            break;
                        }
                        if let Some(BatteryStatus::Available { level: Some(level), .. }) =
                            crate::split::driver::current_peripheral_battery_status(peripheral_id)
                            && let Err(e) = self.battery_levels[slot].notify(self.conn, &level, false).await
                        {
                            error!("Failed to notify peripheral {} battery level: {:?}", peripheral_id, e);
                        }
                        pending_slots[slot] = false;
                    }
                    if pending_slots.contains(&true) {
                        continue;
                    }
                }
                Either::Second(result) => {
                    handle_peripheral_battery_event(
                        self.server,
                        &self.battery_levels,
                        &crate::SPLIT_BATTERY_PERIPHERAL_IDS,
                        &mut pending_slots,
                        result,
                    );
                }
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
    use rmk_types::battery::{BatteryStatus, ChargeState};
    use trouble_host::prelude::{AttributeTable, Characteristic, CharacteristicProp, characteristic};

    use super::{
        BatteryService, MAIN_BATTERY_PRESENTATION_FORMAT, Server, add_peripheral_battery_level,
        find_peripheral_battery_slot, peripheral_battery_presentation_format, update_battery_value, wait_until_awake,
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

        let mut pending_slots = [false; 2];
        pending_slots[find_peripheral_battery_slot(&configured_ids, 2).unwrap()] = true;
        assert_eq!(pending_slots, [false, true]);
    }

    #[test]
    fn battery_value_updates_gatt_storage_without_notification() {
        let server = Server::new_default("rmk").unwrap();
        let battery_level = server.battery_service.level;

        for level in [80, 79, 78] {
            update_battery_value(
                &server,
                &battery_level,
                &BatteryStatus::Available {
                    charge_state: ChargeState::Discharging,
                    level: Some(level),
                },
            );
        }

        assert_eq!(server.get(&battery_level), Ok(78));
    }

    #[test]
    fn activity_timestamp_watch_supports_multiple_receivers() {
        let watch = embassy_sync::watch::Watch::<NoopRawMutex, u32, 2>::new();
        let mut first = watch.receiver().unwrap();
        let mut second = watch.receiver().unwrap();
        watch.sender().send(42);

        crate::test_support::test_block_on(async {
            assert_eq!(first.changed().await, 42);
            assert_eq!(second.changed().await, 42);
        });
    }

    #[test]
    fn activity_timestamp_is_consumed_once_per_receiver() {
        let watch = embassy_sync::watch::Watch::<NoopRawMutex, u32, 2>::new();
        let mut receiver = watch.receiver().unwrap();
        watch.sender().send(42);

        assert_eq!(receiver.try_changed(), Some(42));
        assert_eq!(receiver.try_changed(), None);

        watch.sender().send(43);
        assert_eq!(receiver.try_changed(), Some(43));
    }

    #[test]
    fn central_battery_report_gate_drops_stale_activity() {
        use embassy_time::{Duration, Instant, MockDriver};

        let watch = embassy_sync::watch::Watch::<NoopRawMutex, u32, 2>::new();
        let mut receiver = watch.receiver().unwrap();
        crate::ble::sleep::SLEEPING_STATE.store(false, core::sync::atomic::Ordering::Release);

        let decision = crate::test_support::test_block_on(async {
            watch.sender().send(0);
            let timeout = Instant::now() + Duration::from_secs(super::BATTERY_REPORT_INTERVAL_SECS);
            MockDriver::get().advance(Duration::from_secs(u64::from(super::RECENT_ACTIVITY_THRESHOLD_SECS)));
            super::wait_for_central_battery_report_decision(&mut receiver, timeout).await
        });
        assert_eq!(decision, super::BatteryReportDecision::Drop);
    }

    #[test]
    fn peripheral_queue_overflow_refreshes_all_slots() {
        use embassy_sync::pubsub::PubSubChannel;

        use crate::event::{BatteryStatusEvent, PeripheralBatteryEvent};

        let channel = PubSubChannel::<NoopRawMutex, PeripheralBatteryEvent, 2, 1, 1>::new();
        let mut subscriber = channel.subscriber().unwrap();
        let publisher = channel.immediate_publisher();
        let mut pending_slots = [true, false];
        for id in [2, 0, 0] {
            publisher.publish_immediate(PeripheralBatteryEvent {
                id,
                state: BatteryStatusEvent(rmk_types::battery::BatteryStatus::Unavailable),
            });
        }
        let server = Server::new_default("rmk").unwrap();
        let battery_levels = server.peripheral_battery_services.levels;
        while let Some(result) = subscriber.try_next_message() {
            super::handle_peripheral_battery_event(&server, &battery_levels, &[0, 2], &mut pending_slots, result);
        }
        assert_eq!(pending_slots, [true, true]);
    }

    #[test]
    fn peripheral_battery_report_gate_allows_awake_without_recent_activity() {
        use embassy_futures::select::{Either, select};
        use embassy_time::Timer;

        let watch = embassy_sync::watch::Watch::<NoopRawMutex, u32, 2>::new();
        let mut receiver = watch.receiver().unwrap();
        crate::ble::sleep::SLEEPING_STATE.store(false, core::sync::atomic::Ordering::Release);

        crate::test_support::test_block_on(async {
            assert!(matches!(
                select(wait_until_awake(&mut receiver), Timer::after_millis(1),).await,
                Either::First(_)
            ));
        });
    }

    #[cfg(feature = "_ble")]
    #[test]
    fn initial_battery_report_gate_returns_while_awake() {
        use embassy_futures::select::{Either, select};
        use embassy_time::Timer;

        let watch = embassy_sync::watch::Watch::<NoopRawMutex, u32, 2>::new();
        let mut receiver = watch.receiver().unwrap();
        crate::ble::sleep::SLEEPING_STATE.store(false, core::sync::atomic::Ordering::Release);

        let completed = crate::test_support::test_block_on(async {
            match select(wait_until_awake(&mut receiver), Timer::after_millis(10)).await {
                Either::First(_) => true,
                Either::Second(_) => false,
            }
        });

        assert!(completed);
    }

    #[cfg(feature = "_ble")]
    #[test]
    fn battery_report_gate_waits_until_wake() {
        use embassy_futures::select::{Either, select};
        use embassy_time::{Instant, Timer};

        let watch = embassy_sync::watch::Watch::<NoopRawMutex, u32, 2>::new();
        let mut receiver = watch.receiver().unwrap();
        watch.sender().send(Instant::now().as_secs() as u32);
        crate::ble::sleep::SLEEPING_STATE.store(true, core::sync::atomic::Ordering::Release);

        let completed = crate::test_support::test_block_on(async {
            match select(wait_until_awake(&mut receiver), Timer::after_millis(10)).await {
                Either::First(_) => true,
                Either::Second(_) => false,
            }
        });

        crate::ble::sleep::SLEEPING_STATE.store(false, core::sync::atomic::Ordering::Release);
        assert!(!completed);
    }

    #[cfg(feature = "_ble")]
    #[test]
    fn battery_report_gate_returns_after_wake_activity() {
        use embassy_futures::select::{Either, select};
        use embassy_time::{Instant, Timer};

        let watch = embassy_sync::watch::Watch::<NoopRawMutex, u32, 2>::new();
        let mut receiver = watch.receiver().unwrap();
        watch.sender().send(Instant::now().as_secs() as u32);
        crate::ble::sleep::SLEEPING_STATE.store(true, core::sync::atomic::Ordering::Release);

        let completed = crate::test_support::test_block_on(async {
            match select(wait_until_awake(&mut receiver), async {
                Timer::after_millis(5).await;
                crate::ble::sleep::SLEEPING_STATE.store(false, core::sync::atomic::Ordering::Release);
                watch.sender().send(Instant::now().as_secs() as u32);
                Timer::after_millis(5).await;
            })
            .await
            {
                Either::First(_) => true,
                Either::Second(_) => false,
            }
        });

        crate::ble::sleep::SLEEPING_STATE.store(false, core::sync::atomic::Ordering::Release);
        assert!(completed);
    }
}
