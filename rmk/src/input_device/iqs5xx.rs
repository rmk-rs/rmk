//! Azoteq IQS5xx trackpad controller driver.
//!
//! This IC is commonly used in keyboards via Azoteq's TPS43 and TPS65 modules.
//!
//! # Communication
//!
//! ```text
//!
//! RDY    | low     | high    | low     | high    |
//! mode   | scan    | I2C     | scan    | I2C     |
//!                  |<-- report rate -->|
//! ```
//!
//! The device operates in cycles. It drives the `RDY` pin high to indicate
//! (§8.1) that scanning/processing is complete and an I2C communication window
//! is open. The window lasts until RMK sends an "end session" command (§8.7)
//! or the I2C timeout is hit (§8.6). The device then restarts scanning/processing
//! to prepare for the next window. It attempts to open a window once every
//! "report rate" (more accurately, report *interval*, as it's measured in
//! milliseconds; §4.1), setting the `RR_MISSED` bit if it's not able to keep up.
//!
//! If RMK requests "event mode", the I2C communication window is skipped
//! entirely when there's no event of interest.
//!
//! RMK can force a communication window to open; the device "clock stretches"
//! until it is able to respond.
//!
//! Report intervals vary based on configuration and mode (active and four
//! successively deeper idle states).
//!
//! In the best case, IQS550 reports are at a rate of ~100 Hz / interval of ~10
//! ms. Notably, this is long enough that (unlike with trackball controllers
//! supported by RMK) it's not necessary to aggregate several reports into one
//! USB HID mouse report. In fact the opposite may be preferred: spreading one
//! report across several USB HID reports for smooth scrolling.
//!
//! # `RDY` vs polling
//!
//! This driver supports usage with or without a `RDY` (ready/motion) pin.
//! Routing such a pin (even by hand-soldering a wire onto an existing PCB) is
//! recommended, particularly if the I2C bus is shared with another device. As
//! mentioned above, sending an I2C command outside a communication window will
//! cause it to respond by I2C clock-stretching, essentially freezing the bus
//! for all devices until it is ready. If RMK communicates promptly after `RDY`
//! goes high, this is unlikely to happen. When RMK just guesses when to
//! communicate based on timing, it's far more likely. It's possible to minimize
//! chances of a stall by making the report interval relatively consistent, but
//! this requires compromises:
//!
//! 1. requesting a conservative report rate: experimentally, even though the
//!    IQS550 advertises "typical report rate: 100 Hz (with single touch /
//!    all channels active)", it will miss targets shorter than ~14 ms (~70 Hz)
//!    during longer touch events.
//! 2. perhaps giving extra time beyond the requested cycle time before polling,
//!    lowering the duty cycle, and raising the communication window timeout to
//!    compensate.
//! 3. preventing transition to idle modes or setting their cycle times to match
//!    the active mode.
//! 4. disabling event mode.
//! 5. disabling auto-tuning during operation (re-ATI).
//!
//! # Reported data
//!
//! The IC exposes (§5.2, §6):
//!
//! * single-finger relative cursor movement (§5.2.2)
//! * per-finger absolute position, pressure, and area (§5.2.3-§5.2.5)
//! * detected gestures (§6): single/two-finger tap, press-and-hold, swipes,
//!   scroll, zoom/pinch
//! * raw per-channel count/delta data (§8.10.6)
//!
//! This driver currently requests only the 10-byte motion block at 0x000C
//! (previous cycle time, gesture events, system info, number of fingers,
//! relative XY) and publishes relative XY as cursor movement. Gestures are
//! left disabled on the IC; absolute finger data and raw channel data are
//! not read.
//!
//! # Configuration
//!
//! The Azoteq driver supports a variety of configuration, including parameters of
//! the physical touchpad and configuration of gesture recognition.
//!
//! * Some of these can be configured persistently shortly after resetting the
//!   device via the `NRST` pin. This driver does not support that mechanism.
//! * All can be configured at runtime. Currently this driver hardcodes a few of these.
//!
//! # References
//!
//! * [datasheet](https://www.azoteq.com/images/stories/pdf/iqs5xx-b000_trackpad_datasheet.pdf).
//!   Section markers (§) in comments refer to the datasheet unless otherwise noted.

use embassy_time::{Duration, Instant, Timer};
use embedded_hal::i2c::Operation;
use embedded_hal_async::digital::Wait;
use embedded_hal_async::i2c::I2c;
use rmk_macro::input_device;

