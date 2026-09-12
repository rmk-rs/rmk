//! Build-time physical key layout.
//!
//! Walk `[layout].map`, apply variants, and build the compressed `GetLayout`
//! blob. Firmware streams the blob opaquely; hosts inflate and decode it with
//! these same types (`rynk::layout` re-exports them), so producer and decoder
//! can't drift.

use std::collections::{HashMap, HashSet};

use pest::Parser;
use pest_derive::Parser;
use serde::{Deserialize, Serialize};

use crate::LayoutTomlConfig;

#[derive(Parser)]
#[grammar = "keymap.pest"]
pub(crate) struct ConfigParser;

/// A key's outline rectangle in key-units.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// The authoring rotation region (KLE's `(r, rx, ry)` cluster triple) a key or
/// encoder was placed by, in the same flat frame as `rect` centers. Equal
/// values = one rigid cluster; `Key::r - deg` is the residual own-center angle.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct Region {
    pub deg: f32,
    pub px: f32,
    pub py: f32,
}

/// One key's placement: matrix position, outline `rect` (center + size),
/// rotation, and an optional second rectangle for L-shaped keys (ISO/big-ass
/// Enter). `r` rotates the whole key, `rect2` included.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct Key {
    pub row: u8,
    pub col: u8,
    pub rect: Rect,
    pub r: f32,
    pub rect2: Option<Rect>,
    /// The rotation region that placed this key — editor metadata only;
    /// `rect`/`r` already carry the final geometry. `None` = flat frame.
    pub pivot: Option<Region>,
}

/// One encoder's placement within a variant: a fixed 1u knob, so just its
/// center — never resized or L-shaped. `pivot` records the region that swung
/// the center; the knob itself carries no angle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct Encoder {
    pub id: u8,
    pub x: f32,
    pub y: f32,
    pub pivot: Option<Region>,
}

/// One render variant (e.g. ANSI / ISO): its own keys and encoders. A hidden key
/// reflows the tokens after it — encoders included — so each variant carries its
/// own encoder positions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct Variant {
    pub name: String,
    pub keys: Vec<Key>,
    pub encoders: Vec<Encoder>,
}

/// The decoded physical layout: one entry per render variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct LayoutInfo {
    pub default_variant: u8,
    pub variants: Vec<Variant>,
}

impl LayoutInfo {
    /// An empty layout, emitted when firmware was built without a `[layout].map`.
    pub fn empty() -> Self {
        Self {
            default_variant: 0,
            variants: Vec::new(),
        }
    }

    /// Decode the compressed `GetLayout` payload this crate's blob builder
    /// produced: raw DEFLATE around a postcard-encoded [`LayoutInfo`]. An empty
    /// blob is a valid "empty" layout.
    pub fn from_compressed_blob(blob: &[u8]) -> Result<Self, String> {
        if blob.is_empty() {
            return Ok(Self::empty());
        }

        let inflated = miniz_oxide::inflate::decompress_to_vec(blob).map_err(|e| format!("inflate failed: {e}"))?;
        postcard::from_bytes(&inflated).map_err(|e| format!("decode failed: {e}"))
    }
}

/// A resolved shape: every default applied. `rect2` is the L-key's second
/// rectangle stored as center-relative offsets — its `x`/`y` are offsets from
/// the primary center, not absolute positions (the walk resolves them).
#[derive(Clone, Copy, Debug)]
struct Shape {
    w: f32,
    h: f32,
    x: f32,
    y: f32,
    r: f32,
    rect2: Option<Rect>,
}

impl Default for Shape {
    fn default() -> Self {
        Shape {
            w: 1.0,
            h: 1.0,
            x: 0.0,
            y: 0.0,
            r: 0.0,
            rect2: None,
        }
    }
}

impl From<&crate::ShapeToml> for Shape {
    fn from(t: &crate::ShapeToml) -> Self {
        let rect2 = t.w2.map(|w2| Rect {
            w: w2,
            h: t.h2.unwrap_or(1.0),
            x: t.x2.unwrap_or(0.0),
            y: t.y2.unwrap_or(0.0),
        });
        Shape {
            w: t.w.unwrap_or(1.0),
            h: t.h.unwrap_or(1.0),
            x: t.x.unwrap_or(0.0),
            y: t.y.unwrap_or(0.0),
            r: t.r.unwrap_or(0.0),
            rect2,
        }
    }
}

/// RMK's shipped stock widths (`@Nu`): N units wide, 1u tall. The single source
/// of truth shared with the KLE converter in `rynk-kle`, which matches against
/// these to emit a `@Nu` reference instead of a generated shape — the two must
/// agree on names or a token the converter emits would fail to resolve here.
pub const STOCK_WIDTHS: &[(&str, f32)] = &[
    ("1.25u", 1.25),
    ("1.5u", 1.5),
    ("1.75u", 1.75),
    ("2u", 2.0),
    ("2.25u", 2.25),
    ("2.75u", 2.75),
    ("3u", 3.0),
    ("6.25u", 6.25),
    ("7u", 7.0),
];

/// The shipped stock shapes (`@2u`, `@1.5u`, …, `@iso_enter`, …). Keyed without
/// the leading `@`. User `[layout.shapes]` entries of the same name override.
fn stock_shapes() -> HashMap<String, Shape> {
    let d = Shape::default();
    let mut m = HashMap::new();
    // Width family: `@Nu` is N units wide, 1u tall.
    for &(name, w) in STOCK_WIDTHS {
        m.insert(name.to_string(), Shape { w, ..d });
    }
    // Tall numpad Plus/Enter.
    m.insert("2u_tall".to_string(), Shape { h: 2.0, ..d });
    // Stepped Caps keeps a single 1.75u footprint.
    m.insert("stepped_caps".to_string(), Shape { w: 1.75, ..d });
    // ISO Enter uses center-relative rect2 offsets, not KLE top-left offsets.
    m.insert(
        "iso_enter".to_string(),
        Shape {
            w: 1.25,
            h: 2.0,
            y: -1.0,
            rect2: Some(Rect {
                w: 1.5,
                h: 1.0,
                x: -0.125,
                y: -0.5,
            }),
            ..d
        },
    );
    // Big-ass Enter: bottom bar plus right-aligned upper cap.
    m.insert(
        "bae".to_string(),
        Shape {
            w: 2.25,
            rect2: Some(Rect {
                w: 1.5,
                h: 1.0,
                x: 0.375,
                y: -1.0,
            }),
            ..d
        },
    );
    m
}

