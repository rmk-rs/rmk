//! Type-erased Rynk lighting binding and standard-engine mailbox bridge.
//!
//! `RynkService` deliberately stays non-generic so existing USB/BLE/UART
//! orchestration does not acquire lighting-engine type parameters. A small
//! protocol mailbox erases those parameters; [`StandardRynkLightingAdapter`]
//! is the only bridge from that mailbox to the standard engine's authoritative
//! mailbox.

use core::cell::{Cell, RefCell};
use core::num::NonZeroU32;

use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use heapless::Vec;
use rmk_types::protocol::rynk::{
    LightingBackgroundMode, LightingBackgroundState, LightingError, LightingMutableState, LightingOverlayCell,
    LightingResult, LightingRgb8, LightingState,
};

use crate::RawMutex;
use crate::core_traits::Runnable;
use crate::lighting::{
    BackgroundMode, BackgroundState, BuiltinEffect, LedId, LightingMailbox, LightingRouting, LightingTopology,
    OverlayBatch, OverlayCell, OverlayError, Rgb8, StandardCommand, StandardError, StandardMutableState, StandardState,
};

/// Maximum number of cells staged by one Rynk overlay replacement.
///
/// A concrete engine may advertise a smaller capacity. Keeping the protocol
/// staging bound independent of the engine's const generic avoids making the
/// host service and every transport generic.
pub const RYNK_LIGHTING_TRANSACTION_CAPACITY: usize = 64;

const RYNK_LIGHTING_COMMAND_CAPACITY: usize = 4;

#[derive(Clone, Copy)]
pub struct RynkLightingDescriptor<'a> {
    pub topology_revision: u32,
    pub topology: LightingTopology<'a>,
    pub routing: LightingRouting<'a>,
}

/// Concrete, type-erased lighting attachment carried by [`super::RynkService`].
#[derive(Clone, Copy)]
pub struct RynkLightingController<'a> {
    pub(super) descriptor: RynkLightingDescriptor<'a>,
    pub(super) overlay_capacity: u16,
    mailbox: &'a RynkLightingMailbox,
}

impl<'a> RynkLightingController<'a> {
    pub const fn new(
        mailbox: &'a RynkLightingMailbox,
        descriptor: RynkLightingDescriptor<'a>,
        overlay_capacity: u16,
    ) -> Self {
        let staged_capacity = RYNK_LIGHTING_TRANSACTION_CAPACITY as u16;
        Self {
            descriptor,
            // The single advertised limit must be valid for both incremental
            // updates and atomic replacement.
            overlay_capacity: if overlay_capacity < staged_capacity {
                overlay_capacity
            } else {
                staged_capacity
            },
            mailbox,
        }
    }

    pub const fn descriptor(&self) -> RynkLightingDescriptor<'a> {
        self.descriptor
    }

    pub const fn overlay_capacity(&self) -> u16 {
        self.overlay_capacity
    }

    pub(super) async fn request(&self, command: RynkLightingCommand) -> LightingResult<LightingState> {
        self.mailbox.request(command).await
    }

    pub(super) async fn replace_overlay(
        &self,
        expected_revision: u32,
        cells: &Vec<LightingOverlayCell, RYNK_LIGHTING_TRANSACTION_CAPACITY>,
    ) -> LightingResult<LightingState> {
        self.mailbox.request_replace(expected_revision, cells).await
    }
}

/// Fixed protocol-facing mailbox. Its adapter forwards every request to the
/// standard engine mailbox; it never owns renderer or compositor state.
pub struct RynkLightingMailbox {
    requests: Channel<RawMutex, MailboxRequest, RYNK_LIGHTING_COMMAND_CAPACITY>,
    response: Signal<RawMutex, MailboxResponse>,
    caller: Mutex<RawMutex, ()>,
    next_id: BlockingMutex<RawMutex, Cell<u32>>,
    replacement: BlockingMutex<RawMutex, RefCell<Option<StagedReplacement>>>,
    replacement_ready: Signal<RawMutex, ()>,
    replacement_available: Signal<RawMutex, ()>,
}

struct StagedReplacement {
    id: u32,
    cells: Vec<LightingOverlayCell, RYNK_LIGHTING_TRANSACTION_CAPACITY>,
}

pub(in crate::host::rynk) struct MailboxRequest {
    pub(in crate::host::rynk) id: u32,
    pub(in crate::host::rynk) command: RynkLightingCommand,
}

struct MailboxResponse {
    id: u32,
    result: LightingResult<LightingState>,
}

impl RynkLightingMailbox {
    pub const fn new() -> Self {
        Self {
            requests: Channel::new(),
            response: Signal::new(),
            caller: Mutex::new(()),
            next_id: BlockingMutex::new(Cell::new(0)),
            replacement: BlockingMutex::new(RefCell::new(None)),
            replacement_ready: Signal::new(),
            replacement_available: Signal::new(),
        }
    }