use crate::event::{AxisEvent, PointingEvent};
use crate::fmt::Debug;

const I2C_ADDR: u8 = 0x74; // default I2C bus address according to §8.2.

const END_SESSION: [u8; 2] = [0xEE, 0xEE]; // §8.7. Address + dummy data byte; a zero-data write doesn't actually trigger end-of-comms.

#[input_device(publish = PointingEvent)]
pub struct Iqs5xx<I, RDY>
where
    I: I2c,
    I::Error: Debug,
    RDY: Wait,
{
    /// The RMK pointing device id of this device (*not* the I2C bus address).
    pointing_device_id: u8,

    i2c: I,

    window_detection: WindowDetection<RDY>,

    initialized: bool,
}

/// Manner of detecting a "communication window" between cycles.
pub enum WindowDetection<RDY> {
    /// Wait for a high state of the given GPIO connected to the `RDY` pin.
    Rdy(RDY),

    /// Open a communication window every `interval` (the device clock-stretches
    /// if we arrive mid-cycle).
    Poll { last_end: Instant, interval_ms: u16 },
}

#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Debug)]
enum Error<I2cError> {
    I2c { tag: &'static str, inner: I2cError },
    InvalidProductInfo([u8; 4]),
    Reset,
}

/// Start communication with the device without waiting for an I2C window.
///
/// As described in §8.8.2, if the device is in lower-power mode, it may
/// initially NAK. The operations should be idempotent for retry.
///
/// Additionally, the device will likely stall the bus via clock stretching.
async fn i2c_force_tx<'a, I: I2c>(
    i2c: &mut I,
    tag: &'static str,
    operations: &mut [Operation<'a>],
) -> Result<(), Error<I::Error>> {
    // There should be at most 1 NAK, but let's give an extra try.
    const MAX_ATTEMPTS: usize = 3;
    let mut attempt = 0;
    while let Err(e) = i2c.transaction(I2C_ADDR, operations).await {
        attempt += 1;
        if attempt == MAX_ATTEMPTS {
            return Err(Error::I2c { tag, inner: e });
        }

        // Datasheet requires 150 µs delay; give a little slack.
        Timer::after(Duration::from_micros(200)).await;
    }
    Ok(())
}

/// Perform operations that are expected to fall within a communication window and thus not require retry.
async fn i2c_tx<'a, I: I2c>(
    i2c: &mut I,
    tag: &'static str,
    operations: &mut [Operation<'a>],
) -> Result<(), Error<I::Error>> {
    i2c.transaction(I2C_ADDR, operations)
        .await
        .map_err(|inner| Error::I2c { tag, inner })
}

impl<I: I2c, RDY> Iqs5xx<I, RDY>
where
    I: I2c,
    I::Error: Debug,
    RDY: Wait,
{
    pub fn new(rmk_id: u8, i2c: I, rdy: Option<RDY>) -> Self {
        Self {
            i2c,
            window_detection: match rdy {
                None => WindowDetection::Poll {
                    last_end: Instant::now(),
                    interval_ms: 15, // conservative value
                },
                Some(rdy) => WindowDetection::Rdy(rdy),
            },
            initialized: false,
            pointing_device_id: rmk_id,
        }
    }

    /// Initialize the device.
    async fn init(&mut self) -> Result<(), Error<I::Error>> {
        // Force-open a communication window; the device may never become ready otherwise.
        let mut product_info = [0u8; 4]; // §7.9.1.
        i2c_force_tx(
            &mut self.i2c,
            "read_product_info",
            &mut [Operation::Write(&[0, 0]), Operation::Read(&mut product_info)],
        )
        .await?;
        let ic = match product_info {
            [0, 40, 0, 15] => "IQS550",
            [0, 58, 0, 15] => "IQS572",
            [0, 52, 0, 15] => "IQS525",
            _ => {
                return Err(Error::InvalidProductInfo(product_info));
            }
        };

        let mut channels = [0u8; 2]; // §5.1.1: 0x063D Total Rx, 0x063E Total Tx
        i2c_tx(
            &mut self.i2c,
            "read_channels",
            &mut [Operation::Write(&[0x06, 0x3D]), Operation::Read(&mut channels)],
        )
        .await?;
        // §8.10.20: with SWITCH_XY_AXIS=0 (as set in `xy_config` below), Rx drives
        // output X and Tx drives output Y. §5.1.1: max useful resolution per axis
        // is (channels - 1) * 256.
        let x_resolution = u16::from(channels[0].saturating_sub(1)) * 256;
        let y_resolution = u16::from(channels[1].saturating_sub(1)) * 256;

        
        
        
        
        let (system_config_0, system_config_1, i2c_timeout_ms, active_interval_ms) = match &mut self.window_detection {
            WindowDetection::Rdy(_) => {
                (0b01100100, 0b00001111, 30, 9)
            }
            WindowDetection::Poll { interval_ms, .. } => {
                (0b11100100, 0, 100, *interval_ms)
            }
        };
        // I2C timeout register at 0x058A; §8.6.
        let i2c_timeout = [0x05, 0x8A, i2c_timeout_ms];
        #[rustfmt::skip]
        let config = [
            0x05, 0x8E, // System Config 0 at 0x058E; §8.10.9
            system_config_0,
            system_config_1, // System Config 1 at 0x058F; §8.10.10
        ];
        // Report rate registers at 0x057A..0x0583; §4.1. Active and idle-touch
        // get the same value so a stationary finger keeps the cadence we asked
        // for (the device drops to idle-touch on a held finger; in poll mode a
        // mismatched idle-touch rate would misalign our polling clock). Idle
        // mode cycle time is pinned at 25 ms so the device doesn't stretch out
        // cycles between touches. LP1/LP2 are left at NV defaults.
        #[rustfmt::skip]
        let report_rates = [
            0x05, 0x7A, // address: 0x057A
            (active_interval_ms >> 8) as u8, active_interval_ms as u8, // active mode
            (active_interval_ms >> 8) as u8, active_interval_ms as u8, // idle-touch mode
            0, 25, // idle mode (ms)
        ];
        const ACK_RESET: [u8; 3] = [
            0x04,
            0x31,        // System Control 0 at 0x0431; §8.10.7
            0b1000_0000, // ACK_RESET bit
        ];
        #[rustfmt::skip]
        const XY_CONFIG: [u8; 3] = [
            0x06, 0x69, // XY Config 0 at 0x0669; §8.10.20
            0b0001,     // FLIP_X | !SWITCH_XY_AXIS (Rx→X, Tx→Y; see resolution above)
        ];
        #[rustfmt::skip]
        let gestures = [
            0x06, 0xB7, // Single-/Multi-finger Gestures at 0x06B7/0x06B8; §8.10.21-§8.10.22
            0,          // single-finger: all disabled
            0,          // multi-finger:  all disabled
        ];

        // X/Y Resolution at 0x066E..0x0671 (2 bytes each); §5.4.
        // We set the maximum possible resolution to get the best precision out of the device.
        // This is beneficial for things like being able to scroll slowly and smoothly.
        #[rustfmt::skip]
        let xy_resolution = [
            0x06, 0x6E,
            (x_resolution >> 8) as u8, x_resolution as u8,
            (y_resolution >> 8) as u8, y_resolution as u8,
        ];

        // Each block must be its own transaction. Per the `embedded_hal::i2c`
        // contract, adjacent `Operation::Write`s in one `transaction` emit no
        // STOP or restart between them — their bytes go on the wire as one
        // contiguous I2C write. Since each of our blocks starts with a 2-byte
        // register address, bundling them would only place the *first* block
        // correctly; every later block's address bytes would land as data in
        // whichever register the IQS5xx's auto-incrementing write pointer has
        // reached by then, and the intended target registers wouldn't be
        // touched at all.
        for (tag, write) in [
            ("i2c_timeout", &i2c_timeout[..]),
            ("config", &config[..]),
            ("report_rates", &report_rates[..]),
            ("ack_reset", &ACK_RESET[..]),
            ("xy_config", &XY_CONFIG[..]),
            ("gestures", &gestures[..]),
            ("xy_resolution", &xy_resolution[..]),
        ] {
            i2c_tx(&mut self.i2c, tag, &mut [Operation::Write(write)]).await?;
        }

        i2c_tx(&mut self.i2c, "end_session", &mut [Operation::Write(&END_SESSION[..])]).await?;

        if let WindowDetection::Poll { ref mut last_end, .. } = self.window_detection {
            *last_end = Instant::now();
        }
        self.initialized = true;
        info!(
            "iqs5xx {}: initialized {} (rx={}, tx={} => x_res={}, y_res={})",
            self.pointing_device_id, ic, channels[0], channels[1], x_resolution, y_resolution,
        );
        Ok(())
    }

    async fn read_motion(&mut self) -> Result<PointingEvent, Error<I::Error>> {
        // Motion block at 0x000C..0x0015 per table 8.1: previous cycle time
        // (§4.1.1), gesture events 0/1 (§8.10.1-§8.10.2), system info 0/1
        // (§8.10.3-§8.10.4), number of fingers (§5.2.1), relative XY (§5.2.2).
        let mut data = [0u8; 10];
        let mut operations = [
            // In theory, it's possible to skip the initial address selection
            // write if the last window closed cleanly and RMK has previously
            // set the "default read address" register. However, it's unclear
            // if this register's contents are preserved across resets, so
            // it's probably unwise to use this in combination with the watchdog.
            // Let's be conservative and select the address each cycle.
            Operation::Write(&[0x00, 0x0C]),
            Operation::Read(&mut data),
            Operation::Write(&END_SESSION[..]),
        ];
        match self.window_detection {
            WindowDetection::Rdy(ref mut rdy) => {
                rdy.wait_for_high().await.expect("pin wait failure");
                i2c_tx(&mut self.i2c, "read_motion", &mut operations).await?;
            }
            WindowDetection::Poll {
                ref mut last_end,
                interval_ms,
            } => {
                Timer::at(last_end.saturating_add(Duration::from_millis(u64::from(interval_ms)))).await;
                i2c_force_tx(&mut self.i2c, "read_motion", &mut operations).await?;
                *last_end = Instant::now();
            }
        }
        let prev_cycle_time_ms = data[0];
        let gesture_events_0 = data[1];
        let gesture_events_1 = data[2];
        let system_info_0 = data[3]; // §8.10.3
        let system_info_1 = data[4]; // §8.10.4
        let number_of_fingers = data[5];
        let dx = i16::from_be_bytes(unwrap!(data[6..8].try_into()));
        let dy = i16::from_be_bytes(unwrap!(data[8..10].try_into()));

        // §8.10.3: system_info_0.
        let charging_mode = match system_info_0 & 0b111 {
            0b000 => "active",
            0b001 => "idle-touch",
            0b010 => "idle",
            0b011 => "lp1",
            0b100 => "lp2",
            _ => "invalid",
        };
        if (system_info_0 & 0b0001_0000) != 0 {
            debug!("iqs5xx {} re-ati", self.pointing_device_id);
        }
        if (system_info_0 & 0b0000_1000) != 0 {
            error!("iqs5xx {} ati error", self.pointing_device_id);
        }
        if (system_info_0 & 0b1000_0000) != 0 {
            self.initialized = false;
            return Err(Error::Reset);
        }
        debug!(
            "iqs5xx {} motion data: cycle_ms={} gestures=[{},{}] mode={} system=[{},{}] n_fingers={} dx={} dy={}",
            self.pointing_device_id,
            prev_cycle_time_ms,
            gesture_events_0,
            gesture_events_1,
            charging_mode,
            system_info_0,
            system_info_1,
            number_of_fingers,
            dx,
            dy,
        );
        Ok(PointingEvent {
            device_id: self.pointing_device_id,
            axes: [
                AxisEvent {
                    typ: crate::event::AxisValType::Rel,
                    axis: crate::event::Axis::X,
                    value: dx,
                },
                AxisEvent {
                    typ: crate::event::AxisValType::Rel,
                    axis: crate::event::Axis::Y,
                    value: dy,
                },
                AxisEvent {
                    typ: crate::event::AxisValType::Rel,
                    axis: crate::event::Axis::Z,
                    value: 0,
                },
            ],
        })
    }

    async fn read_pointing_event(&mut self) -> PointingEvent {
        loop {
            // Check initialization status on each iteration because the device
            // can reset and require re-initialization.
            if !self.initialized
                && let Err(e) = self.init().await
            {
                error!(
                    "iqs5xx {} initialization failed: {:?}; will retry in 1 second",
                    self.pointing_device_id, e,
                );
                Timer::after_secs(1).await;
                continue;
            }
            match self.read_motion().await {
                Ok(e) => {
                    if e.axes.iter().any(|axis| axis.value != 0) {
                        return e;
                    }
                }
                Err(e) => {
                    error!("iqs5xx {} failure: {:?}", self.pointing_device_id, e);
                    Timer::after_millis(5).await;
                }
            }
        }
    }
}
