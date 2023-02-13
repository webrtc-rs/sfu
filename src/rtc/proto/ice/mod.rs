use log::warn;
use retty::transport::TaggedBytesMut;
use std::error::Error;
use std::time::Instant;
use stun::message::{Message, CLASS_INDICATION, CLASS_REQUEST, CLASS_SUCCESS_RESPONSE};
use webrtc_ice::util::{assert_inbound_message_integrity, assert_inbound_username};

#[derive(Default, Clone)]
pub(crate) struct UfragPwd {
    pub(crate) local_ufrag: String,
    pub(crate) local_pwd: String,
    pub(crate) remote_ufrag: String,
    pub(crate) remote_pwd: String,
}

#[derive(Default)]
pub(crate) struct Agent {
    pub(crate) ufrag_pwd: UfragPwd,
}

impl Agent {
    pub(crate) fn stun_server_handle_message(
        &mut self,
        _msg: &TaggedBytesMut,
        _stun_message: &Message,
    ) {
    }

    pub(crate) fn stun_client_handle_response(&mut self, _now: Instant, _stun_message: &Message) {}

    pub(crate) async fn handle_inbound_msg(
        &mut self,
        msg: TaggedBytesMut,
        mut stun_message: Message,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        if stun_message.typ.class == CLASS_REQUEST {
            let username = format!(
                "{}:{}",
                self.ufrag_pwd.local_ufrag, self.ufrag_pwd.remote_ufrag
            );

            if let Err(err) = assert_inbound_username(&stun_message, &username) {
                warn!(
                    "discard message from ({:?}), {}",
                    msg.transport.peer_addr, err
                );
                return Err(Box::new(err));
            }

            if let Err(err) = assert_inbound_message_integrity(
                &mut stun_message,
                self.ufrag_pwd.local_pwd.as_bytes(),
            ) {
                warn!(
                    "discard message from ({:?}), {}",
                    msg.transport.peer_addr, err
                );
                return Err(Box::new(err));
            }

            self.stun_server_handle_message(&msg, &stun_message);
        } else if stun_message.typ.class == CLASS_SUCCESS_RESPONSE {
            if let Err(err) = assert_inbound_message_integrity(
                &mut stun_message,
                self.ufrag_pwd.remote_pwd.as_bytes(),
            ) {
                warn!(
                    "discard message from ({:?}), {}",
                    msg.transport.peer_addr, err
                );
                return Err(Box::new(err));
            }

            self.stun_client_handle_response(msg.now, &stun_message);
        } else if stun_message.typ.class == CLASS_INDICATION {
        }

        Ok(())
    }
}
