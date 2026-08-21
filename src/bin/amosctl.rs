use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::Utc;
use clap::{Parser, Subcommand};
use ed25519_dalek::SigningKey;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::json;
use zeroize::Zeroizing;

use amos::{
    AmosError, AmosRuntime, Result,
    auth::HashedTokenIdentityProvider,
    deployment::{ServerConfig, read_capability_key},
    domain::{Identity, OperationLimits, PlanStep, TypedPlan},
    seed,
    solution_pack::{SignedSolutionPack, SolutionPackRegistry, TrustStore},
    store::Store,
    tools::{ToolImplementation, ToolManifest, ToolRegistry},
    workers::{CapabilityIssuer, ToolboxWorker},
};

#[derive(Parser)]
#[command(
    name = "amosctl",
    version,
    about = "Install and diagnose an AMOS customer-evaluation server"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Validate {
        #[arg(long, env = "AMOS_CONFIG")]
        config: PathBuf,
    },
    Preflight {
        #[arg(long, env = "AMOS_CONFIG")]
        config: PathBuf,
        #[arg(long)]
        require_initialized: bool,
    },
    BootstrapReference {
        #[arg(long, env = "AMOS_CONFIG")]
        config: PathBuf,
    },
    Status {
        #[arg(long, env = "AMOS_CONFIG")]
        config: PathBuf,
    },
    HashToken,
    Health {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8000)]
        port: u16,
        #[arg(long, default_value_t = 3)]
        timeout_seconds: u64,
    },
    Tools {
        #[command(subcommand)]
        command: ToolsCommand,
    },
    SolutionPacks {
        #[command(subcommand)]
        command: SolutionPacksCommand,
    },
}

#[derive(Subcommand)]
enum ToolsCommand {
    List,
    Show {
        #[arg(long)]
        tool_id: String,
    },
    Validate {
        #[arg(long, required = true)]
        manifest: Vec<PathBuf>,
    },
    Smoke {
        #[arg(long, default_value = "toolbox:9000")]
        endpoint: String,
        #[arg(long)]
        capability_key_file: PathBuf,
        #[arg(long)]
        tool_id: Option<String>,
    },
}

#[derive(Subcommand)]
enum SolutionPacksCommand {
    Validate {
        #[arg(long, required = true)]
        pack: Vec<PathBuf>,
        #[arg(long)]
        trust_store: PathBuf,
        #[arg(long)]
        tenant: String,
        #[arg(long, default_value = env!("CARGO_PKG_VERSION"))]
        core_version: String,
    },
    Sign {
        #[arg(long)]
        pack: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        key_id: String,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Validate { config } => validate(config),
        Command::Preflight {
            config,
            require_initialized,
        } => preflight(config, require_initialized),
        Command::BootstrapReference { config } => bootstrap_reference(config),
        Command::Status { config } => status(config),
        Command::HashToken => hash_token(),
        Command::Health {
            host,
            port,
            timeout_seconds,
        } => health(&host, port, timeout_seconds),
        Command::Tools { command } => match command {
            ToolsCommand::List => tools_list(),
            ToolsCommand::Show { tool_id } => tools_show(&tool_id),
            ToolsCommand::Validate { manifest } => tools_validate(&manifest),
            ToolsCommand::Smoke {
                endpoint,
                capability_key_file,
                tool_id,
            } => tools_smoke(&endpoint, &capability_key_file, tool_id.as_deref()),
        },
        Command::SolutionPacks { command } => match command {
            SolutionPacksCommand::Validate {
                pack,
                trust_store,
                tenant,
                core_version,
            } => solution_packs_validate(&pack, &trust_store, &tenant, &core_version),
            SolutionPacksCommand::Sign {
                pack,
                output,
                key_id,
            } => solution_pack_sign(&pack, &output, &key_id),
        },
    }
}

fn solution_packs_validate(
    paths: &[PathBuf],
    trust_store_path: &Path,
    tenant: &str,
    core_version: &str,
) -> Result<()> {
    let trust_store = TrustStore::load(trust_store_path)?;
    let now = Utc::now();
    let mut registry = SolutionPackRegistry::default();
    let mut verified = Vec::with_capacity(paths.len());
    for path in paths {
        let pack = SignedSolutionPack::load(path)?;
        let result = registry.activate(pack, &trust_store, tenant, core_version, now)?;
        verified.push(json!({
            "path": path,
            "pack_id": result.pack_id,
            "workflow_id": result.workflow_id,
            "version": result.version,
            "manifest_hash": result.manifest_hash,
            "verified_key_id": result.verified_key_id,
            "tenant_id": result.tenant_id,
            "status": "valid_for_activation"
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": "valid",
            "pack_count": verified.len(),
            "core_version": core_version,
            "validated_at": now,
            "packs": verified
        }))?
    );
    Ok(())
}

