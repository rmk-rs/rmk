//! Lighting endpoints across the full Rynk loopback.
//!
//! A scenario cannot reach these. The topology a lighting board advertises is a
//! compile-time fixture rather than a keymap, and the stateful endpoints answer
//! from a lighting engine that has to be driven concurrently with the request —
//! two things a timeline has no vocabulary for. So these cases build a
//! `RynkService` directly and play the host over an in-memory duplex, which is
//! the same byte stream the USB and BLE adapters hand `run_session`.

use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::pipe::Pipe;
use rmk::config::{BehaviorConfig, PositionalConfig, RmkConfig};
use rmk::event::publish_event;
use rmk::host::{HostService as RynkService, RynkLightingController, RynkLightingDescriptor, RynkLightingMailbox};
use rmk::keymap::{KeyMap, KeymapData};
use rmk::lighting::{LedId, LedMetadata, LightingRouting, LightingTopology, MatrixSize, ZoneSpan};
use rmk::physical_layout::{Coordinate, KeyPosition, KeySize, PhysicalKey, PhysicalLayout, Point3, Rotation};
use rmk::test_support::test_block_on;
use rmk::types::action::KeyAction;
use rmk_types::constants::RYNK_BUFFER_SIZE;
use rmk_types::protocol::rynk::{
    Cmd, Deframer, DeviceCapabilities, LightingChanged, LightingError, LightingKeysPageResult, LightingMatrixPosition,
    LightingPageRequest, RYNK_HEADER_SIZE, RynkError, RynkHeader, encode_frame,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

/// One direction of the link, sized so any single legal frame fits and the
/// writer never blocks on a reader that has not been polled yet.
type Link = Pipe<NoopRawMutex, RYNK_BUFFER_SIZE>;

/// Host end of the duplex. Encodes and decodes exactly as a real host does, so
/// a drift between the two ends fails here rather than on a keyboard.
struct Host<'p> {
    rx: &'p Link,
    tx: &'p Link,
    df: Deframer,
    rxbuf: [u8; RYNK_BUFFER_SIZE],
}

impl Host<'_> {
    /// Round-trip one request and decode its `Result<Resp, RynkError>` envelope.
    /// `seq` correlates the reply; topic pushes use seq 0, so request seqs must
    /// be non-zero to stay unambiguous.
    async fn request<Req: Serialize, Resp: DeserializeOwned>(
        &mut self,
        cmd: Cmd,
        seq: u8,
        req: &Req,
    ) -> Result<Resp, RynkError> {
        assert_ne!(seq, 0, "request seqs must not alias topic frames");
        let mut buf = [0u8; RYNK_BUFFER_SIZE];
        let n = encode_frame(&mut buf, RynkHeader { cmd, seq }, req).expect("build request frame");
        self.tx.write_all(&buf[..n]).await;

        loop {
            let (header, payload) = self.recv().await;
            if header.seq == 0 {
                continue; // an unsolicited topic push raced the reply
            }
            assert_eq!(header.seq, seq, "reply must echo the request seq");
            assert_eq!(header.cmd, cmd, "reply must echo the request cmd");
            let (envelope, rest) = postcard::take_from_bytes::<Result<Resp, RynkError>>(&payload)
                .expect("response payload must decode as an envelope");
            assert!(rest.is_empty(), "response payload has {} trailing byte(s)", rest.len());
            return envelope;
        }
    }

    /// `run_session` mutes topic pushes until the client has asked for
    /// capabilities, as every real host does at connect.
    async fn handshake(&mut self) {
        self.request::<(), DeviceCapabilities>(Cmd::GetCapabilities, 0x7E, &())
            .await
            .expect("handshake");
    }

    /// Await the next unsolicited topic push and decode its bare payload.
    /// Stricter than the host's lenient topic decode on purpose: this pins the
    /// same-version encoder's exact output, trailing bytes included.
    async fn recv_topic<T: DeserializeOwned>(&mut self, cmd: Cmd) -> T {
        let (header, payload) = self.recv().await;
        assert_eq!(header.cmd, cmd, "unexpected topic");
        assert_eq!(header.seq, 0, "topic frames use seq 0");
        let (value, rest) = postcard::take_from_bytes::<T>(&payload).expect("topic payload must decode");
        assert!(rest.is_empty(), "topic payload has {} trailing byte(s)", rest.len());
        value
    }

    async fn recv(&mut self) -> (RynkHeader, Vec<u8>) {
        loop {
            if let Some(len) = self.df.next(&mut self.rxbuf) {
                let frame = &self.rxbuf[..len];
                let header = RynkHeader::parse(frame[..RYNK_HEADER_SIZE].try_into().unwrap());
                return (header, frame[RYNK_HEADER_SIZE..].to_vec());
            }
            let n = self.rx.read(self.df.tail(&mut self.rxbuf)).await;
            self.df.commit(n);
        }
    }
}

