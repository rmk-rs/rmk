use trouble_host::prelude::*;
use usbd_hid::descriptor::{AsInputReport, SerializedDescriptor};

use super::battery_service::BatteryService;
#[cfg(feature = "split")]
use super::battery_service::PeripheralBatteryServices;
use super::device_info::DeviceConfigurationService;
#[cfg(feature = "rynk")]
use crate::hid::RynkHidReport;
#[cfg(feature = "vial")]
use crate::hid::ViaReport;
use crate::hid::{BleCompositeReport, CompositeReportType, HidError, HidWriterTrait, Report};

// Used for saving the client attribute (CCCD) table. Tracks the trouble-host
// per-connection client-specific attribute buffer size.
pub(crate) const CCCD_TABLE_SIZE: usize = trouble_host::config::CLIENT_ATT_TABLE_SIZE;

#[cfg(feature = "rynk")]
use rmk_types::protocol::rynk::{
    RYNK_BLE_CHUNK_SIZE, RYNK_HID_REPORT_SIZE, RYNK_INPUT_CHAR_UUID, RYNK_OUTPUT_CHAR_UUID, RYNK_SERVICE_UUID,
};

#[cfg(all(feature = "vial", feature = "split"))]
#[gatt_server]
pub(crate) struct Server {
    pub(crate) battery_service: BatteryService,
    pub(crate) peripheral_battery_services: PeripheralBatteryServices,
    pub(crate) hid_service: HidService,
    pub(crate) vial_service: VialGattService,
    pub(crate) device_config_service: DeviceConfigurationService,
}

#[cfg(all(feature = "vial", not(feature = "split")))]
#[gatt_server]
pub(crate) struct Server {
    pub(crate) battery_service: BatteryService,
    pub(crate) hid_service: HidService,
    pub(crate) vial_service: VialGattService,
    pub(crate) device_config_service: DeviceConfigurationService,
}

#[cfg(all(feature = "rynk", feature = "split"))]
#[gatt_server]
pub(crate) struct Server {
    pub(crate) battery_service: BatteryService,
    pub(crate) peripheral_battery_services: PeripheralBatteryServices,
    pub(crate) hid_service: HidService,
    pub(crate) rynk_service: RynkGattService,
    pub(crate) rynk_hid_service: RynkHidService,
    pub(crate) device_config_service: DeviceConfigurationService,
}

#[cfg(all(feature = "rynk", not(feature = "split")))]
#[gatt_server]
pub(crate) struct Server {
    pub(crate) battery_service: BatteryService,
    pub(crate) hid_service: HidService,
    pub(crate) rynk_service: RynkGattService,
    pub(crate) rynk_hid_service: RynkHidService,
    pub(crate) device_config_service: DeviceConfigurationService,
}

#[cfg(all(not(feature = "host"), feature = "split"))]
#[gatt_server]
pub(crate) struct Server {
    pub(crate) battery_service: BatteryService,
    pub(crate) peripheral_battery_services: PeripheralBatteryServices,
    pub(crate) hid_service: HidService,
    pub(crate) device_config_service: DeviceConfigurationService,
}

#[cfg(all(not(feature = "host"), not(feature = "split")))]
#[gatt_server]
pub(crate) struct Server {
    pub(crate) battery_service: BatteryService,
    pub(crate) hid_service: HidService,
    pub(crate) device_config_service: DeviceConfigurationService,
}

