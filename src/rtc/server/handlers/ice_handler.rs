use async_trait::async_trait;
use log::{trace, warn};
use std::error::Error;
use std::sync::Arc;

use crate::rtc::server::ServerStates;

use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use retty::transport::TaggedBytesMut;
use stun::{
    attributes::ATTR_USERNAME,
    message::{Message, METHOD_BINDING},
};

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
    fn extract_ids(stun_message: &Message) -> Result<(u64, u64), Box<dyn Error + Send + Sync>> {
        let local_ufrag = match stun_message.get(ATTR_USERNAME) {
            Ok(username) => {
                let fields: Vec<&[u8]> = username.splitn(2, |c| *c == b':').collect();
                String::from_utf8(fields[0].to_vec()).unwrap()
            }
            Err(err) => return Err(Box::new(err)),
        };

        if local_ufrag.len() >= 32 {
            let room_id = u64::from_str_radix(&local_ufrag[0..16], 16)?;
            let endpoint_id = u64::from_str_radix(&local_ufrag[16..32], 16)?;
            Ok((room_id, endpoint_id))
        } else {
            Err(Box::new(stun::Error::ErrDecodeToNil))
        }
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

            if stun_message.typ.method == METHOD_BINDING {
                let (room_id, endpoint_id) = match ICEDecoder::extract_ids(&stun_message) {
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

                let result = {
                    let mut ice_agent = endpoint.ice_agent.lock().await;
                    ice_agent.handle_inbound_msg(msg, stun_message).await
                };
                if let Err(err) = result {
                    ctx.fire_read_exception(err).await;
                    return;
                }
            } else {
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
