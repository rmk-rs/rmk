use core::cell::RefCell;

use embassy_time::Duration;
use rmk_types::action::{EncoderAction, KeyAction};
use rmk_types::fork::Fork;
use rmk_types::morse::{Morse, MorseProfile};
use rmk_types::sticky::StickyProfile;
#[cfg(all(feature = "storage", feature = "host"))]
use {
    crate::{boot::reboot_keyboard, storage::Storage},
    embedded_storage_async::nor_flash::NorFlash,
};

use crate::config::{BehaviorConfig, Hand, MouseKeyConfig, PositionalConfig};
use crate::event::{KeyboardEvent, KeyboardEventPos, LayerChangeEvent, publish_event};
use crate::input_device::rotary_encoder::Direction;
use crate::keyboard::combo::Combo;
use crate::keyboard::macros::Macros;
#[cfg(feature = "host_lock")]
use crate::matrix::MatrixState;

pub(crate) const HOLD_BUFFER_SIZE: usize = 16;

/// All allocated data needed to build a [`KeyMap`].
pub struct KeymapData<const ROW: usize, const COL: usize, const NUM_LAYER: usize, const NUM_ENCODER: usize = 0> {
    /// Per-layer key actions
    pub(crate) keymap: [[[KeyAction; COL]; ROW]; NUM_LAYER],
    /// Per-layer encoder actions
    pub(crate) encoder_map: [[EncoderAction; NUM_ENCODER]; NUM_LAYER],
    /// Per-layer activation flags
    layer_state: [bool; NUM_LAYER],
    /// Layer cache for key positions
    layer_cache: [[u8; COL]; ROW],
    /// Layer cache for encoder directions
    encoder_layer_cache: [[u8; 2]; NUM_ENCODER],
    /// VIA/Vial layout options; persisted via `LayoutOption`
    pub(crate) layout_option: u32,
    /// The macro buffer, see [`crate::keyboard::macros`].
    #[cfg(feature = "host")]
    pub(crate) macros: [u8; crate::MACRO_SPACE_SIZE],
    /// Whether flash holds the buffer; otherwise it is seeded from the defaults.
    #[cfg(feature = "host")]
    pub(crate) macros_stored: bool,
}

impl<const ROW: usize, const COL: usize, const NUM_LAYER: usize> KeymapData<ROW, COL, NUM_LAYER, 0> {
    /// Create keymap data for a keyboard without encoders.
    pub const fn new(keymap: [[[KeyAction; COL]; ROW]; NUM_LAYER]) -> Self {
        Self {
            keymap,
            encoder_map: [const { [] }; NUM_LAYER],
            layer_state: [false; NUM_LAYER],
            layer_cache: [[0; COL]; ROW],
            encoder_layer_cache: [],
            layout_option: 0,
            #[cfg(feature = "host")]
            macros: [0; crate::MACRO_SPACE_SIZE],
            #[cfg(feature = "host")]
            macros_stored: false,
        }
    }
}

impl<const ROW: usize, const COL: usize, const NUM_LAYER: usize, const NUM_ENCODER: usize>
    KeymapData<ROW, COL, NUM_LAYER, NUM_ENCODER>
{
    /// Create keymap data for a keyboard with encoders.
    pub const fn new_with_encoder(
        keymap: [[[KeyAction; COL]; ROW]; NUM_LAYER],
        encoder_map: [[EncoderAction; NUM_ENCODER]; NUM_LAYER],
    ) -> Self {
        Self {
            keymap,
            encoder_map,
            layer_state: [false; NUM_LAYER],
            layer_cache: [[0; COL]; ROW],
            encoder_layer_cache: [[0u8; 2]; NUM_ENCODER],
            layout_option: 0,
            #[cfg(feature = "host")]
            macros: [0; crate::MACRO_SPACE_SIZE],
            #[cfg(feature = "host")]
            macros_stored: false,
        }
    }
}

/// fills up the vector to its capacity
pub(crate) fn fill_vec<T: Default + Clone, const N: usize>(vector: &mut heapless::Vec<T, N>) {
    vector
        .resize(vector.capacity(), T::default())
        .expect("impossible error, as we resize to the capacity of the vector!");
}

/// KeyMap with hidden interior mutability.
///
/// Consumers use `&KeyMap` with plain method calls — no generics needed.
/// All const generic parameters are erased at construction time.
pub struct KeyMap<'a> {
    inner: RefCell<KeyMapInner<'a>>,
}

struct KeyMapInner<'a> {
    row: usize,
    col: usize,
    num_layer: usize,
    num_encoder: usize,
    /// Flat layer data: num_layer * row * col
    layers: &'a mut [KeyAction],
    /// Flat encoder data: num_layer * num_encoder (None if no encoders)
    encoders: Option<&'a mut [EncoderAction]>,
    /// Per-layer activation state
    layer_state: &'a mut [bool],
    /// (activated, deactivated): whether any layer became active/inactive since the last take_layer_changes call.
    /// Used in sticky key's "layer_activate"/"layer_deactivate" release condition.
    layer_changes: (bool, bool),
    /// Layer cache for keys: row * col
    layer_cache: &'a mut [u8],
    /// Layer cache for encoders: num_encoder * 2
    encoder_layer_cache: &'a mut [u8],
    /// Behavior configuration
    behavior: &'a mut BehaviorConfig,
    /// Hand info: row * col (read-only)
    hand: &'a [Hand],
    /// Mouse button state
    mouse_buttons: u8,
    /// VIA/Vial layout options; persisted via `LayoutOption`
    layout_option: u32,
    macros: Macros<'a>,
    /// Matrix state for vial lock
    #[cfg(feature = "host_lock")]
    matrix_state: MatrixState,
}

