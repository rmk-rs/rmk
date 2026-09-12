#[macro_export]
macro_rules! config_matrix_pins_rp {
    (peripherals: $p:ident, input: [$($in_pin:ident),*], output: [$($out_pin:ident),*]) => {
        {
            let output_pins = [$(Output::new($p.$out_pin, Level::Low)),*];
            let input_pins = [$(Input::new($p.$in_pin, embassy_rp::gpio::Pull::Down)),*];
            (input_pins, output_pins)
        }
    };
}
