//! Session lifecycle and basic networking-facing state machine.
//!
//! This is an engine-level bridge for multiplayer behavior. It intentionally
//! keeps transport details separable so gameplay layers can be tested with
//! deterministic inputs without requiring active sockets in early stages.

use std::collections::VecDeque;
use std::io;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};

use crate::multiplayer::matchmaking::PlayerInfo;

use super::net_types::{
    deserialize_relay_packet, serialize_relay_packet, DesyncMode, MatchState, NetMessage,
    NetworkEvent, NetworkPolicy, NetworkTick, PlayerInputFrame, RelayClientPacket,
    RelayServerPacket, SyncMode, RELAY_PROTOCOL_VERSION,
};
use crate::multiplayer::matchmaking::{CtfSlotAssignment, GameMode, MapSize};

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
    game_mode: GameMode,
    map_size: MapSize,
    ctf_assignments: Vec<CtfSlotAssignment>,
    shared_seed: u64,
    socket: UdpSocket,
    local_addr: SocketAddr,
    relay: crate::multiplayer::matchmaking::RelayConnectInfo,
    relay_addr: SocketAddr,
    peers: Vec<(u64, SocketAddr)>,
    outbound_sequence: u64,
    last_relay_heartbeat_tick: NetworkTick,
    tick: NetworkTick,
    input_queue: VecDeque<PlayerInputFrame>,
    event_queue: VecDeque<NetworkEvent>,
}

impl MatchSession {
    /// Start a session from matchmaker state.
    pub fn new(config: NetworkPolicy, state: MatchState, local_peer_id: u64) -> io::Result<Self> {
        let relay_addr = resolve_relay_addr(&state)?;
        let bind_addr = if relay_addr.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let socket = UdpSocket::bind(bind_addr)?;
        Self::new_with_socket(config, state, local_peer_id, socket)
    }

