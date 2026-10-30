use quote::{format_ident, quote};
use rmk_config::EncoderPhase;
use rmk_config::resolved::hardware::{ChipModel, EncoderConfig};

use super::Initializer;
use crate::codegen::chip::gpio::convert_gpio_str_to_input_pin;

/// Expand encoder device, this function returns the (device_initializer, processor_initializer)
///
/// `id_offset` is the offset of the encoder id, it is used to distinguish the encoder id between central and peripheral
pub(crate) fn expand_encoder_device(
    id_offset: usize,
    encoder_config: Vec<EncoderConfig>,
    chip: &ChipModel,
) -> (Vec<Initializer>, Vec<Initializer>) {
    if encoder_config.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let mut device_initializer = vec![];

    // Create rotary encoders
    for (idx, encoder) in encoder_config.iter().enumerate() {
        let encoder_id = idx as u8 + id_offset as u8;

        let pull = if encoder.internal_pullup {
            Some(true)
        } else {
            None
        };

        // Initialize pins
        let pin_a = convert_gpio_str_to_input_pin(chip, encoder.pin_a.clone(), false, pull);
        let pin_b = convert_gpio_str_to_input_pin(chip, encoder.pin_b.clone(), false, pull);

        let encoder_name = format_ident!("encoder_{}", encoder_id);
        // encoder_names.push(encoder_name.clone());

        let debounce_chain = match encoder.debounce_ms {
            Some(ms) if ms > 0 => quote! { .with_debounce(#ms) },
            _ => quote! {},
        };

        // Create different types of encoders based on the phase field
        let encoder_device = match encoder.phase {
            EncoderPhase::E8h7 => {
                quote! {
                    let mut #encoder_name = ::rmk::input_device::rotary_encoder::RotaryEncoder::with_phase(
                        #pin_a,
                        #pin_b,
                        ::rmk::input_device::rotary_encoder::E8H7Phase,
                        #encoder_id
                    )#debounce_chain;
                }
            }
            EncoderPhase::Resolution => {
                // When phase is "resolution", ensure resolution and reverse are set
                let resolution = encoder
                    .resolution
                    .as_ref()
                    .expect("`resolution` field needs to be set when the encoder's mode is 'resolution'")
                    .value()
                    .unwrap_or_else(|e| panic!("\n\u{274c} keyboard.toml: {e}"));
                let reverse = encoder.reverse.unwrap_or(false);

                quote! {
                    let mut #encoder_name = ::rmk::input_device::rotary_encoder::RotaryEncoder::with_resolution(
                        #pin_a,
                        #pin_b,
                        #resolution,
                        #reverse,
                        #encoder_id
                    )#debounce_chain;
                }
            }
            EncoderPhase::Default => {
                quote! {
                    let mut #encoder_name = ::rmk::input_device::rotary_encoder::RotaryEncoder::with_phase(
                        #pin_a,
                        #pin_b,
                        ::rmk::input_device::rotary_encoder::DefaultPhase,
                        #encoder_id
                    )#debounce_chain;
                }
            }
        };

        device_initializer.push(Initializer {
            initializer: encoder_device,
            var_name: encoder_name,
        });
    }

    (device_initializer, vec![])
}
