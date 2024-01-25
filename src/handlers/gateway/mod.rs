use crate::messages::{
    ApplicationMessage, DTLSMessageEvent, MessageEvent, RTPMessageEvent, STUNMessageEvent,
    TaggedMessageEvent,
};
use crate::server::endpoint::{candidate::Candidate, Endpoint};
use crate::server::session::description::sdp_type::RTCSdpType;
use crate::server::session::description::RTCSessionDescription;
use crate::server::states::ServerStates;
use bytes::BytesMut;
use log::{debug, info, warn};
use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use retty::transport::TransportContext;
use shared::error::{Error, Result};
use std::rc::Rc;
use std::time::Instant;
use stun::attributes::{
    ATTR_ICE_CONTROLLED, ATTR_ICE_CONTROLLING, ATTR_NETWORK_COST, ATTR_PRIORITY, ATTR_USERNAME,
    ATTR_USE_CANDIDATE,
};
use stun::fingerprint::FINGERPRINT;
use stun::integrity::MessageIntegrity;
use stun::message::{Setter, TransactionId, BINDING_SUCCESS};
use stun::textattrs::TextAttribute;
use stun::xoraddr::XorMappedAddress;

struct GatewayInbound {
    server_states: Rc<ServerStates>,
}
struct GatewayOutbound {
    server_states: Rc<ServerStates>,
}

pub struct GatewayHandler {
    gateway_inbound: GatewayInbound,
    gateway_outbound: GatewayOutbound,
}

impl GatewayHandler {
    pub fn new(server_states: Rc<ServerStates>) -> Self {
        GatewayHandler {
            gateway_inbound: GatewayInbound {
                server_states: Rc::clone(&server_states),
            },
            gateway_outbound: GatewayOutbound { server_states },
        }
    }
}

impl InboundHandler for GatewayInbound {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;

    fn read(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, msg: Self::Rin) {
        let try_read = || -> Result<Option<Vec<TaggedMessageEvent>>> {
            match msg.message {
                MessageEvent::STUN(STUNMessageEvent::STUN(message)) => {
                    Ok(Some(vec![self.handle_stun_message(
                        msg.now,
                        msg.transport,
                        message,
                    )?]))
                }
                MessageEvent::DTLS(DTLSMessageEvent::APPLICATION(message)) => {
                    Ok(Some(vec![self.handle_dtls_message(
                        msg.now,
                        msg.transport,
                        message,
                    )?]))
                }
                MessageEvent::RTP(RTPMessageEvent::RTP(message)) => Ok(Some(
                    self.handle_rtp_message(msg.now, msg.transport, message)?,
                )),
                MessageEvent::RTP(RTPMessageEvent::RTCP(message)) => Ok(Some(
                    self.handle_rtcp_message(msg.now, msg.transport, message)?,
                )),
                _ => {
                    warn!(
                        "drop unsupported message {:?} from {}",
                        msg.message, msg.transport.peer_addr
                    );
                    Ok(None)
                }
            }
        };

        match try_read() {
            Ok(messages) => {
                if let Some(messages) = messages {
                    for message in messages {
                        ctx.fire_write(message);
                    }
                }
            }
            Err(err) => {
                warn!("try_read got error {}", err);
                ctx.fire_read_exception(Box::new(err));
            }
        }
    }
}

impl OutboundHandler for GatewayOutbound {
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn write(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>, msg: Self::Win) {
        ctx.fire_write(msg);
    }
}

impl Handler for GatewayHandler {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn name(&self) -> &str {
        "GatewayHandler"
    }

    fn split(
        self,
    ) -> (
        Box<dyn InboundHandler<Rin = Self::Rin, Rout = Self::Rout>>,
        Box<dyn OutboundHandler<Win = Self::Win, Wout = Self::Wout>>,
    ) {
        (
            Box::new(self.gateway_inbound),
            Box::new(self.gateway_outbound),
        )
    }
}

