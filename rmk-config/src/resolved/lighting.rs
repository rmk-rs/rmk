use std::collections::{HashMap, HashSet};

use super::Keymap;
use super::layout::{FixedPoint3, Layout};
use crate::{LightingBackgroundModeToml, LightingEffectTomlConfig, LightingTargetTomlConfig};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LightingKey {
    pub matrix: [u8; 2],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LightingZone {
    pub id: u8,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LightingEmitter {
    pub id: u16,
    pub key: Option<[u8; 2]>,
    pub position: Option<FixedPoint3>,
    pub zone_start: u16,
    pub zone_len: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LightingOutput {
    pub node: u8,
    pub id: u8,
    pub pixel_count: u16,
    pub capabilities: u8,
    pub sparse: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LightingRoute {
    pub slot: u16,
    pub node: u8,
    pub output: u8,
    pub physical_index: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LightingEffect {
    Solid {
        color: [u8; 3],
    },
    Blink {
        color: [u8; 3],
        period_ms: u32,
        phase_ms: u32,
        duty_percent: u8,
    },
    Breathe {
        color: [u8; 3],
        period_ms: u32,
        phase_ms: u32,
        step_ms: u16,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LightingSceneCell {
    pub slot: u16,
    pub effect: LightingEffect,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LightingLayerScene {
    pub layer: u8,
    pub cells: Vec<LightingSceneCell>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LightingBackgroundMode {
    Solid,
    Breathe,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LightingBackground {
    pub enabled: bool,
    pub hue: u8,
    pub saturation: u8,
    pub value: u8,
    pub speed: u8,
    pub mode: LightingBackgroundMode,
}

impl Default for LightingBackground {
    fn default() -> Self {
        Self {
            enabled: true,
            hue: 0,
            saturation: 0,
            value: 32,
            speed: 128,
            mode: LightingBackgroundMode::Solid,
        }
    }
}

/// Fully validated build-time lighting data. Key identities and fallback key
/// geometry come from the already-resolved `[layout].map`; emitters and routes
/// add semantic and electrical topology without redefining the board layout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Lighting {
    pub topology_revision: u32,
    pub matrix: [u8; 2],
    pub keys: Vec<LightingKey>,
    pub zones: Vec<LightingZone>,
    pub emitters: Vec<LightingEmitter>,
    pub zone_memberships: Vec<u8>,
    pub outputs: Vec<LightingOutput>,
    pub routes: Vec<LightingRoute>,
    pub layer_scenes: Vec<LightingLayerScene>,
    pub background: LightingBackground,
}

impl crate::KeyboardTomlConfig {
    pub fn lighting(&self, layout: &Layout, keymap: &Keymap) -> Result<Option<Lighting>, String> {
        let Some(config) = &self.lighting else {
            return Ok(None);
        };
        if layout.keys.is_empty() {
            return Err("[lighting] requires `[layout].map` as the canonical logical key layout".into());
        }
        if config.emitters.is_empty() {
            return Err("[lighting] must define at least one [[lighting.emitter]]".into());
        }
        if config.emitters.len() > u16::MAX as usize {
            return Err("[lighting] emitter count exceeds LedSlot u16 capacity".into());
        }

        let zones = resolve_zones(&config.zones)?;
        let zone_ids: HashSet<u8> = zones.iter().map(|zone| zone.id).collect();
        let mut emitter_ids = HashSet::new();
        let mut zone_memberships = Vec::new();
        let mut emitters = Vec::with_capacity(config.emitters.len());
        let mut routes = Vec::with_capacity(config.emitters.len());
        for (slot, emitter) in config.emitters.iter().enumerate() {
            if !emitter_ids.insert(emitter.id) {
                return Err(format!("duplicate lighting emitter id {}", emitter.id));
            }
            if let Some(key) = emitter.key
                && !layout.keys.contains(&key)
            {
                return Err(format!(
                    "lighting emitter {} key [{}, {}] is not a logical key in layout.map",
                    emitter.id, key[0], key[1]
                ));
            }
            let mut local_zones = HashSet::new();
            for zone in &emitter.zones {
                if !zone_ids.contains(zone) {
                    return Err(format!(
                        "lighting emitter {} references unknown zone {zone}",
                        emitter.id
                    ));
                }
                if !local_zones.insert(*zone) {
                    return Err(format!("lighting emitter {} repeats zone {zone}", emitter.id));
                }
            }
            if emitter.zones.len() > u8::MAX as usize
                || zone_memberships.len() + emitter.zones.len() > u16::MAX as usize
            {
                return Err("lighting zone membership table exceeds bounded representation".into());
            }
            let zone_start = zone_memberships.len() as u16;
            zone_memberships.extend_from_slice(&emitter.zones);
            emitters.push(LightingEmitter {
                id: emitter.id,
                key: emitter.key,
                position: emitter
                    .position
                    .map(to_fixed_point)
                    .transpose()
                    .map_err(str::to_owned)?,
                zone_start,
                zone_len: emitter.zones.len() as u8,
            });
            routes.push(LightingRoute {
                slot: slot as u16,
                node: emitter.node,
                output: emitter.output,
                physical_index: emitter.physical_index,
            });
        }

        let outputs = resolve_outputs(&config.outputs)?;
        validate_routes(&outputs, &routes)?;
        let layer_scenes = resolve_layer_scenes(
            keymap.layers,
            &config.layer_scenes,
            &emitters,
            &zone_memberships,
            &zone_ids,
        )?;
        let background = config
            .background
            .as_ref()
            .map(|background| LightingBackground {
                enabled: background.enabled,
                hue: background.hue,
                saturation: background.saturation,
                value: background.value,
                speed: background.speed,
                mode: match background.mode {
                    LightingBackgroundModeToml::Solid => LightingBackgroundMode::Solid,
                    LightingBackgroundModeToml::Breathe => LightingBackgroundMode::Breathe,
                },
            })
            .unwrap_or_default();

        Ok(Some(Lighting {
            topology_revision: config.topology_revision,
            matrix: [layout.rows, layout.cols],
            keys: layout
                .keys
                .iter()
                .copied()
                .map(|matrix| LightingKey { matrix })
                .collect(),
            zones,
            emitters,
            zone_memberships,
            outputs,
            routes,
            layer_scenes,
            background,
        }))
    }
}

fn to_fixed_point(point: [f32; 3]) -> Result<FixedPoint3, &'static str> {
    fn axis(value: f32) -> Result<i16, &'static str> {
        if !value.is_finite() {
            return Err("must contain finite coordinates");
        }
        let raw = (value * 256.0).round();
        if raw < i16::MIN as f32 || raw > i16::MAX as f32 {
            return Err("does not fit signed Q8.8 key-pitch units");
        }
        Ok(raw as i16)
    }
    Ok(FixedPoint3 {
        x: axis(point[0])?,
        y: axis(point[1])?,
        z: axis(point[2])?,
    })
}

fn resolve_zones(config: &[crate::LightingZoneTomlConfig]) -> Result<Vec<LightingZone>, String> {
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    config
        .iter()
        .map(|zone| {
            if !ids.insert(zone.id) {
                return Err(format!("duplicate lighting zone id {}", zone.id));
            }
            if zone.name.is_empty() || !names.insert(zone.name.clone()) {
                return Err(format!("duplicate or empty lighting zone name {:?}", zone.name));
            }
            Ok(LightingZone {
                id: zone.id,
                name: zone.name.clone(),
            })
        })
        .collect()
}

fn resolve_outputs(config: &[crate::LightingOutputTomlConfig]) -> Result<Vec<LightingOutput>, String> {
    let mut ids = HashSet::new();
    config
        .iter()
        .map(|output| {
            if !ids.insert((output.node, output.id)) {
                return Err(format!(
                    "duplicate lighting output node {} id {}",
                    output.node, output.id
                ));
            }
            if output.pixel_count == 0 {
                return Err(format!(
                    "lighting output node {} id {} has zero pixels",
                    output.node, output.id
                ));
            }
            let mut capabilities = 0u8;
            for capability in &output.capabilities {
                let bit = match capability.as_str() {
                    "binary" => 1 << 0,
                    "intensity" => 1 << 1,
                    "rgb" => 1 << 2,
                    "white" => 1 << 3,
                    "rgbw" => (1 << 2) | (1 << 3),
                    "addressable" => 1 << 4,
                    other => return Err(format!("unknown lighting output capability {other:?}")),
                };
                if capabilities & bit != 0 {
                    return Err(format!(
                        "lighting output node {} id {} repeats capability {capability:?}",
                        output.node, output.id
                    ));
                }
                capabilities |= bit;
            }
            if capabilities & 0b1111 == 0 {
                return Err(format!(
                    "lighting output node {} id {} has no color capability",
                    output.node, output.id
                ));
            }
            Ok(LightingOutput {
                node: output.node,
                id: output.id,
                pixel_count: output.pixel_count,
                capabilities,
                sparse: output.sparse,
            })
        })
        .collect()
}

fn validate_routes(outputs: &[LightingOutput], routes: &[LightingRoute]) -> Result<(), String> {
    let output_map: HashMap<(u8, u8), &LightingOutput> = outputs
        .iter()
        .map(|output| ((output.node, output.id), output))
        .collect();
    let mut addresses = HashSet::new();
    for route in routes {
        let Some(output) = output_map.get(&(route.node, route.output)) else {
            return Err(format!(
                "lighting slot {} routes to unknown node {} output {}",
                route.slot, route.node, route.output
            ));
        };
        if route.physical_index >= output.pixel_count {
            return Err(format!(
                "lighting slot {} physical index {} is outside node {} output {} length {}",
                route.slot, route.physical_index, route.node, route.output, output.pixel_count
            ));
        }
        if !addresses.insert((route.node, route.output, route.physical_index)) {
            return Err(format!(
                "duplicate lighting physical route node {} output {} index {}",
                route.node, route.output, route.physical_index
            ));
        }
    }
    for output in outputs.iter().filter(|output| !output.sparse) {
        for physical_index in 0..output.pixel_count {
            if !addresses.contains(&(output.node, output.id, physical_index)) {
                return Err(format!(
                    "complete lighting output node {} id {} has hole at index {}",
                    output.node, output.id, physical_index
                ));
            }
        }
    }
    Ok(())
}

fn resolve_layer_scenes(
    layer_count: u8,
    config: &[crate::LightingLayerSceneTomlConfig],
    emitters: &[LightingEmitter],
    zone_memberships: &[u8],
    zone_ids: &HashSet<u8>,
) -> Result<Vec<LightingLayerScene>, String> {
    let id_to_slot: HashMap<u16, u16> = emitters
        .iter()
        .enumerate()
        .map(|(slot, emitter)| (emitter.id, slot as u16))
        .collect();
    let mut scenes = Vec::with_capacity(config.len());
    for scene in config {
        if scene.layer >= layer_count {
            return Err(format!(
                "lighting layer scene {} is outside configured layer count {}",
                scene.layer, layer_count
            ));
        }
        if scene.cells.is_empty() {
            return Err(format!("lighting layer scene {} has no cells", scene.layer));
        }
        let mut cells = Vec::new();
        for cell in &scene.cells {
            let slots: Vec<u16> = match cell.target {
                LightingTargetTomlConfig::Led { led } => vec![
                    *id_to_slot
                        .get(&led)
                        .ok_or_else(|| format!("lighting scene references unknown emitter id {led}"))?,
                ],
                LightingTargetTomlConfig::Key { key } => emitters
                    .iter()
                    .enumerate()
                    .filter(|(_, emitter)| emitter.key == Some(key))
                    .map(|(slot, _)| slot as u16)
                    .collect(),
                LightingTargetTomlConfig::Zone { zone } => {
                    if !zone_ids.contains(&zone) {
                        return Err(format!("lighting scene references unknown zone {zone}"));
                    }
                    emitters
                        .iter()
                        .enumerate()
                        .filter(|(_, emitter)| {
                            let start = emitter.zone_start as usize;
                            let end = start + emitter.zone_len as usize;
                            zone_memberships[start..end].contains(&zone)
                        })
                        .map(|(slot, _)| slot as u16)
                        .collect()
                }
                LightingTargetTomlConfig::All { all: true } => (0..emitters.len() as u16).collect(),
                LightingTargetTomlConfig::All { all: false } => {
                    return Err("lighting target `{ all = false }` is invalid".into());
                }
            };
            if slots.is_empty() {
                return Err(format!(
                    "lighting layer scene {} target resolves to no emitters",
                    scene.layer
                ));
            }
            let effect = resolve_effect(&cell.effect)?;
            cells.extend(slots.into_iter().map(|slot| LightingSceneCell { slot, effect }));
        }
        scenes.push(LightingLayerScene {
            layer: scene.layer,
            cells,
        });
    }
    Ok(scenes)
}

fn resolve_effect(config: &LightingEffectTomlConfig) -> Result<LightingEffect, String> {
    Ok(match *config {
        LightingEffectTomlConfig::Solid { color } => LightingEffect::Solid { color },
        LightingEffectTomlConfig::Blink {
            color,
            period_ms,
            phase_ms,
            duty_percent,
        } => {
            if period_ms == 0 {
                return Err("blink period_ms must be greater than zero".into());
            }
            if duty_percent > 100 {
                return Err(format!("blink duty_percent {duty_percent} exceeds 100"));
            }
            LightingEffect::Blink {
                color,
                period_ms,
                phase_ms,
                duty_percent,
            }
        }
        LightingEffectTomlConfig::Breathe {
            color,
            period_ms,
            phase_ms,
            step_ms,
        } => {
            if period_ms < 2 {
                return Err("breathe period_ms must be at least two".into());
            }
            if step_ms == 0 || u32::from(step_ms) >= period_ms {
                return Err(format!(
                    "breathe step_ms {step_ms} must be greater than zero and less than period_ms {period_ms}"
                ));
            }
            LightingEffect::Breathe {
                color,
                period_ms,
                phase_ms,
                step_ms,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    fn parse(config: &str) -> crate::KeyboardTomlConfig {
        toml::from_str(config).unwrap()
    }

    const BASE: &str = r#"
[matrix]
row_pins = ["r0"]
col_pins = ["c0", "c1"]

[layout]
rows = 1
cols = 2
map = "(0,0,@wide) (0,1)"

[layout.shapes]
wide = { w = 1.5, r = -7.5 }

[keymap]
layers = 2
[[keymap.layer]]
keys = "A B"
[[keymap.layer]]
keys = "A B"

[lighting]
topology_revision = 7
[[lighting.zone]]
id = 1
name = "keys"
[[lighting.output]]
node = 0
id = 0
pixel_count = 2
capabilities = ["rgb", "addressable"]
[[lighting.emitter]]
id = 10
key = [0, 0]
zones = [1]
node = 0
output = 0
physical_index = 1
[[lighting.emitter]]
id = 20
key = [0, 1]
position = [1.0, 0.0, 0.25]
zones = [1]
node = 0
output = 0
physical_index = 0
[[lighting.layer_scene]]
layer = 1
[[lighting.layer_scene.cell]]
target = { zone = 1 }
effect = { kind = "solid", color = [1, 2, 3] }
"#;

    #[test]
    fn derives_geometry_and_logical_keys_from_layout_map() {
        let config = parse(BASE);
        let layout = config.layout().unwrap();
        let keymap = config.keymap().unwrap();
        assert_eq!(layout.keys, vec![[0, 0], [0, 1]]);
        assert_eq!(layout.physical.keys[0].size.width, 384);
        assert_eq!(layout.physical.keys[0].rotation_centidegrees, -750);
        let lighting = config.lighting(&layout, &keymap).unwrap().unwrap();
        assert_eq!(lighting.keys.len(), 2);
        assert_eq!(lighting.emitters.len(), 2);
        assert_eq!(lighting.routes[0].physical_index, 1);
        assert_eq!(lighting.layer_scenes[0].cells.len(), 2);
    }

    #[test]
    fn rejects_emitter_key_that_is_only_inside_matrix_bounds() {
        let hole = BASE
            .replace("col_pins = [\"c0\", \"c1\"]", "col_pins = [\"c0\", \"c1\", \"c2\"]")
            .replace("cols = 2", "cols = 3")
            .replace("(0,1)", "(0,2)");
        let config = parse(&hole);
        let layout = config.layout().unwrap();
        let keymap = config.keymap().unwrap();
        let error = config.lighting(&layout, &keymap).unwrap_err();
        assert!(error.contains("not a logical key in layout.map"), "{error}");
    }

    #[test]
    fn hidden_default_variant_key_remains_a_logical_emitter_key_without_geometry() {
        let source = BASE.replace(
            "map = \"(0,0,@wide) (0,1)\"",
            r#"map = "(0,0,@wide) (0,1)"
default_variant = "compact"
[[layout.variant]]
name = "full"
[[layout.variant]]
name = "compact"
hidden = ["(0,1)"]"#,
        );
        let config = parse(&source);
        let layout = config.layout().unwrap();
        let keymap = config.keymap().unwrap();

        assert!(layout.keys.contains(&[0, 1]), "logical identity is variant-independent");
        assert!(
            layout.physical.keys.iter().all(|key| key.matrix != [0, 1]),
            "the selected/default variant has no fallback center for its hidden key"
        );
        let lighting = config.lighting(&layout, &keymap).unwrap().unwrap();
        assert_eq!(lighting.emitters[1].key, Some([0, 1]));
        assert_eq!(lighting.emitters[1].position.unwrap().z, 64);
    }

    #[test]
    fn rejects_degenerate_animated_effects() {
        for (source, expected) in [
            (
                BASE.replace(
                    "effect = { kind = \"solid\", color = [1, 2, 3] }",
                    "effect = { kind = \"blink\", color = [1, 2, 3], period_ms = 0, duty_percent = 50 }",
                ),
                "blink period_ms",
            ),
            (
                BASE.replace(
                    "effect = { kind = \"solid\", color = [1, 2, 3] }",
                    "effect = { kind = \"breathe\", color = [1, 2, 3], period_ms = 1, step_ms = 1 }",
                ),
                "breathe period_ms",
            ),
            (
                BASE.replace(
                    "effect = { kind = \"solid\", color = [1, 2, 3] }",
                    "effect = { kind = \"breathe\", color = [1, 2, 3], period_ms = 100, step_ms = 100 }",
                ),
                "breathe step_ms",
            ),
        ] {
            let config = parse(&source);
            let layout = config.layout().unwrap();
            let keymap = config.keymap().unwrap();
            let error = config.lighting(&layout, &keymap).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
    }
}
