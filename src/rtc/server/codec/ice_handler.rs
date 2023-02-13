use async_trait::async_trait;
use log::{trace, warn};
use std::error::Error;
use std::sync::Arc;
use std::time::Instant;

use crate::rtc::{endpoint::Endpoint, server::ServerStates};

use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use retty::transport::TaggedBytesMut;
use stun::{
    attributes::ATTR_USERNAME,
    message::{Message, CLASS_REQUEST, CLASS_SUCCESS_RESPONSE, METHOD_BINDING},
};
use webrtc_ice::util::{assert_inbound_message_integrity, assert_inbound_username};

struct ICEDecoder {
    server_states: Arc<ServerStates>,
}
struct ICEEncoder;

pub struct ICEHandler {
    decoder: ICEDecoder,
    encoder: ICEEncoder,
}

impl ICEHandler {
    pub fn new(server_states: Arc<ServerStates>) -> Self {
        ICEHandler {
            decoder: ICEDecoder { server_states },
            encoder: ICEEncoder {},
        }
    }
}

impl ICEDecoder {
    fn split_username(
        stun_message: &Message,
    ) -> Result<(String, String), Box<dyn Error + Send + Sync>> {
        match stun_message.get(ATTR_USERNAME) {
            Ok(username) => {
                let fields: Vec<&[u8]> = username.splitn(2, |c| *c == b':').collect();
                Ok((
                    String::from_utf8(fields[0].to_vec()).unwrap(),
                    String::from_utf8(fields[1].to_vec()).unwrap(),
                ))
            }
            Err(err) => Err(Box::new(err)),
        }
    }

    fn extract_ids(local_ufrag: &str) -> Result<(u64, u64), Box<dyn Error + Send + Sync>> {
        if local_ufrag.len() >= 32 {
            let room_id = u64::from_str_radix(&local_ufrag[0..16], 16)?;
            let endpoint_id = u64::from_str_radix(&local_ufrag[16..32], 16)?;
            Ok((room_id, endpoint_id))
        } else {
            Err(Box::new(stun::Error::ErrDecodeToNil))
        }
    }

    fn stun_server_handle_message(
        _endpoint: Arc<Endpoint>,
        _msg: &TaggedBytesMut,
        _stun_message: &Message,
    ) {
    }

    fn stun_client_handle_response(
        _endpoint: Arc<Endpoint>,
        _now: Instant,
        _stun_message: &Message,
    ) {
    }
}

#[async_trait]
impl InboundHandler for ICEDecoder {
    type Rin = TaggedBytesMut;
    type Rout = Self::Rin;

    async fn read(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, msg: Self::Rin) {
        if stun::message::is_message(&msg.message) {
            let mut stun_message = stun::message::Message::default();
            if let Err(err) = stun_message.unmarshal_binary(&msg.message) {
                warn!(
                    "Failed to handle decode ICE from {:?} to {}: {}",
                    msg.transport.peer_addr, msg.transport.local_addr, err
                );
                ctx.fire_read_exception(Box::new(err)).await;
                return;
            }

            let mut handled = true;
            if stun_message.typ.method == METHOD_BINDING {
                let (local_ufrag, _remote_ufrag) = match ICEDecoder::split_username(&stun_message) {
                    Ok(username) => username,
                    Err(err) => {
                        ctx.fire_read_exception(err).await;
                        return;
                    }
                };
                let (room_id, endpoint_id) = match ICEDecoder::extract_ids(&local_ufrag) {
                    Ok(ids) => ids,
                    Err(err) => {
                        ctx.fire_read_exception(err).await;
                        return;
                    }
                };

                let room = match self.server_states.get(room_id).await {
                    Some(room) => room,
                    None => {
                        ctx.fire_read_exception(Box::new(stun::Error::ErrNoConnection))
                            .await;
                        return;
                    }
                };
                let endpoint = match room.get(endpoint_id).await {
                    Some(endpoint) => endpoint,
                    None => {
                        ctx.fire_read_exception(Box::new(stun::Error::ErrNoConnection))
                            .await;
                        return;
                    }
                };

                let ufrag_pwd = endpoint.ufrag_pwd();
                if stun_message.typ.class == CLASS_REQUEST {
                    let username = format!("{}:{}", ufrag_pwd.local_ufrag, ufrag_pwd.remote_ufrag);

                    if let Err(err) = assert_inbound_username(&stun_message, &username) {
                        warn!(
                            "discard message from ({:?}), {}",
                            msg.transport.peer_addr, err
                        );
                        ctx.fire_read_exception(Box::new(err)).await;
                        return;
                    }

                    if let Err(err) = assert_inbound_message_integrity(
                        &mut stun_message,
                        ufrag_pwd.local_pwd.as_bytes(),
                    ) {
                        warn!(
                            "discard message from ({:?}), {}",
                            msg.transport.peer_addr, err
                        );
                        ctx.fire_read_exception(Box::new(err)).await;
                        return;
                    }

                    ICEDecoder::stun_server_handle_message(endpoint, &msg, &stun_message);
                } else if stun_message.typ.class == CLASS_SUCCESS_RESPONSE {
                    if let Err(err) = assert_inbound_message_integrity(
                        &mut stun_message,
                        ufrag_pwd.remote_pwd.as_bytes(),
                    ) {
                        warn!(
                            "discard message from ({:?}), {}",
                            msg.transport.peer_addr, err
                        );
                        ctx.fire_read_exception(Box::new(err)).await;
                        return;
                    }

                    ICEDecoder::stun_client_handle_response(endpoint, msg.now, &stun_message);
                    //TODO: } else if stun_message.typ.class == CLASS_INDICATION {
                } else {
                    handled = false;
                }
            } else {
                handled = false;
            }
            if !handled {
                trace!(
                    "Unhandled STUN from {:?} to {} class({}) method({})",
                    msg.transport.peer_addr,
                    msg.transport.local_addr,
                    stun_message.typ.class,
                    stun_message.typ.method
                );
            }
        } else {
            ctx.fire_read(msg).await;
        }
    }
}

#[async_trait]
impl OutboundHandler for ICEEncoder {
    type Win = TaggedBytesMut;
    type Wout = Self::Win;

    async fn write(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>, msg: Self::Win) {
        ctx.fire_write(msg).await;
    }
}

impl Handler for ICEHandler {
    type Rin = TaggedBytesMut;
    type Rout = Self::Rin;
    type Win = TaggedBytesMut;
    type Wout = Self::Win;

    fn name(&self) -> &str {
        "ICEHandler"
    }

    fn split(
        self,
    ) -> (
        Box<dyn InboundHandler<Rin = Self::Rin, Rout = Self::Rout>>,
        Box<dyn OutboundHandler<Win = Self::Win, Wout = Self::Wout>>,
    ) {
        (Box::new(self.decoder), Box::new(self.encoder))
    }
}