impl GatewayInbound {
    fn handle_stun_message(
        &mut self,
        now: Instant,
        transport_context: TransportContext,
        mut request: stun::message::Message,
    ) -> Result<TaggedMessageEvent> {
        let candidate = match self.check_stun_message(&mut request)? {
            Some(candidate) => candidate,
            None => {
                return self.send_server_reflective_address(
                    now,
                    transport_context,
                    request.transaction_id,
                );
            }
        };

        let _ = self.add_endpoint(&request, &candidate, &transport_context)?;

        let mut response = stun::message::Message::new();
        response.build(&[
            Box::new(BINDING_SUCCESS),
            Box::new(request.transaction_id),
            Box::new(XorMappedAddress {
                ip: transport_context.peer_addr.ip(),
                port: transport_context.peer_addr.port(),
            }),
        ])?;
        let integrity = MessageIntegrity::new_short_term_integrity(
            candidate.get_local_parameters().password.clone(),
        );
        integrity.add_to(&mut response)?;
        FINGERPRINT.add_to(&mut response)?;

        debug!(
            "handle_stun_message response type {} with ip {} and port {} sent",
            response.typ,
            transport_context.peer_addr.ip(),
            transport_context.peer_addr.port()
        );

        Ok(TaggedMessageEvent {
            now,
            transport: transport_context,
            message: MessageEvent::STUN(STUNMessageEvent::STUN(response)),
        })
    }

    fn handle_dtls_message(
        &mut self,
        now: Instant,
        transport_context: TransportContext,
        mut message: ApplicationMessage,
    ) -> Result<TaggedMessageEvent> {
        let request_sdp_str = String::from_utf8(message.payload.to_vec())?;
        info!("handle_dtls_message: request_sdp {}", request_sdp_str);

        let request_sdp = serde_json::from_str::<RTCSessionDescription>(&request_sdp_str)
            .map_err(|err| Error::Other(err.to_string()))?;

        let four_tuple = (&transport_context).into();
        let (session_id, endpoint_id) = {
            let endpoint = self
                .server_states
                .find_endpoint(&four_tuple)
                .ok_or(Error::ErrClientTransportNotSet)?;
            let session = endpoint.session().upgrade().ok_or(Error::SessionEof)?;
            (session.session_id(), endpoint.endpoint_id())
        };

        let response_sdp = match request_sdp.sdp_type {
            RTCSdpType::Offer => self.server_states.accept_offer(
                session_id,
                endpoint_id,
                Some((&transport_context).into()),
                request_sdp,
            )?,
            //TODO: RTCSdpType::Answer => self.handle_answer_sdp(request_sdp)?,
            _ => {
                return Err(Error::Other(format!(
                    "Unsupported SDP type {}",
                    request_sdp.sdp_type
                )))
            }
        };

        let response_sdp_str =
            serde_json::to_string(&response_sdp).map_err(|err| Error::Other(err.to_string()))?;
        info!("handle_dtls_message: response_sdp {}", response_sdp_str);
        message.payload = BytesMut::from(response_sdp_str.as_str());

        Ok(TaggedMessageEvent {
            now,
            transport: transport_context,
            message: MessageEvent::DTLS(DTLSMessageEvent::APPLICATION(message)),
        })
    }

    fn handle_rtp_message(
        &mut self,
        _now: Instant,
        _transport_context: TransportContext,
        mut _rtp_packet: rtp::packet::Packet,
    ) -> Result<Vec<TaggedMessageEvent>> {
        //TODO: Selective Forwarding RTP Packets
        Ok(vec![])
    }

    fn handle_rtcp_message(
        &mut self,
        _now: Instant,
        _transport_context: TransportContext,
        mut _rtcp_packets: Vec<Box<dyn rtcp::packet::Packet>>,
    ) -> Result<Vec<TaggedMessageEvent>> {
        //TODO: Selective Forwarding RTCP Packets
        Ok(vec![])
    }

