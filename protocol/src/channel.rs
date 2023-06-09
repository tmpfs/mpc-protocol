//! Utility functions for the encrypted server channel.
//!
//! You should not use these functions directly, they are
//! exposed so they can be shared between the client and server.
use crate::{
    decode, encode, Encoding, Error, ProtocolState, RequestMessage,
    ResponseMessage, Result, SealedEnvelope, TAGLEN,
};

/// Encrypt a message to send to the server.
///
/// The protocol must be in transport mode.
#[doc(hidden)]
pub async fn encrypt_server_channel(
    server: &mut ProtocolState,
    payload: Vec<u8>,
) -> Result<Vec<u8>> {
    match server {
        ProtocolState::Transport(transport) => {
            let mut contents = vec![0; payload.len() + TAGLEN];
            let length =
                transport.write_message(&payload, &mut contents)?;
            let envelope = SealedEnvelope {
                length,
                encoding: Encoding::Blob,
                payload: contents,
            };
            Ok(encode(&envelope).await?)
        }
        _ => Err(Error::NotTransportState),
    }
}

/// Decrypt a message received from the server.
///
/// The protocol must be in transport mode.
#[doc(hidden)]
pub async fn decrypt_server_channel(
    server: &mut ProtocolState,
    payload: Vec<u8>,
) -> Result<(Encoding, Vec<u8>)> {
    match server {
        ProtocolState::Transport(transport) => {
            let envelope: SealedEnvelope = decode(&payload).await?;
            let mut contents = vec![0; envelope.length];
            transport.read_message(
                &envelope.payload[..envelope.length],
                &mut contents,
            )?;
            let new_length = contents.len() - TAGLEN;
            contents.truncate(new_length);
            Ok((envelope.encoding, contents))
        }
        _ => Err(Error::NotTransportState),
    }
}
