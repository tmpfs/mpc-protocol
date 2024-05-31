use anyhow::Result;
use futures::{select, FutureExt, StreamExt};
use std::collections::HashMap;

use mpc_driver::{
    cggmp::KeyGenDriver,
    k256::{
        self,
        ecdsa::{SigningKey, VerifyingKey},
    },
    synedrion::{KeyShare, TestParams},
    Driver, SessionEventHandler, SessionInitiator,
    SessionParticipant,
};

use crate::test_utils::new_client;
use mpc_client::{NetworkTransport, Transport};
use mpc_protocol::{Keypair, SessionState};
use rand::{rngs::OsRng, Rng};

type KeyShareOutput = KeyShare<TestParams, VerifyingKey>;

pub async fn run_keygen(
    server: &str,
    server_public_key: Vec<u8>,
) -> Result<()> {
    let n = 3;

    let rng = &mut OsRng;
    let shared_randomness: [u8; 32] = rng.gen();
    let mut signing_keys = Vec::new();
    for _ in 0..n {
        signing_keys.push(k256::ecdsa::SigningKey::random(rng));
    }

    let (key_shares, _transport_keypairs) = cggmp_keygen(
        server,
        server_public_key.clone(),
        &shared_randomness,
        signing_keys.clone(),
    )
    .await?;

    assert_eq!(3, key_shares.len());

    Ok(())
}

/// Create a new session and then perform distributed key generation.
async fn cggmp_keygen(
    server: &str,
    server_public_key: Vec<u8>,
    shared_randomness: &[u8],
    mut signing_keys: Vec<SigningKey>,
) -> Result<(HashMap<Vec<u8>, KeyShareOutput>, Vec<Keypair>)> {
    let verifiers: Vec<VerifyingKey> = signing_keys
        .iter()
        .map(|k| k.verifying_key().clone())
        .collect();
    let signing_key_1 = signing_keys.remove(0);
    let signing_key_2 = signing_keys.remove(0);
    let signing_key_3 = signing_keys.remove(0);

    let mut sessions: Vec<SessionState> = Vec::new();

    // Create new clients
    let (client_i, event_loop_i, initiator_key) =
        new_client::<anyhow::Error>(
            server,
            server_public_key.clone(),
        )
        .await?;
    let (client_p_1, event_loop_p_1, participant_key_1) =
        new_client::<anyhow::Error>(
            server,
            server_public_key.clone(),
        )
        .await?;

    let (client_p_2, event_loop_p_2, participant_key_2) =
        new_client::<anyhow::Error>(
            server,
            server_public_key.clone(),
        )
        .await?;

    let keypairs = vec![
        initiator_key.clone(),
        participant_key_1.clone(),
        participant_key_2.clone(),
    ];

    let mut client_i_transport: Transport = client_i.into();
    let mut client_p_1_transport: Transport = client_p_1.into();
    let mut client_p_2_transport: Transport = client_p_2.into();

    // Each client handshakes with the server
    client_i_transport.connect().await?;
    client_p_1_transport.connect().await?;
    client_p_2_transport.connect().await?;

    // Event loop streams
    let mut s_i = event_loop_i.run();
    let mut s_p_1 = event_loop_p_1.run();
    let mut s_p_2 = event_loop_p_2.run();

    let session_participants = vec![
        participant_key_1.public_key().to_vec(),
        participant_key_2.public_key().to_vec(),
    ];

    let mut client_i_session = SessionInitiator::new(
        client_i_transport,
        session_participants,
    );
    let mut client_p_1_session =
        SessionParticipant::new(client_p_1_transport);

    let mut client_p_2_session =
        SessionParticipant::new(client_p_2_transport);

    // Prepare the sessions for each party
    loop {
        if sessions.len() == 3 {
            break;
        }

        select! {
            event = s_i.next().fuse() => {
                match event {
                    Some(event) => {
                        let event = event?;

                        if let Some(session) =
                            client_i_session.handle_event(event).await? {
                            sessions.push(session);
                        }
                    }
                    _ => {}
                }
            },
            event = s_p_1.next().fuse() => {
                match event {
                    Some(event) => {
                        let event = event?;
                        if let Some(session) =
                            client_p_1_session.handle_event(event).await? {
                            sessions.push(session);
                        }
                    }
                    _ => {}
                }
            },
            event = s_p_2.next().fuse() => {
                match event {
                    Some(event) => {
                        let event = event?;
                        if let Some(session) =
                            client_p_2_session.handle_event(event).await? {
                            sessions.push(session);
                        }
                    }
                    _ => {}
                }
            },
        }
    }

    // Prepare for key generation
    let client_i_transport: Transport = client_i_session.into();
    let client_p_1_transport: Transport = client_p_1_session.into();
    let client_p_2_transport: Transport = client_p_2_session.into();

    let session_i = sessions.remove(0);
    let session_p_1 = sessions.remove(0);
    let session_p_2 = sessions.remove(0);

    let mut keygen_i = KeyGenDriver::<TestParams>::new(
        client_i_transport.clone(),
        session_i,
        shared_randomness,
        signing_key_1,
        verifiers.clone(),
    )?;
    let mut keygen_p_1 = KeyGenDriver::<TestParams>::new(
        client_p_1_transport.clone(),
        session_p_1,
        shared_randomness,
        signing_key_2,
        verifiers.clone(),
    )?;
    let mut keygen_p_2 = KeyGenDriver::<TestParams>::new(
        client_p_2_transport.clone(),
        session_p_2,
        shared_randomness,
        signing_key_3,
        verifiers.clone(),
    )?;

    // Each party starts key generation protocol.
    keygen_i.execute().await?;
    keygen_p_1.execute().await?;
    keygen_p_2.execute().await?;

    let mut key_shares: HashMap<Vec<u8>, KeyShareOutput> =
        HashMap::new();

    loop {
        if key_shares.len() == 3 {
            break;
        }
        select! {
            event = s_i.next().fuse() => {
                match event {
                    Some(event) => {
                        let event = event?;
                        if let Some(key_share) =
                            keygen_i.handle_event(event).await? {
                            key_shares.insert(
                                client_i_transport.public_key().to_vec(),
                                key_share.0);
                        }
                    }
                    _ => {}
                }
            },
            event = s_p_1.next().fuse() => {
                match event {
                    Some(event) => {
                        let event = event?;
                        if let Some(key_share) =
                            keygen_p_1.handle_event(event).await? {
                            key_shares.insert(
                                client_p_1_transport.public_key().to_vec(),
                                key_share.0);
                        }
                    }
                    _ => {}
                }
            },
            event = s_p_2.next().fuse() => {
                match event {
                    Some(event) => {
                        let event = event?;
                        if let Some(key_share) =
                            keygen_p_2.handle_event(event).await? {
                            key_shares.insert(
                                client_p_2_transport.public_key().to_vec(),
                                key_share.0);
                        }
                    }
                    _ => {}
                }
            },
        }
    }

    Ok((key_shares, keypairs))
}