/// One token of the `[layout].map` grammar. The single shared representation:
/// keymap resolution takes the `Key` write-order + `hand`, the render walk
/// takes every token (and ignores `hand`).
pub(crate) enum MapToken {
    Key {
        row: u8,
        col: u8,
        hand: char,
        shape: Option<String>,
    },
    Encoder {
        id: u8,
    },
    /// A `[n]` horizontal gap, in key-units.
    Gap(f32),
    /// A `[y=n]` extra vertical step for the next row.
    VStep(f32),
    /// A `[r=deg@(x,y)]` rotation region: keys and encoders after it rotate
    /// `deg` clockwise about the pivot until the next `[r=...]`. `[r=0]`
    /// (pivot optional) returns to the flat frame.
    Rot {
        deg: f32,
        px: f32,
        py: f32,
    },
    Newline,
}

fn parse_u8(s: &str, what: &str) -> Result<u8, String> {
    s.parse::<u8>()
        .map_err(|e| format!("keyboard.toml: bad {what} '{s}' in layout.map: {e}"))
}

fn parse_f32(s: &str, what: &str) -> Result<f32, String> {
    s.parse::<f32>()
        .map_err(|e| format!("keyboard.toml: bad {what} '{s}' in layout.map: {e}"))
}

/// The `@`-stripped name inside a `shape_ref` pair.
fn shape_name_of(pair: pest::iterators::Pair<Rule>) -> String {
    pair.into_inner()
        .next()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default()
}

/// Parse the `[layout].map` string into its token stream, validating that every
/// key coordinate is in bounds and unique. This is the single source of truth for
/// both keymap resolution (`get_keymap_config`) and the render walk, so the two
/// can never disagree on which positions are keys or in what order.
pub(crate) fn parse_map(map: &str, rows: u8, cols: u8) -> Result<Vec<MapToken>, String> {
    let pairs =
        ConfigParser::parse(Rule::layout_map, map).map_err(|e| format!("keyboard.toml: Error in `layout.map`: {e}"))?;
    let mut tokens = Vec::new();
    let mut seen: HashSet<(u8, u8)> = HashSet::new();
    for pair in pairs {
        if pair.as_rule() != Rule::layout_map {
            continue;
        }
        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::keypos_info => {
                    let mut it = inner.into_inner();
                    let row = parse_u8(it.next().ok_or("missing row")?.as_str(), "row")?;
                    let col = parse_u8(it.next().ok_or("missing col")?.as_str(), "col")?;
                    let mut hand = 'C';
                    let mut shape = None;
                    for part in it {
                        match part.as_rule() {
                            Rule::left_hand => hand = 'L',
                            Rule::right_hand => hand = 'R',
                            Rule::bilateral_hand => hand = '*',
                            Rule::shape_ref => shape = Some(shape_name_of(part)),
                            _ => {}
                        }
                    }
                    if row >= rows || col >= cols {
                        return Err(format!(
                            "keyboard.toml: layout.map coordinate ({row},{col}) is out of bounds ([0..{}], [0..{}])",
                            rows.saturating_sub(1),
                            cols.saturating_sub(1)
                        ));
                    }
                    if !seen.insert((row, col)) {
                        return Err(format!(
                            "keyboard.toml: duplicate coordinate ({row},{col}) in layout.map"
                        ));
                    }
                    tokens.push(MapToken::Key { row, col, hand, shape });
                }
                Rule::encoder_info => {
                    let id = parse_u8(
                        inner.into_inner().next().ok_or("missing encoder id")?.as_str(),
                        "encoder id",
                    )?;
                    tokens.push(MapToken::Encoder { id });
                }
                Rule::spacer => {
                    let u = inner.into_inner().next().ok_or("missing gap")?.as_str();
                    tokens.push(MapToken::Gap(parse_f32(u, "gap")?));
                }
                Rule::vertical => {
                    let u = inner.into_inner().next().ok_or("missing y-step")?.as_str();
                    tokens.push(MapToken::VStep(parse_f32(u, "y-step")?));
                }
                Rule::rotation => {
                    let vals = inner
                        .into_inner()
                        .map(|p| parse_f32(p.as_str(), "rotation"))
                        .collect::<Result<Vec<f32>, String>>()?;
                    let (deg, px, py) = match vals.as_slice() {
                        [deg, px, py] => (*deg, *px, *py),
                        [deg] if *deg == 0.0 => (0.0, 0.0, 0.0),
                        [deg] => {
                            return Err(format!(
                                "keyboard.toml: [r={deg}] in layout.map needs a pivot — write \
                                 [r={deg}@(x,y)]; only [r=0] (end of region) may omit it"
                            ));
                        }
                        _ => return Err("keyboard.toml: malformed [r=...] in layout.map".to_string()),
                    };
                    if ![deg, px, py].iter().all(|v| v.is_finite()) {
                        return Err("keyboard.toml: non-finite value in layout.map [r=...]".to_string());
                    }
                    tokens.push(MapToken::Rot { deg, px, py });
                }
                Rule::newline => tokens.push(MapToken::Newline),
                _ => {}
            }
        }
    }
    Ok(tokens)
}

pub(crate) fn logical_key_positions(layout: &LayoutTomlConfig) -> Result<HashSet<[u8; 2]>, String> {
    if let Some(map) = &layout.map {
        return Ok(parse_map(map, layout.rows, layout.cols)?
            .into_iter()
            .filter_map(|token| match token {
                MapToken::Key { row, col, .. } => Some([row, col]),
                _ => None,
            })
            .collect());
    }

    Ok((0..layout.rows)
        .flat_map(|row| (0..layout.cols).map(move |col| [row, col]))
        .collect())
}

/// Cursor state. A row's baseline `y` is the TOP of the row; a key stores its
/// center. The advance to the next row is *lazy*: a newline only arms a break
/// (so a lone `[y=n]` line doesn't itself consume a row), and the next key /
/// encoder / gap performs the `1 + pending_vstep` drop.
struct Walker {
    cursor_x: f32,
    baseline_y: f32,
    row_has_content: bool,
    break_pending: bool,
    pending_vstep: f32,
}

impl Walker {
    fn new() -> Self {
        Walker {
            cursor_x: 0.0,
            baseline_y: 0.0,
            row_has_content: false,
            break_pending: false,
            pending_vstep: 0.0,
        }
    }

    fn advance_if_pending(&mut self) {
        if self.break_pending {
            self.baseline_y += 1.0 + self.pending_vstep;
            self.pending_vstep = 0.0;
            self.cursor_x = 0.0;
            self.break_pending = false;
            self.row_has_content = false;
        }
    }
}

fn resolve_shape(name: Option<&str>, shapes: &HashMap<String, Shape>) -> Result<Shape, String> {
    match name {
        None => Ok(Shape::default()),
        Some(n) => shapes
            .get(n)
            .copied()
            .ok_or_else(|| format!("keyboard.toml: unknown shape '@{n}' in layout.map")),
    }
}

