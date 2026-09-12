macro_rules! config_matrix_pins_nrf {
    (peripherals: $p:ident, input: [$($in_pin:ident), *], output: [$($out_pin:ident), +]) => {
        {
            let output_pins = [$(Output::new($p.$out_pin, embassy_nrf::gpio::Level::Low, embassy_nrf::gpio::OutputDrive::Standard)), +];
            let input_pins = [$(Input::new($p.$in_pin, embassy_nrf::gpio::Pull::Down)), +];
            (input_pins, output_pins)
        }
    };
}
