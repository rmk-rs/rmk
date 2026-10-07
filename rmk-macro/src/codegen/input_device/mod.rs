use adc::expand_adc_device;
use battery::expand_battery_devices;
use encoder::expand_encoder_device;
use iqs5xx::expand_iqs5xx_device;
use pmw33xx::expand_pmw33xx_device;
use pmw3610::expand_pmw3610_device;
use proc_macro2::{Ident, TokenStream};
use quote::quote;
use rmk_config::PointingAccelerationConfig;
use rmk_config::resolved::Hardware;
use rmk_config::resolved::hardware::{BoardConfig, InputDeviceConfig, UniBodyConfig};

pub(crate) mod adc;
pub(crate) mod battery;
pub(crate) mod encoder;
pub(crate) mod iqs5xx;
pub(crate) mod pmw33xx;
pub(crate) mod pmw3610;

/// Initializer struct for input devices
pub(crate) struct Initializer {
    pub(crate) initializer: TokenStream,
    pub(crate) var_name: Ident,
}

/// Expands `proc_acceleration` into the `PointingProcessorConfig::acceleration` value.
pub(crate) fn expand_pointing_acceleration(
    acceleration: &Option<PointingAccelerationConfig>,
) -> TokenStream {
    match acceleration {
        Some(PointingAccelerationConfig { from, max }) => quote! {
            Some(::rmk::input_device::pointing::PointerAcceleration {
                from_counts_per_s: #from,
                max_percent: #max,
            })
        },
        None => quote! { None },
    }
}

