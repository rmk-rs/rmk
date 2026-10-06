//! Turn KLE keys into an RMK `[layout]` section.
//!
//! RMK's `[layout].map` is the same kind of cursor walk KLE is: the row top maps
//! to KLE `y`, the column cursor to KLE `x`, and a key's center is `cursor +
//! size/2`. So the conversion is nearly one-to-one — horizontal offsets become
//! `[gap]` tokens, per-row vertical offsets become `[y=n]`, a KLE rotation
//! cluster `(r, rx, ry)` becomes an `[r=deg@(px,py)]` region (keys keep their
//! clean flat coordinates), and any non-1u width/height/L-shape becomes an
//! `@shape` (reusing RMK's stock shapes where they fit, generating
//! `[layout.shapes]` entries otherwise). VIA layout options become best-effort
//! `[[layout.variant]]` overlays.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use kle_serial::Key as SerialKey;
use serde_json::Value;

use crate::Generated;
use crate::kle::AnnotatedKey;

const EPS: f64 = 1e-4;

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < EPS
}

/// Format a number for TOML/map use: always keep a decimal point (so the `toml`
/// crate reads it as a float, and the map grammar's `unit` accepts it), trimming
/// noise digits.
fn fmt(v: f64) -> String {
    let r = (v * 1e4).round() / 1e4;
    if approx(r, r.round()) {
        format!("{:.1}", r)
    } else {
        let mut s = format!("{r:.4}");
        while s.ends_with('0') {
            s.pop();
        }
        s
    }
}

pub(crate) struct GenInput<'a> {
    pub(crate) keys: &'a [AnnotatedKey],
    pub(crate) matrix_rows: u8,
    pub(crate) matrix_cols: u8,
    /// `layouts.labels`, used only to name variants.
    pub(crate) labels: Option<&'a Value>,
}

/// A resolved shape in RMK's convention: `(x, y)` nudge the center, `rect2` is
/// `(w2, h2, x2, y2)` with the second rect's center offset from the primary center.
#[derive(Clone, Copy, Debug)]
struct ShapeDesc {
    w: f64,
    h: f64,
    x: f64,
    y: f64,
    r: f64,
    rect2: Option<(f64, f64, f64, f64)>,
}

impl ShapeDesc {
    fn plain() -> Self {
        ShapeDesc {
            w: 1.0,
            h: 1.0,
            x: 0.0,
            y: 0.0,
            r: 0.0,
            rect2: None,
        }
    }

    fn is_plain(&self) -> bool {
        approx(self.w, 1.0)
            && approx(self.h, 1.0)
            && approx(self.x, 0.0)
            && approx(self.y, 0.0)
            && approx(self.r, 0.0)
            && self.rect2.is_none()
    }

    fn key(&self) -> [i64; 10] {
        let q = |v: f64| (v * 1e4).round() as i64;
        let mut values = [0; 10];
        values[..5].copy_from_slice(&[q(self.w), q(self.h), q(self.x), q(self.y), q(self.r)]);
        if let Some((w, h, x, y)) = self.rect2 {
            values[5..].copy_from_slice(&[1, q(w), q(h), q(x), q(y)]);
        }
        values
    }

    fn toml(&self) -> String {
        let mut parts = Vec::new();
        for (label, value, default) in [
            ("w", self.w, 1.0),
            ("h", self.h, 1.0),
            ("x", self.x, 0.0),
            ("y", self.y, 0.0),
            ("r", self.r, 0.0),
        ] {
            if !approx(value, default) {
                parts.push(format!("{label} = {}", fmt(value)));
            }
        }
        if let Some((w2, h2, x2, y2)) = self.rect2 {
            parts.push(format!("w2 = {}", fmt(w2)));
            parts.push(format!("h2 = {}", fmt(h2)));
            parts.push(format!("x2 = {}", fmt(x2)));
            parts.push(format!("y2 = {}", fmt(y2)));
        }
        if parts.is_empty() {
            // A deliberate 1u reset shape (used to shrink a wide key in a variant).
            "{ w = 1.0 }".to_string()
        } else {
            format!("{{ {} }}", parts.join(", "))
        }
    }
}

/// Assigns stock names and records generated shapes in insertion order.
struct ShapeRegistry {
    generated: Vec<([i64; 10], String, ShapeDesc)>,
}

impl ShapeRegistry {
    fn name_for(&mut self, d: &ShapeDesc) -> String {
        // Stock shapes: only pure width/height changes, no nudge/rotation/L-shape.
        if approx(d.x, 0.0) && approx(d.y, 0.0) && approx(d.r, 0.0) && d.rect2.is_none() {
            if approx(d.h, 1.0) {
                for &(name, w) in rmk_config::STOCK_WIDTHS {
                    if approx(d.w, w as f64) {
                        return name.to_string();
                    }
                }
            }
            if approx(d.w, 1.0) && approx(d.h, 2.0) {
                return "2u_tall".to_string();
            }
        }
        let key = d.key();
        if let Some((_, name, _)) = self.generated.iter().find(|(existing, _, _)| *existing == key) {
            return name.clone();
        }
        let name = format!("s{}", self.generated.len() + 1);
        self.generated.push((key, name.clone(), *d));
        name
    }
}

/// True only for a genuine L-shaped cap: the secondary rect is offset or a
/// different size from the primary. (`kle_serial` fills the secondary rect with
/// the primary's size for ordinary keys, which is *not* an L-shape.)
fn has_rect2(k: &SerialKey<f64>) -> bool {
    const E: f64 = 1e-6;
    k.x2.abs() > E
        || k.y2.abs() > E
        || (k.width2.abs() > E && (k.width2 - k.width).abs() > E)
        || (k.height2.abs() > E && (k.height2 - k.height).abs() > E)
}

/// A key's KLE rotation cluster — `(angle, pivot_x, pivot_y)` — or `None` when
/// the key is unrotated (an angle of 0 makes any stale KLE `rx`/`ry` moot).
/// Maps one-to-one onto an RMK `[r=angle@(px,py)]` region.
fn rot_of(k: &SerialKey<f64>) -> Option<(f64, f64, f64)> {
    (!approx(k.rotation, 0.0)).then_some((k.rotation, k.rx, k.ry))
}

/// Rotate `(x, y)` about `(px, py)` by `deg` — KLE/CSS convention: positive is
/// clockwise in screen space.
fn swing(x: f64, y: f64, deg: f64, px: f64, py: f64) -> (f64, f64) {
    let (sin, cos) = deg.to_radians().sin_cos();
    let (dx, dy) = (x - px, y - py);
    (px + dx * cos - dy * sin, py + dx * sin + dy * cos)
}

