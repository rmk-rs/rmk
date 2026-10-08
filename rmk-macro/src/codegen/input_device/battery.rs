use quote::{format_ident, quote};
use rmk_config::resolved::hardware::{BatteryConfig, ChipModel};

use super::Initializer;
use crate::codegen::chip::gpio::{convert_gpio_str_to_input_pin, convert_gpio_str_to_output_pin};

/// Build the same battery pipeline on every board. ADC channels are initialized with other analog inputs.
pub(crate) fn expand_battery_devices(
    chip: &ChipModel,
    config: &BatteryConfig,
) -> (Vec<Initializer>, Vec<Initializer>) {
    let mut devices = Vec::new();
    let mut processors = Vec::new();

    if config.adc.is_some() || config.charge_state.is_some() {
        let constructor = if let Some(adc) = &config.adc {
            let measured = adc.divider_measured;
            let total = adc.divider_total;
            quote! { ::rmk::input_device::battery::BatteryProcessor::new(#measured, #total) }
        } else {
            quote! { ::rmk::input_device::battery::BatteryProcessor::charging_only() }
        };
        processors.push(Initializer {
            initializer: quote! { let mut battery_processor = #constructor; },
            var_name: format_ident!("battery_processor"),
        });
    }

    if let Some(pin) = &config.charge_state {
        let input =
            convert_gpio_str_to_input_pin(chip, pin.pin.clone(), false, Some(pin.low_active));
        let low_active = pin.low_active;
        devices.push(Initializer {
            initializer: quote! {
                let mut charging_state_reader = ::rmk::input_device::battery::ChargingStateReader::new(#input, #low_active);
            },
            var_name: format_ident!("charging_state_reader"),
        });
    }

    if let Some(pin) = &config.charge_led {
        let output = convert_gpio_str_to_output_pin(chip, pin.pin.clone(), pin.low_active);
        let low_active = pin.low_active;
        processors.push(Initializer {
            initializer: quote! {
                let mut charge_led_processor = ::rmk::processor::builtin::battery_led::BatteryLedProcessor::new(#output, #low_active);
            },
            var_name: format_ident!("charge_led_processor"),
        });
    }
    (devices, processors)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rmk_config::{KeyboardTomlConfig, PinConfig};

    use crate::codegen::input_device::expand_input_device_config;
    use crate::codegen::split::peripheral::expand_peripheral_input_device_config;

    #[test]
    fn every_board_assembles_one_processor_for_either_battery_source() {
        for (example, side, adc_pin, charge_pin, led_pin) in [
            ("nrf52840_ble", None, "P0_05", "P0_20", "P0_21"),
            ("esp32c3_ble", None, "GPIO0", "GPIO1", "GPIO2"),
            ("nrf52840_ble_split", None, "P0_05", "P0_20", "P0_21"),
            ("nrf52840_ble_split", Some(0), "P0_05", "P0_20", "P0_21"),
        ] {
            for (adc, charging) in [(false, false), (false, true), (true, false), (true, true)] {
                let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("../examples/use_config/{example}/keyboard.toml"));
                let mut hardware = KeyboardTomlConfig::new_from_toml_path(&path)
                    .hardware()
                    .unwrap();
                let pin = adc.then(|| adc_pin.to_string());
                let state = charging.then(|| PinConfig {
                    pin: charge_pin.into(),
                    low_active: true,
                });
                let led = (adc || charging).then(|| PinConfig {
                    pin: led_pin.into(),
                    low_active: false,
                });
                let battery = rmk_config::resolved::hardware::BatteryConfig {
                    adc: pin.map(|pin| rmk_config::resolved::hardware::BatteryAdcConfig {
                        pin,
                        divider_measured: 1,
                        divider_total: 2,
                    }),
                    charge_state: state,
                    charge_led: led,
                };
                match side {
                    Some(id) => hardware.peripheral_batteries[id] = battery,
                    None => hardware.battery = battery,
                }
                let (init, devices, processors) = match side {
                    None => expand_input_device_config(&hardware),
                    Some(id) => expand_peripheral_input_device_config(id, &hardware),
                };
                let device_names: Vec<_> = devices.iter().map(ToString::to_string).collect();
                let processor_names: Vec<_> = processors.iter().map(ToString::to_string).collect();
                assert_eq!(device_names.contains(&"adc_device".into()), adc);
                assert_eq!(
                    device_names.contains(&"charging_state_reader".into()),
                    charging
                );
                assert_eq!(
                    processor_names
                        .iter()
                        .filter(|name| *name == "battery_processor")
                        .count(),
                    usize::from(adc || charging)
                );
                assert_eq!(
                    processor_names.contains(&"charge_led_processor".into()),
                    adc || charging
                );
                assert_eq!(init.to_string().contains("charging_only"), charging && !adc);
                if charging {
                    assert!(init.to_string().contains("Pull :: Up"));
                }
                if adc || charging {
                    assert!(init.to_string().contains("Level :: Low"));
                }
            }
        }
    }
}
