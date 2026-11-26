use core::cell::Cell;
use core::fmt::Write as _;

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
#[cfg(all(feature = "dongle", feature = "custom_message"))]
use postcard::experimental::max_size::MaxSize;
use rmk_types::ble::BLE_ADV_NAME_MAX_LEN;
use rmk_types::led_indicator::LedIndicator;
use trouble_host::att::{AttClient, AttReq};
use trouble_host::prelude::*;
use usbd_hid::descriptor::{AsInputReport, SerializedDescriptor};

use super::battery_service::BatteryServices;
use super::device_info::{DeviceConfigurationService, PnPID};
use crate::BLE_PROFILE_NAME_SUFFIX;
use crate::config::DeviceConfig;
#[cfg(all(feature = "dongle", feature = "custom_message"))]
use crate::dongle::event::{CUSTOM_TO_DONGLE_UUID, CUSTOM_TO_KEYBOARD_UUID};
#[cfg(feature = "dongle")]
use crate::dongle::event::{DONGLE_EVENT_CHAR_UUID, DONGLE_EVENT_MAX, DONGLE_EVENT_SERVICE_UUID};
#[cfg(feature = "rynk")]
use crate::hid::RynkHidReport;
#[cfg(feature = "vial")]
use crate::hid::ViaReport;
use crate::hid::{BleCompositeReport, CompositeReportType, MOUSE_REPORT_SIZE, Report};

// Used for saving the client attribute (CCCD) table. Tracks the trouble-host
// per-connection client-specific attribute buffer size.
pub(crate) const CCCD_TABLE_SIZE: usize = trouble_host::config::CLIENT_ATT_TABLE_SIZE;

#[cfg(feature = "rynk")]
use rmk_types::protocol::rynk::{
    RYNK_BLE_CHUNK_SIZE, RYNK_HID_REPORT_SIZE, RYNK_INPUT_CHAR_UUID, RYNK_OUTPUT_CHAR_UUID, RYNK_SERVICE_UUID,
};

/// Keep GAP/GATT first and append other services to preserve bonded hosts' cached handles.
#[gatt_server(mutex_type = NoopRawMutex, connections_max = crate::ble::NUM_HOST_LINKS)]
pub(crate) struct Server {
    profile_name: ProfileName,
    pub(super) batteries: BatteryServices,
    pub(super) hid_service: HidService,
    #[cfg(feature = "vial")]
    pub(crate) vial_service: VialGattService,
    #[cfg(feature = "rynk")]
    pub(crate) rynk_service: RynkGattService,
    #[cfg(feature = "rynk")]
    pub(crate) rynk_hid_service: RynkHidService,
    device_config_service: DeviceConfigurationService,
    #[cfg(feature = "dongle")]
    pub(crate) dongle_event_service: DongleEventService,
}

struct ProfileName {
    characteristic: [Characteristic<heapless::String<BLE_ADV_NAME_MAX_LEN>>; BLE_PROFILE_NAME_SUFFIX as usize],
    product_name: &'static str,
}

impl ProfileName {
    const ATTRIBUTE_COUNT: usize = trouble_host::gap::GAP_SERVICE_ATTRIBUTE_COUNT * BLE_PROFILE_NAME_SUFFIX as usize;
    const CCCD_COUNT: usize = 0;

    fn new<const MAX: usize>(table: &mut AttributeTable<'_, NoopRawMutex, MAX>) -> Self {
        Self {
            characteristic: core::array::from_fn(|_| {
                let gap = GapService::new(table);
                GenericAttributeService::new(table);
                gap.device_name
            }),
            product_name: "",
        }
    }
}

#[gatt_service(uuid = service::GAP)]
struct GapService {
    #[characteristic(uuid = characteristic::DEVICE_NAME, read)]
    device_name: heapless::String<BLE_ADV_NAME_MAX_LEN>,
    #[characteristic(uuid = characteristic::APPEARANCE, read, value = appearance::human_interface_device::KEYBOARD)]
    appearance: BluetoothUuid16,
    #[cfg(any(feature = "split", feature = "dongle"))]
    #[characteristic(uuid = characteristic::CENTRAL_ADDRESS_RESOLUTION, read, value = 1)]
    address_resolution: u8,
}