/// The top-left corner to lay a key at in `region`'s flat frame, so that the
/// region's `[r=deg@(px,py)]` swing lands it exactly where KLE renders it.
///
/// A key inside its own cluster (every key the map walk places) is simply its
/// flat KLE `(x, y)` — that is the whole point of emitting regions: the clean
/// pre-rotation coordinates survive. A frame mismatch (a layout-option
/// alternate rotated differently than the map's canonical key) un-swings the
/// true rendered center back into the region's flat frame instead.
fn region_top_left(k: &SerialKey<f64>, region: Option<(f64, f64, f64)>) -> (f64, f64) {
    if rot_of(k) == region {
        return (k.x, k.y);
    }
    let (cx, cy) = (k.x + k.width / 2.0, k.y + k.height / 2.0);
    let (cx, cy) = match rot_of(k) {
        Some((deg, px, py)) => swing(cx, cy, deg, px, py),
        None => (cx, cy),
    };
    let (cx, cy) = match region {
        Some((deg, px, py)) => swing(cx, cy, -deg, px, py),
        None => (cx, cy),
    };
    (cx - k.width / 2.0, cy - k.height / 2.0)
}

/// KLE position of one key → an RMK shape (relative to its row's baseline `y`),
/// laid out in `region`'s flat frame. `x_nudge` carries any horizontal offset
/// the row's `[gap]` tokens can't (a backward shift between overlapping caps).
/// Returns `None` for a plain 1u key that needs no shape at all.
fn shape_desc(k: &SerialKey<f64>, baseline: f64, x_nudge: f64, region: Option<(f64, f64, f64)>) -> Option<ShapeDesc> {
    let rect2 = has_rect2(k).then(|| {
        let w2 = if approx(k.width2, 0.0) { k.width } else { k.width2 };
        let h2 = if approx(k.height2, 0.0) { k.height } else { k.height2 };
        // KLE's secondary rect is offset from the primary top-left; RMK wants the
        // offset between the two rects' centers.
        let x2 = k.x2 + w2 / 2.0 - k.width / 2.0;
        let y2 = k.y2 + h2 / 2.0 - k.height / 2.0;
        (w2, h2, x2, y2)
    });
    let d = ShapeDesc {
        w: k.width,
        h: k.height,
        x: x_nudge,
        // The vertical part of the flat-frame position; the horizontal part is
        // carried by `[gap]` tokens plus `x_nudge`.
        y: region_top_left(k, region).1 - baseline,
        // The region carries the cluster angle, so a key in its own cluster has
        // r = 0 here; only a frame mismatch leaves a delta (angles add at decode).
        r: k.rotation - region.map_or(0.0, |(deg, ..)| deg),
        rect2,
    };
    (!d.is_plain()).then_some(d)
}

struct VariantOut {
    name: String,
    hidden: Vec<(u8, u8)>,
    shapes: Vec<((u8, u8), String)>,
}

/// Best-effort variant name from `layouts.labels`. A string label is a toggle
/// (choice 1 = the label); an array is a dropdown (`[groupName, c0, c1, …]`).
fn variant_name(labels: Option<&Value>, g: u8, c: u8, used: &mut HashSet<String>) -> String {
    let raw = labels
        .and_then(|l| l.as_array())
        .and_then(|arr| arr.get(g as usize))
        .and_then(|entry| match entry {
            Value::String(s) => Some(s.clone()),
            Value::Array(choices) => {
                let group = choices.first().and_then(Value::as_str).unwrap_or("");
                let choice = choices.get(1 + c as usize).and_then(Value::as_str).unwrap_or("");
                Some(if group.is_empty() {
                    choice.to_string()
                } else {
                    format!("{group} {choice}")
                })
            }
            _ => None,
        })
        .unwrap_or_default();

    let mut name: String = raw
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    name = name.trim_matches('_').to_string();
    if name.is_empty() {
        name = format!("g{g}_c{c}");
    }
    // Keep names unique.
    let base = name.clone();
    let mut n = 1;
    while !used.insert(name.clone()) {
        n += 1;
        name = format!("{base}_{n}");
    }
    name
}

struct EncoderRender {
    id: u8,
    key: SerialKey<f64>,
    row_index: usize,
}

fn synthetic_key(x: f64, y: f64, width: f64, height: f64) -> SerialKey<f64> {
    SerialKey::<f64> {
        x,
        y,
        width,
        height,
        width2: 0.0,
        height2: 0.0,
        ..Default::default()
    }
}

/// Group instance indices by `key(inst)`, preserving first-seen key order.
/// Returns `(order, groups, index)`: keys in first-seen order, the instances per
/// key aligned with `order`, and a key → position lookup.
fn group_first_seen<K: Copy + Eq + std::hash::Hash>(
    insts: &[usize],
    key: impl Fn(usize) -> K,
) -> (Vec<K>, Vec<Vec<usize>>, HashMap<K, usize>) {
    let mut order: Vec<K> = Vec::new();
    let mut index: HashMap<K, usize> = HashMap::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for &inst in insts {
        let k = key(inst);
        match index.get(&k) {
            Some(&i) => groups[i].push(inst),
            None => {
                index.insert(k, order.len());
                order.push(k);
                groups.push(vec![inst]);
            }
        }
    }
    (order, groups, index)
}

