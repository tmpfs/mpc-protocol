use crate::{RequestMessage, ResponseMessage};
use axum::http::StatusCode;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// Error generated when a file is expected.
    #[error(r#"not a file "{0}""#)]
    NotFile(PathBuf),

    /// Error generated by the server in response to a client request.
    #[error("{0} {1}")]
    ServerError(StatusCode, String),

    /// Error generated when the config server key file was not specified.
    #[error("server config requires path to a key file")]
    KeyFileRequired,

    /// Error generated if the client expects a reply but none was received.
    #[error("server did not reply")]
    NoReply,

    /// Error generated if the client read loop cannot be taken.
    #[error("client read loop does not exists, probably already taken")]
    ClientReadLoopAlreadyTaken,

    /// Error generated when the config server key file was not found.
    #[error(r#"key file "{0}" not found"#)]
    KeyNotFound(PathBuf),

    /// Error generated attempting to handshake with a peer that
    /// already exists.
    #[error("peer already exists")]
    PeerAlreadyExists,

    /// Error generated when a peer could not be found.
    #[error(r#"peer "{0}" not found "#)]
    PeerNotFound(String),

    /// Error generated when the PEM encoding does not match
    /// the expected format.
    #[error("encoding in PEM is invalid")]
    BadKeypairPem,

    /// Error generated when a file does not have a parent directory.
    #[error("no parent directory")]
    NoParentDir,

    /// Error generated when a client request does not receive a binary
    /// reply message.
    #[error("binary reply expected")]
    BinaryReplyExpected,

    /// Error generated when a participant expects to be in the handshake
    /// protocol state.
    #[error("not handshake protocol state")]
    NotHandshakeState,

    /// Error generated when the server replies with a message other than
    /// the expected handshake response.
    #[error("not handshake reply")]
    NotHandshakeReply,

    /// Error generated decoding the kind for an encoding is invalid.
    #[error("invalid encoding kind identifier {0}")]
    EncodingKind(u8),

    /// Error generated by input/output.
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Error generated by the web server library.
    #[error(transparent)]
    Axum(#[from] axum::Error),

    /// Error generated by the noise protocol library.
    #[error(transparent)]
    Snow(#[from] snow::error::Error),

    /// Error generated serializing or deserializing JSON.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// Error generated parsing TOML.
    #[error(transparent)]
    Toml(#[from] toml::de::Error),

    /// Error generated when a header value is invalid.
    #[error(transparent)]
    HeaderValue(#[from] axum::http::header::InvalidHeaderValue),

    /// Error generated decoding PEM data.
    #[error(transparent)]
    Pem(#[from] pem::PemError),

    /// Error generated by the client websocket library.
    #[error(transparent)]
    Websocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error(transparent)]
    MpscSend(#[from] tokio::sync::mpsc::error::SendError<Vec<u8>>),

    #[error(transparent)]
    RequestMpscSend(
        #[from] tokio::sync::mpsc::error::SendError<RequestMessage>,
    ),

    #[error(transparent)]
    ResponseMpscSend(
        #[from] tokio::sync::mpsc::error::SendError<ResponseMessage>,
    ),

    #[error(transparent)]
    BroadcastSend(
        #[from] tokio::sync::broadcast::error::SendError<Vec<u8>>,
    ),
}
