//! Executor-independent lighting service ownership and scheduling.
//!
//! The service in this module deliberately knows nothing about RMK events,
//! Embassy, pixels, or physical LED protocols. An adapter translates its event
//! types into [`LightingEngine::Input`], polls [`LightingService::next_action`],
//! performs the requested asynchronous output operation, and reports the
//! result with [`LightingService::complete_output`]. Keeping that loop outside
//! the core makes command ordering, deadlines, and retry behavior deterministic
//! in host tests.

use core::num::NonZeroU32;

/// Read-only access to the current authoritative keyboard state.
///
/// Notifications are only invalidations: a consumer must take a fresh
/// snapshot instead of reconstructing state from event history. This lets
/// lossy or coalesced state notifications still converge after startup and
/// bursts of changes. Edge-sensitive operations belong in
/// [`LightingEngine::Input`] or [`LightingEngine::Command`].
pub trait SnapshotProvider {
    type Snapshot;

    fn snapshot(&self) -> Self::Snapshot;
}

/// Authoritative state-change and render invalidation produced by an engine.
///
/// `Render` means engine-owned state changed and another render is required.
/// Application-owned state such as layers requests a render directly through
/// [`LightingService::request_render`] and therefore does not emit a lighting
/// state-change notification.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Invalidation {
    #[default]
    None,
    Render,
    /// Engine-owned state changed without changing the visible frame.
    StateChanged,
}

impl Invalidation {
    pub const fn requires_render(self) -> bool {
        matches!(self, Self::Render)
    }

    pub const fn state_changed(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Result of a serialized mutation request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandResult<R> {
    pub reply: R,
    pub invalidation: Invalidation,
}

impl<R> CommandResult<R> {
    pub const fn new(reply: R, invalidation: Invalidation) -> Self {
        Self { reply, invalidation }
    }

    pub const fn unchanged(reply: R) -> Self {
        Self::new(reply, Invalidation::None)
    }

    pub const fn render(reply: R) -> Self {
        Self::new(reply, Invalidation::Render)
    }

    pub const fn changed(reply: R) -> Self {
        Self::new(reply, Invalidation::StateChanged)
    }
}

/// Inputs to one pure render pass.
#[derive(Clone, Copy, Debug)]
pub struct RenderInput<'a, S> {
    pub now_ms: u64,
    pub snapshot: &'a S,
}

/// Result of one render pass.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RenderOutcome {
    /// The completed frame differs from the last successfully presented frame.
    pub changed: bool,
    /// Rendering itself changed authoritative engine state, for example by
    /// pruning an expired overlay. The service coalesces this into one unit
    /// invalidation for consumers to refresh their snapshot.
    pub state_changed: bool,
    /// Positive relative delay until visible output can next differ.
    ///
    /// `None` means static output and therefore no render timer. Relative time
    /// keeps the engine independent of an executor's clock representation.
    pub next_wake_in_ms: Option<NonZeroU32>,
}

impl RenderOutcome {
    pub const fn static_frame(changed: bool) -> Self {
        Self {
            changed,
            state_changed: false,
            next_wake_in_ms: None,
        }
    }
}

/// Stateful rendering policy owned exclusively by a [`LightingService`].
///
/// `Frame` is the *whole* logical output shape selected by the board. It may
/// be a conventional dense RGB frame, but it can equally be a struct combining
/// RGB chains, PWM channels, and indicator bits. This boundary intentionally
/// makes no homogeneous-slice assumption.
pub trait LightingEngine<S> {
    type Frame;
    type Input;
    type Command;
    type Reply;
    type Error;

    /// Consume an edge-sensitive or transient input.
    fn on_input(&mut self, input: Self::Input, snapshot: &S) -> Result<Invalidation, Self::Error>;

    /// Apply one serialized mutation and return protocol-independent readback.
    fn handle_command(
        &mut self,
        now_ms: u64,
        command: Self::Command,
        snapshot: &S,
    ) -> Result<CommandResult<Self::Reply>, Self::Error>;

