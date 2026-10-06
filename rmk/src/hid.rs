/// Traits and types for HID message reporting and listening.
use core::future::Future;
use core::sync::atomic::Ordering;

use embassy_usb::class::hid::ReadError;
use embassy_usb::driver::EndpointError;
use rmk_types::connection::ConnectionType;
use rmk_types::led_indicator::LedIndicator;
#[cfg(feature = "rynk")]
use rmk_types::protocol::rynk::RYNK_HID_REPORT_SIZE;
use serde::Serialize;
use usbd_hid::descriptor::generator_prelude::*;
use usbd_hid::descriptor::{AsInputReport, BufferOverflow, MediaKeyboardReport, SystemControlReport};

use crate::event::{LedIndicatorEvent, publish_event};
use crate::keyboard::LOCK_LED_STATES;

/// KeyboardReport describes a report and its companion descriptor that can be
/// used to send keyboard button presses to a host and receive the status of the
/// keyboard LEDs.
#[gen_hid_descriptor(
    (collection = APPLICATION, usage_page = GENERIC_DESKTOP, usage = KEYBOARD) = {
        (usage_page = KEYBOARD, usage_min = 0xE0, usage_max = 0xE7) = {
            #[packed_bits = 8] #[item_settings(data,variable,absolute)] modifier=input;
        };
        (logical_min = 0,) = {
            #[item_settings(constant,variable,absolute)] reserved=input;
        };
        (usage_page = LEDS, usage_min = 0x01, usage_max = 0x05) = {
            #[packed_bits = 5] #[item_settings(data,variable,absolute)] leds=output;
        };
        (usage_page = KEYBOARD, usage_min = 0x00, usage_max = 0xDD) = {
            #[item_settings(data,array,absolute)] keycodes=input;
        };
    }
)]
#[allow(dead_code)]
#[derive(Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct KeyboardReport {
    pub modifier: u8, // ModifierCombination
    pub reserved: u8,
    pub leds: u8, // LedIndicator
    pub keycodes: [u8; 6],
}

#[gen_hid_descriptor(
    (collection = APPLICATION, usage_page = 0xFF60, usage = 0x61) = {
        (usage = 0x62, logical_min = 0x0) = {
            #[item_settings(data,variable,absolute)] input_data=input;
        };
        (usage = 0x63, logical_min = 0x0) = {
            #[item_settings(data,variable,absolute)] output_data=output;
        };
    }
)]
#[derive(Default)]
pub struct ViaReport {
    pub(crate) input_data: [u8; 32],
    pub(crate) output_data: [u8; 32],
}

/// Vendor HID report carrying the Rynk config protocol over HID.
#[cfg(feature = "rynk")]
#[gen_hid_descriptor(
    (collection = APPLICATION, usage_page = 0xFF14, usage = 0x61) = {
        (usage = 0x62, logical_min = 0x0) = {
            #[item_settings(data,variable,absolute)] input_data=input;
        };
        (usage = 0x63, logical_min = 0x0) = {
            #[item_settings(data,variable,absolute)] output_data=output;
        };
    }
)]
#[derive(Default)]
pub struct RynkHidReport {
    // `gen_hid_descriptor` needs a literal length; keep it in lockstep with
    // RYNK_HID_REPORT_SIZE (asserted below), which sizes the GATT chars/channel.
    pub(crate) input_data: [u8; 32],
    pub(crate) output_data: [u8; 32],
}

// `core::assert!`: a `defmt` build's crate-level `assert!` isn't const-callable.
#[cfg(feature = "rynk")]
const _: () = core::assert!(
    RYNK_HID_REPORT_SIZE == 32,
    "RynkHidReport literal length must equal RYNK_HID_REPORT_SIZE"
);

/// Predefined report ids for composite hid report.
/// Should be same with `#[gen_hid_descriptor]` of `CompositeReport` and `BleCompositeReport`
/// DO NOT EDIT
#[repr(u8)]
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]

pub enum CompositeReportType {
    #[default]
    None = 0x00,
    /// Used only in the BLE report map; the USB keyboard interface stays a
    /// report-ID-less boot keyboard.
    Keyboard = 0x01,
    Mouse = 0x02,
    Media = 0x03,
    System = 0x04,
}

