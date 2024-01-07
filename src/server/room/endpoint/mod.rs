pub mod candidate;

use crate::server::room::Room;
use crate::shared::types::EndpointId;
use std::rc::Weak;

#[derive(Default, Debug)]
pub struct Endpoint {
    room: Weak<Room>,
    endpoint_id: EndpointId,
}

impl Endpoint {
    pub fn new(room: Weak<Room>, endpoint_id: EndpointId) -> Self {
        Self { room, endpoint_id }
    }

    pub fn room(&self) -> &Weak<Room> {
        &self.room
    }

    pub fn endpoint_id(&self) -> EndpointId {
        self.endpoint_id
    }
}