    /// Render the complete logical frame from current authoritative state.
    fn render(&mut self, input: RenderInput<'_, S>, frame: &mut Self::Frame) -> Result<RenderOutcome, Self::Error>;

    /// Commit renderer history after, and only after, successful presentation.
    ///
    /// A compositor commonly uses this hook to update the frame against which
    /// its next `changed` result is calculated. Failed writes must not call it.
    fn on_presented(&mut self, _frame: &Self::Frame) {}
}

/// Hardware-facing lifecycle and presentation contract.
///
/// The pure [`LightingService`] does not run these futures. An RMK or other
/// executor adapter performs the operation requested by [`ServiceAction`] and
/// reports its result. Implementations retain electrical ordering, encoding,
/// power sequencing, and hard safety limits.
#[allow(async_fn_in_trait)]
pub trait LightingOutput<F> {
    type Error;

    async fn initialize(&mut self) -> Result<(), Self::Error>;
    async fn present(&mut self, frame: &F) -> Result<(), Self::Error>;
    async fn suspend(&mut self) -> Result<(), Self::Error>;
    async fn resume(&mut self) -> Result<(), Self::Error>;

    /// Select an automatic retry delay for a failed operation.
    ///
    /// Returning `None` blocks that operation until the adapter explicitly
    /// calls [`LightingService::retry_output_now`].
    fn retry_after(&self, _operation: OutputOperation, _error: &Self::Error) -> Option<NonZeroU32> {
        None
    }
}

/// Desired logical power state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PowerState {
    #[default]
    Active,
    Suspended,
}

/// Last successfully completed output lifecycle state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OutputState {
    #[default]
    Uninitialized,
    Active,
    Suspended,
}

/// One operation performed by the executor-facing adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputOperation {
    Initialize,
    Present,
    Suspend,
    Resume,
}

/// The next externally performed service action.
#[derive(Debug, Eq, PartialEq)]
pub enum ServiceAction<'a, F> {
    /// No immediate work. `next_wake_ms` is the earliest absolute instant at
    /// which the adapter should poll again, or `None` when only an external
    /// input/command can make progress.
    Wait {
        next_wake_ms: Option<u64>,
    },
    Initialize,
    Present(&'a F),
    Suspend,
    Resume,
}

impl<F> ServiceAction<'_, F> {
    pub const fn operation(&self) -> Option<OutputOperation> {
        match self {
            Self::Wait { .. } => None,
            Self::Initialize => Some(OutputOperation::Initialize),
            Self::Present(_) => Some(OutputOperation::Present),
            Self::Suspend => Some(OutputOperation::Suspend),
            Self::Resume => Some(OutputOperation::Resume),
        }
    }
}

/// Result reported after an output operation completes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputCompletion {
    Succeeded,
    Failed {
        /// Positive relative retry delay selected by
        /// [`LightingOutput::retry_after`]. `None` requires explicit retry.
        retry_in_ms: Option<NonZeroU32>,
    },
}

/// Failure while calculating the next service action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceError<E> {
    Engine(E),
    OperationInFlight(OutputOperation),
}

/// Invalid completion/retry call made by an adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionError {
    NoOperationInFlight,
    OperationInFlight(OutputOperation),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RetryState {
    operation: OutputOperation,
    at_ms: Option<u64>,
}

/// Synchronous ownership, scheduling, and output-retry state machine.
///
/// The service owns the snapshot provider, engine, and complete logical frame.
/// It intentionally does not own an async output object: this keeps the state
/// transitions directly testable and lets RMK, another executor, or a blocking
/// host harness drive the exact same logic.
pub struct LightingService<P, E>
where
    P: SnapshotProvider,
    E: LightingEngine<P::Snapshot>,
{
    provider: P,
    engine: E,
    frame: E::Frame,
    desired_power: PowerState,
    output_state: OutputState,
    dirty: bool,
    present_required: bool,
    next_render_ms: Option<u64>,
    retry: Option<RetryState>,
    in_flight: Option<OutputOperation>,
    lighting_change_pending: bool,
}