/// Plover HID stenography report.
///
/// Plover (v5.1+) enumerates the keyboard as a stenography machine when it
/// finds an HID device exposing usage page `0xFF50` / usage `0x4C56`; the
/// pair encodes the ASCII string `"STN"` (`0xFF`, `'S'`, `'T'`, `'N'`).
/// Once connected, Plover reads 9-byte reports (`[report_id=0x50, k0, k1,
/// ..., k7]`) where the eight payload bytes are a 64-bit big-endian bitmap
/// of the live steno chord, one bit per [`crate::types::steno::StenoKey`],
/// where `StenoKey::S1` (chart index 0) is the most significant bit of `k0`
/// and `StenoKey::X26` (chart index 63) is the least significant bit of
/// `k7`.
///
/// The descriptor is the same as the Plover HID project's reference: a
/// Logical collection containing 64 single-bit Ordinal usages.
///
/// Reference: <https://github.com/dnaq/plover-machine-hid>
#[cfg(feature = "steno")]
#[gen_hid_descriptor(
    (collection = LOGICAL, usage_page = 0xFF50, usage = 0x4C56) = {
        (report_id = 0x50, usage_page = 0x0A, usage_min = 0x0, usage_max = 0x3F, logical_min = 0x0) = {
            #[packed_bits = 64] #[item_settings(data,variable,absolute)] keys=input;
        };
    }
)]
#[derive(Default)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct StenoReport {
    pub keys: [u8; 8],
}

// `gen_hid_descriptor` skips the `AsInputReport` impl when a `report_id`
// is present, so the wire format must be assembled by hand: byte 0 is the
// Plover HID report ID followed by the eight chord-bitmap bytes.
#[cfg(feature = "steno")]
impl usbd_hid::descriptor::AsInputReport for StenoReport {
    fn serialize(&self, buffer: &mut [u8]) -> Result<usize, usbd_hid::descriptor::BufferOverflow> {
        if buffer.len() < 9 {
            return Err(usbd_hid::descriptor::BufferOverflow);
        }
        buffer[0] = rmk_types::steno::PLOVER_HID_REPORT_ID;
        buffer[1..9].copy_from_slice(&self.keys);
        Ok(9)
    }
}

#[cfg(all(test, feature = "steno"))]
mod steno_tests {
    use usbd_hid::descriptor::SerializedDescriptor;

    use super::StenoReport;

    #[test]
    fn descriptor_advertises_plover_identifiers() {
        let desc = StenoReport::desc();
        fn contains(haystack: &[u8], needle: &[u8]) -> bool {
            haystack.windows(needle.len()).any(|w| w == needle)
        }
        assert!(contains(desc, &[0x06, 0x50, 0xff]), "missing UsagePage 0xFF50");
        assert!(contains(desc, &[0x0a, 0x56, 0x4c]), "missing Usage 0x4C56");
        assert!(contains(desc, &[0xa1, 0x02]), "missing Logical collection");
        assert!(contains(desc, &[0x85, 0x50]), "missing ReportID 0x50");
        assert!(contains(desc, &[0x75, 0x01]), "missing ReportSize 1");
        assert!(contains(desc, &[0x95, 0x40]), "missing ReportCount 64");
        assert!(contains(desc, &[0x05, 0x0a]), "missing Ordinal UsagePage");
        assert!(contains(desc, &[0x19, 0x00]), "missing UsageMin 0");
        assert!(contains(desc, &[0x29, 0x3f]), "missing UsageMax 63");
    }
}

/// Wire size of [`MouseReport`]: sizes the BLE report characteristic and the
/// USB composite write buffer.
pub(crate) const MOUSE_REPORT_SIZE: usize = 9;

/// Mouse HID report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct MouseReport {
    pub buttons: u8, // MouseButtons
    pub x: i16,
    pub y: i16,
    /// Scroll down (negative) or up (positive) this many units
    pub wheel: i16,
    /// Scroll left (negative) or right (positive) this many units
    pub pan: i16,
}

