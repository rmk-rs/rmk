//! Generate flash-resident shared geometry and semantic lighting topology.

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use rmk_config::resolved::lighting::{
    Lighting, LightingBackgroundMode, LightingChargeCondition, LightingConditionalSceneCell,
    LightingEffect, LightingOutputMode, LightingPoweredOnlyScope, LightingSceneCell,
};
use rmk_config::resolved::{FixedPoint3, PhysicalLayout};

/// Statics for a hand-written main: resolve `[layout]` and `[lighting]`
/// directly from `KEYBOARD_TOML_PATH` without the full `#[rmk_keyboard]`
/// pipeline (which would require a `[matrix]` or `[split]` section).
pub(crate) fn expand_standalone_lighting_config() -> TokenStream2 {
    // Load without the chip-default layer: it requires `[keyboard].chip` and
    // only fills in sections ([storage], [ble], ...) that a hand-written main
    // configures in Rust. rmk-types' build.rs treats such tomls the same way.
    let config_toml_path = std::env::var("KEYBOARD_TOML_PATH")
        .expect("[ERROR]: KEYBOARD_TOML_PATH should be set in `.cargo/config.toml`");
    let config =
        rmk_config::KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&config_toml_path);
    let layout = config
        .layout_standalone()
        .expect("failed to resolve layout config");
    let lighting = config
        .lighting_standalone(&layout)
        .expect("failed to resolve lighting config");
    let physical_layout = expand_physical_layout(&layout.physical);
    let topology = expand_lighting_topology(lighting.as_ref());
    let blob_lit = proc_macro2::Literal::byte_string(&layout.blob);
    quote! {
        #physical_layout
        #topology
        pub static LAYOUT_BLOB: &[u8] = #blob_lit;
    }
}

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
    let powered_only_scope = match lighting.controls.powered_only_scope {
        LightingPoweredOnlyScope::Authority => {
            quote! { ::rmk::lighting::PoweredOnlyScope::Authority }
        }
        LightingPoweredOnlyScope::Local => quote! { ::rmk::lighting::PoweredOnlyScope::Local },
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
                powered_only_scope: #powered_only_scope,
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
            conditional_scene_cells: Vec::new(),
            controls: Default::default(),
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

    // Exact copy of rmk-zsa-voyager/keyboard.toml: a board-wide 12x7 layout
    // with 52 emitters split across two IS31FL3731 outputs on a single node.
    const VOYAGER_TOML: &str = r#"# Consumed at build time by rmk-types (event channel sizing) and by the
# `rmk_lighting_config!` macro in main.rs (physical layout + `[lighting]`
# statics). The scan hardware and keymap stay in Rust: the left half is a
# direct-GPIO matrix and the right half arrives over an MCP23018, which the
# `#[rmk_keyboard]` generated main cannot express.

# The logical 12x7 matrix. Rows 0-5 are the left half, rows 6-11 the right
# half; see src/keymap.rs for the physical wiring behind this arrangement.
[layout]
rows = 12
cols = 7
map = """
(0,1) (0,2) (0,3) (0,4) (0,5) (0,6)
(1,1) (1,2) (1,3) (1,4) (1,5) (1,6)
(2,1) (2,2) (2,3) (2,4) (2,5) (2,6)
(3,1) (3,2) (3,3) (3,4) (3,5)
(4,4)
(5,0) (5,1)
(6,0) (6,1) (6,2) (6,3) (6,4) (6,5)
(7,0) (7,1) (7,2) (7,3) (7,4) (7,5)
(8,0) (8,1) (8,2) (8,3) (8,4) (8,5)
(9,1) (9,2) (9,3) (9,4) (9,5)
(10,2)
(11,5) (11,6)
"""

# Layer count only; the default keymap itself lives in src/keymap.rs.
[keymap]
layers = 3

# Board-wide lighting topology: one node (the Voyager is not an RMK split),
# two IS31FL3731 chips as separate outputs. Emitter ids 0-25 are the left
# chip and 26-51 the right chip, in `LED_TABLE` order (src/is31fl3731.rs);
# `physical_index` is the chip-relative index into that table.
[lighting]
topology_revision = 1

[[lighting.zone]]
id = 1
name = "per-key"

[[lighting.output]]
node = 0
id = 0
pixel_count = 26
capabilities = ["rgb", "addressable"]

[[lighting.output]]
node = 0
id = 1
pixel_count = 26
capabilities = ["rgb", "addressable"]

# The animated base-layer background comes from the rmk-palettefx extension
# source, not the uniform background.
[lighting.background]
enabled = false
hue = 0
saturation = 0
value = 0
speed = 128
mode = "solid"

[lighting.controls]
initial_output_mode = "always_on"

# Non-base layers replace the animation with a solid wash, exactly covering
# every emitter so the extension band sleeps while a layer is held.
[[lighting.layer_scene]]
layer = 1

[[lighting.layer_scene.cell]]
target = { all = true }
effect = { kind = "solid", color = [0, 16, 64] } # symbols/F-keys: cool blue

[[lighting.layer_scene]]
layer = 2

[[lighting.layer_scene.cell]]
target = { all = true }
effect = { kind = "solid", color = [48, 0, 48] } # media/nav: magenta

# Left chip (0x74), LED_TABLE entries 0-25.
[[lighting.emitter]]
id = 0
key = [0, 1]
zones = [1]
node = 0
output = 0
physical_index = 0

[[lighting.emitter]]
id = 1
key = [0, 2]
zones = [1]
node = 0
output = 0
physical_index = 1

[[lighting.emitter]]
id = 2
key = [0, 3]
zones = [1]
node = 0
output = 0
physical_index = 2

[[lighting.emitter]]
id = 3
key = [0, 4]
zones = [1]
node = 0
output = 0
physical_index = 3

[[lighting.emitter]]
id = 4
key = [0, 5]
zones = [1]
node = 0
output = 0
physical_index = 4

[[lighting.emitter]]
id = 5
key = [0, 6]
zones = [1]
node = 0
output = 0
physical_index = 5

[[lighting.emitter]]
id = 6
key = [1, 1]
zones = [1]
node = 0
output = 0
physical_index = 6

[[lighting.emitter]]
id = 7
key = [1, 2]
zones = [1]
node = 0
output = 0
physical_index = 7

[[lighting.emitter]]
id = 8
key = [1, 3]
zones = [1]
node = 0
output = 0
physical_index = 8

[[lighting.emitter]]
id = 9
key = [1, 4]
zones = [1]
node = 0
output = 0
physical_index = 9

[[lighting.emitter]]
id = 10
key = [1, 5]
zones = [1]
node = 0
output = 0
physical_index = 10

[[lighting.emitter]]
id = 11
key = [1, 6]
zones = [1]
node = 0
output = 0
physical_index = 11

[[lighting.emitter]]
id = 12
key = [2, 1]
zones = [1]
node = 0
output = 0
physical_index = 12

[[lighting.emitter]]
id = 13
key = [2, 2]
zones = [1]
node = 0
output = 0
physical_index = 13

[[lighting.emitter]]
id = 14
key = [2, 3]
zones = [1]
node = 0
output = 0
physical_index = 14

[[lighting.emitter]]
id = 15
key = [2, 4]
zones = [1]
node = 0
output = 0
physical_index = 15

[[lighting.emitter]]
id = 16
key = [2, 5]
zones = [1]
node = 0
output = 0
physical_index = 16

[[lighting.emitter]]
id = 17
key = [2, 6]
zones = [1]
node = 0
output = 0
physical_index = 17

[[lighting.emitter]]
id = 18
key = [3, 1]
zones = [1]
node = 0
output = 0
physical_index = 18

[[lighting.emitter]]
id = 19
key = [3, 2]
zones = [1]
node = 0
output = 0
physical_index = 19

[[lighting.emitter]]
id = 20
key = [3, 3]
zones = [1]
node = 0
output = 0
physical_index = 20

[[lighting.emitter]]
id = 21
key = [3, 4]
zones = [1]
node = 0
output = 0
physical_index = 21

[[lighting.emitter]]
id = 22
key = [3, 5]
zones = [1]
node = 0
output = 0
physical_index = 22

[[lighting.emitter]]
id = 23
key = [4, 4]
zones = [1]
node = 0
output = 0
physical_index = 23

[[lighting.emitter]]
id = 24
key = [5, 0]
zones = [1]
node = 0
output = 0
physical_index = 24

[[lighting.emitter]]
id = 25
key = [5, 1]
zones = [1]
node = 0
output = 0
physical_index = 25

# Right chip (0x77), LED_TABLE entries 26-51.
[[lighting.emitter]]
id = 26
key = [6, 0]
zones = [1]
node = 0
output = 1
physical_index = 0

[[lighting.emitter]]
id = 27
key = [6, 1]
zones = [1]
node = 0
output = 1
physical_index = 1

[[lighting.emitter]]
id = 28
key = [6, 2]
zones = [1]
node = 0
output = 1
physical_index = 2

[[lighting.emitter]]
id = 29
key = [6, 3]
zones = [1]
node = 0
output = 1
physical_index = 3

[[lighting.emitter]]
id = 30
key = [6, 4]
zones = [1]
node = 0
output = 1
physical_index = 4

[[lighting.emitter]]
id = 31
key = [6, 5]
zones = [1]
node = 0
output = 1
physical_index = 5

[[lighting.emitter]]
id = 32
key = [7, 0]
zones = [1]
node = 0
output = 1
physical_index = 6

[[lighting.emitter]]
id = 33
key = [7, 1]
zones = [1]
node = 0
output = 1
physical_index = 7

[[lighting.emitter]]
id = 34
key = [7, 2]
zones = [1]
node = 0
output = 1
physical_index = 8

[[lighting.emitter]]
id = 35
key = [7, 3]
zones = [1]
node = 0
output = 1
physical_index = 9

[[lighting.emitter]]
id = 36
key = [7, 4]
zones = [1]
node = 0
output = 1
physical_index = 10

[[lighting.emitter]]
id = 37
key = [7, 5]
zones = [1]
node = 0
output = 1
physical_index = 11

[[lighting.emitter]]
id = 38
key = [8, 0]
zones = [1]
node = 0
output = 1
physical_index = 12

[[lighting.emitter]]
id = 39
key = [8, 1]
zones = [1]
node = 0
output = 1
physical_index = 13

[[lighting.emitter]]
id = 40
key = [8, 2]
zones = [1]
node = 0
output = 1
physical_index = 14

[[lighting.emitter]]
id = 41
key = [8, 3]
zones = [1]
node = 0
output = 1
physical_index = 15

[[lighting.emitter]]
id = 42
key = [8, 4]
zones = [1]
node = 0
output = 1
physical_index = 16

[[lighting.emitter]]
id = 43
key = [8, 5]
zones = [1]
node = 0
output = 1
physical_index = 17

[[lighting.emitter]]
id = 44
key = [10, 2]
zones = [1]
node = 0
output = 1
physical_index = 18

[[lighting.emitter]]
id = 45
key = [9, 1]
zones = [1]
node = 0
output = 1
physical_index = 19

[[lighting.emitter]]
id = 46
key = [9, 2]
zones = [1]
node = 0
output = 1
physical_index = 20

[[lighting.emitter]]
id = 47
key = [9, 3]
zones = [1]
node = 0
output = 1
physical_index = 21

[[lighting.emitter]]
id = 48
key = [9, 4]
zones = [1]
node = 0
output = 1
physical_index = 22

[[lighting.emitter]]
id = 49
key = [9, 5]
zones = [1]
node = 0
output = 1
physical_index = 23

[[lighting.emitter]]
id = 50
key = [11, 5]
zones = [1]
node = 0
output = 1
physical_index = 24

[[lighting.emitter]]
id = 51
key = [11, 6]
zones = [1]
node = 0
output = 1
physical_index = 25

# Event channel sizing beyond the defaults: the status-LED task and the
# lighting processor both subscribe to layer changes.
[event.layer_change]
subs = 2
"#;

    #[test]
    fn resolves_and_expands_the_voyager_keyboard_toml() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rmk-macro-voyager-{}-{unique}.toml",
            std::process::id()
        ));
        std::fs::write(&path, VOYAGER_TOML).unwrap();
        let config = rmk_config::KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path);
        let _ = std::fs::remove_file(&path);

        let layout = config.layout_standalone().unwrap();
        assert_eq!(layout.keys.len(), 52);
        let lighting = config.lighting_standalone(&layout).unwrap().unwrap();
        assert_eq!(lighting.emitters.len(), 52);
        assert_eq!(lighting.outputs.len(), 2);
        assert_eq!(lighting.routes.len(), 52);
        let scenes: Vec<_> = lighting
            .layer_scenes
            .iter()
            .map(|scene| (scene.layer, scene.cells.len()))
            .collect();
        assert_eq!(
            scenes,
            vec![(1, 52), (2, 52)],
            "target {{ all = true }} must expand to every emitter slot"
        );

        let geometry = expand_physical_layout(&layout.physical).to_string();
        assert!(geometry.contains("PHYSICAL_LAYOUT"));

        let topology = expand_lighting_topology(Some(&lighting)).to_string();
        assert!(topology.contains("LIGHTING_LED_COUNT : usize = 52usize"));
        for symbol in [
            "LIGHTING_TOPOLOGY",
            "LIGHTING_ROUTING",
            "LIGHTING_LAYER_SCENES",
            "LIGHTING_CONTROLS",
            "LIGHTING_BACKGROUND",
        ] {
            assert!(topology.contains(symbol), "missing {symbol}");
        }
    }
}