impl KeyMapInner<'_> {
    #[inline]
    fn layer_index(&self, layer: usize, row: usize, col: usize) -> usize {
        layer * self.row * self.col + row * self.col + col
    }

    #[inline]
    fn encoder_index(&self, layer: usize, id: usize) -> usize {
        layer * self.num_encoder + id
    }

    #[inline]
    fn cache_index(&self, row: usize, col: usize) -> usize {
        row * self.col + col
    }

    #[inline]
    fn encoder_cache_index(&self, id: usize, direction: usize) -> usize {
        id * 2 + direction
    }
}

impl KeyMapInner<'_> {
    fn get_keymap_config(&self) -> (usize, usize, usize) {
        (self.row, self.col, self.num_layer)
    }

    fn get_default_layer(&self) -> u8 {
        self.behavior.default_layer
    }

    fn set_default_layer(&mut self, layer_num: u8) {
        if layer_num as usize >= self.num_layer {
            warn!(
                "Not a valid default layer {}, keyboard supports only {} layers",
                layer_num, self.num_layer
            );
            return;
        }
        let old_default = self.behavior.default_layer;
        if old_default == layer_num {
            return;
        }
        let activated = !self.layer_state[layer_num as usize];
        let deactivated = self
            .layer_state
            .get(old_default as usize)
            .is_some_and(|&active| !active);
        self.behavior.default_layer = layer_num;
        self.record_layer_changes(activated, deactivated);
    }

    fn get_action_at(&self, pos: KeyboardEventPos, layer_num: usize) -> KeyAction {
        match pos {
            KeyboardEventPos::Key(key_pos) => {
                let row = key_pos.row as usize;
                let col = key_pos.col as usize;
                if row >= self.row || col >= self.col || layer_num >= self.num_layer {
                    // Positions may come from remote split peers; never panic on them.
                    warn!("Key position ({}, {}) out of range on layer {}", row, col, layer_num);
                    return KeyAction::No;
                }
                self.layers[self.layer_index(layer_num, row, col)]
            }
            KeyboardEventPos::RotaryEncoder(encoder_pos) => {
                if let Some(encoders) = &self.encoders
                    && encoder_pos.direction != Direction::None
                {
                    let idx = self.encoder_index(layer_num, encoder_pos.id as usize);
                    if let Some(encoder_action) = encoders.get(idx) {
                        return match encoder_pos.direction {
                            Direction::Clockwise => encoder_action.clockwise,
                            Direction::CounterClockwise => encoder_action.counter_clockwise,
                            Direction::None => KeyAction::No,
                        };
                    }
                }
                KeyAction::No
            }
            KeyboardEventPos::Combo(_)
            | KeyboardEventPos::Macro
            | KeyboardEventPos::Virtual(_)
            | KeyboardEventPos::Sticky(_) => KeyAction::No,
        }
    }

    fn set_action_at(&mut self, pos: KeyboardEventPos, layer_num: usize, action: KeyAction) {
        match pos {
            KeyboardEventPos::Key(key_pos) => {
                let row = key_pos.row as usize;
                let col = key_pos.col as usize;
                if row >= self.row || col >= self.col || layer_num >= self.num_layer {
                    warn!("Key position ({}, {}) out of range on layer {}", row, col, layer_num);
                    return;
                }
                let idx = self.layer_index(layer_num, row, col);
                self.layers[idx] = action;
            }
            KeyboardEventPos::RotaryEncoder(encoder_pos) => {
                let idx = self.encoder_index(layer_num, encoder_pos.id as usize);
                if let Some(encoders) = &mut self.encoders
                    && let Some(encoder_action) = encoders.get_mut(idx)
                {
                    match encoder_pos.direction {
                        Direction::Clockwise => encoder_action.clockwise = action,
                        Direction::CounterClockwise => encoder_action.counter_clockwise = action,
                        Direction::None => {}
                    }
                }
            }
            KeyboardEventPos::Combo(_)
            | KeyboardEventPos::Macro
            | KeyboardEventPos::Virtual(_)
            | KeyboardEventPos::Sticky(_) => {}
        }
    }

    fn get_action_with_layer_cache(&mut self, event: KeyboardEvent) -> KeyAction {
        if !event.pressed {
            let layer = self.pop_layer_from_cache(event.pos);
            return self.get_action_at(event.pos, layer as usize);
        }

        for layer_idx in (0..self.num_layer).rev() {
            if self.layer_state[layer_idx] || layer_idx as u8 == self.behavior.default_layer {
                let action = self.get_action_at(event.pos, layer_idx);
                if action == KeyAction::Transparent {
                    continue;
                }
                self.save_layer_cache(event.pos, layer_idx as u8);
                return action;
            }
            if layer_idx as u8 == self.behavior.default_layer {
                break;
            }
        }

        // Keep release on the same transparent default-layer action as press.
        self.save_layer_cache(event.pos, self.behavior.default_layer);
        KeyAction::No
    }

    fn get_activated_layer(&self) -> u8 {
        for layer_idx in (0..self.num_layer).rev() {
            if self.layer_state[layer_idx] || layer_idx as u8 == self.behavior.default_layer {
                return layer_idx as u8;
            }
        }
        self.behavior.default_layer
    }

    fn pop_layer_from_cache(&mut self, pos: KeyboardEventPos) -> u8 {
        match pos {
            KeyboardEventPos::Key(key_pos) => {
                let ci = self.cache_index(key_pos.row as usize, key_pos.col as usize);
                if let Some(cache) = self.layer_cache.get_mut(ci) {
                    let layer = *cache;
                    *cache = self.behavior.default_layer;
                    return layer;
                }
                self.behavior.default_layer
            }
            KeyboardEventPos::RotaryEncoder(encoder_pos) => {
                if encoder_pos.direction != Direction::None {
                    let ci = self.encoder_cache_index(encoder_pos.id as usize, encoder_pos.direction as usize);
                    if let Some(cache) = self.encoder_layer_cache.get_mut(ci) {
                        let layer = *cache;
                        *cache = self.behavior.default_layer;
                        return layer;
                    }
                }
                self.behavior.default_layer
            }
            KeyboardEventPos::Combo(_)
            | KeyboardEventPos::Macro
            | KeyboardEventPos::Virtual(_)
            | KeyboardEventPos::Sticky(_) => self.behavior.default_layer,
        }
    }

    fn save_layer_cache(&mut self, pos: KeyboardEventPos, layer_num: u8) {
        match pos {
            KeyboardEventPos::Key(key_pos) => {
                let ci = self.cache_index(key_pos.row as usize, key_pos.col as usize);
                if let Some(cache) = self.layer_cache.get_mut(ci) {
                    *cache = layer_num;
                }
            }
            KeyboardEventPos::RotaryEncoder(encoder_pos) => {
                if encoder_pos.direction != Direction::None {
                    let ci = self.encoder_cache_index(encoder_pos.id as usize, encoder_pos.direction as usize);
                    if let Some(cache) = self.encoder_layer_cache.get_mut(ci) {
                        *cache = layer_num;
                    }
                }
            }
            KeyboardEventPos::Combo(_)
            | KeyboardEventPos::Macro
            | KeyboardEventPos::Virtual(_)
            | KeyboardEventPos::Sticky(_) => {}
        }
    }

    fn update_fn_layer_state(&mut self) {
        if self.num_layer > 3 {
            let before = self.is_layer_effective(3);
            self.layer_state[3] = self.layer_state[1] && self.layer_state[2];
            let after = self.is_layer_effective(3);
            self.record_layer_changes(!before && after, before && !after);
        }
    }

    fn is_layer_effective(&self, layer: usize) -> bool {
        self.layer_state[layer] || layer == self.behavior.default_layer as usize
    }

    fn set_layer_state(&mut self, layer_num: u8, enabled: bool) {
        if layer_num as usize >= self.num_layer {
            warn!(
                "Not a valid layer {}, keyboard supports only {} layers",
                layer_num, self.num_layer
            );
            return;
        }
        let layer = layer_num as usize;
        let before = self.is_layer_effective(layer);
        let adjust_before = self
            .behavior
            .tri_layer
            .map(|[_, _, adjust]| (adjust as usize, self.is_layer_effective(adjust as usize)));

        self.layer_state[layer] = enabled;
        if let Some([upper, lower, adjust]) = self.behavior.tri_layer {
            self.layer_state[adjust as usize] = self.layer_state[upper as usize] && self.layer_state[lower as usize];
        }

        // Tri-layer can overwrite the target, so compare only the final states.
        let after = self.is_layer_effective(layer);
        let mut activated = !before && after;
        let mut deactivated = before && !after;
        if let Some((adjust, before)) = adjust_before {
            let after = self.is_layer_effective(adjust);
            activated |= !before && after;
            deactivated |= before && !after;
        }
        self.record_layer_changes(activated, deactivated);
    }

    // Record if there's a layer activated or deactivated and publish the event.
    fn record_layer_changes(&mut self, activated: bool, deactivated: bool) {
        if activated || deactivated {
            self.layer_changes.0 |= activated;
            self.layer_changes.1 |= deactivated;
            publish_event(LayerChangeEvent::new(self.get_activated_layer()));
        }
    }
}