    async fn request(&self, command: RynkLightingCommand) -> LightingResult<LightingState> {
        let _caller = self.caller.lock().await;
        let id = self.allocate_id();
        self.send_and_wait(id, command).await
    }

    async fn request_replace(
        &self,
        expected_revision: u32,
        cells: &Vec<LightingOverlayCell, RYNK_LIGHTING_TRANSACTION_CAPACITY>,
    ) -> LightingResult<LightingState> {
        let _caller = self.caller.lock().await;
        while self.replacement.lock(|replacement| replacement.borrow().is_some()) {
            self.replacement_available.wait().await;
        }

        let id = self.allocate_id();
        // Enqueue first. After this await, staging and its ready signal are
        // synchronous, so cancellation cannot leave an orphaned stage without
        // a matching command. The adapter may receive the token first and wait.
        self.requests
            .send(MailboxRequest {
                id,
                command: RynkLightingCommand::ReplaceOverlay { expected_revision },
            })
            .await;
        self.replacement.lock(|replacement| {
            let previous = replacement.borrow_mut().replace(StagedReplacement {
                id,
                cells: cells.clone(),
            });
            debug_assert!(previous.is_none(), "replacement slot was checked before enqueue");
        });
        self.replacement_ready.signal(());
        self.wait_for_reply(id).await
    }

    fn allocate_id(&self) -> u32 {
        self.next_id.lock(|next| {
            let id = next.get();
            next.set(id.wrapping_add(1));
            id
        })
    }

    async fn send_and_wait(&self, id: u32, command: RynkLightingCommand) -> LightingResult<LightingState> {
        self.requests.send(MailboxRequest { id, command }).await;
        self.wait_for_reply(id).await
    }

    async fn wait_for_reply(&self, id: u32) -> LightingResult<LightingState> {
        loop {
            let response = self.response.wait().await;
            if response.id == id {
                return response.result;
            }
        }
    }

    pub(in crate::host::rynk) async fn receive(&self) -> MailboxRequest {
        self.requests.receive().await
    }

    pub(in crate::host::rynk) fn reply(&self, id: u32, result: LightingResult<LightingState>) {
        self.response.signal(MailboxResponse { id, result });
    }

    pub(in crate::host::rynk) async fn take_replacement(
        &self,
        id: u32,
    ) -> Vec<LightingOverlayCell, RYNK_LIGHTING_TRANSACTION_CAPACITY> {
        loop {
            let staged = self.replacement.lock(|replacement| {
                let mut replacement = replacement.borrow_mut();
                if replacement.as_ref().is_some_and(|staged| staged.id == id) {
                    replacement.take()
                } else {
                    None
                }
            });
            if let Some(staged) = staged {
                self.replacement_available.signal(());
                return staged.cells;
            }
            self.replacement_ready.wait().await;
        }
    }
}

impl Default for RynkLightingMailbox {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) enum RynkLightingCommand {
    ReadState,
    SetState {
        expected_revision: u32,
        state: LightingMutableState,
    },
    SetOverlay {
        expected_revision: u32,
        cell: LightingOverlayCell,
    },
    UnsetOverlay {
        expected_revision: u32,
        led_id: rmk_types::protocol::rynk::LightingLedId,
    },
    ClearOverlay {
        expected_revision: u32,
    },
    ReplaceOverlay {
        expected_revision: u32,
    },
}

/// Bridges type-erased Rynk commands into one concrete standard lighting
/// mailbox. Boards spawn this alongside `LightingProcessor`.
pub struct StandardRynkLightingAdapter<'a, const OVERLAY_CAPACITY: usize, const CORE_COMMAND_CAPACITY: usize> {
    protocol: &'a RynkLightingMailbox,
    core: &'a LightingMailbox<StandardCommand<OVERLAY_CAPACITY>, StandardState, StandardError, CORE_COMMAND_CAPACITY>,
    topology: LightingTopology<'a>,
}

