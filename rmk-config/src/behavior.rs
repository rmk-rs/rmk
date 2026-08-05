use std::collections::HashSet;

use crate::{BehaviorConfig, LayoutTomlConfig, MacroOperation, MorseProfile, MorsesConfig};

fn expand_hold_trigger_regions(
    layout: Option<&LayoutTomlConfig>,
    positions: &mut Option<Vec<[u8; 2]>>,
    region_names: Option<&[String]>,
    context: &str,
) -> Result<(), String> {
    let Some(layout) = layout else {
        if region_names.is_some_and(|names| !names.is_empty()) {
            return Err(format!(
                "keyboard.toml: {context}.hold_trigger_regions requires [layout]"
            ));
        }
        return Ok(());
    };
    let valid_positions = crate::layout::logical_key_positions(layout)?;
    let mut expanded = positions.take().unwrap_or_default();

    if let Some(names) = region_names {
        let regions = layout.regions.as_ref();
        for name in names {
            let region = regions.and_then(|regions| regions.get(name)).ok_or_else(|| {
                format!("keyboard.toml: {context}.hold_trigger_regions references unknown layout region '{name}'")
            })?;
            expanded.extend(region);
        }
    }

    let mut seen = HashSet::new();
    expanded.retain(|position| seen.insert(*position));
    if let Some(position) = expanded.iter().find(|position| !valid_positions.contains(*position)) {
        return Err(format!(
            "keyboard.toml: {context} references hold trigger position [{}, {}], which is not a key in layout.map",
            position[0], position[1]
        ));
    }
    *positions = (!expanded.is_empty()).then_some(expanded);
    Ok(())
}

fn expand_morse_regions(layout: Option<&LayoutTomlConfig>, morse: &mut MorsesConfig) -> Result<(), String> {
    expand_hold_trigger_regions(
        layout,
        &mut morse.hold_trigger_key_positions,
        morse.hold_trigger_regions.as_deref(),
        "behavior.morse",
    )?;
    if let Some(profiles) = &mut morse.profiles {
        for (name, profile) in profiles {
            expand_profile_regions(layout, name, profile)?;
        }
    }
    Ok(())
}

fn expand_profile_regions(
    layout: Option<&LayoutTomlConfig>,
    name: &str,
    profile: &mut MorseProfile,
) -> Result<(), String> {
    expand_hold_trigger_regions(
        layout,
        &mut profile.hold_trigger_key_positions,
        profile.hold_trigger_regions.as_deref(),
        &format!("behavior.morse.profiles.{name}"),
    )
}