// Keep `inner` borrows inside sync methods so no borrow crosses an await.

impl<'a> KeyMap<'a> {
    /// Flatten [`KeymapData`] and build the `KeyMap`.
    ///
    /// This is the shared construction logic used by both `new` and `new_from_storage`.
    /// Uses `as_flattened_mut()` / `as_flattened()` (Rust 1.85+, no unsafe).
    fn build<const ROW: usize, const COL: usize, const NUM_LAYER: usize, const NUM_ENCODER: usize>(
        data: &'a mut KeymapData<ROW, COL, NUM_LAYER, NUM_ENCODER>,
        behavior: &'a mut BehaviorConfig,
        positional_config: &'a PositionalConfig<ROW, COL>,
    ) -> Self {
        let layers = data.keymap.as_mut_slice().as_flattened_mut().as_flattened_mut();
        let encoders = if NUM_ENCODER > 0 {
            Some(data.encoder_map.as_mut_slice().as_flattened_mut())
        } else {
            None
        };
        let layer_state = &mut data.layer_state;
        let layer_cache = data.layer_cache.as_mut_slice().as_flattened_mut();
        let encoder_layer_cache = data.encoder_layer_cache.as_mut_slice().as_flattened_mut();
        let hand = positional_config.hand.as_slice().as_flattened();
        #[cfg(feature = "host")]
        let macros = Macros::new(behavior.keyboard_macros, &mut data.macros, data.macros_stored);
        #[cfg(not(feature = "host"))]
        let macros = Macros::new(behavior.keyboard_macros, &mut [], false);

        KeyMap {
            inner: RefCell::new(KeyMapInner {
                row: ROW,
                col: COL,
                num_layer: NUM_LAYER,
                num_encoder: NUM_ENCODER,
                layers,
                encoders,
                layer_state,
                layer_changes: (false, false),
                layer_cache,
                encoder_layer_cache,
                behavior,
                hand,
                mouse_buttons: 0,
                layout_option: data.layout_option,
                macros,
                #[cfg(feature = "host_lock")]
                matrix_state: MatrixState::new(ROW, COL),
            }),
        }
    }

