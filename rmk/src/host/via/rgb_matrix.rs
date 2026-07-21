//! VIA Protocol 11 RGB Matrix compatibility over RMK's designated background.
//!
//! This is only a protocol adapter. It does not own a renderer or lighting
//! state: live controls are translated into atomic commands sent to the
//! authoritative lighting engine.

pub const CHANNEL: u8 = 3;

const COMMAND_SET: u8 = 0x07;
const COMMAND_GET: u8 = 0x08;
const COMMAND_SAVE: u8 = 0x09;
const COMMAND_UNHANDLED: u8 = 0xFF;

const VALUE_BRIGHTNESS: u8 = 1;
const VALUE_EFFECT: u8 = 2;
const VALUE_SPEED: u8 = 3;
const VALUE_COLOR: u8 = 4;

const EFFECT_OFF: u8 = 0;
const EFFECT_SOLID: u8 = 1;
const EFFECT_BREATHE: u8 = 5;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BackgroundMode {
    Solid,
    Breathe,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct BackgroundState {
    pub enabled: bool,
    pub hue: u8,
    pub saturation: u8,
    pub value: u8,
    pub speed: u8,
    pub mode: BackgroundMode,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct BackgroundPatch {
    pub enabled: Option<bool>,
    pub hue: Option<u8>,
    pub saturation: Option<u8>,
    pub value: Option<u8>,
    pub speed: Option<u8>,
    pub mode: Option<BackgroundMode>,
}

/// Authoritative control port used by the pure VIA packet adapter.
///
/// `patch_background` must apply the patch atomically. A save may report
/// success only after a durable backend confirms the correlated operation.
#[allow(async_fn_in_trait)]
pub trait ViaRgbMatrixControl {
    type Error;

    async fn read_background(&self) -> Result<BackgroundState, Self::Error>;
    async fn patch_background(&self, patch: BackgroundPatch) -> Result<BackgroundState, Self::Error>;
    async fn save_background(&self) -> Result<(), Self::Error>;
}

/// Default for a Vial service that was not explicitly bound to lighting.
#[derive(Copy, Clone, Debug, Default)]
pub struct Unsupported;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct UnsupportedError;

impl ViaRgbMatrixControl for Unsupported {
    type Error = UnsupportedError;

    async fn read_background(&self) -> Result<BackgroundState, Self::Error> {
        Err(UnsupportedError)
    }

    async fn patch_background(&self, _patch: BackgroundPatch) -> Result<BackgroundState, Self::Error> {
        Err(UnsupportedError)
    }

    async fn save_background(&self) -> Result<(), Self::Error> {
        Err(UnsupportedError)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Value {
    Brightness,
    Effect,
    Speed,
    Color,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum Operation {
    Get(Value),
    Patch(BackgroundPatch),
    Save,
}

fn value(value: u8) -> Result<Value, ()> {
    match value {
        VALUE_BRIGHTNESS => Ok(Value::Brightness),
        VALUE_EFFECT => Ok(Value::Effect),
        VALUE_SPEED => Ok(Value::Speed),
        VALUE_COLOR => Ok(Value::Color),
        _ => Err(()),
    }
}

fn parse(request: &[u8]) -> Result<Operation, ()> {
    let command = *request.first().ok_or(())?;
    if request.get(1).copied() != Some(CHANNEL) {
        return Err(());
    }

    match command {
        COMMAND_GET => Ok(Operation::Get(value(*request.get(2).ok_or(())?)?)),
        COMMAND_SAVE => Ok(Operation::Save),
        COMMAND_SET => {
            let selected = value(*request.get(2).ok_or(())?)?;
            let first = *request.get(3).ok_or(())?;
            let patch = match selected {
                Value::Brightness => BackgroundPatch {
                    value: Some(first),
                    ..BackgroundPatch::default()
                },
                Value::Effect => match first {
                    EFFECT_OFF => BackgroundPatch {
                        enabled: Some(false),
                        ..BackgroundPatch::default()
                    },
                    EFFECT_SOLID => BackgroundPatch {
                        enabled: Some(true),
                        mode: Some(BackgroundMode::Solid),
                        ..BackgroundPatch::default()
                    },
                    EFFECT_BREATHE => BackgroundPatch {
                        enabled: Some(true),
                        mode: Some(BackgroundMode::Breathe),
                        ..BackgroundPatch::default()
                    },
                    _ => return Err(()),
                },
                Value::Speed => BackgroundPatch {
                    speed: Some(first),
                    ..BackgroundPatch::default()
                },
                Value::Color => BackgroundPatch {
                    hue: Some(first),
                    saturation: Some(*request.get(4).ok_or(())?),
                    ..BackgroundPatch::default()
                },
            };
            Ok(Operation::Patch(patch))
        }
        _ => Err(()),
    }
}

fn write_value(response: &mut [u8], value: Value, state: BackgroundState) -> Result<(), ()> {
    match value {
        Value::Brightness => *response.get_mut(3).ok_or(())? = state.value,
        Value::Effect => {
            *response.get_mut(3).ok_or(())? = if !state.enabled {
                EFFECT_OFF
            } else {
                match state.mode {
                    BackgroundMode::Solid => EFFECT_SOLID,
                    BackgroundMode::Breathe => EFFECT_BREATHE,
                }
            };
        }
        Value::Speed => *response.get_mut(3).ok_or(())? = state.speed,
        Value::Color => {
            let data = response.get_mut(3..5).ok_or(())?;
            data[0] = state.hue;
            data[1] = state.saturation;
        }
    }
    Ok(())
}

async fn handle<C: ViaRgbMatrixControl>(control: &C, request: &[u8], response: &mut [u8]) -> Result<(), ()> {
    match parse(request)? {
        Operation::Get(value) => {
            let state = control.read_background().await.map_err(|_| ())?;
            write_value(response, value, state)
        }
        Operation::Patch(patch) => {
            control.patch_background(patch).await.map_err(|_| ())?;
            Ok(())
        }
        Operation::Save => control.save_background().await.map_err(|_| ()),
    }
}

/// Process one custom-value packet. Unsupported operations and failed control
/// requests return VIA's unhandled marker without an adapter-side fallback.
pub async fn process_packet<C: ViaRgbMatrixControl>(control: &C, request: &[u8], response: &mut [u8]) {
    if handle(control, request, response).await.is_err()
        && let Some(command) = response.first_mut()
    {
        *command = COMMAND_UNHANDLED;
    }
}

#[cfg(feature = "lighting")]
mod standard_control {
    use super::{
        BackgroundMode as ViaMode, BackgroundPatch as ViaPatch, BackgroundState as ViaState, ViaRgbMatrixControl,
    };
    use crate::lighting::processor::LightingMailbox;
    use crate::lighting::standard::{
        BackgroundMode, BackgroundPatch, StandardCommand, StandardError, StandardReply, StandardState,
    };

    /// Mailbox-backed live control. Persistence remains unavailable until RMK
    /// has a correlated durable completion path.
    pub struct StandardControl<'a, const OVERLAY_CAP: usize, const MAILBOX_CAP: usize> {
        mailbox: &'a LightingMailbox<StandardCommand<OVERLAY_CAP>, StandardReply, StandardError, MAILBOX_CAP>,
    }

    impl<'a, const OVERLAY_CAP: usize, const MAILBOX_CAP: usize> StandardControl<'a, OVERLAY_CAP, MAILBOX_CAP> {
        pub const fn new(
            mailbox: &'a LightingMailbox<StandardCommand<OVERLAY_CAP>, StandardReply, StandardError, MAILBOX_CAP>,
        ) -> Self {
            Self { mailbox }
        }
    }

    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub enum Error {
        Lighting(StandardError),
        PersistenceUnavailable,
    }

    fn state(state: StandardState) -> ViaState {
        ViaState {
            enabled: state.background.enabled,
            hue: state.background.hue,
            saturation: state.background.saturation,
            value: state.background.value,
            speed: state.background.speed,
            mode: match state.background.mode {
                BackgroundMode::Solid => ViaMode::Solid,
                BackgroundMode::Breathe => ViaMode::Breathe,
            },
        }
    }

    fn reply_state(reply: StandardReply) -> Result<StandardState, Error> {
        reply.state().ok_or(Error::PersistenceUnavailable)
    }

    fn patch(patch: ViaPatch) -> BackgroundPatch {
        BackgroundPatch {
            enabled: patch.enabled,
            hue: patch.hue,
            saturation: patch.saturation,
            value: patch.value,
            speed: patch.speed,
            mode: patch.mode.map(|mode| match mode {
                ViaMode::Solid => BackgroundMode::Solid,
                ViaMode::Breathe => BackgroundMode::Breathe,
            }),
        }
    }

    impl<const OVERLAY_CAP: usize, const MAILBOX_CAP: usize> ViaRgbMatrixControl
        for StandardControl<'_, OVERLAY_CAP, MAILBOX_CAP>
    {
        type Error = Error;

        async fn read_background(&self) -> Result<ViaState, Self::Error> {
            self.mailbox
                .request(StandardCommand::ReadState)
                .await
                .map_err(Error::Lighting)
                .and_then(reply_state)
                .map(state)
        }

        async fn patch_background(&self, update: ViaPatch) -> Result<ViaState, Self::Error> {
            self.mailbox
                .request(StandardCommand::PatchBackground(patch(update)))
                .await
                .map_err(Error::Lighting)
                .and_then(reply_state)
                .map(state)
        }

        async fn save_background(&self) -> Result<(), Self::Error> {
            Err(Error::PersistenceUnavailable)
        }
    }
}

#[cfg(feature = "lighting")]
pub use standard_control::StandardControl;

#[cfg(test)]
mod tests {
    use core::cell::{Cell, RefCell};

    use embassy_futures::block_on;

    use super::*;

    struct FakeControl {
        state: RefCell<BackgroundState>,
        patches: Cell<u8>,
        saves: Cell<u8>,
        save_supported: bool,
    }

    impl FakeControl {
        fn new(state: BackgroundState) -> Self {
            Self {
                state: RefCell::new(state),
                patches: Cell::new(0),
                saves: Cell::new(0),
                save_supported: false,
            }
        }

        fn with_save(mut self) -> Self {
            self.save_supported = true;
            self
        }
    }

    fn apply(patch: BackgroundPatch, state: &mut BackgroundState) {
        if let Some(value) = patch.enabled {
            state.enabled = value;
        }
        if let Some(value) = patch.hue {
            state.hue = value;
        }
        if let Some(value) = patch.saturation {
            state.saturation = value;
        }
        if let Some(value) = patch.value {
            state.value = value;
        }
        if let Some(value) = patch.speed {
            state.speed = value;
        }
        if let Some(value) = patch.mode {
            state.mode = value;
        }
    }

    impl ViaRgbMatrixControl for FakeControl {
        type Error = ();

        async fn read_background(&self) -> Result<BackgroundState, Self::Error> {
            Ok(*self.state.borrow())
        }

        async fn patch_background(&self, patch: BackgroundPatch) -> Result<BackgroundState, Self::Error> {
            self.patches.set(self.patches.get() + 1);
            apply(patch, &mut self.state.borrow_mut());
            Ok(*self.state.borrow())
        }

        async fn save_background(&self) -> Result<(), Self::Error> {
            if !self.save_supported {
                return Err(());
            }
            self.saves.set(self.saves.get() + 1);
            Ok(())
        }
    }

    fn initial() -> BackgroundState {
        BackgroundState {
            enabled: true,
            hue: 11,
            saturation: 22,
            value: 33,
            speed: 44,
            mode: BackgroundMode::Breathe,
        }
    }

    fn packet(command: u8, value: u8, first: u8, second: u8) -> [u8; 32] {
        let mut packet = [0; 32];
        packet[..5].copy_from_slice(&[command, CHANNEL, value, first, second]);
        packet
    }

    fn process(control: &impl ViaRgbMatrixControl, request: [u8; 32]) -> [u8; 32] {
        let mut response = request;
        block_on(process_packet(control, &request, &mut response));
        response
    }

    #[test]
    fn set_values_apply_atomic_patches_and_preserve_other_fields() {
        let control = FakeControl::new(initial());
        process(&control, packet(COMMAND_SET, VALUE_BRIGHTNESS, 99, 0));
        process(&control, packet(COMMAND_SET, VALUE_SPEED, 88, 0));
        process(&control, packet(COMMAND_SET, VALUE_COLOR, 77, 66));

        assert_eq!(control.patches.get(), 3);
        assert_eq!(
            *control.state.borrow(),
            BackgroundState {
                enabled: true,
                hue: 77,
                saturation: 66,
                value: 99,
                speed: 88,
                mode: BackgroundMode::Breathe,
            }
        );
    }

    #[test]
    fn off_preserves_mode_and_supported_effects_enable_atomically() {
        let control = FakeControl::new(initial());
        process(&control, packet(COMMAND_SET, VALUE_EFFECT, EFFECT_OFF, 0));
        assert!(!control.state.borrow().enabled);
        assert_eq!(control.state.borrow().mode, BackgroundMode::Breathe);

        process(&control, packet(COMMAND_SET, VALUE_EFFECT, EFFECT_SOLID, 0));
        assert!(control.state.borrow().enabled);
        assert_eq!(control.state.borrow().mode, BackgroundMode::Solid);

        process(&control, packet(COMMAND_SET, VALUE_EFFECT, EFFECT_BREATHE, 0));
        assert!(control.state.borrow().enabled);
        assert_eq!(control.state.borrow().mode, BackgroundMode::Breathe);
    }

    #[test]
    fn get_values_return_via_wire_ids_and_data() {
        let control = FakeControl::new(initial());
        assert_eq!(process(&control, packet(COMMAND_GET, VALUE_BRIGHTNESS, 0, 0))[3], 33);
        assert_eq!(
            process(&control, packet(COMMAND_GET, VALUE_EFFECT, 0, 0))[3],
            EFFECT_BREATHE
        );
        assert_eq!(process(&control, packet(COMMAND_GET, VALUE_SPEED, 0, 0))[3], 44);
        assert_eq!(
            &process(&control, packet(COMMAND_GET, VALUE_COLOR, 0, 0))[3..5],
            &[11, 22]
        );
    }

    #[test]
    fn unsupported_operations_are_unhandled_without_mutation() {
        let control = FakeControl::new(initial());
        let before = *control.state.borrow();
        let mut wrong_channel = packet(COMMAND_SET, VALUE_BRIGHTNESS, 1, 0);
        wrong_channel[1] = 2;
        for request in [
            wrong_channel,
            packet(COMMAND_SET, 99, 1, 0),
            packet(COMMAND_SET, VALUE_EFFECT, 2, 0),
        ] {
            assert_eq!(process(&control, request)[0], COMMAND_UNHANDLED);
        }
        assert_eq!(control.patches.get(), 0);
        assert_eq!(*control.state.borrow(), before);
    }

    #[test]
    fn save_is_honest_about_persistence_capability() {
        let unsupported = FakeControl::new(initial());
        assert_eq!(
            process(&unsupported, packet(COMMAND_SAVE, 0, 0, 0))[0],
            COMMAND_UNHANDLED
        );
        assert_eq!(unsupported.saves.get(), 0);

        let supported = FakeControl::new(initial()).with_save();
        assert_eq!(process(&supported, packet(COMMAND_SAVE, 0, 0, 0))[0], COMMAND_SAVE);
        assert_eq!(supported.saves.get(), 1);
    }

    #[cfg(feature = "lighting")]
    #[test]
    fn standard_mailbox_control_does_not_claim_persistence() {
        use crate::lighting::processor::LightingMailbox;
        use crate::lighting::standard::{StandardCommand, StandardError, StandardReply};

        let mailbox = LightingMailbox::<StandardCommand<1>, StandardReply, StandardError, 1>::new();
        let control = StandardControl::new(&mailbox);
        assert_eq!(
            block_on(control.save_background()),
            Err(standard_control::Error::PersistenceUnavailable)
        );
    }
}
