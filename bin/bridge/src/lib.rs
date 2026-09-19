use std::error::Error;

use config::BridgeConfig;
// use telegram::Session;
use tokio::task::LocalSet;
use tracing::error;

pub async fn bridge_main(config: &BridgeConfig) -> Result<(), Box<dyn Error + Sync + Send>> {
    // let (session, _event_rx) = Session::new(config).await?;
    // let _command_tx = session.command_tx();

    // let local = LocalSet::new();
    // let join_handle = local.spawn_local(session.listen());

    // local.await;
    // // TODO:
    // // event_rx might receive now that I need to log in, need to shut down etc.

    // if let Err(e) = join_handle.await {
    //     error!("telegram task panicked: {e}")
    // }
    // Ok(())
    todo!()
}