    /// Generic constructor — const generics stop here.
    pub async fn new<const ROW: usize, const COL: usize, const NUM_LAYER: usize, const NUM_ENCODER: usize>(
        data: &'a mut KeymapData<ROW, COL, NUM_LAYER, NUM_ENCODER>,
        behavior: &'a mut BehaviorConfig,
        positional_config: &'a PositionalConfig<ROW, COL>,
    ) -> Self {
        fill_vec(&mut behavior.fork.forks);
        fill_vec(&mut behavior.morse.morses);
        Self::build(data, behavior, positional_config)
    }

    #[cfg(all(feature = "storage", feature = "host"))]
    pub async fn new_from_storage<
        F: NorFlash,
        const ROW: usize,
        const COL: usize,
        const NUM_LAYER: usize,
        const NUM_ENCODER: usize,
    >(
        data: &'a mut KeymapData<ROW, COL, NUM_LAYER, NUM_ENCODER>,
        storage: Option<&mut Storage<F, ROW, COL, NUM_LAYER, NUM_ENCODER>>,
        behavior: &'a mut BehaviorConfig,
        positional_config: &'a PositionalConfig<ROW, COL>,
    ) -> Self {
        fill_vec(&mut behavior.fork.forks);
        fill_vec(&mut behavior.morse.morses);

        // Read from storage BEFORE flattening (storage expects typed arrays).
        if let Some(storage) = storage {
            if storage.clear_layout {
                debug!("`clear_layout` is set, rewriting the items the compiled-in layout owns.");
                storage.write_layout(data, behavior).await;
            } else if storage.read_keymap(data, behavior).await.is_err() {
                error!("Failed to read from storage, clearing...");
                storage.flash.erase_all().await.ok();
                reboot_keyboard();
            }
        }

        Self::build(data, behavior, positional_config)
    }

    pub(crate) fn get_action_with_layer_cache(&self, event: KeyboardEvent) -> KeyAction {
        self.inner.borrow_mut().get_action_with_layer_cache(event)
    }

    pub(crate) fn get_action_at(&self, pos: KeyboardEventPos, layer: usize) -> KeyAction {
        self.inner.borrow().get_action_at(pos, layer)
    }

    /// Read the action currently bound to a `(layer, row, col)` position.
    ///
    /// This is the post-storage, post-Vial state — i.e. what the keyboard will
    /// actually emit when that key fires, not the compile-time default. Useful
    /// for accessory displays / status surfaces that want to mirror the live
    /// keymap.
    pub fn action_at_pos(&self, layer: usize, row: u8, col: u8) -> KeyAction {
        self.inner
            .borrow()
            .get_action_at(KeyboardEventPos::key_pos(col, row), layer)
    }

    /// Active layer index (after layer-toggle/momentary updates).
    pub fn active_layer(&self) -> u8 {
        self.inner.borrow().get_activated_layer()
    }

    pub(crate) fn set_action_at(&self, pos: KeyboardEventPos, layer: usize, action: KeyAction) {
        self.inner.borrow_mut().set_action_at(pos, layer, action);
    }

    pub(crate) fn activate_layer(&self, layer_num: u8) {
        self.inner.borrow_mut().set_layer_state(layer_num, true);
    }

    pub(crate) fn deactivate_layer(&self, layer_num: u8) {
        self.inner.borrow_mut().set_layer_state(layer_num, false);
    }

    pub(crate) fn toggle_layer(&self, layer_num: u8) {
        let mut inner = self.inner.borrow_mut();
        let Some(&active) = inner.layer_state.get(layer_num as usize) else {
            warn!(
                "Not a valid layer {}, keyboard supports only {} layers",
                layer_num, inner.num_layer
            );
            return;
        };
        inner.set_layer_state(layer_num, !active);
    }

    /// Activate `layer_num` only if it is currently inactive.
    ///
    /// Returns `true` if this call performed the activation, `false` if the
    /// layer was already active (or the index is out of range). Folds the
    /// "check then activate" sequence into a single borrow so callers can't
    /// accidentally race against other layer mutations.
    pub(crate) fn activate_layer_if_inactive(&self, layer_num: u8) -> bool {
        let mut inner = self.inner.borrow_mut();
        let idx = layer_num as usize;
        if idx >= inner.num_layer || inner.layer_state[idx] {
            return false;
        }
        inner.set_layer_state(layer_num, true);
        true
    }

    pub(crate) fn deactivate_layer_if_active(&self, layer_num: u8) {
        let mut inner = self.inner.borrow_mut();
        let idx = layer_num as usize;
        if idx >= inner.num_layer || !inner.layer_state[idx] {
            return;
        }
        inner.set_layer_state(layer_num, false);
    }

    /// Consume layer changes, used in sticky key only.
    pub(crate) fn take_layer_changes(&self) -> (bool, bool) {
        core::mem::take(&mut self.inner.borrow_mut().layer_changes)
    }

    pub(crate) fn auto_mouse_layer_configs(
        &self,
    ) -> heapless::Vec<crate::config::AutoMouseLayerConfig, { crate::AUTO_MOUSE_LAYER_MAX_NUM }> {
        self.inner.borrow().behavior.auto_mouse_layer.clone()
    }

    /// Whether `layer_num` is set in the layer mask.
    ///
    /// Unlike [`Self::active_layer`] (which returns only the topmost), this
    /// reports each layer individually.
    pub(crate) fn is_layer_active(&self, layer_num: u8) -> bool {
        let inner = self.inner.borrow();
        let idx = layer_num as usize;
        idx < inner.num_layer && inner.layer_state[idx]
    }

    pub(crate) fn num_layer(&self) -> usize {
        self.inner.borrow().num_layer
    }

    pub(crate) fn get_activated_layer(&self) -> u8 {
        self.inner.borrow().get_activated_layer()
    }

    pub(crate) fn get_default_layer(&self) -> u8 {
        self.inner.borrow().get_default_layer()
    }

