//! Session lifecycle and basic networking-facing state machine.
//!
//! This is an engine-level bridge for multiplayer behavior. It intentionally
//! keeps transport details separable so gameplay layers can be tested with
//! deterministic inputs without requiring active sockets in early stages.

use std::collections::VecDeque;
use std::io;
use std::net::{SocketAddr, UdpSocket};

use crate::multiplayer::matchmaking::PlayerInfo;

use super::net_types::{
    DesyncMode,
    MatchState,
    NetworkEvent,
    NetworkTick,
    NetworkPolicy,
    PlayerInputFrame,
    NetMessage,
    SyncMode,
};

/// Internal role derived from lobby/host-election outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchRole {
    /// This process owns authority for host snapshots and reconciliation.
    Host,
    /// This process receives host corrections and sends inputs.
    Client,
}

/// Host/client runtime session for one gameplay match.
#[derive(Debug)]
pub struct MatchSession {
    role: MatchRole,
    config: NetworkPolicy,
    lobby_code: String,
    local_peer_id: u64,
    host_peer_id: u64,
    players: Vec<PlayerInfo>,
    shared_seed: u64,
    socket: UdpSocket,
    local_addr: SocketAddr,
    host_addr: Option<SocketAddr>,
    peers: Vec<(u64, SocketAddr)>,
    tick: NetworkTick,
    input_queue: VecDeque<PlayerInputFrame>,
    event_queue: VecDeque<NetworkEvent>,
}

