use std::{env, fs, net::IpAddr, path::PathBuf, sync::Arc, time::Duration};

use clap::{Parser, Subcommand};

use amos::{
    AmosRuntime, Result, RuntimeConfig, api,
    model::{
        DEFAULT_GEMMA_MODEL, GemmaApiConfig, GemmaApiProvider, ModelProvider, SecretValue,
        UnavailableModelProvider,
    },
    packs::AnalysisPack,
    privacy::{ModelRouteClass, PrivacyBoundaryConfig, PrivacyProfile},
    seed,
};

const DEMO_QUESTION: &str = "Why did SMB logo churn increase this week, and should the executive dashboard attribute it to the pricing email?";
const DEFAULT_MODEL_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

#[derive(Parser)]
#[command(
    name = "amos",
    version,
    about = "Rust-native AMOS governed analyst control layer"
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
        #[arg(long, env = "AMOS_BIND_ADDRESS", default_value = "127.0.0.1")]
        bind: IpAddr,
        #[arg(long)]
        seed_demo: bool,
    },
    Run {
        #[arg(long)]
        request: String,
        #[arg(long)]
        task_type: Option<String>,
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
    ModelProbe,
    Pack {
        #[command(subcommand)]
        command: PackCommand,
    },
}

#[derive(Subcommand)]
enum PackCommand {
    Validate {
        path: PathBuf,
    },
    DryRun {
        path: PathBuf,
    },
    Install {
        path: PathBuf,
        #[arg(long, default_value = "admin")]
        identity: String,
    },
    List {
        #[arg(long, default_value = "admin")]
        identity: String,
    },
    Show {
        pack_id: String,
        #[arg(long, default_value = "admin")]
        identity: String,
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
    let mut config = RuntimeConfig::demo(&cli.root)?;
    match cli.command.unwrap_or(Command::Serve {
        port: 8000,
        bind: IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        seed_demo: false,
    }) {
        Command::Seed => {
            let store = amos::store::Store::open(&config.control_db)?;
            seed::seed_demo(&store, &config.warehouse_db)?;
            println!("Seeded AMOS subscription demo under {}", cli.root.display());
        }
        Command::Serve {
            port,
            bind,
            seed_demo,
        } => {
            validate_demo_bind(bind)?;
            if seed_demo || !config.warehouse_db.exists() {
                let store = amos::store::Store::open(&config.control_db)?;
                seed::seed_demo(&store, &config.warehouse_db)?;
            }
            let provider = configured_model_provider(&mut config)?;
            let runtime = AmosRuntime::open_with_model(config, provider)?;
            let app = api::demo_router(runtime);
            let listener = tokio::net::TcpListener::bind((bind, port))
                .await
                .map_err(|error| amos::AmosError::Storage(error.to_string()))?;
            println!("AMOS listening on http://{bind}:{port}");
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    if let Err(error) = tokio::signal::ctrl_c().await {
                        tracing::error!(%error, "failed to install Ctrl-C shutdown listener");
                    }
                })
                .await
                .map_err(|error| amos::AmosError::Storage(error.to_string()))?;
        }
        Command::Run {
            request,
            task_type,
            identity,
            idempotency_key,
        } => {
            seed_if_missing(&config)?;
            let provider = configured_model_provider(&mut config)?;
            let runtime = AmosRuntime::open_with_model(config, provider)?;
            let identities = api::demo_identities();
            let identity = identities
                .get(&identity)
                .ok_or_else(|| amos::AmosError::Unauthenticated("unknown identity".into()))?;
            let result = runtime
                .run_task_typed(identity, request, task_type, idempotency_key)
                .await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Command::Replay {
            artifact_id,
            identity,
            idempotency_key,
        } => {
            let provider = configured_model_provider(&mut config)?;
            let runtime = AmosRuntime::open_with_model(config, provider)?;
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
        Command::ModelProbe => {
            seed_if_missing(&config)?;
            let provider = configured_model_provider(&mut config)?;
            let runtime = AmosRuntime::open_with_model(config, provider)?;
            let identities = api::demo_identities();
            let result = runtime
                .run_task(
                    &identities["analyst_001"],
                    DEMO_QUESTION.into(),
                    amos::domain::new_id("live_model_probe"),
                )
                .await?;
            let invocations = runtime
                .store
                .list_model_invocations(seed::TENANT, &result.transaction.atxn_id)?;
            let safe_summary = invocations
                .into_iter()
                .map(|invocation| {
                    serde_json::json!({
                        "model":invocation.model,
                        "purpose":invocation.purpose,
                        "invocation_id":invocation.invocation_id,
                        "latency_ms":invocation.latency_ms,
                        "input_tokens":invocation.input_tokens,
                        "output_tokens":invocation.output_tokens,
                        "input_payload_hash":invocation.input_payload_hash,
                        "output_hash":invocation.output_hash,
                    })
                })
                .collect::<Vec<_>>();
            println!("{}", serde_json::to_string_pretty(&safe_summary)?);
        }
        Command::Pack { command } => match command {
            PackCommand::Validate { path } => {
                let pack = AnalysisPack::load(&path)?;
                println!(
                    "valid pack_id={} task_type={} version={}",
                    pack.pack_id, pack.task_type, pack.version
                );
            }
            PackCommand::DryRun { path } => {
                let pack = AnalysisPack::load(&path)?;
                let (relation, source) = pack.primary_relation()?;
                let definition = pack.to_task_definition(seed::TENANT)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "pack_id": pack.pack_id,
                        "task_type": pack.task_type,
                        "version": pack.version,
                        "primary_relation": relation,
                        "source": source,
                        "required_roles": pack.required_roles,
                        "required_analysis_kinds": pack.required_analysis_kinds,
                        "metric_required_filters": pack.metric_required_filters,
                        "task_definition": definition,
                    }))?
                );
            }
            PackCommand::Install { path, identity } => {
                seed_if_missing(&config)?;
                let pack = AnalysisPack::load(&path)?;
                let provider = configured_model_provider(&mut config)?;
                let runtime = AmosRuntime::open_with_model(config, provider)?;
                let identities = api::demo_identities();
                let identity = identities
                    .get(&identity)
                    .ok_or_else(|| amos::AmosError::Unauthenticated("unknown identity".into()))?;
                let (newly_installed, pack) = runtime.install_pack(identity, pack)?;
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "newly_installed": newly_installed,
                        "pack_id": pack.pack_id,
                        "task_type": pack.task_type,
                        "version": pack.version,
                    }))?
                );
            }
            PackCommand::List { identity } => {
                seed_if_missing(&config)?;
                let provider = configured_model_provider(&mut config)?;
                let runtime = AmosRuntime::open_with_model(config, provider)?;
                let identities = api::demo_identities();
                let identity = identities
                    .get(&identity)
                    .ok_or_else(|| amos::AmosError::Unauthenticated("unknown identity".into()))?;
                runtime.authorize_operations(identity)?;
                let packs = runtime.list_installed_packs(identity)?;
                println!("{}", serde_json::to_string_pretty(&packs)?);
            }
            PackCommand::Show { pack_id, identity } => {
                seed_if_missing(&config)?;
                let provider = configured_model_provider(&mut config)?;
                let runtime = AmosRuntime::open_with_model(config, provider)?;
                let identities = api::demo_identities();
                let identity = identities
                    .get(&identity)
                    .ok_or_else(|| amos::AmosError::Unauthenticated("unknown identity".into()))?;
                runtime.authorize_operations(identity)?;
                let pack = runtime.get_installed_pack(identity, &pack_id)?;
                println!("{}", serde_json::to_string_pretty(&pack)?);
            }
        },
    }
    Ok(())
}

