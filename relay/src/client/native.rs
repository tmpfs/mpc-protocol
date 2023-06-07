use futures::{
    select,
    sink::SinkExt,
    stream::{SplitSink, SplitStream},
    FutureExt, StreamExt,
};
use snow::Builder;
use std::{collections::HashMap, sync::Arc};
use tokio::{
    net::TcpStream,
    sync::{mpsc, RwLock},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest, handshake::client::Response,
        protocol::Message,
    },
    MaybeTlsStream, WebSocketStream,
};

use crate::{
    constants::PATTERN, decode, encode, ClientOptions, Error,
    HandshakeType, PeerMessage, ProtocolState, RequestMessage,
    ResponseMessage, Result,
};

type Peers = Arc<RwLock<HashMap<Vec<u8>, ProtocolState>>>;

/// Native websocket client using the tokio tungstenite library.
pub struct NativeClient {
    options: Arc<ClientOptions>,
    server_handshake: mpsc::Receiver<ResponseMessage>,
    outbound_tx: mpsc::Sender<RequestMessage>,
    response: Response,
    state: Option<ProtocolState>,
    peers: Peers,
}

impl NativeClient {
    /// Create a new native client.
    pub async fn new<R>(
        request: R,
        options: ClientOptions,
    ) -> Result<(Self, EventLoop)>
    where
        R: IntoClientRequest + Unpin,
    {
        let (stream, response) = connect_async(request).await?;
        let (ws_writer, ws_reader) = stream.split();

        let builder = Builder::new(PATTERN.parse()?);
        let handshake = builder
            .local_private_key(&options.keypair.private)
            .remote_public_key(&options.server_public_key)
            .build_initiator()?;

        let (outbound_tx, outbound_rx) =
            mpsc::channel::<RequestMessage>(32);
        let (event_loop_tx, event_loop_rx) =
            mpsc::channel::<ResponseMessage>(32);
        let (server_handshake_tx, server_handshake) =
            mpsc::channel::<ResponseMessage>(32);

        let peers = Arc::new(RwLock::new(Default::default()));
        let options = Arc::new(options);
        let client = Self {
            options: Arc::clone(&options),
            server_handshake,
            outbound_tx: outbound_tx.clone(),
            response,
            state: Some(ProtocolState::Handshake(handshake)),
            peers: Arc::clone(&peers),
        };

        let event_loop = EventLoop {
            options,
            ws_reader,
            ws_writer,
            event_loop_tx,
            event_loop_rx,
            outbound_tx,
            outbound_rx,
            server_handshake: server_handshake_tx,
            peers,
        };

        Ok((client, event_loop))
    }

    /// Perform initial handshake with the server.
    pub async fn handshake(&mut self) -> Result<()> {
        let (len, payload) = match &mut self.state {
            Some(ProtocolState::Handshake(initiator)) => {
                let mut request = vec![0u8; 1024];
                let len = initiator.write_message(&[], &mut request)?;
                (len, request)
            }
            _ => return Err(Error::NotHandshakeState),
        };

        let request = RequestMessage::HandshakeInitiator(
            HandshakeType::Server,
            len,
            payload,
        );

        self.outbound_tx.send(request).await?;

        while let Some(response) = self.server_handshake.recv().await {
            let transport = match self.state.take() {
                Some(ProtocolState::Handshake(mut initiator)) => {
                    match response {
                        ResponseMessage::HandshakeResponder(
                            HandshakeType::Server,
                            len,
                            buf,
                        ) => {
                            let mut read_buf = vec![0u8; 1024];
                            initiator.read_message(
                                &buf[..len],
                                &mut read_buf,
                            )?;

                            initiator.into_transport_mode()?
                        }
                        _ => return Err(Error::NotHandshakeReply),
                    }
                }
                _ => return Err(Error::NotHandshakeState),
            };

            self.state = Some(ProtocolState::Transport(transport));

            break;
        }
        Ok(())
    }

