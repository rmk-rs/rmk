#![no_main]
#![no_std]

use rmk::macros::rmk_keyboard;

// Create and run your keyboard with a single macro: `rmk_keyboard`
#[rmk_keyboard]
mod keyboard {
    use embassy_stm32::Config;
    use embassy_stm32::rcc::*;
    use embassy_stm32::time::Hertz;

    #[Override(chip_config)]
    fn config() -> Config {
        let mut config = Config::default();
        config.rcc.hse = Some(Hse {
            freq: Hertz(8_000_000),
            // Oscillator for bluepill, Bypass for nucleos.
            mode: HseMode::Oscillator,
        });
        config.rcc.pll = Some(Pll {
            src: PllSource::HSE,
            prediv: PllPreDiv::Div1,
            mul: PllMul::Mul9,
        });
        config.rcc.sys = Sysclk::Pll1P;
        config.rcc.ahb_pre = AHBPrescaler::Div1;
        config.rcc.apb1_pre = APBPrescaler::Div2;
        config.rcc.apb2_pre = APBPrescaler::Div1;
        config
    }
}
