macro_rules! config_matrix_pins_nrf {
    (peripherals: $p:ident, direct_pins: [$([$($pin:tt),+ $(,)?]),+ $(,)?]) => {
        {
            #[allow(unused_mut)]
            let mut pins = [
                $(
                    [
                        $(
                            config_matrix_pin_nrf!(@pin $p, $pin)
                        ),+
                    ]
                ),+
            ];
            pins
        }
    };
}

macro_rules! config_matrix_pin_nrf {
    (@pin $p:ident, _) => {
        None
    };

    (@pin $p:ident, $pin:ident) => {
        Some(embassy_nrf::gpio::Input::new($p.$pin, embassy_nrf::gpio::Pull::Up))
    };
}
