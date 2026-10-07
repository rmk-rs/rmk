use quote::{format_ident, quote};
use rmk_config::resolved::hardware::{ChipModel, ChipSeries, Pmw3610Config};

use super::{expand_pointing_acceleration, Initializer};

/// Expand PMW3610 device configuration.
/// Returns (device initializers, processor initializers)
pub(crate) fn expand_pmw3610_device(
    pmw3610_config: Vec<Pmw3610Config>,
    chip: &ChipModel,
) -> (Vec<Initializer>, Vec<Initializer>) {
    if pmw3610_config.is_empty() {
        return (Vec::new(), Vec::new());
    }

    // PMW3610 is only supported on nRF52 and RP2040
    match chip.series {
        ChipSeries::Nrf52 | ChipSeries::Rp2040 => {}
        _ => {
            panic!("PMW3610 is only supported on nRF52 and RP2040 chips");
        }
    }

    let mut device_initializers = vec![];
    let mut processor_initializers = vec![];

    for (idx, sensor) in pmw3610_config.iter().enumerate() {
        let sensor_id = sensor.id.unwrap_or(0);
        let sensor_name = if sensor.name.is_empty() {
            format!("pmw3610_{}_id{}", idx, sensor_id)
        } else {
            format!("{}_id{}", sensor.name.clone(), sensor_id)
        };

        let device_ident = format_ident!("{}_device", sensor_name);
        let processor_ident = format_ident!("{}_processor", sensor_name);
        let processor_ident_config = format_ident!("{}_config", processor_ident);

        // Generate pin initialization
        let spi = &sensor.spi;
        let mosi_pin = &spi.mosi;
        let miso_pin = &spi.miso;

        if mosi_pin.is_empty() && miso_pin.is_empty() {
            panic!("pmw3610 requires at least one of spi.mosi or spi.miso");
        }
        if !(mosi_pin == miso_pin || mosi_pin.is_empty() || miso_pin.is_empty()) {
            panic!("pmw3610 requires spi.mosi == spi.miso, or one of them empty");
        }

        let sdio_pin = if mosi_pin.is_empty() {
            miso_pin
        } else {
            mosi_pin
        };

        let sck_ident = format_ident!("{}", spi.sck);
        let sdio_ident = format_ident!("{}", sdio_pin);
        let cs_ident = format_ident!(
            "{}",
            spi.cs
                .as_ref()
                .expect("pmw3610 requires `cs` in spi config")
        );

        // Generate config values
        let res_cpi: i16 = sensor.cpi.map(|c| c as i16).unwrap_or(-1);
        let invert_x = sensor.invert_x;
        let invert_y = sensor.invert_y;
        let swap_xy = sensor.swap_xy;
        let proc_invert_x = sensor.proc_invert_x;
        let proc_invert_y = sensor.proc_invert_y;
        let proc_swap_xy = sensor.proc_swap_xy;
        let proc_acceleration = expand_pointing_acceleration(&sensor.proc_acceleration);
        let proc_scroll_acceleration =
            expand_pointing_acceleration(&sensor.proc_scroll_acceleration);
        let force_awake = sensor.force_awake;
        let smart_mode = sensor.smart_mode;
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
                _ => unreachable!(),
            }
        };

        // Generate device initialization based on chip series
        let device_init = match chip.series {
            ChipSeries::Nrf52 => quote! {
                let mut #device_ident = {
                    use ::embassy_nrf::gpio::{Output, Flex, Level, OutputDrive};
                    use ::rmk::input_device::pmw3610::{BitBangSpiBus, Pmw3610, Pmw3610Config};
                    use ::rmk::input_device::pointing::PointingDevice;

                    let sck = Output::new(p.#sck_ident, Level::High, OutputDrive::Standard);
                    let sdio = Flex::new(p.#sdio_ident);
                    let cs = Output::new(p.#cs_ident, Level::High, OutputDrive::Standard);
                    let motion = #motion_pin_init;

                    let spi_bus = BitBangSpiBus::new(sck, sdio);

                    let config = Pmw3610Config {
                        res_cpi: #res_cpi,
                        invert_x: #invert_x,
                        invert_y: #invert_y,
                        swap_xy: #swap_xy,
                        force_awake: #force_awake,
                        smart_mode: #smart_mode,
                    };

                    PointingDevice::<Pmw3610<_, _, _>>::with_report_hz(#sensor_id, spi_bus, cs, motion, config, #report_hz)
                };
            },
            ChipSeries::Rp2040 => quote! {
                let mut #device_ident = {
                    use ::embassy_rp::gpio::{Output, Flex, Level};
                    use ::rmk::input_device::pmw3610::{BitBangSpiBus, Pmw3610, Pmw3610Config};
                    use ::rmk::input_device::pointing::PointingDevice;

                    let sck = Output::new(p.#sck_ident, Level::High);
                    let sdio = Flex::new(p.#sdio_ident);
                    let cs = Output::new(p.#cs_ident, Level::High);
                    let motion = #motion_pin_init;

                    let spi_bus = BitBangSpiBus::new(sck, sdio);

                    let config = Pmw3610Config {
                        res_cpi: #res_cpi,
                        invert_x: #invert_x,
                        invert_y: #invert_y,
                        swap_xy: #swap_xy,
                        force_awake: #force_awake,
                        smart_mode: #smart_mode,
                    };

                    PointingDevice::<Pmw3610<_, _, _>>::with_report_hz(#sensor_id, spi_bus, cs, motion, config, #report_hz)
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
