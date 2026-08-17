# rynk-kle

Convert a physical keyboard layout between [KLE](http://www.keyboard-layout-editor.com/)/[Vial](https://get.vial.today/) JSON and RMK/Rynk's `[layout]` section — as a library. The `rmkit layout` CLI wraps it; the `wasm` feature exposes the same pipeline to JavaScript.

- **Forward** — `convert_kle(&serde_json::Value)`: a raw KLE JSON export or a `vial.json` (same KLE blob wrapped in `layouts.keymap`) becomes a `Generated { layout_toml, warnings }`. Key positions, cap sizes, split gaps, rotation, ISO/L-shaped caps, encoders, and VIA layout options are converted to `map` tokens plus `[layout.shapes]` / `[[layout.variant]]` entries. KLE carries no keycodes, so no `[keymap]` is emitted.
- **Reverse** — `keyboard_toml_to_vial(&str)`: a `keyboard.toml`'s `[layout]` back into a minimal `vial.json` (default variant, encoders as Vial CW/CCW switch pairs).
- **Decode** — `decode_layout(&str)`: any `[layout]` TOML (a full `keyboard.toml` or a bare `rows`/`cols`/`map` snippet) into `layout::LayoutInfo` (re-exported from `rynk`), through `rmk_config::layout_info_from_toml` — the same builder that produces the blob the firmware serves over `GetLayout`, so what you get is exactly what a Rynk host decodes from that blob.

The unit tests in `src/to_layout.rs` feed every generated `[layout]` back through `decode_layout`, so RMK's own builder must accept it, and verify over the `tests/fixtures/*.json` boards that the rendered layout is preserved through `vial.json → [layout] → vial.json`.

## Web

```sh
wasm-pack build --target web --features wasm
```

exports string-in bindings: `convert_kle(json)` (a `{ layout_toml, warnings }` object), `keyboard_toml_to_vial(toml)` (the `vial.json` as a pretty-printed JSON string), and `decode_layout(toml)` (a `LayoutInfo` object for drawing a preview).
