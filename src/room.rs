use rtc::shared::TaggedBytesMut;
use rtc::shared::error::Error;
use sansio::Protocol;
use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::time::Instant;

use crate::client::{Client, ClientBuilder, ClientId};
use crate::event::Event;
use crate::forward::ForwardTable;

pub type RoomId = u64;

#[derive(Default)]
pub(crate) struct Room {
    id: RoomId,
    clients: HashMap<ClientId, Client>,
    forward_table: ForwardTable,

    transmits: VecDeque<TaggedBytesMut>,
    events: VecDeque<Event>,
}

impl Room {
    pub(crate) fn new(id: RoomId) -> Self {
        Self {
            id,
            ..Default::default()
        }
    }

    pub(crate) fn id(&self) -> RoomId {
        self.id
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }
}

impl Protocol<TaggedBytesMut, Infallible, Event> for Room {
    type Rout = Infallible;
    type Wout = TaggedBytesMut;
    type Eout = Event;
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

    fn handle_event(&mut self, evt: Event) -> Result<(), Self::Error> {
        let room_id = if let Some(room_id) = evt.room_id() {
            if room_id != self.id {
                return Err(Error::Other(format!("invalid room id: {}", room_id)));
            }
            room_id
        } else {
            return Err(Error::Other("empty room id".to_string()));
        };

        if let Some(client_id) = evt.client_id() {
            let mut remove_client = false;
            if let Some(client) = self.clients.get_mut(&client_id) {
                if let Event::Leave { .. } = &evt {
                    client.close()?;
                    remove_client = true;
                } else {
                    client.handle_event(evt)?;
                }
            } else if let Event::Join { .. } = &evt {
                //TODO: ClientBuilder with configurable
                let client = ClientBuilder::new(client_id, room_id).build()?;
                self.clients.insert(client_id, client);
            }

            if remove_client {
                self.clients.remove(&client_id);
            }
        }

        Ok(())
    }

    fn poll_event(&mut self) -> Option<Self::Eout> {
        self.events.pop_front()
    }

    fn handle_timeout(&mut self, now: Self::Time) -> Result<(), Self::Error> {
        for client in self.clients.values_mut() {
            let _ = client.handle_timeout(now);
        }
        Ok(())
    }

    fn poll_timeout(&mut self) -> Option<Self::Time> {
        let mut eto: Option<Instant> = None;
        for client in self.clients.values_mut() {
            if let Some(next) = client.poll_timeout() {
                eto = Some(eto.map_or(next, |curr| std::cmp::min(curr, next)));
            }
        }
        eto
    }

    fn close(&mut self) -> Result<(), Self::Error> {
        self.clients.clear();
        self.transmits.clear();
        self.events.clear();
        Ok(())
    }
}
