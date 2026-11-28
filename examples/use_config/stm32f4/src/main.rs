#![no_main]
#![no_std]

use rmk::macros::rmk_keyboard;

// Create and run your keyboard with a single macro: `rmk_keyboard`
#[rmk_keyboard]
mod keyboard {
    use embassy_stm32::Config;
    use embassy_stm32::time::Hertz;

    #[Override(chip_config)]
    fn config() -> Config {
        // RCC config
        // Tested on stm32f411 with 25MHZ HSE and stm32f407 with 8MHZ HSE
        // The commented rcc configuration code is for stm32f407
        let mut config = Config::default();
        {
            use embassy_stm32::rcc::*;
            config.rcc.hse = Some(Hse {
                freq: Hertz(25_000_000),
                // freq: Hertz(8_000_000),
                mode: HseMode::Oscillator,
            });
            config.rcc.pll_src = PllSource::Hse;
            config.rcc.pll = Some(Pll {
                prediv: PllPreDiv::Div25,
                // prediv: PllPreDiv::Div8,
                mul: PllMul::Mul192,
                divp: Some(PllPDiv::Div2), // 25mhz / 25 * 192 / 2 = 96Mhz.
                divq: Some(PllQDiv::Div4), // 25mhz / 25 * 192 / 4 = 48Mhz.
                divr: None,
            });
            config.rcc.ahb_pre = AHBPrescaler::Div1;
            config.rcc.apb1_pre = APBPrescaler::Div2;
            config.rcc.apb2_pre = APBPrescaler::Div1;
            // config.rcc.apb1_pre = APBPrescaler::Div4;
            // config.rcc.apb2_pre = APBPrescaler::Div2;
            config.rcc.sys = Sysclk::Pll1P;
        }
        config
    }
}