/// Play `script` against `service` over the duplex, running `background`
/// alongside it for the cases that need a live lighting engine.
///
/// The pipes never signal EOF, so `run_session` would loop forever; it is
/// dropped once the script returns. Its resolving first is a framing bug.
fn loopback<T>(
    service: &RynkService<'_>,
    background: impl Future<Output = ()>,
    script: impl AsyncFnOnce(&mut Host<'_>) -> T,
) -> T {
    let (h2d, d2h) = (Link::new(), Link::new());
    let (mut dev_rx, mut dev_tx): (&Link, &Link) = (&h2d, &d2h);
    let mut host = Host {
        rx: &d2h,
        tx: &h2d,
        df: Deframer::new(),
        rxbuf: [0u8; RYNK_BUFFER_SIZE],
    };
    test_block_on(async {
        // Drain test flash writes so a storage handler cannot block the session.
        let device = select(
            select(service.run_session(&mut dev_rx, &mut dev_tx), background),
            rmk::channel::drain_flash_channel_for_test(),
        );
        match select(device, script(&mut host)).await {
            Either::First(_) => panic!("a service loop ended before the host script finished"),
            Either::Second(value) => value,
        }
    })
}

static LIGHTING_KEYS: [KeyPosition; 2] = [KeyPosition::new(0, 0), KeyPosition::new(0, 2)];
static LIGHTING_PHYSICAL_KEYS: [PhysicalKey; 1] = [PhysicalKey {
    matrix: KeyPosition::new(0, 0),
    center: Point3::new(Coordinate::ZERO, Coordinate::ONE, Coordinate::ZERO),
    size: KeySize::ONE,
    rotation: Rotation::ZERO,
}];
static LIGHTING_LEDS: [LedMetadata; 1] = [LedMetadata {
    id: LedId(99),
    key: Some(KeyPosition::new(0, 0)),
    position: None,
    zones: ZoneSpan::new(0, 0),
}];

/// A revision other than 0 or 1, so a page pinned to the wrong one is visibly
/// wrong rather than coincidentally right.
const TOPOLOGY_REVISION: u32 = 23;

fn descriptor() -> RynkLightingDescriptor<'static> {
    RynkLightingDescriptor {
        topology_revision: TOPOLOGY_REVISION,
        topology: LightingTopology {
            matrix: MatrixSize::new(1, 3),
            keys: &LIGHTING_KEYS,
            physical_layout: PhysicalLayout::new(&LIGHTING_PHYSICAL_KEYS),
            leds: &LIGHTING_LEDS,
            zones: &[],
            zone_memberships: &[],
        },
        routing: LightingRouting {
            outputs: &[],
            routes: &[],
        },
    }
}

/// A 1x3 service matching the descriptor's matrix, always unlocked so these
/// cases exercise lighting rather than the lock gate.
fn service_with(controller: RynkLightingController<'static>) -> RynkService<'static> {
    let mut config = RmkConfig::default();
    config.lock_config.insecure = true;
    let config: &'static RmkConfig<'static> = Box::leak(Box::new(config));

    let behavior: &'static mut BehaviorConfig = Box::leak(Box::new(BehaviorConfig::default()));
    let positional: &'static PositionalConfig<1, 3> = Box::leak(Box::new(PositionalConfig::default()));
    let data = Box::leak(Box::new(KeymapData::new([[[KeyAction::No; 3]; 1]])));
    let keymap: &'static KeyMap<'static> = Box::leak(Box::new(test_block_on(KeyMap::new(data, behavior, positional))));

    RynkService::new(keymap, config).with_lighting(controller)
}

/// Topology readback answers from the descriptor, so this needs no engine.
fn descriptor_only_service() -> RynkService<'static> {
    let mailbox: &'static RynkLightingMailbox = Box::leak(Box::new(RynkLightingMailbox::new()));
    service_with(RynkLightingController::new(mailbox, descriptor(), 8))
}

