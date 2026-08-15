use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use rmk_config::SplitConnection;
use rmk_config::resolved::Hardware;
use rmk_config::resolved::hardware::{
    BleConfig, BoardConfig, ChipModel, ChipSeries, CommunicationConfig, InputDeviceConfig,
    MatrixType, SplitBoardConfig, SplitConfig,
};
use syn::ItemMod;

use super::central::expand_serial_init;
use crate::codegen::chip::chip_init::expand_chip_init;
use crate::codegen::chip::comm::expand_usb_init;
use crate::codegen::chip::flash::expand_flash_init;
use crate::codegen::chip::gpio::expand_output_initialization;
use crate::codegen::display::{expand_display_config, expand_display_interrupt};
use crate::codegen::entry::join_all_tasks;
use crate::codegen::feature::{get_rmk_features, is_feature_enabled};
use crate::codegen::import::expand_custom_imports;
use crate::codegen::input_device::adc::expand_adc_device;
use crate::codegen::input_device::encoder::expand_encoder_device;
use crate::codegen::input_device::iqs5xx::{expand_iqs5xx_device, expand_iqs5xx_interrupts};
use crate::codegen::input_device::pmw33xx::expand_pmw33xx_device;
use crate::codegen::input_device::pmw3610::expand_pmw3610_device;
use crate::codegen::keyboard_config::read_keyboard_toml_config;
use crate::codegen::matrix::{
    expand_bootmagic_check, expand_matrix_direct_pins, expand_matrix_input_output_pins,
};
use crate::codegen::orchestrator::get_debouncer_type;
use crate::codegen::registered_processor::expand_registered_processor_init;
use crate::codegen::watchdog::expand_watchdog_init;
use rmk_config::resolved::Identity;

/// Parse split peripheral mod and generate a valid RMK main function with all needed code
pub(crate) fn parse_split_peripheral_mod(
    id: usize,
    _attr: proc_macro::TokenStream,
    item_mod: ItemMod,
) -> TokenStream2 {
    // Reject invalid `#[Overwritten(...)]` attributes up front instead of
    // silently falling back to the generated defaults (#966)
    if let Some(errors) = crate::codegen::override_helper::validate_overwritten_attrs(&item_mod) {
        return errors;
    }

    let rmk_features = get_rmk_features();
    if !is_feature_enabled(&rmk_features, "split") {
        panic!("\"split\" feature of RMK should be enabled");
    }

    let toml_config = read_keyboard_toml_config();
    let hardware = toml_config
        .hardware()
        .expect("failed to resolve hardware config");
    let identity = toml_config
        .identity()
        .expect("failed to resolve identity config");

    let dfu_enabled =
        is_feature_enabled(&rmk_features, "dfu_rp") || is_feature_enabled(&rmk_features, "dfu_nrf");
    let usb_log_enabled = is_feature_enabled(&rmk_features, "usb_log");
    let device_config = if dfu_enabled || usb_log_enabled {
        let vid = identity.vendor_id;
        let pid = identity.product_id;
        let manufacturer = &identity.manufacturer;
        let product_name = &identity.product_name;
        let serial_number_tokens = match &identity.serial_number {
            Some(s) => quote! { #s },
            None => quote! { ::rmk::config::RMK_BUILD_INFO },
        };
        quote! {
            const KEYBOARD_DEVICE_CONFIG: ::rmk::config::DeviceConfig = ::rmk::config::DeviceConfig {
                vid: #vid,
                pid: #pid,
                manufacturer: #manufacturer,
                product_name: #product_name,
                serial_number: #serial_number_tokens,
            };
        }
    } else {
        quote! {}
    };

    let main_function = expand_split_peripheral(id, &identity, &hardware, item_mod, &rmk_features);

    let bind_interrupts =
        expand_bind_interrupt_for_split_peripheral(&hardware.chip, &hardware, id, &rmk_features);

    let chip = &hardware.chip;
    let main_function_sig = if chip.series == ChipSeries::Esp32 {
        quote! {
            use esp_alloc as _;
            use esp_backtrace as _;
            ::esp_bootloader_esp_idf::esp_app_desc!();
            #[esp_rtos::main]
            async fn main(_s: ::embassy_executor::Spawner)
        }
    } else {
        quote! {
            use defmt_rtt as _;
            use panic_probe as _;

            #bind_interrupts
            #[::embassy_executor::main]
            async fn main(spawner: ::embassy_executor::Spawner)
        }
    };

    quote! {
        #device_config
        #main_function_sig {
            // ::defmt::info!("RMK start!");
            #main_function
        }
    }
}