    pub(crate) fn set_default_layer(&self, layer_num: u8) {
        self.inner.borrow_mut().set_default_layer(layer_num);
    }

    pub(crate) fn layout_option(&self) -> u32 {
        self.inner.borrow().layout_option
    }

    pub(crate) fn set_layout_option(&self, layout_option: u32) {
        self.inner.borrow_mut().layout_option = layout_option;
    }

    /// The behavior config as one storage item; taken after a RAM change so the
    /// next writer's snapshot includes it.
    #[cfg(feature = "storage")]
    pub(crate) fn behavior_snapshot(&self) -> crate::storage::BehaviorConfig {
        (&*self.inner.borrow().behavior).into()
    }

    pub(crate) fn update_fn_layer_state(&self) {
        self.inner.borrow_mut().update_fn_layer_state();
    }

    pub(crate) fn get_keymap_config(&self) -> (usize, usize, usize) {
        self.inner.borrow().get_keymap_config()
    }

    pub(crate) fn hand_at(&self, row: usize, col: usize) -> Hand {
        let inner = self.inner.borrow();
        let idx = inner.cache_index(row, col);
        if idx < inner.hand.len() {
            inner.hand[idx]
        } else {
            Hand::Unknown
        }
    }

    pub(crate) fn combo_timeout(&self) -> Duration {
        self.inner.borrow().behavior.combo.timeout
    }

    pub(crate) fn combo_prior_idle_time(&self) -> Option<Duration> {
        self.inner.borrow().behavior.combo.prior_idle_time
    }

    /// Waiting timeout of the default Sticky Key profile, in milliseconds.
    pub(crate) fn default_sticky_wait_timeout_ms(&self) -> u16 {
        self.inner.borrow().behavior.sticky_key.default_profile.wait_timeout_ms
    }

    pub(crate) fn tap_interval(&self) -> u16 {
        self.inner.borrow().behavior.tap.tap_interval
    }

    pub(crate) fn tap_capslock_interval(&self) -> u16 {
        self.inner.borrow().behavior.tap.tap_capslock_interval
    }

    pub(crate) fn morse_enable_flow_tap(&self) -> bool {
        self.inner.borrow().behavior.morse.enable_flow_tap
    }

    pub(crate) fn morse_prior_idle_time(&self) -> Duration {
        self.inner.borrow().behavior.morse.prior_idle_time
    }

    #[cfg(feature = "rynk")]
    pub(crate) fn sticky_profile(&self, idx: u8) -> Option<StickyProfile> {
        let inner = self.inner.borrow();
        let config = &inner.behavior.sticky_key;
        if idx == rmk_types::sticky::STICKY_PROFILE_DEFAULT {
            Some(config.default_profile.clone())
        } else {
            config.profiles.get(idx as usize).cloned()
        }
    }

    /// Return an owned profile, using the configured default for an absent index.
    pub(crate) fn sticky_profile_or_default(&self, idx: u8) -> StickyProfile {
        self.inner.borrow().behavior.sticky_key.get_profile(idx).clone()
    }

    #[cfg(feature = "rynk")]
    pub(crate) fn sticky_profiles_len(&self) -> usize {
        self.inner.borrow().behavior.sticky_key.profiles.len()
    }

    #[cfg(feature = "rynk")]
    pub(crate) fn set_sticky_profile(&self, idx: u8, profile: StickyProfile) -> bool {
        let mut inner = self.inner.borrow_mut();
        let config = &mut inner.behavior.sticky_key;
        let slot = if idx == rmk_types::sticky::STICKY_PROFILE_DEFAULT {
            Some(&mut config.default_profile)
        } else {
            config.profiles.get_mut(idx as usize)
        };
        if let Some(slot) = slot {
            *slot = profile;
            true
        } else {
            false
        }
    }

    pub(crate) fn morse_default_profile(&self) -> MorseProfile {
        self.inner.borrow().behavior.morse.default_profile
    }

    /// Resolve a per-key morse profile by its table index: the table entry if
    /// present, otherwise the user-configured default profile. Fields left
    /// `None` by the resolved profile are still filled in per-field by the
    /// callers (default profile, then hardcoded fallbacks).
    pub(crate) fn morse_profile(&self, idx: u8) -> MorseProfile {
        let inner = self.inner.borrow();
        let morse = &inner.behavior.morse;
        morse
            .profiles
            .get(idx as usize)
            .copied()
            .unwrap_or(morse.default_profile)
    }

    pub(crate) fn mouse_key_config(&self) -> MouseKeyConfig {
        self.inner.borrow().behavior.mouse_key
    }

    pub(crate) fn forks_is_empty(&self) -> bool {
        self.inner.borrow().behavior.fork.forks.is_empty()
    }

    pub(crate) fn morses_len(&self) -> usize {
        self.inner.borrow().behavior.morse.morses.len()
    }

    pub(crate) fn set_combo_timeout(&self, timeout: Duration) {
        self.inner.borrow_mut().behavior.combo.timeout = timeout;
    }

    pub(crate) fn set_default_sticky_wait_timeout_ms(&self, timeout_ms: u16) {
        self.inner
            .borrow_mut()
            .behavior
            .sticky_key
            .default_profile
            .wait_timeout_ms = timeout_ms;
    }

    pub(crate) fn set_tap_interval(&self, interval: u16) {
        self.inner.borrow_mut().behavior.tap.tap_interval = interval;
    }

    pub(crate) fn set_tap_capslock_interval(&self, interval: u16) {
        self.inner.borrow_mut().behavior.tap.tap_capslock_interval = interval;
    }

    pub(crate) fn set_morse_default_profile(&self, profile: MorseProfile) {
        self.inner.borrow_mut().behavior.morse.default_profile = profile;
    }