    /// Handshake with a peer.
    pub async fn connect_peer(
        &mut self,
        public_key: impl AsRef<[u8]>,
    ) -> Result<()> {
        let mut peers = self.peers.write().await;

        if peers.get(public_key.as_ref()).is_some() {
            return Err(Error::PeerAlreadyExists);
        }

        tracing::debug!(
            to = ?hex::encode(public_key.as_ref()),
            "peer handshake initiator"
        );

        let builder = Builder::new(PATTERN.parse()?);
        let handshake = builder
            .local_private_key(&self.options.keypair.private)
            .remote_public_key(public_key.as_ref())
            .build_initiator()?;
        let peer_state = ProtocolState::Handshake(handshake);

        let state = peers
            .entry(public_key.as_ref().to_vec())
            .or_insert(peer_state);

        let (len, payload) = match state {
            ProtocolState::Handshake(initiator) => {
                let mut request = vec![0u8; 1024];
                let len = initiator.write_message(&[], &mut request)?;
                (len, request)
            }
            _ => return Err(Error::NotHandshakeState),
        };
        drop(peers);

        let inner_request: PeerMessage =
            RequestMessage::HandshakeInitiator(
                HandshakeType::Peer,
                len,
                payload,
            )
            .into();
        let inner_message = encode(&inner_request).await?;

        let request = RequestMessage::RelayPeer {
            public_key: public_key.as_ref().to_vec(),
            message: inner_message,
        };

        self.outbound_tx.send(request).await?;
        Ok(())
    }
}

/// Event loop for a websocket client.
pub struct EventLoop {
    options: Arc<ClientOptions>,
    ws_reader: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    ws_writer:
        SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
    event_loop_tx: mpsc::Sender<ResponseMessage>,
    event_loop_rx: mpsc::Receiver<ResponseMessage>,
    outbound_tx: mpsc::Sender<RequestMessage>,
    outbound_rx: mpsc::Receiver<RequestMessage>,
    server_handshake: mpsc::Sender<ResponseMessage>,
    peers: Peers,
}

impl EventLoop {
    /// Start the client event loop running.
    pub async fn run(mut self) {
        loop {
            select!(
                message_in = self.ws_reader.next().fuse() => match message_in {
                    Some(message) => {
                        match message {
                            Ok(message) => {
                                Self::decode_socket_message(
                                    message,
                                    &mut self.event_loop_tx,
                                    &mut self.server_handshake,
                                ).await;
                            }
                            Err(e) => {
                                tracing::error!("{}", e);
                            }
                        }
                    }
                    _ => {}
                },
                message_out = self.outbound_rx.recv().fuse() => match message_out {
                    Some(message) => {
                        if let Err(e) = self.send_message(message).await {
                            tracing::error!("{}", e);
                        }
                    }
                    _ => {}
                },
                event_message = self.event_loop_rx.recv().fuse() => match event_message {
                    Some(event_message) => {

                        if let Err(e) = self.handle_incoming_message(event_message).await {
                            tracing::error!("{}", e);
                        }
                    }
                    _ => {}
                },
            );
        }
    }

    /// Send a message to the socket and flush the stream.
    async fn send_message(
        &mut self,
        message: RequestMessage,
    ) -> Result<()> {
        let message = Message::Binary(encode(&message).await?);
        self.ws_writer.send(message).await?;
        Ok(self.ws_writer.flush().await?)
    }

