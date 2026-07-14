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
    },
    Replay {
        artifact_id: String,
        #[arg(long, default_value = "analyst_001")]
        identity: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let cli = Cli::parse();
    let config = RuntimeConfig::local(&cli.root);
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
            let app = api::router(runtime);
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
                .await
                .map_err(|e| amos::AmosError::Storage(e.to_string()))?;
            println!("AMOS listening on http://127.0.0.1:{port}");
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await
                .map_err(|e| amos::AmosError::Storage(e.to_string()))?;
        }
        Command::Run { request, identity } => {
            if !config.warehouse_db.exists() {
                let store = amos::store::Store::open(&config.control_db)?;
                seed::seed_demo(&store, &config.warehouse_db)?;
            }
            let runtime = AmosRuntime::open(config)?;
            let identities = api::demo_identities();
            let identity = identities
                .get(&identity)
                .ok_or_else(|| amos::AmosError::PermissionDenied("unknown identity".into()))?;
            let result = runtime
                .run_task(identity, request, amos::domain::new_id("cli"))
                .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Replay {
            artifact_id,
            identity,
        } => {
            let runtime = AmosRuntime::open(config)?;
            let identities = api::demo_identities();
            let identity = identities
                .get(&identity)
                .ok_or_else(|| amos::AmosError::PermissionDenied("unknown identity".into()))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&runtime.replay(identity, &artifact_id)?)?
            );
        }
    }
    Ok(())
}
