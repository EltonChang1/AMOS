use std::{net::IpAddr, path::PathBuf};

use clap::{Parser, Subcommand};

use amos::{AmosError, AmosRuntime, Result, RuntimeConfig, api, deployment::ServerConfig, seed};

#[derive(Parser)]
#[command(
    name = "amos",
    version,
    about = "Rust-native AMOS memory operating layer"
)]
struct Cli {
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long, env = "AMOS_DEMO")]
    demo: bool,
    #[arg(long, env = "AMOS_CONFIG")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    Seed,
    Serve {
        #[arg(long)]
        port: Option<u16>,
        #[arg(long)]
        seed_demo: bool,
    },
    Run {
        #[arg(long)]
        request: String,
        #[arg(long, default_value = "analyst_001")]
        identity: String,
        #[arg(long)]
        idempotency_key: String,
    },
    Replay {
        artifact_id: String,
        #[arg(long, default_value = "analyst_001")]
        identity: String,
        #[arg(long)]
        idempotency_key: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let cli = Cli::parse();
    if cli.demo && cli.config.is_some() {
        return Err(AmosError::Validation(
            "--demo and --config are mutually exclusive".into(),
        ));
    }
    if cli.demo {
        return run_demo(cli).await;
    }
    if let Some(config_path) = cli.config.clone() {
        return run_configured(cli, config_path).await;
    }
    Err(AmosError::Validation(
        "the bundled binary has no production identity provider or implicit production configuration; use --demo only for the explicit local demo, or provide a reviewed --config for customer-evaluation mode"
            .into(),
    ))
}

async fn run_demo(cli: Cli) -> Result<()> {
    let config = RuntimeConfig::demo(&cli.root);
    match cli.command.unwrap_or(Command::Serve {
        port: None,
        seed_demo: false,
    }) {
        Command::Seed => {
            let store = amos::store::Store::open(&config.control_db)?;
            seed::seed_demo(&store, &config.warehouse_db)?;
            println!("Seeded Rust AMOS demo under {}", cli.root.display());
        }
        Command::Serve { port, seed_demo } => {
            if seed_demo || !config.warehouse_db.exists() {
                let store = amos::store::Store::open(&config.control_db)?;
                seed::seed_demo(&store, &config.warehouse_db)?;
            }
            serve(
                AmosRuntime::open(config)?,
                api::demo_router,
                IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                port.unwrap_or(8000),
                "explicit local demo",
            )
            .await?;
        }
        Command::Run {
            request,
            identity,
            idempotency_key,
        } => {
            if !config.warehouse_db.exists() {
                let store = amos::store::Store::open(&config.control_db)?;
                seed::seed_demo(&store, &config.warehouse_db)?;
            }
            let runtime = AmosRuntime::open(config)?;
            let identities = api::demo_identities();
            let identity = identities
                .get(&identity)
                .ok_or_else(|| AmosError::Unauthenticated("unknown identity".into()))?;
            let result = runtime.run_task(identity, request, idempotency_key).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Replay {
            artifact_id,
            identity,
            idempotency_key,
        } => {
            let runtime = AmosRuntime::open(config)?;
            let identities = api::demo_identities();
            let identity = identities
                .get(&identity)
                .ok_or_else(|| AmosError::Unauthenticated("unknown identity".into()))?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &runtime
                        .replay_async(identity, artifact_id, idempotency_key)
                        .await?,
                )?
            );
        }
    }
    Ok(())
}

async fn run_configured(cli: Cli, config_path: PathBuf) -> Result<()> {
    let server_config = ServerConfig::load(&config_path)?;
    match cli.command.unwrap_or(Command::Serve {
        port: None,
        seed_demo: false,
    }) {
        Command::Serve { port, seed_demo } => {
            if port.is_some() || seed_demo {
                return Err(AmosError::Validation(
                    "configured server bind/port cannot be overridden and --seed-demo is forbidden; use amosctl bootstrap-reference explicitly for the evaluation fixture"
                        .into(),
                ));
            }
            server_config.ensure_data_directories()?;
            if !server_config.warehouse_db.is_file() {
                return Err(AmosError::Validation(format!(
                    "configured reference warehouse {} does not exist; initialize it explicitly with amosctl bootstrap-reference --config {}",
                    server_config.warehouse_db.display(),
                    config_path.display()
                )));
            }
            let loaded = server_config.load_runtime()?;
            let identity_provider = loaded.identity_provider.clone();
            let bind = loaded.server.bind_address;
            let port = loaded.server.port;
            serve(
                AmosRuntime::open(loaded.runtime)?,
                move |runtime| api::router(runtime, identity_provider),
                bind,
                port,
                "customer evaluation",
            )
            .await?;
        }
        Command::Seed | Command::Run { .. } | Command::Replay { .. } => {
            return Err(AmosError::Validation(
                "configured mode supports the server only; use the authenticated HTTP API and amosctl operations"
                    .into(),
            ));
        }
    }
    Ok(())
}

async fn serve<F>(
    runtime: AmosRuntime,
    router: F,
    bind_address: IpAddr,
    port: u16,
    mode: &str,
) -> Result<()>
where
    F: FnOnce(AmosRuntime) -> axum::Router,
{
    let app = router(runtime);
    let listener = tokio::net::TcpListener::bind((bind_address, port))
        .await
        .map_err(|error| AmosError::Storage(error.to_string()))?;
    println!("AMOS {mode} listening on http://{bind_address}:{port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|error| AmosError::Storage(error.to_string()))
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    result = tokio::signal::ctrl_c() => {
                        if let Err(error) = result {
                            tracing::error!(%error, "failed to install Ctrl-C shutdown listener");
                        }
                    }
                    _ = terminate.recv() => {}
                }
            }
            Err(error) => {
                tracing::error!(%error, "failed to install SIGTERM shutdown listener");
                if let Err(error) = tokio::signal::ctrl_c().await {
                    tracing::error!(%error, "failed to install Ctrl-C shutdown listener");
                }
            }
        }
    }
    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to install Ctrl-C shutdown listener");
    }
}
