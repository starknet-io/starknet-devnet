use std::collections::HashMap;
use std::future::IntoFuture;
use std::net::IpAddr;
use std::result::Result::Ok;
use std::sync::Arc;
use std::time::Duration;

use clap::{CommandFactory, FromArgMatches};
use cli::Args;
use futures::future::join_all;
use serde::de::IntoDeserializer;
use serde_json::json;
use server::ServerConfig;
use server::api::{Api, JsonRpcHandler, RPC_SPEC_VERSION};
use server::dump_util::{read_dump, write_dump};
use server::server::serve_http_json_rpc;
use starknet_core::account::Account;
use starknet_core::constants::{
    ARGENT_CONTRACT_CLASS_HASH, ARGENT_MULTISIG_CONTRACT_CLASS_HASH, ETH_ERC20_CONTRACT_ADDRESS,
    STRK_ERC20_CONTRACT_ADDRESS, UDC_CONTRACT_ADDRESS, UDC_CONTRACT_CLASS_HASH,
};
use starknet_core::starknet::Starknet;
use starknet_core::starknet::dump::{DumpLoadOptions, StateDump};
use starknet_core::starknet::starknet_config::{
    BlockGenerationOn, DumpOn, ForkConfig, StarknetConfig,
};
use starknet_rs_core::types::ContractClass::{Legacy, Sierra};
use starknet_rs_core::types::{
    BlockId, BlockTag, Felt, MaybePreConfirmedBlockWithTxHashes, StarknetError,
};
use starknet_rs_providers::jsonrpc::HttpTransport;
use starknet_rs_providers::{JsonRpcClient, Provider, ProviderError};
use starknet_types::chain_id::ChainId;
use starknet_types::rpc::state::Balance;
use starknet_types::serde_helpers::rpc_sierra_contract_class_to_sierra_contract_class::deserialize_to_sierra_contract_class;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, UnixStream};
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};
#[cfg(windows)]
use tokio::signal::windows::ctrl_c;
use tokio::task::{self};
use tokio::time::{interval, sleep};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

mod cli;
mod initial_balance_wrapper;
mod ip_addr_wrapper;
mod metrics;

const REQUEST_LOG_ENV_VAR: &str = "request";
const RESPONSE_LOG_ENV_VAR: &str = "response";

/// Configures tracing with default level INFO,
/// If the environment variable `RUST_LOG` is set, it will be used instead.
/// Added are two more directives: `request` and `response`. If they are present, then have to be
/// removed to be able to construct the `EnvFilter` correctly, because tracing_subscriber recognizes
/// them as path syntax (way to access a module) and assigns them TRACE level. Because they are not
/// paths to some module like this one: `starknet-devnet::cli` nothing gets logged. For example:
/// `RUST_LOG=request` is translated to `request=TRACE`, which means that will log TRACE level for
/// request module.
fn configure_tracing() {
    let log_env_var = std::env::var(EnvFilter::DEFAULT_ENV).unwrap_or_default().to_lowercase();

    // Remove the `request` and `response` directives from the environment variable.
    // And trim empty spaces around each directive
    let log_env_var = log_env_var
        .split(',')
        .map(|el| el.trim())
        .filter(|el| ![REQUEST_LOG_ENV_VAR, RESPONSE_LOG_ENV_VAR].contains(el))
        .collect::<Vec<&str>>()
        .join(",");

    let mut level_filter_layer = EnvFilter::builder()
        .with_default_directive(tracing::Level::INFO.into())
        .parse_lossy(log_env_var);

    if let Ok(cairo_runner_directive) = "cairo_vm::vm::runners::cairo_runner=off".parse() {
        level_filter_layer = level_filter_layer.add_directive(cairo_runner_directive);
    }

    tracing_subscriber::fmt().with_env_filter(level_filter_layer).init();
}

fn log_predeployed_accounts(
    predeployed_accounts: &Vec<Account>,
    seed: u32,
    initial_balance: Balance,
) {
    if let Some(predeployed_account) = predeployed_accounts.first() {
        println!(
            "Predeployed accounts using class {} with hash: {}",
            predeployed_account.class_metadata,
            predeployed_account.class_hash.to_fixed_hex_string()
        );
    }

    for account in predeployed_accounts {
        println!(
            r"
| Account address |  {}
| Private key     |  {}
| Public key      |  {}",
            account.account_address.to_fixed_hex_string(),
            account.keys.private_key.to_fixed_hex_string(),
            account.keys.public_key.to_fixed_hex_string()
        );
    }

    if !predeployed_accounts.is_empty() {
        println!();
        println!("Initial balance of each account: {initial_balance} WEI and FRI");
        println!("Seed to replicate this account sequence: {seed}");
        println!();
    }
}

