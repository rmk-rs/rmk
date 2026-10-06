//! USB DFU runtime interface for dongles: `DFU_DETACH` reboots into the bootloader.
//!
//! A dongle relays its keyboard's host protocol, so the Rynk/Vial bootloader
//! commands never reach it; this interface is how `dfu-util` and rmk-gui ask it
//! to leave. Having no keys, it honours the DETACH only for 30 s after plug-in.

use embassy_time::{Duration, Instant};
use embassy_usb::class::dfu::app_mode::{self, DfuState};
use embassy_usb::class::dfu::consts::DfuAttributes;
use embassy_usb::driver::Driver;
use embassy_usb::{Builder, msos};
use static_cell::StaticCell;

/// Bus-powered, so boot time is plug-in time.
const PLUG_WINDOW: Duration = Duration::from_secs(30);

struct Detach;

impl app_mode::Handler for Detach {
    fn enter_dfu(&mut self) {
        if Instant::now() <= Instant::MIN + PLUG_WINDOW {
            crate::boot::jump_to_bootloader()
        } else {
            info!("DFU_DETACH ignored: plug-in window over");
        }
    }
}

pub(crate) fn register<D: Driver<'static>>(builder: &mut Builder<'static, D>) {
    // A Vial dongle has no vendor interface to write the MS OS 2.0 header first.
    if builder.msos_writer().is_empty() {
        builder.msos_descriptor(msos::windows_version::WIN8_1, super::MSOS_VENDOR_CODE);
    }
    static STATE: StaticCell<DfuState<Detach>> = StaticCell::new();
    // `WILL_DETACH`: reset on DETACH itself; WebUSB and nusb on Windows cannot bus-reset.
    let state = STATE.init(DfuState::new(
        Detach,
        DfuAttributes::CAN_DOWNLOAD | DfuAttributes::WILL_DETACH,
        Duration::from_millis(1000),
    ));
    // WinUSB, so a browser or libusb can send the DETACH on Windows.
    app_mode::usb_dfu(builder, state, |func| {
        func.msos_feature(msos::CompatibleIdFeatureDescriptor::new("WINUSB", ""));
    });
}