pub(crate) fn generate(input: GenInput) -> Result<Generated, String> {
    let mut warnings = Vec::new();

    // 1. Split real switches into matrix keys and Vial encoder switches (drop decals).
    let mut key_insts: Vec<usize> = Vec::new();
    let mut enc_insts: Vec<usize> = Vec::new();
    for (i, k) in input.keys.iter().enumerate() {
        if k.decal {
            continue;
        }
        if k.encoder.is_some() {
            enc_insts.push(i);
        } else if k.matrix.is_some() {
            key_insts.push(i);
        } else {
            warnings.push(format!(
                "key at KLE ({}, {}) has no `row,col` legend — skipped",
                fmt(k.x),
                fmt(k.y)
            ));
        }
    }
    if key_insts.is_empty() {
        return Err("no keys with a matrix position found in the layout".to_string());
    }

    // 2. Group key instances by matrix cell, preserving first-seen order.
    let (order, cells, index) = group_first_seen(&key_insts, |ki| input.keys[ki].matrix.unwrap());
    // A cell repeated with no layout option is a genuine duplicate; only the first
    // is placed (RMK requires unique coordinates).
    for (i, insts) in cells.iter().enumerate() {
        let dups = insts.iter().filter(|&&ki| input.keys[ki].option.is_none()).count();
        if dups > 1 {
            let (r, c) = order[i];
            warnings.push(format!(
                "matrix ({r},{c}) appears {dups} times without a layout option — keeping the first"
            ));
        }
    }

    // 3. Collapse Vial CW/CCW switch pairs into one physical encoder knob.
    let (enc_order, enc_switches, enc_index) = group_first_seen(&enc_insts, |ki| input.keys[ki].encoder.unwrap().0);
    let mut encoders: Vec<EncoderRender> = Vec::new();
    for (i, sw) in enc_switches.iter().enumerate() {
        let id = enc_order[i];
        // Mixed rotation clusters have no exact single knob frame.
        let cluster = rot_of(&input.keys[sw[0]]);
        let uniform = sw.iter().all(|&si| rot_of(&input.keys[si]) == cluster);
        if !uniform {
            warnings.push(format!(
                "encoder {id} mixes rotation clusters — its position is approximate"
            ));
        }
        let min_x = sw.iter().map(|&si| input.keys[si].x).fold(f64::INFINITY, f64::min);
        let min_y = sw.iter().map(|&si| input.keys[si].y).fold(f64::INFINITY, f64::min);
        let max_x = sw
            .iter()
            .map(|&si| input.keys[si].x + input.keys[si].width)
            .fold(f64::NEG_INFINITY, f64::max);
        let max_y = sw
            .iter()
            .map(|&si| input.keys[si].y + input.keys[si].height)
            .fold(f64::NEG_INFINITY, f64::max);
        let (w, h) = if sw.len() > 1 {
            (1.0, 1.0)
        } else {
            (input.keys[sw[0]].width, input.keys[sw[0]].height)
        };
        let mut key = synthetic_key((min_x + max_x) / 2.0 - w / 2.0, (min_y + max_y) / 2.0 - h / 2.0, w, h);
        if let (true, Some((deg, px, py))) = (uniform, cluster) {
            key.rotation = deg;
            key.rx = px;
            key.ry = py;
        }
        encoders.push(EncoderRender {
            id,
            key,
            row_index: input.keys[sw[0]].row_index,
        });
    }
    let mut ids: Vec<u8> = enc_order.clone();
    ids.sort_unstable();
    if ids.iter().enumerate().any(|(want, &id)| id as usize != want) {
        warnings.push(format!(
            "encoder ids are not a dense 0..N range ({ids:?}) — RMK requires contiguous ids"
        ));
    }

    // 4. Bounds. Bump rows/cols if a matrix coordinate exceeds the declared matrix
    //    (0x0 means none was declared — a raw KLE export — so nothing to warn about).
    let max_row = order.iter().map(|&(r, _)| r).max().unwrap();
    let max_col = order.iter().map(|&(_, c)| c).max().unwrap();
    let required_rows = max_row
        .checked_add(1)
        .ok_or("matrix row 255 would require 256 rows, but RMK dimensions are u8")?;
    let required_cols = max_col
        .checked_add(1)
        .ok_or("matrix column 255 would require 256 columns, but RMK dimensions are u8")?;
    let rows = input.matrix_rows.max(required_rows);
    let cols = input.matrix_cols.max(required_cols);
    if (rows, cols) != (input.matrix_rows, input.matrix_cols) && (input.matrix_rows, input.matrix_cols) != (0, 0) {
        warnings.push(format!(
            "declared matrix is {}x{} but keys reach ({max_row},{max_col}); using {rows}x{cols}",
            input.matrix_rows, input.matrix_cols
        ));
    }

    // Canonical instance = the one shown by default (no option, else choice 0).
    let canonical = |insts: &[usize]| -> usize {
        *insts
            .iter()
            .find(|&&ki| input.keys[ki].option.is_none())
            .or_else(|| insts.iter().find(|&&ki| matches!(input.keys[ki].option, Some((_, 0)))))
            .unwrap_or(&insts[0])
    };

    // 5. Order keys and encoders into map units by first appearance (interleaved as
    //    authored), then bucket by source row.
    enum Unit {
        Key(usize),
        Enc(usize),
    }
    let mut units: Vec<Unit> = Vec::new();
    let mut seen_key: HashSet<(u8, u8)> = HashSet::new();
    let mut seen_enc: HashSet<u8> = HashSet::new();
    for k in input.keys {
        if k.decal {
            continue;
        }
        if let Some((id, _)) = k.encoder {
            if seen_enc.insert(id) {
                units.push(Unit::Enc(enc_index[&id]));
            }
        } else if let Some(rc) = k.matrix
            && seen_key.insert(rc)
        {
            units.push(Unit::Key(index[&rc]));
        }
    }
    struct UnitRender<'a> {
        key: &'a SerialKey<f64>,
        row_index: usize,
        option: Option<(u8, u8)>,
    }
    let unit_render = |u: &Unit| match u {
        Unit::Key(ci) => {
            let ki = canonical(&cells[*ci]);
            UnitRender {
                key: &input.keys[ki],
                row_index: input.keys[ki].row_index,
                option: input.keys[ki].option,
            }
        }
        Unit::Enc(ei) => UnitRender {
            key: &encoders[*ei].key,
            row_index: encoders[*ei].row_index,
            option: None,
        },
    };
    let mut buckets: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (ui, u) in units.iter().enumerate() {
        buckets.entry(unit_render(u).row_index).or_default().push(ui);
    }

    let mut reg = ShapeRegistry { generated: Vec::new() };
    let mut map_lines: Vec<String> = Vec::new();
    let mut map_shape: Vec<Option<String>> = vec![None; cells.len()];
    let mut cell_baseline: Vec<f64> = vec![0.0; cells.len()];
    let mut cell_region: Vec<Option<(f64, f64, f64)>> = vec![None; cells.len()];
    let mut prev_baseline: Option<f64> = None;
    // The `[r=...]` region in effect — sticky across lines, mirroring the walk.
    let mut active_region: Option<(f64, f64, f64)> = None;
    // Translate KLE pivot y into the map frame whose first baseline is 0.
    let y0 = buckets
        .values()
        .next()
        .map(|idxs| unit_render(&units[idxs[0]]).key.y)
        .unwrap_or(0.0);

    for unit_idxs in buckets.values() {
        // Emit pre-rotation coordinates; `[r=...]` reapplies the swing.
        let baseline = unit_render(&units[unit_idxs[0]]).key.y;
        if let Some(pb) = prev_baseline {
            let vstep = baseline - pb - 1.0;
            if !approx(vstep, 0.0) {
                map_lines.push(format!("[y={}]", fmt(vstep)));
            }
        }
        let mut cursor_x = 0.0;
        let mut row_tokens = Vec::new();
        for &ui in unit_idxs {
            // Backward gaps ride `x_nudge`; variant alternates reflow instead.
            let g = unit_render(&units[ui]);
            let region = rot_of(g.key);
            if region != active_region {
                row_tokens.push(match region {
                    Some((deg, px, py)) => format!("[r={}@({},{})]", fmt(deg), fmt(px), fmt(py - y0)),
                    None => "[r=0]".to_string(),
                });
                active_region = region;
            }
            let ex = g.key.x;
            let gap = ex - cursor_x;
            let mut x_nudge = 0.0;
            if gap > EPS {
                row_tokens.push(format!("[{}]", fmt(gap)));
            } else if gap < -EPS && g.option.is_none() {
                x_nudge = gap;
            }
            match &units[ui] {
                Unit::Key(ci) => {
                    let ci = *ci;
                    cell_baseline[ci] = baseline;
                    cell_region[ci] = region;
                    let (r, c) = order[ci];
                    let token = match shape_desc(g.key, baseline, x_nudge, region) {
                        None => format!("({r},{c})"),
                        Some(d) => {
                            let name = reg.name_for(&d);
                            map_shape[ci] = Some(name.clone());
                            format!("({r},{c},@{name})")
                        }
                    };
                    row_tokens.push(token);
                }
                Unit::Enc(ei) => {
                    // Encoders are fixed 1u knobs.
                    row_tokens.push(format!("(e,{})", encoders[*ei].id));
                }
            }
            // Shape `x` nudges only the center, not cursor advance.
            cursor_x = ex - x_nudge + g.key.width;
        }
        map_lines.push(row_tokens.join(" "));
        prev_baseline = Some(baseline);
    }

    // 5. Layout options → best-effort variants.
    let mut groups: BTreeMap<u8, BTreeSet<u8>> = BTreeMap::new();
    for insts in &cells {
        for &ki in insts {
            if let Some((g, c)) = input.keys[ki].option {
                groups.entry(g).or_default().insert(c);
            }
        }
    }

    let mut variants: Vec<VariantOut> = Vec::new();
    let mut default_variant = String::new();
    if !groups.is_empty() {
        if groups.len() > 1 {
            warnings.push(format!(
                "{} layout-option groups found — RMK variants are flat, so only \
                 one-group-at-a-time combinations were generated; author cross-group \
                 combinations by hand",
                groups.len()
            ));
        }
        let base: BTreeMap<u8, u8> = groups.keys().map(|&g| (g, 0)).collect();
        let mut used_names = HashSet::new();
        used_names.insert("default".to_string());
        default_variant = "default".to_string();

        // Each variant fixes one group to a non-default choice (others default).
        let mut targets: Vec<(String, BTreeMap<u8, u8>)> = vec![("default".to_string(), base.clone())];
        for (&g, choices) in &groups {
            for &c in choices {
                if c == 0 {
                    continue;
                }
                let mut s = base.clone();
                s.insert(g, c);
                targets.push((variant_name(input.labels, g, c, &mut used_names), s));
            }
        }

        for (name, settings) in targets {
            let mut hidden = Vec::new();
            let mut shapes = Vec::new();
            for ci in 0..cells.len() {
                let matches = |opt: Option<(u8, u8)>| match opt {
                    None => true,
                    Some((g, c)) => settings.get(&g) == Some(&c),
                };
                let chosen = cells[ci]
                    .iter()
                    .copied()
                    .filter(|&ki| matches(input.keys[ki].option))
                    // Prefer the choice-specific instance over the always-on base.
                    .max_by_key(|&ki| input.keys[ki].option.is_some() as u8);
                match chosen {
                    None => hidden.push(order[ci]),
                    Some(ki) => {
                        // Un-swing mismatched clusters into the map cell frame.
                        let name_opt = shape_desc(&input.keys[ki], cell_baseline[ci], 0.0, cell_region[ci])
                            .map(|d| reg.name_for(&d));
                        if name_opt != map_shape[ci] {
                            let ov = name_opt.unwrap_or_else(|| reg.name_for(&ShapeDesc::plain()));
                            shapes.push((order[ci], ov));
                        }
                    }
                }
            }
            variants.push(VariantOut { name, hidden, shapes });
        }
        if variants.len() > 256 {
            warnings.push(format!(
                "{} variants generated but RMK allows at most 256 — truncated",
                variants.len()
            ));
            variants.truncate(256);
        }
    }

    // 6. Render.
    let map_block = map_lines.join("\n");
    let shape_defs: Vec<(String, String)> = reg
        .generated
        .iter()
        .map(|(_, name, desc)| (name.clone(), desc.toml()))
        .collect();

    let ctx = RenderCtx {
        rows,
        cols,
        default_variant: &default_variant,
        map_block: &map_block,
        shape_defs: &shape_defs,
        variants: &variants,
    };
    Ok(Generated {
        layout_toml: render(&ctx),
        warnings,
    })
}