/// Rynk-over-GATT transport. The host writes request chunks to `output_data`;
/// the firmware replies via `input_data` notify. Variable-length values use
/// `heapless::Vec` so each notify only spends MTU − 3 bytes for the bytes it
/// actually carries (a fixed `[u8; N]` would always send N).
///
/// `gatt_events_task` forwards `output_data` writes into
/// [`crate::channel::RYNK_BLE_RX_PIPE`] for [`crate::ble::host::HostGattHandler::run`] to drain.
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
/// `gatt_events_task` feeds the payload into [`crate::channel::RYNK_BLE_RX_PIPE`],
/// [`crate::ble::host::HostGattHandler::run`] drains the pipe to get data.
#[cfg(feature = "rynk")]
#[gatt_service(uuid = service::HUMAN_INTERFACE_DEVICE)]
pub(crate) struct RynkHidService {
    #[characteristic(uuid = "2a4a", read, value = [0x01, 0x01, 0x00, 0x03])]
    pub(crate) hid_info: [u8; 4],
    #[characteristic(uuid = "2a4b", read, value = RynkHidReport::desc().try_into().expect("Failed to convert RynkHidReport to [u8; 27]"))]
    pub(crate) report_map: [u8; 27],
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
/// forwards `output_data` writes into [`crate::channel::VIAL_BLE_RX_CHANNEL`] for
/// [`crate::ble::host::HostGattHandler::run`] to drain.
#[cfg(feature = "vial")]
#[gatt_service(uuid = service::HUMAN_INTERFACE_DEVICE)]
pub(crate) struct VialGattService {
    #[characteristic(uuid = "2a4a", read, value = [0x01, 0x01, 0x00, 0x03])]
    pub(crate) hid_info: [u8; 4],
    #[characteristic(uuid = "2a4b", read, value = ViaReport::desc().try_into().expect("Failed to convert ViaReport to [u8; 27]"))]
    pub(crate) report_map: [u8; 27],
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
    #[characteristic(uuid = "2a4b", read, value = BleCompositeReport::desc().try_into().expect("Failed to convert BleCompositeReport to [u8; 178]"))]
    pub(crate) report_map: [u8; 178],
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
    pub(crate) mouse_report: [u8; 5],
    #[descriptor(uuid = "2908", read, value = [CompositeReportType::Media as u8, 1u8])]
    #[characteristic(uuid = "2a4d", read, notify)]
    pub(crate) media_report: [u8; 2],
    #[descriptor(uuid = "2908", read, value = [CompositeReportType::System as u8, 1u8])]
    #[characteristic(uuid = "2a4d", read, notify)]
    pub(crate) system_report: [u8; 1],
}

pub(crate) struct BleHidServer<'stack, 'server, 'conn, P: PacketPool> {
    input_keyboard: Characteristic<[u8; 8]>,
    mouse_report: Characteristic<[u8; 5]>,
    media_report: Characteristic<[u8; 2]>,
    system_report: Characteristic<[u8; 1]>,
    conn: &'conn GattConnection<'stack, 'server, P>,
}

impl<'stack, 'server, 'conn, P: PacketPool> BleHidServer<'stack, 'server, 'conn, P> {
    pub(crate) fn new(server: &Server, conn: &'conn GattConnection<'stack, 'server, P>) -> Self {
        Self {
            input_keyboard: server.hid_service.input_keyboard,
            mouse_report: server.hid_service.mouse_report,
            media_report: server.hid_service.media_report,
            system_report: server.hid_service.system_report,
            conn,
        }
    }

    async fn notify_report<R: AsInputReport, const N: usize>(
        &self,
        characteristic: Characteristic<[u8; N]>,
        report: &R,
    ) -> Result<usize, HidError> {
        let mut buf = [0u8; N];
        let n = report.serialize(&mut buf).map_err(|_| HidError::ReportSerializeError)?;
        characteristic.notify(self.conn, &buf, true).await.map_err(|e| {
            error!("Failed to notify HID report: {:?}", e);
            HidError::BleError
        })?;
        Ok(n)
    }
}

impl<P: PacketPool> HidWriterTrait for BleHidServer<'_, '_, '_, P> {
    type ReportType = Report;

    async fn write_report(&mut self, report: &Self::ReportType) -> Result<usize, HidError> {
        match report {
            Report::KeyboardReport(r) => self.notify_report(self.input_keyboard, r).await,
            Report::MouseReport(r) => self.notify_report(self.mouse_report, r).await,
            Report::MediaKeyboardReport(r) => self.notify_report(self.media_report, r).await,
            Report::SystemControlReport(r) => self.notify_report(self.system_report, r).await,
            // Plover HID over BLE is not supported: the stock HID-over-GATT service
            // has no stenography characteristic. Drop silently at the writer.
            #[cfg(feature = "steno")]
            Report::StenoReport(_) => {
                debug!("Steno chord dropped: Plover HID over BLE is not supported");
                Ok(0)
            }
        }
    }
}
