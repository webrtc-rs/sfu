use crate::shared::types::{EndpointId, RoomId};

#[derive(Default, Debug)]
pub struct Endpoint {
    room_id: RoomId,
    endpoint_id: EndpointId,
}

impl Endpoint {
    pub fn new(room_id: RoomId, endpoint_id: EndpointId) -> Self {
        Self {
            room_id,
            endpoint_id,
        }
    }

    pub fn room_id(&self) -> RoomId {
        self.room_id
    }

    pub fn endpoint_id(&self) -> EndpointId {
        self.endpoint_id
    }
}