impl<'a, const OVERLAY_CAPACITY: usize, const CORE_COMMAND_CAPACITY: usize>
    StandardRynkLightingAdapter<'a, OVERLAY_CAPACITY, CORE_COMMAND_CAPACITY>
{
    pub const fn new(
        protocol: &'a RynkLightingMailbox,
        core: &'a LightingMailbox<
            StandardCommand<OVERLAY_CAPACITY>,
            StandardState,
            StandardError,
            CORE_COMMAND_CAPACITY,
        >,
        topology: LightingTopology<'a>,
    ) -> Self {
        Self {
            protocol,
            core,
            topology,
        }
    }

    /// Process one protocol command. Exposed for board-specific executors and
    /// deterministic tests; [`Runnable`] simply repeats it forever.
    pub async fn process_next(&mut self) {
        let request = self.protocol.receive().await;
        let result = self.dispatch(request.id, request.command).await;
        self.protocol.reply(request.id, result);
    }

    async fn dispatch(&self, request_id: u32, command: RynkLightingCommand) -> LightingResult<LightingState> {
        let core_command = match command {
            RynkLightingCommand::ReadState => StandardCommand::ReadState,
            RynkLightingCommand::SetState {
                expected_revision,
                state,
            } => StandardCommand::SetStateIfRevision {
                expected_revision,
                state: mutable_state_from_wire(state),
            },
            RynkLightingCommand::SetOverlay {
                expected_revision,
                cell,
            } => StandardCommand::SetOverlayIfRevision {
                expected_revision,
                cell: self.overlay_cell(cell)?,
            },
            RynkLightingCommand::UnsetOverlay {
                expected_revision,
                led_id,
            } => StandardCommand::UnsetOverlayIfRevision {
                expected_revision,
                slot: self
                    .topology
                    .slot(LedId(led_id.0))
                    .ok_or(LightingError::UnknownLed { led_id })?,
            },
            RynkLightingCommand::ClearOverlay { expected_revision } => {
                StandardCommand::ClearOverlayIfRevision { expected_revision }
            }
            RynkLightingCommand::ReplaceOverlay { expected_revision } => {
                let cells = self.protocol.take_replacement(request_id).await;
                let mut batch = OverlayBatch::new();
                for cell in cells {
                    let cell = self.overlay_cell(cell)?;
                    if batch.as_slice().iter().any(|existing| existing.slot == cell.slot) {
                        return Err(LightingError::InvalidRequest);
                    }
                    batch
                        .push(cell)
                        .map_err(|error| map_overlay_error(error, OVERLAY_CAPACITY))?;
                }
                StandardCommand::ReplaceOverlayIfRevision {
                    expected_revision,
                    batch,
                }
            }
        };

        self.core
            .request(core_command)
            .await
            .map(state_to_wire)
            .map_err(|error| map_standard_error(error, OVERLAY_CAPACITY))
    }

    fn overlay_cell(&self, cell: LightingOverlayCell) -> LightingResult<OverlayCell> {
        cell.validate()?;
        let slot = self
            .topology
            .slot(LedId(cell.led_id.0))
            .ok_or(LightingError::UnknownLed { led_id: cell.led_id })?;
        Ok(OverlayCell {
            slot,
            effect: effect_from_wire(cell.effect),
            ttl_ms: cell.ttl_ms.and_then(NonZeroU32::new),
        })
    }
}

impl<const OVERLAY_CAPACITY: usize, const CORE_COMMAND_CAPACITY: usize> Runnable
    for StandardRynkLightingAdapter<'_, OVERLAY_CAPACITY, CORE_COMMAND_CAPACITY>
{
    async fn run(&mut self) -> ! {
        loop {
            self.process_next().await;
        }
    }
}

fn mutable_state_from_wire(state: LightingMutableState) -> StandardMutableState {
    StandardMutableState {
        output_enabled: state.output_enabled,
        output_brightness: state.output_brightness,
        background: BackgroundState {
            enabled: state.background.enabled,
            hue: state.background.hue,
            saturation: state.background.saturation,
            value: state.background.value,
            speed: state.background.speed,
            mode: match state.background.mode {
                LightingBackgroundMode::Solid => BackgroundMode::Solid,
                LightingBackgroundMode::Breathe => BackgroundMode::Breathe,
            },
        },
    }
}

pub(super) fn state_to_wire(state: StandardState) -> LightingState {
    LightingState {
        revision: state.revision,
        output_enabled: state.output_enabled,
        output_brightness: state.output_brightness,
        background: LightingBackgroundState {
            enabled: state.background.enabled,
            hue: state.background.hue,
            saturation: state.background.saturation,
            value: state.background.value,
            speed: state.background.speed,
            mode: match state.background.mode {
                BackgroundMode::Solid => LightingBackgroundMode::Solid,
                BackgroundMode::Breathe => LightingBackgroundMode::Breathe,
            },
        },
        overlay_len: state.overlay_len.min(u16::MAX as usize) as u16,
    }
}

fn effect_from_wire(effect: rmk_types::protocol::rynk::LightingEffect) -> BuiltinEffect {
    use rmk_types::protocol::rynk::LightingEffect;

    match effect {
        LightingEffect::Solid { color } => BuiltinEffect::Solid {
            color: rgb_from_wire(color),
        },
        LightingEffect::Blink {
            color,
            period_ms,
            phase_ms,
            duty,
        } => BuiltinEffect::Blink {
            color: rgb_from_wire(color),
            period_ms,
            phase_ms,
            duty,
        },
        LightingEffect::Breathe {
            color,
            period_ms,
            phase_ms,
            step_ms,
        } => BuiltinEffect::Breathe {
            color: rgb_from_wire(color),
            period_ms,
            phase_ms,
            step_ms,
        },
    }
}

