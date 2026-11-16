//! Initialize matrix initialization boilerplate of RMK
//!
use quote::quote;
use rmk_config::resolved::Hardware;
use rmk_config::resolved::hardware::{
    BoardConfig, ChipModel, ChipSeries, MatrixConfig, MatrixType, UniBodyConfig,
};

use super::chip::gpio::{
    convert_direct_pins_to_initializers, convert_input_pins_to_initializers,
    convert_output_pins_to_initializers, get_input_pin_type, get_output_pin_type,
};
use super::feature::is_feature_enabled;

pub(crate) fn expand_matrix_config(
    hardware: &Hardware,
    rmk_features: &Option<Vec<String>>,
) -> proc_macro2::TokenStream {
    let async_matrix = is_feature_enabled(rmk_features, "async_matrix");
    let mut matrix_config = proc_macro2::TokenStream::new();
    match &hardware.board {
        BoardConfig::UniBody(UniBodyConfig { matrix, .. }) => match matrix.matrix_type {
            MatrixType::Normal => {
                matrix_config.extend(expand_matrix_input_output_pins(
                    &hardware.chip,
                    matrix.row_pins.clone().unwrap(),
                    matrix.col_pins.clone().unwrap(),
                    matrix.row2col,
                    async_matrix,
                ));
            }
            MatrixType::DirectPin => {
                matrix_config.extend(expand_matrix_direct_pins(
                    &hardware.chip,
                    matrix.direct_pins.clone().unwrap(),
                    async_matrix,
                    matrix.direct_pin_low_active,
                ));
                let direct_pins = matrix.direct_pins.as_ref().unwrap();
                let rows = direct_pins.len();
                let cols = direct_pins.first().map_or(0, |row| row.len());
                let size = rows * cols;
                let low_active = matrix.direct_pin_low_active;
                matrix_config.extend(quote! {
                    pub(crate) const SIZE: usize = #size;
                    let low_active = #low_active;
                });
            }
        },
        BoardConfig::Split(split_config) => {
            // Matrix config for split central
            match split_config.central.matrix.matrix_type {
                MatrixType::Normal => matrix_config.extend(expand_matrix_input_output_pins(
                    &hardware.chip,
                    split_config.central.matrix.row_pins.clone().unwrap(),
                    split_config.central.matrix.col_pins.clone().unwrap(),
                    split_config.central.matrix.row2col,
                    async_matrix,
                )),
                MatrixType::DirectPin => matrix_config.extend(expand_matrix_direct_pins(
                    &hardware.chip,
                    split_config.central.matrix.direct_pins.clone().unwrap(),
                    async_matrix,
                    split_config.central.matrix.direct_pin_low_active,
                )),
            }
        }
    };
    matrix_config
}