#[gatt_service(uuid = service::GATT)]
struct GenericAttributeService {}

impl Server<'_> {
    // Not inlined, so the by-value attribute table it builds leaves the stack before the
    // BLE tasks are polled; the returned server is written straight into the caller's future.
    #[inline(never)]
    pub(super) fn new_keyboard(product_name: &'static str) -> Result<Self, &'static str> {
        if BLE_PROFILE_NAME_SUFFIX {
            let mut server = Self::new(AttributeTable::new());
            server.profile_name.product_name = product_name;
            return Ok(server);
        }
        Self::new_with_config(GapConfig::Peripheral(PeripheralConfig {
            name: product_name,
            appearance: &appearance::human_interface_device::KEYBOARD,
        }))
    }

    /// Kept separate from [`Self::new_keyboard`] so `new_keyboard` can tail-return
    /// `Self::new_with_config` without moving the multi-KiB `Server` through `?`/`Ok(...)`
    /// on the BLE task stack.
    pub(super) fn configure(&self, device: &DeviceConfig<'_>) {
        let info = &self.device_config_service;
        let pnp_id = PnPID {
            vendor_id: device.vid,
            product_id: device.pid,
            ..Default::default()
        };
        let manufacturer = heapless::String::try_from(device.manufacturer).expect("manufacturer is too long for BLE");
        unwrap!(self.set(&info.pnp_id, &pnp_id));
        unwrap!(self.set(&info.serial_number, &device.serial_number));
        unwrap!(self.set(&info.manufacturer_name, &manufacturer));
    }

    /// The empty return array avoids a name buffer in disabled builds.
    pub(super) fn profile_name(
        &self,
        slot: u8,
    ) -> [heapless::String<BLE_ADV_NAME_MAX_LEN>; BLE_PROFILE_NAME_SUFFIX as usize] {
        core::array::from_fn(|_| {
            let mut name =
                heapless::String::try_from(self.profile_name.product_name).expect("product name is too long for BLE");
            if (slot as usize) < crate::NUM_BLE_PROFILES {
                write!(name, "_{}", slot + 1).expect("product name including BLE profile suffix exceeds 16 bytes");
            }
            name
        })
    }

    pub(super) fn prepare_name_read(&self, event: &GattEvent<'_, '_, DefaultPacketPool>, slot: u8) {
        let [name] = self.profile_name.characteristic.as_slice() else {
            return;
        };
        let uuid = Uuid::from(characteristic::DEVICE_NAME);
        let reads_name = match event {
            GattEvent::Read(read) => read.handle() == name.handle,
            GattEvent::Other(other) => match other.payload().incoming() {
                AttClient::Request(AttReq::ReadByType {
                    attribute_type,
                    start,
                    end,
                }) => attribute_type == uuid && (start..=end).contains(&name.handle),
                AttClient::Request(AttReq::FindByTypeValue {
                    att_type,
                    start_handle,
                    end_handle,
                    ..
                }) => Uuid::from(att_type) == uuid && (start_handle..=end_handle).contains(&name.handle),
                _ => false,
            },
            _ => false,
        };
        if reads_name && let Some(value) = self.profile_name(slot).first() {
            unwrap!(self.set(name, value));
        }
    }
}

/// One postcard-encoded [`crate::dongle::event::DongleEvent`] per notification.
#[cfg(feature = "dongle")]
#[gatt_service(uuid = DONGLE_EVENT_SERVICE_UUID)]
pub(crate) struct DongleEventService {
    #[characteristic(uuid = DONGLE_EVENT_CHAR_UUID, notify, permissions(encrypted))]
    pub(crate) event: heapless::Vec<u8, DONGLE_EVENT_MAX>,
    #[cfg(feature = "custom_message")]
    #[characteristic(uuid = CUSTOM_TO_DONGLE_UUID, notify, permissions(encrypted))]
    pub(crate) custom_to_dongle: heapless::Vec<u8, { crate::custom_message::CustomMessage::POSTCARD_MAX_SIZE }>,
    #[cfg(feature = "custom_message")]
    #[characteristic(uuid = CUSTOM_TO_KEYBOARD_UUID, write_without_response, permissions(encrypted))]
    pub(crate) custom_to_keyboard: heapless::Vec<u8, { crate::custom_message::CustomMessage::POSTCARD_MAX_SIZE }>,
}