    pub(crate) fn set_morse_prior_idle_time(&self, time: Duration) {
        self.inner.borrow_mut().behavior.morse.prior_idle_time = time;
    }

    pub(crate) fn get_morse(&self, idx: usize) -> Option<Morse> {
        self.inner.borrow().behavior.morse.morses.get(idx).cloned()
    }

    pub(crate) fn with_morse_mut<R>(&self, idx: usize, f: impl FnOnce(&mut Morse) -> R) -> Option<R> {
        self.inner.borrow_mut().behavior.morse.morses.get_mut(idx).map(f)
    }

    pub(crate) fn with_forks<R>(&self, f: impl FnOnce(&[Fork]) -> R) -> R {
        let inner = self.inner.borrow();
        f(&inner.behavior.fork.forks)
    }

    pub(crate) fn with_forks_mut<R>(&self, f: impl FnOnce(&mut [Fork]) -> R) -> R {
        let mut inner = self.inner.borrow_mut();
        f(&mut inner.behavior.fork.forks)
    }

    pub(crate) fn with_combos<R>(&self, f: impl FnOnce(&[Option<Combo>]) -> R) -> R {
        let inner = self.inner.borrow();
        f(&inner.behavior.combo.combos)
    }

    pub(crate) fn with_combos_mut<R>(&self, f: impl FnOnce(&mut [Option<Combo>]) -> R) -> R {
        let mut inner = self.inner.borrow_mut();
        f(&mut inner.behavior.combo.combos)
    }

    pub(crate) fn macros<R>(&self, f: impl FnOnce(&mut Macros<'a>) -> R) -> R {
        f(&mut self.inner.borrow_mut().macros)
    }

    pub(crate) fn mouse_buttons(&self) -> u8 {
        self.inner.borrow().mouse_buttons
    }

    pub(crate) fn set_mouse_buttons(&self, buttons: u8) {
        self.inner.borrow_mut().mouse_buttons = buttons;
    }

    pub(crate) fn get_action_by_flat_index(&self, index: usize) -> KeyAction {
        let inner = self.inner.borrow();
        if index < inner.layers.len() {
            inner.layers[index]
        } else {
            KeyAction::No
        }
    }

    pub(crate) fn num_encoders(&self) -> usize {
        self.inner.borrow().num_encoder
    }

    pub(crate) fn get_encoder_action(&self, layer: usize, id: usize) -> Option<EncoderAction> {
        let inner = self.inner.borrow();
        inner.encoders.as_ref().and_then(|encoders| {
            let idx = inner.encoder_index(layer, id);
            encoders.get(idx).copied()
        })
    }

    pub(crate) fn set_encoder_clockwise(&self, layer: usize, id: usize, action: KeyAction) -> Option<EncoderAction> {
        let mut inner = self.inner.borrow_mut();
        let idx = inner.encoder_index(layer, id);
        if let Some(encoders) = &mut inner.encoders
            && let Some(encoder_action) = encoders.get_mut(idx)
        {
            encoder_action.clockwise = action;
            return Some(*encoder_action);
        }
        None
    }

    pub(crate) fn set_encoder_counter_clockwise(
        &self,
        layer: usize,
        id: usize,
        action: KeyAction,
    ) -> Option<EncoderAction> {
        let mut inner = self.inner.borrow_mut();
        let idx = inner.encoder_index(layer, id);
        if let Some(encoders) = &mut inner.encoders
            && let Some(encoder_action) = encoders.get_mut(idx)
        {
            encoder_action.counter_clockwise = action;
            return Some(*encoder_action);
        }
        None
    }

    /// Write both directions of an encoder under one borrow. Returns `false`
    /// if the slot is out of range.
    pub(crate) fn set_encoder(&self, layer: usize, id: usize, action: EncoderAction) -> bool {
        let mut inner = self.inner.borrow_mut();
        let idx = inner.encoder_index(layer, id);
        if let Some(encoders) = &mut inner.encoders
            && let Some(slot) = encoders.get_mut(idx)
        {
            *slot = action;
            return true;
        }
        false
    }

    #[cfg(feature = "host_lock")]
    pub(crate) fn update_matrix_state(&self, event: &KeyboardEvent) {
        self.inner.borrow_mut().matrix_state.update(event);
    }

    #[cfg(feature = "host_lock")]
    pub(crate) fn read_matrix_state(&self, target: &mut [u8]) {
        self.inner.borrow().matrix_state.read_all(target);
    }

    #[cfg(feature = "host_lock")]
    pub(crate) fn read_matrix_key(&self, row: u8, col: u8) -> bool {
        self.inner.borrow().matrix_state.read(row, col)
    }
}

#[cfg(test)]
mod test {
    use rmk_types::fork::{Fork, StateBits};
    use rmk_types::modifier::ModifierCombination;

    use crate::keyboard::combo::{Combo, ComboConfig};
    use crate::keymap::fill_vec;
    use crate::{COMBO_MAX_NUM, FORK_MAX_NUM, k};

    #[test]
    fn test_fill_vec() {
        let mut combos: heapless::Vec<_, COMBO_MAX_NUM> = heapless::Vec::from_slice(&[
            Combo::new(ComboConfig::new([k!(A), k!(B), k!(C), k!(D)], k!(Z), None)),
            Combo::new(ComboConfig::new([k!(A), k!(B)], k!(X), None)),
            Combo::new(ComboConfig::new([k!(A), k!(B), k!(C)], k!(Y), None)),
        ])
        .unwrap();

        fill_vec(&mut combos);

        assert_eq!(combos.len(), COMBO_MAX_NUM);

        let mut forks: heapless::Vec<_, FORK_MAX_NUM> = heapless::Vec::from_slice(&[
            Fork::new(
                k!(A),
                k!(Y),
                k!(F),
                StateBits::default(),
                StateBits::default(),
                ModifierCombination::new(),
                false,
            ),
            Fork::new(
                k!(B),
                k!(B),
                k!(F),
                StateBits::default(),
                StateBits::default(),
                ModifierCombination::new(),
                false,
            ),
            Fork::new(
                k!(C),
                k!(Y),
                k!(Y),
                StateBits::default(),
                StateBits::default(),
                ModifierCombination::new(),
                false,
            ),
        ])
        .unwrap();

        fill_vec(&mut forks);

        assert_eq!(forks.len(), FORK_MAX_NUM);
    }

