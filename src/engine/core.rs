use crate::RoomId;
use crate::engine::demuxer::Demuxer;
use crate::engine::event::{SFUCommand, SFUEvent};
use crate::engine::room::Room;
use rtc::shared::TaggedBytesMut;
use rtc::shared::error::Error;
use sansio::Protocol;
use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::time::Instant;

#[derive(Default)]
pub(crate) struct SFUCore {
    demuxer: Demuxer,
    rooms: HashMap<RoomId, Room>,

    transmits: VecDeque<TaggedBytesMut>,
    events: VecDeque<SFUEvent>,
}

impl SFUCore {
    pub fn new() -> Self {
        Self::default()
    }

    /*
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

    pub fn poll_client_timeout(&mut self, client_id: ClientId) -> Option<Instant> {
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
    }*/
}

impl Protocol<TaggedBytesMut, Infallible, SFUCommand> for SFUCore {
    type Rout = Infallible;
    type Wout = TaggedBytesMut;
    type Eout = SFUEvent;
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
        for room in self.rooms.values_mut() {
            while let Some(transmit) = room.poll_write() {
                self.transmits.push_back(transmit);
            }
        }
        self.transmits.pop_front()
    }

    fn handle_event(&mut self, _evt: SFUCommand) -> Result<(), Self::Error> {
        /*match evt {
            SFUCommand::AcceptOffer {
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
                self.events.push_back(SFUEvent::Offer { client, offer });
                self.mark_dirty(client);
            }
            SFUCommand::AcceptAnswer {
                request_id,
                client,
                answer,
            } => {
                self.events.push_back(SFUEvent::Answer {
                    request_id,
                    client,
                    answer,
                });
                self.mark_dirty(client);
            }
            SFUCommand::AddRemoteCandidate { client, candidate } => {
                self.events
                    .push_back(SFUEvent::LocalCandidate { client, candidate });
                self.mark_dirty(client);
            }
            SFUCommand::CloseClient { client } => {
                if let Some(existing) = self.clients.remove(&client)
                    && let Some(room) = self.rooms.get_mut(&existing.room_id)
                {
                    room.clients.remove(&client);
                }
                self.events
                    .push_back(SFUEvent::ClientDisconnected { client });
                self.dirty.remove(&client);
            }
        }*/
        Ok(())
    }

    fn poll_event(&mut self) -> Option<Self::Eout> {
        for room in self.rooms.values_mut() {
            while let Some(event) = room.poll_event() {
                self.events.push_back(event);
            }
        }

        self.events.pop_front()
    }

    fn handle_timeout(&mut self, now: Self::Time) -> Result<(), Self::Error> {
        for room in self.rooms.values_mut() {
            let _ = room.handle_timeout(now);
        }
        Ok(())
    }

    fn poll_timeout(&mut self) -> Option<Self::Time> {
        for _room in self.rooms.values_mut() {
            //TODO:  if let Some(timeout) = room.poll_timeout() {}
        }
        None
    }

    fn close(&mut self) -> Result<(), Self::Error> {
        self.rooms.clear();
        Ok(())
    }
}
