//! RMK event, command, deadline, and output integration for lighting.

use core::cell::Cell;
use core::num::NonZeroU32;

use embassy_futures::select::{Either3, Either4, select3, select4};
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::{Instant, Timer};
use rmk_types::action::LightAction;

use super::service::{
    LightingEngine, LightingOutput, LightingService, OutputCompletion, OutputOperation, PowerState, ServiceAction,
    ServiceError, SnapshotProvider,
};
use crate::RawMutex;
use crate::core_traits::Runnable;
use crate::event::{
    ConnectionStatusChangeEvent, EventSubscriber, LayerChangeEvent, LedIndicatorEvent, LightingChangedEvent,
    SleepStateEvent, publish_event,
};
use crate::processor::Processor;

/// Reliable single-owner request/reply path used by Rynk, Vial, or a board
/// API. Concurrent callers are serialized; commands are never coalesced.
pub struct LightingMailbox<Command, Reply, Error, const CAPACITY: usize> {
    requests: Channel<RawMutex, MailboxRequest<Command>, CAPACITY>,
    response: Signal<RawMutex, MailboxResponse<Reply, Error>>,
    retry_output: Signal<RawMutex, ()>,
    snapshot_changed: Signal<RawMutex, ()>,
    caller: Mutex<RawMutex, ()>,
    next_id: BlockingMutex<RawMutex, Cell<u32>>,
}

struct MailboxRequest<Command> {
    id: u32,
    command: Command,
}

struct MailboxResponse<Reply, Error> {
    id: u32,
    result: Result<Reply, Error>,
}

impl<Command, Reply, Error, const CAPACITY: usize> LightingMailbox<Command, Reply, Error, CAPACITY> {
    pub const fn new() -> Self {
        Self {
            requests: Channel::new(),
            response: Signal::new(),
            retry_output: Signal::new(),
            snapshot_changed: Signal::new(),
            caller: Mutex::new(()),
            next_id: BlockingMutex::new(Cell::new(0)),
        }
    }

    /// Submit one command and wait for authoritative service readback.
    pub async fn request(&self, command: Command) -> Result<Reply, Error> {
        let _caller = self.caller.lock().await;
        let id = self.next_id.lock(|next| {
            let id = next.get();
            next.set(id.wrapping_add(1));
            id
        });
        self.requests.send(MailboxRequest { id, command }).await;
        loop {
            let response = self.response.wait().await;
            if response.id == id {
                return response.result;
            }
        }
    }

    /// Wake a processor whose output policy blocked automatic retries.
    pub fn retry_output(&self) {
        self.retry_output.signal(());
    }

    /// Coalescing invalidation for application-owned snapshot fields.
    ///
    /// Boards use this after changing context outside the standard RMK
    /// layer/indicator snapshots (for example battery or sensor state). The
    /// processor takes a fresh snapshot, rerenders, and notifies replicas.
    pub fn snapshot_changed(&self) {
        self.snapshot_changed.signal(());
    }

    /// Service side: receive the next command. Normally only
    /// [`LightingProcessor`] drives this; it is public for board-specific
    /// executors and deterministic tests.
    pub async fn receive_request(&self) -> (u32, Command) {
        let request = self.requests.receive().await;
        (request.id, request.command)
    }

    /// Service side: publish the reply for a previously received command.
    pub fn publish_reply(&self, id: u32, result: Result<Reply, Error>) {
        self.response.signal(MailboxResponse { id, result });
    }

    async fn receive(&self) -> MailboxRequest<Command> {
        self.requests.receive().await
    }

    fn reply(&self, id: u32, result: Result<Reply, Error>) {
        self.response.signal(MailboxResponse { id, result });
    }
}

impl<Command, Reply, Error, const CAPACITY: usize> Default for LightingMailbox<Command, Reply, Error, CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

/// Sole mutable runtime owner for one lighting engine and output.
///
/// State notifications invalidate a fresh authoritative snapshot. LightAction
/// edges and mailbox commands remain reliable and ordered. The loop arms only
/// the deadline returned by the service; static output has no timer.
#[::rmk::macros::processor(subscribe = [LayerChangeEvent, LedIndicatorEvent, SleepStateEvent, ConnectionStatusChangeEvent])]
#[::rmk::macros::runnable_generated]
pub struct LightingProcessor<'mailbox, P, E, O, const COMMAND_CAPACITY: usize>
where
    P: SnapshotProvider,
    E: LightingEngine<P::Snapshot>,
    E::Input: From<LightAction>,
    E::Command: Send,
    E::Reply: Send,
    E::Error: Send,
    O: LightingOutput<E::Frame>,
{
    service: LightingService<P, E>,
    output: O,
    mailbox: &'mailbox LightingMailbox<E::Command, E::Reply, E::Error, COMMAND_CAPACITY>,
    engine_retry: NonZeroU32,
}

