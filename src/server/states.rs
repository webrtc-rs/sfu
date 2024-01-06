use crate::server::config::ServerConfig;
use crate::server::room::{endpoint::Endpoint, Room};
use crate::shared::types::{FourTuple, RoomId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

#[derive(Default, Debug)]
pub struct ServerStates {
    config: Arc<ServerConfig>,
    rooms: RefCell<HashMap<RoomId, Rc<Room>>>,
    endpoints: RefCell<HashMap<FourTuple, Rc<Endpoint>>>,
}

impl ServerStates {
    pub fn new(config: Arc<ServerConfig>) -> Self {
        Self {
            config,
            ..Default::default()
        }
    }
}