impl<P, E> LightingService<P, E>
where
    P: SnapshotProvider,
    E: LightingEngine<P::Snapshot>,
{
    /// Construct a service with explicit board-selected frame storage.
    ///
    /// The first active poll initializes the output, renders authoritative
    /// state, and presents once even if the engine reports `changed == false`.
    pub const fn new(provider: P, engine: E, frame: E::Frame) -> Self {
        Self {
            provider,
            engine,
            frame,
            desired_power: PowerState::Active,
            output_state: OutputState::Uninitialized,
            dirty: true,
            present_required: true,
            next_render_ms: None,
            retry: None,
            in_flight: None,
            lighting_change_pending: false,
        }
    }

    pub const fn desired_power(&self) -> PowerState {
        self.desired_power
    }

    pub const fn output_state(&self) -> OutputState {
        self.output_state
    }

    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub const fn presentation_pending(&self) -> bool {
        self.present_required
    }

    pub const fn next_render_ms(&self) -> Option<u64> {
        self.next_render_ms
    }

    pub const fn in_flight(&self) -> Option<OutputOperation> {
        self.in_flight
    }

    pub const fn lighting_change_pending(&self) -> bool {
        self.lighting_change_pending
    }

    /// Take one coalesced authoritative-lighting-state invalidation.
    pub fn take_lighting_change(&mut self) -> bool {
        core::mem::take(&mut self.lighting_change_pending)
    }

    pub const fn frame(&self) -> &E::Frame {
        &self.frame
    }

    pub const fn engine(&self) -> &E {
        &self.engine
    }

    pub fn engine_mut(&mut self) -> &mut E {
        &mut self.engine
    }

    pub const fn snapshot_provider(&self) -> &P {
        &self.provider
    }

    pub fn snapshot_provider_mut(&mut self) -> &mut P {
        &mut self.provider
    }

    /// Request active or suspended hardware state.
    ///
    /// Power transitions supersede a blocked retry of a now-irrelevant output
    /// operation. Rendering deadlines are disarmed while suspended; current
    /// state is rendered and force-presented after a successful resume.
    pub fn set_power(&mut self, power: PowerState) -> Result<(), CompletionError> {
        if let Some(operation) = self.in_flight {
            return Err(CompletionError::OperationInFlight(operation));
        }
        if self.desired_power != power {
            self.desired_power = power;
            self.retry = None;
            if power == PowerState::Suspended {
                self.next_render_ms = None;
            }
        }
        Ok(())
    }

    /// Deliver one edge-sensitive input using a fresh authoritative snapshot.
    pub fn on_input(&mut self, input: E::Input) -> Result<(), E::Error> {
        let snapshot = self.provider.snapshot();
        let invalidation = self.engine.on_input(input, &snapshot)?;
        self.invalidate(invalidation);
        Ok(())
    }

    /// Apply one mutation synchronously through the sole mutable owner.
    pub fn handle_command(&mut self, now_ms: u64, command: E::Command) -> Result<E::Reply, E::Error> {
        let snapshot = self.provider.snapshot();
        let result = self.engine.handle_command(now_ms, command, &snapshot)?;
        self.invalidate(result.invalidation);
        Ok(result.reply)
    }

    /// Explicitly request a render after application-owned state changes.
    pub fn request_render(&mut self) {
        self.dirty = true;
    }

    /// Unblock the last failed operation without waiting for another event.
    pub fn retry_output_now(&mut self) -> Result<(), CompletionError> {
        if let Some(operation) = self.in_flight {
            return Err(CompletionError::OperationInFlight(operation));
        }
        self.retry = None;
        Ok(())
    }

    /// Produce the next action and mark output operations as in flight.
    ///
    /// Callers must report every non-`Wait` action through
    /// [`complete_output`](Self::complete_output) before polling again.
    pub fn next_action(&mut self, now_ms: u64) -> Result<ServiceAction<'_, E::Frame>, ServiceError<E::Error>> {
        if let Some(operation) = self.in_flight {
            return Err(ServiceError::OperationInFlight(operation));
        }

        let lifecycle_operation = match (self.output_state, self.desired_power) {
            (OutputState::Uninitialized, _) => Some(OutputOperation::Initialize),
            (OutputState::Active, PowerState::Suspended) => Some(OutputOperation::Suspend),
            (OutputState::Suspended, PowerState::Active) => Some(OutputOperation::Resume),
            _ => None,
        };
        if let Some(operation) = lifecycle_operation {
            if self.operation_ready(operation, now_ms) {
                return Ok(self.begin_operation(operation));
            }
            return Ok(ServiceAction::Wait {
                next_wake_ms: self.wait_deadline(),
            });
        }

        if self.output_state == OutputState::Suspended {
            return Ok(ServiceAction::Wait { next_wake_ms: None });
        }

        if self.next_render_ms.is_some_and(|deadline| deadline <= now_ms) {
            self.next_render_ms = None;
            self.dirty = true;
        }

        if self.dirty {
            let snapshot = self.provider.snapshot();
            let outcome = self
                .engine
                .render(
                    RenderInput {
                        now_ms,
                        snapshot: &snapshot,
                    },
                    &mut self.frame,
                )
                .map_err(ServiceError::Engine)?;
            self.dirty = false;
            self.next_render_ms = outcome
                .next_wake_in_ms
                .and_then(|delay| now_ms.checked_add(delay.get() as u64));
            self.present_required |= outcome.changed;
            self.lighting_change_pending |= outcome.state_changed;
        }

        if self.present_required {
            if self.operation_ready(OutputOperation::Present, now_ms) {
                self.in_flight = Some(OutputOperation::Present);
                return Ok(ServiceAction::Present(&self.frame));
            }
            return Ok(ServiceAction::Wait {
                next_wake_ms: self.wait_deadline(),
            });
        }

        // A successful render/present makes any stale presentation retry moot.
        if self
            .retry
            .is_some_and(|retry| retry.operation == OutputOperation::Present)
        {
            self.retry = None;
        }
        Ok(ServiceAction::Wait {
            next_wake_ms: self.wait_deadline(),
        })
    }

    /// Complete the operation most recently returned by [`next_action`].
    pub fn complete_output(&mut self, now_ms: u64, completion: OutputCompletion) -> Result<(), CompletionError> {
        let operation = self.in_flight.take().ok_or(CompletionError::NoOperationInFlight)?;

        match completion {
            OutputCompletion::Succeeded => {
                self.retry = None;
                match operation {
                    OutputOperation::Initialize => {
                        self.output_state = OutputState::Active;
                    }
                    OutputOperation::Present => {
                        self.engine.on_presented(&self.frame);
                        self.present_required = false;
                    }
                    OutputOperation::Suspend => {
                        self.output_state = OutputState::Suspended;
                        self.next_render_ms = None;
                    }
                    OutputOperation::Resume => {
                        self.output_state = OutputState::Active;
                        self.dirty = true;
                        self.present_required = true;
                    }
                }
            }
            OutputCompletion::Failed { retry_in_ms } => {
                self.retry = Some(RetryState {
                    operation,
                    at_ms: retry_in_ms.and_then(|delay| now_ms.checked_add(delay.get() as u64)),
                });
                // Lifecycle state and successfully presented renderer history
                // intentionally remain unchanged.
                if operation == OutputOperation::Present {
                    self.present_required = true;
                }
            }
        }
        Ok(())
    }

    fn invalidate(&mut self, invalidation: Invalidation) {
        self.lighting_change_pending |= invalidation.state_changed();
        if invalidation.requires_render() {
            self.dirty = true;
        }
    }

    fn operation_ready(&mut self, operation: OutputOperation, now_ms: u64) -> bool {
        let Some(retry) = self.retry else {
            return true;
        };
        if retry.operation != operation {
            self.retry = None;
            return true;
        }
        match retry.at_ms {
            Some(deadline) if deadline <= now_ms => {
                self.retry = None;
                true
            }
            Some(_) | None => false,
        }
    }

    fn begin_operation(&mut self, operation: OutputOperation) -> ServiceAction<'_, E::Frame> {
        self.in_flight = Some(operation);
        match operation {
            OutputOperation::Initialize => ServiceAction::Initialize,
            OutputOperation::Present => ServiceAction::Present(&self.frame),
            OutputOperation::Suspend => ServiceAction::Suspend,
            OutputOperation::Resume => ServiceAction::Resume,
        }
    }

    fn wait_deadline(&self) -> Option<u64> {
        earliest(self.next_render_ms, self.retry.and_then(|retry| retry.at_ms))
    }
}

