use rmk_types::DEVICE_INFO_STRING_MAX_LEN;

/// Device identity reported over USB, BLE, and Rynk.
#[derive(Clone, Debug)]
pub struct DeviceConfig<'a> {
    /// USB vendor ID.
    pub vid: u16,
    /// USB product ID.
    pub pid: u16,
    /// USB device release number (bcdDevice), encoded in binary-coded decimal.
    pub bcd_version: u16,
    /// Manufacturer name.
    pub manufacturer: &'a str,
    /// Product name.
    pub product_name: &'a str,
    /// Complete serial number, including the protocol marker. See [`build_serial_number`].
    pub serial_number: heapless::String<DEVICE_INFO_STRING_MAX_LEN>,
}

// Vial discovers keyboards by this prefix.
#[cfg(feature = "vial")]
const PROTOCOL_MARKER: &str = "vial:f64c2b3c:";
#[cfg(all(not(feature = "vial"), any(feature = "rynk", feature = "dongle")))]
const PROTOCOL_MARKER: &str = rmk_types::protocol::rynk::RYNK_MAGIC;
#[cfg(not(any(feature = "vial", feature = "rynk", feature = "dongle")))]
const PROTOCOL_MARKER: &str = "";

/// A custom ID or hardware UID accepted by [`build_serial_number`].
pub trait SerialNumberInput {
    /// Build a serial number using the rules in [`build_serial_number`].
    fn into_serial_number(self) -> heapless::String<DEVICE_INFO_STRING_MAX_LEN>;
}

impl SerialNumberInput for &str {
    fn into_serial_number(self) -> heapless::String<DEVICE_INFO_STRING_MAX_LEN> {
        // Older Vial serials used a semicolon before the device ID.
        let id = match self.strip_prefix("vial:f64c2b3c") {
            Some(rest) => rest.trim_start_matches([':', ';']),
            None => self.strip_prefix("rynk:").unwrap_or(self),
        };
        let id = if id.is_empty() { "RMK" } else { id };
        let mut serial = heapless::String::new();
        serial
            .push_str(PROTOCOL_MARKER)
            .and_then(|_| serial.push_str(id))
            .expect("serial_number exceeds DEVICE_INFO_STRING_MAX_LEN including its protocol marker");
        serial
    }
}