/// Rynk-over-GATT transport. The host writes request chunks to `output_data`;
/// the firmware replies via `input_data` notify. Variable-length values use
/// `heapless::Vec` so each notify only spends MTU − 3 bytes for the bytes it
/// actually carries (a fixed `[u8; N]` would always send N).
///
/// `gatt_events_task` feeds `output_data` writes to the connection's Rynk pipe.
#[cfg(feature = "rynk")]
#[gatt_service(uuid = RYNK_SERVICE_UUID)]
pub(crate) struct RynkGattService {
    // ATT enforces encryption; the event loop still filters the data path.
    #[descriptor(uuid = "2908", read, value = [0u8, 1u8])]
    #[characteristic(uuid = RYNK_INPUT_CHAR_UUID, read, notify, permissions(encrypted))]
    pub(crate) input_data: heapless::Vec<u8, RYNK_BLE_CHUNK_SIZE>,
    #[characteristic(uuid = RYNK_OUTPUT_CHAR_UUID, read, write, write_without_response, permissions(encrypted))]
    pub(crate) output_data: heapless::Vec<u8, RYNK_BLE_CHUNK_SIZE>,
}

/// Rynk HID-over-GATT service.
/// `gatt_events_task` feeds the payload to the same connection's Rynk pipe.
#[cfg(feature = "rynk")]
#[gatt_service(uuid = service::HUMAN_INTERFACE_DEVICE)]
pub(crate) struct RynkHidService {
    #[characteristic(uuid = "2a4a", read, value = [0x01, 0x01, 0x00, 0x03])]
    pub(crate) hid_info: [u8; 4],
    #[characteristic(uuid = "2a4b", read, value = RynkHidReport::desc())]
    pub(crate) report_map: &'static [u8],
    #[characteristic(uuid = "2a4c", write_without_response)]
    pub(crate) hid_control_point: u8,
    #[characteristic(uuid = "2a4e", read, write_without_response, value = 1)]
    pub(crate) protocol_mode: u8,
    // Report characteristics carry rynk frames — require an encrypted link at
    // the ATT layer (the app guard in `gatt_events_task` remains the data-path filter).
    #[descriptor(uuid = "2908", read, value = [0u8, 1u8])]
    #[characteristic(uuid = "2a4d", read, notify, permissions(encrypted))]
    pub(crate) input_data: [u8; RYNK_HID_REPORT_SIZE],
    #[descriptor(uuid = "2908", read, value = [0u8, 2u8])]
    #[characteristic(uuid = "2a4d", read, write, write_without_response, permissions(encrypted))]
    pub(crate) output_data: [u8; RYNK_HID_REPORT_SIZE],
}

/// GATT service exposing the Vial-over-HID protocol. The keyboard writes replies via
/// `input_data` notify; hosts push requests through `output_data`. `gatt_events_task`
/// forwards `output_data` writes to the connection's Vial queue.
#[cfg(feature = "vial")]
#[gatt_service(uuid = service::HUMAN_INTERFACE_DEVICE)]
pub(crate) struct VialGattService {
    #[characteristic(uuid = "2a4a", read, value = [0x01, 0x01, 0x00, 0x03])]
    pub(crate) hid_info: [u8; 4],
    #[characteristic(uuid = "2a4b", read, value = ViaReport::desc())]
    pub(crate) report_map: &'static [u8],
    #[characteristic(uuid = "2a4c", write_without_response)]
    pub(crate) hid_control_point: u8,
    #[characteristic(uuid = "2a4e", read, write_without_response, value = 1)]
    pub(crate) protocol_mode: u8,
    #[descriptor(uuid = "2908", read, value = [0u8, 1u8])]
    #[characteristic(uuid = "2a4d", read, notify)]
    pub(crate) input_data: [u8; 32],
    #[descriptor(uuid = "2908", read, value = [0u8, 2u8])]
    #[characteristic(uuid = "2a4d", read, write, write_without_response)]
    pub(crate) output_data: [u8; 32],
}