impl crate::KeyboardTomlConfig {
    pub(crate) fn get_behavior_config(&self) -> Result<BehaviorConfig, String> {
        let default = self.behavior.clone().unwrap_or_default();
        let num_layers = self.num_layers();
        match self.behavior.clone() {
            Some(mut behavior) => {
                behavior.tri_layer = match behavior.tri_layer {
                    Some(tri_layer) => {
                        if tri_layer.upper >= num_layers {
                            return Err("keyboard.toml: Tri layer upper is larger than [keymap].layers".to_string());
                        } else if tri_layer.lower >= num_layers {
                            return Err("keyboard.toml: Tri layer lower is larger than [keymap].layers".to_string());
                        } else if tri_layer.adjust >= num_layers {
                            return Err("keyboard.toml: Tri layer adjust is larger than [keymap].layers".to_string());
                        }
                        Some(tri_layer)
                    }
                    None => default.tri_layer,
                };
                behavior.one_shot = behavior.one_shot.or(default.one_shot);
                behavior.one_shot_modifiers = behavior.one_shot_modifiers.or(default.one_shot_modifiers);
                behavior.combo = behavior.combo.or(default.combo);
                if let Some(combo) = &behavior.combo {
                    if combo.combos.len() > self.rmk.combo_max_num {
                        return Err("keyboard.toml: number of combos is greater than combo_max_num configured under [rmk] section".to_string());
                    }
                    for (i, c) in combo.combos.iter().enumerate() {
                        if c.actions.len() > self.rmk.combo_max_length {
                            return Err(format!(
                                "keyboard.toml: number of keys in combo #{} is greater than combo_max_length configured under [rmk] section",
                                i
                            ));
                        }
                        if let Some(layer) = c.layer
                            && layer >= num_layers
                        {
                            return Err(format!(
                                "keyboard.toml: layer in combo #{} is greater than [keymap].layers",
                                i
                            ));
                        }
                    }
                }
                behavior.macros = behavior.macros.or(default.macros);
                if let Some(macros) = &behavior.macros {
                    let macros_size = macros
                        .macros
                        .iter()
                        .map(|m| {
                            m.operations
                                .iter()
                                .map(|op| match op {
                                    MacroOperation::Tap { .. }
                                    | MacroOperation::Down { .. }
                                    | MacroOperation::Up { .. } => 3,
                                    MacroOperation::Delay { .. } => 4,
                                    MacroOperation::Text { text } => text.len(),
                                })
                                .sum::<usize>()
                        })
                        .sum::<usize>();

                    if macros_size > self.rmk.macro_space_size {
                        return Err(format!(
                            "keyboard.toml: total size of macros ({}) is greater than macro_space_size configured under [rmk] section",
                            macros_size
                        ));
                    }
                }
                behavior.fork = behavior.fork.or(default.fork);
                if let Some(fork) = &behavior.fork
                    && fork.forks.len() > self.rmk.fork_max_num
                {
                    return Err(
                        "keyboard.toml: number of forks is greater than fork_max_num configured under [rmk] section"
                            .to_string(),
                    );
                }
                behavior.auto_mouse_layer = behavior.auto_mouse_layer.or(default.auto_mouse_layer);
                if let Some(entries) = &behavior.auto_mouse_layer {
                    let auto_mouse_layer_max_num = self
                        .rmk
                        .auto_mouse_layer_max_num
                        .unwrap_or(crate::resolved::behavior::DEFAULT_AUTO_MOUSE_LAYER_MAX_NUM);
                    if entries.len() > auto_mouse_layer_max_num {
                        return Err(format!(
                            "keyboard.toml: number of [[behavior.auto_mouse_layer]] entries ({}) exceeds auto_mouse_layer_max_num ({}) configured under [rmk] section",
                            entries.len(),
                            auto_mouse_layer_max_num
                        ));
                    }
                    let mut seen_device_ids: Vec<Option<u8>> = Vec::new();
                    for entry in entries {
                        if entry.target_layer >= num_layers {
                            return Err(format!(
                                "keyboard.toml: [[behavior.auto_mouse_layer]].target_layer must be a valid layer index (< [keymap].layers = {}), got {}",
                                num_layers, entry.target_layer
                            ));
                        }
                        if entry.threshold == Some(0) {
                            return Err(
                                "keyboard.toml: [[behavior.auto_mouse_layer]].threshold must be at least 1".to_string(),
                            );
                        }
                        if let Some(timeout) = &entry.timeout {
                            let timeout_ms = timeout.0;
                            if timeout_ms == 0 {
                                return Err(
                                    "keyboard.toml: [[behavior.auto_mouse_layer]].timeout must be at least 1ms"
                                        .to_string(),
                                );
                            }
                            if timeout_ms > u32::MAX as u64 {
                                return Err(format!(
                                    "keyboard.toml: [[behavior.auto_mouse_layer]].timeout must be <= {}ms (~49 days), got {}ms",
                                    u32::MAX,
                                    timeout_ms
                                ));
                            }
                        }
                        if (entry.deactivate_on_key == Some(true) || entry.reset_timeout_on_key == Some(true))
                            && self.event.action.subs == 0
                        {
                            return Err(
                                "keyboard.toml: [[behavior.auto_mouse_layer]].deactivate_on_key / reset_timeout_on_key require [event.action] subs to be at least 1".to_string(),
                            );
                        }
                        if seen_device_ids.contains(&entry.device_id) {
                            return Err(match entry.device_id {
                                Some(id) => format!(
                                    "keyboard.toml: duplicate [[behavior.auto_mouse_layer]] entries for device_id = {id}"
                                ),
                                None => "keyboard.toml: multiple [[behavior.auto_mouse_layer]] entries without device_id; only one fallback is allowed".to_string(),
                            });
                        }
                        seen_device_ids.push(entry.device_id);
                    }
                }
                behavior.morse = behavior.morse.or(default.morse);
                if let Some(morse) = &mut behavior.morse {
                    expand_morse_regions(self.layout.as_ref(), morse)?;
                }
                if let Some(morse) = &behavior.morse
                    && let Some(morses) = &morse.morses
                    && morses.len() > self.rmk.morse_max_num
                {
                    return Err(
                        "keyboard.toml: number of morses is greater than morse_max_num configured under [rmk] section"
                            .to_string(),
                    );
                }
                Ok(behavior)
            }
            None => Ok(default),
        }
    }
}