#[test]
fn lighting_discovery_and_revision_pinned_keys_cross_the_full_loopback() {
    let service = descriptor_only_service();
    loopback(&service, core::future::pending(), async |host| {
        let caps = host
            .request::<(), DeviceCapabilities>(Cmd::GetCapabilities, 0x71, &())
            .await
            .expect("outer capability envelope");
        assert!(caps.lighting_enabled);

        let keys = host
            .request::<_, LightingKeysPageResult>(
                Cmd::GetLightingKeys,
                0x72,
                &LightingPageRequest {
                    topology_revision: TOPOLOGY_REVISION,
                    offset: 0,
                },
            )
            .await
            .expect("outer keys envelope")
            .expect("lighting keys page");
        assert_eq!(keys.total_count, 2);
        assert_eq!(keys.items[1], LightingMatrixPosition { row: 0, col: 2 });

        // A page pinned to a topology the board no longer serves is refused
        // rather than answered from the current one.
        let stale = host
            .request::<_, LightingKeysPageResult>(
                Cmd::GetLightingKeys,
                0x73,
                &LightingPageRequest {
                    topology_revision: TOPOLOGY_REVISION - 1,
                    offset: 0,
                },
            )
            .await
            .expect("outer stale-page envelope");
        assert_eq!(
            stale,
            Err(LightingError::TopologyRevisionConflict {
                expected: TOPOLOGY_REVISION - 1,
                current: TOPOLOGY_REVISION,
            })
        );
    });
}

/// The lighting readback invalidation is a bare signal: hosts re-read rather
/// than diff it, so the topic carries no state.
#[test]
fn topic_lighting_change_is_a_best_effort_readback_invalidation() {
    let service = descriptor_only_service();
    loopback(&service, core::future::pending(), async |host| {
        host.handshake().await;
        publish_event(rmk::event::LightingChangedEvent::new());
        host.recv_topic::<LightingChanged>(Cmd::LightingChange).await;
    });
}

/// Zero-target lighting source serving only the extension hooks, so the
/// extension endpoints are exercised without a renderable board.
struct ExtensionOnlySource {
    state: rmk::lighting::compositor::ExtensionState,
}

impl<Context> rmk::lighting::compositor::LightingSource<rmk::lighting::Rgb8, Context> for ExtensionOnlySource {
    fn len(&self, _: &rmk::lighting::compositor::RenderInput<'_, Context>) -> usize {
        0
    }

    fn slot(&self, _: usize, _: &rmk::lighting::compositor::RenderInput<'_, Context>) -> rmk::lighting::LedSlot {
        unreachable!("extension-only source has no targets")
    }

    fn contribution(
        &mut self,
        _: usize,
        _: &rmk::lighting::compositor::RenderInput<'_, Context>,
    ) -> rmk::lighting::compositor::Contribution<rmk::lighting::Rgb8> {
        unreachable!("extension-only source has no samples")
    }

    fn extension_descriptor(&self) -> Option<rmk::lighting::compositor::ExtensionDescriptor> {
        Some(rmk::lighting::compositor::ExtensionDescriptor {
            effects: &["Gradient", "Flow"],
            palettes: &["Lava", "Ocean", "Forest"],
            params: &[],
        })
    }

    fn extension_state(&self) -> Option<rmk::lighting::compositor::ExtensionState> {
        Some(self.state)
    }

    fn apply_extension_state(&mut self, state: rmk::lighting::compositor::ExtensionState) -> bool {
        self.state = state;
        true
    }
}