fn seed_if_missing(config: &RuntimeConfig) -> Result<()> {
    if !config.warehouse_db.exists() {
        let store = amos::store::Store::open(&config.control_db)?;
        seed::seed_demo(&store, &config.warehouse_db)?;
    }
    Ok(())
}

fn configured_model_provider(config: &mut RuntimeConfig) -> Result<Arc<dyn ModelProvider>> {
    let provider = env::var("AMOS_MODEL_PROVIDER").unwrap_or_default();
    if provider.trim().is_empty() || provider == "unavailable" {
        return Ok(Arc::new(UnavailableModelProvider::new(
            "AMOS_MODEL_PROVIDER=gemma_api and a model credential are required",
        )));
    }
    if provider != "gemma_api" {
        return Err(amos::AmosError::Validation(
            "AMOS_MODEL_PROVIDER is unsupported".into(),
        ));
    }
    let model = env::var("AMOS_MODEL_NAME").unwrap_or_else(|_| DEFAULT_GEMMA_MODEL.into());
    let base_url =
        env::var("AMOS_MODEL_BASE_URL").unwrap_or_else(|_| DEFAULT_MODEL_BASE_URL.into());
    let route_class = parse_route_class(
        &env::var("AMOS_MODEL_ROUTE_CLASS").unwrap_or_else(|_| "approved_hosted_api".into()),
    )?;
    let profile = parse_privacy_profile(
        &env::var("AMOS_PRIVACY_PROFILE").unwrap_or_else(|_| "approved_api".into()),
    )?;
    let timeout_seconds = parse_env_u64("AMOS_MODEL_TIMEOUT_SECONDS", 45)?;
    if timeout_seconds == 0 {
        return Err(amos::AmosError::Validation(
            "AMOS_MODEL_TIMEOUT_SECONDS must be positive".into(),
        ));
    }
    config.model_max_attempts = parse_env_u32("AMOS_MODEL_MAX_ATTEMPTS", 2)?;
    config.model_temperature = parse_env_f32("AMOS_MODEL_TEMPERATURE", 0.1)?;
    let external_telemetry = parse_env_bool("AMOS_EXTERNAL_TELEMETRY", false)?;
    let allowed_egress_hosts = env::var("AMOS_ALLOWED_EGRESS_HOSTS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    config.privacy = PrivacyBoundaryConfig {
        profile,
        model_route_class: route_class,
        model_base_url: base_url.clone(),
        allowed_egress_hosts,
        external_telemetry,
    };
    config.privacy.validate()?;
    let secret = SecretValue::new(model_secret()?)?;
    Ok(Arc::new(GemmaApiProvider::new(GemmaApiConfig {
        model,
        base_url,
        route_class,
        timeout: Duration::from_secs(timeout_seconds),
        api_key: secret,
    })?))
}