fn log_predeployed_contracts(config: &StarknetConfig) {
    println!("Predeployed FeeToken");
    println!("ETH Address: 0x{:X}", ETH_ERC20_CONTRACT_ADDRESS);
    println!("Class Hash: 0x{:X}", config.eth_erc20_class_hash);
    println!("STRK Address: 0x{:X}", STRK_ERC20_CONTRACT_ADDRESS);
    println!("Class Hash: 0x{:X}", config.strk_erc20_class_hash);
    println!();
    println!("Predeployed UDC");
    println!("Address: 0x{:X}", UDC_CONTRACT_ADDRESS);
    println!("Class Hash: 0x{:X}", UDC_CONTRACT_CLASS_HASH);
    println!();
}

fn log_other_predeclared_contracts(config: &StarknetConfig) {
    if config.predeclare_argent {
        println!("Predeclared Argent account classes");
        println!("Regular class hash: 0x{:X}", ARGENT_CONTRACT_CLASS_HASH);
        println!("Multisig class hash: 0x{:X}", ARGENT_MULTISIG_CONTRACT_CLASS_HASH);
        println!();
    }
}

fn log_chain_id(chain_id: &ChainId) {
    println!("Chain ID: {} ({})", chain_id, chain_id.to_felt().to_hex_string());
    println!();
}

async fn check_forking_spec_version(
    client: &JsonRpcClient<HttpTransport>,
) -> Result<(), anyhow::Error> {
    let origin_spec_version = client.spec_version().await?;
    if origin_spec_version != RPC_SPEC_VERSION {
        warn!(
            "JSON-RPC API version of origin ({}) does not match this Devnet's version ({}).",
            origin_spec_version, RPC_SPEC_VERSION
        );
    }
    Ok(())
}

