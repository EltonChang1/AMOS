use std::path::PathBuf;

use clap::{Parser, Subcommand};

use amos::{AmosRuntime, Result, RuntimeConfig, api, seed};

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
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    Seed,
    Serve {
        #[arg(long, default_value_t = 8000)]
        port: u16,
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
    if !cli.demo {
        return Err(amos::AmosError::Validation(
            "the bundled binary has no production identity provider; use --demo only for the explicit local demo, or embed amos::api::router with an IdentityProvider".into(),
        ));
    }
    let config = RuntimeConfig::demo(&cli.root);
    match cli.command.unwrap_or(Command::Serve {
        port: 8000,
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
            let runtime = AmosRuntime::open(config)?;
            let app = api::demo_router(runtime);
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
                .await
                .map_err(|e| amos::AmosError::Storage(e.to_string()))?;
            println!("AMOS listening on http://127.0.0.1:{port}");
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    if let Err(error) = tokio::signal::ctrl_c().await {
                        tracing::error!(%error, "failed to install Ctrl-C shutdown listener");
                    }
                })
                .await
                .map_err(|e| amos::AmosError::Storage(e.to_string()))?;
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
                .ok_or_else(|| amos::AmosError::Unauthenticated("unknown identity".into()))?;
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
                .ok_or_else(|| amos::AmosError::Unauthenticated("unknown identity".into()))?;
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