impl SerialNumberInput for &[u8] {
    fn into_serial_number(self) -> heapless::String<DEVICE_INFO_STRING_MAX_LEN> {
        if self.is_empty() {
            return build_serial_number("");
        }
        let id = if self.len() <= 3 {
            self.iter().fold(0, |id, &byte| id << 8 | u32::from(byte))
        } else {
            // 32-bit FNV-1a, XOR-folded to 24 bits
            let hash = self.iter().fold(0x811c_9dc5_u32, |hash, &byte| {
                (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
            });
            (hash >> 24) ^ (hash & 0x00ff_ffff)
        };
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let generated: heapless::String<6> = (0..6)
            .rev()
            .map(|digit| HEX[(id >> (digit * 4)) as usize & 0xf] as char)
            .collect();
        build_serial_number(generated.as_str())
    }
}

impl<const N: usize> SerialNumberInput for &[u8; N] {
    fn into_serial_number(self) -> heapless::String<DEVICE_INFO_STRING_MAX_LEN> {
        self.as_slice().into_serial_number()
    }
}

/// Build a serial number with the current protocol marker.
///
/// Pass `&str` for a custom ID, or `&[u8]` / `&[u8; N]` for a UID. UIDs become six
/// hex digits, hashing inputs longer than three bytes. Empty IDs fall back to `RMK`.
/// Existing protocol markers are replaced.
///
/// # Panics
///
/// Panics if the result, including its marker, exceeds [`DEVICE_INFO_STRING_MAX_LEN`] bytes.
pub fn build_serial_number(input: impl SerialNumberInput) -> heapless::String<DEVICE_INFO_STRING_MAX_LEN> {
    input.into_serial_number()
}

/// Build a serial number from the RP2040 flash UID, using `RMK` if unavailable.
/// Pass `p.FLASH.reborrow()` before creating a storage or DFU driver.
#[cfg(feature = "rp2040")]
pub fn rp2040_serial_number(
    flash: embassy_rp::Peri<'_, embassy_rp::peripherals::FLASH>,
) -> heapless::String<DEVICE_INFO_STRING_MAX_LEN> {
    let mut flash = embassy_rp::flash::Flash::<_, embassy_rp::flash::Blocking, 0>::new_blocking(flash);
    // Flash chips answer the Winbond UID command with 8 bytes, except Zetta's 16.
    let len = if matches!(flash.blocking_jedec_id(), Ok(0xba6015)) {
        16
    } else {
        8
    };
    let mut uid = [0; 16];
    let uid = &mut uid[..len];
    match flash.blocking_unique_id(uid) {
        // A chip without the command reads back blank.
        Ok(()) if uid.iter().any(|&b| b != 0) && uid.iter().any(|&b| b != 0xff) => build_serial_number(&uid[..]),
        _ => build_serial_number(&[]),
    }
}

impl Default for DeviceConfig<'_> {
    fn default() -> Self {
        #[cfg(feature = "_nrf_ble")]
        let serial_number = {
            #[cfg(any(feature = "nrf54l15_ble", feature = "nrf54lm20_ble"))]
            let id = embassy_nrf::pac::FICR.deviceaddr(1).read();
            #[cfg(not(any(feature = "nrf54l15_ble", feature = "nrf54lm20_ble")))]
            let id = embassy_nrf::pac::FICR.deviceid(1).read();
            // Keep the 24 bits used by existing nRF device IDs.
            build_serial_number(&id.to_be_bytes()[..3])
        };
        // Skip the vendor prefix shared by Espressif chips.
        #[cfg(feature = "_esp_ble")]
        let serial_number = build_serial_number(&esp_hal::efuse::base_mac_address().as_bytes()[3..]);
        #[cfg(not(any(feature = "_nrf_ble", feature = "_esp_ble")))]
        let serial_number = build_serial_number(&[]);
        Self {
            vid: 0x4c4b,
            pid: 0x4643,
            bcd_version: 0x0010,
            manufacturer: "RMK",
            product_name: "RMK Keyboard",
            serial_number,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DEVICE_INFO_STRING_MAX_LEN, DeviceConfig, PROTOCOL_MARKER, build_serial_number};

    fn marked(id: &str) -> heapless::String<DEVICE_INFO_STRING_MAX_LEN> {
        let mut serial = heapless::String::new();
        serial.push_str(PROTOCOL_MARKER).unwrap();
        serial.push_str(id).unwrap();
        serial
    }

    #[test]
    fn short_uid_is_used_as_given() {
        assert_eq!(build_serial_number(&[0x01, 0x23, 0xab]), marked("0123ab"));
        assert_eq!(build_serial_number(&[0x7f]), marked("00007f"));
        assert_eq!(build_serial_number(&[0, 0, 0]), marked("000000"));
    }

    #[test]
    fn long_uid_hash_is_stable() {
        // Changing the hash changes the serial number of every board that uses it.
        let uid = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
        assert_eq!(build_serial_number(&uid), marked("3f1176"));
    }

    #[test]
    fn explicit_id_gets_the_protocol_marker() {
        assert_eq!(build_serial_number("000001"), marked("000001"));
        assert_eq!(build_serial_number("board-1"), marked("board-1"));
    }

    #[test]
    fn input_type_selects_custom_id_or_uid() {
        let uid = b"123";
        assert_eq!(build_serial_number("123"), marked("123"));
        assert_eq!(build_serial_number(uid), marked("313233"));
        assert_eq!(build_serial_number(uid.as_slice()), marked("313233"));
    }

    #[test]
    fn existing_markers_are_replaced() {
        assert_eq!(build_serial_number("vial:f64c2b3c:000001"), marked("000001"));
        assert_eq!(build_serial_number("vial:f64c2b3c;board:1"), marked("board:1"));
        assert_eq!(build_serial_number("rynk:000001"), marked("000001"));
    }

    #[test]
    fn missing_id_falls_back_to_rmk() {
        assert_eq!(build_serial_number(&[]), marked("RMK"));
        assert_eq!(build_serial_number(b"".as_slice()), marked("RMK"));
        assert_eq!(build_serial_number(""), marked("RMK"));
        assert_eq!(build_serial_number("vial:f64c2b3c"), marked("RMK"));
        assert_eq!(DeviceConfig::default().serial_number, marked("RMK"));
    }

    #[test]
    fn serial_number_fills_the_shared_capacity() {
        let id = "0123456789abcdef0123456789abcdef";
        let id = &id[..DEVICE_INFO_STRING_MAX_LEN - PROTOCOL_MARKER.len()];
        let serial = build_serial_number(id);
        assert_eq!(serial.len(), DEVICE_INFO_STRING_MAX_LEN);
        assert_eq!(serial, marked(id));
    }

    #[test]
    #[should_panic(expected = "serial_number exceeds DEVICE_INFO_STRING_MAX_LEN including its protocol marker")]
    fn serial_number_rejects_one_byte_over_capacity() {
        let id = "0123456789abcdef0123456789abcdef0";
        build_serial_number(&id[..DEVICE_INFO_STRING_MAX_LEN + 1 - PROTOCOL_MARKER.len()]);
    }
}