// Sets in the starknet_config the class hash and contract class of the ERC20 contracts if they are
// different from the default ones.
async fn set_erc20_contract_class_and_class_hash_if_different_than_default(
    json_rpc_client: &JsonRpcClient<HttpTransport>,
    starknet_config: &mut StarknetConfig,
) -> Result<(), anyhow::Error> {
    let block_id = BlockId::Number(
        starknet_config
            .fork_config
            .block_number
            .ok_or(anyhow::anyhow!("Forking block number is not set"))?,
    );

    async fn get_origin_class_hash_and_contract_class_if_different_from_default(
        json_rpc_client: &JsonRpcClient<HttpTransport>,
        block_id: BlockId,
        contract_address: Felt,
        default_class_hash: Felt,
    ) -> Result<Option<(Felt, String)>, anyhow::Error> {
        match json_rpc_client.get_class_hash_at(block_id, contract_address).await {
            Ok(origin_class_hash) => {
                if origin_class_hash != default_class_hash {
                    tracing::debug!(
                        "Found ERC20 class hash difference at address {contract_address:#x}; \
                         origin={origin_class_hash:#x}, default={default_class_hash:#x}. \
                         Replacing..."
                    );
                    let origin_contract_class =
                        json_rpc_client.get_class(block_id, origin_class_hash).await?;
                    let contract_class_json_str = match origin_contract_class {
                        Sierra(_) => {
                            let contract_class_json_value =
                                serde_json::to_value(origin_contract_class)?;
                            let sierra_contract_class = deserialize_to_sierra_contract_class(
                                contract_class_json_value.into_deserializer(),
                            )?;
                            serde_json::to_string(&sierra_contract_class)?
                        }
                        Legacy(_) => serde_json::to_string(&origin_contract_class)?,
                    };

                    Ok(Some((origin_class_hash, contract_class_json_str)))
                } else {
                    Ok(None)
                }
            }
            // if the contract is not found, then dont return an error. It means that the
            // contract was not deployed at this state of the origin blockchain
            Err(ProviderError::StarknetError(StarknetError::ContractNotFound)) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    if let Some((class_hash, contract_class)) =
        get_origin_class_hash_and_contract_class_if_different_from_default(
            json_rpc_client,
            block_id,
            ETH_ERC20_CONTRACT_ADDRESS,
            starknet_config.eth_erc20_class_hash,
        )
        .await?
    {
        starknet_config.eth_erc20_class_hash = class_hash;
        starknet_config.eth_erc20_contract_class = contract_class;
    }

    if let Some((class_hash, contract_class)) =
        get_origin_class_hash_and_contract_class_if_different_from_default(
            json_rpc_client,
            block_id,
            STRK_ERC20_CONTRACT_ADDRESS,
            starknet_config.strk_erc20_class_hash,
        )
        .await?
    {
        starknet_config.strk_erc20_class_hash = class_hash;
        starknet_config.strk_erc20_contract_class = contract_class;
    }

    Ok(())
}

async fn get_block_hash(
    block_id: BlockId,
    json_rpc_client: &JsonRpcClient<HttpTransport>,
) -> Result<(u64, Felt), anyhow::Error> {
    let block = json_rpc_client.get_block_with_tx_hashes(block_id).await?;

    match block {
        MaybePreConfirmedBlockWithTxHashes::Block(b) => Ok((b.block_number, b.block_hash)),
        MaybePreConfirmedBlockWithTxHashes::PreConfirmedBlock(_) => Err(anyhow::Error::msg(
            "Block deserialized as pre-confirmed, no hash available. Most likely RPC version \
             incompatibility.",
        )),
    }
}

// Originally defined in blockifier, but not worth importing module just for this constant. Probably
// won't change
pub const STORED_BLOCK_HASH_BUFFER: u64 = 10;

/// Logs forking info if forking specified. If block_number is not specified, it is set to the
/// latest block number.
pub async fn set_and_log_fork_config(
    fork_config: &mut ForkConfig,
    json_rpc_client: &JsonRpcClient<HttpTransport>,
) -> Result<(), anyhow::Error> {
    check_forking_spec_version(json_rpc_client).await?;

    let block_id = fork_config.block_number.map_or(BlockId::Tag(BlockTag::Latest), BlockId::Number);

    let (fork_block_number, fork_block_hash) = get_block_hash(block_id, json_rpc_client).await?;

    fork_config.block_number = Some(fork_block_number);
    fork_config.block_hash = Some(fork_block_hash);

    let start = fork_block_number.saturating_sub(STORED_BLOCK_HASH_BUFFER);
    let end = fork_block_number.saturating_sub(1);

    let mut blocks = HashMap::new();

    for i in start..end {
        tracing::info!("Fetching block {:?} into forking struct", i);
        let (num, hash) = get_block_hash(BlockId::Number(i), json_rpc_client).await?;
        blocks.insert(num, hash);
    }
    blocks.insert(fork_block_number, fork_block_hash);

    fork_config.recent_blocks = Some(blocks);

    Ok(())
}

async fn bind_port(
    host: IpAddr,
    specified_port: u16,
) -> Result<(String, TcpListener), anyhow::Error> {
    let binding_address = format!("{host}:{specified_port}");
    let listener = TcpListener::bind(binding_address.clone()).await?;

    let acquired_port = listener.local_addr()?.port();
    Ok((format!("{host}:{acquired_port}"), listener))
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let timestamp = std::time::Instant::now();
    configure_tracing();

    #[cfg(feature = "cairo_native")]
    info!("cairo_native enabled: blockifier will use native execution");

    // parse arguments
    let matches = Args::command().get_matches();
    let args = Args::from_arg_matches(&matches)?;
    let (mut starknet_config, server_config) = args.to_config()?;

    let startup_dump = match args.startup_dump_path()? {
        Some(path) => Some(
            read_dump(path).map_err(|e| anyhow::anyhow!("Failed to read dump at {path}: {e}"))?,
        ),
        None => None,
    };

    // A dump carries the configuration it applies to; legacy dumps are replayed on top of
    // an instance built from the command line.
    let (starknet, legacy_dump, origin_accepted_on_l1_through) = match startup_dump {
        Some(serde_json::Value::Object(_)) if args.legacy_dump_conversion_path().is_some() => {
            anyhow::bail!("The dump at --load-path is not a legacy dump; load it as it is");
        }
        Some(dump @ serde_json::Value::Object(_)) => {
            let dump = StateDump::from_value(dump)?;
            let options = DumpLoadOptions {
                dump_on: starknet_config.dump_on,
                dump_path: starknet_config.dump_path.clone(),
                fork_url: starknet_config.fork_config.url.clone(),
            };
            let dumped_config = dump.starknet_config(&options)?;
            cli::assert_options_match_dump(&matches, &starknet_config, &dumped_config)?;
            if let ForkConfig { url: Some(url), block_number: Some(number), block_hash, .. } =
                &dumped_config.fork_config
            {
                assert_fork_origin_matches(url, *number, *block_hash).await?;
            }

            let origin_accepted_on_l1_through = dump.origin_accepted_on_l1_through;
            (Starknet::from_dump(dump, &options)?, None, origin_accepted_on_l1_through)
        }
        legacy_dump => {
            // If fork url is provided, then set fork config and chain_id from forked network
            if let Some(url) = starknet_config.fork_config.url.as_ref() {
                let json_rpc_client = JsonRpcClient::new(HttpTransport::new(url.clone()));
                set_and_log_fork_config(&mut starknet_config.fork_config, &json_rpc_client).await?;
                set_erc20_contract_class_and_class_hash_if_different_than_default(
                    &json_rpc_client,
                    &mut starknet_config,
                )
                .await?;
                // TODO: Add check for UDC contract
                starknet_config.chain_id = json_rpc_client.chain_id().await?.into();
            }

            let mut starknet = Starknet::new(&starknet_config)?;
            // set block timestamp shift during startup if start time is set
            if let Some(start_time) = starknet_config.start_time {
                starknet.set_block_timestamp_shift(
                    start_time as i64 - Starknet::get_unix_timestamp_as_seconds() as i64,
                );
            };
            (starknet, legacy_dump, None)
        }
    };
    let starknet_config = starknet.config.clone();

    if let Some(output_path) = args.legacy_dump_conversion_path() {
        let legacy_dump = legacy_dump
            .ok_or_else(|| anyhow::anyhow!("No legacy dump found at --load-path to convert"))?;
        return convert_legacy_dump(starknet, server_config, legacy_dump, output_path).await;
    }

    let (address, listener) = bind_port(server_config.host, server_config.port).await?;

    log_chain_id(&starknet_config.chain_id);
    log_predeployed_contracts(&starknet_config);
    log_other_predeclared_contracts(&starknet_config);
    log_predeployed_accounts(
        &starknet.get_predeployed_accounts(),
        starknet_config.seed,
        starknet_config.predeployed_accounts_initial_balance.clone(),
    );

    let api = Api::new(starknet, server_config);
    let ui_enabled = api.server_config.ui_enabled;
    let json_rpc_handler = JsonRpcHandler::new(api.clone());
    json_rpc_handler.restore_origin_acceptance(origin_accepted_on_l1_through).await;
    if let Some(legacy_dump) = legacy_dump {
        let _lifecycle = api.lifecycle.write().await;
        json_rpc_handler
            .load_dump(legacy_dump)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to load dump: {e}"))?;
    }

    let acquired_port = listener.local_addr()?.port();

    let server = serve_http_json_rpc(listener, &api.server_config, json_rpc_handler.clone()).await;
    info!("Starknet Devnet listening on {}", address);
    if ui_enabled {
        info!("Web UI explorer enabled at http://{}/ui", address);
    }

    #[cfg(unix)]
    if let Ok(socket_path) = std::env::var("UNIX_SOCKET") {
        match UnixStream::connect(&socket_path).await {
            Ok(mut stream) => {
                stream.write_all(&acquired_port.to_be_bytes()).await?;
                stream.shutdown().await?;
                println!("Successfully wrote port to unix socket: {}", socket_path);
            }
            Err(e) => {
                println!("Couldn't connect to unix socket: {socket_path}, reason: {e}");
            }
        }
    }

    let mut tasks = vec![];

    // Start metrics server if configured
    if let Some(metrics_addr) = args.get_metrics_addr() {
        let metrics_handle = task::spawn(metrics::start_metrics_server(metrics_addr));
        tasks.push(metrics_handle);
    }

    if let BlockGenerationOn::Interval(seconds) = starknet_config.block_generation_on {
        // use JoinHandle to run block interval creation as a task
        let full_address = format!("http://{address}");
        let block_interval_handle = task::spawn(create_block_interval(seconds, full_address));

        tasks.push(block_interval_handle);
    }

    // run server also as a JoinHandle
    let exit_dump_failure = Arc::new(std::sync::Mutex::new(None));
    let server_handle = task::spawn(
        server
            .with_graceful_shutdown(shutdown_signal(json_rpc_handler, exit_dump_failure.clone()))
            .into_future(),
    );
    tasks.push(server_handle);

    tracing::debug!("Starknet Devnet started in {:.2?}", timestamp.elapsed());

    // join all tasks
    let results = join_all(tasks).await;

    // handle the results of the tasks
    for result in results {
        result??;
    }

    match exit_dump_failure.lock().map_err(|e| anyhow::anyhow!("{e}"))?.take() {
        Some(failure) => Err(failure),
        None => Ok(()),
    }
}

/// Replays `legacy_dump` on top of `starknet` and writes the result as a dump to
/// `output_path`. Transitional, for migrating legacy dumps.
async fn convert_legacy_dump(
    starknet: Starknet,
    server_config: ServerConfig,
    legacy_dump: serde_json::Value,
    output_path: &str,
) -> Result<(), anyhow::Error> {
    let json_rpc_handler = JsonRpcHandler::new(Api::new(starknet, server_config));
    json_rpc_handler
        .load_dump(legacy_dump)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load legacy dump: {e}"))?;
    let dump = json_rpc_handler.state_dump().await?;
    write_dump(&dump, output_path)?;
    info!("Converted the legacy dump and wrote it to {output_path}");
    Ok(())
}

/// Fails if the forking origin's block differs from the one the dump was forked from.
async fn assert_fork_origin_matches(
    url: &url::Url,
    block_number: u64,
    dumped_block_hash: Option<Felt>,
) -> Result<(), anyhow::Error> {
    let json_rpc_client = JsonRpcClient::new(HttpTransport::new(url.clone()));
    check_forking_spec_version(&json_rpc_client).await?;
    let (_, origin_block_hash) =
        get_block_hash(BlockId::Number(block_number), &json_rpc_client).await?;
    if dumped_block_hash != Some(origin_block_hash) {
        anyhow::bail!(
            "Block {block_number} of the forking origin {url} has hash {origin_block_hash:#x}, \
             which differs from the block the dump was forked from"
        );
    }
    Ok(())
}

#[allow(clippy::expect_used)]
async fn create_block_interval(
    block_interval_seconds: u64,
    devnet_address: String,
) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    let mut sigint = { signal(SignalKind::interrupt()).expect("Failed to setup SIGINT handler") };

    #[cfg(windows)]
    let mut sigint = {
        let ctrl_c_signal = ctrl_c().expect("Failed to setup Ctrl+C handler");
        Box::pin(ctrl_c_signal)
    };

    let devnet_client = reqwest::Client::new();
    let block_req_body = json!({ "jsonrpc": "2.0", "id": 0, "method": "devnet_createBlock" });

    // avoid creating block instantly after startup
    sleep(Duration::from_secs(block_interval_seconds)).await;

    let mut interval = interval(Duration::from_secs(block_interval_seconds));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                // By sending a request, we take care of: 1) dumping 2) notifying subscribers
                match devnet_client.post(&devnet_address).json(&block_req_body).send().await {
                    Ok(_) => info!("Generating block on time interval"),
                    Err(e) => error!("Failed block creation on time interval: {e:?}")
                }
            }
            _ = sigint.recv() => {
                return Ok(())
            }
        }
    }
}

