use retty::channel::{Handler, InboundContext, InboundHandler, OutboundContext, OutboundHandler};
use retty::transport::TransportContext;
use std::cell::RefCell;
use std::net::SocketAddr;
use std::rc::Rc;
use std::time::Instant;

use crate::messages::{DTLSMessageEvent, MessageEvent, TaggedMessageEvent};
use crate::server::states::ServerStates;
use crate::types::FourTuple;
use dtls::endpoint::EndpointEvent;
use dtls::extension::extension_use_srtp::SrtpProtectionProfile;
use dtls::state::State;
use log::{debug, error, warn};
use shared::error::{Error, Result};
use srtp::context::Context;
use srtp::option::{srtcp_replay_protection, srtp_replay_protection};
use srtp::protection_profile::ProtectionProfile;

struct DtlsInbound {
    local_addr: SocketAddr,
    server_states: Rc<RefCell<ServerStates>>,
    dtls_endpoint: Rc<RefCell<dtls::endpoint::Endpoint>>,
}
struct DtlsOutbound {
    local_addr: SocketAddr,
    dtls_endpoint: Rc<RefCell<dtls::endpoint::Endpoint>>,
}
pub struct DtlsHandler {
    dtls_inbound: DtlsInbound,
    dtls_outbound: DtlsOutbound,
}

impl DtlsHandler {
    pub fn new(
        local_addr: SocketAddr,
        server_states: Rc<RefCell<ServerStates>>,
        dtls_handshake_config: dtls::config::HandshakeConfig,
    ) -> Self {
        let dtls_endpoint = Rc::new(RefCell::new(dtls::endpoint::Endpoint::new(Some(
            dtls_handshake_config,
        ))));

        DtlsHandler {
            dtls_inbound: DtlsInbound {
                local_addr,
                server_states,
                dtls_endpoint: Rc::clone(&dtls_endpoint),
            },
            dtls_outbound: DtlsOutbound {
                local_addr,
                dtls_endpoint,
            },
        }
    }
}

impl InboundHandler for DtlsInbound {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;

    fn read(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, msg: Self::Rin) {
        if let MessageEvent::DTLS(DTLSMessageEvent::RAW(dtls_message)) = msg.message {
            debug!("recv dtls RAW {:?}", msg.transport.peer_addr);
            let try_read = || -> Result<Vec<EndpointEvent>> {
                let mut dtls_endpoint = self.dtls_endpoint.borrow_mut();
                let messages = dtls_endpoint.read(
                    msg.now,
                    msg.transport.peer_addr,
                    Some(msg.transport.local_addr.ip()),
                    msg.transport.ecn,
                    dtls_message,
                )?;
                Ok(messages)
            };
            match try_read() {
                Ok(messages) => {
                    for message in messages {
                        match message {
                            EndpointEvent::HandshakeComplete => {
                                debug!("recv dtls handshake complete");
                                let dtls_endpoint = self.dtls_endpoint.borrow_mut();
                                if let Some(state) =
                                    dtls_endpoint.get_connection_state(msg.transport.peer_addr)
                                {
                                    if let Err(err) =
                                        self.update_srtp_contexts(state, (&msg.transport).into())
                                    {
                                        error!("try_read with error {}", err);
                                        ctx.fire_read_exception(Box::new(err));
                                    }
                                } else {
                                    warn!(
                                        "Unable to find connection state for {}",
                                        msg.transport.peer_addr
                                    );
                                }
                            }
                            EndpointEvent::ApplicationData(message) => {
                                debug!("recv dtls application RAW {:?}", msg.transport.peer_addr);
                                ctx.fire_read(TaggedMessageEvent {
                                    now: msg.now,
                                    transport: msg.transport,
                                    message: MessageEvent::DTLS(DTLSMessageEvent::RAW(message)),
                                });
                            }
                        }
                    }
                }
                Err(err) => {
                    error!("try_read with error {}", err);
                    ctx.fire_read_exception(Box::new(err))
                }
            };
            handle_outgoing(ctx, &self.dtls_endpoint, msg.transport.local_addr);
        } else {
            // Bypass
            debug!("bypass dtls read {:?}", msg.transport.peer_addr);
            ctx.fire_read(msg);
        }
    }

    fn handle_timeout(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, now: Instant) {
        let try_timeout = || -> Result<()> {
            let mut dtls_endpoint = self.dtls_endpoint.borrow_mut();
            let remotes: Vec<SocketAddr> = dtls_endpoint.get_connections_keys().copied().collect();
            for remote in remotes {
                let _ = dtls_endpoint.handle_timeout(remote, now);
                //TODO: timeout errors
            }
            Ok(())
        };
        if let Err(err) = try_timeout() {
            error!("try_timeout with error {}", err);
            ctx.fire_read_exception(Box::new(err));
        }
        handle_outgoing(ctx, &self.dtls_endpoint, self.local_addr);

        ctx.fire_handle_timeout(now);
    }

    fn poll_timeout(&mut self, ctx: &InboundContext<Self::Rin, Self::Rout>, eto: &mut Instant) {
        {
            let dtls_endpoint = self.dtls_endpoint.borrow();
            let remotes = dtls_endpoint.get_connections_keys();
            for remote in remotes {
                let _ = dtls_endpoint.poll_timeout(*remote, eto);
            }
        }
        ctx.fire_poll_timeout(eto);
    }
}

