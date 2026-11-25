//! An HCI controller for tests. Every command succeeds at once, and a test
//! injects the events a host's link would cause.

use core::convert::Infallible;
use std::sync::{Arc, Mutex};

use bt_hci::cmd::le::{LeReadBufferSize, LeReadNumberOfSupportedAdvSets, LeReadPhy};
use bt_hci::cmd::link_control::Disconnect;
use bt_hci::cmd::{self, AsyncCmd, Cmd, Opcode, SyncCmd};
use bt_hci::controller::{Controller, ControllerCmdAsync, ControllerCmdSync};
use bt_hci::data::{AclPacket, IsoPacket, SyncPacket};
use bt_hci::{ControllerToHostPacket, FromHciBytes, PacketKind, WriteHci};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use trouble_host::prelude::{Uuid, declarations, descriptors};

#[derive(Clone, Default)]
pub(crate) struct FakeController {
    state: Arc<State>,
    /// Leaves every command pending, like a controller that never answers.
    stalled: bool,
}

struct State {
    events: Channel<CriticalSectionRawMutex, (PacketKind, Vec<u8>), 16>,
    /// The opcode and parameters of every command, in order.
    commands: Mutex<Vec<(Opcode, Vec<u8>)>>,
    /// The connection handle and payload of every ACL packet sent, in order.
    acl: Mutex<Vec<(u16, Vec<u8>)>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            events: Channel::new(),
            commands: Mutex::default(),
            acl: Mutex::default(),
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

    /// The value of every notification of attribute `attr` sent on `handle`.
    pub(crate) fn notifications(&self, handle: u16, attr: u16) -> Vec<Vec<u8>> {
        let [lo, hi] = attr.to_le_bytes();
        let acl = self.state.acl.lock().unwrap();
        acl.iter()
            .filter(|(link, _)| *link == handle)
            // L2CAP length and the ATT channel, then a Handle Value Notification.
            .filter_map(|(_, data)| match data.as_slice() {
                [_, _, 4, 0, 0x1B, l, h, value @ ..] if [*l, *h] == [lo, hi] => Some(value.to_vec()),
                _ => None,
            })
            .collect()
    }

    /// Put a client ATT request on the connection.
    pub(crate) fn att_request(&self, handle: u16, request: &[u8]) {
        let len = u16::try_from(request.len()).unwrap();
        let mut packet = Vec::new();
        packet.extend_from_slice(&(handle | 0x2000).to_le_bytes());
        packet.extend_from_slice(&(len + 4).to_le_bytes());
        packet.extend_from_slice(&len.to_le_bytes());
        packet.extend_from_slice(&[4, 0]);
        packet.extend_from_slice(request);
        self.state.events.try_send((PacketKind::AclData, packet)).unwrap();
    }

    pub(crate) async fn att_exchange(&self, link: u16, request: &[u8], response_opcode: u8) -> Vec<u8> {
        let start = self.state.acl.lock().unwrap().len();
        self.att_request(link, request);
        embassy_time::Timer::after_millis(1).await;
        self.state.acl.lock().unwrap()[start..]
            .iter()
            .find_map(|(handle, frame)| match frame.as_slice() {
                [_, _, 4, 0, op, payload @ ..] if *handle == link && *op == response_opcode => Some(payload.to_vec()),
                _ => None,
            })
            .expect("missing ATT response to this request")
    }

    pub(super) async fn read_by_type(&self, link: u16, start_handle: u16, uuid: Uuid) -> Vec<u8> {
        let [lo, hi] = start_handle.to_le_bytes();
        let mut request = vec![0x08, lo, hi, 0xff, 0xff];
        request.extend_from_slice(uuid.as_raw());
        self.att_exchange(link, &request, 0x09).await
    }