/// The whole stack for the extension endpoints: wire frames → dispatch →
/// handler → protocol mailbox → adapter → standard engine, and back.
#[test]
fn lighting_extension_endpoints_cross_the_full_loopback() {
    use rmk::host::StandardRynkLightingAdapter;
    use rmk::lighting::compositor::ExtensionState;
    use rmk::lighting::{
        BackgroundState, EmptySource, LayerPolicy, LayerScenes, LightingContext, LightingEngine, LightingMailbox,
        StandardCommand, StandardError, StandardLightingEngine, StandardReply,
    };
    use rmk_types::protocol::rynk::{
        LightingCapabilitiesResult, LightingExtensionNameKind, LightingExtensionNamesPageResult,
        LightingExtensionNamesRequest, LightingExtensionResult, LightingExtensionState, LightingFeatureFlags,
        LightingStateResult, SetLightingExtensionStateRequest,
    };

    let mailbox: &'static RynkLightingMailbox = Box::leak(Box::new(RynkLightingMailbox::new()));
    let service = service_with(RynkLightingController::new(mailbox, descriptor(), 8).with_extension_effects());

    let core = LightingMailbox::<StandardCommand<2>, StandardReply, StandardError, 1>::new();
    let mut adapter = StandardRynkLightingAdapter::<2, 1>::new(mailbox, &core, descriptor().topology);
    let mut engine: StandardLightingEngine<'static, ExtensionOnlySource, EmptySource, 1, 2, 0> =
        StandardLightingEngine::new(
            BackgroundState::default(),
            LayerScenes {
                scenes: &[],
                policy: LayerPolicy::EffectiveOnly,
            },
            ExtensionOnlySource {
                state: ExtensionState {
                    effect: 0,
                    palette: 2,
                    value: 200,
                    speed: 40,
                },
            },
            EmptySource,
        );

    let context = LightingContext::default();
    let engine_and_adapter = async {
        let adapter_loop = async {
            loop {
                adapter.process_next().await;
            }
        };
        let engine_loop = async {
            loop {
                let (id, command) = core.receive_request().await;
                let result = engine.handle_command(0, command, &context).map(|outcome| outcome.reply);
                core.publish_reply(id, result);
            }
        };
        select(adapter_loop, engine_loop).await;
    };

    loopback(&service, engine_and_adapter, async |host| {
        let caps = host
            .request::<(), LightingCapabilitiesResult>(Cmd::GetLightingCapabilities, 0x74, &())
            .await
            .expect("outer capabilities envelope")
            .expect("lighting capabilities");
        assert!(caps.features.contains(LightingFeatureFlags::EXTENSION_EFFECTS));

        let extension = host
            .request::<(), LightingExtensionResult>(Cmd::GetLightingExtension, 0x75, &())
            .await
            .expect("outer extension envelope")
            .expect("extension discovery");
        assert_eq!(extension.revision, 0);
        assert_eq!(extension.effect_count, 2);
        assert_eq!(extension.palette_count, 3);
        assert_eq!(
            extension.state,
            LightingExtensionState {
                effect: 0,
                palette: 2,
                value: 200,
                speed: 40,
            }
        );

        let palettes = host
            .request::<_, LightingExtensionNamesPageResult>(
                Cmd::GetLightingExtensionNames,
                0x76,
                &LightingExtensionNamesRequest {
                    kind: LightingExtensionNameKind::Palettes,
                    offset: 0,
                },
            )
            .await
            .expect("outer names envelope")
            .expect("palette names page");
        assert_eq!(palettes.total, 3);
        assert_eq!(palettes.items.len(), 3);
        assert_eq!(palettes.items[1].as_str(), "Ocean");

        let selected = LightingExtensionState {
            effect: 1,
            palette: 0,
            value: 9,
            speed: 8,
        };
        let state = host
            .request::<_, LightingStateResult>(
                Cmd::SetLightingExtensionState,
                0x77,
                &SetLightingExtensionStateRequest {
                    expected_revision: 0,
                    state: selected,
                },
            )
            .await
            .expect("outer set envelope")
            .expect("extension selection");
        assert_eq!(state.revision, 1);

        let extension = host
            .request::<(), LightingExtensionResult>(Cmd::GetLightingExtension, 0x78, &())
            .await
            .expect("outer extension envelope")
            .expect("extension readback");
        assert_eq!(extension.revision, 1);
        assert_eq!(extension.state, selected);

        // The write bumped the revision, so replaying the same expectation is
        // now a stale write rather than an idempotent one.
        let stale = host
            .request::<_, LightingStateResult>(
                Cmd::SetLightingExtensionState,
                0x79,
                &SetLightingExtensionStateRequest {
                    expected_revision: 0,
                    state: selected,
                },
            )
            .await
            .expect("outer stale envelope");
        assert_eq!(
            stale,
            Err(LightingError::StateRevisionConflict {
                expected: 0,
                current: 1,
            })
        );
    });
}

/// A board that never advertised extension effects refuses the endpoint before
/// it can reach any mailbox — which is why this needs no engine behind it.
#[test]
fn extension_endpoints_are_unsupported_until_a_board_advertises_them() {
    use rmk_types::protocol::rynk::LightingExtensionResult;

    let service = descriptor_only_service();
    loopback(&service, core::future::pending(), async |host| {
        let unsupported = host
            .request::<(), LightingExtensionResult>(Cmd::GetLightingExtension, 0x7A, &())
            .await
            .expect("outer unsupported envelope");
        assert_eq!(unsupported, Err(LightingError::Unsupported));
    });
}