fn expand_bind_interrupt_for_split_peripheral(
    chip: &ChipModel,
    hardware: &Hardware,
    peripheral_id: usize,
    rmk_features: &Option<Vec<String>>,
) -> TokenStream2 {
    let communication = &hardware.communication;

    let display_interrupt = match &hardware.board {
        BoardConfig::Split(split_config) => {
            if let Some(display_config) = &split_config.peripheral[peripheral_id].display {
                expand_display_interrupt(&chip.series, display_config)
            } else {
                quote! {}
            }
        }
        _ => quote! {},
    };

    let iqs5xx_config_for_irq = match &hardware.board {
        BoardConfig::Split(split_config) => split_config.peripheral[peripheral_id]
            .input_device
            .clone()
            .unwrap_or(InputDeviceConfig::default())
            .iqs5xx
            .unwrap_or(Vec::new()),
        _ => Vec::new(),
    };
    let iqs5xx_interrupt = expand_iqs5xx_interrupts(&chip.series, &iqs5xx_config_for_irq);

    let dfu_enabled =
        is_feature_enabled(rmk_features, "dfu_rp") || is_feature_enabled(rmk_features, "dfu_nrf");
    let usb_log_enabled = is_feature_enabled(rmk_features, "usb_log");
    let usb_enabled = dfu_enabled || usb_log_enabled;

    match chip.series {
        ChipSeries::Nrf52 => {
            let usb_interrupt = if usb_enabled {
                quote! {
                    USBD => ::embassy_nrf::usb::InterruptHandler<::embassy_nrf::peripherals::USBD>;
                }
            } else {
                quote! {}
            };

            let ble_config = communication.get_ble_config().unwrap();
            let tx_power = if let Some(pwr) = ble_config.default_tx_power {
                quote! { .default_tx_power(#pwr)?  }
            } else {
                quote! {}
            };
            let use_2m_phy = if ble_config.use_2m_phy.unwrap_or(true) {
                quote! { .support_le_2m_phy() }
            } else {
                quote! {}
            };

            // Extract PMW33xx configuration
            let split_config = match &hardware.board {
                BoardConfig::Split(split_config) => split_config,
                _ => panic!("Expected split configuration"),
            };

            let pmw33xx_config = split_config.peripheral[peripheral_id]
                .input_device
                .clone()
                .unwrap_or(InputDeviceConfig::default())
                .pmw33xx
                .unwrap_or(Vec::new());

            // Generate SPI interrupts for each sensor
            let mut pmw33xx_spi_interrupts = Vec::new();

            for sensor in &pmw33xx_config {
                let instance_ident = format_ident!("{}", &sensor.spi.instance);

                pmw33xx_spi_interrupts.push(quote! {
                    #instance_ident => spim::InterruptHandler<peripherals::#instance_ident>;
                });
            }

            let pmw33xx_spi_interrupts = if pmw33xx_spi_interrupts.is_empty() {
                quote! {}
            } else {
                quote! {
                    #(#pmw33xx_spi_interrupts)*
                }
            };
            let spim_import = if !pmw33xx_config.is_empty() {
                quote! {
                    use ::embassy_nrf::spim;
                    use embassy_nrf::peripherals;
                }
            } else {
                quote! {}
            };

            let clock_power_handler = if usb_enabled {
                quote! {
                    CLOCK_POWER => ::nrf_sdc::mpsl::ClockInterruptHandler, ::embassy_nrf::usb::vbus_detect::InterruptHandler;
                }
            } else {
                quote! {
                    CLOCK_POWER => ::nrf_sdc::mpsl::ClockInterruptHandler;
                }
            };

            quote! {
                use ::embassy_nrf::bind_interrupts;
                #spim_import
                bind_interrupts!(struct Irqs {
                    #clock_power_handler
                    #usb_interrupt
                    RNG => ::embassy_nrf::rng::InterruptHandler<::embassy_nrf::peripherals::RNG>;
                    EGU0_SWI0 => ::nrf_sdc::mpsl::LowPrioInterruptHandler;
                    RADIO => ::nrf_sdc::mpsl::HighPrioInterruptHandler;
                    TIMER0 => ::nrf_sdc::mpsl::HighPrioInterruptHandler;
                    RTC0 => ::nrf_sdc::mpsl::HighPrioInterruptHandler;
                    #pmw33xx_spi_interrupts
                    #iqs5xx_interrupt
                    #display_interrupt
                });

                #[::embassy_executor::task]
                async fn mpsl_task(mpsl: &'static ::nrf_sdc::mpsl::MultiprotocolServiceLayer<'static>) -> ! {
                    mpsl.run().await
                }
                /// How many outgoing L2CAP buffers per link
                const L2CAP_TXQ: u8 = 3;

                /// How many incoming L2CAP buffers per link
                const L2CAP_RXQ: u8 = 3;

                /// Size of L2CAP packets
                const L2CAP_MTU: usize = 251;
                fn build_sdc<'d, const N: usize>(
                    p: ::nrf_sdc::Peripherals<'d>,
                    rng: &'d mut ::embassy_nrf::rng::Rng<::embassy_nrf::mode::Async>,
                    mpsl: &'d ::nrf_sdc::mpsl::MultiprotocolServiceLayer,
                    mem: &'d mut ::nrf_sdc::Mem<N>,
                ) -> Result<::nrf_sdc::SoftdeviceController<'d>, ::nrf_sdc::Error> {
                    ::nrf_sdc::Builder::new()?
                        .support_adv()
                        .support_peripheral()
                        .support_dle_peripheral()
                        .support_dle_central()
                        .support_phy_update_central()
                        .support_phy_update_peripheral()
                        #use_2m_phy
                        #tx_power
                        .peripheral_count(1)?
                        .buffer_cfg(L2CAP_MTU as u16, L2CAP_MTU as u16, L2CAP_TXQ, L2CAP_RXQ)?
                        .build(p, rng, mpsl, mem)
                }
            }
        }
        ChipSeries::Rp2040 => {
            let usb_int = if usb_enabled {
                quote! { USBCTRL_IRQ => ::embassy_rp::usb::InterruptHandler<::embassy_rp::peripherals::USB>; }
            } else {
                quote! {}
            };
            if communication.ble_enabled() {
                quote! {
                    use ::embassy_rp::bind_interrupts;
                    bind_interrupts!(struct Irqs {
                        #usb_int
                        PIO0_IRQ_0 => ::embassy_rp::pio::InterruptHandler<::embassy_rp::peripherals::PIO0>;
                        DMA_IRQ_0 => ::embassy_rp::dma::InterruptHandler<::embassy_rp::peripherals::DMA_CH0>, ::embassy_rp::dma::InterruptHandler<::embassy_rp::peripherals::DMA_CH1>, ::embassy_rp::dma::InterruptHandler<::embassy_rp::peripherals::DMA_CH2>;
                        #iqs5xx_interrupt
                        #display_interrupt
                    });
                    #[::embassy_executor::task]
                    async fn cyw43_task(runner: ::cyw43::Runner<'static, ::cyw43::SpiBus<::embassy_rp::gpio::Output<'static>, ::cyw43_pio::PioSpi<'static, ::embassy_rp::peripherals::PIO0, 0>>, ::cyw43::Cyw43439>) -> ! {
                        runner.run().await
                    }
                }
            } else if !display_interrupt.is_empty() || !iqs5xx_interrupt.is_empty() || dfu_enabled {
                quote! {
                    use ::embassy_rp::bind_interrupts;
                    bind_interrupts!(struct Irqs {
                        #usb_int
                        #iqs5xx_interrupt
                        #display_interrupt
                    });
                }
            } else {
                quote! {}
            }
        }
        _ => quote! {},
    }
}

