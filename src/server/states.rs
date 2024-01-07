use crate::server::config::ServerConfig;
use crate::server::room::{endpoint::Endpoint, Room};
use crate::shared::types::{FourTuple, RoomId};
use shared::error::{Error, Result};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Debug)]
pub struct ServerStates {
    config: Arc<ServerConfig>,
    rooms: RefCell<HashMap<RoomId, Rc<Room>>>,
    endpoints: RefCell<HashMap<FourTuple, Rc<Endpoint>>>,
}

impl ServerStates {
    pub fn new(config: Arc<ServerConfig>) -> Result<Self> {
        let _ = config
            .certificate
            .get_fingerprints()
            .first()
            .ok_or(Error::ErrInvalidCertificate)?;

        Ok(Self {
            config,
            rooms: RefCell::new(HashMap::new()),
            endpoints: RefCell::new(HashMap::new()),
        })
    }

    pub fn create_or_get_session(&self, room_id: RoomId) -> Rc<Room> {
        let mut rooms = self.rooms.borrow_mut();
        if let Some(room) = rooms.get(&room_id) {
            room.clone()
        } else {
            let room = Rc::new(Room::new(
                room_id,
                self.config
                    .certificate
                    .get_fingerprints()
                    .first()
                    .unwrap()
                    .clone(),
            ));
            rooms.insert(room_id, Rc::clone(&room));
            room
        }
    }
}