const fn rgb_from_wire(color: LightingRgb8) -> Rgb8 {
    Rgb8::new(color.r, color.g, color.b)
}

fn map_standard_error(error: StandardError, capacity: usize) -> LightingError {
    match error {
        StandardError::RevisionConflict { expected, current } => {
            LightingError::StateRevisionConflict { expected, current }
        }
        StandardError::Overlay(error) => map_overlay_error(error, capacity),
        StandardError::DeadlineOverflow => LightingError::InvalidTtl,
        StandardError::Render(_) => LightingError::Unsupported,
    }
}

fn map_overlay_error(error: OverlayError, capacity: usize) -> LightingError {
    match error {
        OverlayError::Full | OverlayError::TooManyEntries { .. } => LightingError::OverlayFull {
            capacity: capacity.min(u16::MAX as usize) as u16,
        },
        OverlayError::DuplicateSlot { .. } => LightingError::InvalidRequest,
    }
}

#[cfg(test)]
mod tests {
    use embassy_futures::join::join;
    use embassy_futures::select::{Either, select};
    use embassy_futures::yield_now;
    use rmk_types::protocol::rynk::{
        LightingBackgroundMode, LightingBackgroundState, LightingEffect, LightingLedId, LightingOverlayCell,
        LightingRgb8,
    };

    use super::*;
    use crate::lighting::{LedMetadata, MatrixSize, ZoneSpan};
    use crate::physical_layout::PhysicalLayout;
    use crate::test_support::test_block_on as block_on;

    static TEST_LEDS: [LedMetadata; 1] = [LedMetadata {
        id: LedId(10),
        key: None,
        position: None,
        zones: ZoneSpan::new(0, 0),
    }];

    fn topology() -> LightingTopology<'static> {
        LightingTopology {
            matrix: MatrixSize::new(0, 0),
            keys: &[],
            physical_layout: PhysicalLayout::EMPTY,
            leds: &TEST_LEDS,
            zones: &[],
            zone_memberships: &[],
        }
    }

    fn cell(id: u16) -> LightingOverlayCell {
        LightingOverlayCell {
            led_id: LightingLedId(id),
            effect: LightingEffect::Solid {
                color: LightingRgb8 { r: 1, g: 2, b: 3 },
            },
            ttl_ms: None,
        }
    }

    fn state(revision: u32) -> LightingState {
        LightingState {
            revision,
            output_enabled: true,
            output_brightness: 255,
            background: LightingBackgroundState {
                enabled: true,
                hue: 0,
                saturation: 0,
                value: 1,
                speed: 0,
                mode: LightingBackgroundMode::Solid,
            },
            overlay_len: 1,
        }
    }

    #[test]
    fn cancelled_replacement_cannot_be_overwritten_by_the_next_caller() {
        let mailbox = RynkLightingMailbox::new();
        let mut first = Vec::new();
        first.push(cell(10)).unwrap();

        let cancelled = block_on(select(mailbox.request_replace(0, &first), async {
            yield_now().await;
        }));
        assert!(matches!(cancelled, Either::Second(())));

        let mut second = Vec::new();
        second.push(cell(20)).unwrap();
        let (reply, ()) = block_on(join(mailbox.request_replace(1, &second), async {
            let abandoned = mailbox.receive().await;
            assert!(matches!(
                abandoned.command,
                RynkLightingCommand::ReplaceOverlay { expected_revision: 0 }
            ));
            let abandoned_cells = mailbox.take_replacement(abandoned.id).await;
            assert_eq!(abandoned_cells.as_slice(), &[cell(10)]);
            mailbox.reply(abandoned.id, Ok(state(1)));

            let current = mailbox.receive().await;
            assert!(matches!(
                current.command,
                RynkLightingCommand::ReplaceOverlay { expected_revision: 1 }
            ));
            let current_cells = mailbox.take_replacement(current.id).await;
            assert_eq!(current_cells.as_slice(), &[cell(20)]);
            mailbox.reply(current.id, Ok(state(2)));
        }));
        assert_eq!(reply, Ok(state(2)));
    }

    #[test]
    fn replacement_rejects_duplicate_stable_ids_before_reaching_the_core() {
        let protocol = RynkLightingMailbox::new();
        let core = LightingMailbox::<StandardCommand<2>, StandardState, StandardError, 1>::new();
        let mut adapter = StandardRynkLightingAdapter::new(&protocol, &core, topology());
        let mut cells = Vec::new();
        cells.push(cell(10)).unwrap();
        cells.push(cell(10)).unwrap();

        let (reply, ()) = block_on(join(protocol.request_replace(0, &cells), adapter.process_next()));
        assert_eq!(reply, Err(LightingError::InvalidRequest));
    }
}