impl<'mailbox, P, E, O, const COMMAND_CAPACITY: usize> LightingProcessor<'mailbox, P, E, O, COMMAND_CAPACITY>
where
    P: SnapshotProvider,
    E: LightingEngine<P::Snapshot>,
    E::Input: From<LightAction>,
    E::Command: Send,
    E::Reply: Send,
    E::Error: Send,
    O: LightingOutput<E::Frame>,
{
    pub fn new(
        service: LightingService<P, E>,
        output: O,
        mailbox: &'mailbox LightingMailbox<E::Command, E::Reply, E::Error, COMMAND_CAPACITY>,
    ) -> Self {
        Self {
            service,
            output,
            mailbox,
            engine_retry: NonZeroU32::new(10).unwrap(),
        }
    }

    pub fn with_engine_retry(mut self, delay: NonZeroU32) -> Self {
        self.engine_retry = delay;
        self
    }

    pub const fn service(&self) -> &LightingService<P, E> {
        &self.service
    }

    pub fn service_mut(&mut self) -> &mut LightingService<P, E> {
        &mut self.service
    }

    fn publish_pending_lighting_change(&mut self) {
        if self.service.take_lighting_change() {
            publish_event(LightingChangedEvent::new());
        }
    }

    /// Drive immediate lifecycle/render/present work and return the next
    /// absolute millisecond deadline.
    async fn drive_until_wait(&mut self) -> Option<u64> {
        loop {
            let now_ms = Instant::now().as_millis();
            let action = match self.service.next_action(now_ms) {
                Ok(action) => action,
                Err(ServiceError::Engine(_)) => {
                    return now_ms.checked_add(self.engine_retry.get() as u64);
                }
                Err(ServiceError::OperationInFlight(_)) => {
                    // Only this method begins and completes operations, so an
                    // in-flight action here is an internal invariant failure.
                    return now_ms.checked_add(self.engine_retry.get() as u64);
                }
            };

            match action {
                ServiceAction::Wait { next_wake_ms } => {
                    self.publish_pending_lighting_change();
                    return next_wake_ms;
                }
                ServiceAction::Initialize => {
                    let completion = match self.output.initialize().await {
                        Ok(()) => OutputCompletion::Succeeded,
                        Err(error) => OutputCompletion::Failed {
                            retry_in_ms: self.output.retry_after(OutputOperation::Initialize, &error),
                        },
                    };
                    let _ = self.service.complete_output(Instant::now().as_millis(), completion);
                }
                ServiceAction::Present(frame) => {
                    let completion = match self.output.present(frame).await {
                        Ok(()) => OutputCompletion::Succeeded,
                        Err(error) => OutputCompletion::Failed {
                            retry_in_ms: self.output.retry_after(OutputOperation::Present, &error),
                        },
                    };
                    let _ = self.service.complete_output(Instant::now().as_millis(), completion);
                }
                ServiceAction::Suspend => {
                    let completion = match self.output.suspend().await {
                        Ok(()) => OutputCompletion::Succeeded,
                        Err(error) => OutputCompletion::Failed {
                            retry_in_ms: self.output.retry_after(OutputOperation::Suspend, &error),
                        },
                    };
                    let _ = self.service.complete_output(Instant::now().as_millis(), completion);
                }
                ServiceAction::Resume => {
                    let completion = match self.output.resume().await {
                        Ok(()) => OutputCompletion::Succeeded,
                        Err(error) => OutputCompletion::Failed {
                            retry_in_ms: self.output.retry_after(OutputOperation::Resume, &error),
                        },
                    };
                    let _ = self.service.complete_output(Instant::now().as_millis(), completion);
                }
            }
            self.publish_pending_lighting_change();
        }
    }

    async fn handle_mailbox_command(&mut self, request: MailboxRequest<E::Command>) {
        let response = self.service.handle_command(Instant::now().as_millis(), request.command);
        self.publish_pending_lighting_change();
        self.mailbox.reply(request.id, response);
    }

    async fn on_layer_change_event(&mut self, _event: LayerChangeEvent) {
        self.service.request_render();
    }

    async fn on_led_indicator_event(&mut self, _event: LedIndicatorEvent) {
        self.service.request_render();
    }

    async fn on_connection_status_change_event(&mut self, _event: ConnectionStatusChangeEvent) {
        self.service.request_render();
    }

    async fn on_sleep_state_event(&mut self, event: SleepStateEvent) {
        let target = if event.0 {
            PowerState::Suspended
        } else {
            PowerState::Active
        };
        let _ = self.service.set_power(target);
    }
}