const fn earliest(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(if left < right { left } else { right }),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    struct Snapshot {
        value: u8,
        indicator: bool,
    }

    #[derive(Debug)]
    struct Provider {
        current: Snapshot,
    }

    impl Provider {
        fn new(value: u8) -> Self {
            Self {
                current: Snapshot {
                    value,
                    indicator: false,
                },
            }
        }
    }

    impl SnapshotProvider for Provider {
        type Snapshot = Snapshot;

        fn snapshot(&self) -> Self::Snapshot {
            // The production trait deliberately takes `&self`; use interior
            // mutability only when read accounting matters. Most tests infer
            // reads through the engine's recorded snapshots instead.
            self.current
        }
    }

    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    struct BoardFrame {
        rgb: [u8; 2],
        indicator: bool,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Input {
        Add(u8),
        Ignore,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Command {
        SetOffset(u8),
        Read,
        Fail,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum EngineError {
        Requested,
        Render,
    }

    struct Engine {
        offset: u8,
        committed: Option<BoardFrame>,
        render_calls: usize,
        presented_calls: usize,
        snapshots: [Option<Snapshot>; 8],
        snapshot_count: usize,
        next_wake: Option<NonZeroU32>,
        fail_render_once: bool,
        state_change_on_render: bool,
    }

    impl Engine {
        fn new() -> Self {
            Self {
                offset: 0,
                committed: None,
                render_calls: 0,
                presented_calls: 0,
                snapshots: [None; 8],
                snapshot_count: 0,
                next_wake: None,
                fail_render_once: false,
                state_change_on_render: false,
            }
        }

        fn record(&mut self, snapshot: Snapshot) {
            if let Some(slot) = self.snapshots.get_mut(self.snapshot_count) {
                *slot = Some(snapshot);
            }
            self.snapshot_count += 1;
        }
    }

    impl LightingEngine<Snapshot> for Engine {
        type Frame = BoardFrame;
        type Input = Input;
        type Command = Command;
        type Reply = u8;
        type Error = EngineError;

        fn on_input(&mut self, input: Self::Input, snapshot: &Snapshot) -> Result<Invalidation, Self::Error> {
            self.record(*snapshot);
            match input {
                Input::Add(value) => {
                    self.offset = self.offset.wrapping_add(value);
                    Ok(Invalidation::Render)
                }
                Input::Ignore => Ok(Invalidation::None),
            }
        }

        fn handle_command(
            &mut self,
            _now_ms: u64,
            command: Self::Command,
            snapshot: &Snapshot,
        ) -> Result<CommandResult<Self::Reply>, Self::Error> {
            self.record(*snapshot);
            match command {
                Command::SetOffset(value) => {
                    let previous = self.offset;
                    self.offset = value;
                    Ok(CommandResult::render(previous))
                }
                Command::Read => Ok(CommandResult::unchanged(self.offset)),
                Command::Fail => Err(EngineError::Requested),
            }
        }

        fn render(
            &mut self,
            input: RenderInput<'_, Snapshot>,
            frame: &mut Self::Frame,
        ) -> Result<RenderOutcome, Self::Error> {
            self.record(*input.snapshot);
            self.render_calls += 1;
            if core::mem::take(&mut self.fail_render_once) {
                return Err(EngineError::Render);
            }
            let value = input.snapshot.value.wrapping_add(self.offset);
            frame.rgb = [value, value.wrapping_add(1)];
            frame.indicator = input.snapshot.indicator;
            Ok(RenderOutcome {
                changed: self.committed.as_ref() != Some(frame),
                state_changed: core::mem::take(&mut self.state_change_on_render),
                next_wake_in_ms: self.next_wake,
            })
        }

        fn on_presented(&mut self, frame: &Self::Frame) {
            self.committed = Some(frame.clone());
            self.presented_calls += 1;
        }
    }

    type Service = LightingService<Provider, Engine>;

    fn service(value: u8) -> Service {
        LightingService::new(Provider::new(value), Engine::new(), BoardFrame::default())
    }

    fn complete(service: &mut Service, now_ms: u64) {
        service.complete_output(now_ms, OutputCompletion::Succeeded).unwrap();
    }

    fn initialize(service: &mut Service, now_ms: u64) {
        assert_eq!(service.next_action(now_ms), Ok(ServiceAction::Initialize));
        complete(service, now_ms);
    }

    fn initial_present(service: &mut Service, now_ms: u64) {
        initialize(service, now_ms);
        let expected = service.provider.current.value;
        match service.next_action(now_ms).unwrap() {
            ServiceAction::Present(frame) => {
                assert_eq!(frame.rgb, [expected, expected + 1]);
            }
            other => panic!("expected presentation, got {other:?}"),
        }
        complete(service, now_ms);
    }

    #[test]
    fn initial_render_uses_latest_authoritative_snapshot_and_whole_frame() {
        let mut service = service(1);
        assert_eq!(service.next_action(0), Ok(ServiceAction::Initialize));
        complete(&mut service, 0);

        // No state event is required: the render queries authoritative state.
        service.provider.current = Snapshot {
            value: 7,
            indicator: true,
        };
        match service.next_action(1).unwrap() {
            ServiceAction::Present(frame) => {
                assert_eq!(frame.rgb, [7, 8]);
                assert!(frame.indicator);
            }
            other => panic!("expected presentation, got {other:?}"),
        }
        complete(&mut service, 1);
        assert_eq!(service.engine.presented_calls, 1);
        assert_eq!(service.engine.committed, Some(service.frame.clone()));
    }

    #[test]
    fn successful_static_frame_is_not_rendered_or_presented_twice() {
        let mut service = service(3);
        initial_present(&mut service, 0);

        assert_eq!(service.next_action(100), Ok(ServiceAction::Wait { next_wake_ms: None }));
        assert_eq!(service.engine.render_calls, 1);
        assert_eq!(service.engine.presented_calls, 1);

        service.on_input(Input::Ignore).unwrap();
        assert!(!service.is_dirty());
        assert_eq!(service.next_action(101), Ok(ServiceAction::Wait { next_wake_ms: None }));
    }

    #[test]
    fn input_and_command_use_fresh_snapshots_and_serialize_mutation() {
        let mut service = service(2);
        service.provider.current.value = 4;
        service.on_input(Input::Add(1)).unwrap();
        service.provider.current.value = 6;
        assert_eq!(service.handle_command(10, Command::SetOffset(9)), Ok(1));
        assert_eq!(service.handle_command(11, Command::Read), Ok(9));
        assert_eq!(service.handle_command(12, Command::Fail), Err(EngineError::Requested));

        assert_eq!(service.engine.snapshots[0].unwrap().value, 4);
        assert_eq!(service.engine.snapshots[1].unwrap().value, 6);
        assert_eq!(service.engine.snapshots[2].unwrap().value, 6);
        assert_eq!(service.engine.snapshots[3].unwrap().value, 6);
        assert!(service.is_dirty());
    }

    #[test]
    fn authoritative_change_is_coalesced_and_render_requests_are_not_changes() {
        let mut service = service(2);
        assert!(!service.lighting_change_pending());

        service.request_render();
        assert!(!service.lighting_change_pending());
        service.on_input(Input::Ignore).unwrap();
        assert!(!service.lighting_change_pending());

        service.on_input(Input::Add(1)).unwrap();
        assert!(service.lighting_change_pending());
        assert_eq!(service.handle_command(1, Command::SetOffset(9)), Ok(1));
        assert!(service.take_lighting_change());
        assert!(!service.take_lighting_change());

        assert_eq!(service.handle_command(2, Command::Read), Ok(9));
        assert_eq!(service.handle_command(3, Command::Fail), Err(EngineError::Requested));
        assert!(!service.lighting_change_pending());
    }

    #[test]
    fn render_owned_state_change_is_reported_once() {
        let mut service = service(2);
        initialize(&mut service, 0);
        service.engine.state_change_on_render = true;
        assert!(matches!(service.next_action(0), Ok(ServiceAction::Present(_))));
        assert!(service.take_lighting_change());
        assert!(!service.take_lighting_change());
    }

    #[test]
    fn failed_present_retries_at_exact_deadline_without_recommitting_or_rerendering() {
        let mut service = service(5);
        initialize(&mut service, 0);
        assert!(matches!(service.next_action(0), Ok(ServiceAction::Present(_))));
        service
            .complete_output(
                0,
                OutputCompletion::Failed {
                    retry_in_ms: NonZeroU32::new(10),
                },
            )
            .unwrap();

        assert_eq!(service.engine.presented_calls, 0);
        assert_eq!(service.engine.render_calls, 1);
        assert_eq!(
            service.next_action(9),
            Ok(ServiceAction::Wait { next_wake_ms: Some(10) })
        );
        assert!(matches!(service.next_action(10), Ok(ServiceAction::Present(_))));
        assert_eq!(service.engine.render_calls, 1);
        complete(&mut service, 10);
        assert_eq!(service.engine.presented_calls, 1);
    }

    #[test]
    fn unscheduled_failure_stays_blocked_until_explicit_retry_but_can_rerender_latest_state() {
        let mut service = service(1);
        initialize(&mut service, 0);
        assert!(matches!(service.next_action(0), Ok(ServiceAction::Present(_))));
        service
            .complete_output(0, OutputCompletion::Failed { retry_in_ms: None })
            .unwrap();

        service.provider.current.value = 8;
        service.request_render();
        assert_eq!(service.next_action(50), Ok(ServiceAction::Wait { next_wake_ms: None }));
        assert_eq!(service.frame.rgb, [8, 9]);
        assert_eq!(service.engine.render_calls, 2);

        service.retry_output_now().unwrap();
        match service.next_action(50).unwrap() {
            ServiceAction::Present(frame) => assert_eq!(frame.rgb, [8, 9]),
            other => panic!("expected presentation, got {other:?}"),
        }
    }

    #[test]
    fn engine_deadline_renders_exactly_when_due_and_reschedules_relatively() {
        let mut service = service(1);
        service.engine.next_wake = NonZeroU32::new(25);
        initial_present(&mut service, 100);
        assert_eq!(service.next_render_ms(), Some(125));
        assert_eq!(
            service.next_action(124),
            Ok(ServiceAction::Wait {
                next_wake_ms: Some(125)
            })
        );

        // The frame is unchanged at the deadline, so there is no write, but
        // the next visible-change deadline is still advanced.
        assert_eq!(
            service.next_action(125),
            Ok(ServiceAction::Wait {
                next_wake_ms: Some(150)
            })
        );
        assert_eq!(service.engine.render_calls, 2);
    }

    #[test]
    fn a_new_event_before_the_old_deadline_replaces_the_schedule() {
        let mut service = service(1);
        service.engine.next_wake = NonZeroU32::new(20);
        initial_present(&mut service, 10);
        assert_eq!(service.next_render_ms(), Some(30));

        service.engine.next_wake = NonZeroU32::new(50);
        service.on_input(Input::Add(1)).unwrap();
        assert!(matches!(service.next_action(15), Ok(ServiceAction::Present(_))));
        assert_eq!(service.next_render_ms(), Some(65));
    }

    #[test]
    fn suspend_disarms_deadline_and_resume_forces_fresh_render_and_present() {
        let mut service = service(4);
        service.engine.next_wake = NonZeroU32::new(10);
        initial_present(&mut service, 0);

        service.set_power(PowerState::Suspended).unwrap();
        assert_eq!(service.next_render_ms(), None);
        assert_eq!(service.next_action(1), Ok(ServiceAction::Suspend));
        complete(&mut service, 1);
        assert_eq!(service.output_state(), OutputState::Suspended);

        service.provider.current.value = 9;
        service.on_input(Input::Add(0)).unwrap();
        assert_eq!(service.next_action(100), Ok(ServiceAction::Wait { next_wake_ms: None }));

        service.set_power(PowerState::Active).unwrap();
        assert_eq!(service.next_action(101), Ok(ServiceAction::Resume));
        complete(&mut service, 101);
        match service.next_action(101).unwrap() {
            ServiceAction::Present(frame) => assert_eq!(frame.rgb, [9, 10]),
            other => panic!("expected presentation, got {other:?}"),
        }
    }

    #[test]
    fn lifecycle_failure_retries_without_changing_successful_state() {
        let mut service = service(0);
        assert_eq!(service.next_action(0), Ok(ServiceAction::Initialize));
        service
            .complete_output(
                0,
                OutputCompletion::Failed {
                    retry_in_ms: NonZeroU32::new(5),
                },
            )
            .unwrap();
        assert_eq!(service.output_state(), OutputState::Uninitialized);
        assert_eq!(
            service.next_action(4),
            Ok(ServiceAction::Wait { next_wake_ms: Some(5) })
        );
        assert_eq!(service.next_action(5), Ok(ServiceAction::Initialize));
        complete(&mut service, 5);
        assert_eq!(service.output_state(), OutputState::Active);
    }

    #[test]
    fn output_operation_must_complete_before_any_other_mutating_transition() {
        let mut service = service(0);
        assert_eq!(service.next_action(0), Ok(ServiceAction::Initialize));
        assert_eq!(
            service.next_action(0),
            Err(ServiceError::OperationInFlight(OutputOperation::Initialize))
        );
        assert_eq!(
            service.set_power(PowerState::Suspended),
            Err(CompletionError::OperationInFlight(OutputOperation::Initialize))
        );
        assert_eq!(
            service.retry_output_now(),
            Err(CompletionError::OperationInFlight(OutputOperation::Initialize))
        );
        complete(&mut service, 0);
        assert_eq!(
            service.complete_output(0, OutputCompletion::Succeeded),
            Err(CompletionError::NoOperationInFlight)
        );
    }

    #[test]
    fn render_failure_leaves_service_dirty_for_a_deterministic_retry() {
        let mut service = service(2);
        initialize(&mut service, 0);
        service.engine.fail_render_once = true;
        assert_eq!(service.next_action(0), Err(ServiceError::Engine(EngineError::Render)));
        assert!(service.is_dirty());
        assert!(matches!(service.next_action(0), Ok(ServiceAction::Present(_))));
        assert_eq!(service.engine.render_calls, 2);
    }

    #[test]
    fn deadline_overflow_does_not_create_a_busy_loop() {
        let mut service = service(0);
        service.engine.next_wake = NonZeroU32::new(2);
        initialize(&mut service, u64::MAX - 1);
        assert!(matches!(
            service.next_action(u64::MAX - 1),
            Ok(ServiceAction::Present(_))
        ));
        assert_eq!(service.next_render_ms(), None);
    }

    #[test]
    fn retry_deadline_overflow_requires_explicit_retry() {
        let mut service = service(0);
        assert_eq!(service.next_action(u64::MAX), Ok(ServiceAction::Initialize));
        service
            .complete_output(
                u64::MAX,
                OutputCompletion::Failed {
                    retry_in_ms: NonZeroU32::new(1),
                },
            )
            .unwrap();
        assert_eq!(
            service.next_action(u64::MAX),
            Ok(ServiceAction::Wait { next_wake_ms: None })
        );
        service.retry_output_now().unwrap();
        assert_eq!(service.next_action(u64::MAX), Ok(ServiceAction::Initialize));
    }

    #[test]
    fn power_change_supersedes_a_blocked_presentation_retry() {
        let mut service = service(0);
        initialize(&mut service, 0);
        assert!(matches!(service.next_action(0), Ok(ServiceAction::Present(_))));
        service
            .complete_output(0, OutputCompletion::Failed { retry_in_ms: None })
            .unwrap();
        service.set_power(PowerState::Suspended).unwrap();
        assert_eq!(service.next_action(1), Ok(ServiceAction::Suspend));
    }
}
