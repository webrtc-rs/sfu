pub mod candidate;

use crate::server::session::Session;
use crate::shared::types::EndpointId;
use std::rc::Weak;

#[derive(Default, Debug)]
pub struct Endpoint {
    session: Weak<Session>,
    endpoint_id: EndpointId,
}

impl Endpoint {
    pub fn new(session: Weak<Session>, endpoint_id: EndpointId) -> Self {
        Self {
            session,
            endpoint_id,
        }
    }

    pub fn session(&self) -> &Weak<Session> {
        &self.session
    }

    pub fn endpoint_id(&self) -> EndpointId {
        self.endpoint_id
    }
}