    #[test]
    fn sticky_profile_values_do_not_hold_keymap_borrows() {
        use rmk_types::sticky::STICKY_PROFILE_DEFAULT;

        use crate::config::{BehaviorConfig, PositionalConfig};
        use crate::keymap::{KeyMap, KeymapData};

        let mut data = KeymapData::<1, 1, 2>::new([[[k!(A)]]; 2]);
        let mut behavior = BehaviorConfig::default();
        behavior.sticky_key.default_profile.wait_timeout_ms = 1234;
        let positional = PositionalConfig::<1, 1>::default();
        let keymap = KeyMap::build(&mut data, &mut behavior, &positional);

        let default = keymap.sticky_profile_or_default(STICKY_PROFILE_DEFAULT);
        let absent = keymap.sticky_profile_or_default(254);
        #[cfg(feature = "rynk")]
        let host_profile = keymap.sticky_profile(STICKY_PROFILE_DEFAULT).unwrap();

        keymap.set_default_sticky_wait_timeout_ms(4321);
        keymap.activate_layer(1);

        assert_eq!(default.wait_timeout_ms, 1234);
        assert_eq!(absent, default);
        #[cfg(feature = "rynk")]
        assert_eq!(host_profile, default);
        assert_eq!(
            keymap.sticky_profile_or_default(STICKY_PROFILE_DEFAULT).wait_timeout_ms,
            4321
        );
        assert!(keymap.is_layer_active(1));
    }

    #[test]
    fn is_layer_active_reports_individual_layer_state() {
        use crate::config::{BehaviorConfig, PositionalConfig};
        use crate::keymap::{KeyMap, KeymapData};

        let mut data = KeymapData::<1, 1, 4>::new([[[k!(A)]], [[k!(B)]], [[k!(C)]], [[k!(D)]]]);
        let mut behavior = BehaviorConfig::default();
        let positional = PositionalConfig::<1, 1>::default();
        let keymap = KeyMap::build(&mut data, &mut behavior, &positional);

        // Layer 0 is the default but not explicitly set in the mask.
        assert!(!keymap.is_layer_active(0));
        assert!(!keymap.is_layer_active(3));
        // Out-of-range returns false (no panic).
        assert!(!keymap.is_layer_active(99));

        assert!(keymap.activate_layer_if_inactive(2));
        assert!(keymap.is_layer_active(2));
        assert!(!keymap.is_layer_active(1));
        assert!(!keymap.is_layer_active(3));
        assert!(!keymap.activate_layer_if_inactive(2));

        keymap.deactivate_layer_if_active(2);
        assert!(!keymap.is_layer_active(2));
        keymap.deactivate_layer_if_active(2);
        assert!(!keymap.is_layer_active(2));

        // Mirrors the auto-mouse Either3::Third guard.
        assert!(keymap.activate_layer_if_inactive(2));
        let self_activated = true;
        assert!(!(self_activated && !keymap.is_layer_active(2)));
        keymap.deactivate_layer_if_active(2);
        assert!(self_activated && !keymap.is_layer_active(2));
    }

    #[test]
    fn layer_changes_keep_hidden_default_and_transient_transitions() {
        use rmk_types::action::KeyAction;

        use crate::config::{BehaviorConfig, PositionalConfig};
        use crate::keymap::{KeyMap, KeymapData};
        let mut data = KeymapData::<1, 1, 40>::new([[[KeyAction::No]]; 40]);
        let mut behavior = BehaviorConfig::default();
        let positional = PositionalConfig::<1, 1>::default();
        let keymap = KeyMap::build(&mut data, &mut behavior, &positional);
        keymap.activate_layer(39);
        assert_eq!(keymap.take_layer_changes(), (true, false));
        keymap.set_default_layer(1);
        assert_eq!(keymap.active_layer(), 39);
        assert_eq!(keymap.take_layer_changes(), (true, true));
        keymap.activate_layer(33);
        keymap.deactivate_layer(33);
        assert_eq!(keymap.take_layer_changes(), (true, true));
        assert_eq!(keymap.take_layer_changes(), (false, false));
        keymap.activate_layer(39);
        keymap.set_default_layer(1);
        assert_eq!(keymap.take_layer_changes(), (false, false));
    }

    #[test]
    fn tri_layer_changes_use_final_effective_state() {
        use crate::config::{BehaviorConfig, PositionalConfig};
        use crate::keymap::{KeyMap, KeymapData};

        let mut data = KeymapData::<1, 1, 4>::new([[[k!(A)]]; 4]);
        let mut behavior = BehaviorConfig {
            default_layer: 1,
            tri_layer: Some([1, 2, 3]),
            ..Default::default()
        };
        let positional = PositionalConfig::<1, 1>::default();
        let keymap = KeyMap::build(&mut data, &mut behavior, &positional);

        // Tri-layer immediately cancels direct activation of its output.
        keymap.activate_layer(3);
        keymap.toggle_layer(3);
        assert!(!keymap.is_layer_active(3));
        assert_eq!(keymap.take_layer_changes(), (false, false));

        keymap.activate_layer(2);
        assert_eq!(keymap.take_layer_changes(), (true, false));
        // The target is already effective as the default; only layer 3 changes.
        keymap.activate_layer(1);
        assert!(keymap.is_layer_active(3));
        assert_eq!(keymap.take_layer_changes(), (true, false));
        keymap.deactivate_layer(1);
        assert!(!keymap.is_layer_active(3));
        assert_eq!(keymap.take_layer_changes(), (false, true));
    }