/// Walk one variant: `overrides` reshape a key, `hidden` drop it from the walk
/// (so following keys reflow). Returns this variant's keys and encoders — a
/// hidden key before an encoder reflows the encoder along with the keys.
fn walk(
    tokens: &[MapToken],
    shapes: &HashMap<String, Shape>,
    overrides: &HashMap<(u8, u8), String>,
    hidden: &HashSet<(u8, u8)>,
) -> Result<(Vec<Key>, Vec<Encoder>), String> {
    let mut w = Walker::new();
    let mut keys = Vec::new();
    let mut encoders = Vec::new();
    // Active rotation region; cursor coordinates stay flat.
    let mut rot: Option<Region> = None;
    let swing = |x: f32, y: f32, rot: &Option<Region>| -> (f32, f32) {
        match rot {
            None => (x, y),
            Some(Region { deg, px, py }) => {
                let (sin, cos) = deg.to_radians().sin_cos();
                let (dx, dy) = (x - px, y - py);
                (px + dx * cos - dy * sin, py + dx * sin + dy * cos)
            }
        }
    };
    for tok in tokens {
        match tok {
            MapToken::Newline => {
                if w.row_has_content {
                    w.break_pending = true;
                }
            }
            MapToken::VStep(n) => {
                // `[y=n]` affects only the next real row break.
                if w.row_has_content {
                    w.pending_vstep += n;
                }
            }
            MapToken::Gap(g) => {
                w.advance_if_pending();
                w.cursor_x += g;
            }
            MapToken::Key { row, col, shape, .. } => {
                w.advance_if_pending();
                // Hidden keys do not advance the cursor.
                if hidden.contains(&(*row, *col)) {
                    continue;
                }
                let name = overrides.get(&(*row, *col)).map(String::as_str).or(shape.as_deref());
                let s = resolve_shape(name, shapes)?;
                let (cx, cy) = swing(w.cursor_x + s.w / 2.0 + s.x, w.baseline_y + s.h / 2.0 + s.y, &rot);
                // rect2 stays in the key frame; region and shape angles add.
                let rect2 = s.rect2.map(|r2| Rect {
                    x: cx + r2.x,
                    y: cy + r2.y,
                    w: r2.w,
                    h: r2.h,
                });
                keys.push(Key {
                    row: *row,
                    col: *col,
                    rect: Rect {
                        x: cx,
                        y: cy,
                        w: s.w,
                        h: s.h,
                    },
                    r: s.r + rot.map_or(0.0, |r| r.deg),
                    rect2,
                    pivot: rot,
                });
                w.cursor_x += s.w;
                w.row_has_content = true;
            }
            MapToken::Encoder { id } => {
                w.advance_if_pending();
                // Encoders are fixed 1u knobs.
                let (x, y) = swing(w.cursor_x + 0.5, w.baseline_y + 0.5, &rot);
                encoders.push(Encoder {
                    id: *id,
                    x,
                    y,
                    pivot: rot,
                });
                w.cursor_x += 1.0;
                w.row_has_content = true;
            }
            MapToken::Rot { deg, px, py } => {
                // Rotation markers consume no row by themselves.
                rot = (*deg != 0.0).then_some(Region {
                    deg: *deg,
                    px: *px,
                    py: *py,
                });
            }
        }
    }
    Ok((keys, encoders))
}

/// Parse a quoted `"(r,c)"` overlay key into `(row, col)`.
fn parse_rc(s: &str) -> Result<(u8, u8), String> {
    let inner = s.trim().trim_start_matches('(').trim_end_matches(')');
    let mut it = inner.split(',');
    let r = parse_u8(it.next().unwrap_or("").trim(), "variant target row")?;
    let c = parse_u8(it.next().unwrap_or("").trim(), "variant target col")?;
    Ok((r, c))
}

/// Every f32 dimension of a shape is finite (rejects `nan`/`inf` from TOML).
fn shape_is_finite(s: &Shape) -> bool {
    [s.w, s.h, s.x, s.y, s.r].iter().all(|v| v.is_finite())
        && s.rect2
            .is_none_or(|r| [r.x, r.y, r.w, r.h].iter().all(|v| v.is_finite()))
}

/// `expected_encoders` is the board's physical encoder count (`Some` from the
/// real build, `None` from the standalone TOML helper which has no board).
fn build_layout_info(
    layout: &LayoutTomlConfig,
    expected_encoders: Option<usize>,
) -> Result<Option<LayoutInfo>, String> {
    let Some(map) = &layout.map else {
        return Ok(None);
    };
    let tokens = parse_map(map, layout.rows, layout.cols)?;

    // Variant overlays must target real map keys.
    let key_coords: HashSet<(u8, u8)> = tokens
        .iter()
        .filter_map(|tok| match tok {
            MapToken::Key { row, col, .. } => Some((*row, *col)),
            _ => None,
        })
        .collect();

    let mut shapes = stock_shapes();
    if let Some(user) = &layout.shapes {
        for (k, v) in user {
            let s = Shape::from(v);
            if !shape_is_finite(&s) {
                return Err(format!(
                    "keyboard.toml: shape '{k}' has a non-finite (nan/inf) dimension"
                ));
            }
            shapes.insert(k.clone(), s);
        }
    }

    let no_variants = Vec::new();
    let variants_toml = layout.variant.as_ref().unwrap_or(&no_variants);
    // `default_variant` is serialized as a u8 index, so at most 256 variants.
    if variants_toml.len() > u8::MAX as usize + 1 {
        return Err(format!(
            "keyboard.toml: too many [[layout.variant]] ({}); at most {}",
            variants_toml.len(),
            u8::MAX as usize + 1
        ));
    }
    // Reject overlay targets that would otherwise be silent no-ops.
    for v in variants_toml {
        let targets = v
            .shapes
            .iter()
            .flatten()
            .map(|(k, _)| k)
            .chain(v.hidden.iter().flatten());
        for rc in targets {
            let coord = parse_rc(rc)?;
            if !key_coords.contains(&coord) {
                return Err(format!(
                    "keyboard.toml: variant '{}' targets ({},{}) which is not a key in layout.map",
                    v.name, coord.0, coord.1
                ));
            }
        }
    }

    // Each variant is complete; hidden keys reflow later keys and encoders.
    let mut variants: Vec<Variant> = Vec::new();
    if variants_toml.is_empty() {
        let (keys, encoders) = walk(&tokens, &shapes, &HashMap::new(), &HashSet::new())?;
        variants.push(Variant {
            name: "default".to_string(),
            keys,
            encoders,
        });
    } else {
        for v in variants_toml {
            let mut overrides = HashMap::new();
            for (rc, name) in v.shapes.iter().flatten() {
                overrides.insert(parse_rc(rc)?, name.trim_start_matches('@').to_string());
            }
            let mut hidden = HashSet::new();
            for rc in v.hidden.iter().flatten() {
                hidden.insert(parse_rc(rc)?);
            }
            let (keys, encoders) = walk(&tokens, &shapes, &overrides, &hidden)?;
            variants.push(Variant {
                name: v.name.clone(),
                keys,
                encoders,
            });
        }
    }

    // Unknown default variant names fall back to variant 0.
    let default_variant = layout
        .default_variant
        .as_ref()
        .and_then(|name| variants.iter().position(|v| &v.name == name))
        .unwrap_or(0) as u8;

    // Encoder ids are variant-invariant; validate one dense 0..N list.
    let encoders = &variants[0].encoders;
    let mut ids: Vec<u8> = encoders.iter().map(|e| e.id).collect();
    ids.sort_unstable();
    for (expected, &id) in ids.iter().enumerate() {
        if id as usize != expected {
            return Err(format!(
                "keyboard.toml: encoder ids in layout.map must be unique and cover 0..{} (got {ids:?})",
                ids.len()
            ));
        }
    }
    if let Some(n) = expected_encoders
        && !encoders.is_empty()
        && encoders.len() != n
    {
        return Err(format!(
            "keyboard.toml: layout.map has {} encoder (e,id) tokens but the board declares {n}",
            encoders.len()
        ));
    }

    Ok(Some(LayoutInfo {
        default_variant,
        variants,
    }))
}