    pub(crate) async fn read_u8_by_type(&self, link: u16, start: u16, uuid: Uuid) -> Vec<(u16, u8)> {
        let reply = self.read_by_type(link, start, uuid).await;
        let [3, entries @ ..] = reply.as_slice() else {
            panic!("unexpected Read By Type response for a u8: {reply:?}")
        };
        let (levels, trailing) = entries.as_chunks::<3>();
        assert!(trailing.is_empty() && !levels.is_empty());
        levels
            .iter()
            .map(|&[lo, hi, level]| (u16::from_le_bytes([lo, hi]), level))
            .collect()
    }

    pub(crate) async fn set_notifications(&self, link: u16, value_handle: u16, enabled: bool) {
        let reply = self
            .read_by_type(
                link,
                value_handle + 1,
                descriptors::CLIENT_CHARACTERISTIC_CONFIGURATION.into(),
            )
            .await;
        let [4, lo, hi, _, _, ..] = reply[..] else {
            panic!("unexpected CCCD response: {reply:?}")
        };
        self.att_exchange(link, &[0x12, lo, hi, u8::from(enabled), 0], 0x13)
            .await;
    }

    pub(crate) async fn characteristic_handle(&self, link: u16, uuid: u16) -> u16 {
        let mut start = 1;
        loop {
            let reply = self
                .read_by_type(link, start, declarations::CHARACTERISTIC.into())
                .await;
            for entry in reply[1..].chunks_exact(usize::from(reply[0])) {
                if entry[5..] == uuid.to_le_bytes() {
                    return u16::from_le_bytes([entry[3], entry[4]]);
                }
                start = u16::from_le_bytes([entry[0], entry[1]]) + 1;
            }
        }
    }

    /// The minimum interval of every advertising setup, in 0.625 ms units.
    pub(crate) fn advertising_intervals(&self) -> Vec<u32> {
        #[cfg(feature = "ext_adv")]
        let intervals = self
            .sent::<bt_hci::cmd::le::LeSetExtAdvParams>()
            .iter()
            .map(|params| u32::from_le_bytes([params[3], params[4], params[5], 0]))
            .collect();
        #[cfg(not(feature = "ext_adv"))]
        let intervals = self
            .sent::<bt_hci::cmd::le::LeSetAdvParams>()
            .iter()
            .map(|params| u16::from_le_bytes([params[0], params[1]]).into())
            .collect();
        intervals
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
        self.state
            .events
            .try_send((PacketKind::Event, event))
            .expect("fake HCI event queue is full");
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
        let handle = packet.handle().raw();
        self.state.acl.lock().unwrap().push((handle, packet.data().to_vec()));
        let [lo, hi] = handle.to_le_bytes();
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
        let (kind, event) = self.state.events.receive().await;
        buf[..event.len()].copy_from_slice(&event);
        Ok(
            ControllerToHostPacket::from_hci_bytes_with_kind(kind, &buf[..event.len()])
                .expect("malformed fake HCI event")
                .0,
        )
    }
}

impl<C: SyncCmd> ControllerCmdSync<C> for FakeController {
    async fn exec(&self, cmd: &C) -> Result<C::Return, cmd::Error<Infallible>> {
        let params = self.record(cmd).await;
        // Zeros answer every command, except where the host needs a usable value.
        let mut ret = [0u8; 256];
        if C::OPCODE == LeReadBufferSize::OPCODE {
            // 251-byte ACL packets, 3 buffers.
            ret[..3].copy_from_slice(&[251, 0, 3]);
        } else if C::OPCODE == LeReadNumberOfSupportedAdvSets::OPCODE {
            ret[0] = 1;
        } else if C::OPCODE == LeReadPhy::OPCODE {
            // Echo the requested handle, then LE 1M on both directions.
            ret[..2].copy_from_slice(&params[..2]);
            ret[2..4].copy_from_slice(&[1, 1]);
        }
        Ok(C::Return::from_hci_bytes(&ret)
            .unwrap_or_else(|_| panic!("fake HCI cannot answer command {:?}", C::OPCODE))
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