fn model_secret() -> Result<Vec<u8>> {
    let direct = env::var_os("GEMINI_API_KEY");
    let file = env::var_os("GEMINI_API_KEY_FILE");
    match (direct, file) {
        (Some(_), Some(_)) => Err(amos::AmosError::Validation(
            "configure either GEMINI_API_KEY or GEMINI_API_KEY_FILE, not both".into(),
        )),
        (Some(value), None) => value
            .into_string()
            .map(String::into_bytes)
            .map_err(|_| amos::AmosError::Validation("GEMINI_API_KEY is not valid Unicode".into())),
        (None, Some(path)) => {
            let mut secret = fs::read(PathBuf::from(path))?;
            while secret.last().is_some_and(u8::is_ascii_whitespace) {
                secret.pop();
            }
            Ok(secret)
        }
        (None, None) => Err(amos::AmosError::ModelUnavailable(
            "model API credential is not configured".into(),
        )),
    }
}

fn parse_route_class(value: &str) -> Result<ModelRouteClass> {
    match value {
        "local" => Ok(ModelRouteClass::Local),
        "customer_vpc_private_endpoint" => Ok(ModelRouteClass::CustomerVpcPrivateEndpoint),
        "approved_hosted_api" => Ok(ModelRouteClass::ApprovedHostedApi),
        _ => Err(amos::AmosError::Validation(
            "AMOS_MODEL_ROUTE_CLASS is invalid".into(),
        )),
    }
}

fn parse_privacy_profile(value: &str) -> Result<PrivacyProfile> {
    match value {
        "air_gapped" => Ok(PrivacyProfile::AirGapped),
        "approved_api" => Ok(PrivacyProfile::ApprovedApi),
        _ => Err(amos::AmosError::Validation(
            "AMOS_PRIVACY_PROFILE is invalid".into(),
        )),
    }
}

fn parse_env_u64(name: &str, default: u64) -> Result<u64> {
    env::var(name).map_or(Ok(default), |value| {
        value
            .parse()
            .map_err(|_| amos::AmosError::Validation(format!("{name} is invalid")))
    })
}

fn parse_env_u32(name: &str, default: u32) -> Result<u32> {
    env::var(name).map_or(Ok(default), |value| {
        value
            .parse()
            .map_err(|_| amos::AmosError::Validation(format!("{name} is invalid")))
    })
}

fn parse_env_f32(name: &str, default: f32) -> Result<f32> {
    env::var(name).map_or(Ok(default), |value| {
        value
            .parse()
            .map_err(|_| amos::AmosError::Validation(format!("{name} is invalid")))
    })
}

fn parse_env_bool(name: &str, default: bool) -> Result<bool> {
    env::var(name).map_or(Ok(default), |value| match value.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(amos::AmosError::Validation(format!("{name} is invalid"))),
    })
}

fn validate_demo_bind(bind: IpAddr) -> Result<()> {
    if bind.is_loopback() || parse_env_bool("AMOS_DEMO_LOOPBACK_PUBLISH", false)? {
        Ok(())
    } else {
        Err(amos::AmosError::Validation(
            "demo identity routes require loopback binding; container bridge binding requires AMOS_DEMO_LOOPBACK_PUBLISH=true and a loopback-only host publish"
                .into(),
        ))
    }
}