/// Build the decoded layout from a `[layout]`-section TOML string (`None` when
/// there's no `map`). What a host decodes from the blob is exactly this value.
pub fn layout_info_from_toml(layout_toml: &str) -> Result<Option<LayoutInfo>, String> {
    let layout: LayoutTomlConfig = toml::from_str(layout_toml).map_err(|e| e.to_string())?;
    build_layout_info(&layout, None)
}

/// Build the compressed layout blob from a `[layout]`-section TOML string —
/// the exact bytes firmware would serve over `GetLayout`.
pub fn layout_blob_from_toml(layout_toml: &str) -> Result<Vec<u8>, String> {
    let layout: LayoutTomlConfig = toml::from_str(layout_toml).map_err(|e| e.to_string())?;
    build_layout_blob(&layout, None)
}

/// Build the compressed, opaque layout blob (empty when there's no `map`).
/// `expected_encoders` is the board's physical encoder count, or `None` to skip
/// that cross-check (the standalone TOML helper has no board).
pub(crate) fn build_layout_blob(
    layout: &LayoutTomlConfig,
    expected_encoders: Option<usize>,
) -> Result<Vec<u8>, String> {
    let Some(info) = build_layout_info(layout, expected_encoders)? else {
        return Ok(Vec::new());
    };
    let bytes =
        postcard::to_allocvec(&info).map_err(|e| format!("keyboard.toml: layout blob serialize failed: {e}"))?;
    // Compression runs at build time; hosts use the matching raw DEFLATE decoder.
    Ok(miniz_oxide::deflate::compress_to_vec(&bytes, 10))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn info_of(toml: &str) -> LayoutInfo {
        let cfg: LayoutTomlConfig = toml::from_str(toml).unwrap();
        build_layout_info(&cfg, None).unwrap().unwrap()
    }

    fn key(v: &Variant, row: u8, col: u8) -> &Key {
        v.keys
            .iter()
            .find(|k| k.row == row && k.col == col)
            .expect("key present")
    }

    #[test]
    fn bare_keys_make_a_unit_grid() {
        let info = info_of("rows = 1\ncols = 3\nmap = \"(0,0) (0,1) (0,2)\"");
        let v = &info.variants[0];
        assert_eq!(v.name, "default");
        for (i, k) in v.keys.iter().enumerate() {
            assert!(approx(k.rect.x, i as f32 + 0.5), "center x");
            assert!(approx(k.rect.y, 0.5), "center y");
            assert!(approx(k.rect.w, 1.0) && approx(k.rect.h, 1.0));
            assert!(k.rect2.is_none());
        }
    }

    #[test]
    fn duplicate_coord_is_rejected() {
        // Blob and keymap paths must reject duplicate cells consistently.
        let cfg: LayoutTomlConfig = toml::from_str("rows = 1\ncols = 2\nmap = \"(0,0) (0,0)\"").unwrap();
        assert!(build_layout_blob(&cfg, None).is_err(), "duplicate (0,0) must fail");
    }

    #[test]
    fn stock_width_moves_the_cursor() {
        // A 2u key advances the next key to x=2.5.
        let info = info_of("rows = 1\ncols = 2\nmap = \"(0,0,@2u) (0,1)\"");
        let v = &info.variants[0];
        assert!(approx(key(v, 0, 0).rect.x, 1.0) && approx(key(v, 0, 0).rect.w, 2.0));
        assert!(approx(key(v, 0, 1).rect.x, 2.5));
    }

    #[test]
    fn stock_iso_enter_is_a_true_l() {
        // rect2 offsets are center-to-center; the overhang aligns right.
        let info = info_of("rows = 1\ncols = 1\nmap = \"(0,0,@iso_enter)\"");
        let k = key(&info.variants[0], 0, 0);
        let r2 = k.rect2.expect("two rects");
        assert!(approx(r2.w, 1.5) && approx(r2.h, 1.0));
        assert!(
            approx(k.rect.x + k.rect.w / 2.0, r2.x + r2.w / 2.0),
            "right edges flush: bar {} vs overhang {}",
            k.rect.x + k.rect.w / 2.0,
            r2.x + r2.w / 2.0
        );
        assert!(
            approx(r2.y, k.rect.y - 0.5),
            "overhang on the upper row: {} vs {}",
            r2.y,
            k.rect.y - 0.5
        );
    }

    #[test]
    fn y_step_is_one_shot_and_lazy() {
        // Row 1 lands 1.25u below row 0.
        let info = info_of("rows = 2\ncols = 2\nmap = \"\"\"\n(0,0) (0,1)\n[y=0.25]\n(1,0) (1,1)\n\"\"\"");
        let v = &info.variants[0];
        assert!(approx(key(v, 0, 0).rect.y, 0.5));
        assert!(approx(key(v, 1, 0).rect.y, 1.75)); // 0.5 + 1.25
    }

    #[test]
    fn leading_y_step_is_dropped() {
        // Leading `[y=n]` must not leak into the first real row break.
        let info = info_of("rows = 3\ncols = 1\nmap = \"\"\"\n[y=0.5]\n(0,0)\n(1,0)\n(2,0)\n\"\"\"");
        let v = &info.variants[0];
        assert!(approx(key(v, 0, 0).rect.y, 0.5));
        assert!(approx(key(v, 1, 0).rect.y, 1.5)); // not 2.0
        assert!(approx(key(v, 2, 0).rect.y, 2.5)); // not 3.0
    }

    #[test]
    fn unknown_default_variant_falls_back_to_zero() {
        // Unknown default names fall back instead of failing the build.
        let info = info_of(
            "rows = 1\ncols = 1\ndefault_variant = \"typo\"\nmap = \"(0,0)\"\n[[variant]]\nname = \"a\"\n[[variant]]\nname = \"b\"",
        );
        assert_eq!(info.default_variant, 0);
        // No variants still resolves to default variant 0.
        let info2 = info_of("rows = 1\ncols = 1\ndefault_variant = \"x\"\nmap = \"(0,0)\"");
        assert_eq!(info2.default_variant, 0);
    }

    #[test]
    fn encoder_ids_must_be_unique_and_dense() {
        let ok = info_of("rows = 1\ncols = 2\nmap = \"(0,0) (e,0) (0,1) (e,1)\"");
        assert_eq!(ok.variants[0].encoders.len(), 2);
        let dup: LayoutTomlConfig = toml::from_str("rows = 1\ncols = 1\nmap = \"(0,0) (e,0) (e,0)\"").unwrap();
        assert!(build_layout_info(&dup, None).is_err(), "duplicate encoder id must fail");
        let gap: LayoutTomlConfig = toml::from_str("rows = 1\ncols = 1\nmap = \"(0,0) (e,0) (e,2)\"").unwrap();
        assert!(
            build_layout_info(&gap, None).is_err(),
            "non-dense encoder ids must fail"
        );
    }

    #[test]
    fn encoder_shape_is_rejected() {
        // Encoders are fixed 1u knobs, so shapes are invalid.
        let cfg: LayoutTomlConfig = toml::from_str("rows = 1\ncols = 1\nmap = \"(0,0) (e,0,@2u)\"").unwrap();
        assert!(build_layout_blob(&cfg, None).is_err(), "(e,id,@shape) must be rejected");
    }

    #[test]
    fn out_of_bounds_coord_is_rejected() {
        let cfg: LayoutTomlConfig = toml::from_str("rows = 1\ncols = 1\nmap = \"(0,0) (0,5)\"").unwrap();
        assert!(build_layout_info(&cfg, None).is_err());
    }

    #[test]
    fn non_finite_shape_is_rejected() {
        let nan: LayoutTomlConfig =
            toml::from_str("rows = 1\ncols = 1\nmap = \"(0,0,@bad)\"\n[shapes]\nbad = { w = nan }").unwrap();
        assert!(build_layout_info(&nan, None).is_err(), "nan width must fail");
        let inf: LayoutTomlConfig =
            toml::from_str("rows = 1\ncols = 1\nmap = \"(0,0,@big)\"\n[shapes]\nbig = { x = inf }").unwrap();
        assert!(build_layout_info(&inf, None).is_err(), "inf nudge must fail");
    }

    #[test]
    fn variant_target_must_be_a_real_key() {
        // `hidden` names a non-key.
        let cfg: LayoutTomlConfig = toml::from_str(
            "rows = 1\ncols = 2\nmap = \"(0,0) (0,1)\"\n[[variant]]\nname = \"a\"\nhidden = [\"(0,9)\"]",
        )
        .unwrap();
        assert!(build_layout_info(&cfg, None).is_err());
    }

    #[test]
    fn encoders_reflow_per_variant() {
        // Hidden keys reflow following encoder positions too.
        let info = info_of(
            "rows = 1\ncols = 2\nmap = \"(0,0) (0,1) (e,0)\"\n[[variant]]\nname = \"full\"\n[[variant]]\nname = \"mini\"\nhidden = [\"(0,0)\"]",
        );
        let full = info.variants.iter().find(|v| v.name == "full").unwrap();
        let mini = info.variants.iter().find(|v| v.name == "mini").unwrap();
        assert_eq!(full.encoders.len(), 1);
        assert!(approx(full.encoders[0].x, 2.5), "full knob x = {}", full.encoders[0].x);
        assert!(
            approx(mini.encoders[0].x, 1.5),
            "mini knob x = {} (reflowed after hiding (0,0))",
            mini.encoders[0].x
        );
        assert!(
            mini.keys.iter().all(|k| !(k.row == 0 && k.col == 0)),
            "(0,0) is hidden in mini"
        );
    }

    #[test]
    fn encoder_count_must_match_board() {
        // One encoder token cannot cover two board encoders.
        let one: LayoutTomlConfig = toml::from_str("rows = 1\ncols = 1\nmap = \"(0,0) (e,0)\"").unwrap();
        assert!(
            build_layout_blob(&one, Some(2)).is_err(),
            "1 token vs 2 board encoders must fail"
        );
        assert!(build_layout_blob(&one, Some(1)).is_ok(), "matching count is fine");
        // Omitting encoder positions opts out of layout placement.
        let none: LayoutTomlConfig = toml::from_str("rows = 1\ncols = 1\nmap = \"(0,0)\"").unwrap();
        assert!(
            build_layout_blob(&none, Some(3)).is_ok(),
            "opting out of encoder positions is allowed"
        );
    }

    const CORNE_SPLIT: &str = r#"
rows = 4
cols = 12
default_variant = "corne42"
map = """
(0,0,L,@cP) (0,1,L,@cR) (0,2,L,@cM) (0,3,L,@cI) (0,4,L,@cI) (0,5,L,@cX) [1.0] (0,6,R,@cX) (0,7,R,@cI) (0,8,R,@cI) (0,9,R,@cM) (0,10,R,@cR) (0,11,R,@cP)
(1,0,L,@cP) (1,1,L,@cR) (1,2,L,@cM) (1,3,L,@cI) (1,4,L,@cI) (1,5,L,@cX) [1.0] (1,6,R,@cX) (1,7,R,@cI) (1,8,R,@cI) (1,9,R,@cM) (1,10,R,@cR) (1,11,R,@cP)
(2,0,L,@cP) (2,1,L,@cR) (2,2,L,@cM) (2,3,L,@cI) (2,4,L,@cI) (2,5,L,@cX) [1.0] (2,6,R,@cX) (2,7,R,@cI) (2,8,R,@cI) (2,9,R,@cM) (2,10,R,@cR) (2,11,R,@cP)
[y=0.05]
[3.5] (3,3,L,@thumbL) (3,4,L) (3,5,L,@thumbR) [1.0] (3,6,R,@thumbL) (3,7,R) (3,8,R,@thumbR)
"""

[shapes]
cP = { y = 0.55 }
cR = { y = 0.25 }
cM = { y = 0.0 }
cI = { y = 0.10 }
cX = { y = 0.25 }
thumbL = { r = 15.0 }
thumbR = { r = -15.0 }

[[variant]]
name = "corne42"

[[variant]]
name = "corne36"
hidden = ["(0,0)", "(1,0)", "(2,0)", "(0,11)", "(1,11)", "(2,11)"]
"#;

    #[test]
    fn rotation_region_swings_keys_about_the_pivot() {
        // Rotation swings both keys onto the same vertical.
        let info = info_of("rows = 1\ncols = 3\nmap = \"(0,0) [1.5] [r=90@(2.5,0)] (0,1) (0,2)\"");
        let v = &info.variants[0];
        assert!(approx(key(v, 0, 0).rect.x, 0.5) && approx(key(v, 0, 0).r, 0.0));
        assert_eq!(key(v, 0, 0).pivot, None);
        let k1 = key(v, 0, 1);
        assert!(
            approx(k1.rect.x, 2.0) && approx(k1.rect.y, 0.5) && approx(k1.r, 90.0),
            "k1 ({}, {}, r={})",
            k1.rect.x,
            k1.rect.y,
            k1.r
        );
        let k2 = key(v, 0, 2);
        assert!(
            approx(k2.rect.x, 2.0) && approx(k2.rect.y, 1.5) && approx(k2.r, 90.0),
            "k2 ({}, {}, r={})",
            k2.rect.x,
            k2.rect.y,
            k2.r
        );
        // Both keys carry the identical authoring region.
        let region = Region {
            deg: 90.0,
            px: 2.5,
            py: 0.0,
        };
        assert_eq!(k1.pivot, Some(region));
        assert_eq!(k2.pivot, Some(region));
    }

    #[test]
    fn rotation_is_rigid_across_rows() {
        // One region applies a rigid transform across rows.
        let info =
            info_of("rows = 2\ncols = 2\nmap = \"\"\"\n[3.5] [r=25@(3.5,0)] (0,0) (0,1)\n[3.5] (1,0) (1,1)\n\"\"\"");
        let v = &info.variants[0];
        assert!(v.keys.iter().all(|k| approx(k.r, 25.0)), "all keys carry the angle");
        // One region token = one bit-identical pivot value on every key.
        let region = Region {
            deg: 25.0,
            px: 3.5,
            py: 0.0,
        };
        assert!(v.keys.iter().all(|k| k.pivot == Some(region)), "shared cluster");
        let d = |a: &Key, b: &Key| ((a.rect.x - b.rect.x).powi(2) + (a.rect.y - b.rect.y).powi(2)).sqrt();
        assert!(approx(d(key(v, 0, 0), key(v, 0, 1)), 1.0));
        assert!(approx(d(key(v, 0, 0), key(v, 1, 0)), 1.0));
        assert!(approx(d(key(v, 0, 0), key(v, 1, 1)), 2f32.sqrt()));
        // Verify the actual rotated center.
        let (sin, cos) = 25f32.to_radians().sin_cos();
        let k = key(v, 0, 0);
        assert!(
            approx(k.rect.x, 3.5 + 0.5 * cos - 0.5 * sin) && approx(k.rect.y, 0.5 * sin + 0.5 * cos),
            "got ({}, {})",
            k.rect.x,
            k.rect.y
        );
    }

    #[test]
    fn rotation_zero_ends_the_region() {
        // `[r=0]` returns to the flat cursor frame.
        let info = info_of("rows = 1\ncols = 3\nmap = \"(0,0) [r=15@(1,0)] (0,1) [r=0] (0,2)\"");
        let v = &info.variants[0];
        let k2 = key(v, 0, 2);
        assert!(approx(k2.rect.x, 2.5) && approx(k2.rect.y, 0.5) && approx(k2.r, 0.0));
        assert_eq!(k2.pivot, None);
        // The reset also holds across lines.
        let info = info_of("rows = 2\ncols = 1\nmap = \"\"\"\n[r=30@(0,0)] (0,0)\n[r=0] (1,0)\n\"\"\"");
        let v = &info.variants[0];
        assert!(approx(key(v, 1, 0).rect.x, 0.5) && approx(key(v, 1, 0).rect.y, 1.5));
        // A pivot on `[r=0...]` is legal noise: a zero-degree region is the flat
        // frame and must not mint a cluster identity.
        let info = info_of("rows = 1\ncols = 2\nmap = \"[r=45@(1,0)] (0,0) [r=0@(1,2)] (0,1)\"");
        assert_eq!(key(&info.variants[0], 0, 1).pivot, None);
    }

    #[test]
    fn rotation_composes_with_shape_r_and_swings_encoders() {
        let toml = "rows = 1\ncols = 1\nmap = \"[r=15@(0,0)] (0,0,@tilt) (e,0)\"\n[shapes]\ntilt = { r = 10.0 }";
        let info = info_of(toml);
        let v = &info.variants[0];
        // Region and shape angles add; the pivot keeps only the region's share,
        // so the residual shape angle stays recoverable as `r - pivot.deg`.
        let region = Region {
            deg: 15.0,
            px: 0.0,
            py: 0.0,
        };
        assert!(approx(key(v, 0, 0).r, 25.0), "r = {}", key(v, 0, 0).r);
        assert_eq!(key(v, 0, 0).pivot, Some(region));
        // Encoder centers swing with the active region and carry it.
        let (sin, cos) = 15f32.to_radians().sin_cos();
        let e = &v.encoders[0];
        assert!(
            approx(e.x, 1.5 * cos - 0.5 * sin) && approx(e.y, 1.5 * sin + 0.5 * cos),
            "knob ({}, {})",
            e.x,
            e.y
        );
        assert_eq!(e.pivot, Some(region));
    }

    #[test]
    fn rotation_without_pivot_is_rejected() {
        let cfg: LayoutTomlConfig = toml::from_str("rows = 1\ncols = 1\nmap = \"[r=15] (0,0)\"").unwrap();
        let err = build_layout_info(&cfg, None).unwrap_err();
        assert!(err.contains("pivot"), "{err}");
    }

    #[test]
    fn pivot_metadata_mirrors_the_active_region() {
        // flat → region A → region B → [r=0] → flat, key by key.
        let info = info_of("rows = 1\ncols = 4\nmap = \"(0,0) [r=10@(1,0)] (0,1) [r=20@(2,0)] (0,2) [r=0] (0,3)\"");
        let v = &info.variants[0];
        let pivot = |c| key(v, 0, c).pivot;
        assert_eq!(pivot(0), None);
        assert_eq!(
            pivot(1),
            Some(Region {
                deg: 10.0,
                px: 1.0,
                py: 0.0
            })
        );
        assert_eq!(
            pivot(2),
            Some(Region {
                deg: 20.0,
                px: 2.0,
                py: 0.0
            })
        );
        assert_eq!(pivot(3), None);
    }

    #[test]
    fn hidden_keys_reflow_inside_a_rotation_region() {
        // Hidden-key reflow happens before rotation.
        let toml = "rows = 1\ncols = 2\nmap = \"[1.0] [r=20@(1,0)] (0,0) (0,1)\"\n[[variant]]\nname = \"full\"\n[[variant]]\nname = \"mini\"\nhidden = [\"(0,0)\"]";
        let info = info_of(toml);
        let full = info.variants.iter().find(|v| v.name == "full").unwrap();
        let mini = info.variants.iter().find(|v| v.name == "mini").unwrap();
        let (a, b) = (&key(full, 0, 0).rect, &key(mini, 0, 1).rect);
        assert!(
            approx(a.x, b.x) && approx(a.y, b.y),
            "({}, {}) vs ({}, {})",
            a.x,
            a.y,
            b.x,
            b.y
        );
        // Reflow moves centers, never the region identity.
        assert_eq!(key(full, 0, 1).pivot, key(mini, 0, 1).pivot);
        assert!(key(mini, 0, 1).pivot.is_some());
    }

    #[test]
    fn y_step_shifts_every_row_below() {
        // Baseline shifts accumulate below `[y=1]`.
        let info = info_of("rows = 3\ncols = 1\nmap = \"\"\"\n(0,0)\n[y=1]\n(1,0)\n(2,0)\n\"\"\"");
        let v = &info.variants[0];
        // Stored y is the key center.
        assert!(approx(key(v, 0, 0).rect.y, 0.5)); // top 0
        assert!(approx(key(v, 1, 0).rect.y, 2.5)); // top 2  (= 1 + 1 shift)
        assert!(approx(key(v, 2, 0).rect.y, 3.5)); // top 3  (= 2 + 1 shift)
    }

    #[test]
    fn blob_sizes_stay_firmware_friendly() {
        for (name, toml) in [
            ("60% ANSI/ISO/split-bs", ANSI_ISO_60),
            ("Corne split (42/36)", CORNE_SPLIT),
        ] {
            let cfg: LayoutTomlConfig = toml::from_str(toml).unwrap();
            let compressed = build_layout_blob(&cfg, None).unwrap().len();
            assert!(compressed < 2048, "{name} blob {compressed} B exceeds 2 KB");
        }
    }

    #[test]
    fn corne_worked_example() {
        // Cover nudge, split gap, and thumb rotation in one fixture.
        let toml = r#"
rows = 4
cols = 12
map = """
(0,0,L,@cP) (0,1,L,@cR) (0,2,L,@cM) (0,3,L,@cI) (0,4,L,@cI) (0,5,L,@cX) [1.0] (0,6,R,@cX) (0,7,R,@cI) (0,8,R,@cI) (0,9,R,@cM) (0,10,R,@cR) (0,11,R,@cP)
(1,0,L,@cP) (1,1,L,@cR) (1,2,L,@cM) (1,3,L,@cI) (1,4,L,@cI) (1,5,L,@cX) [1.0] (1,6,R,@cX) (1,7,R,@cI) (1,8,R,@cI) (1,9,R,@cM) (1,10,R,@cR) (1,11,R,@cP)
(2,0,L,@cP) (2,1,L,@cR) (2,2,L,@cM) (2,3,L,@cI) (2,4,L,@cI) (2,5,L,@cX) [1.0] (2,6,R,@cX) (2,7,R,@cI) (2,8,R,@cI) (2,9,R,@cM) (2,10,R,@cR) (2,11,R,@cP)
[y=0.05]
[3.5] (3,3,L,@thumbL) (3,4,L) (3,5,L,@thumbR) [1.0] (3,6,R,@thumbL) (3,7,R) (3,8,R,@thumbR)
"""

[shapes]
cP = { y = 0.55 }
cR = { y = 0.25 }
cM = { y = 0.0 }
cI = { y = 0.10 }
cX = { y = 0.25 }
thumbL = { r = 15.0 }
thumbR = { r = -15.0 }
"#;
        let info = info_of(toml);
        let v = &info.variants[0];
        // Shape nudge affects the center.
        let k00 = key(v, 0, 0);
        assert!(
            approx(k00.rect.x, 0.5) && approx(k00.rect.y, 1.05),
            "got ({}, {})",
            k00.rect.x,
            k00.rect.y
        );
        // The split gap moves the right half.
        assert!(approx(key(v, 0, 6).rect.x, 7.5), "right half x");
        // Thumb shape carries its angle.
        let t = key(v, 3, 3);
        assert!(
            approx(t.rect.x, 4.0) && approx(t.rect.y, 3.55),
            "thumb ({}, {})",
            t.rect.x,
            t.rect.y
        );
        assert!(approx(t.r, 15.0));
        // A shape's own `r` is not a region: no cluster identity.
        assert_eq!(t.pivot, None);
        // 36 grid keys plus 6 thumbs.
        assert_eq!(v.keys.len(), 42);
    }

    #[test]
    fn iso_variant_reflows_to_match_ansi() {
        // ANSI and ISO should keep the first alpha aligned.
        let toml = r#"
rows = 4
cols = 16
map = """
(3,0,@2.25u) (3,14,@isokey) (3,1) (3,2)
"""

[shapes]
isokey = { w = 1.0 }
lsft_iso = { w = 1.25 }

[[variant]]
name = "ansi"
hidden = ["(3,14)"]

[[variant]]
name = "iso"
shapes = { "(3,0)" = "@lsft_iso" }
"#;
        let info = info_of(toml);
        let ansi = &info.variants[0];
        let iso = &info.variants[1];
        // First alpha key aligns across variants.
        assert!(
            approx(key(ansi, 3, 1).rect.x, key(iso, 3, 1).rect.x),
            "ansi {} vs iso {}",
            key(ansi, 3, 1).rect.x,
            key(iso, 3, 1).rect.x
        );
        // ISO-only key visibility differs by variant.
        assert!(ansi.keys.iter().all(|k| !(k.row == 3 && k.col == 14)));
        assert!(iso.keys.iter().any(|k| k.row == 3 && k.col == 14));
    }

    #[test]
    fn empty_blob_decodes_to_empty_layout() {
        assert_eq!(LayoutInfo::from_compressed_blob(&[]).unwrap(), LayoutInfo::empty());
    }

    #[test]
    fn blob_round_trips_through_compression() {
        // Cover both pivot arms and rect2 across the encode/decode pair.
        let toml = "rows = 1\ncols = 2\nmap = \"(0,0,@iso_enter) [r=30@(1.5,0)] (0,1) (e,0)\"";
        let info = info_of(toml);
        let cfg: LayoutTomlConfig = toml::from_str(toml).unwrap();
        let blob = build_layout_blob(&cfg, None).unwrap();
        let decoded = LayoutInfo::from_compressed_blob(&blob).unwrap();
        assert_eq!(decoded, info);
        // ISO Enter carries a second rectangle and no region.
        let v = &decoded.variants[0];
        assert!(v.keys[0].rect2.is_some() && v.keys[0].pivot.is_none());
        let region = Region {
            deg: 30.0,
            px: 1.5,
            py: 0.0,
        };
        assert_eq!(v.keys[1].pivot, Some(region));
        assert_eq!(v.encoders[0].pivot, Some(region));
    }

    /// ANSI/ISO/split-bs 60%: one keymap, three render variants over the superset map.
    const ANSI_ISO_60: &str = r#"
rows = 5
cols = 16
default_variant = "ansi"
map = """
(0,0) (0,1) (0,2) (0,3) (0,4) (0,5) (0,6) (0,7) (0,8) (0,9) (0,10) (0,11) (0,12) (0,13,@bs) (0,14,@bsr)
(1,0,@tab) (1,1) (1,2) (1,3) (1,4) (1,5) (1,6) (1,7) (1,8) (1,9) (1,10) (1,11) (1,12) (1,13)
(2,0,@caps) (2,1) (2,2) (2,3) (2,4) (2,5) (2,6) (2,7) (2,8) (2,9) (2,10) (2,11) (2,12,@enter)
(3,0,@lsft) (3,14,@isokey) (3,1) (3,2) (3,3) (3,4) (3,5) (3,6) (3,7) (3,8) (3,9) (3,10) (3,11,@rsft)
(4,0,@mod) (4,1,@mod) (4,2,@mod) (4,3,@space) (4,9,@mod) (4,10,@mod) (4,11,@mod) (4,12,@mod)
"""

[shapes]
bs = { w = 2.0 }
bsr = { w = 1.0 }
bsl = { w = 1.0 }
tab = { w = 1.5 }
caps = { w = 1.75 }
enter = { w = 2.25 }
isoenter = { w = 1.25, h = 2.0, y = -1.0, w2 = 1.5, h2 = 1.0, x2 = -0.125, y2 = -0.5 }
lsft = { w = 2.25 }
lsft_iso = { w = 1.25 }
isokey = { w = 1.0 }
rsft = { w = 2.75 }
mod = { w = 1.25 }
space = { w = 6.25 }

[[variant]]
name = "ansi"
hidden = ["(3,14)", "(0,14)"]

[[variant]]
name = "iso"
shapes = { "(2,12)" = "@isoenter", "(3,0)" = "@lsft_iso" }
hidden = ["(0,14)"]

[[variant]]
name = "split-bs"
shapes = { "(0,13)" = "@bsl" }
hidden = ["(3,14)"]
"#;

    #[test]
    fn multi_variant_60_percent() {
        let info = info_of(ANSI_ISO_60);
        // Three render variants over one superset.
        assert_eq!(info.variants.len(), 3);
        let names: Vec<_> = info.variants.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["ansi", "iso", "split-bs"]);
        assert_eq!(info.default_variant, 0);

        let ansi = &info.variants[0];
        let iso = &info.variants[1];
        let splitbs = &info.variants[2];

        // Variant visibility changes without changing key identity.
        let has = |v: &Variant, r, c| v.keys.iter().any(|k| k.row == r && k.col == c);
        assert!(!has(ansi, 3, 14) && !has(ansi, 0, 14));
        assert!(has(iso, 3, 14) && !has(iso, 0, 14));
        assert!(has(splitbs, 0, 14) && !has(splitbs, 3, 14));

        // ISO Enter is L-shaped only in the ISO variant.
        assert!(key(iso, 2, 12).rect2.is_some());
        assert!(key(ansi, 2, 12).rect2.is_none());
        assert!(approx(key(iso, 3, 0).rect.w, 1.25)); // LShift shrank for the extra key

        // Reflow keeps the first alpha aligned.
        assert!(
            approx(key(ansi, 3, 1).rect.x, key(iso, 3, 1).rect.x),
            "row-3 alpha must align: ansi {} vs iso {}",
            key(ansi, 3, 1).rect.x,
            key(iso, 3, 1).rect.x
        );

        // Row-width keys preserve the classic stagger.
        assert!(key(ansi, 1, 1).rect.x > key(ansi, 0, 1).rect.x);
        assert!(key(ansi, 2, 1).rect.x > key(ansi, 1, 1).rect.x);
    }

    #[test]
    fn multi_variant_60_blob_is_small() {
        let cfg: LayoutTomlConfig = toml::from_str(ANSI_ISO_60).unwrap();
        let blob = build_layout_blob(&cfg, None).unwrap();
        // Keep the blob BLE-friendly.
        assert!(!blob.is_empty() && blob.len() < 2048, "blob len = {}", blob.len());
        // The blob decodes back to the same layout.
        let back = miniz_oxide::inflate::decompress_to_vec(&blob).unwrap();
        let decoded: LayoutInfo = postcard::from_bytes(&back).unwrap();
        assert_eq!(decoded, build_layout_info(&cfg, None).unwrap().unwrap());
    }

    #[test]
    fn example_nrf52840_numpad_layout() {
        // Mirror the shipped numpad example that uses stock shapes.
        let toml = r#"
rows = 5
cols = 4
map = """
(0,0) (0,1) (0,2) (0,3)
(1,0) (1,1) (1,2) (1,3,@2u_tall)
(2,0) (2,1) (2,2)
(3,0) (3,1) (3,2) (3,3,@2u_tall)
    (4,0,@2u)    (4,1)
"""
"#;
        let info = info_of(toml);
        let v = &info.variants[0];
        assert_eq!(v.keys.len(), 17); // 4 + 4 + 3 + 4 + 2
        // Plus and Enter are 2u tall.
        assert!(approx(key(v, 1, 3).rect.h, 2.0) && approx(key(v, 1, 3).rect.y, 2.0));
        assert!(approx(key(v, 3, 3).rect.h, 2.0) && approx(key(v, 3, 3).rect.y, 4.0));
        // Zero is 2u wide; dot follows it.
        assert!(approx(key(v, 4, 0).rect.w, 2.0) && approx(key(v, 4, 0).rect.x, 1.0));
        assert!(approx(key(v, 4, 1).rect.x, 2.5));
    }

    #[test]
    fn split_corne_36_key_variant() {
        // The 36-key view hides outer pinky columns only.
        let info = info_of(CORNE_SPLIT);
        assert_eq!(info.variants.len(), 2);
        let full = &info.variants[0];
        let mini = &info.variants[1];
        assert_eq!(full.keys.len(), 42); // 36 grid + 6 thumbs
        assert_eq!(mini.keys.len(), 36); // outer pinky columns hidden

        // The split gap separates the inner columns.
        assert!(key(full, 0, 6).rect.x - key(full, 0, 5).rect.x > 1.5);

        // Hiding the left pinky reflows row 0 by 1u.
        assert!(approx(key(full, 0, 1).rect.x - key(mini, 0, 1).rect.x, 1.0));
    }
}