    fn check_stun_message(
        &self,
        request: &mut stun::message::Message,
    ) -> Result<Option<Rc<Candidate>>> {
        match TextAttribute::get_from_as(request, ATTR_USERNAME) {
            Ok(username) => {
                if !request.contains(ATTR_PRIORITY) {
                    return Err(Error::Other(
                        "invalid STUN message without ATTR_PRIORITY".to_string(),
                    ));
                }

                if request.contains(ATTR_ICE_CONTROLLING) {
                    if request.contains(ATTR_ICE_CONTROLLED) {
                        return Err(Error::Other("invalid STUN message with both ATTR_ICE_CONTROLLING and ATTR_ICE_CONTROLLED".to_string()));
                    }
                } else if request.contains(ATTR_ICE_CONTROLLED) {
                    if request.contains(ATTR_USE_CANDIDATE) {
                        return Err(Error::Other("invalid STUN message with both ATTR_USE_CANDIDATE and ATTR_ICE_CONTROLLED".to_string()));
                    }
                } else {
                    return Err(Error::Other(
                        "invalid STUN message without ATTR_ICE_CONTROLLING or ATTR_ICE_CONTROLLED"
                            .to_string(),
                    ));
                }

                if let Some(candidate) = self.server_states.find_candidate(&username.text) {
                    let password = candidate.get_local_parameters().password.clone();
                    let integrity = MessageIntegrity::new_short_term_integrity(password);
                    integrity.check(request)?;
                    Ok(Some(candidate))
                } else {
                    Err(Error::Other("username not found".to_string()))
                }
            }
            Err(_) => {
                if request.contains(ATTR_ICE_CONTROLLED)
                    || request.contains(ATTR_ICE_CONTROLLING)
                    || request.contains(ATTR_NETWORK_COST)
                    || request.contains(ATTR_PRIORITY)
                    || request.contains(ATTR_USE_CANDIDATE)
                {
                    Err(Error::Other("unexpected attribute".to_string()))
                } else {
                    Ok(None)
                }
            }
        }
    }

    fn send_server_reflective_address(
        &mut self,
        now: Instant,
        transport_context: TransportContext,
        transaction_id: TransactionId,
    ) -> Result<TaggedMessageEvent> {
        let mut response = stun::message::Message::new();
        response.build(&[
            Box::new(BINDING_SUCCESS),
            Box::new(transaction_id),
            Box::new(XorMappedAddress {
                ip: transport_context.peer_addr.ip(),
                port: transport_context.peer_addr.port(),
            }),
        ])?;

        debug!(
            "send_server_reflective_address response type {} sent",
            response.typ
        );

        Ok(TaggedMessageEvent {
            now,
            transport: transport_context,
            message: MessageEvent::STUN(STUNMessageEvent::STUN(response)),
        })
    }

    #[allow(clippy::type_complexity)]
    fn add_endpoint(
        &mut self,
        request: &stun::message::Message,
        candidate: &Rc<Candidate>,
        transport_context: &TransportContext,
    ) -> Result<(bool, Option<Rc<Endpoint>>)> {
        let mut is_new_endpoint = false;

        let session_id = candidate.session_id();
        let session = self
            .server_states
            .get_session(&session_id)
            .ok_or(Error::Other(format!("session {} not found", session_id)))?;

        let endpoint_id = candidate.endpoint_id();
        let endpoint = session.get_endpoint(&endpoint_id);
        let four_tuple = transport_context.into();
        let has_transport = if let Some(endpoint) = &endpoint {
            endpoint.has_transport(&four_tuple)
        } else {
            is_new_endpoint = true;
            false
        };

        if !request.contains(ATTR_USE_CANDIDATE) || has_transport {
            return Ok((is_new_endpoint, endpoint));
        }

        let (is_new_endpoint, endpoint) = session.add_endpoint(candidate, transport_context)?;

        self.server_states
            .add_endpoint(four_tuple, Rc::clone(&endpoint));

        Ok((is_new_endpoint, Some(endpoint)))
    }
}