impl MatchSession {
    /// Start a session from matchmaker state.
    pub fn new(
        config: NetworkPolicy,
        state: MatchState,
        local_peer_id: u64,
    ) -> io::Result<Self> {
        let role = if local_peer_id == state.host_peer_id {
            MatchRole::Host
        } else {
            MatchRole::Client
        };

        let endpoints = resolve_endpoints(&state.players)?;
        let local_addr = state
            .players
            .iter()
            .find(|player| player.client_id == local_peer_id)
            .and_then(|player| endpoints.get(&player.client_id).copied())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "local peer id not in player list")
            })?;

        let host_addr = state
            .players
            .iter()
            .find(|player| player.client_id == state.host_peer_id)
            .and_then(|player| endpoints.get(&player.client_id).copied())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "host peer id not in player list")
            })?;

        let socket = UdpSocket::bind(local_addr)?;

        socket
            .set_nonblocking(true)
            .map_err(|error| io::Error::new(error.kind(), format!("set_nonblocking failed: {error}")))?;

        let peers: Vec<(u64, SocketAddr)> = state
            .players
            .iter()
            .filter_map(|player| {
                if player.client_id == local_peer_id {
                    return None;
                }
                endpoints.get(&player.client_id).copied().map(|address| (player.client_id, address))
            })
            .collect();

        let endpoint_summary = endpoints
            .iter()
            .map(|(id, addr)| format!("{id}@{addr}"))
            .collect::<Vec<_>>()
            .join(", ");
        let peer_summary = peers
            .iter()
            .map(|(id, addr)| format!("{id}@{addr}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "session init: local_peer={} role={role:?} host_peer={} local_addr={local_addr} host_addr={host_addr:?} endpoints=[{endpoint_summary}] peers=[{peer_summary}]",
            local_peer_id,
            state.host_peer_id,
            role = role,
        );

        Ok(Self {
            role,
            config,
            lobby_code: state.lobby_code,
            local_peer_id,
            host_peer_id: state.host_peer_id,
            players: state.players,
            shared_seed: state.shared_seed,
            socket,
            local_addr,
            host_addr: Some(host_addr),
            peers,
            tick: state.start_tick,
            input_queue: VecDeque::new(),
            event_queue: VecDeque::new(),
        })
    }

    /// Construct a session with explicit policy using defaults.
    pub fn with_defaults(state: MatchState, local_peer_id: u64) -> io::Result<Self> {
        Self::new(
            NetworkPolicy {
            sync_mode: SyncMode::InputReplication,
            desync_mode: DesyncMode::SnapshotCorrections,
            tick_rate: super::net_types::DEFAULT_TICK_RATE,
            },
            state,
            local_peer_id,
        )
    }

    /// Whether this peer is the host.
    pub fn is_host(&self) -> bool {
        self.role == MatchRole::Host
    }

    /// Logical peer id for this process.
    pub fn local_peer_id(&self) -> u64 {
        self.local_peer_id
    }

    /// Host peer id for this match.
    pub fn host_peer_id(&self) -> u64 {
        self.host_peer_id
    }

    /// Shared seed that deterministic systems can use for determinism.
    pub fn shared_seed(&self) -> u64 {
        self.shared_seed
    }

    /// Lobby code associated with this session.
    pub fn lobby_code(&self) -> &str {
        &self.lobby_code
    }

    /// Ordered player list.
    pub fn players(&self) -> &[PlayerInfo] {
        &self.players
    }

    /// Current tick counter.
    pub fn current_tick(&self) -> NetworkTick {
        self.tick
    }

    /// Current tick-rate setting.
    pub fn tick_rate(&self) -> u32 {
        self.config.tick_rate
    }

    /// Current sync policy.
    pub fn policy(&self) -> &NetworkPolicy {
        &self.config
    }

    /// Local gameplay socket bound for this peer.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Address of the host for client peers.
    pub fn host_addr(&self) -> Option<SocketAddr> {
        self.host_addr
    }

    /// List of peers this session is aware of (excluding local peer).
    pub fn peers(&self) -> &[(u64, SocketAddr)] {
        &self.peers
    }

    /// Queue local input for the current tick.
    pub fn enqueue_local_input(&mut self, mut frame: PlayerInputFrame) {
        if frame.tick == 0 {
            frame.tick = self.tick;
        }
        self.input_queue.push_back(frame);
    }

    /// Consume queued local input in tick order.
    pub fn drain_local_input(&mut self) -> Vec<PlayerInputFrame> {
        self.input_queue.drain(..).collect()
    }

    /// Push an event from transport handling.
    pub fn push_network_event(&mut self, event: NetworkEvent) {
        self.event_queue.push_back(event);
    }

    /// Drain all pending simulation-relevant events.
    pub fn drain_network_events(&mut self) -> Vec<NetworkEvent> {
        self.event_queue.drain(..).collect()
    }

    /// Progress one simulation tick.
    ///
    /// This advances the authoritative tick counter, drains outbound local inputs,
    /// and applies one non-blocking receive pass for gameplay packets.
    pub fn tick(&mut self) {
        // Host receives all input frames, clients forward local input to host.
        let frames = self.drain_local_input();

        if self.is_host() {
            frames.into_iter().for_each(|frame| {
                self.send_input_to_peers(frame.clone());
                self.event_queue
                    .push_back(NetworkEvent::InputReceived(frame));
            });
        } else {
            frames.into_iter().for_each(|frame| {
                self.send_input_to_host(frame);
            });
        }

        self.receive_packets();
        self.tick = self.tick.wrapping_add(1);

        if self.event_queue.len() > 2_000 {
            self.event_queue.truncate(2_000);
        }
    }

    fn send_input_to_host(&mut self, input: PlayerInputFrame) {
        if let Some(host_addr) = self.host_addr {
            let message = NetMessage::Input(input);
            let payload = encode_message(&message);
            if let Ok(payload) = payload {
                let _ = self.socket.send_to(&payload, host_addr);
            }
        }
    }

    fn send_input_to_peers(&mut self, input: PlayerInputFrame) {
        let message = NetMessage::Input(input);
        let payload = encode_message(&message);
        if let Ok(payload) = payload {
            self.peers.iter().for_each(|(_, peer_addr)| {
                let _ = self.socket.send_to(&payload, peer_addr);
            });
        }
    }

    fn receive_packets(&mut self) {
        let mut buffer = [0_u8; 65_536];
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((size, _from)) => {
                        if let Ok(message) = decode_message(&buffer[..size]) {
                            match message {
                            NetMessage::Input(frame) => {
                                if self.is_host() {
                                    self.send_input_to_peers(frame.clone());
                                }
                                self.event_queue.push_back(NetworkEvent::InputReceived(frame));
                            }
                            NetMessage::HostHash { .. } => {
                                // Reserved for a later phase where we compare local/remote hashes.
                            }
                            NetMessage::HostCorrection { tick, snapshot } => {
                                self.event_queue.push_back(NetworkEvent::CorrectionReceived {
                                    tick,
                                    snapshot,
                                });
                            }
                        }
                    }
                }
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        || error.kind() == io::ErrorKind::TimedOut =>
                {
                    break;
                }
                Err(_) => {
                    break;
                }
            }
        }
    }
}

fn encode_message(message: &NetMessage) -> io::Result<Vec<u8>> {
    bincode::serialize(message).map_err(io::Error::other)
}

fn decode_message(bytes: &[u8]) -> io::Result<NetMessage> {
    bincode::deserialize(bytes).map_err(io::Error::other)
}

fn resolve_endpoints(players: &[PlayerInfo]) -> io::Result<std::collections::HashMap<u64, SocketAddr>> {
    players
        .iter()
        .map(|player| {
            let addr = player
                .game_addr
                .parse::<SocketAddr>()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, format!("invalid game_addr '{}': {error}", player.game_addr)))?;
            Ok((player.client_id, addr))
        })
        .collect()
}