fn solution_pack_sign(input: &Path, output: &Path, key_id: &str) -> Result<()> {
    let mut private_key_input = Zeroizing::new(String::new());
    io::stdin().read_to_string(&mut private_key_input)?;
    let decoded = Zeroizing::new(hex::decode(private_key_input.trim()).map_err(|_| {
        AmosError::Validation(
            "Ed25519 private key on stdin must be exactly 32 bytes of hexadecimal".into(),
        )
    })?);
    let private_key = Zeroizing::new(<[u8; 32]>::try_from(decoded.as_slice()).map_err(|_| {
        AmosError::Validation(format!(
            "Ed25519 private key on stdin must be exactly 32 bytes, found {}",
            decoded.len()
        ))
    })?);
    let mut pack = SignedSolutionPack::load(input)?;
    pack.sign(key_id, &private_key)?;
    pack.save_pretty(output)?;
    let signing_key = SigningKey::from_bytes(&private_key);
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": "signed",
            "pack_id": pack.manifest.pack_id,
            "manifest_hash": pack.manifest_hash()?,
            "key_id": key_id,
            "public_key_hex": hex::encode(signing_key.verifying_key().to_bytes()),
            "output": output
        }))?
    );
    Ok(())
}

fn tools_smoke(endpoint: &str, capability_key_file: &Path, selected: Option<&str>) -> Result<()> {
    let registry = ToolRegistry::builtins()?;
    let issuer = CapabilityIssuer::new(read_capability_key(capability_key_file)?)?;
    let worker = ToolboxWorker::new(endpoint, issuer.clone())?;
    let identity = Identity {
        tenant_id: "tenant_toolbox_smoke".into(),
        subject_id: "amosctl".into(),
        roles: BTreeSet::from(["operator".into()]),
        groups: BTreeSet::new(),
        permissions: BTreeSet::new(),
        policy_attributes: BTreeMap::new(),
        policy_epoch: 1,
    };
    let tool_ids = registry
        .list()
        .into_iter()
        .filter(|manifest| {
            matches!(
                registry.implementation(&manifest.tool_id),
                Ok(ToolImplementation::ExternalToolbox)
            ) && selected.is_none_or(|selected| selected == manifest.tool_id)
        })
        .map(|manifest| manifest.tool_id.clone())
        .collect::<Vec<_>>();
    if tool_ids.is_empty() {
        return Err(AmosError::Validation(
            "no matching external toolbox tool is registered".into(),
        ));
    }
    let mut results = Vec::with_capacity(tool_ids.len());
    for tool_id in tool_ids {
        let manifest = registry.get(&tool_id)?;
        let step = PlanStep {
            step_id: format!("smoke-{}", tool_id.replace('.', "-")),
            purpose: "operator executable conformance smoke test".into(),
            tool: tool_id.clone(),
            source_id: String::new(),
            input_object_ids: vec![],
            parameter_schema: tool_id.clone(),
            parameters: smoke_parameters(&tool_id)?,
            expected_output_schema: tool_id.clone(),
            limits: OperationLimits {
                seconds: manifest.resource_limits.max_seconds.min(300),
                rows: manifest.resource_limits.max_rows,
                bytes: manifest.resource_limits.max_bytes,
            },
            max_attempts: 1,
            repair_classes: BTreeSet::new(),
            verifier_profile: manifest.verifier_profile.clone(),
        };
        manifest.validate_parameters(&step.parameters)?;
        let plan = TypedPlan {
            plan_id: format!("plan-{}", step.step_id),
            tenant_id: identity.tenant_id.clone(),
            atxn_id: format!("atxn-{}", step.step_id),
            task_definition: "toolbox-smoke.v1".into(),
            manifest_id: "operator-smoke".into(),
            model_identity: "amosctl".into(),
            steps: vec![step.clone()],
        };
        let capability = issuer.issue_for_tool(&identity, &plan, &step, 1, manifest)?;
        let execution = worker.execute(&identity, &plan, &step, manifest, &capability, 1)?;
        manifest.validate_output(&execution.output)?;
        results.push(json!({
            "tool_id": tool_id,
            "status": execution.status,
            "row_count": execution.row_count,
            "byte_count": execution.byte_count,
            "output_hash": execution.output_hash,
            "runtime_versions": execution.input_versions
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": "pass",
            "endpoint": endpoint,
            "tool_count": results.len(),
            "tools": results
        }))?
    );
    Ok(())
}

