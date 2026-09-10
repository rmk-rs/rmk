use rmk_types::ble::BLE_DIS_STRING_MAX_LEN;
use trouble_host::prelude::*;

// The one string RMK supplies itself; `rmk_macro` rejects over-long ones from keyboard.toml.
const _: () = core::assert!(crate::config::RMK_BUILD_INFO.len() <= BLE_DIS_STRING_MAX_LEN);

#[repr(u8)]
#[derive(Clone, Copy)]
pub(crate) enum VidSource {
    BluetoothSIG = 1,
    UsbIF = 2,
}

/// PnP ID characteristic is a set of values used to create a unique device ID.
/// These values are used to identify all devices of a given type/model/version using numbers.
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub(crate) struct PnPID {
    pub(crate) vid_source: VidSource,
    pub(crate) vendor_id: u16,
    pub(crate) product_id: u16,
    pub(crate) product_version: u16,
}

impl AsGatt for PnPID {
    const MIN_SIZE: usize = core::mem::size_of::<PnPID>();

    const MAX_SIZE: usize = core::mem::size_of::<PnPID>();

    fn as_gatt(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self as *const _ as *const u8, core::mem::size_of::<PnPID>()) }
    }
}

impl Default for PnPID {
    fn default() -> Self {
        Self {
            vid_source: VidSource::UsbIF,
            vendor_id: 0xE118,
            product_id: 0x0001,
            product_version: 0x0001,
        }
    }
}

#[gatt_service(uuid = service::DEVICE_INFORMATION)]
pub(crate) struct DeviceConfigurationService {
    #[characteristic(uuid = "2a50", read)]
    pub(crate) pnp_id: PnPID,
    #[characteristic(
        uuid = "2a25",
        read,
        value = heapless::String::try_from("vial:f64c2b3c:000001").unwrap()
    )]
    pub(crate) serial_number: heapless::String<BLE_DIS_STRING_MAX_LEN>,
    #[characteristic(uuid = "2a29", read)]
    pub(crate) manufacturer_name: heapless::String<BLE_DIS_STRING_MAX_LEN>,
}