    async fn handle_incoming_message(
        &self,
        incoming: ResponseMessage,
    ) -> Result<()> {
        match incoming {
            ResponseMessage::Error(code, message) => {
                tracing::error!("{} {}", code, message);
            }
            ResponseMessage::RelayPeer {
                public_key,
                message,
            } => {
                // Decode the inner message
                match decode::<PeerMessage>(message).await {
                    Ok(relayed) => match relayed {
                        PeerMessage::Request(
                            RequestMessage::HandshakeInitiator(
                                HandshakeType::Peer,
                                len,
                                buf,
                            ),
                        ) => {
                            if let Err(e) = self
                                .peer_handshake_responder(
                                    public_key, len, buf,
                                )
                                .await
                            {
                                tracing::error!("{}", e);
                            }
                        }
                        PeerMessage::Response(
                            ResponseMessage::HandshakeResponder(
                                HandshakeType::Peer,
                                len,
                                buf,
                            ),
                        ) => {
                            if let Err(e) = self
                                .peer_handshake_ack(public_key, len, buf)
                                .await
                            {
                                tracing::error!("{}", e);
                            }
                        }
                        _ => todo!(),
                    },
                    Err(e) => {
                        tracing::error!(
                            "client decode error (inner message) {}",
                            e
                        );
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    async fn peer_handshake_responder(
        &self,
        public_key: impl AsRef<[u8]>,
        len: usize,
        buf: Vec<u8>,
    ) -> Result<()> {
        let mut peers = self.peers.write().await;
        if peers.get(public_key.as_ref()).is_some() {
            return Err(Error::PeerAlreadyExists);
        }

        tracing::debug!(
            from = ?hex::encode(public_key.as_ref()),
            "peer handshake responder"
        );

        let builder = Builder::new(PATTERN.parse()?);
        let mut responder = builder
            .local_private_key(&self.options.keypair.private)
            .remote_public_key(public_key.as_ref())
            .build_responder()?;

        let mut read_buf = vec![0u8; 1024];
        responder.read_message(&buf[..len], &mut read_buf)?;

        let mut payload = vec![0u8; 1024];
        let len = responder.write_message(&[], &mut payload)?;

        let transport = responder.into_transport_mode()?;
        peers.insert(
            public_key.as_ref().to_vec(),
            ProtocolState::Transport(transport),
        );

        let inner_request: PeerMessage =
            ResponseMessage::HandshakeResponder(
                HandshakeType::Peer,
                len,
                payload,
            )
            .into();
        let inner_message = encode(&inner_request).await?;

        let request = RequestMessage::RelayPeer {
            public_key: public_key.as_ref().to_vec(),
            message: inner_message,
        };

        self.outbound_tx.send(request).await?;

        Ok(())
    }

    async fn peer_handshake_ack(
        &self,
        public_key: impl AsRef<[u8]>,
        len: usize,
        buf: Vec<u8>,
    ) -> Result<()> {
        let mut peers = self.peers.write().await;

        let peer = if let Some(peer) = peers.remove(public_key.as_ref()) {
            peer
        } else {
            return Err(Error::PeerNotFound(hex::encode(
                public_key.as_ref().to_vec(),
            )));
        };

        tracing::debug!(
            from = ?hex::encode(public_key.as_ref()),
            "peer handshake ack"
        );

        let transport = match peer {
            ProtocolState::Handshake(mut initiator) => {
                let mut read_buf = vec![0u8; 1024];
                initiator.read_message(&buf[..len], &mut read_buf)?;
                initiator.into_transport_mode()?
            }
            _ => return Err(Error::NotHandshakeState),
        };

        peers.insert(
            public_key.as_ref().to_vec(),
            ProtocolState::Transport(transport),
        );

        Ok(())
    }

    /// Decode socket messages and send to the appropriate channel.
    async fn decode_socket_message(
        incoming: Message,
        event_loop: &mut mpsc::Sender<ResponseMessage>,
        server_handshake: &mut mpsc::Sender<ResponseMessage>,
    ) {
        let buffer = incoming.into_data();
        match decode::<ResponseMessage>(buffer).await {
            Ok(response) => match response {
                ResponseMessage::HandshakeResponder(
                    HandshakeType::Server,
                    _,
                    _,
                ) => {
                    let _ = server_handshake.send(response).await;
                }
                _ => {
                    let _ = event_loop.send(response).await;
                }
            },
            Err(e) => {
                tracing::error!("client decode error {}", e);
            }
        }
    }
}
