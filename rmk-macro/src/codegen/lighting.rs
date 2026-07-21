//! Generate flash-resident shared geometry and semantic lighting topology.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use rmk_config::resolved::lighting::{
    Lighting, LightingBackgroundMode, LightingChargeCondition, LightingConditionalSceneCell,
    LightingEffect, LightingOutputMode, LightingSceneCell,
};
use rmk_config::resolved::{FixedPoint3, PhysicalLayout};

pub(crate) fn expand_physical_layout(layout: &PhysicalLayout) -> TokenStream2 {
    let keys = layout.keys.iter().map(|key| {
        let [row, col] = key.matrix;
        let center = expand_point(key.center);
        let width = key.size.width;
        let height = key.size.height;
        let rotation = key.rotation_centidegrees;
        quote! {
            ::rmk::physical_layout::PhysicalKey {
                matrix: ::rmk::physical_layout::KeyPosition::new(#row, #col),
                center: #center,
                size: ::rmk::physical_layout::KeySize::new(
                    ::rmk::physical_layout::Extent::from_raw(#width),
                    ::rmk::physical_layout::Extent::from_raw(#height),
                ),
                rotation: ::rmk::physical_layout::Rotation::from_centidegrees(#rotation),
            }
        }
    });
    let len = layout.keys.len();

    quote! {
        pub static PHYSICAL_KEYS: [::rmk::physical_layout::PhysicalKey; #len] = [#(#keys),*];
        pub const PHYSICAL_LAYOUT: ::rmk::physical_layout::PhysicalLayout<'static> =
            ::rmk::physical_layout::PhysicalLayout::new(&PHYSICAL_KEYS);
    }
}

/// Generate the flash-resident topology, routing, and built-in semantic
/// lighting configuration for a resolved `[lighting]` section.
pub(crate) fn expand_lighting_topology(lighting: Option<&Lighting>) -> TokenStream2 {
    let Some(lighting) = lighting else {
        return TokenStream2::new();
    };
    let revision = lighting.topology_revision;
    let [rows, cols] = lighting.matrix;
    let led_count = lighting.emitters.len();

    let keys = lighting.keys.iter().map(|key| {
        let [row, col] = key.matrix;
        quote! { ::rmk::lighting::topology::MatrixPosition::new(#row, #col) }
    });
    let key_count = lighting.keys.len();
    let zones = lighting.zones.iter().map(|zone| {
        let id = zone.id;
        let name = &zone.name;
        quote! {
            ::rmk::lighting::topology::ZoneMetadata {
                id: ::rmk::lighting::topology::ZoneId(#id),
                name: #name,
            }
        }
    });
    let zone_count = lighting.zones.len();
    let emitters = lighting.emitters.iter().map(|emitter| {
        let id = emitter.id;
        let key = match emitter.key {
            Some([row, col]) => quote! {
                ::core::option::Option::Some(::rmk::lighting::topology::MatrixPosition::new(#row, #col))
            },
            None => quote! { ::core::option::Option::None },
        };
        let position = match emitter.position {
            Some(point) => {
                let point = expand_point(point);
                quote! { ::core::option::Option::Some(#point) }
            }
            None => quote! { ::core::option::Option::None },
        };
        let zone_start = emitter.zone_start;
        let zone_len = emitter.zone_len;
        quote! {
            ::rmk::lighting::topology::LedMetadata {
                id: ::rmk::lighting::topology::LedId(#id),
                key: #key,
                position: #position,
                zones: ::rmk::lighting::topology::ZoneSpan::new(#zone_start, #zone_len),
            }
        }
    });
    let memberships = lighting.zone_memberships.iter().map(|id| {
        quote! { ::rmk::lighting::topology::ZoneId(#id) }
    });
    let membership_count = lighting.zone_memberships.len();
    let outputs = lighting.outputs.iter().map(|output| {
        let node = output.node;
        let id = output.id;
        let pixel_count = output.pixel_count;
        let capabilities = output.capabilities;
        let coverage = if output.sparse {
            quote! { ::rmk::lighting::topology::OutputCoverage::Sparse }
        } else {
            quote! { ::rmk::lighting::topology::OutputCoverage::Complete }
        };
        quote! {
            ::rmk::lighting::topology::OutputMetadata {
                node: ::rmk::lighting::topology::LightingNodeId(#node),
                id: ::rmk::lighting::topology::OutputId(#id),
                pixel_count: #pixel_count,
                capabilities: ::rmk::lighting::topology::OutputCapabilities::from_bits(#capabilities)
                    .expect("rmk-config emitted validated output capabilities"),
                coverage: #coverage,
            }
        }
    });
    let output_count = lighting.outputs.len();
    let routes = lighting.routes.iter().map(|route| {
        let slot = route.slot;
        let node = route.node;
        let output = route.output;
        let physical_index = route.physical_index;
        quote! {
            ::rmk::lighting::topology::PhysicalRoute {
                slot: ::rmk::lighting::topology::LedSlot(#slot),
                node: ::rmk::lighting::topology::LightingNodeId(#node),
                output: ::rmk::lighting::topology::OutputId(#output),
                physical_index: #physical_index,
            }
        }
    });
    let route_count = lighting.routes.len();
    let renderer_config = expand_lighting_renderer_config(Some(lighting));

    quote! {
        pub const LIGHTING_TOPOLOGY_REVISION: u32 = #revision;
        pub const LIGHTING_LED_COUNT: usize = #led_count;
        pub static LIGHTING_KEYS: [::rmk::lighting::topology::MatrixPosition; #key_count] = [#(#keys),*];
        pub static LIGHTING_ZONES: [::rmk::lighting::topology::ZoneMetadata<'static>; #zone_count] = [#(#zones),*];
        pub static LIGHTING_EMITTERS: [::rmk::lighting::topology::LedMetadata; #led_count] = [#(#emitters),*];
        pub static LIGHTING_ZONE_MEMBERSHIPS: [::rmk::lighting::topology::ZoneId; #membership_count] = [#(#memberships),*];
        pub static LIGHTING_OUTPUTS: [::rmk::lighting::topology::OutputMetadata; #output_count] = [#(#outputs),*];
        pub static LIGHTING_ROUTES: [::rmk::lighting::topology::PhysicalRoute; #route_count] = [#(#routes),*];
        pub const LIGHTING_TOPOLOGY: ::rmk::lighting::topology::LightingTopology<'static> =
            ::rmk::lighting::topology::LightingTopology {
                matrix: ::rmk::lighting::topology::MatrixSize::new(#rows, #cols),
                keys: &LIGHTING_KEYS,
                physical_layout: PHYSICAL_LAYOUT,
                leds: &LIGHTING_EMITTERS,
                zones: &LIGHTING_ZONES,
                zone_memberships: &LIGHTING_ZONE_MEMBERSHIPS,
            };
        pub const LIGHTING_ROUTING: ::rmk::lighting::topology::LightingRouting<'static> =
            ::rmk::lighting::topology::LightingRouting {
                outputs: &LIGHTING_OUTPUTS,
                routes: &LIGHTING_ROUTES,
            };

        #renderer_config
    }
}

/// Generate the semantic configuration required by a local renderer. Split
/// peripherals do not need the central's host-facing topology and routing,
/// but they do need the same built-in scenes and background configuration.
pub(crate) fn expand_lighting_renderer_config(lighting: Option<&Lighting>) -> TokenStream2 {
    let Some(lighting) = lighting else {
        return TokenStream2::new();
    };
    let layer_scene_cells = lighting.layer_scenes.iter().enumerate().map(|(index, scene)| {
        let name = quote::format_ident!("LIGHTING_LAYER_SCENE_{index}_CELLS");
        let cells = scene.cells.iter().map(expand_scene_cell);
        let len = scene.cells.len();
        quote! {
            pub static #name: [::rmk::lighting::SceneCell<::rmk::lighting::BuiltinEffect>; #len] =
                [#(#cells),*];
        }
    });
    let layer_scene_table = lighting
        .layer_scenes
        .iter()
        .enumerate()
        .map(|(index, scene)| {
            let name = quote::format_ident!("LIGHTING_LAYER_SCENE_{index}_CELLS");
            let layer = scene.layer;
            quote! {
                ::rmk::lighting::LayerScene {
                    layer: #layer,
                    cells: &#name,
                }
            }
        });
    let layer_scene_count = lighting.layer_scenes.len();
    let conditional_cells = lighting
        .conditional_scene_cells
        .iter()
        .map(expand_conditional_scene_cell);
    let conditional_cell_count = lighting.conditional_scene_cells.len();
    let output_toggle_user_action = match lighting.controls.output_toggle_user_action {
        Some(action) => quote! { Some(#action) },
        None => quote! { None },
    };
    let output_mode_cycle_user_action = match lighting.controls.output_mode_cycle_user_action {
        Some(action) => quote! { Some(#action) },
        None => quote! { None },
    };
    let wake_layer = match lighting.controls.wake_layer {
        Some(layer) => quote! { Some(#layer) },
        None => quote! { None },
    };
    let initial_output_mode = match lighting.controls.initial_output_mode {
        LightingOutputMode::AlwaysOn => quote! { ::rmk::lighting::OutputMode::AlwaysOn },
        LightingOutputMode::AlwaysOff => quote! { ::rmk::lighting::OutputMode::AlwaysOff },
        LightingOutputMode::PoweredOnly => quote! { ::rmk::lighting::OutputMode::PoweredOnly },
    };
    let output_mode_indicator = match lighting.controls.output_mode_indicator {
        Some(indicator) => {
            let slot = indicator.slot;
            let always_on = expand_effect(indicator.always_on);
            let always_off = expand_effect(indicator.always_off);
            let powered_only = expand_effect(indicator.powered_only);
            quote! {
                Some(::rmk::lighting::OutputModeIndicator {
                    slot: ::rmk::lighting::LedSlot(#slot),
                    always_on: #always_on,
                    always_off: #always_off,
                    powered_only: #powered_only,
                })
            }
        }
        None => quote! { None },
    };
    let background = &lighting.background;
    let background_enabled = background.enabled;
    let background_hue = background.hue;
    let background_saturation = background.saturation;
    let background_value = background.value;
    let background_speed = background.speed;
    let background_mode = match background.mode {
        LightingBackgroundMode::Solid => quote! { ::rmk::lighting::BackgroundMode::Solid },
        LightingBackgroundMode::Breathe => quote! { ::rmk::lighting::BackgroundMode::Breathe },
    };

    quote! {
        #(#layer_scene_cells)*
        pub static LIGHTING_LAYER_SCENE_TABLE:
            [::rmk::lighting::LayerScene<'static, ::rmk::lighting::BuiltinEffect>; #layer_scene_count] =
            [#(#layer_scene_table),*];
        pub const LIGHTING_LAYER_SCENES:
            ::rmk::lighting::LayerScenes<'static, ::rmk::lighting::BuiltinEffect> =
            ::rmk::lighting::LayerScenes {
                scenes: &LIGHTING_LAYER_SCENE_TABLE,
                policy: ::rmk::lighting::LayerPolicy::ActiveStack,
            };
        pub static LIGHTING_CONDITIONAL_SCENE_CELLS:
            [::rmk::lighting::ConditionalSceneCell<::rmk::lighting::BuiltinEffect>; #conditional_cell_count] =
            [#(#conditional_cells),*];
        pub const LIGHTING_CONTROLS: ::rmk::lighting::LightingControls =
            ::rmk::lighting::LightingControls {
                output_toggle_user_action: #output_toggle_user_action,
                output_mode_cycle_user_action: #output_mode_cycle_user_action,
                wake_layer: #wake_layer,
                initial_output_mode: #initial_output_mode,
                output_mode_indicator: #output_mode_indicator,
            };
        pub const LIGHTING_BACKGROUND: ::rmk::lighting::BackgroundState =
            ::rmk::lighting::BackgroundState {
                enabled: #background_enabled,
                hue: #background_hue,
                saturation: #background_saturation,
                value: #background_value,
                speed: #background_speed,
                mode: #background_mode,
            };
    }
}

fn expand_conditional_scene_cell(cell: &LightingConditionalSceneCell) -> TokenStream2 {
    let slot = cell.slot;
    let effect = expand_effect(cell.effect);
    let layer = match cell.conditions.layer {
        Some(condition) => {
            let layer = condition.layer;
            let active = condition.active;
            quote! {
                ::core::option::Option::Some(::rmk::lighting::LayerCondition {
                    layer: #layer,
                    active: #active,
                })
            }
        }
        None => quote! { ::core::option::Option::None },
    };
    let battery = match cell.conditions.battery {
        Some(condition) => {
            let node = condition.node;
            let min_level = expand_option_u8(condition.min_level);
            let max_level = expand_option_u8(condition.max_level);
            let charge = match condition.charge {
                LightingChargeCondition::Any => quote! { ::rmk::lighting::ChargeCondition::Any },
                LightingChargeCondition::Charging => {
                    quote! { ::rmk::lighting::ChargeCondition::Charging }
                }
                LightingChargeCondition::Discharging => {
                    quote! { ::rmk::lighting::ChargeCondition::Discharging }
                }
                LightingChargeCondition::Unknown => {
                    quote! { ::rmk::lighting::ChargeCondition::Unknown }
                }
            };
            quote! {
                ::core::option::Option::Some(::rmk::lighting::BatteryCondition {
                    node: #node,
                    min_level: #min_level,
                    max_level: #max_level,
                    charge: #charge,
                })
            }
        }
        None => quote! { ::core::option::Option::None },
    };
    quote! {
        ::rmk::lighting::ConditionalSceneCell {
            conditions: ::rmk::lighting::ConditionSet {
                layer: #layer,
                battery: #battery,
            },
            slot: ::rmk::lighting::LedSlot(#slot),
            effect: #effect,
        }
    }
}

fn expand_option_u8(value: Option<u8>) -> TokenStream2 {
    match value {
        Some(value) => quote! { ::core::option::Option::Some(#value) },
        None => quote! { ::core::option::Option::None },
    }
}

fn expand_scene_cell(cell: &LightingSceneCell) -> TokenStream2 {
    let slot = cell.slot;
    let effect = expand_effect(cell.effect);
    quote! {
        ::rmk::lighting::SceneCell {
            slot: ::rmk::lighting::topology::LedSlot(#slot),
            effect: #effect,
        }
    }
}

fn expand_effect(effect: LightingEffect) -> TokenStream2 {
    match effect {
        LightingEffect::Solid { color } => {
            let [r, g, b] = color;
            quote! {
                ::rmk::lighting::BuiltinEffect::Solid {
                    color: ::rmk::lighting::Rgb8::new(#r, #g, #b),
                }
            }
        }
        LightingEffect::Blink {
            color,
            period_ms,
            phase_ms,
            duty_percent,
        } => {
            let [r, g, b] = color;
            quote! {
                ::rmk::lighting::BuiltinEffect::Blink {
                    color: ::rmk::lighting::Rgb8::new(#r, #g, #b),
                    period_ms: #period_ms,
                    phase_ms: #phase_ms,
                    duty: #duty_percent,
                }
            }
        }
        LightingEffect::Breathe {
            color,
            period_ms,
            phase_ms,
            step_ms,
        } => {
            let [r, g, b] = color;
            quote! {
                ::rmk::lighting::BuiltinEffect::Breathe {
                    color: ::rmk::lighting::Rgb8::new(#r, #g, #b),
                    period_ms: #period_ms,
                    phase_ms: #phase_ms,
                    step_ms: #step_ms,
                }
            }
        }
    }
}

fn expand_point(point: FixedPoint3) -> TokenStream2 {
    let x = point.x;
    let y = point.y;
    let z = point.z;
    quote! {
        ::rmk::physical_layout::Point3::new(
            ::rmk::physical_layout::Coordinate::from_raw(#x),
            ::rmk::physical_layout::Coordinate::from_raw(#y),
            ::rmk::physical_layout::Coordinate::from_raw(#z),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmk_config::resolved::lighting::{
        LightingBackground, LightingEmitter, LightingKey, LightingLayerScene, LightingOutput,
        LightingRoute, LightingSceneCell, LightingZone,
    };
    use rmk_config::resolved::{FixedSize2, PhysicalKey};

    #[test]
    fn emits_shared_geometry_and_topology_without_electrical_order_leaking_into_ids() {
        let physical = PhysicalLayout {
            keys: vec![PhysicalKey {
                matrix: [0, 0],
                center: FixedPoint3 {
                    x: -128,
                    y: 256,
                    z: 0,
                },
                size: FixedSize2 {
                    width: 384,
                    height: 256,
                },
                rotation_centidegrees: -750,
            }],
        };
        let geometry = expand_physical_layout(&physical).to_string();
        assert!(geometry.contains("PHYSICAL_LAYOUT"));
        assert!(geometry.contains("from_raw (- 128i16)"));

        let lighting = Lighting {
            topology_revision: 4,
            matrix: [1, 1],
            keys: vec![LightingKey { matrix: [0, 0] }],
            zones: vec![LightingZone {
                id: 1,
                name: "keys".into(),
            }],
            emitters: vec![LightingEmitter {
                id: 42,
                key: Some([0, 0]),
                position: None,
                zone_start: 0,
                zone_len: 1,
            }],
            zone_memberships: vec![1],
            outputs: vec![LightingOutput {
                node: 2,
                id: 3,
                pixel_count: 2,
                capabilities: 0b10100,
                sparse: true,
            }],
            routes: vec![LightingRoute {
                slot: 0,
                node: 2,
                output: 3,
                physical_index: 1,
            }],
            layer_scenes: vec![
                LightingLayerScene {
                    layer: 0,
                    cells: vec![LightingSceneCell {
                        slot: 0,
                        effect: LightingEffect::Solid { color: [1, 2, 3] },
                    }],
                },
                LightingLayerScene {
                    layer: 1,
                    cells: vec![
                        LightingSceneCell {
                            slot: 0,
                            effect: LightingEffect::Blink {
                                color: [4, 5, 6],
                                period_ms: 1000,
                                phase_ms: 250,
                                duty_percent: 40,
                            },
                        },
                        LightingSceneCell {
                            slot: 0,
                            effect: LightingEffect::Breathe {
                                color: [7, 8, 9],
                                period_ms: 2000,
                                phase_ms: 500,
                                step_ms: 20,
                            },
                        },
                    ],
                },
            ],
            background: LightingBackground {
                enabled: false,
                hue: 11,
                saturation: 22,
                value: 33,
                speed: 44,
                mode: LightingBackgroundMode::Breathe,
            },
        };
        let topology = expand_lighting_topology(Some(&lighting)).to_string();
        assert!(topology.contains("LedId (42u16)"));
        assert!(topology.contains("physical_index : 1u16"));
        assert!(topology.contains("physical_layout : PHYSICAL_LAYOUT"));
        assert!(topology.contains("LIGHTING_LAYER_SCENE_0_CELLS"));
        assert!(topology.contains("LIGHTING_LAYER_SCENE_1_CELLS"));
        assert!(topology.contains("LIGHTING_LAYER_SCENE_TABLE"));
        assert!(topology.contains("LIGHTING_LAYER_SCENES"));
        assert!(topology.contains("LayerPolicy :: ActiveStack"));
        assert!(topology.contains("BuiltinEffect :: Solid"));
        assert!(topology.contains("BuiltinEffect :: Blink"));
        assert!(topology.contains("duty : 40u8"));
        assert!(topology.contains("BuiltinEffect :: Breathe"));
        assert!(topology.contains("step_ms : 20u16"));
        assert!(topology.contains("LIGHTING_BACKGROUND"));
        assert!(topology.contains("enabled : false"));
        assert!(topology.contains("hue : 11u8"));
        assert!(topology.contains("mode : :: rmk :: lighting :: BackgroundMode :: Breathe"));

        let renderer = expand_lighting_renderer_config(Some(&lighting)).to_string();
        assert!(renderer.contains("LIGHTING_LAYER_SCENES"));
        assert!(renderer.contains("LIGHTING_BACKGROUND"));
        assert!(!renderer.contains("LIGHTING_TOPOLOGY"));
        assert!(!renderer.contains("LIGHTING_ROUTING"));
    }

    #[test]
    fn omits_all_lighting_symbols_without_resolved_lighting() {
        assert!(expand_lighting_topology(None).is_empty());
        assert!(expand_lighting_renderer_config(None).is_empty());
    }
}