fn smoke_parameters(tool_id: &str) -> Result<serde_json::Value> {
    let table = json!({
        "columns": ["region", "value"],
        "rows": [["east", 1], ["east", 2], ["west", 4]]
    });
    Ok(match tool_id {
        "spark.dataframe.aggregate.v1" => json!({
            "columns": table["columns"], "rows": table["rows"], "group_by": ["region"],
            "metrics": [{"function": "sum", "column": "value", "alias": "total"}], "filters": []
        }),
        "r.statistics.v1" => json!({
            "columns": ["x", "y"], "rows": [[1, 2], [2, 4], [3, 6]],
            "method": "linear_regression", "outcome": "y", "predictors": ["x"], "seed": 7
        }),
        "python.dataframe.aggregate.v1" | "polars.dataframe.aggregate.v1" => json!({
            "columns": table["columns"], "rows": table["rows"], "group_by": ["region"],
            "metrics": [{"function": "sum", "column": "value", "alias": "total"}]
        }),
        "duckdb.readonly.v1" => json!({
            "columns": table["columns"], "rows": table["rows"],
            "sql": "SELECT region, SUM(value) AS total FROM input_data GROUP BY region ORDER BY region"
        }),
        "dbt.manifest.validate.v1" => json!({
            "manifest_json": "{\"metadata\":{\"dbt_schema_version\":\"v12\",\"dbt_version\":\"smoke\"},\"nodes\":{}}"
        }),
        "stats.regression.v1" => json!({"x": [[1], [2], [3]], "y": [2, 4, 6]}),
        "stats.forecast.v1" => json!({"values": [1, 2, 3], "horizon": 2}),
        "stats.pca.v1" => json!({"matrix": [[1, 2], [2, 4], [3, 6]], "components": 1}),
        "spreadsheet.xlsx.v1" => json!({"sheets": [{
            "title": "Data", "rows": [["region", "value"], ["east", 1], ["west", 4]], "freeze_header": true
        }]}),
        "presentation.pptx.v1" => json!({"slides": [{
            "title": "AMOS toolbox", "bullets": ["Editable smoke-test presentation"]
        }]}),
        "notebook.inspect.v1" => json!({
            "notebook_json": "{\"nbformat\":4,\"nbformat_minor\":5,\"metadata\":{},\"cells\":[]}"
        }),
        _ => {
            return Err(AmosError::Validation(format!(
                "no smoke payload is defined for {tool_id}"
            )));
        }
    })
}

fn tools_list() -> Result<()> {
    let registry = ToolRegistry::builtins()?;
    println!("{}", serde_json::to_string_pretty(&registry.list())?);
    Ok(())
}

fn tools_show(tool_id: &str) -> Result<()> {
    let registry = ToolRegistry::builtins()?;
    println!("{}", serde_json::to_string_pretty(registry.get(tool_id)?)?);
    Ok(())
}

fn tools_validate(paths: &[PathBuf]) -> Result<()> {
    let mut registry = ToolRegistry::default();
    let mut validated = Vec::with_capacity(paths.len());
    for path in paths {
        let manifest = ToolManifest::load(path)?;
        registry.register_contract(manifest.clone())?;
        validated.push(json!({
            "path": path,
            "tool_id": manifest.tool_id,
            "availability": manifest.availability,
            "activated": false,
            "executable": false,
            "status": "valid"
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": "valid",
            "manifest_count": validated.len(),
            "manifests": validated
        }))?
    );
    Ok(())
}

fn validate(config_path: PathBuf) -> Result<()> {
    let config = ServerConfig::load(&config_path)?;
    let loaded = config.load_runtime()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": "valid",
            "config": config_path,
            "schema_version": loaded.server.schema_version,
            "deployment_mode": loaded.server.deployment_mode,
            "bind_address": loaded.server.bind_address,
            "port": loaded.server.port,
            "public_base_url": loaded.server.public_base_url,
            "local_reference_adapters_acknowledged": loaded.server.acknowledge_local_reference_adapters,
            "secrets_loaded": true
        }))?
    );
    Ok(())
}

fn preflight(config_path: PathBuf, require_initialized: bool) -> Result<()> {
    let config = ServerConfig::load(&config_path)?;
    let _loaded = config.load_runtime()?;
    let control = file_state(&config.control_db)?;
    let warehouse = file_state(&config.warehouse_db)?;
    if require_initialized && (!control.exists || !warehouse.exists) {
        return Err(AmosError::Validation(
            "preflight requires initialized control and reference warehouse databases".into(),
        ));
    }
    if require_initialized
        && (sqlite_schema_version(&config.control_db)?.is_none()
            || !sqlite_has_table(&config.warehouse_db, "payment_events")?)
    {
        return Err(AmosError::Validation(
            "preflight found database files but not the expected initialized AMOS control schema and payment reference table"
                .into(),
        ));
    }
    let cpus = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1);
    let tool_registry = ToolRegistry::builtins()?;
    let tools = tool_registry
        .list()
        .into_iter()
        .map(|manifest| {
            Ok(json!({
                "tool_id": manifest.tool_id,
                "availability": manifest.availability,
                "executable_plan_step": tool_registry.is_executable(&manifest.tool_id)?
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": "pass",
            "configuration": config_path,
            "platform": {
                "os": std::env::consts::OS,
                "architecture": std::env::consts::ARCH,
                "logical_cpus": cpus
            },
            "bind": format!("{}:{}", config.bind_address, config.port),
            "control_database": control,
            "reference_warehouse": warehouse,
            "object_root": {
                "path": config.object_root,
                "exists": config.object_root.is_dir()
            },
            "governed_tools": tools,
            "evaluation_integrations": {
                "separate_toolbox_container": config.toolbox_endpoint.is_some()
            },
            "production_integrations": {
                "postgresql": false,
                "enterprise_oidc": false,
                "production_connector": false,
                "per_risk_worker_pools": false,
                "object_storage": false
            }
        }))?
    );
    Ok(())
}

