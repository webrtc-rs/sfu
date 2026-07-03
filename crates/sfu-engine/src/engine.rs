use crate::client::Client;
use crate::command::{SfuCommand, SfuEvent};
use crate::forward::ForwardTable;
use crate::ids::{ClientId, RoomId};
use crate::room::Room;
use crate::router::Router;
use rtc::shared::TaggedBytesMut;
use rtc::shared::error::Error;
use sansio::Protocol;
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::time::Instant;

#[derive(Default)]
pub struct SfuEngine {
    pub router: Router,
    pub rooms: HashMap<RoomId, Room>,
    pub clients: HashMap<ClientId, Client>,
    pub forward: ForwardTable,
    transmits: VecDeque<TaggedBytesMut>,
    events: VecDeque<SfuEvent>,
    dirty: HashSet<ClientId>,
    next_timeout: Option<Instant>,
}

impl SfuEngine {
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
}

impl Protocol<TaggedBytesMut, Infallible, SfuCommand> for SfuEngine {
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
