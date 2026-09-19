use bridge::bridge_main;
use clap::{Parser, Subcommand};
use telegram::login;
use tracing::{error, info, level_filters::LevelFilter};
use tracing_subscriber::{Layer, layer::SubscriberExt, reload, util::SubscriberInitExt};

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
    let base_filter_level = LevelFilter::DEBUG;
    let (filter, reload_handle) = reload::Layer::new(base_filter_level);

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter))
        .try_init()?;

    let args = BinArgs::parse();

    let bridge_config = match config::load("bridge_config.toml") {
        Ok(config) => config,
        Err(e) => {
            error!(error = %e, "failed to load bridge config on startup");
            std::process::exit(1);
        }
    };

    let new_level: LevelFilter = bridge_config.log_level.parse().unwrap_or(LevelFilter::INFO);

    if base_filter_level != new_level {
        info!("log level changing: {} -> {}", base_filter_level, new_level);
        reload_handle.modify(|f| *f = new_level)?;
    }

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