/// Hand-written because `gen_hid_descriptor` emits a `repr(packed)` struct,
/// which with 16-bit axes makes every `&report.x` a compile error for callers.
impl AsInputReport for MouseReport {
    fn serialize(&self, buffer: &mut [u8]) -> Result<usize, BufferOverflow> {
        if buffer.len() < MOUSE_REPORT_SIZE {
            return Err(BufferOverflow);
        }
        buffer[0] = self.buttons;
        buffer[1..3].copy_from_slice(&self.x.to_le_bytes());
        buffer[3..5].copy_from_slice(&self.y.to_le_bytes());
        buffer[5..7].copy_from_slice(&self.wheel.to_le_bytes());
        buffer[7..9].copy_from_slice(&self.pan.to_le_bytes());
        Ok(MOUSE_REPORT_SIZE)
    }
}

/// A composite hid report which contains mouse, consumer, system reports.
/// Report id is used to distinguish from them.
#[gen_hid_descriptor(
    (collection = APPLICATION, usage_page = GENERIC_DESKTOP, usage = MOUSE) = {
        (collection = PHYSICAL, usage = POINTER) = {
            (report_id = 0x02,) = {
                (usage_page = BUTTON, usage_min = BUTTON_1, usage_max = BUTTON_8) = {
                    #[packed_bits = 8] #[item_settings(data,variable,absolute)] buttons=input;
                };
                (usage_page = GENERIC_DESKTOP,) = {
                    (usage = X,) = {
                        #[item_settings(data,variable,relative)] x=input;
                    };
                    (usage = Y,) = {
                        #[item_settings(data,variable,relative)] y=input;
                    };
                    (usage = WHEEL,) = {
                        #[item_settings(data,variable,relative)] wheel=input;
                    };
                };
                (usage_page = CONSUMER,) = {
                    (usage = AC_PAN,) = {
                        #[item_settings(data,variable,relative)] pan=input;
                    };
                };
            };
        };
    },
    (collection = APPLICATION, usage_page = CONSUMER, usage = CONSUMER_CONTROL) = {
        (report_id = 0x03,) = {
            (usage_page = CONSUMER, usage_min = 0x00, usage_max = 0x514) = {
            #[item_settings(data,array,absolute,not_null)] media_usage_id=input;
            }
        };
    },
    (collection = APPLICATION, usage_page = GENERIC_DESKTOP, usage = SYSTEM_CONTROL) = {
        (report_id = 0x04,) = {
            (usage_min = 0x01, usage_max = 0xB7, logical_min = 1) = {
                #[item_settings(data,array,absolute,not_null)] system_usage_id=input;
            };
        };
    }
)]
#[derive(Default, Serialize)]
pub struct CompositeReport {
    pub(crate) buttons: u8, // MouseButtons
    pub(crate) x: i16,
    pub(crate) y: i16,
    pub(crate) wheel: i16, // Scroll down (negative) or up (positive) this many units
    pub(crate) pan: i16,   // Scroll left (negative) or right (positive) this many units
    pub(crate) media_usage_id: u16,
    pub(crate) system_usage_id: u8,
}

#[cfg(test)]
mod mouse_report_tests {
    use usbd_hid::descriptor::{AsInputReport, SerializedDescriptor};

    use super::{CompositeReport, MOUSE_REPORT_SIZE, MouseReport};

    /// The report map tells the host how to parse the axes and `serialize`
    /// writes them; only this test holds the two to the same widths.
    #[test]
    fn mouse_report_matches_composite_descriptor() {
        let desc = CompositeReport::desc();
        fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
            haystack.windows(needle.len()).position(|w| w == needle)
        }
        let start = find(desc, &[0x85, 0x02]).expect("missing mouse ReportID 2");
        let end = find(desc, &[0x85, 0x03]).expect("missing consumer ReportID 3");
        let mouse = &desc[start..end];
        assert!(
            find(mouse, &[0x17, 0x01, 0x80, 0xff, 0xff]).is_some(),
            "missing LogicalMinimum -32767"
        );
        assert!(
            find(mouse, &[0x26, 0xff, 0x7f]).is_some(),
            "missing LogicalMaximum 32767"
        );
        assert!(find(mouse, &[0x75, 0x10]).is_some(), "missing ReportSize 16");
        assert!(find(mouse, &[0x25, 0x7f]).is_none(), "an axis is still 8-bit");

