use std::{collections::HashMap, sync::Arc};

use anyhow::{Result, bail};
use asterisk_ari::{
    AriClient, AriError,
    apis::{
        bridges::{
            models::BridgeType,
            params::{AddChannelRequest, CreateRequest},
        },
        channels::params::{
            ContinueRequest, DeleteRequest, Encapsulation, ExternalMediaRequest, Transport,
        },
        client::Client,
    },
    ws::models::StasisStart,
};
use config::AsteriskConfig;
use tokio::sync::{
    Mutex,
    mpsc::{self, Sender},
};
use tracing::{debug, info, warn};

use crate::{
    ChannelId, start_audiosocket_listener,
    types::{AsteriskCommand, AsteriskEvent, ExternalMediaId},
};

pub struct AsteriskClient {
    client: AriClient,
    config: AsteriskConfig,
    cmd_rx: mpsc::Receiver<AsteriskCommand>,
    event_tx: mpsc::Sender<AsteriskEvent>,
    channel_hash: Arc<Mutex<HashMap<ChannelId, ChannelId>>>,
}

pub struct AsteriskHandler {
    pub cmd_tx: mpsc::Sender<AsteriskCommand>,
    pub event_rx: mpsc::Receiver<AsteriskEvent>,
}

impl AsteriskClient {
    pub fn init(config: AsteriskConfig) -> (AsteriskClient, AsteriskHandler) {
        let config::AsteriskConfig {
            ari_url,
            ari_user,
            ari_password,
            ..
        } = &config;

        let ari_config = asterisk_ari::Config::new(ari_url, ari_user, ari_password);
        let client = asterisk_ari::AriClient::with_config(ari_config);

        let (cmd_tx, cmd_rx) = mpsc::channel(200); // TODO: consider a field in config
        let (event_tx, event_rx) = mpsc::channel(200);
        let channel_hash = Arc::new(Mutex::new(HashMap::new()));

        let client = AsteriskClient {
            client,
            config,
            cmd_rx,
            event_tx,
            channel_hash,
        };

        let handler = AsteriskHandler { cmd_tx, event_rx };

        (client, handler)
    }

    pub async fn run(&mut self) -> Result<()> {
        let event_tx = self.event_tx.clone();
        let app_name = self.config.ari_app_name.clone();
        let audio_format = self.config.audio_format.clone();
        let channel_hash = self.channel_hash.clone();

        self.client.on_stasis_start(move |client, event| {
            let event_tx = event_tx.clone();
            let app_name = app_name.clone();
            let audio_format = audio_format.clone();
            let channel_hash = channel_hash.clone();

            async move {
                let stasis_event = event.data;

                match stasis_event.channel.name.split_once("/") {
                    Some(("PJSIP", _)) => handle_sip_connection(
                        stasis_event,
                        client,
                        &app_name,
                        &audio_format,
                        event_tx,
                        channel_hash,
                    )
                    .await
                    .map_err(|e| {
                        AriError::Internal(format!("Error when handling sip connection: {e}"))
                    }),
                    Some(("AudioSocket", _)) => handle_external_media(stasis_event, client)
                        .await
                        .map_err(|e| {
                            AriError::Internal(format!(
                                "Error when handling external media connection: {e}"
                            ))
                        }),
                    Some((prefix, _)) => {
                        info!("unknown stasis start event type: {prefix}");
                        Ok(())
                    }
                    None => {
                        info!("bad channel name string: {}", stasis_event.channel.name);
                        Err(AriError::Internal(format!(
                            "bad channel name string: {}",
                            stasis_event.channel.name
                        )))
                    }
                }
            }
        });

        let event_tx = self.event_tx.clone();
        let channel_hash = self.channel_hash.clone();
        self.client.on_channel_destroyed(move |client, event| {
            let event_tx = event_tx.clone();
            let channel_hash = channel_hash.clone();
            async move {
                let channel_id = event.data.channel.id.clone();
                debug!(
                    channel_id,
                    "channel destroyed because of: {}", event.data.cause_txt
                );

                let ext_media_chan = {
                    let mut guard = channel_hash.lock().await;
                    guard.remove(&channel_id.into())
                };

                // TODO: check all edge cases, no connection, two connections, etc.
                // do we _ALWAYS_ have a ext media chan?
                match ext_media_chan {
                    None => {
                        debug!("no media channel for {}", event.data.channel.id);
                        Ok(())
                    }

                    Some(id) => {
                        let req = DeleteRequest::new(id);
                        let req = req.with_reason(
                            asterisk_ari::apis::channels::params::CancelReason::Normal,
                        );
                        client.channels().delete(req).await?;
                        event_tx.send(AsteriskEvent::CallEnded).await.map_err(|e| {
                            AriError::Internal(format!("when sending call ended event: {e}"))
                        })
                    }
                }
            }
        });

        self.client.start(self.config.ari_app_name.clone()).await?;

        loop {
            tokio::select! {
                Some(cmd) = self.cmd_rx.recv() => {
                    match cmd {
                        AsteriskCommand::Shutdown => {
                            info!("shutdown command recieved");
                            self.client.stop().await?;
                            break;
                        },
                        AsteriskCommand::Hangup(channel_id) => {
                            info!("Hangup command recieved");
                            // TODO: what to do if no call is ongoing?
                            self.event_tx.send(AsteriskEvent::CallEnded).await?;
                            self.client.channels().r#continue(ContinueRequest::new(channel_id)).await?;
                        },
                        AsteriskCommand::AcceptCall(channel_id) => {
                            info!("AcceptCall command recieved");
                            // TODO: what to do if no call is ongoing?
                            self.client.channels().answer(channel_id).await?;
                        },
                        AsteriskCommand::IncomingCall => todo!(),
                    }
                }
            }
        }

        Ok(())
    }
}

