use crate::engine::client::Client;
use crate::engine::command::{SfuCommand, SfuEvent};
use crate::engine::forward::ForwardTable;
use crate::engine::ids::{ClientId, RoomId};
use crate::engine::room::Room;
use crate::engine::router::Router;
use rtc::peer_connection::event::RTCPeerConnectionEvent;
use rtc::peer_connection::message::RTCMessage;
use rtc::shared::TaggedBytesMut;
use rtc::shared::error::Error;
use sansio::Protocol;
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::time::Instant;

#[derive(Default)]
pub struct SfuCore {
    pub router: Router,
    pub rooms: HashMap<RoomId, Room>,
    pub clients: HashMap<ClientId, Client>,
    pub forward: ForwardTable,
    transmits: VecDeque<TaggedBytesMut>,
    events: VecDeque<SfuEvent>,
    dirty: HashSet<ClientId>,
    next_timeout: Option<Instant>,
}

impl SfuCore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mark_dirty(&mut self, client_id: ClientId) {
        self.dirty.insert(client_id);
    }

    fn ensure_room(&mut self, room_id: RoomId) -> &mut Room {
        self.rooms
            .entry(room_id)
            .or_insert_with(|| Room::new(room_id))
    }

    pub fn upsert_client(&mut self, client: Client) {
        let room_id = client.room_id;
        let client_id = client.id;
        self.ensure_room(room_id).clients.insert(client_id);
        self.clients.insert(client_id, client);
    }

    pub fn client(&self, client_id: ClientId) -> Option<&Client> {
        self.clients.get(&client_id)
    }

    pub fn client_mut(&mut self, client_id: ClientId) -> Option<&mut Client> {
        self.clients.get_mut(&client_id)
    }

    pub fn clients_with_peer_connections(&self) -> impl Iterator<Item = ClientId> + '_ {
        self.clients
            .iter()
            .filter_map(|(client_id, client)| client.has_peer_connection().then_some(*client_id))
    }

    pub fn drain_client(
        &mut self,
        client_id: ClientId,
        transmits: &mut VecDeque<TaggedBytesMut>,
        events: &mut VecDeque<RTCPeerConnectionEvent>,
        reads: &mut VecDeque<RTCMessage>,
    ) {
        let Some(client) = self.client_mut(client_id) else {
            return;
        };
        let Some(pc) = client.pc.as_mut() else {
            return;
        };

        while let Some(msg) = pc.poll_write() {
            transmits.push_back(msg);
        }

        while let Some(event) = pc.poll_event() {
            events.push_back(event);
        }

        while let Some(read) = pc.poll_read() {
            reads.push_back(read);
        }
    }

    pub fn client_timeout(&mut self, client_id: ClientId) -> Option<Instant> {
        self.client_mut(client_id)
            .and_then(|client| client.pc.as_mut())
            .and_then(|pc| pc.poll_timeout())
    }

    pub fn handle_client_timeout(
        &mut self,
        client_id: ClientId,
        now: Instant,
    ) -> Result<(), Error> {
        if let Some(client) = self.client_mut(client_id)
            && let Some(pc) = client.pc.as_mut()
        {
            pc.handle_timeout(now)?;
        }

        Ok(())
    }
}

impl Protocol<TaggedBytesMut, Infallible, SfuCommand> for SfuCore {
    type Rout = Infallible;
    type Wout = TaggedBytesMut;
    type Eout = SfuEvent;
    type Error = Error;
    type Time = Instant;

    fn handle_read(&mut self, _msg: TaggedBytesMut) -> Result<(), Self::Error> {
        Ok(())
    }

    fn poll_read(&mut self) -> Option<Self::Rout> {
        None
    }

    fn handle_write(&mut self, _msg: Infallible) -> Result<(), Self::Error> {
        match _msg {}
    }

    fn poll_write(&mut self) -> Option<Self::Wout> {
        self.transmits.pop_front()
    }

    fn handle_event(&mut self, evt: SfuCommand) -> Result<(), Self::Error> {
        match evt {
            SfuCommand::AcceptOffer {
                request_id,
                room,
                client,
                offer,
            } => {
                let room_ref = self.ensure_room(room);
                room_ref.clients.insert(client);
                let client_ref = self
                    .clients
                    .entry(client)
                    .or_insert_with(|| Client::new(client, room));
                client_ref.pending_request = Some(request_id);
                self.events.push_back(SfuEvent::Offer { client, offer });
                self.mark_dirty(client);
            }
            SfuCommand::AcceptAnswer {
                request_id,
                client,
                answer,
            } => {
                self.events.push_back(SfuEvent::Answer {
                    request_id,
                    client,
                    answer,
                });
                self.mark_dirty(client);
            }
            SfuCommand::AddLocalCandidate { client, candidate } => {
                self.events
                    .push_back(SfuEvent::LocalCandidate { client, candidate });
                self.mark_dirty(client);
            }
            SfuCommand::CloseClient { client } => {
                if let Some(existing) = self.clients.remove(&client)
                    && let Some(room) = self.rooms.get_mut(&existing.room_id)
                {
                    room.clients.remove(&client);
                }
                self.events
                    .push_back(SfuEvent::ClientDisconnected { client });
                self.dirty.remove(&client);
            }
        }
        Ok(())
    }

    fn poll_event(&mut self) -> Option<Self::Eout> {
        self.events.pop_front()
    }

    fn handle_timeout(&mut self, now: Self::Time) -> Result<(), Self::Error> {
        self.next_timeout = Some(now);
        Ok(())
    }

    fn poll_timeout(&mut self) -> Option<Self::Time> {
        self.next_timeout
    }

    fn close(&mut self) -> Result<(), Self::Error> {
        self.clients.clear();
        self.rooms.clear();
        self.dirty.clear();
        Ok(())
    }
}