        // A delta past the old 8-bit ceiling survives the wire encoding.
        let report = MouseReport {
            buttons: 0x03,
            x: 300,
            y: -300,
            wheel: 1,
            pan: -1,
        };
        let mut buf = [0u8; MOUSE_REPORT_SIZE];
        assert_eq!(report.serialize(&mut buf).unwrap(), MOUSE_REPORT_SIZE);
        assert_eq!(buf, [0x03, 0x2c, 0x01, 0xd4, 0xfe, 0x01, 0x00, 0xff, 0xff]);
    }
}

/// The BLE report map: everything in one HID service, distinguished by report id.
///
/// Android's HID host only attaches to the first HID service instance (AOSP
/// `bta_hh_le.cc`, b/286413526), so unlike USB the keyboard/mouse/media/system
/// reports must share a single service. Only `desc()` is used; the actual
/// payloads are still serialized from `KeyboardReport`, `MouseReport`, etc.,
/// as HID-over-GATT carries the report id in the Report Reference descriptor
/// instead of the payload.
#[cfg(feature = "_ble")]
#[gen_hid_descriptor(
    (collection = APPLICATION, usage_page = GENERIC_DESKTOP, usage = KEYBOARD) = {
        (report_id = 0x01,) = {
            (usage_page = KEYBOARD, usage_min = 0xE0, usage_max = 0xE7) = {
                #[packed_bits = 8] #[item_settings(data,variable,absolute)] modifier=input;
            };
            (logical_min = 0,) = {
                #[item_settings(constant,variable,absolute)] reserved=input;
            };
            (usage_page = LEDS, usage_min = 0x01, usage_max = 0x05) = {
                #[packed_bits = 5] #[item_settings(data,variable,absolute)] leds=output;
            };
            (usage_page = KEYBOARD, usage_min = 0x00, usage_max = 0xDD) = {
                #[item_settings(data,array,absolute)] keycodes=input;
            };
        };
    },
    (collection = APPLICATION, usage_page = GENERIC_DESKTOP, usage = MOUSE) = {
        (collection = PHYSICAL, usage = POINTER) = {
            (report_id = 0x02,) = {
                (usage_page = BUTTON, usage_min = BUTTON_1, usage_max = BUTTON_8) = {
                    #[packed_bits = 8] #[item_settings(data,variable,absolute)] buttons=input;
                };
                (usage_page = GENERIC_DESKTOP,) = {
                    (usage = X,) = {
                        #[item_settings(data,variable,relative)] x=input;
                    };
                    (usage = Y,) = {
                        #[item_settings(data,variable,relative)] y=input;
                    };
                    (usage = WHEEL,) = {
                        #[item_settings(data,variable,relative)] wheel=input;
                    };
                };
                (usage_page = CONSUMER,) = {
                    (usage = AC_PAN,) = {
                        #[item_settings(data,variable,relative)] pan=input;
                    };
                };
            };
        };
    },
    (collection = APPLICATION, usage_page = CONSUMER, usage = CONSUMER_CONTROL) = {
        (report_id = 0x03,) = {
            (usage_page = CONSUMER, usage_min = 0x00, usage_max = 0x514) = {
            #[item_settings(data,array,absolute,not_null)] media_usage_id=input;
            }
        };
    },
    (collection = APPLICATION, usage_page = GENERIC_DESKTOP, usage = SYSTEM_CONTROL) = {
        (report_id = 0x04,) = {
            (usage_min = 0x01, usage_max = 0xB7, logical_min = 1) = {
                #[item_settings(data,array,absolute,not_null)] system_usage_id=input;
            };
        };
    }
)]
#[allow(dead_code)]
#[derive(Default)]
pub struct BleCompositeReport {
    pub(crate) modifier: u8,
    pub(crate) reserved: u8,
    pub(crate) leds: u8,
    pub(crate) keycodes: [u8; 6],
    pub(crate) buttons: u8,
    pub(crate) x: i16,
    pub(crate) y: i16,
    pub(crate) wheel: i16,
    pub(crate) pan: i16,
    pub(crate) media_usage_id: u16,
    pub(crate) system_usage_id: u8,
}

