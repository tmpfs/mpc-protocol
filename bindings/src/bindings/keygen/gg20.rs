//! Distributed key generation for the GG20 protocol.
use wasm_bindgen::prelude::*;

use crate::{new_client_with_keypair, SessionOptions};
use mpc_client::{NetworkTransport, Transport};
use mpc_driver::{
    gg20::KeyGenDriver, wait_for_driver, wait_for_session,
    SessionHandler, SessionInitiator, SessionParticipant,
};

pub(crate) async fn keygen(
    options: SessionOptions,
    participants: Option<Vec<Vec<u8>>>,
) -> Result<JsValue, JsValue> {
    let is_initiator = participants.is_some();

    log::info!("keygen starting {}", is_initiator);

    // Create the client
    let (client, event_loop) = new_client_with_keypair(
        &options.server.server_url,
        options.server.server_public_key.clone(),
        options.keypair.clone(),
    )
    .await?;

    let mut transport: Transport = client.into();

    // Handshake with the server
    transport.connect().await?;

    log::info!("keygen connected to server, prepare session");

    // Start the event stream
    let mut stream = event_loop.run();

    // Wait for the session to become active
    let client_session = if let Some(participants) = participants {
        SessionHandler::Initiator(SessionInitiator::new(
            transport,
            participants,
            Some(options.session_id),
        ))
    } else {
        SessionHandler::Participant(SessionParticipant::new(
            transport,
        ))
    };

    let (transport, session) =
        wait_for_session(&mut stream, client_session).await?;

    log::info!("keygen, session is ready");

    let session_id = session.session_id;

    // Wait for key generation
    let keygen = KeyGenDriver::new(
        transport,
        options.parameters.clone(),
        session,
    )?;
    let (mut transport, key_share) =
        wait_for_driver(&mut stream, keygen).await?;

    // Close the session and socket
    if is_initiator {
        transport.close_session(session_id).await?;
    }
    transport.close().await?;

    Ok(serde_wasm_bindgen::to_value(&key_share)?)
}
