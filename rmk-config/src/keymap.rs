use std::collections::HashMap;

use pest::Parser;

use crate::layout::{ConfigParser, MapToken, Rule, parse_map};
use crate::{KeyInfo, KeyboardTomlConfig, KeymapConfig, LayerTomlConfig};

// Prevent alias cycles.
const MAX_ALIAS_RESOLUTION_DEPTH: usize = 10;

type GridCoordinate = (u8, u8);
type KeyGrid = Vec<Vec<KeyInfo>>;

struct LayerBuildContext<'a> {
    sequence_to_grid: &'a [GridCoordinate],
    aliases: &'a HashMap<String, String>,
    layer_names: &'a HashMap<String, u32>,
    num_layers: u8,
    rows: u8,
    cols: u8,
}

impl KeyboardTomlConfig {
    /// The resolved layer count: explicit `[keymap].layers`, else the number of
    /// `[[keymap.layer]]` blocks (0 when there's no `[keymap]`). The check that an
    /// explicit count isn't *below* the block count lives in [`get_keymap_config`].
    ///
    /// [`get_keymap_config`]: Self::get_keymap_config
    pub(crate) fn num_layers(&self) -> u8 {
        self.keymap
            .as_ref()
            .map(|k| k.layers.unwrap_or(k.layer.len() as u8))
            .unwrap_or_default()
    }

    /// Resolve `[keymap]` into a dense per-layer action grid plus per-key info.
    ///
    /// `[layout].map` fixes the write-order of keys; each `[[keymap.layer]]` lists its
    /// actions in that order, which we scatter back onto the `rows × cols` grid. Layers
    /// beyond those defined are padded transparent up to `[keymap].layers`.
    pub(crate) fn get_keymap_config(&self) -> Result<(KeymapConfig, Vec<Vec<KeyInfo>>), String> {
        let aliases = self.aliases.clone().unwrap_or_default();
        let keymap_cfg = self.keymap.as_ref().ok_or_else(|| {
            "keyboard.toml: [keymap] section is required — add `[keymap]` with at least one [[keymap.layer]]"
                .to_string()
        })?;
        let layout = self.layout.as_ref().ok_or_else(|| {
            "keyboard.toml: [layout] section is required — add `[layout]` with `rows`, `cols` and a `map`".to_string()
        })?;
        let layers = &keymap_cfg.layer;

        // Aliases are referenced as `@name`, and the name ends at whitespace or a
        // delimiter — a key containing one of those could never be matched.
        for key in aliases.keys() {
            if key.chars().any(|c| c.is_whitespace() || "(),@".contains(c)) {
                return Err(format!(
                    "keyboard.toml: Alias key '{}' must not contain whitespace or any of `(`, `)`, `,`, `@`",
                    key
                ));
            }
        }

        // Explicit layer count may reserve empty layers, not hide defined ones.
        if let Some(n) = keymap_cfg.layers
            && (layers.len() as u8) > n
        {
            return Err(format!(
                "keyboard.toml: {} [[keymap.layer]] entries exceed [keymap].layers = {}",
                layers.len(),
                n
            ));
        }
        let num_layers = self.num_layers();
        if num_layers == 0 {
            return Err(
                "keyboard.toml: [keymap] must define at least one layer using `[[keymap.layer]]` or `layers`"
                    .to_string(),
            );
        }

        // Defined layers need `[layout].map`; explicitly reserved layers are transparent.
        let mut keymap = Vec::with_capacity(num_layers as usize);
        let key_info = match layout.map.as_deref() {
            Some(map) => {
                let (sequence_to_grid, key_info) = Self::build_key_grid(map, layout.rows, layout.cols)?;
                let layer_names = Self::collect_layer_names(layers)?;
                let context = LayerBuildContext {
                    sequence_to_grid: &sequence_to_grid,
                    aliases: &aliases,
                    layer_names: &layer_names,
                    num_layers,
                    rows: layout.rows,
                    cols: layout.cols,
                };
                for (index, layer) in layers.iter().enumerate() {
                    keymap.push(Self::build_layer_grid(index, layer, &context)?);
                }
                key_info
            }
            None if layers.is_empty() => {
                vec![vec![KeyInfo::default(); layout.cols as usize]; layout.rows as usize]
            }
            None => {
                return Err("keyboard.toml: `[layout].map` is required to place `[[keymap.layer]]` keys".to_string());
            }
        };

        // Pad undefined layers with transparent keys up to the configured count.
        for _ in keymap.len()..num_layers as usize {
            keymap.push(vec![vec!["_".to_string(); layout.cols as usize]; layout.rows as usize]);
        }

        let encoder_map = Self::resolve_encoders(layers, &aliases)?;

        Ok((
            KeymapConfig {
                rows: layout.rows,
                cols: layout.cols,
                layers: num_layers,
                keymap,
                encoder_map,
            },
            key_info,
        ))
    }