    /// Start a session using a prebound gameplay socket.
    ///
    /// The socket only needs outbound UDP access to the relay. It is never
    /// advertised to other players.
    pub fn new_with_socket(
        config: NetworkPolicy,
        state: MatchState,
        local_peer_id: u64,
        socket: UdpSocket,
    ) -> io::Result<Self> {
        Self::build_session(config, state, local_peer_id, socket)
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

    fn build_session(
        config: NetworkPolicy,
        state: MatchState,
        local_peer_id: u64,
        socket: UdpSocket,
    ) -> io::Result<Self> {
        let role = if local_peer_id == state.host_peer_id {
            MatchRole::Host
        } else {
            MatchRole::Client
        };

        if state.relay.client_id != local_peer_id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "relay token belongs to client {}, not local peer {}",
                    state.relay.client_id, local_peer_id
                ),
            ));
        }

        let local_addr = socket.local_addr()?;
        let relay_addr = resolve_relay_addr(&state)?;
        let _host_player = state
            .players
            .iter()
            .find(|player| player.client_id == state.host_peer_id)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "host peer id not in player list")
            })?;

        socket.set_nonblocking(true).map_err(|error| {
            io::Error::new(error.kind(), format!("set_nonblocking failed: {error}"))
        })?;

        let peers: Vec<(u64, SocketAddr)> = state
            .players
            .iter()
            .filter(|player| player.client_id != local_peer_id)
            .map(|player| (player.client_id, relay_addr))
            .collect();

        let peer_summary = peers
            .iter()
            .map(|(id, addr)| format!("{id}@{addr}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "session init: local_peer={} role={role:?} host_peer={} local_addr={local_addr} relay_addr={relay_addr} peers=[{peer_summary}]",
            local_peer_id,
            state.host_peer_id,
            role = role,
        );

        register_with_relay(&socket, relay_addr, &state.relay)?;

        Ok(Self {
            role,
            config,
            lobby_code: state.lobby_code,
            local_peer_id,
            host_peer_id: state.host_peer_id,
            players: state.players,
            game_mode: state.game_mode,
            map_size: state.map_size,
            ctf_assignments: state.ctf_assignments,
            shared_seed: state.shared_seed,
            socket,
            local_addr,
            relay: state.relay,
            relay_addr,
            peers,
            outbound_sequence: 0,
            last_relay_heartbeat_tick: 0,
            tick: state.start_tick,
            input_queue: VecDeque::new(),
            event_queue: VecDeque::new(),
        })
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

    /// Game mode selected for this session.
    pub fn game_mode(&self) -> GameMode {
        self.game_mode
    }

    /// CTF slot assignments selected by the lobby host.
    pub fn ctf_assignments(&self) -> &[CtfSlotAssignment] {
        &self.ctf_assignments
    }

    /// CTF arena size selected by the lobby host.
    pub fn map_size(&self) -> MapSize {
        self.map_size
    }

    /// Update the active CTF arena size after an in-match level restart.
    pub fn set_map_size(&mut self, map_size: MapSize) {
        self.map_size = map_size;
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

    /// Relay address used for all gameplay packets.
    pub fn relay_addr(&self) -> SocketAddr {
        self.relay_addr
    }

    /// Address of the host for client peers.
    ///
    /// Relay-based sessions do not expose a peer host address, so this returns
    /// the relay endpoint for compatibility with older call sites.
    pub fn host_addr(&self) -> Option<SocketAddr> {
        Some(self.relay_addr)
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
        self.maybe_send_relay_heartbeat();

        // Host receives all input frames, clients forward local input to host.
        let frames = self.drain_local_input();

        if self.is_host() {
            frames.into_iter().for_each(|frame| {
                self.send_message_to_relay(NetMessage::Input(frame.clone()));
                self.event_queue
                    .push_back(NetworkEvent::InputReceived(frame));
            });
        } else {
            frames.into_iter().for_each(|frame| {
                self.send_message_to_relay(NetMessage::Input(frame));
            });
        }

        self.receive_packets();
        self.tick = self.tick.wrapping_add(1);

        if self.event_queue.len() > 2_000 {
            self.event_queue.truncate(2_000);
        }
    }

    /// Broadcast a state hash to all peers (host only).
    pub fn broadcast_hash(&mut self, tick: NetworkTick, hash: u64) {
        self.send_message_to_relay(NetMessage::HostHash { tick, hash });
    }

    /// Send an authoritative correction snapshot to all peers (host only).
    pub fn send_correction(&mut self, tick: NetworkTick, snapshot: super::net_types::Snapshot) {
        self.send_message_to_relay(NetMessage::HostCorrection { tick, snapshot });
    }

    /// Send an authority-scoped snapshot to all peers.
    pub fn send_authority_snapshot(
        &mut self,
        tick: NetworkTick,
        snapshot: super::net_types::Snapshot,
    ) {
        self.send_message_to_relay(NetMessage::AuthoritySnapshot { tick, snapshot });
    }

    fn receive_packets(&mut self) {
        let mut buffer = [0_u8; 65_536];
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((size, _from)) => {
                    if let Ok(packet) =
                        deserialize_relay_packet::<RelayServerPacket>(&buffer[..size])
                    {
                        self.handle_relay_packet(packet);
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

    fn handle_relay_packet(&mut self, packet: RelayServerPacket) {
        match packet {
            RelayServerPacket::Registered { .. } => {}
            RelayServerPacket::Payload {
                from_client_id,
                message,
                ..
            } => {
                if from_client_id == self.local_peer_id {
                    return;
                }
                match message {
                    NetMessage::Input(frame) => {
                        self.event_queue
                            .push_back(NetworkEvent::InputReceived(frame));
                    }
                    NetMessage::HostHash { tick, hash } => {
                        self.event_queue.push_back(NetworkEvent::HostHashReceived {
                            tick,
                            host_hash: hash,
                        });
                    }
                    NetMessage::HostCorrection { tick, snapshot } => {
                        self.event_queue
                            .push_back(NetworkEvent::CorrectionReceived {
                                from_peer_id: from_client_id,
                                tick,
                                snapshot,
                            });
                    }
                    NetMessage::AuthoritySnapshot { tick, snapshot } => {
                        self.event_queue
                            .push_back(NetworkEvent::AuthoritySnapshotReceived {
                                from_peer_id: from_client_id,
                                tick,
                                snapshot,
                            });
                    }
                }
            }
            RelayServerPacket::Error { message } => {
                eprintln!("relay error: {message}");
            }
        }
    }

    fn send_message_to_relay(&mut self, message: NetMessage) {
        self.outbound_sequence = self.outbound_sequence.wrapping_add(1);
        let packet = RelayClientPacket::Payload {
            protocol_version: RELAY_PROTOCOL_VERSION,
            match_id: self.relay.match_id.clone(),
            client_id: self.local_peer_id,
            session_token: self.relay.session_token.clone(),
            sequence: self.outbound_sequence,
            message,
        };
        self.send_relay_packet(packet);
    }

    fn maybe_send_relay_heartbeat(&mut self) {
        let heartbeat_ticks = self
            .relay
            .heartbeat_secs
            .max(1)
            .saturating_mul(self.config.tick_rate.max(1) as u64)
            .min(u32::MAX as u64) as NetworkTick;
        if self.tick != 0
            && self.tick.wrapping_sub(self.last_relay_heartbeat_tick) < heartbeat_ticks
        {
            return;
        }
        self.last_relay_heartbeat_tick = self.tick;
        self.send_relay_packet(RelayClientPacket::Heartbeat {
            protocol_version: RELAY_PROTOCOL_VERSION,
            match_id: self.relay.match_id.clone(),
            client_id: self.local_peer_id,
            session_token: self.relay.session_token.clone(),
        });
    }

    fn send_relay_packet(&self, packet: RelayClientPacket) {
        if let Ok(payload) = serialize_relay_packet(&packet) {
            let _ = self.socket.send_to(&payload, self.relay_addr);
        }
    }
}

fn resolve_relay_addr(state: &MatchState) -> io::Result<SocketAddr> {
    state
        .relay
        .udp_endpoint
        .to_socket_addrs()
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "relay endpoint '{}' could not be resolved; expected host:port like 136.118.42.95:7001: {error}",
                    state.relay.udp_endpoint
                ),
            )
        })?
        .next()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "relay endpoint '{}' did not resolve; expected host:port like 136.118.42.95:7001",
                    state.relay.udp_endpoint
                ),
            )
        })
}

fn register_with_relay(
    socket: &UdpSocket,
    relay_addr: SocketAddr,
    relay: &crate::multiplayer::matchmaking::RelayConnectInfo,
) -> io::Result<()> {
    let packet = RelayClientPacket::Register {
        protocol_version: RELAY_PROTOCOL_VERSION,
        match_id: relay.match_id.clone(),
        client_id: relay.client_id,
        session_token: relay.session_token.clone(),
    };
    let payload = serialize_relay_packet(&packet)?;
    let _ = socket.send_to(&payload, relay_addr)?;
    Ok(())
}