fn expand_split_peripheral(
    id: usize,
    _identity: &Identity,
    hardware: &Hardware,
    item_mod: ItemMod,
    rmk_features: &Option<Vec<String>>,
) -> TokenStream2 {
    // Check whether keyboard.toml contains split section
    let split_config = match &hardware.board {
        BoardConfig::Split(split) => split,
        _ => {
            panic!("No `split` field in `keyboard.toml`");
        }
    };

    let dfu_enabled =
        is_feature_enabled(rmk_features, "dfu_rp") || is_feature_enabled(rmk_features, "dfu_nrf");

    let peripheral_config = split_config
        .peripheral
        .get(id)
        .expect("Missing peripheral config");

    let imports = expand_custom_imports(&item_mod);
    let mut chip_init = expand_chip_init(hardware, Some(id), &item_mod);
    if split_config.connection == SplitConnection::Ble {
        // Add storage when using BLE split
        let flash_init = expand_flash_init(hardware);
        chip_init.extend(quote! {
            #flash_init
            let mut storage = ::rmk::storage::new_storage_without_keymap(flash, storage_config).await;
        });
    } else if dfu_enabled {
        let flash_init = expand_flash_init(hardware);
        chip_init.extend(quote! { #flash_init });
    }

    // Mark booted when DFU is enabled so the bootloader doesn't
    // revert the previous update.
    if dfu_enabled {
        chip_init.extend(quote! { ::rmk::dfu::mark_booted(); });
    }
    let usb_log_enabled = is_feature_enabled(rmk_features, "usb_log");
    let usb_enabled = dfu_enabled || usb_log_enabled;

    // Run usb device if dfu or usb_log is enabled.
    let usb_task_future = if usb_enabled {
        let usb_init = expand_usb_init(hardware, &item_mod);
        chip_init.extend(usb_init);
        Some(quote! {
            ::rmk::usb::run_peripheral_usb(driver, KEYBOARD_DEVICE_CONFIG)
        })
    } else {
        None
    };

    // Debouncer config
    let col = peripheral_config.cols;
    let row = peripheral_config.rows;

    // Matrix config
    let async_matrix = is_feature_enabled(rmk_features, "async_matrix");
    let chip = &hardware.chip;
    let mut matrix_config = proc_macro2::TokenStream::new();
    let bootmagic = expand_bootmagic_check(&peripheral_config.matrix);
    let debouncer_type = get_debouncer_type(&peripheral_config.matrix);
    match &peripheral_config.matrix.matrix_type {
        MatrixType::Normal => {
            matrix_config.extend(expand_matrix_input_output_pins(
                chip,
                peripheral_config
                    .matrix
                    .row_pins
                    .clone()
                    .expect("split.peripheral.matrix.row_pins is required"),
                peripheral_config
                    .matrix
                    .col_pins
                    .clone()
                    .expect("split.peripheral.matrix.col_pins is required"),
                peripheral_config.matrix.row2col,
                async_matrix,
            ));
            let col2row = !peripheral_config.matrix.row2col;
            let num_row = peripheral_config.rows;
            let num_col = peripheral_config.cols;

            matrix_config.extend(quote! {
                #bootmagic
                let debouncer = #debouncer_type::new();
                let mut matrix = ::rmk::matrix::Matrix::<_, _, _, #num_row, #num_col, #col2row>::new(row_pins, col_pins, debouncer);
            });
        }
        MatrixType::DirectPin => {
            matrix_config.extend(expand_matrix_direct_pins(
                chip,
                peripheral_config
                    .matrix
                    .direct_pins
                    .clone()
                    .expect("split.peripheral.matrix.direct_pins is required"),
                async_matrix,
                peripheral_config.matrix.direct_pin_low_active,
            ));
            // `generic_arg_infer` is a nightly feature. Const arguments cannot yet be inferred with `_` in stable now.
            // So we need to declaring them in advance.
            let size = row * col;
            let low_active = peripheral_config.matrix.direct_pin_low_active;

            matrix_config.extend(quote! {
                #bootmagic
                let debouncer = #debouncer_type::new();
                let mut matrix = ::rmk::matrix::direct_pin::DirectPinMatrix::<_, _, #row, #col, #size>::new(direct_pins, debouncer, #low_active);
            });
        }
    }

    let output_config =
        expand_output_initialization(peripheral_config.output.clone().unwrap_or_default(), chip);

    // Get peripheral device and processor configuration
    let (device_initialization, devices, processors) =
        expand_peripheral_input_device_config(id, hardware);

    let needs_keymap = peripheral_config
        .input_device
        .as_ref()
        .map(|input| input.joystick.as_ref().is_some_and(|v| !v.is_empty()))
        .unwrap_or(false);

    // Generate minimal keymap when processors may read from it.
    let keymap_init = if needs_keymap {
        quote! {
            // Create a minimal keymap for processors that may read from it.
            // Peripheral doesn't use keymap for key processing.
            let mut keymap_data = ::rmk::KeymapData::new([[[::rmk::types::action::KeyAction::No; 1]; 1]; 1]);
            let mut behavior_config = ::rmk::config::BehaviorConfig::default();
            let per_key_config = ::rmk::config::PositionalConfig::default();
            let keymap = ::rmk::initialize_keymap(
                &mut keymap_data,
                &mut behavior_config,
                &per_key_config,
            ).await;
        }
    } else {
        quote! {}
    };

    // Add processor support for peripherals
    let (registered_processor_initializers, mut registered_processors) =
        expand_registered_processor_init(hardware, &item_mod, rmk_features);

    // Display configuration for this peripheral
    let display_init = if let Some(display_config) = &peripheral_config.display {
        let (init, processor) = expand_display_config(&chip.series, display_config);
        let processor_initializer = processor.initializer;
        let processor_var = processor.var_name;
        registered_processors.push(quote! { #processor_var.run() });
        quote! {
            #init
            #processor_initializer
        }
    } else {
        quote! {}
    };

    let (watchdog_init, watchdog_task) = expand_watchdog_init(hardware);

    let runnable_import = if !registered_processors.is_empty() || watchdog_task.is_some() {
        quote! { use ::rmk::core_traits::Runnable; }
    } else {
        quote! {}
    };

    let run_rmk_peripheral = expand_split_peripheral_entry(
        id,
        chip,
        split_config,
        peripheral_config,
        devices,
        processors,
        registered_processors,
        watchdog_task,
        usb_task_future,
    );

    quote! {
        #imports
        #runnable_import
        #chip_init
        #registered_processor_initializers
        #matrix_config
        #keymap_init
        #output_config
        #device_initialization
        #display_init
        #watchdog_init
        #run_rmk_peripheral
    }
}

#[allow(clippy::too_many_arguments)]
fn expand_split_peripheral_entry(
    id: usize,
    chip: &ChipModel,
    split_config: &SplitConfig,
    peripheral_config: &SplitBoardConfig,
    devices: Vec<TokenStream2>,
    processors: Vec<TokenStream2>,
    registered_processors: Vec<TokenStream2>,
    watchdog_task: Option<TokenStream2>,
    usb_task_future: Option<TokenStream2>,
) -> TokenStream2 {
    // Add matrix to devices, and run all devices
    let mut devs = devices.clone();
    devs.push(quote! {matrix});
    if split_config.connection == SplitConnection::Ble {
        devs.push(quote! {storage});
    }
    let device_task = quote! {
        ::rmk::run_all! (
            #(#devs),*
        )
    };

    // Create processor task if there are processors
    let processor_task = if !processors.is_empty() {
        quote! {
            ::rmk::run_all! (
                #(#processors),*
            )
        }
    } else {
        quote! {}
    };

    match split_config.connection {
        SplitConnection::Ble => {
            let peripheral_run = quote! {
                ::rmk::split::peripheral::run_rmk_split_peripheral(
                    #id,
                    ble_controller,
                    ble_addr,
                )
            };
            // Build task list: device, processor (if any), peripheral, registered_processors, dfu
            let mut tasks = vec![device_task];
            if !processors.is_empty() {
                tasks.push(processor_task);
            }
            tasks.push(peripheral_run);
            tasks.extend(registered_processors);
            if let Some(t) = &watchdog_task {
                tasks.push(t.clone());
            }

            if let Some(t) = &usb_task_future {
                tasks.push(t.clone());
            }

            let run_rmk_peripheral = join_all_tasks(tasks);
            quote! {
                #run_rmk_peripheral
            }
        }
        SplitConnection::Serial => {
            let peripheral_serial = peripheral_config
                .serial
                .clone()
                .expect("Missing peripheral serial config");
            if peripheral_serial.len() != 1 {
                panic!("Peripheral should have only one serial config");
            }
            let serial_init = expand_serial_init(chip, peripheral_serial);

            let uart_instance = format_ident!(
                "{}",
                peripheral_config
                    .serial
                    .as_ref()
                    .expect("Missing peripheral serial config")
                    .first()
                    .expect("Peripheral should have only one serial config")
                    .instance
                    .to_lowercase()
            );
            let peripheral_run = quote! {
                ::rmk::split::peripheral::run_rmk_split_peripheral(#uart_instance)
            };
            let mut tasks = vec![device_task, peripheral_run];
            tasks.extend(registered_processors);
            if let Some(t) = &watchdog_task {
                tasks.push(t.clone());
            }

            if let Some(t) = &usb_task_future {
                tasks.push(t.clone());
            }

            let run_rmk_peripheral = join_all_tasks(tasks);
            quote! {
                #serial_init
                #run_rmk_peripheral
            }
        }
    }
}

/// Returns (device initializations, device_names, processor_names)
pub(crate) fn expand_peripheral_input_device_config(
    id: usize,
    hardware: &Hardware,
) -> (TokenStream2, Vec<TokenStream2>, Vec<TokenStream2>) {
    let mut initializations = TokenStream2::new();
    let mut devices = Vec::new();
    let mut processors = Vec::new();

    let communication = &hardware.communication;
    let ble_config = match communication {
        CommunicationConfig::Ble(ble_config) | CommunicationConfig::Both(_, ble_config) => {
            Some(ble_config.clone())
        }
        _ => None,
    };
    let board = &hardware.board;
    let chip = &hardware.chip;

    // Create peripheral-specific BLE config for battery
    // Only use peripheral's own battery config, do NOT fallback to top-level BLE config
    let peripheral_ble_config = match board {
        BoardConfig::Split(split_config) => {
            let peripheral_board = &split_config.peripheral[id];
            // If peripheral has battery config, create a BleConfig with those settings
            if peripheral_board.battery_adc_pin.is_some() {
                Some(BleConfig {
                    enabled: true,
                    battery_adc_pin: peripheral_board.battery_adc_pin.clone(),
                    adc_divider_measured: peripheral_board.adc_divider_measured,
                    adc_divider_total: peripheral_board.adc_divider_total,
                    ..Default::default()
                })
            } else {
                None
            }
        }
        _ => ble_config.clone(),
    };

    // generate ADC configuration
    let (adc_devices, adc_processors) = match board {
        BoardConfig::Split(split_config) => expand_adc_device(
            split_config.peripheral[id]
                .input_device
                .clone()
                .unwrap_or(InputDeviceConfig::default())
                .joystick
                .unwrap_or(Vec::new()),
            peripheral_ble_config,
            chip.series.clone(),
        ),
        _ => (vec![], vec![]),
    };

    for initializer in adc_devices {
        initializations.extend(initializer.initializer);
        let device_name = initializer.var_name;
        devices.push(quote! { #device_name });
    }

    for initializer in adc_processors {
        initializations.extend(initializer.initializer);
        let processor_name = initializer.var_name;
        processors.push(quote! { #processor_name });
    }

    // generate encoder configuration, processors are ignored
    let num_encoders = hardware.board.get_num_encoder();
    // The num_encoders[0] is always the number of encoders on the central, so the offset is the sum of num_encoders[0..id + 1], where id is the index of the peripheral
    let encoder_id_offset = num_encoders[0..id + 1].iter().sum::<usize>();
    let (encoder_devices, _encoder_processors) = match board {
        BoardConfig::Split(split_config) => expand_encoder_device(
            encoder_id_offset,
            split_config.peripheral[id]
                .input_device
                .clone()
                .unwrap_or(InputDeviceConfig::default())
                .encoder
                .unwrap_or(Vec::new()),
            chip,
        ),
        _ => (vec![], vec![]),
    };

    for initializer in encoder_devices {
        initializations.extend(initializer.initializer);
        let device_name = initializer.var_name;
        devices.push(quote! { #device_name });
    }

    // generate PMW3610 configuration
    let (pmw3610_devices, _pmw3610_processors) = match board {
        BoardConfig::Split(split_config) => expand_pmw3610_device(
            split_config.peripheral[id]
                .input_device
                .clone()
                .unwrap_or(InputDeviceConfig::default())
                .pmw3610
                .unwrap_or(Vec::new()),
            chip,
        ),
        _ => (vec![], vec![]),
    };

    for initializer in pmw3610_devices {
        initializations.extend(initializer.initializer);
        let device_name = initializer.var_name;
        devices.push(quote! { #device_name });
    }

    // generate PMW33xx configuration
    let (pmw33xx_devices, _pmw33xx_processors) = match board {
        BoardConfig::Split(split_config) => expand_pmw33xx_device(
            split_config.peripheral[id]
                .input_device
                .clone()
                .unwrap_or(InputDeviceConfig::default())
                .pmw33xx
                .unwrap_or(Vec::new()),
            chip,
        ),
        _ => (vec![], vec![]),
    };

    for initializer in pmw33xx_devices {
        initializations.extend(initializer.initializer);
        let device_name = initializer.var_name;
        devices.push(quote! { #device_name });
    }

    // generate IQS5xx configuration
    let (iqs5xx_devices, _iqs5xx_processors) = match board {
        BoardConfig::Split(split_config) => expand_iqs5xx_device(
            split_config.peripheral[id]
                .input_device
                .clone()
                .unwrap_or(InputDeviceConfig::default())
                .iqs5xx
                .unwrap_or(Vec::new()),
            chip,
        ),
        _ => (vec![], vec![]),
    };

    for initializer in iqs5xx_devices {
        initializations.extend(initializer.initializer);
        let device_name = initializer.var_name;
        devices.push(quote! { #device_name });
    }

    (initializations, devices, processors)
}