    /// Build the write-order → grid-coordinate sequence and per-key info (hand) from
    /// `[layout].map`. `parse_map` (shared with the layout blob) already bounds-checks
    /// and de-dupes the coordinates, so here we just keep the `Key` tokens in order.
    fn build_key_grid(map: &str, rows: u8, cols: u8) -> Result<(Vec<GridCoordinate>, KeyGrid), String> {
        let mut key_info = vec![vec![KeyInfo::default(); cols as usize]; rows as usize];
        let mut sequence_to_grid = Vec::new();
        for token in parse_map(map, rows, cols)? {
            if let MapToken::Key { row, col, hand, .. } = token {
                key_info[row as usize][col as usize] = KeyInfo { hand };
                sequence_to_grid.push((row, col));
            }
        }
        Ok((sequence_to_grid, key_info))
    }

    /// Map each named layer to its index, rejecting duplicate names.
    fn collect_layer_names(layers: &[LayerTomlConfig]) -> Result<HashMap<String, u32>, String> {
        let mut layer_names = HashMap::new();
        for (index, layer) in layers.iter().enumerate() {
            if let Some(name) = &layer.name
                && layer_names.insert(name.clone(), index as u32).is_some()
            {
                return Err(format!(
                    "keyboard.toml: Duplicate layer name '{}' found in `[[keymap.layer]]`",
                    name
                ));
            }
        }
        Ok(layer_names)
    }

    /// Scatter one layer's alias-resolved key sequence onto a `rows × cols` grid in the
    /// order fixed by `[layout].map`. An exact count match is required: an under-filled
    /// layer silently shifting every following action onto the wrong key is the worst
    /// keymap failure mode, so missing positions must be explicit (`_`).
    fn build_layer_grid(
        index: usize,
        layer: &LayerTomlConfig,
        context: &LayerBuildContext<'_>,
    ) -> Result<Vec<Vec<String>>, String> {
        let layer_display = match &layer.name {
            Some(name) => format!("layer {index} ('{name}')"),
            None => format!("layer {index}"),
        };
        let key_actions = Self::keymap_parser(&layer.keys, context.aliases, context.layer_names, context.num_layers)
            .map_err(|e| format!("keyboard.toml: Error in `[[keymap.layer]]` {layer_display}: {e}"))?;
        if key_actions.len() != context.sequence_to_grid.len() {
            return Err(format!(
                "keyboard.toml: {layer_display} has {} keys but `layout.map` defines {} positions — every mapped position needs an action (use `_` for transparent)",
                key_actions.len(),
                context.sequence_to_grid.len()
            ));
        }

        let mut grid = vec![vec!["No".to_string(); context.cols as usize]; context.rows as usize];
        for ((row, col), action) in context.sequence_to_grid.iter().zip(key_actions) {
            grid[*row as usize][*col as usize] = action;
        }
        Ok(grid)
    }

    /// Resolve `[[keymap.layer]].encoders` (alias-expanded), one entry per layer.
    fn resolve_encoders(
        layers: &[LayerTomlConfig],
        aliases: &HashMap<String, String>,
    ) -> Result<Vec<Vec<[String; 2]>>, String> {
        let mut encoder_map = Vec::with_capacity(layers.len());
        for layer in layers {
            let mut encoders = layer.encoders.clone().unwrap_or_default();
            for [cw, ccw] in &mut encoders {
                *cw = Self::alias_resolver(cw, aliases)?;
                *ccw = Self::alias_resolver(ccw, aliases)?;
            }
            encoder_map.push(encoders);
        }
        Ok(encoder_map)
    }

