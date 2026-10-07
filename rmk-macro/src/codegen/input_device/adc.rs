use quote::{format_ident, quote};
use rmk_config::resolved::hardware::{BatteryAdcConfig, ChipSeries, JoystickConfig};

use super::Initializer;

/// Expand the ADC device configuration.
/// Returns (device initializers, processor initializers)
pub(crate) fn expand_adc_device(
    joystick_config: Vec<JoystickConfig>,
    battery_adc: Option<&BatteryAdcConfig>,
    chip_model: ChipSeries,
) -> (Vec<Initializer>, Vec<Initializer>) {
    match chip_model {
        ChipSeries::Nrf52 => {
            let mut channel_cfg = vec![];
            let mut adc_type = vec![];
            let mut event_device_ids: Vec<u8> = vec![];
            let mut default_polling_interval = 30000u16; // default 30s
            let mut light_sleep: Option<u16> = None;
            // TODO: deep sleep

            let mut devices = vec![];
            let mut processors = vec![];

            if let Some(adc) = battery_adc {
                let adc_pin = &adc.pin;
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
            }

            // polling interval with joystick
            if !joystick_config.is_empty() {
                default_polling_interval = 20;
                light_sleep = Some(350);
            }

            for (joy_idx, joystick) in joystick_config.into_iter().enumerate() {
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

                adc_type.push(quote! {
                    ::rmk::input_device::adc::AnalogEventType::Joystick(#cnt)
                });
                let joy_ident = format_ident!("joystick_processor_{}", joystick.name);
                let JoystickConfig {
                    transform,
                    bias,
                    resolution,
                    ..
                } = joystick;
                let joystick_processor = Initializer {
                    initializer: quote! {
                        let mut #joy_ident = rmk::input_device::joystick::JoystickProcessor::new(#device_id, [#([#(#transform),*]),*], [#(#bias),*], #resolution, &keymap);
                    },
                    var_name: joy_ident,
                };
                processors.push(joystick_processor);
            }

            if !channel_cfg.is_empty() {
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
        ChipSeries::Rp2040 => {
            let Some(adc) = battery_adc else {
                return (Vec::new(), Vec::new());
            };
            let pin = format_ident!("{}", adc.pin);
            (
                vec![Initializer {
                    initializer: quote! {
                        let mut adc_device = {
                            use ::embassy_rp::adc::{Adc, Channel, Config, InterruptHandler};
                            ::embassy_rp::bind_interrupts!(struct BatteryAdcIrqs {
                                ADC_IRQ_FIFO => InterruptHandler;
                            });
                            let adc = Adc::new(p.ADC, BatteryAdcIrqs, Config::default());
                            let pin = Channel::new_pin(p.#pin, ::embassy_rp::gpio::Pull::None);
                            ::rmk::input_device::adc::rp2040::Rp2040BatteryAdc::new(
                                adc, pin, ::rmk::embassy_time::Duration::from_secs(30),
                            )
                        };
                    },
                    var_name: format_ident!("adc_device"),
                }],
                Vec::new(),
            )
        }
        _ => {
            assert!(
                battery_adc.is_none(),
                "Automatic battery ADC setup is not available for this chip"
            );
            (Vec::new(), Vec::new())
        }
    }
}