async fn handle_sip_connection(
    event: StasisStart,
    client: Arc<Client>,
    app_name: &str,
    audio_format: &str,
    event_tx: Sender<AsteriskEvent>,
    channel_hash: Arc<Mutex<HashMap<ChannelId, ChannelId>>>,
) -> Result<()> {
    let Some(raw) = event.args.first() else {
        warn!("StasisStart missing call_num arg");
        bail!("missing call_num arg");
    };

    let call_num = match raw.parse::<u8>() {
        Ok(n) => n,
        Err(e) => {
            warn!("Failed to parse call_num: {e}");
            bail!("call_num parsing error");
        }
    };

    let channel_id = event.channel.id;

    client.channels().ring(&channel_id).await?;

    let (port, audio_rx) = start_audiosocket_listener().await?;
    info!("AudioSocket listener ready on port {port}");

    let mut req =
                    // TODO: maybe use a crate to get the proper Lan IP address o maybe DNS it
                    // ExternalMediaRequest::new(app_name, format!("127.0.0.1:{port}"), audio_format); TODO: maybe move pbx to it's own host now
                    ExternalMediaRequest::new(app_name, format!("192.168.1.131:{port}"), audio_format);

    // TODO add channel ID information somewhere for internal timeout to hang up if call not answered.
    // all the info should be in a call-info struct CallSession or something similar
    req = req.with_encapsulation(Encapsulation::AudioSocket);
    req = req.with_transport(Transport::Tcp);
    let external_media_id = ExternalMediaId::new();
    req = req.with_data(format!("{external_media_id}"));

    // TODO: create bridge + add current channel to bridge
    let mut bridge_req = CreateRequest::new();
    bridge_req = bridge_req.with_type(BridgeType::Mixing);

    let bridge = client.bridges().create(bridge_req).await?;
    let bridge = Box::new(bridge);

    let ext_media_channel = client.channels().external_media(req).await?;

    client
        .bridges()
        .add_channel(AddChannelRequest::new(&bridge.id, &ext_media_channel.id))
        .await?;
    // make sure we handle StasisEnd/ChannelHangupRequest as ringing is going on as well
    client
        .bridges()
        .add_channel(AddChannelRequest::new(&bridge.id, &channel_id))
        .await?;

    {
        let mut guard = channel_hash.lock().await;
        guard.insert(channel_id.clone().into(), ext_media_channel.id.into());
    }

    let _ = event_tx
        .send(AsteriskEvent::CallRequested {
            channel_id: channel_id.into(),
            call_num,
            audio_rx,
            bridge,
        })
        .await;

    Ok(())
}

async fn handle_external_media(event: StasisStart, _client: Arc<Client>) -> Result<()> {
    let channel_id = event.channel.id;
    let args = event.args;
    let name = event.channel.name;

    info!(
        channel_id,
        "external media stasis request, name: {}, args: {:?}", name, args
    );
    Ok(())
}

// async fn handle_channel_hangup(event: ChannelStateChange)
