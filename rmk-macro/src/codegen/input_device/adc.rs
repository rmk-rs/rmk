use quote::{format_ident, quote};
use rmk_config::resolved::hardware::{BleConfig, ChipSeries, JoystickConfig};

use super::Initializer;

/// Expand the ADC device configuration.
/// Returns (device initializers, processor initializers)
pub(crate) fn expand_adc_device(
    joystick_config: Vec<JoystickConfig>,
    ble_config: Option<BleConfig>,
    chip_model: ChipSeries,
) -> (Vec<Initializer>, Vec<Initializer>) {
    match chip_model {
        ChipSeries::Nrf52 => {
            let mut channel_cfg = vec![];
            let mut adc_type = vec![];
            let mut event_device_ids: Vec<u8> = vec![];
            let mut power_pins = vec![];
            let mut axis_bias = vec![];
            let mut deadzones = vec![];
            let power_config = joystick_config.first().cloned();
            let mut default_polling_interval = 30000u16; // default 30s
            let mut light_sleep: Option<u16> = None;
            // TODO: deep sleep

            let mut devices = vec![];
            let mut processors = vec![];

            if let Some(ble) = ble_config
                && ble.enabled
                && let Some(adc_pin) = ble.battery_adc_pin
            {
                let adc_pin_def = if adc_pin == "vddh" {
                    quote! {
                        saadc::ChannelConfig::single_ended(saadc::VddhDiv5Input.degrade_saadc())
                    }
                } else {
                    let adc_pin_def = format_ident!("{}", adc_pin);
                    quote! {
                        saadc::ChannelConfig::single_ended(p.#adc_pin_def.degrade_saadc())
                    }
                };
                channel_cfg.push(adc_pin_def);
                adc_type.push(quote! {
                    ::rmk::input_device::adc::AnalogEventType::Battery
                });
                // Battery event slot: device_id unused, fill with 0
                event_device_ids.push(0u8);
                power_pins.push(quote! { None });
                axis_bias.push(quote! { [0i16; 3] });
                deadzones.push(0u16);

                let (adc_divider_measured, adc_divider_total) = if adc_pin == "vddh" {
                    (1, 5)
                } else {
                    (
                        ble.adc_divider_measured.unwrap_or(1),
                        ble.adc_divider_total.unwrap_or(1),
                    )
                };
                let bat_ident = format_ident!("battery_processor");
                let battery_processor = Initializer {
                    initializer: quote! {
                        let mut #bat_ident = ::rmk::input_device::battery::BatteryProcessor::new(#adc_divider_measured, #adc_divider_total);
                    },
                    var_name: bat_ident,
                };
                processors.push(battery_processor);
            }

            // polling interval with joystick
            if !joystick_config.is_empty() {
                default_polling_interval = 20;
                light_sleep = Some(350);
            }

            for (joy_idx, joystick) in joystick_config.into_iter().enumerate() {
                joystick
                    .validate_power_config()
                    .unwrap_or_else(|e| panic!("{e}"));
                if let Some(first) = &power_config {
                    assert!(
                        (
                            first.polling_rate_hz,
                            first.idle_polling_rate_hz,
                            first.sample_settle_us,
                            first.boot_settle_ms
                        ) == (
                            joystick.polling_rate_hz,
                            joystick.idle_polling_rate_hz,
                            joystick.sample_settle_us,
                            joystick.boot_settle_ms
                        ),
                        "joysticks sharing one SAADC must use the same polling/settling parameters"
                    );
                }
                power_pins.push(match &joystick.power_pin {
                    Some(pin) => {
                        let pin = format_ident!("{}", pin);
                        quote! { Some(embassy_nrf::gpio::Output::new(
                        p.#pin, embassy_nrf::gpio::Level::Low,
                        embassy_nrf::gpio::OutputDrive::Standard)) }
                    }
                    None => quote! { None },
                });
                let mut bias = [0i16; 3];
                for (dst, src) in bias.iter_mut().zip(&joystick.bias) {
                    *dst = *src;
                }
                axis_bias.push(quote! { [#(#bias),*] });
                deadzones.push(joystick.deadzone);
                // Assign device id: use configured id or fall back to sequential index
                let device_id: u8 = joystick.id.unwrap_or(joy_idx as u8);
                event_device_ids.push(device_id);
                let mut cnt = 0u8;
                for pin in [joystick.pin_x, joystick.pin_y, joystick.pin_z].iter() {
                    if pin == "_" {
                        break;
                    }
                    let adc_pin_def = format_ident!("{}", pin);
                    channel_cfg.push(quote! {
                        saadc::ChannelConfig::single_ended(p.#adc_pin_def.degrade_saadc())
                    });
                    cnt += 1;
                }
                assert!(
                    (2..=3).contains(&cnt),
                    "joystick must have X/Y, and optionally Z"
                );
                assert!(
                    joystick.bias.len() == cnt as usize
                        && joystick.transform.len() == cnt as usize
                        && joystick
                            .transform
                            .iter()
                            .all(|row| row.len() == cnt as usize),
                    "joystick bias/transform dimensions must match its ADC axes"
                );

                adc_type.push(quote! {
                    ::rmk::input_device::adc::AnalogEventType::Joystick(#cnt)
                });
                let joy_ident = format_ident!("joystick_processor_{}", joystick.name);
                let JoystickConfig {
                    transform,
                    bias,
                    resolution,
                    deadzone,
                    ..
                } = joystick;
                let joystick_processor = Initializer {
                    initializer: quote! {
                        let mut #joy_ident = rmk::input_device::joystick::JoystickProcessor::new(#device_id, [#([#(#transform),*]),*], [#(#bias),*], #resolution, &keymap).with_deadzone(#deadzone);
                    },
                    var_name: joy_ident,
                };
                processors.push(joystick_processor);
            }

            if !processors.is_empty() {
                let power_builder = power_config.map(|c| {
                    let JoystickConfig {
                        polling_rate_hz,
                        idle_polling_rate_hz,
                        sample_settle_us,
                        boot_settle_ms,
                        ..
                    } = c;
                    quote! {
                        .with_power_management(
                            rmk::input_device::joystick::JoystickPowerConfig {
                                polling_rate_hz: #polling_rate_hz,
                                idle_polling_rate_hz: #idle_polling_rate_hz,
                                sample_settle_us: #sample_settle_us, boot_settle_ms: #boot_settle_ms,
                            }, [#(#power_pins),*], [#(#axis_bias),*], [#(#deadzones),*])
                    }
                });
                let light_sleep_option = if let Some(light_sleep_interval) = light_sleep {
                    quote! {Some(Duration::from_millis(#light_sleep_interval as u64))}
                } else {
                    quote! {None}
                };
                let adc_device = Initializer {
                    initializer: quote! {
                        let mut adc_device = {
                        use embassy_time::Duration;
                        use embassy_nrf::saadc::{self, Input as _};
                        ::embassy_nrf::bind_interrupts!(struct SaadcIrqs {
                            SAADC => ::embassy_nrf::saadc::InterruptHandler;
                        });
                        let saadc_config = saadc::Config::default();
                        embassy_nrf::interrupt::SAADC.set_priority(embassy_nrf::interrupt::Priority::P3);

                        let adc = saadc::Saadc::new(p.SAADC, SaadcIrqs, saadc_config, [#(#channel_cfg),*]);
                        adc.calibrate().await;

                        rmk::input_device::adc::NrfAdc::new(
                                adc,
                                [#(#adc_type),*],
                                [#(#event_device_ids),*],
                                Duration::from_millis(#default_polling_interval as u64),
                                #light_sleep_option,
                            )
                            #power_builder
                        };
                    },
                    var_name: format_ident!("adc_device"),
                };
                devices.push(adc_device);
                (devices, processors)
            } else {
                (Vec::new(), Vec::new())
            }
        }
        _ => (Vec::new(), Vec::new()),
    }
}
