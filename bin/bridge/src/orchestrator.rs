use std::thread;

use anyhow::Result;
use asterisk::AsteriskClient;
use config::BridgeConfig;
use tracing::error;

use crate::types::{ActorHandlers, ActorTaskHandlers};

pub async fn run_connections(config: &BridgeConfig) -> Result<(ActorTaskHandlers, ActorHandlers)> {
    let (tg_handler, tg_actor) = telegram::spawn_client(config).await?;

    let tg_thread = thread::spawn(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build Telegram runtime"); // TODO: dont panic, signal back for a shutdown

        let local = tokio::task::LocalSet::new();

        rt.block_on(local.run_until(async {
            tokio::task::spawn_local(async move {
                if let Err(e) = tg_actor.run().await {
                    error!("[telegram] actor died: {e:#}");
                }
            })
            .await
            .expect("actor.run panicked"); // TODO: handle differently or bind to a handler if needed
        }))
    });

    let telegram_client_task = tokio::task::spawn_blocking(move || {
        tg_thread.join().expect("telegram thread panicked");
    });

    let asterisk_config = config.asterisk.clone();
    let (mut asterisk_client, asterisk_handler) = AsteriskClient::init(asterisk_config);

    let asterisk_client_task = tokio::spawn(async move {
        let result = asterisk_client.run().await;
        if let Err(e) = &result {
            error!("asterisk client.run() failed: {e}");
        }
        result
    });

    let task_handlers = ActorTaskHandlers {
        asterisk: asterisk_client_task,
        telegram: telegram_client_task,
    };

    let actor_handlers = ActorHandlers::new(asterisk_handler, tg_handler);

    Ok((task_handlers, actor_handlers))
}