impl OutboundHandler for DtlsOutbound {
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn write(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>, msg: Self::Win) {
        if let MessageEvent::DTLS(DTLSMessageEvent::RAW(dtls_message)) = msg.message {
            debug!("send dtls RAW {:?}", msg.transport.peer_addr);
            let try_write = || -> Result<()> {
                let mut dtls_endpoint = self.dtls_endpoint.borrow_mut();
                dtls_endpoint.write(msg.transport.peer_addr, &dtls_message)
            };
            if let Err(err) = try_write() {
                error!("try_write with error {}", err);
                ctx.fire_write_exception(Box::new(err));
            }
            handle_outgoing(ctx, &self.dtls_endpoint, msg.transport.local_addr);
        } else {
            // Bypass
            debug!("Bypass dtls write {:?}", msg.transport.peer_addr);
            ctx.fire_write(msg);
        }
    }

    fn close(&mut self, ctx: &OutboundContext<Self::Win, Self::Wout>) {
        {
            let mut dtls_endpoint = self.dtls_endpoint.borrow_mut();
            let remotes: Vec<SocketAddr> = dtls_endpoint.get_connections_keys().copied().collect();
            for remote in remotes {
                let _ = dtls_endpoint.close(remote);
            }
        }
        handle_outgoing(ctx, &self.dtls_endpoint, self.local_addr);

        ctx.fire_close();
    }
}

impl Handler for DtlsHandler {
    type Rin = TaggedMessageEvent;
    type Rout = Self::Rin;
    type Win = TaggedMessageEvent;
    type Wout = Self::Win;

    fn name(&self) -> &str {
        "DtlsHandler"
    }

    fn split(
        self,
    ) -> (
        Box<dyn InboundHandler<Rin = Self::Rin, Rout = Self::Rout>>,
        Box<dyn OutboundHandler<Win = Self::Win, Wout = Self::Wout>>,
    ) {
        (Box::new(self.dtls_inbound), Box::new(self.dtls_outbound))
    }
}

fn handle_outgoing(
    ctx: &OutboundContext<TaggedMessageEvent, TaggedMessageEvent>,
    dtls_endpoint: &Rc<RefCell<dtls::endpoint::Endpoint>>,
    local_addr: SocketAddr,
) {
    let mut transmits = vec![];
    {
        let mut e = dtls_endpoint.borrow_mut();
        while let Some(transmit) = e.poll_transmit() {
            transmits.push(transmit);
        }
    };
    for transmit in transmits {
        ctx.fire_write(TaggedMessageEvent {
            now: transmit.now,
            transport: TransportContext {
                local_addr,
                peer_addr: transmit.remote,
                ecn: transmit.ecn,
            },
            message: MessageEvent::DTLS(DTLSMessageEvent::RAW(transmit.payload)),
        });
    }
}

impl DtlsInbound {
    const DEFAULT_SESSION_SRTP_REPLAY_PROTECTION_WINDOW: usize = 64;
    const DEFAULT_SESSION_SRTCP_REPLAY_PROTECTION_WINDOW: usize = 64;
    fn update_srtp_contexts(&self, state: &State, four_tuple: FourTuple) -> Result<()> {
        let profile = match state.srtp_protection_profile() {
            SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80 => {
                ProtectionProfile::Aes128CmHmacSha1_80
            }
            SrtpProtectionProfile::Srtp_Aead_Aes_128_Gcm => ProtectionProfile::AeadAes128Gcm,
            _ => return Err(Error::ErrNoSuchSrtpProfile),
        };

        let mut srtp_config = srtp::config::Config {
            profile,
            ..Default::default()
        };
        /*if self.setting_engine.replay_protection.srtp != 0 {
            srtp_config.remote_rtp_options = Some(srtp::option::srtp_replay_protection(
                self.setting_engine.replay_protection.srtp,
            ));
        } else if self.setting_engine.disable_srtp_replay_protection {
            srtp_config.remote_rtp_options = Some(srtp::option::srtp_no_replay_protection());
        }*/

        srtp_config.extract_session_keys_from_dtls(state, false)?;

        let local_context = Context::new(
            &srtp_config.keys.local_master_key,
            &srtp_config.keys.local_master_salt,
            srtp_config.profile,
            srtp_config.local_rtp_options,
            srtp_config.local_rtcp_options,
        )?;

        let remote_context = Context::new(
            &srtp_config.keys.remote_master_key,
            &srtp_config.keys.remote_master_salt,
            srtp_config.profile,
            if srtp_config.remote_rtp_options.is_none() {
                Some(srtp_replay_protection(
                    Self::DEFAULT_SESSION_SRTP_REPLAY_PROTECTION_WINDOW,
                ))
            } else {
                srtp_config.remote_rtp_options
            },
            if srtp_config.remote_rtcp_options.is_none() {
                Some(srtcp_replay_protection(
                    Self::DEFAULT_SESSION_SRTCP_REPLAY_PROTECTION_WINDOW,
                ))
            } else {
                srtp_config.remote_rtcp_options
            },
        )?;

        let mut server_states = self.server_states.borrow_mut();
        match server_states.get_mut_transport(&four_tuple) {
            Ok(transport) => {
                transport.set_local_srtp_context(local_context);
                transport.set_remote_srtp_context(remote_context);
            }
            Err(err) => {
                warn!("can't find transport with {:?} due to {}", four_tuple, err);
            }
        }

        Ok(())
    }
}