    fn alias_resolver(keys: &str, aliases: &HashMap<String, String>) -> Result<String, String> {
        let mut current_keys = keys.to_string();

        let mut iterations = 0;

        loop {
            let mut next_keys = String::with_capacity(current_keys.capacity());
            let mut made_replacement = false;
            let mut last_index = 0;

            while let Some(at_index) = current_keys[last_index..].find('@') {
                let start_index = last_index + at_index;

                next_keys.push_str(&current_keys[last_index..start_index]);

                // The name ends at the grammar's delimiters, so `LM(1, @mods)`
                // resolves `mods` — not a bogus `mods)`. Scan by `char` so a
                // non-ASCII name never produces an invalid UTF-8 slice.
                let alias_start = start_index + '@'.len_utf8();
                let alias_len = current_keys[alias_start..]
                    .char_indices()
                    .find_map(|(offset, c)| (c.is_whitespace() || matches!(c, '(' | ')' | ',' | '@')).then_some(offset))
                    .unwrap_or(current_keys.len() - alias_start);
                let end_index = alias_start + alias_len;

                let alias_key = &current_keys[alias_start..end_index];
                if alias_key.is_empty() {
                    // A bare `@` (trailing, or right before a delimiter) is literal.
                    next_keys.push('@');
                    last_index = alias_start;
                    continue;
                }
                match aliases.get(alias_key) {
                    Some(value) => {
                        next_keys.push_str(value);
                        made_replacement = true;
                    }
                    None => return Err(format!("Undefined alias: {}", alias_key)),
                }
                last_index = end_index;
            }

            next_keys.push_str(&current_keys[last_index..]);

            iterations += 1;
            if iterations >= MAX_ALIAS_RESOLUTION_DEPTH {
                return Err(format!(
                    "Alias resolution exceeded maximum depth ({}), potential infinite loop detected in '{}'",
                    MAX_ALIAS_RESOLUTION_DEPTH, keys
                ));
            }

            if !made_replacement {
                break;
            }

            current_keys = next_keys;
        }

        Ok(current_keys)
    }

    /// Reconstruct an action string from a parsed pair, resolving every named
    /// layer reference (`MO(base)`) to its numeric index (`MO(0)`).
    ///
    /// Layer names may appear at any nesting depth (e.g. inside the tap slot of
    /// `TH(MO(nav), A)`), so this walks the whole subtree, collects the source
    /// span of each `layer_name`, and rewrites those spans in place. Actions
    /// without layer names are returned verbatim.
    fn resolve_layer_names(
        pair: &pest::iterators::Pair<Rule>,
        layer_names: &HashMap<String, u32>,
        num_layers: u8,
    ) -> Result<String, String> {
        let base = pair.as_span().start();
        let mut replacements: Vec<(usize, usize, String)> = Vec::new();
        Self::collect_layer_name_spans(pair.clone(), layer_names, num_layers, &mut replacements)?;

        // Apply right-to-left so earlier byte offsets stay valid.
        replacements.sort_by_key(|(start, _, _)| *start);
        let mut result = pair.as_str().to_string();
        for (start, end, replacement) in replacements.into_iter().rev() {
            result.replace_range(start - base..end - base, &replacement);
        }
        Ok(result)
    }

    /// Recursively collect `(start, end, resolved_number)` for every `layer_name`
    /// in the subtree, validating each against the known layer names. Numeric
    /// layer references are range-checked on the same walk — an out-of-range
    /// index would otherwise compile fine and produce a dead key at runtime.
    fn collect_layer_name_spans(
        pair: pest::iterators::Pair<Rule>,
        layer_names: &HashMap<String, u32>,
        num_layers: u8,
        out: &mut Vec<(usize, usize, String)>,
    ) -> Result<(), String> {
        match pair.as_rule() {
            Rule::layer_name => {
                let layer_name = pair.as_str();
                match layer_names.get(layer_name) {
                    Some(layer_number) => {
                        let span = pair.as_span();
                        out.push((span.start(), span.end(), layer_number.to_string()));
                    }
                    None => return Err(format!("Invalid layer name: {}", layer_name)),
                }
                return Ok(());
            }
            Rule::layer_number => {
                let n: u32 = pair
                    .as_str()
                    .parse()
                    .map_err(|_| format!("Invalid layer number: {}", pair.as_str()))?;
                if n >= num_layers as u32 {
                    return Err(format!(
                        "layer {n} in `{}` is out of range — the keymap defines {num_layers} layer(s), valid indices are 0..={}",
                        pair.as_str(),
                        num_layers.saturating_sub(1)
                    ));
                }
                return Ok(());
            }
            _ => {}
        }
        for inner in pair.into_inner() {
            Self::collect_layer_name_spans(inner, layer_names, num_layers, out)?;
        }
        Ok(())
    }