#[cfg(all(test, feature = "_ble"))]
mod ble_report_map_tests {
    use usbd_hid::descriptor::SerializedDescriptor;

    use super::BleCompositeReport;

    /// Pins the report map: `ble_server::HidService` hardcodes its length, and
    /// the report ids must match `CompositeReportType`.
    #[test]
    fn ble_report_map_matches_service_definition() {
        let desc = BleCompositeReport::desc();
        fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
            haystack.windows(needle.len()).position(|w| w == needle)
        }
        assert_eq!(desc.len(), 177, "update HidService's report_map size on change");
        let keyboard = find(desc, &[0x09, 0x06]).expect("missing Usage Keyboard");
        for report_id in 1u8..=4 {
            let id = find(desc, &[0x85, report_id]).unwrap_or_else(|| panic!("missing ReportID {report_id}"));
            if report_id == 0x01 {
                assert!(keyboard < id, "keyboard collection must own ReportID 1");
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum Report {
    /// Normal keyboard hid report
    KeyboardReport(KeyboardReport),
    /// Mouse hid report
    MouseReport(MouseReport),
    /// Media keyboard report
    MediaKeyboardReport(MediaKeyboardReport),
    /// System control report
    SystemControlReport(SystemControlReport),
    /// Plover HID stenography chord report
    #[cfg(feature = "steno")]
    StenoReport(StenoReport),
}

impl AsInputReport for Report {
    fn serialize(&self, buffer: &mut [u8]) -> Result<usize, usbd_hid::descriptor::BufferOverflow> {
        match self {
            Report::KeyboardReport(r) => r.serialize(buffer),
            Report::MouseReport(r) => r.serialize(buffer),
            Report::MediaKeyboardReport(r) => r.serialize(buffer),
            Report::SystemControlReport(r) => r.serialize(buffer),
            #[cfg(feature = "steno")]
            Report::StenoReport(r) => r.serialize(buffer),
        }
    }
}

#[derive(PartialEq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum HidError {
    UsbReadError(ReadError),
    UsbEndpointError(EndpointError),
    ReportSerializeError,
    BleError,
}

/// HidWriter trait is used for reporting HID messages to the host, via USB, BLE, etc.
pub trait HidWriterTrait {
    /// The report type that the reporter receives from input processors.
    type ReportType: AsInputReport;

    /// Write report to the host, return the number of bytes written if success.
    fn write_report(&mut self, report: &Self::ReportType) -> impl Future<Output = Result<usize, HidError>>;
}

/// HidReader trait is used for listening to HID messages from the host, via USB, BLE, etc.
///
/// HidReader only receives `[u8; READ_N]`, the raw HID report from the host.
/// Then processes the received message, forward to other tasks
pub trait HidReaderTrait {
    /// Report type
    type ReportType;

    /// Read HID report from the host
    fn read_report(&mut self) -> impl Future<Output = Result<Self::ReportType, HidError>>;
}

/// Drain LED indicator OUT reports from `reader` and republish them as
/// [`LedIndicatorEvent`]s whenever `kind` is the active output transport.
pub(crate) async fn run_led_reader<R: HidReaderTrait<ReportType = LedIndicator>>(
    reader: &mut R,
    kind: ConnectionType,
) -> ! {
    loop {
        match reader.read_report().await {
            Ok(led_indicator) => {
                info!("Got led indicator");
                if crate::state::active_transport() == Some(kind) {
                    LOCK_LED_STATES.store(led_indicator.into_bits(), Ordering::Relaxed);
                    publish_event(LedIndicatorEvent::new(led_indicator));
                }
            }
            Err(e) => {
                debug!("Read HID LED indicator error: {:?}", e);
                embassy_time::Timer::after_millis(1000).await;
            }
        }
    }
}
