#![no_main]
#![no_std]

//! NOTE: This example compiles on latest main branch, which may be different from released version

use rmk::macros::rmk_keyboard;

/// There is an example of full customization of the keyboard with `rmk_keyboard` macro
#[rmk_keyboard]
mod my_keyboard {
    use embassy_stm32::Config;
    use embassy_stm32::time::Hertz;
    use embassy_stm32::usb::Driver;
    use rmk::processor::builtin::wpm::WpmProcessor;
    use rmk::run_all;
    use rmk::usb::UsbTransport;
    use static_cell::StaticCell;

    // If you want customize interrupte binding , use `#[Override(bind_interrupt)]` to override default interrupt binding.
    // The override replaces the whole generated `bind_interrupts!` block, so it has to bind
    // everything this configuration needs (here: USB, plus the EXTI lines used by `async_matrix`).
    #[Override(bind_interrupt)]
    fn bind_interrupt() {
        embassy_stm32::bind_interrupts!(struct Irqs {
            OTG_HS => embassy_stm32::usb::InterruptHandler<embassy_stm32::peripherals::USB_OTG_HS>;
            EXTI9_5 => embassy_stm32::exti::InterruptHandler<embassy_stm32::peripherals::EXTI9>,
                embassy_stm32::exti::InterruptHandler<embassy_stm32::peripherals::EXTI8>;
            EXTI15_10 => embassy_stm32::exti::InterruptHandler<embassy_stm32::peripherals::EXTI13>,
                embassy_stm32::exti::InterruptHandler<embassy_stm32::peripherals::EXTI12>;
        });
    }

    // If you're using custom chip config, use `#[Override(chip_config)]` to override embassy's default config
    #[Override(chip_config)]
    fn config() -> Config {
        let mut config = Config::default();
        {
            use embassy_stm32::rcc::{mux, *};
            config.rcc.hsi = Some(HSIPrescaler::Div1);
            config.rcc.csi = true;
            // Needed for USB
            config.rcc.hsi48 = Some(Hsi48Config { sync_from_usb: true });
            // External oscillator 25MHZ
            config.rcc.hse = Some(Hse {
                freq: Hertz(25_000_000),
                mode: HseMode::Oscillator,
            });
            config.rcc.pll1 = Some(Pll {
                source: PllSource::Hse,
                prediv: PllPreDiv::Div5,
                mul: PllMul::Mul112,
                fracn: None,
                divp: Some(PllDiv::Div2),
                divq: Some(PllDiv::Div2),
                divr: Some(PllDiv::Div2),
            });
            config.rcc.sys = Sysclk::Pll1P;
            config.rcc.ahb_pre = AHBPrescaler::Div2;
            config.rcc.apb1_pre = APBPrescaler::Div2;
            config.rcc.apb2_pre = APBPrescaler::Div2;
            config.rcc.apb3_pre = APBPrescaler::Div2;
            config.rcc.apb4_pre = APBPrescaler::Div2;
            config.rcc.voltage_scale = VoltageScale::Scale0;
            // Configure USB clock mux to use HSI48 (internal 48MHz oscillator)
            // HSI48 is internal, no external crystal needed for USB - better compatibility
            config.rcc.mux.usbsel = mux::Usbsel::Hsi48;
        }
        config
    }

    // If you're using custom usb config, use `#[Override(usb)]` to override default usb config
    #[Override(usb)]
    fn usb() -> Driver<'_, USB_OTG_HS> {
        static EP_OUT_BUFFER: StaticCell<[u8; 1024]> = StaticCell::new();
        let mut usb_config = embassy_stm32::usb::Config::default();
        usb_config.vbus_detection = false;

        Driver::new_fs(
            p.USB_OTG_HS,
            p.PA12,
            p.PA11,
            Irqs,
            &mut EP_OUT_BUFFER.init([0; 1024])[..],
            usb_config,
        )
    }

    // Use `#[Override(entry)]` to override default rmk keyboard runner
    #[Override(entry)]
    fn run() {
        let mut usb_transport = UsbTransport::new(driver, rmk_config.device_config).with_host_service(&host_service);
        let mut wpm_processor = WpmProcessor::new();

        // Start
        run_all!(matrix, usb_transport, wpm_processor, keyboard).await;
    }
}