    fn keymap_parser(
        layer_keys: &str,
        aliases: &HashMap<String, String>,
        layer_names: &HashMap<String, u32>,
        num_layers: u8,
    ) -> Result<Vec<String>, String> {
        // Resolve aliases first, since the grammar below doesn't know about them.
        let layer_keys = Self::alias_resolver(layer_keys, aliases)?;

        let pairs =
            ConfigParser::parse(Rule::key_map, &layer_keys).map_err(|e| format!("Invalid keymap format: {}", e))?;

        let mut key_action_sequence = Vec::new();
        for pair in pairs {
            if pair.as_rule() == Rule::key_map {
                for inner_pair in pair.into_inner() {
                    match inner_pair.as_rule() {
                        Rule::EOI | Rule::WHITESPACE => {}
                        // Resolve layer names even when nested inside an action.
                        _ => {
                            key_action_sequence.push(Self::resolve_layer_names(&inner_pair, layer_names, num_layers)?);
                        }
                    }
                }
            }
        }

        Ok(key_action_sequence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unicode_alias_resolution() {
        let aliases = HashMap::from([
            ("ö".to_string(), "Kc0".to_string()),
            ("ő".to_string(), "LeftBracket".to_string()),
        ]);

        assert_eq!(
            KeyboardTomlConfig::alias_resolver("@ö @ő", &aliases),
            Ok("Kc0 LeftBracket".to_string())
        );
    }

    #[test]
    fn test_unicode_alias_expands_action_and_recurses() {
        let aliases = HashMap::from([
            ("é".to_string(), "WM(Semicolon, RAlt)".to_string()),
            ("magyar".to_string(), "@é".to_string()),
        ]);
        let layer_names = HashMap::new();

        assert_eq!(
            KeyboardTomlConfig::keymap_parser("@magyar", &aliases, &layer_names, 8),
            Ok(vec!["WM(Semicolon, RAlt)".to_string()])
        );
    }

    #[test]
    fn test_undefined_unicode_alias_returns_error() {
        assert_eq!(
            KeyboardTomlConfig::alias_resolver("不存在", &HashMap::new()),
            Ok("不存在".to_string())
        );
        assert_eq!(
            KeyboardTomlConfig::alias_resolver("@不存在", &HashMap::new()),
            Err("Undefined alias: 不存在".to_string())
        );
    }

    #[test]
    fn test_unicode_whitespace_separates_aliases_and_actions() {
        let aliases = HashMap::from([("ö".to_string(), "Kc0".to_string())]);
        let layer_names = HashMap::new();

        for separator in ['\u{00a0}', '\u{3000}'] {
            let keymap = format!("@ö{separator}A");
            assert_eq!(
                KeyboardTomlConfig::keymap_parser(&keymap, &aliases, &layer_names, 8),
                Ok(vec!["Kc0".to_string(), "A".to_string()]),
                "separator U+{:04X}",
                separator as u32
            );
        }
    }

    #[test]
    fn test_no_action_parsing() {
        let test_cases = vec![
            ("No ", vec!["No"]),
            ("No\n", vec!["No"]),
            ("No\t", vec!["No"]),
            ("No  A", vec!["No", "A"]),
            ("A No B", vec!["A", "No", "B"]),
            ("No No No", vec!["No", "No", "No"]),
        ];

        for (input, expected) in test_cases {
            let result = ConfigParser::parse(Rule::key_map, input);
            assert!(result.is_ok(), "Failed to parse: {}", input);

            let mut actions = Vec::new();
            for pair in result.unwrap() {
                if pair.as_rule() == Rule::key_map {
                    for inner_pair in pair.into_inner() {
                        match inner_pair.as_rule() {
                            Rule::no_action | Rule::simple_keycode => {
                                actions.push(inner_pair.as_str().to_string());
                            }
                            Rule::EOI | Rule::WHITESPACE => {}
                            _ => {}
                        }
                    }
                }
            }

            assert_eq!(actions, expected, "Input: {}", input);
        }
    }

    #[test]
    fn test_no_vs_no_prefixed_keycodes() {
        let test_cases = vec![
            ("No", Rule::no_action),
            ("NoUsSlash", Rule::simple_keycode),
            ("NonUsSlash", Rule::simple_keycode),
            ("NoReturn", Rule::simple_keycode),
            ("NoBrake", Rule::simple_keycode),
        ];

        for (input, expected_rule) in test_cases {
            let result = ConfigParser::parse(Rule::key_map, input);
            assert!(result.is_ok(), "Failed to parse: {}", input);

            let mut found_rule = None;
            for pair in result.unwrap() {
                if pair.as_rule() == Rule::key_map {
                    for inner_pair in pair.into_inner() {
                        match inner_pair.as_rule() {
                            Rule::no_action | Rule::simple_keycode => {
                                found_rule = Some(inner_pair.as_rule());
                            }
                            _ => {}
                        }
                    }
                }
            }

            assert_eq!(
                found_rule,
                Some(expected_rule),
                "Input: {} should be parsed as {:?}",
                input,
                expected_rule
            );
        }
    }

    #[test]
    fn test_keymap_parser_with_no_actions() {
        let aliases = HashMap::new();
        let layer_names = HashMap::new();

        let keymap = "A B No C No NoUsSlash NonUsSlash D No";
        let result = KeyboardTomlConfig::keymap_parser(keymap, &aliases, &layer_names, 8);

        assert!(result.is_ok());
        let actions = result.unwrap();
        assert_eq!(
            actions,
            vec!["A", "B", "No", "C", "No", "NoUsSlash", "NonUsSlash", "D", "No"]
        );
    }

    #[test]
    fn test_keymap_parser_with_comma_alias() {
        let aliases = HashMap::new();
        let layer_names = HashMap::new();

        let keymap = "A , SHIFTED(,) B";
        let result = KeyboardTomlConfig::keymap_parser(keymap, &aliases, &layer_names, 8);

        assert!(result.is_ok());
        let actions = result.unwrap();
        assert_eq!(actions, vec!["A", ",", "SHIFTED(,)", "B"]);
    }

    #[test]
    fn test_comma_separator_compatibility_in_multi_arg_actions() {
        let aliases = HashMap::new();
        let layer_names = HashMap::new();

        // Comma keeps working as argument separator in multi-argument actions.
        let keymap = "TH(A, B) TH(Comma, B) TH(A, Comma)";
        let result = KeyboardTomlConfig::keymap_parser(keymap, &aliases, &layer_names, 8);

        assert!(result.is_ok());
        let actions = result.unwrap();
        assert_eq!(actions, vec!["TH(A, B)", "TH(Comma, B)", "TH(A, Comma)"]);
    }

    #[test]
    fn test_multi_arg_actions_reject_symbol_comma_as_key_argument() {
        let invalid_cases = ["TH(A, ,)", "TH(, ,)", "WM(, LShift)", "LT(1, ,)", "MT(, LShift)"];

        for input in invalid_cases {
            let result = ConfigParser::parse(Rule::key_map, input);
            assert!(result.is_err(), "Input should be rejected: {}", input);
        }
    }

    #[test]
    fn malformed_layer_returns_err_not_panic() {
        // A keymap the grammar can't parse returns Err rather than panicking the
        // build (keymap_parser used to `panic!` on the pest error).
        let aliases = HashMap::new();
        let layer_names = HashMap::new();
        let result = KeyboardTomlConfig::keymap_parser("TH(A, ,)", &aliases, &layer_names, 8);
        assert!(result.is_err(), "unparseable keymap must be Err, not a panic");
    }

    #[test]
    fn test_single_key_arg_actions_accept_symbol_comma() {
        let valid_cases = ["SHIFTED(,)", "SHIFTED(Comma)", ","];

        for input in valid_cases {
            let result = ConfigParser::parse(Rule::key_map, input);
            assert!(result.is_ok(), "Input should be accepted: {}", input);
        }
    }

    #[test]
    fn test_morse_action_parsing() {
        let aliases = HashMap::new();
        let layer_names = HashMap::new();

        let keymap = "A TD(0) B TD(1) C TD(255)";
        let result = KeyboardTomlConfig::keymap_parser(keymap, &aliases, &layer_names, 8);

        assert!(result.is_ok());
        let actions = result.unwrap();
        assert_eq!(actions, vec!["A", "TD(0)", "B", "TD(1)", "C", "TD(255)"]);
    }

    #[test]
    fn test_macro_trigger_action_parsing() {
        let aliases = std::collections::HashMap::new();
        let layer_names = std::collections::HashMap::new();

        let keymap = "A Macro(0) B MACRO(1) C macro(255)";
        let result = KeyboardTomlConfig::keymap_parser(keymap, &aliases, &layer_names, 8);

        assert!(result.is_ok());
        let actions = result.unwrap();
        assert_eq!(actions, vec!["A", "Macro(0)", "B", "MACRO(1)", "C", "macro(255)"]);
    }

    #[test]
    fn test_held_modifier_action_parsing() {
        let aliases = HashMap::new();
        let layer_names = HashMap::new();

        let keymap = "MOD(LCtrl | LAlt | LGui) mod(RShift)";
        let result = KeyboardTomlConfig::keymap_parser(keymap, &aliases, &layer_names, 8);

        assert_eq!(result.unwrap(), vec!["MOD(LCtrl | LAlt | LGui)", "mod(RShift)"]);
    }

    #[test]
    fn test_morse_action_grammar() {
        let test_cases = vec![
            ("TD(0)", Rule::morse_action),
            ("TD(1)", Rule::morse_action),
            ("TD(255)", Rule::morse_action),
            ("td(0)", Rule::morse_action), // Case insensitive
            ("td(1)", Rule::morse_action),
            ("MORSE(0)", Rule::morse_action),
            ("MORSE(1)", Rule::morse_action),
            ("MORSE(255)", Rule::morse_action),
            ("Morse(0)", Rule::morse_action), // Case insensitive
            ("morse(1)", Rule::morse_action),
        ];

        for (input, expected_rule) in test_cases {
            let result = ConfigParser::parse(Rule::key_map, input);
            assert!(result.is_ok(), "Failed to parse: {}", input);

            let mut found_rule = None;
            for pair in result.unwrap() {
                if pair.as_rule() == Rule::key_map {
                    for inner_pair in pair.into_inner() {
                        if inner_pair.as_rule() == Rule::morse_action {
                            found_rule = Some(inner_pair.as_rule());
                        }
                    }
                }
            }

            assert_eq!(
                found_rule,
                Some(expected_rule),
                "Input: {} should be parsed as {:?}",
                input,
                expected_rule
            );
        }
    }

    #[test]
    fn test_macro_grammar() {
        let test_cases = vec![
            ("Macro(0)", Rule::trigger_macro_action),
            ("Macro(1)", Rule::trigger_macro_action),
            ("Macro(255)", Rule::trigger_macro_action),
            ("MACRO(0)", Rule::trigger_macro_action), // Case insensitive
            ("MACRO(1)", Rule::trigger_macro_action),
            ("macro(0)", Rule::trigger_macro_action), // Case insensitive
            ("macro(1)", Rule::trigger_macro_action),
            ("macro(255)", Rule::trigger_macro_action),
        ];

        for (input, expected_rule) in test_cases {
            let result = ConfigParser::parse(Rule::key_map, input);
            assert!(result.is_ok(), "Failed to parse: {}", input);

            let mut found_rule = None;
            for pair in result.unwrap() {
                if pair.as_rule() == Rule::key_map {
                    for inner_pair in pair.into_inner() {
                        if inner_pair.as_rule() == Rule::trigger_macro_action {
                            found_rule = Some(inner_pair.as_rule());
                        }
                    }
                }
            }

            assert_eq!(
                found_rule,
                Some(expected_rule),
                "Input: {} should be parsed as {:?}",
                input,
                expected_rule
            );
        }
    }

    #[test]
    fn test_nested_actions_in_tap_hold_slots() {
        let aliases = HashMap::new();
        let layer_names = HashMap::new();

        // Single-action forms can appear in the tap/hold slots of
        // MT/TH/LT and is forwarded verbatim for the proc-macro to expand.
        let keymap = "MT(WM(P, RAlt), LShift, HRM) TH(WM(A, LShift), MO(2)) LT(1, MOD(LCtrl | LGui))";
        let result = KeyboardTomlConfig::keymap_parser(keymap, &aliases, &layer_names, 8);

        assert!(result.is_ok(), "{:?}", result);
        assert_eq!(
            result.unwrap(),
            vec![
                "MT(WM(P, RAlt), LShift, HRM)",
                "TH(WM(A, LShift), MO(2))",
                "LT(1, MOD(LCtrl | LGui))",
            ]
        );
    }

    #[test]
    fn test_layer_name_resolution_nested() {
        let aliases = HashMap::new();
        let mut layer_names = HashMap::new();
        layer_names.insert("nav".to_string(), 3u32);

        // Layer names are resolved to indices even when nested inside a slot.
        let keymap = "MO(nav) TH(A, MO(nav))";
        let result = KeyboardTomlConfig::keymap_parser(keymap, &aliases, &layer_names, 8);

        assert!(result.is_ok(), "{:?}", result);
        assert_eq!(result.unwrap(), vec!["MO(3)", "TH(A, MO(3))"]);
    }

    #[test]
    fn test_composite_actions_rejected_in_slots() {
        // Tap-hold / morse forms are not single `Action`s, so they cannot nest
        // inside a slot. The grammar must reject these.
        let invalid_cases = ["MT(MT(A, LCtrl), LShift)", "TH(TD(0), B)", "MT(LT(1, A), LShift)"];

        for input in invalid_cases {
            let result = ConfigParser::parse(Rule::key_map, input);
            assert!(result.is_err(), "Input should be rejected: {}", input);
        }
    }

    #[test]
    fn double_slash_is_not_a_comment() {
        let aliases = HashMap::new();
        let layer_names = HashMap::new();
        // `keys` is data-only now: `//` is no longer a comment, just (garbage) tokens.
        let result = KeyboardTomlConfig::keymap_parser("A // B", &aliases, &layer_names, 8);
        assert_eq!(result.unwrap(), vec!["A", "//", "B"]);
    }

    fn config(toml: &str) -> KeyboardTomlConfig {
        toml::from_str(toml).expect("parse test config")
    }

    #[test]
    fn layers_defaults_to_block_count() {
        let cfg = config(
            "[layout]\nrows = 1\ncols = 2\nmap = \"(0,0) (0,1)\"\n\
             [keymap]\n[[keymap.layer]]\nkeys = \"A B\"\n[[keymap.layer]]\nkeys = \"C D\"\n",
        );
        let (km, _) = cfg.get_keymap_config().unwrap();
        assert_eq!(km.layers, 2);
        assert_eq!(km.keymap.len(), 2);
    }

    #[test]
    fn explicit_layers_reserves_extra_transparent_layers() {
        let cfg = config(
            "[layout]\nrows = 1\ncols = 1\nmap = \"(0,0)\"\n\
             [keymap]\nlayers = 4\n[[keymap.layer]]\nkeys = \"A\"\n",
        );
        let (km, _) = cfg.get_keymap_config().unwrap();
        assert_eq!(km.layers, 4);
        assert_eq!(km.keymap.len(), 4);
        assert_eq!(km.keymap[3][0][0], "_"); // reserved layers are transparent
    }

    #[test]
    fn explicit_layers_below_block_count_is_rejected() {
        let cfg = config(
            "[layout]\nrows = 1\ncols = 1\nmap = \"(0,0)\"\n\
             [keymap]\nlayers = 1\n[[keymap.layer]]\nkeys = \"A\"\n[[keymap.layer]]\nkeys = \"B\"\n",
        );
        assert!(cfg.get_keymap_config().is_err());
    }

    #[test]
    fn empty_keymap_is_rejected() {
        let cfg = config("[layout]\nrows = 1\ncols = 1\nmap = \"(0,0)\"\n[keymap]\n");
        let Err(err) = cfg.get_keymap_config() else {
            panic!("empty keymap must be rejected");
        };
        assert!(err.contains("must define at least one layer"));
    }

    #[test]
    fn explicit_zero_layers_is_rejected() {
        let cfg = config("[layout]\nrows = 1\ncols = 1\nmap = \"(0,0)\"\n[keymap]\nlayers = 0\n");
        let Err(err) = cfg.get_keymap_config() else {
            panic!("zero keymap layers must be rejected");
        };
        assert!(err.contains("must define at least one layer"));
    }

    #[test]
    fn layer_with_more_encoders_than_hardware_is_rejected() {
        // Reject layer encoders that exceed the board hardware.
        let cfg = config(
            "[matrix]\nrow_pins = [\"r0\"]\ncol_pins = [\"c0\"]\n\
             [layout]\nrows = 1\ncols = 1\nmap = \"(0,0)\"\n\
             [keymap]\n[[keymap.layer]]\nkeys = \"A\"\nencoders = [[\"Up\", \"Down\"]]\n",
        );
        assert!(cfg.keymap().is_err());
    }

    /// A board with two physical encoders (declared via top-level `[[input_device.encoder]]`).
    fn two_encoder_config(encoders_line: &str) -> KeyboardTomlConfig {
        config(&format!(
            "[matrix]\nrow_pins = [\"r0\"]\ncol_pins = [\"c0\"]\n\
             [layout]\nrows = 1\ncols = 1\nmap = \"(0,0)\"\n\
             [keymap]\n[[keymap.layer]]\nkeys = \"A\"\n{encoders_line}\n\
             [[input_device.encoder]]\npin_a = \"a0\"\npin_b = \"b0\"\n\
             [[input_device.encoder]]\npin_a = \"a1\"\npin_b = \"b1\"\n"
        ))
    }

    #[test]
    fn layer_with_partial_encoders_is_rejected() {
        // Partial encoder maps would silently drop hardware.
        assert!(two_encoder_config("encoders = [[\"Up\", \"Down\"]]").keymap().is_err());
    }

    #[test]
    fn layer_with_all_encoders_is_accepted() {
        assert!(
            two_encoder_config("encoders = [[\"Up\", \"Down\"], [\"Left\", \"Right\"]]")
                .keymap()
                .is_ok()
        );
    }

    #[test]
    fn layer_with_no_encoders_is_accepted() {
        // Omitted encoder maps default to No.
        assert!(two_encoder_config("").keymap().is_ok());
    }

    #[test]
    fn encoders_resolve_without_a_board() {
        // Simulator scenarios have no [matrix]/[split]; [input_device] still counts.
        let cfg = config(
            "[layout]\nrows = 1\ncols = 1\nmap = \"(0,0)\"\n\
             [keymap]\n[[keymap.layer]]\nkeys = \"A\"\nencoders = [[\"Up\", \"Down\"]]\n\
             [[input_device.encoder]]\npin_a = \"a0\"\npin_b = \"b0\"\n",
        );
        assert_eq!(cfg.keymap().unwrap().num_encoder, 1);
    }

    #[test]
    fn numeric_layer_reference_must_be_in_range() {
        let aliases = HashMap::new();
        let layer_names = HashMap::new();
        // Out-of-range layer refs would be dead keys at runtime.
        let err = KeyboardTomlConfig::keymap_parser("MO(2)", &aliases, &layer_names, 2).unwrap_err();
        assert!(err.contains("out of range"), "{err}");
        // Every layer-taking form goes through the same check, `PDF` included.
        let err = KeyboardTomlConfig::keymap_parser("PDF(5)", &aliases, &layer_names, 2).unwrap_err();
        assert!(err.contains("out of range"), "{err}");
        // In-range references pass, including nested ones.
        assert!(KeyboardTomlConfig::keymap_parser("MO(1) LT(0, A) TH(MO(1), B)", &aliases, &layer_names, 2).is_ok());
    }

    #[test]
    fn under_filled_layer_is_rejected() {
        let aliases = HashMap::new();
        let layer_names = HashMap::new();
        let layer = LayerTomlConfig {
            name: None,
            keys: "A B".to_string(),
            encoders: None,
        };
        let seq = vec![(0u8, 0u8), (0, 1), (0, 2)];
        let context = LayerBuildContext {
            sequence_to_grid: &seq,
            aliases: &aliases,
            layer_names: &layer_names,
            num_layers: 1,
            rows: 1,
            cols: 3,
        };
        let err = KeyboardTomlConfig::build_layer_grid(0, &layer, &context).unwrap_err();
        assert!(err.contains("has 2 keys but `layout.map` defines 3 positions"), "{err}");
    }

    #[test]
    fn alias_ends_at_delimiters() {
        let aliases = HashMap::from([
            ("mods".to_string(), "LShift | LGui".to_string()),
            ("a".to_string(), "A".to_string()),
            ("b".to_string(), "B".to_string()),
        ]);

        // The motivating case: an alias directly before `)` resolves.
        assert_eq!(
            KeyboardTomlConfig::alias_resolver("LM(1, @mods)", &aliases).unwrap(),
            "LM(1, LShift | LGui)"
        );
        // Aliases directly inside parens and before commas.
        assert_eq!(
            KeyboardTomlConfig::alias_resolver("WM(@a, LShift) TH(@a, @b)", &aliases).unwrap(),
            "WM(A, LShift) TH(A, B)"
        );
        // Adjacent aliases resolve independently.
        assert_eq!(KeyboardTomlConfig::alias_resolver("@a@b", &aliases).unwrap(), "AB");
    }

    #[test]
    fn bare_at_is_literal() {
        let aliases = HashMap::from([("a".to_string(), "A".to_string())]);
        // Trailing, before whitespace, and before a delimiter: all literal.
        assert_eq!(KeyboardTomlConfig::alias_resolver("X @", &aliases).unwrap(), "X @");
        assert_eq!(KeyboardTomlConfig::alias_resolver("@ X", &aliases).unwrap(), "@ X");
        assert_eq!(KeyboardTomlConfig::alias_resolver("X@)", &aliases).unwrap(), "X@)");
    }

    #[test]
    fn undefined_alias_reports_bare_name() {
        let aliases = HashMap::new();
        let err = KeyboardTomlConfig::alias_resolver("LM(1, @mods)", &aliases).unwrap_err();
        assert_eq!(err, "Undefined alias: mods");
    }

    #[test]
    fn alias_recursion_depth_is_bounded() {
        let aliases = HashMap::from([("loop".to_string(), "@loop".to_string())]);
        let err = KeyboardTomlConfig::alias_resolver("@loop", &aliases).unwrap_err();
        assert!(err.contains("maximum depth"), "{err}");
    }
}
