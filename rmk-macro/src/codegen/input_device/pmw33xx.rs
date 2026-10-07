use quote::{format_ident, quote};
use rmk_config::resolved::hardware::{ChipModel, ChipSeries, Pmw33xxConfig, Pmw33xxType};

use super::{expand_pointing_acceleration, Initializer};

/// Expand PMW33xx device configuration.
/// Returns (device initializers, processor initializers)
pub(crate) fn expand_pmw33xx_device(
    pmw33xx_config: Vec<Pmw33xxConfig>,
    chip: &ChipModel,
) -> (Vec<Initializer>, Vec<Initializer>) {
    if pmw33xx_config.is_empty() {
        return (Vec::new(), Vec::new());
    }

    // PMW33xx is only supported on nRF52, STM32 and RP2040
    match chip.series {
        // NOTE: Nonblocking SPI with DMA is still marked unstable in esp_hal.
        ChipSeries::Nrf52 | ChipSeries::Rp2040 | ChipSeries::Stm32 => {}
        _ => {
            panic!("PMW33xx is only supported on nRF52, STM32, RP2350 and RP2040 chips");
        }
    }

    let mut device_initializers = vec![];
    let mut processor_initializers = vec![];

    for (idx, sensor) in pmw33xx_config.iter().enumerate() {
        let sensor_type = match sensor.sensor_type {
            Pmw33xxType::PMW3360 => "pmw3360",
            Pmw33xxType::PMW3389 => "pmw3389",
        };
        let sensor_id = sensor.id.unwrap_or(0);
        let sensor_name = if sensor.name.is_empty() {
            format!("{}_{}_id{}", sensor_type, idx, sensor_id)
        } else {
            format!("{}_id{}", sensor.name.clone(), sensor_id)
        };

        let device_ident = format_ident!("{}_device", sensor_name);
        let processor_ident = format_ident!("{}_processor", sensor_name);
        let processor_ident_config = format_ident!("{}_config", processor_ident);
        let sensor_spec_ident = match sensor.sensor_type {
            Pmw33xxType::PMW3360 => format_ident!("{}", "Pmw3360Spec"),
            Pmw33xxType::PMW3389 => format_ident!("{}", "Pmw3389Spec"),
        };

        // Generate pin initialization
        let spi = &sensor.spi;

        let sck_ident = format_ident!("{}", spi.sck);
        let mosi_ident = format_ident!("{}", spi.mosi);
        let miso_ident = format_ident!("{}", spi.miso);
        let cs_ident = format_ident!(
            "{}",
            spi.cs
                .as_ref()
                .expect("pmw33xx requires `cs` in spi config")
        );
        let instance_ident = format_ident!("{}", spi.instance);

        let rx_dma_ident = match chip.series {
            ChipSeries::Nrf52 => {
                quote! {}
            }

            ChipSeries::Rp2040 | ChipSeries::Stm32 => {
                if let Some(rx_dma) = &spi.rx_dma {
                    let rx_dma_ident = format_ident!("{}", rx_dma);
                    quote! { #rx_dma_ident }
                } else {
                    quote! {}
                }
            }

            _ => unreachable!(),
        };
        let tx_dma_ident = match chip.series {
            ChipSeries::Nrf52 => {
                quote! {}
            }

            ChipSeries::Rp2040 | ChipSeries::Stm32 => {
                if let Some(tx_dma) = &spi.tx_dma {
                    let tx_dma_ident = format_ident!("{}", tx_dma);
                    quote! { #tx_dma_ident }
                } else {
                    quote! {}
                }
            }

            _ => unreachable!(),
        };

        // if one dma channel is specified, the other one must also be
        let spi_bus_init = match (&spi.tx_dma, &spi.rx_dma) {
            (Some(_), None) => {
                panic!(
                    "{}: tx_dma is specified but rx_dma is missing. Both must be present or both absent.",
                    sensor_name
                );
            }
            (None, Some(_)) => {
                panic!(
                    "{}: rx_dma is specified but tx_dma is missing. Both must be present or both absent.",
                    sensor_name
                );
            }
            (None, None) => {
                quote! {
                    ::embassy_embedded_hal::adapter::BlockingAsync::new(Spi::new_blocking(spi_inst, sck, mosi, miso, spi_config))
                }
            }
            (Some(_), Some(_)) => {
                quote! {
                    Spi::new(spi_inst, sck, mosi, miso, p.#tx_dma_ident, p.#rx_dma_ident, spi_config)
                }
            }
        };

        // Generate config values
        let res_cpi: u16 = sensor.cpi.unwrap_or(1600);
        let rot_trans_angle: i8 = sensor.rot_trans_angle.unwrap_or(0);
        let liftoff_dist: u8 = sensor.liftoff_dist.unwrap_or(0);
        let proc_invert_x = sensor.proc_invert_x;
        let proc_invert_y = sensor.proc_invert_y;
        let proc_swap_xy = sensor.proc_swap_xy;
        let proc_acceleration = expand_pointing_acceleration(&sensor.proc_acceleration);
        let proc_scroll_acceleration =
            expand_pointing_acceleration(&sensor.proc_scroll_acceleration);
        let report_hz: u16 = sensor.report_hz;

        // Generate motion pin initialization (optional)
        let motion_pin_init = if let Some(motion_pin) = &sensor.motion {
            let motion_ident = format_ident!("{}", motion_pin);
            match chip.series {
                ChipSeries::Nrf52 => quote! {
                    Some(::embassy_nrf::gpio::Input::new(p.#motion_ident, ::embassy_nrf::gpio::Pull::Up))
                },
                ChipSeries::Rp2040 => quote! {
                    Some(::embassy_rp::gpio::Input::new(p.#motion_ident, ::embassy_rp::gpio::Pull::Up))
                },
                ChipSeries::Stm32 => quote! {
                    None::<::embassy_stm32::exti::ExtiInput<'static, ::embassy_stm32::mode::Async>>
                },
                _ => unreachable!(),
            }
        } else {
            match chip.series {
                ChipSeries::Nrf52 => quote! {
                    None::<::embassy_nrf::gpio::Input<'static>>
                },
                ChipSeries::Rp2040 => quote! {
                    None::<::embassy_rp::gpio::Input<'static>>
                },
                ChipSeries::Stm32 => quote! {
                    None::<::embassy_stm32::exti::ExtiInput<'static, ::embassy_stm32::mode::Async>>
                },
                _ => unreachable!(),
            }
        };

        // Generate device initialization based on chip series
        let device_init = match chip.series {
            ChipSeries::Nrf52 => quote! {
                let mut #device_ident = {
                    use ::embassy_nrf::spim::{Frequency, Spim, Config, MODE_3};
                    use ::embassy_nrf::gpio::{Output, OutputDrive, Level};
                    use ::rmk::input_device::pmw33xx::{Pmw33xx, Pmw33xxConfig, #sensor_spec_ident};
                    use ::rmk::input_device::pointing::PointingDevice;

                    let spi_inst = p.#instance_ident;
                    let sck = p.#sck_ident;
                    let mosi = p.#mosi_ident;
                    let miso = p.#miso_ident;
                    let cs = Output::new(p.#cs_ident, Level::High, OutputDrive::Standard);
                    let motion = #motion_pin_init;

                    let mut spi_config = Config::default();
                    spi_config.frequency = Frequency::M2;
                    spi_config.mode = MODE_3;

                    let spi_bus = Spim::new(spi_inst, Irqs, sck, miso, mosi, spi_config);

                    let config = Pmw33xxConfig {
                        res_cpi: #res_cpi,
                        rot_trans_angle: #rot_trans_angle,
                        liftoff_dist: #liftoff_dist,
                        ..Default::default()
                    };

                    PointingDevice::<Pmw33xx<_, _, _, #sensor_spec_ident>>::with_report_hz(#sensor_id, spi_bus, cs, motion, config, #report_hz)
                };
            },
            ChipSeries::Rp2040 => quote! {
                let mut #device_ident = {
                    use ::embassy_rp::spi::{Spi, Config, Polarity, Phase};
                    use ::embassy_rp::gpio::{Output, Level, Pull};
                    use ::rmk::input_device::pmw33xx::{Pmw33xx, Pmw33xxConfig, #sensor_spec_ident};
                    use ::rmk::input_device::pointing::PointingDevice;

                    let spi_inst = p.#instance_ident;
                    let sck = p.#sck_ident;
                    let mosi = p.#mosi_ident;
                    let miso = p.#miso_ident;
                    let cs = Output::new(p.#cs_ident, Level::High);
                    let motion = #motion_pin_init;

                    let mut spi_config = Config::default();
                    spi_config.polarity = Polarity::IdleHigh;
                    spi_config.phase = Phase::CaptureOnSecondTransition;
                    spi_config.frequency = 2_000_000;

                    let spi_bus = #spi_bus_init;

                    let config = Pmw33xxConfig {
                        res_cpi: #res_cpi,
                        rot_trans_angle: #rot_trans_angle,
                        liftoff_dist: #liftoff_dist,
                        ..Default::default()
                    };

                    PointingDevice::<Pmw33xx<_, _, _, #sensor_spec_ident>>::with_report_hz(#sensor_id, spi_bus, cs, motion, config, #report_hz)
                };
            },
            ChipSeries::Stm32 => quote! {
                let mut #device_ident = {
                    use ::embassy_stm32::spi::{Spi, Config, MODE_3};
                    use ::embassy_stm32::gpio::{Output, Level, Pull, Speed};
                    use ::embassy_stm32::time::Hertz;
                    use ::embassy_stm32::exti::ExtiInput;
                    use ::rmk::input_device::pmw33xx::{Pmw33xx, Pmw33xxConfig, #sensor_spec_ident};
                    use ::rmk::input_device::pointing::PointingDevice;

                    let spi_inst = p.#instance_ident;
                    let sck = p.#sck_ident;
                    let mosi = p.#mosi_ident;
                    let miso = p.#miso_ident;
                    let cs = Output::new(p.#cs_ident, Level::High, Speed::Medium);
                    let motion = #motion_pin_init;

                    let mut spi_config = Config::default();
                    spi_config.frequency = Hertz::mhz(2);
                    spi_config.mode = MODE_3;

                    let spi_bus = #spi_bus_init;

                    let config = Pmw33xxConfig {
                        res_cpi: #res_cpi,
                        rot_trans_angle: #rot_trans_angle,
                        liftoff_dist: #liftoff_dist,
                        ..Default::default()
                    };

                    PointingDevice::<Pmw33xx<_, _, _, #sensor_spec_ident>>::with_report_hz(#sensor_id, spi_bus, cs, motion, config, #report_hz)

                };
            },
            _ => unreachable!(),
        };

        device_initializers.push(Initializer {
            initializer: device_init,
            var_name: device_ident,
        });

        // Generate processor initialization
        let processor_init = quote! {

            let #processor_ident_config = ::rmk::input_device::pointing::PointingProcessorConfig {
                device_id: #sensor_id,
                invert_x: #proc_invert_x,
                invert_y: #proc_invert_y,
                swap_xy: #proc_swap_xy,
                acceleration: #proc_acceleration,
                scroll_acceleration: #proc_scroll_acceleration,
                ..Default::default()
            };

            let mut #processor_ident = ::rmk::input_device::pointing::PointingProcessor::new(&keymap, #processor_ident_config);
        };

        processor_initializers.push(Initializer {
            initializer: processor_init,
            var_name: processor_ident,
        });
    }

    (device_initializers, processor_initializers)
}
