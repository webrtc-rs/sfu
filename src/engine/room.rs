use crate::{Client, ClientId, ForwardTable, SFUCommand, SFUEvent};
use rtc::shared::TaggedBytesMut;
use rtc::shared::error::Error;
use sansio::Protocol;
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::time::Instant;

pub type RoomId = u64;

#[derive(Default)]
pub(crate) struct Room {
    id: RoomId,
    clients: HashMap<ClientId, Client>,
    forward_table: ForwardTable,

    transmits: VecDeque<TaggedBytesMut>,
    events: VecDeque<SFUEvent>,
    dirty: HashSet<ClientId>,
}

impl Room {
    pub fn new(id: RoomId) -> Self {
        Self {
            id,
            ..Default::default()
        }
    }
}

impl Protocol<TaggedBytesMut, Infallible, SFUCommand> for Room {
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
        self.events.pop_front()
    }

    fn handle_timeout(&mut self, now: Self::Time) -> Result<(), Self::Error> {
        for client in self.clients.values_mut() {
            if let Some(pc) = client.pc.as_mut() {
                let _ = pc.handle_timeout(now);
            }
        }
        Ok(())
    }

    fn poll_timeout(&mut self) -> Option<Self::Time> {
        //TODO:
        None
    }

    fn close(&mut self) -> Result<(), Self::Error> {
        self.clients.clear();
        self.dirty.clear();
        Ok(())
    }
}