/// Expands the input device configuration.
/// Returns a tuple containing: (device_and_processors_initialization, devices, processors)
pub(crate) fn expand_input_device_config(
    hardware: &Hardware,
) -> (TokenStream, Vec<TokenStream>, Vec<TokenStream>) {
    let mut initialization = TokenStream::new();
    let mut devices = Vec::new();
    let mut processors = Vec::new();

    let board = &hardware.board;
    let chip = &hardware.chip;
    let battery = &hardware.battery;
    let input_device = match board {
        BoardConfig::UniBody(board) => board.input_device.clone(),
        BoardConfig::Split(split) => split.central.input_device.clone().unwrap_or_default(),
    };
    let (adc_initializers, adc_processors) = expand_adc_device(
        input_device.joystick.unwrap_or_default(),
        battery.adc.as_ref(),
        chip.series.clone(),
    );
    let (battery_devices, battery_processors) = expand_battery_devices(chip, battery);

    for initializer in adc_initializers.into_iter().chain(battery_devices) {
        initialization.extend(initializer.initializer);
        let device_name = initializer.var_name;
        devices.push(quote! { #device_name });
    }

    for initializer in adc_processors.into_iter().chain(battery_processors) {
        initialization.extend(initializer.initializer);
        let processor_name = initializer.var_name;
        processors.push(quote! { #processor_name });
    }

    // generate encoder configuration
    let (device_initializer, processor_initializer) = match board {
        BoardConfig::UniBody(UniBodyConfig { input_device, .. }) => {
            expand_encoder_device(0, input_device.clone().encoder.unwrap_or(Vec::new()), chip)
        }
        BoardConfig::Split(split_config) => expand_encoder_device(
            0,
            split_config
                .central
                .input_device
                .clone()
                .unwrap_or(InputDeviceConfig::default())
                .encoder
                .unwrap_or(Vec::new()),
            chip,
        ),
    };
    for initializer in device_initializer {
        initialization.extend(initializer.initializer);
        let device_name = initializer.var_name;
        devices.push(quote! { #device_name });
    }

    for initializer in processor_initializer {
        initialization.extend(initializer.initializer);
        let processor_name = initializer.var_name;
        processors.push(quote! { #processor_name });
    }

    // generate PMW3610 configuration
    let (pmw3610_device_initializers, pmw3610_processor_initializers) = match board {
        BoardConfig::UniBody(UniBodyConfig { input_device, .. }) => {
            expand_pmw3610_device(input_device.clone().pmw3610.unwrap_or(Vec::new()), chip)
        }
        BoardConfig::Split(split_config) => expand_pmw3610_device(
            split_config
                .central
                .input_device
                .clone()
                .unwrap_or(InputDeviceConfig::default())
                .pmw3610
                .unwrap_or(Vec::new()),
            chip,
        ),
    };

    for initializer in pmw3610_device_initializers {
        initialization.extend(initializer.initializer);
        let device_name = initializer.var_name;
        devices.push(quote! { #device_name });
    }

    for initializer in pmw3610_processor_initializers {
        initialization.extend(initializer.initializer);
        let processor_name = initializer.var_name;
        processors.push(quote! { #processor_name });
    }

    // For split keyboards, also generate processors for PMW3610 devices on peripherals
    // The devices run on peripherals, but processors need to run on central to handle the events
    if let BoardConfig::Split(split_config) = board {
        for peripheral in &split_config.peripheral {
            let peripheral_pmw3610_config = peripheral
                .input_device
                .clone()
                .unwrap_or(InputDeviceConfig::default())
                .pmw3610
                .unwrap_or(Vec::new());

            // Only generate processors (not devices) for peripheral PMW3610
            let (_, peripheral_pmw3610_processors) =
                expand_pmw3610_device(peripheral_pmw3610_config, chip);

            for initializer in peripheral_pmw3610_processors {
                initialization.extend(initializer.initializer);
                let processor_name = initializer.var_name;
                processors.push(quote! { #processor_name });
            }
        }
    }

    // generate PMW33xx configuration
    let (pmw33xx_device_initializers, pmw33xx_processor_initializers) = match board {
        BoardConfig::UniBody(UniBodyConfig { input_device, .. }) => {
            expand_pmw33xx_device(input_device.clone().pmw33xx.unwrap_or(Vec::new()), chip)
        }
        BoardConfig::Split(split_config) => expand_pmw33xx_device(
            split_config
                .central
                .input_device
                .clone()
                .unwrap_or(InputDeviceConfig::default())
                .pmw33xx
                .unwrap_or(Vec::new()),
            chip,
        ),
    };

    for initializer in pmw33xx_device_initializers {
        initialization.extend(initializer.initializer);
        let device_name = initializer.var_name;
        devices.push(quote! { #device_name });
    }

    for initializer in pmw33xx_processor_initializers {
        initialization.extend(initializer.initializer);
        let processor_name = initializer.var_name;
        processors.push(quote! { #processor_name });
    }

    // For split keyboards, also generate processors for PMW33xx devices on peripherals
    // The devices run on peripherals, but processors need to run on central to handle the events
    if let BoardConfig::Split(split_config) = board {
        for peripheral in &split_config.peripheral {
            let peripheral_pmw33xx_config = peripheral
                .input_device
                .clone()
                .unwrap_or(InputDeviceConfig::default())
                .pmw33xx
                .unwrap_or(Vec::new());

            // Only generate processors (not devices) for peripheral PMW33xx
            let (_, peripheral_pmw33xx_processors) =
                expand_pmw33xx_device(peripheral_pmw33xx_config, chip);

            for initializer in peripheral_pmw33xx_processors {
                initialization.extend(initializer.initializer);
                let processor_name = initializer.var_name;
                processors.push(quote! { #processor_name });
            }
        }
    }

    // generate IQS5xx configuration
    let (iqs5xx_device_initializers, iqs5xx_processor_initializers) = match board {
        BoardConfig::UniBody(UniBodyConfig { input_device, .. }) => {
            expand_iqs5xx_device(input_device.clone().iqs5xx.unwrap_or(Vec::new()), chip)
        }
        BoardConfig::Split(split_config) => expand_iqs5xx_device(
            split_config
                .central
                .input_device
                .clone()
                .unwrap_or(InputDeviceConfig::default())
                .iqs5xx
                .unwrap_or(Vec::new()),
            chip,
        ),
    };

    for initializer in iqs5xx_device_initializers {
        initialization.extend(initializer.initializer);
        let device_name = initializer.var_name;
        devices.push(quote! { #device_name });
    }

    for initializer in iqs5xx_processor_initializers {
        initialization.extend(initializer.initializer);
        let processor_name = initializer.var_name;
        processors.push(quote! { #processor_name });
    }

    // For split keyboards, also generate processors for IQS5xx devices on peripherals
    // The devices run on peripherals, but processors need to run on central to handle the events
    if let BoardConfig::Split(split_config) = board {
        for peripheral in &split_config.peripheral {
            let peripheral_iqs5xx_config = peripheral
                .input_device
                .clone()
                .unwrap_or(InputDeviceConfig::default())
                .iqs5xx
                .unwrap_or(Vec::new());

            // Only generate processors (not devices) for peripheral IQS5xx
            let (_, peripheral_iqs5xx_processors) =
                expand_iqs5xx_device(peripheral_iqs5xx_config, chip);

            for initializer in peripheral_iqs5xx_processors {
                initialization.extend(initializer.initializer);
                let processor_name = initializer.var_name;
                processors.push(quote! { #processor_name });
            }
        }
    }

    (initialization, devices, processors)
}
