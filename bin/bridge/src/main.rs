use anyhow::Result;
use bridge::run_connections;
use clap::{Parser, Subcommand};
use config::BridgeConfig;
use telegram::login;
use tracing::{error, info};

#[derive(Parser)]
#[command(name = "bridge")]
#[command(about = "Bridges SIP into different protocols")]
#[command(version)]
#[command(subcommand_required = false)]
struct BinArgs {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    RunBridge,
    Login,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync + 'static>> {
    config::config_tracing().await?;

    let args = BinArgs::parse();

    let bridge_config = match config::load("bridge_config.toml") {
        Ok(config) => config,
        Err(e) => {
            error!(error = %e, "failed to load bridge config on startup");
            std::process::exit(1);
        }
    };

    match args.command.unwrap_or(Commands::RunBridge) {
        Commands::RunBridge => {
            info!("running bridge");
            match bridge_main(&bridge_config).await {
                Ok(_) => info!("shutting down, goodbye"),
                Err(err) => {
                    error!("error: {err}");
                    error!("main bridge returned an error");
                }
            };
        }

        Commands::Login => {
            info!("attempting login as {}", bridge_config.telegram.phone_num);
            match login(&bridge_config).await {
                Ok(_) => info!("logged in successful"),
                Err(err) => {
                    error!("error: {err}");
                    error!("Login unsucessful, try again")
                }
            };
        }
    }
    Ok(())
}

async fn bridge_main(config: &BridgeConfig) -> Result<()> {
    let (task_handlers, actor_handlers) = run_connections(config).await?;

    let asterisk_client_task = task_handlers.asterisk;
    let telegram_client_task = task_handlers.telegram;

    let (asterisk_result, tg_result) = tokio::join!(asterisk_client_task, telegram_client_task,);

    asterisk_result??;
    tg_result?;
    Ok(())
}