struct RenderCtx<'a> {
    rows: u8,
    cols: u8,
    default_variant: &'a str,
    map_block: &'a str,
    shape_defs: &'a [(String, String)],
    variants: &'a [VariantOut],
}

fn rc_list(items: &[(u8, u8)]) -> String {
    items
        .iter()
        .map(|(r, c)| format!("\"({r},{c})\""))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render(ctx: &RenderCtx) -> String {
    let mut out = String::new();
    out.push_str("[layout]\n");
    out.push_str(&format!("rows = {}\n", ctx.rows));
    out.push_str(&format!("cols = {}\n", ctx.cols));
    if !ctx.default_variant.is_empty() {
        out.push_str(&format!("default_variant = \"{}\"\n", ctx.default_variant));
    }
    out.push_str(&format!("map = \"\"\"\n{}\n\"\"\"\n", ctx.map_block));

    if !ctx.shape_defs.is_empty() {
        out.push('\n');
        out.push_str("[layout.shapes]");
        out.push('\n');
        for (name, def) in ctx.shape_defs {
            out.push_str(&format!("{name} = {def}\n"));
        }
    }

    for v in ctx.variants {
        out.push('\n');
        out.push_str("[[layout.variant]]");
        out.push('\n');
        out.push_str(&format!("name = \"{}\"\n", v.name));
        if !v.hidden.is_empty() {
            out.push_str(&format!("hidden = [{}]\n", rc_list(&v.hidden)));
        }
        if !v.shapes.is_empty() {
            let entries = v
                .shapes
                .iter()
                .map(|((r, c), name)| format!("\"({r},{c})\" = \"@{name}\""))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("shapes = {{ {entries} }}\n"));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::kle::{ParsedKeymap, parse_keymap};

    /// The top-left corner a key *displays* at — KLE's ground truth, used to
    /// verify the converted layout: the key's center swung about its own
    /// cluster origin, handed back as a top-left.
    fn display_top_left(k: &SerialKey<f64>) -> (f64, f64) {
        match rot_of(k) {
            None => (k.x, k.y),
            Some((deg, px, py)) => {
                let (cx, cy) = swing(k.x + k.width / 2.0, k.y + k.height / 2.0, deg, px, py);
                (cx - k.width / 2.0, cy - k.height / 2.0)
            }
        }
    }

    fn gen_layout(v: Value, rows: u8, cols: u8) -> Generated {
        let parsed = parse_keymap(&v).unwrap();
        gen_parsed(&parsed, rows, cols)
    }

    fn gen_parsed(parsed: &ParsedKeymap, rows: u8, cols: u8) -> Generated {
        generate(GenInput {
            keys: &parsed.keys,
            matrix_rows: rows,
            matrix_cols: cols,
            labels: None,
        })
        .unwrap()
    }

    /// Every generated `[layout]` must round-trip through RMK's own builder.
    fn assert_valid(g: &Generated) {
        crate::decode_layout(&g.layout_toml)
            .unwrap_or_else(|e| panic!("rmk-config rejected generated layout: {e}\n---\n{}", g.layout_toml));
    }

    #[test]
    fn plain_grid() {
        let g = gen_layout(json!([["0,0", "0,1", "0,2"], ["1,0", "1,1", "1,2"]]), 2, 3);
        assert!(g.layout_toml.contains("rows = 2"));
        assert!(g.layout_toml.contains("cols = 3"));
        assert!(g.layout_toml.contains("(0,0) (0,1) (0,2)"));
        assert!(g.layout_toml.contains("(1,0) (1,1) (1,2)"));
        // KLE carries no keycodes — the output is render-only, no [keymap].
        assert!(!g.layout_toml.contains("[keymap]"));
        assert!(!g.layout_toml.contains("[layout.shapes]")); // all 1u
        assert_valid(&g);
    }

    #[test]
    fn stock_widths_need_no_shape_defs() {
        // A 2u and a 6.25u space map to stock @2u / @6.25u — no [layout.shapes].
        let g = gen_layout(json!([[{"w": 2.0}, "0,0"], [{"w": 6.25}, "1,0"]]), 2, 1);
        assert!(g.layout_toml.contains("(0,0,@2u)"));
        assert!(g.layout_toml.contains("(1,0,@6.25u)"));
        assert!(!g.layout_toml.contains("[layout.shapes]"));
        assert_valid(&g);
    }

    #[test]
    fn tall_key_uses_2u_tall() {
        let g = gen_layout(json!([[{"h": 2.0}, "0,0"]]), 1, 1);
        assert!(g.layout_toml.contains("(0,0,@2u_tall)"));
        assert_valid(&g);
    }

    #[test]
    fn custom_width_generates_shape() {
        let g = gen_layout(json!([[{"w": 1.3}, "0,0", "0,1"]]), 1, 2);
        assert!(g.layout_toml.contains("[layout.shapes]"));
        assert!(g.layout_toml.contains("s1 = { w = 1.3 }"));
        assert!(g.layout_toml.contains("(0,0,@s1)"));
        assert_valid(&g);
    }

    #[test]
    fn horizontal_gap_between_split_halves() {
        // Two keys, then a 1u gap, then two more on the same row.
        let g = gen_layout(json!([["0,0", "0,1", {"x": 1.0}, "0,2", "0,3"]]), 1, 4);
        assert!(g.layout_toml.contains("(0,0) (0,1) [1.0] (0,2) (0,3)"));
        assert_valid(&g);
    }

    #[test]
    fn raised_row_uses_negative_vstep() {
        // The rp2040 vial.json trick: last row jumps up 2 units.
        let g = gen_layout(
            json!([
                ["0,0", "0,1", "0,2"],
                ["1,0", "1,1", "1,2"],
                ["2,0", "2,1", "2,2"],
                [{"y": -2.0, "x": 4.0}, "3,0", "3,2"],
            ]),
            4,
            3,
        );
        assert!(g.layout_toml.contains("[y=-2.0]"));
        assert!(g.layout_toml.contains("[4.0] (3,0)"));
        assert_valid(&g);
    }

    #[test]
    fn split_backspace_becomes_a_variant() {
        // Default: 2u backspace at (0,13). Option 0,1: two 1u keys (0,13)+(0,14).
        let g = gen_layout(
            json!([[
                {"w": 2.0}, "0,13",
                "0,13\n\n\n\n\n\n\n\n\n0,1", "0,14\n\n\n\n\n\n\n\n\n0,1"
            ]]),
            1,
            15,
        );
        // Map carries the default 2u key and the extra split key.
        assert!(g.layout_toml.contains("(0,13,@2u)"));
        assert!(g.layout_toml.contains("[[layout.variant]]"));
        assert!(g.layout_toml.contains("name = \"default\""));
        // The default view hides the split-only key; the split view reshapes 0,13.
        assert!(g.layout_toml.contains("hidden = [\"(0,14)\"]"));
        assert_valid(&g);
    }

    #[test]
    fn identical_shapes_are_deduped() {
        // Two keys of the same non-stock width share one generated shape.
        let g = gen_layout(json!([[{"w": 1.3}, "0,0", {"w": 1.3}, "0,1"]]), 1, 2);
        assert_eq!(g.layout_toml.matches("s1 = ").count(), 1);
        assert!(!g.layout_toml.contains("s2 = "));
        assert!(g.layout_toml.contains("(0,0,@s1)"));
        assert!(g.layout_toml.contains("(0,1,@s1)"));
        assert_valid(&g);
    }

    #[test]
    fn rotation_resolves_to_the_true_center() {
        // A 1u key at (1,0) rotated 90° about the origin: its center swings from
        // (1.5, 0.5) to (-0.5, 1.5), so the displayed top-left is (-1, 1).
        let mut k = SerialKey::<f64> {
            x: 1.0,
            rotation: 90.0,
            ..Default::default()
        };
        let (ex, ey) = display_top_left(&k);
        assert!(approx(ex, -1.0) && approx(ey, 1.0), "got ({ex}, {ey})");
        // An unrotated key is returned untouched.
        k.rotation = 0.0;
        assert_eq!(display_top_left(&k), (1.0, 0.0));
    }

    #[test]
    fn rotated_key_becomes_a_rotation_region() {
        // KLE rotation clusters become `[r=...]` regions, not baked shapes.
        let g = gen_layout(json!([[{"r": 30, "rx": 3, "ry": 1, "x": 1}, "0,0"]]), 1, 1);
        assert!(
            g.layout_toml.contains("[r=30.0@(3.0,0.0)] [4.0] (0,0)"),
            "{}",
            g.layout_toml
        );
        assert!(!g.layout_toml.contains("[layout.shapes]"), "{}", g.layout_toml);
        assert_valid(&g);
    }

    #[test]
    fn corne_thumbs_emit_readable_regions() {
        // The corne fixture's four rotated thumb keys each carry a KLE cluster;
        // the map reproduces them as regions and no shape bakes an angle.
        let (parsed, rows, cols) = load_fixture("corne.json");
        let g = gen_parsed(&parsed, rows, cols);
        assert!(g.layout_toml.contains("[r=15.0@(4.5,8.1)]"), "{}", g.layout_toml);
        assert!(g.layout_toml.contains("[r=-15.0@(12.0,8.1)]"), "{}", g.layout_toml);
        assert!(
            !g.layout_toml.contains("r ="),
            "no baked rotation shapes:\n{}",
            g.layout_toml
        );
        assert_valid(&g);
    }

    #[test]
    fn auto_assigned_plain_kle_generates_and_validates() {
        // A raw KLE export with label legends: the row-major fallback supplies the
        // matrix, and an undeclared (0x0) matrix is derived without a warning.
        let mut parsed = parse_keymap(&json!([["Esc", "Q", "W"], [{"w": 1.5}, "Tab", "A"]])).unwrap();
        crate::kle::assign_matrix_by_position(&mut parsed).unwrap();
        let g = generate(GenInput {
            keys: &parsed.keys,
            matrix_rows: 0,
            matrix_cols: 0,
            labels: None,
        })
        .unwrap();
        assert!(g.layout_toml.contains("rows = 2"));
        assert!(g.layout_toml.contains("cols = 3"));
        assert!(g.layout_toml.contains("(0,0) (0,1) (0,2)"));
        assert!(g.layout_toml.contains("(1,0,@1.5u) (1,1)"));
        assert!(g.warnings.is_empty(), "{:?}", g.warnings);
        assert_valid(&g);
    }

    // Faithfulness: decode the blob and compare against kle-serial geometry.

    /// The canonical (default-shown) instance of a cell — mirrors `generate`.
    fn canon(insts: &[usize], parsed: &ParsedKeymap) -> usize {
        *insts
            .iter()
            .find(|&&ki| parsed.keys[ki].option.is_none())
            .or_else(|| insts.iter().find(|&&ki| matches!(parsed.keys[ki].option, Some((_, 0)))))
            .unwrap_or(&insts[0])
    }

    /// (row,col) → (center_x, center_y, w, h, r, rect2) as kle-serial sees
    /// it. `rect2` is `(w2, h2, center_dx, center_dy)` in the key's local frame.
    type ExpectedRender = HashMap<(u8, u8), (f64, f64, f64, f64, f64, Option<(f64, f64, f64, f64)>)>;

    /// What kle-serial says each default-shown key's center/size/rotation is.
    fn expected_default(parsed: &ParsedKeymap) -> ExpectedRender {
        let mut idx: HashMap<(u8, u8), usize> = HashMap::new();
        let mut cells: Vec<Vec<usize>> = Vec::new();
        for (ki, key) in parsed.keys.iter().enumerate() {
            let annotation = &parsed.keys[ki];
            if key.decal || annotation.matrix.is_none() {
                continue;
            }
            let rc = annotation.matrix.unwrap();
            match idx.get(&rc) {
                Some(&i) => cells[i].push(ki),
                None => {
                    idx.insert(rc, cells.len());
                    cells.push(vec![ki]);
                }
            }
        }
        let mut out = HashMap::new();
        for insts in &cells {
            let ki = canon(insts, parsed);
            let k = &parsed.keys[ki];
            let annotation = &parsed.keys[ki];
            if annotation.option.is_none() || matches!(annotation.option, Some((_, 0))) {
                let (tlx, tly) = display_top_left(k);
                let rect2 = has_rect2(k).then(|| {
                    let w2 = if approx(k.width2, 0.0) { k.width } else { k.width2 };
                    let h2 = if approx(k.height2, 0.0) { k.height } else { k.height2 };
                    (
                        w2,
                        h2,
                        k.x2 + w2 / 2.0 - k.width / 2.0,
                        k.y2 + h2 / 2.0 - k.height / 2.0,
                    )
                });
                out.insert(
                    annotation.matrix.unwrap(),
                    (
                        tlx + k.width / 2.0,
                        tly + k.height / 2.0,
                        k.width,
                        k.height,
                        k.rotation,
                        rect2,
                    ),
                );
            }
        }
        out
    }

    /// Convert, build the blob, and decode it exactly as the host client does.
    fn decode_info(parsed: &ParsedKeymap, rows: u8, cols: u8) -> rynk::LayoutInfo {
        let g = gen_parsed(parsed, rows, cols);
        crate::decode_layout(&g.layout_toml).unwrap()
    }

    fn decode_default(parsed: &ParsedKeymap, rows: u8, cols: u8) -> Vec<rynk::layout::Key> {
        let info = decode_info(parsed, rows, cols);
        info.variants
            .into_iter()
            .nth(info.default_variant as usize)
            .unwrap()
            .keys
    }

    /// The decoded default variant reproduces kle-serial's rendering, up to one
    /// whole-board translation (the display frame's origin is free).
    fn assert_faithful(decoded: &[rynk::layout::Key], expected: &ExpectedRender, ctx: &str) {
        assert_eq!(decoded.len(), expected.len(), "{ctx}: key count");
        let close = |a: f64, b: f64| (a - b).abs() < 5e-3;
        let k0 = &decoded[0];
        let e0 = expected[&(k0.row, k0.col)];
        let (ox, oy) = (k0.rect.x as f64 - e0.0, k0.rect.y as f64 - e0.1);
        for k in decoded {
            let e = expected
                .get(&(k.row, k.col))
                .unwrap_or_else(|| panic!("{ctx}: decoded ({},{}) absent from kle set", k.row, k.col));
            assert!(
                close(k.rect.x as f64 - e.0, ox) && close(k.rect.y as f64 - e.1, oy),
                "{ctx}: center ({},{}) decoded ({:.4},{:.4}) vs kle ({:.4},{:.4}), frame off ({ox:.4},{oy:.4})",
                k.row,
                k.col,
                k.rect.x,
                k.rect.y,
                e.0,
                e.1
            );
            assert!(
                close(k.rect.w as f64, e.2) && close(k.rect.h as f64, e.3) && close(k.r as f64, e.4),
                "{ctx}: size/rot ({},{}) decoded (w{},h{},r{}) vs kle (w{},h{},r{})",
                k.row,
                k.col,
                k.rect.w,
                k.rect.h,
                k.r,
                e.2,
                e.3,
                e.4
            );
            match (&k.rect2, e.5) {
                (Some(actual), Some((w2, h2, dx2, dy2))) => assert!(
                    close(actual.w as f64, w2)
                        && close(actual.h as f64, h2)
                        && close((actual.x - k.rect.x) as f64, dx2)
                        && close((actual.y - k.rect.y) as f64, dy2),
                    "{ctx}: rect2 ({},{}) decoded {:?} vs kle {:?}",
                    k.row,
                    k.col,
                    actual,
                    e.5
                ),
                (None, None) => {}
                (actual, expected) => panic!(
                    "{ctx}: rect2 presence ({},{}) decoded {} vs kle {}",
                    k.row,
                    k.col,
                    actual.is_some(),
                    expected.is_some()
                ),
            }
        }
    }

    #[test]
    fn overlapping_keys_dont_drift_the_row() {
        // Overlap nudge must not drag following keys.
        let km = json!([["0,0", {"x": -0.25}, "0,1", "0,2"]]);
        let parsed = parse_keymap(&km).unwrap();
        assert_faithful(&decode_default(&parsed, 1, 3), &expected_default(&parsed), "overlap");
    }

    #[test]
    fn roundtrip_preserves_widths_and_tall_keys() {
        let km = json!([
            [{"w": 1.5}, "0,0", "0,1", "0,2", {"w": 2.0}, "0,3"],
            ["1,0", {"h": 2.0}, "1,1", "1,2", "1,3"],
        ]);
        let parsed = parse_keymap(&km).unwrap();
        assert_faithful(
            &decode_default(&parsed, 2, 4),
            &expected_default(&parsed),
            "widths+tall",
        );
    }

    #[test]
    fn roundtrip_preserves_rotation() {
        // The decoded centers must match kle-serial's *rotated* centers (the
        // exactness we implemented), and the angle must survive.
        let km = json!([["0,0", "0,1"], [{"r": 20, "rx": 2, "ry": 1}, "1,0", "1,1"]]);
        let parsed = parse_keymap(&km).unwrap();
        let dec = decode_default(&parsed, 2, 2);
        assert_faithful(&dec, &expected_default(&parsed), "rotation");
        assert!(dec.iter().any(|k| (k.r - 20.0).abs() < 1e-3), "rotation carried");
    }

    #[test]
    fn cluster_pivot_survives_decode_and_reexport() {
        // KLE cluster → map region → decoded pivot metadata → re-exported KLE
        // cluster with the same pivot; the flat key stays out.
        let km = json!([["0,0"], [{"r": 20, "rx": 2, "ry": 1}, "1,0", "1,1"]]);
        let parsed0 = parse_keymap(&km).unwrap();
        let info = decode_info(&parsed0, 2, 2);
        let dv = &info.variants[info.default_variant as usize];
        let pivot = |r, c| dv.keys.iter().find(|k| (k.row, k.col) == (r, c)).unwrap().pivot;
        assert_eq!(pivot(0, 0), None);
        let reg = pivot(1, 0).expect("rotated key carries its region");
        assert_eq!(Some(reg), pivot(1, 1), "one shared cluster");
        assert!(
            (reg.deg - 20.0).abs() < 1e-3 && (reg.px - 2.0).abs() < 1e-3 && (reg.py - 1.0).abs() < 1e-3,
            "region ({}, {}, {})",
            reg.deg,
            reg.px,
            reg.py
        );

        // Re-export: geometry faithful, and the pair shares one (r, rx, ry).
        let regenerated = crate::to_kle::variant_to_kle(dv);
        let parsed1 = parse_keymap(&regenerated).unwrap();
        assert_faithful(&decode_default(&parsed1, 2, 2), &expected_default(&parsed0), "reexport");
        let cluster = |rc| {
            parsed1
                .keys
                .iter()
                .find(|k| k.matrix == Some(rc))
                .map(|k| (k.rotation, k.rx, k.ry))
                .unwrap()
        };
        assert_eq!(cluster((1, 0)), cluster((1, 1)), "shared cluster in the export");
        assert!((cluster((1, 0)).0 - 20.0).abs() < 1e-4);
    }

    #[test]
    fn roundtrip_ansi60_fixture_default_variant() {
        let path = format!("{}/tests/fixtures/ansi60_splitbs.json", env!("CARGO_MANIFEST_DIR"));
        let root: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let keymap = root.get("layouts").and_then(|l| l.get("keymap")).unwrap();
        let parsed = parse_keymap(keymap).unwrap();
        // The default view hides the split-only (0,14), so it's absent both sides.
        let exp = expected_default(&parsed);
        assert!(!exp.contains_key(&(0, 14)));
        assert_faithful(&decode_default(&parsed, 5, 15), &exp, "ansi60");

        // The Split_Backspace variant shrinks (0,13) to 1u and reflows (0,14) to
        // sit exactly one unit to its right.
        let info = decode_info(&parsed, 5, 15);
        // With labels omitted here the variant is auto-named; it's the only
        // non-default one.
        let split = info.variants.iter().find(|v| v.name != "default").unwrap();
        let at = |r, c| split.keys.iter().find(|k| k.row == r && k.col == c).unwrap();
        assert!((at(0, 13).rect.w - 1.0).abs() < 5e-3, "split (0,13) shrank to 1u");
        assert!((at(0, 14).rect.w - 1.0).abs() < 5e-3, "split (0,14) present");
        assert!(
            (at(0, 14).rect.x - at(0, 13).rect.x - 1.0).abs() < 5e-3,
            "split key reflowed adjacent"
        );
    }

    #[test]
    fn roundtrip_preserves_encoders() {
        // Row 1 is two encoders, each a Vial CW/CCW pair of side-by-side 1u
        // switches, with a 0.5u gap between the knobs.
        let km = json!([
            ["0,0", "0,1"],
            [
                "0,0\n\n\n\n\n\n\n\n\ne", "0,1\n\n\n\n\n\n\n\n\ne",
                {"x": 0.5}, "1,0\n\n\n\n\n\n\n\n\ne", "1,1\n\n\n\n\n\n\n\n\ne"
            ],
        ]);
        let parsed = parse_keymap(&km).unwrap();
        let info = decode_info(&parsed, 1, 2);
        let dv = &info.variants[info.default_variant as usize];

        // The encoder switches did NOT become phantom matrix keys.
        assert_faithful(&dv.keys, &expected_default(&parsed), "enc-board keys");

        // Two knobs, each a 1u knob at the center of its CW/CCW pair; knob 0
        // centered at (1, 1.5), knob 1 a 0.5u gap further right.
        assert_eq!(dv.encoders.len(), 2);
        let e = |id| dv.encoders.iter().find(|e| e.id == id).unwrap();
        assert!(
            (e(0).x - 1.0).abs() < 5e-3 && (e(0).y - 1.5).abs() < 5e-3,
            "e0 at ({},{})",
            e(0).x,
            e(0).y
        );
        assert!((e(1).x - 3.5).abs() < 5e-3, "e1 x = {}", e(1).x);
    }

    /// Load a committed fixture (vial.json or raw KLE export) as (keys, matrix
    /// rows, matrix cols), mirroring the binary's input handling — including the
    /// row-major matrix fallback for plain KLE legends.
    fn load_fixture(name: &str) -> (ParsedKeymap, u8, u8) {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        let root: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let keymap = if root.is_array() {
            &root
        } else {
            root.get("layouts").and_then(|l| l.get("keymap")).unwrap()
        };
        let mut parsed = parse_keymap(keymap).unwrap();
        if !parsed.has_matrix_or_encoder() {
            crate::kle::assign_matrix_by_position(&mut parsed).unwrap();
        }
        let dim = |k| crate::matrix_dim(&root, k).unwrap();
        (parsed, dim("rows"), dim("cols"))
    }

    /// Every committed fixture (sorted) — sweeps must cover them all.
    fn fixture_names() -> Vec<String> {
        let dir = format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"));
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert!(
            names.len() >= 3,
            "expected the committed fixtures, found {}",
            names.len()
        );
        names
    }

    #[test]
    fn roundtrip_all_fixtures() {
        for name in fixture_names() {
            let (parsed, rows, cols) = load_fixture(&name);
            let info = decode_info(&parsed, rows, cols);
            let dv = &info.variants[info.default_variant as usize];
            assert_faithful(&dv.keys, &expected_default(&parsed), &name);
            let enc_ids: BTreeSet<u8> = parsed
                .keys
                .iter()
                .filter_map(|annotation| annotation.encoder.map(|(id, _)| id))
                .collect();
            assert_eq!(dv.encoders.len(), enc_ids.len(), "{name}: encoder count");
        }
    }

    #[test]
    fn corne_fixture_is_a_42key_split_with_rotated_thumbs() {
        let (parsed, rows, cols) = load_fixture("corne.json");
        let info = decode_info(&parsed, rows, cols);
        let dv = &info.variants[info.default_variant as usize];
        assert_eq!(dv.keys.len(), 42);
        assert_eq!(dv.encoders.len(), 0);
        // The four thumb keys are rotated ±15 / ±30 about their cluster origins.
        assert_eq!(dv.keys.iter().filter(|k| k.r.abs() > 1e-3).count(), 4);
        assert_faithful(&dv.keys, &expected_default(&parsed), "corne");
    }

    #[test]
    fn balice65_fixture_is_rotated_with_one_encoder() {
        let (parsed, rows, cols) = load_fixture("balice65.json");
        let info = decode_info(&parsed, rows, cols);
        let dv = &info.variants[info.default_variant as usize];
        // Exactly 75 matrix keys (77 KLE entries minus the CW/CCW encoder pair);
        // assert_faithful alone can't see keys dropped from both sides at parse.
        assert_eq!(dv.keys.len(), 75);
        assert_eq!(dv.encoders.len(), 1);
        // Nearly every key carries its own rotation angle — the exactness stress test.
        assert!(
            dv.keys.iter().filter(|k| k.r.abs() > 1e-3).count() > 50,
            "expected most keys rotated"
        );
        assert_faithful(&dv.keys, &expected_default(&parsed), "balice65");
    }

    /// Per-encoder switch footprint (id → center_x, center_y, w, h) as the
    /// bounding box of its KLE switches. The knob RMK stores is 1u at that
    /// center, and the reverse direction re-emits the standard side-by-side
    /// CW/CCW pair, so for conventionally drawn boards the footprint (not just
    /// the center) survives the round-trip.
    fn encoder_bbox(parsed: &ParsedKeymap) -> BTreeMap<u8, (f64, f64, f64, f64)> {
        let mut ext: BTreeMap<u8, (f64, f64, f64, f64)> = BTreeMap::new();
        for key in &parsed.keys {
            let Some((id, _)) = key.encoder else {
                continue;
            };
            let e = ext
                .entry(id)
                .or_insert((f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY));
            e.0 = e.0.min(key.x);
            e.1 = e.1.min(key.y);
            e.2 = e.2.max(key.x + key.width);
            e.3 = e.3.max(key.y + key.height);
        }
        ext.into_iter()
            .map(|(id, (x0, y0, x1, y1))| (id, ((x0 + x1) / 2.0, (y0 + y1) / 2.0, x1 - x0, y1 - y0)))
            .collect()
    }

    /// Full `vial.json → keyboard.toml → vial.json`: forward to the RMK layout,
    /// reverse it back to KLE, re-parse, and require the render to match the
    /// original (up to one whole-board translation — the display frame is free).
    fn assert_vial_roundtrip(parsed0: &ParsedKeymap, rows: u8, cols: u8, ctx: &str) {
        let info = decode_info(parsed0, rows, cols);
        let dv = &info.variants[info.default_variant as usize];
        let regenerated = crate::to_kle::variant_to_kle(dv); // keyboard.toml → vial keymap
        let parsed1 = parse_keymap(&regenerated).unwrap();

        let g0 = expected_default(parsed0);
        let g1 = expected_default(&parsed1);
        assert_eq!(g0.len(), g1.len(), "{ctx}: key count {} vs {}", g0.len(), g1.len());
        let close = |a: f64, b: f64| (a - b).abs() < 5e-3;
        // Frame offset between the original and RMK-framed regenerated layout.
        let (rc0, a0) = g0.iter().next().unwrap();
        let b0 = g1.get(rc0).unwrap();
        let (ox, oy) = (b0.0 - a0.0, b0.1 - a0.1);
        for (rc, a) in &g0 {
            let b = g1.get(rc).unwrap_or_else(|| panic!("{ctx}: {rc:?} lost in round-trip"));
            assert!(
                close(b.0 - a.0, ox) && close(b.1 - a.1, oy),
                "{ctx} {rc:?} center {a:?} -> {b:?}"
            );
            assert!(
                close(a.2, b.2) && close(a.3, b.3) && close(a.4, b.4),
                "{ctx} {rc:?} size/rot {a:?} -> {b:?}"
            );
            match (a.5, b.5) {
                (Some(a2), Some(b2)) => assert!(
                    close(a2.0, b2.0) && close(a2.1, b2.1) && close(a2.2, b2.2) && close(a2.3, b2.3),
                    "{ctx} {rc:?} rect2 {a2:?} -> {b2:?}"
                ),
                (None, None) => {}
                (a2, b2) => panic!("{ctx} {rc:?} rect2 presence {a2:?} -> {b2:?}"),
            }
        }
        let e0 = encoder_bbox(parsed0);
        let e1 = encoder_bbox(&parsed1);
        assert_eq!(
            e0.keys().copied().collect::<Vec<_>>(),
            e1.keys().copied().collect::<Vec<_>>(),
            "{ctx}: encoder ids"
        );
        for (id, a) in &e0 {
            let b = &e1[id];
            assert!(
                close(b.0 - a.0, ox) && close(b.1 - a.1, oy) && close(a.2, b.2) && close(a.3, b.3),
                "{ctx} encoder {id}: {a:?} -> {b:?}"
            );
        }
        // Cluster fidelity: rotated keys sharing a KLE (r, rx, ry) cluster must
        // still share one after the round-trip, pivots moved by the frame offset.
        let clusters = |p: &ParsedKeymap| -> BTreeMap<(u8, u8), (f64, f64, f64)> {
            p.keys
                .iter()
                .filter(|k| k.encoder.is_none())
                .filter_map(|k| k.matrix.zip(rot_of(k)))
                .collect()
        };
        let (c0, c1) = (clusters(parsed0), clusters(&parsed1));
        assert_eq!(
            c0.keys().collect::<Vec<_>>(),
            c1.keys().collect::<Vec<_>>(),
            "{ctx}: rotated key set"
        );
        for (rc, (r0, px0, py0)) in &c0 {
            let (r1, px1, py1) = c1[rc];
            assert!(
                close(r1, *r0) && close(px1 - px0, ox) && close(py1 - py0, oy),
                "{ctx} {rc:?}: cluster ({r0},{px0},{py0}) -> ({r1},{px1},{py1})"
            );
        }
    }

    #[test]
    fn vial_toml_vial_roundtrip_all_fixtures() {
        for name in fixture_names() {
            let (parsed, rows, cols) = load_fixture(&name);
            assert_vial_roundtrip(&parsed, rows, cols, &name);
        }
    }

    #[test]
    fn ansi104_big_enter_loopback_preserves_secondary_rect() {
        let (parsed, rows, cols) = load_fixture("ansi104_bigenter.json");
        assert_eq!(parsed.keys.iter().filter(|key| has_rect2(key)).count(), 1);
        assert_vial_roundtrip(&parsed, rows, cols, "ansi104_bigenter");
    }

    #[test]
    fn roundtrip_all_repo_examples() {
        let root = format!("{}/../../examples", env!("CARGO_MANIFEST_DIR"));
        let root = std::path::Path::new(&root);
        if !root.exists() {
            return; // built outside the RMK tree — nothing to sweep
        }
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(rd) = std::fs::read_dir(dir) else {
                return;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.file_name().is_some_and(|n| n == "vial.json") {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        walk(root, &mut files);
        assert!(
            files.len() > 10,
            "expected many example vial.json, found {}",
            files.len()
        );
        for f in &files {
            let root_json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
            let Some(keymap) = root_json.get("layouts").and_then(|l| l.get("keymap")) else {
                continue;
            };
            let dim = |k| crate::matrix_dim(&root_json, k).unwrap();
            let parsed = parse_keymap(keymap).unwrap();
            let ctx = f.strip_prefix(root).unwrap().to_string_lossy().into_owned();
            let info = decode_info(&parsed, dim("rows"), dim("cols"));
            let dv = &info.variants[info.default_variant as usize];
            // Forward faithfulness: vial.json → toml → decode matches kle-serial.
            assert_faithful(&dv.keys, &expected_default(&parsed), &ctx);
            // Every distinct Vial encoder index becomes exactly one decoded knob.
            let enc_ids: BTreeSet<u8> = parsed
                .keys
                .iter()
                .filter_map(|annotation| annotation.encoder.map(|(id, _)| id))
                .collect();
            assert_eq!(dv.encoders.len(), enc_ids.len(), "{ctx}: encoder count");
            // Full round-trip: vial.json → toml → vial.json preserves the rendered layout.
            assert_vial_roundtrip(&parsed, dim("rows"), dim("cols"), &ctx);
        }
    }
}