/// The single HID service carrying all reports, distinguished by report id via
/// each characteristic's Report Reference descriptor. Android's HID host only
/// attaches to the first HID service instance, so the reports must not be
/// spread over multiple service instances.
#[gatt_service(uuid = service::HUMAN_INTERFACE_DEVICE)]
pub(crate) struct HidService {
    #[characteristic(uuid = "2a4a", read, value = [0x01, 0x01, 0x00, 0x03])]
    pub(crate) hid_info: [u8; 4],
    // A `&'static` read-only value is served from flash instead of a RAM copy.
    #[characteristic(uuid = "2a4b", read, value = BleCompositeReport::desc())]
    pub(crate) report_map: &'static [u8],
    #[characteristic(uuid = "2a4c", write_without_response)]
    pub(crate) hid_control_point: u8,
    #[characteristic(uuid = "2a4e", read, write_without_response, value = 1)]
    pub(crate) protocol_mode: u8,
    #[descriptor(uuid = "2908", read, value = [CompositeReportType::Keyboard as u8, 1u8])]
    #[characteristic(uuid = "2a4d", read, notify)]
    pub(crate) input_keyboard: [u8; 8],
    #[descriptor(uuid = "2908", read, value = [CompositeReportType::Keyboard as u8, 2u8])]
    #[characteristic(uuid = "2a4d", read, write, write_without_response)]
    pub(crate) output_keyboard: [u8; 1],
    #[descriptor(uuid = "2908", read, value = [CompositeReportType::Mouse as u8, 1u8])]
    #[characteristic(uuid = "2a4d", read, notify)]
    pub(crate) mouse_report: [u8; MOUSE_REPORT_SIZE],
    #[descriptor(uuid = "2908", read, value = [CompositeReportType::Media as u8, 1u8])]
    #[characteristic(uuid = "2a4d", read, notify)]
    pub(crate) media_report: [u8; 2],
    #[descriptor(uuid = "2908", read, value = [CompositeReportType::System as u8, 1u8])]
    #[characteristic(uuid = "2a4d", read, notify)]
    pub(crate) system_report: [u8; 1],
}

pub(super) struct HidConnection<'stack, 'server, 'conn> {
    pub(super) service: &'conn HidService,
    pub(super) conn: &'conn GattConnection<'stack, 'server, DefaultPacketPool>,
    pub(super) leds: Cell<LedIndicator>,
}

impl HidConnection<'_, '_, '_> {
    /// Callers decide whether a failure is worth logging.
    pub(super) async fn write_report(&self, report: &Report) -> Result<(), trouble_host::Error> {
        // Every report goes through one `notify_raw`: a typed `notify` would be instantiated
        // and inlined once per characteristic value type.
        fn raw<const N: usize>(characteristic: &Characteristic<[u8; N]>) -> (Characteristic<[u8]>, usize) {
            (characteristic.to_raw(), N)
        }
        let (characteristic, len) = match report {
            Report::KeyboardReport(_) => raw(&self.service.input_keyboard),
            Report::MouseReport(_) => raw(&self.service.mouse_report),
            Report::MediaKeyboardReport(_) => raw(&self.service.media_report),
            Report::SystemControlReport(_) => raw(&self.service.system_report),
            // Plover HID over BLE is not supported: the stock HID-over-GATT service
            // has no stenography characteristic. Drop silently at the writer.
            #[cfg(feature = "steno")]
            Report::StenoReport(_) => {
                debug!("Steno chord dropped: Plover HID over BLE is not supported");
                return Ok(());
            }
        };
        // The mouse report is the longest one.
        let mut buf = [0u8; MOUSE_REPORT_SIZE];
        let buf = &mut buf[..len];
        report.serialize(buf).map_err(|_| trouble_host::Error::InvalidValue)?;
        characteristic.notify_raw(self.conn, buf, true).await
    }
}