/// Waits for Ctrl+C and dumps on exit if configured. A failed dump keeps the previous file and is
/// recorded in `failure`, so the process exits with an error.
#[allow(clippy::expect_used)]
pub async fn shutdown_signal(
    handler: JsonRpcHandler,
    failure: Arc<std::sync::Mutex<Option<anyhow::Error>>>,
) {
    tokio::signal::ctrl_c().await.expect("Failed to install CTRL+C signal handler");

    let config = &handler.api.config;
    if let (Some(DumpOn::Exit), Some(dump_path)) = (config.dump_on, &config.dump_path) {
        let _lifecycle = handler.api.lifecycle.write().await;
        let result = match handler.state_dump().await {
            Ok(dump) => write_dump(&dump, dump_path).map_err(anyhow::Error::from),
            Err(e) => Err(anyhow::Error::from(e)),
        };
        match result {
            Ok(()) => info!("Dumped Devnet to {dump_path}"),
            Err(e) => {
                error!("Failed to dump Devnet to {dump_path}: {e}");
                if let Ok(mut failure) = failure.lock() {
                    *failure = Some(e.context(format!("Failed to dump Devnet to {dump_path}")));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use tracing::level_filters::LevelFilter;
    use tracing_subscriber::EnvFilter;

    use crate::configure_tracing;

    #[test]
    fn test_generated_log_level_from_empty_environment_variable_is_info() {
        assert_environment_variable_sets_expected_log_level("", LevelFilter::INFO);
    }

    fn assert_environment_variable_sets_expected_log_level(
        env_var: &str,
        expected_level: LevelFilter,
    ) {
        unsafe {
            std::env::set_var(EnvFilter::DEFAULT_ENV, env_var);
        }
        configure_tracing();

        assert_eq!(LevelFilter::current(), expected_level);
    }
}