pub(crate) fn expand_matrix_direct_pins(
    chip: &ChipModel,
    direct_pins: Vec<Vec<String>>,
    async_matrix: bool,
    low_active: bool,
) -> proc_macro2::TokenStream {
    let mut pin_initialization = proc_macro2::TokenStream::new();
    // Get input pin type
    let input_pin_type = get_input_pin_type(chip, async_matrix);
    let rows = direct_pins.len();
    let cols = direct_pins.first().map_or(0, |row| row.len());
    // Initialize input pins
    pin_initialization.extend(convert_direct_pins_to_initializers(
        chip,
        direct_pins,
        async_matrix,
        low_active,
    ));

    quote! {
        let direct_pins: [[Option<#input_pin_type>; #cols]; #rows] = {
            #pin_initialization
            direct_pins
        };
    }
}

/// Emit a one-shot bootmagic scan: if the configured key is held while the
/// firmware boots, jump to the chip bootloader instead of starting normal
/// operation.
///
/// The emitted code expects the matrix pin arrays to already be in scope:
/// `row_pins` and `col_pins` for normal matrices, or `direct_pins` for the
/// direct-pin variant. It must be inserted *after* pin init but *before*
/// `Matrix::new` consumes the arrays.
///
/// Bootmagic coordinates are bounds-checked at macro-expansion time against
/// the dimensions inferred from the matrix configuration itself, so callers
/// don't need to pass them.
pub(crate) fn expand_bootmagic_check(matrix: &MatrixConfig) -> proc_macro2::TokenStream {
    let Some(bm) = matrix.bootmagic else {
        return quote! {};
    };
    let row = bm.0 as usize;
    let col = bm.1 as usize;
    match matrix.matrix_type {
        MatrixType::Normal => {
            let num_rows = matrix.row_pins.as_deref().map_or(0, <[_]>::len);
            let num_cols = matrix.col_pins.as_deref().map_or(0, <[_]>::len);
            if row >= num_rows || col >= num_cols {
                panic!(
                    "bootmagic key ({row}, {col}) is out of range for matrix {num_rows}×{num_cols}"
                );
            }
            let (output_pins, output_idx, input_pins, input_idx) = if matrix.row2col {
                (quote!(row_pins), row, quote!(col_pins), col)
            } else {
                (quote!(col_pins), col, quote!(row_pins), row)
            };
            quote! {
                {
                    // Bootmagic: drop into the bootloader if the configured
                    // key is held during boot.
                    #output_pins[#output_idx].set_high();
                    ::embassy_time::Timer::after_micros(50).await;
                    let bootmagic_pressed = #input_pins[#input_idx].is_high();
                    #output_pins[#output_idx].set_low();
                    if bootmagic_pressed {
                        ::rmk::boot::jump_to_bootloader();
                    }
                }
            }
        }
        MatrixType::DirectPin => {
            if matrix
                .direct_pins
                .as_deref()
                .and_then(|pins| pins.get(row))
                .and_then(|r| r.get(col))
                .is_none_or(|name| name == "_" || name.eq_ignore_ascii_case("trns"))
            {
                panic!("bootmagic cell ({row}, {col}) has no pin assigned")
            }
            let pressed_call = if matrix.direct_pin_low_active {
                quote! { pin.is_low() }
            } else {
                quote! { pin.is_high() }
            };
            quote! {
                {
                    // Bootmagic: drop into the bootloader if the configured
                    // key is held during boot.
                    if let Some(ref pin) = direct_pins[#row][#col] {
                        if #pressed_call {
                            ::rmk::boot::jump_to_bootloader();
                        }
                    }
                }
            }
        }
    }
}

pub(crate) fn expand_matrix_input_output_pins(
    chip: &ChipModel,
    row_pins: Vec<String>,
    col_pins: Vec<String>,
    row2col: bool,
    async_matrix: bool,
) -> proc_macro2::TokenStream {
    let mut pin_initialization = proc_macro2::TokenStream::new();
    // Extra import when using `ExtiInput`
    let extra_import = if chip.series == ChipSeries::Stm32 && async_matrix {
        quote! {
            use ::embassy_stm32::exti::Channel;
        }
    } else {
        quote! {}
    };
    let (input_pins, output_pins) = if row2col {
        (col_pins, row_pins)
    } else {
        (row_pins, col_pins)
    };

    let input_pin_len = input_pins.len();
    let output_pin_len = output_pins.len();

    // Get pin types
    let input_pin_type = get_input_pin_type(chip, async_matrix);
    let output_pin_type = get_output_pin_type(chip);

    // Initialize input pins
    pin_initialization.extend(convert_input_pins_to_initializers(
        chip,
        input_pins,
        async_matrix,
    ));
    // Initialize output pins
    pin_initialization.extend(convert_output_pins_to_initializers(chip, output_pins));
    let pin_names = if row2col {
        quote! { (mut col_pins, mut row_pins) }
    } else {
        quote! { (mut row_pins, mut col_pins) }
    };
    // Generate a macro that does pin matrix config
    quote! {
        #extra_import
        let #pin_names: ([ #input_pin_type; #input_pin_len], [ #output_pin_type; #output_pin_len]) = {
            #pin_initialization
            (input_pins, output_pins)
        };
    }
}