impl<P, E, O, const COMMAND_CAPACITY: usize> Runnable for LightingProcessor<'_, P, E, O, COMMAND_CAPACITY>
where
    P: SnapshotProvider,
    E: LightingEngine<P::Snapshot>,
    E::Input: From<LightAction>,
    E::Command: Send,
    E::Reply: Send,
    E::Error: Send,
    O: LightingOutput<E::Frame>,
{
    async fn run(&mut self) -> ! {
        let mut subscriber = <Self as Processor>::subscriber();
        if crate::state::current_sleeping() {
            let _ = self.service.set_power(PowerState::Suspended);
        }

        loop {
            let deadline = self.drive_until_wait().await;
            match deadline {
                Some(deadline) => {
                    match select4(
                        Timer::at(Instant::from_millis(deadline)),
                        subscriber.next_event(),
                        self.mailbox.receive(),
                        select3(
                            self.mailbox.retry_output.wait(),
                            super::next_light_action(),
                            self.mailbox.snapshot_changed.wait(),
                        ),
                    )
                    .await
                    {
                        Either4::First(_) => {}
                        Either4::Second(event) => self.process(event).await,
                        Either4::Third(request) => self.handle_mailbox_command(request).await,
                        Either4::Fourth(Either3::First(())) => {
                            let _ = self.service.retry_output_now();
                        }
                        Either4::Fourth(Either3::Second(action)) => {
                            let _ = self.service.on_input(E::Input::from(action));
                            self.publish_pending_lighting_change();
                        }
                        Either4::Fourth(Either3::Third(())) => {
                            self.service.request_render();
                            publish_event(LightingChangedEvent::new());
                        }
                    }
                }
                None => {
                    match select3(
                        subscriber.next_event(),
                        self.mailbox.receive(),
                        select3(
                            self.mailbox.retry_output.wait(),
                            super::next_light_action(),
                            self.mailbox.snapshot_changed.wait(),
                        ),
                    )
                    .await
                    {
                        Either3::First(event) => self.process(event).await,
                        Either3::Second(request) => self.handle_mailbox_command(request).await,
                        Either3::Third(Either3::First(())) => {
                            let _ = self.service.retry_output_now();
                        }
                        Either3::Third(Either3::Second(action)) => {
                            let _ = self.service.on_input(E::Input::from(action));
                            self.publish_pending_lighting_change();
                        }
                        Either3::Third(Either3::Third(())) => {
                            self.service.request_render();
                            publish_event(LightingChangedEvent::new());
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use embassy_futures::join::join;

    use super::LightingMailbox;
    use crate::test_support::test_block_on as block_on;

    #[test]
    fn mailbox_delivers_ordered_request_and_authoritative_reply() {
        let mailbox = LightingMailbox::<u8, u16, (), 2>::new();
        let (reply, ()) = block_on(join(mailbox.request(7), async {
            let request = mailbox.receive().await;
            assert_eq!(request.command, 7);
            mailbox.reply(request.id, Ok(42));
        }));
        assert_eq!(reply, Ok(42));
    }

    #[test]
    fn mailbox_propagates_command_errors() {
        let mailbox = LightingMailbox::<(), (), u8, 1>::new();
        let (reply, ()) = block_on(join(mailbox.request(()), async {
            let request = mailbox.receive().await;
            mailbox.reply(request.id, Err(9));
        }));
        assert_eq!(reply, Err(9));
    }

    #[test]
    fn snapshot_change_notifications_coalesce() {
        let mailbox = LightingMailbox::<(), (), (), 1>::new();
        mailbox.snapshot_changed();
        mailbox.snapshot_changed();

        assert_eq!(mailbox.snapshot_changed.try_take(), Some(()));
        assert_eq!(mailbox.snapshot_changed.try_take(), None);
    }

    #[test]
    fn cancelled_request_reply_cannot_poison_the_next_caller() {
        use embassy_futures::select::{Either, select};
        use embassy_futures::yield_now;

        let mailbox = LightingMailbox::<u8, u16, (), 2>::new();
        let cancelled = block_on(select(mailbox.request(1), async {
            yield_now().await;
        }));
        assert!(matches!(cancelled, Either::Second(())));

        let (reply, ()) = block_on(join(mailbox.request(2), async {
            let abandoned = mailbox.receive().await;
            assert_eq!(abandoned.command, 1);
            mailbox.reply(abandoned.id, Ok(11));
            let current = mailbox.receive().await;
            assert_eq!(current.command, 2);
            mailbox.reply(current.id, Ok(22));
        }));
        assert_eq!(reply, Ok(22));
    }
}