    #[test]
    fn default_layer_changes_preserve_explicitly_active_layers() {
        use crate::config::{BehaviorConfig, PositionalConfig};
        use crate::keymap::{KeyMap, KeymapData};

        let mut data = KeymapData::<1, 1, 3>::new([[[k!(A)]]; 3]);
        let mut behavior = BehaviorConfig::default();
        let positional = PositionalConfig::<1, 1>::default();
        let keymap = KeyMap::build(&mut data, &mut behavior, &positional);

        keymap.activate_layer(0);
        assert_eq!(keymap.take_layer_changes(), (false, false));
        keymap.set_default_layer(1);
        assert_eq!(keymap.take_layer_changes(), (true, false));
        keymap.set_default_layer(0);
        assert_eq!(keymap.take_layer_changes(), (false, true));

        keymap.activate_layer(1);
        assert_eq!(keymap.take_layer_changes(), (true, false));
        keymap.set_default_layer(1);
        assert_eq!(keymap.take_layer_changes(), (false, false));
        keymap.set_default_layer(99);
        assert_eq!(keymap.get_default_layer(), 1);
        assert_eq!(keymap.take_layer_changes(), (false, false));
    }

    #[test]
    fn fn_layer_changes_include_a_target_also_used_as_an_input() {
        use crate::config::{BehaviorConfig, PositionalConfig};
        use crate::keymap::{KeyMap, KeymapData};

        let mut data = KeymapData::<1, 1, 4>::new([[[k!(A)]]; 4]);
        let mut behavior = BehaviorConfig {
            tri_layer: Some([2, 3, 3]),
            ..Default::default()
        };
        let positional = PositionalConfig::<1, 1>::default();
        let keymap = KeyMap::build(&mut data, &mut behavior, &positional);

        keymap.activate_layer(1);
        keymap.activate_layer(2);
        assert_eq!(keymap.take_layer_changes(), (true, false));
        keymap.update_fn_layer_state();
        assert_eq!(keymap.take_layer_changes(), (true, false));
        keymap.update_fn_layer_state();
        assert_eq!(keymap.take_layer_changes(), (false, false));
        keymap.toggle_layer(3);
        assert!(!keymap.is_layer_active(3));
        assert_eq!(keymap.take_layer_changes(), (false, true));
    }

    #[test]
    fn out_of_range_positions_do_not_panic_or_wrap() {
        use rmk_types::action::KeyAction;

        use crate::config::{BehaviorConfig, PositionalConfig};
        use crate::event::KeyboardEventPos;
        use crate::keymap::{KeyMap, KeymapData};

        // One key per layer: layer 0 holds A, layer 1 holds B.
        let mut data = KeymapData::<1, 1, 2>::new([[[k!(A)]], [[k!(B)]]]);
        let mut behavior = BehaviorConfig::default();
        let positional = PositionalConfig::<1, 1>::default();
        let keymap = KeyMap::build(&mut data, &mut behavior, &positional);

        // Positive control: a valid position resolves normally.
        assert_eq!(keymap.get_action_at(KeyboardEventPos::key_pos(0, 0), 0), k!(A));

        // Out-of-range col and layer read as No instead of panicking.
        assert_eq!(keymap.get_action_at(KeyboardEventPos::key_pos(1, 0), 0), KeyAction::No);
        assert_eq!(keymap.get_action_at(KeyboardEventPos::key_pos(0, 0), 2), KeyAction::No);

        // Out-of-range row: the flat index would wrap into layer 1's [0][0],
        // which holds B. The bounds check must return No, not the wrapped action.
        assert_eq!(keymap.get_action_at(KeyboardEventPos::key_pos(0, 1), 0), KeyAction::No);

        // Out-of-range writes are dropped without panicking or corrupting
        // the wrapped slot.
        keymap.set_action_at(KeyboardEventPos::key_pos(0, 1), 0, k!(C));
        assert_eq!(keymap.get_action_at(KeyboardEventPos::key_pos(0, 0), 1), k!(B));
    }

    #[test]
    fn out_of_range_events_do_not_panic_in_layer_cache() {
        use rmk_types::action::KeyAction;

        use crate::config::{BehaviorConfig, PositionalConfig};
        use crate::event::KeyboardEvent;
        use crate::keymap::{KeyMap, KeymapData};

        // One key per layer: layer 0 holds A, layer 1 holds B.
        let mut data = KeymapData::<1, 1, 2>::new([[[k!(A)]], [[k!(B)]]]);
        let mut behavior = BehaviorConfig::default();
        let positional = PositionalConfig::<1, 1>::default();
        let keymap = KeyMap::build(&mut data, &mut behavior, &positional);

        // Press and release at an out-of-range col: the flat cache index would
        // be out of bounds. Both directions must resolve to No without panicking.
        assert_eq!(
            keymap.get_action_with_layer_cache(KeyboardEvent::key(0, 1, true)),
            KeyAction::No
        );
        assert_eq!(
            keymap.get_action_with_layer_cache(KeyboardEvent::key(0, 1, false)),
            KeyAction::No
        );

        // The cache entry for the valid position is untouched by the
        // out-of-range events.
        assert_eq!(
            keymap.get_action_with_layer_cache(KeyboardEvent::key(0, 0, true)),
            k!(A)
        );
        assert_eq!(
            keymap.get_action_with_layer_cache(KeyboardEvent::key(0, 0, false)),
            k!(A)
        );
    }
}
