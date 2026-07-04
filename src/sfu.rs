use rtc::shared::TaggedBytesMut;
use rtc::shared::error::Error;
use sansio::Protocol;
use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::time::Instant;

use crate::demuxer::Demuxer;
use crate::event::Event;
use crate::room::{Room, RoomId};

pub type SfuId = u64;

#[derive(Default)]
pub struct Sfu {
    id: SfuId,
    demuxer: Demuxer,
    rooms: HashMap<RoomId, Room>,

    transmits: VecDeque<TaggedBytesMut>,
    events: VecDeque<Event>,
}

impl Sfu {
    pub fn new(id: SfuId) -> Self {
        Self {
            id,
            ..Default::default()
        }
    }
}

impl Protocol<TaggedBytesMut, Infallible, Event> for Sfu {
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
        for room in self.rooms.values_mut() {
            while let Some(transmit) = room.poll_write() {
                self.transmits.push_back(transmit);
            }
        }
        self.transmits.pop_front()
    }

    fn handle_event(&mut self, evt: Event) -> Result<(), Self::Error> {
        if let Some(room_id) = evt.room_id() {
            if let Some(room) = self.rooms.get_mut(&room_id) {
                room.handle_event(evt)?;
            } else if let Event::Join { .. } = &evt {
                let mut room = Room::new(room_id);
                room.handle_event(evt)?;
                self.rooms.insert(room_id, room);
            }
        }

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
        let mut eto: Option<Instant> = None;
        for room in self.rooms.values_mut() {
            if let Some(next) = room.poll_timeout() {
                eto = Some(eto.map_or(next, |curr| std::cmp::min(curr, next)));
            }
        }
        eto
    }

    fn close(&mut self) -> Result<(), Self::Error> {
        self.rooms.clear();
        self.transmits.clear();
        self.events.clear();
        Ok(())
    }
}
