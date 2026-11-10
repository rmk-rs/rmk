//! An HCI controller for tests. Every command succeeds at once, and a test
//! injects the events a host's link would cause.

use core::convert::Infallible;
use std::sync::{Arc, Mutex};

use bt_hci::cmd::le::{LeReadBufferSize, LeReadNumberOfSupportedAdvSets};
use bt_hci::cmd::link_control::Disconnect;
use bt_hci::cmd::{self, AsyncCmd, Cmd, Opcode, SyncCmd};
use bt_hci::controller::{Controller, ControllerCmdAsync, ControllerCmdSync};
use bt_hci::data::{AclPacket, IsoPacket, SyncPacket};
use bt_hci::{ControllerToHostPacket, FromHciBytes, PacketKind, WriteHci};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;

#[derive(Clone, Default)]
pub(crate) struct FakeController {
    state: Arc<State>,
    /// Leaves every command pending, like a controller that never answers.
    stalled: bool,
}

struct State {
    events: Channel<CriticalSectionRawMutex, Vec<u8>, 16>,
    /// The opcode and parameters of every command, in order.
    commands: Mutex<Vec<(Opcode, Vec<u8>)>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            events: Channel::new(),
            commands: Mutex::default(),
        }
    }
}

impl FakeController {
    pub(crate) fn stalled() -> Self {
        Self {
            stalled: true,
            ..Default::default()
        }
    }

    /// The parameters of every `C` command sent so far.
    pub(crate) fn sent<C: Cmd>(&self) -> Vec<Vec<u8>> {
        let commands = self.state.commands.lock().unwrap();
        commands
            .iter()
            .filter(|(opcode, _)| *opcode == C::OPCODE)
            .map(|(_, params)| params.clone())
            .collect()
    }

    /// How many times advertising has been enabled.
    pub(crate) fn advertising_starts(&self) -> usize {
        #[cfg(feature = "ext_adv")]
        let enables = self.sent::<bt_hci::cmd::le::LeSetExtAdvEnable<'static>>();
        #[cfg(not(feature = "ext_adv"))]
        let enables = self.sent::<bt_hci::cmd::le::LeSetAdvEnable>();
        enables.iter().filter(|params| params[0] == 1).count()
    }

    /// A host at random address `peer` connects to the advertisement on `handle`.
    pub(crate) fn connect(&self, handle: u16, peer: [u8; 6]) {
        let [lo, hi] = handle.to_le_bytes();
        // LE Enhanced Connection Complete: success, we are the peripheral, random peer address.
        let mut event = vec![0x3E, 31, 0x0A, 0, lo, hi, 1, 1];
        event.extend_from_slice(&peer);
        // No private addresses; 15 ms interval, no latency, 4 s supervision timeout.
        event.extend_from_slice(&[0; 12]);
        event.extend_from_slice(&[12, 0, 0, 0, 0x90, 0x01, 0]);
        self.push(event);
        // LE Advertising Set Terminated: the only advertising set made this connection.
        #[cfg(feature = "ext_adv")]
        self.push(vec![0x3E, 6, 0x12, 0, 0, lo, hi, 0]);
    }

    /// The host on `handle` resumes encryption with the stored bond.
    pub(crate) fn encrypt(&self, handle: u16) {
        let [lo, hi] = handle.to_le_bytes();
        self.push(vec![0x08, 4, 0, lo, hi, 1]);
    }

    /// The host on `handle` leaves.
    pub(crate) fn drop_link(&self, handle: u16) {
        let [lo, hi] = handle.to_le_bytes();
        self.push(vec![0x05, 4, 0, lo, hi, 0x13]);
    }

    fn push(&self, event: Vec<u8>) {
        self.state.events.try_send(event).expect("fake HCI event queue is full");
    }

    async fn record<C: Cmd>(&self, cmd: &C) -> Vec<u8> {
        if self.stalled {
            core::future::pending::<()>().await;
        }
        let mut params = vec![0; cmd.params().size()];
        cmd.params().write_hci(params.as_mut_slice()).unwrap();
        self.state.commands.lock().unwrap().push((C::OPCODE, params.clone()));
        params
    }
}

impl embedded_io_async::ErrorType for FakeController {
    type Error = Infallible;
}

impl Controller for FakeController {
    type Buffer<'a> = [u8; 259];

    fn alloc_buf(&self) -> Result<Self::Buffer<'_>, Infallible> {
        Ok([0; 259])
    }

    async fn write_acl_data(&self, packet: &AclPacket<'_>) -> Result<(), Infallible> {
        // Number Of Completed Packets returns the buffer credit at once.
        let [lo, hi] = packet.handle().raw().to_le_bytes();
        self.push(vec![0x13, 5, 1, lo, hi, 1, 0]);
        Ok(())
    }

    async fn write_sync_data(&self, _: &SyncPacket<'_>) -> Result<(), Infallible> {
        Ok(())
    }

    async fn write_iso_data(&self, _: &IsoPacket<'_>) -> Result<(), Infallible> {
        Ok(())
    }

    async fn read<'a>(&self, buf: &'a mut Self::Buffer<'_>) -> Result<ControllerToHostPacket<'a>, Infallible> {
        let event = self.state.events.receive().await;
        buf[..event.len()].copy_from_slice(&event);
        Ok(
            ControllerToHostPacket::from_hci_bytes_with_kind(PacketKind::Event, &buf[..event.len()])
                .expect("malformed fake HCI event")
                .0,
        )
    }
}

impl<C: SyncCmd> ControllerCmdSync<C> for FakeController {
    async fn exec(&self, cmd: &C) -> Result<C::Return, cmd::Error<Infallible>> {
        self.record(cmd).await;
        // Zeros answer every command, except where the host needs a usable value.
        let mut ret = [0u8; 256];
        if C::OPCODE == LeReadBufferSize::OPCODE {
            // 251-byte ACL packets, 3 buffers.
            ret[..3].copy_from_slice(&[251, 0, 3]);
        } else if C::OPCODE == LeReadNumberOfSupportedAdvSets::OPCODE {
            ret[0] = 1;
        }
        Ok(C::Return::from_hci_bytes(&ret)
            .expect("fake HCI cannot answer this command")
            .0)
    }
}

impl<C: AsyncCmd> ControllerCmdAsync<C> for FakeController {
    async fn exec(&self, cmd: &C) -> Result<(), cmd::Error<Infallible>> {
        let params = self.record(cmd).await;
        if C::OPCODE == Disconnect::OPCODE {
            // Disconnection Complete, terminated by the local host.
            self.push(vec![0x05, 4, 0, params[0], params[1], 0x16]);
        }
        Ok(())
    }
}