fn bootstrap_reference(config_path: PathBuf) -> Result<()> {
    let config = ServerConfig::load(&config_path)?;
    let _loaded = config.load_runtime()?;
    if config.control_db.exists() || config.warehouse_db.exists() {
        return Err(AmosError::Conflict(
            "bootstrap-reference refuses to overwrite or merge existing database files".into(),
        ));
    }
    config.ensure_data_directories()?;
    let store = Store::open(&config.control_db)?;
    seed::seed_demo(&store, &config.warehouse_db)?;
    let loaded = config.load_runtime()?;
    let _runtime = AmosRuntime::open(loaded.runtime)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": "initialized",
            "boundary": "payment reference fixture for customer evaluation only",
            "control_database": config.control_db,
            "reference_warehouse": config.warehouse_db,
            "object_root": config.object_root
        }))?
    );
    Ok(())
}

fn status(config_path: PathBuf) -> Result<()> {
    let config = ServerConfig::load(&config_path)?;
    let _loaded = config.load_runtime()?;
    let schema_version = sqlite_schema_version(&config.control_db)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": if schema_version.is_some() { "initialized" } else { "not_initialized" },
            "version": env!("CARGO_PKG_VERSION"),
            "deployment_mode": config.deployment_mode,
            "control_database": file_state(&config.control_db)?,
            "reference_warehouse": file_state(&config.warehouse_db)?,
            "control_schema_version": schema_version,
            "object_root_exists": config.object_root.is_dir()
        }))?
    );
    Ok(())
}

fn hash_token() -> Result<()> {
    let mut token = String::new();
    io::stdin().read_to_string(&mut token)?;
    let token = token.trim_end_matches(['\r', '\n']);
    println!("{}", HashedTokenIdentityProvider::token_hash(token)?);
    Ok(())
}

fn health(host: &str, port: u16, timeout_seconds: u64) -> Result<()> {
    let address: SocketAddr = format!("{host}:{port}").parse().map_err(|_| {
        AmosError::Validation("health host and port must resolve to a socket address".into())
    })?;
    let timeout = Duration::from_secs(timeout_seconds.clamp(1, 30));
    let mut stream = TcpStream::connect_timeout(&address, timeout)
        .map_err(|error| AmosError::Storage(format!("health connection failed: {error}")))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream.write_all(
        format!(
            "GET /health HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nUser-Agent: amosctl/{}\r\n\r\n",
            env!("CARGO_PKG_VERSION")
        )
        .as_bytes(),
    )?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let status_line = response.lines().next().unwrap_or_default();
    if !status_line.contains(" 200 ") || !response.contains("\"status\":\"ok\"") {
        return Err(AmosError::Storage(format!(
            "health endpoint returned an unexpected response: {status_line}"
        )));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": "ok",
            "endpoint": format!("http://{host}:{port}/health")
        }))?
    );
    Ok(())
}

#[derive(serde::Serialize)]
struct FileState {
    path: PathBuf,
    exists: bool,
    bytes: Option<u64>,
}

fn file_state(path: &Path) -> Result<FileState> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(FileState {
            path: path.to_path_buf(),
            exists: true,
            bytes: Some(metadata.len()),
        }),
        Ok(_) => Err(AmosError::Validation(format!(
            "{} exists but is not a regular file",
            path.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(FileState {
            path: path.to_path_buf(),
            exists: false,
            bytes: None,
        }),
        Err(error) => Err(AmosError::Storage(format!(
            "failed to inspect {}: {error}",
            path.display()
        ))),
    }
}

fn sqlite_schema_version(path: &Path) -> Result<Option<u32>> {
    if !path.exists() {
        return Ok(None);
    }
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let has_ledger: Option<String> = connection
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if has_ledger.is_none() {
        return Ok(None);
    }
    connection
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get::<_, Option<u32>>(0)
        })
        .map_err(Into::into)
}

fn sqlite_has_table(path: &Path, table: &str) -> Result<bool> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            [table],
            |row| row.get(0),
        )
        .map_err(Into::into)
}
