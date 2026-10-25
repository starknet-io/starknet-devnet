use std::num::NonZeroU128;

use clap::Parser;
use clap::parser::{ArgMatches, ValueSource};
use server::ServerConfig;
use server::api::models::JsonRpcRequest;
use server::restrictive_mode::DEFAULT_RESTRICTED_JSON_RPC_METHODS;
use starknet_core::constants::{
    ARGENT_CONTRACT_VERSION, ARGENT_MULTISIG_CONTRACT_VERSION, DEVNET_DEFAULT_L1_DATA_GAS_PRICE,
    DEVNET_DEFAULT_L1_GAS_PRICE, DEVNET_DEFAULT_L2_GAS_PRICE, DEVNET_DEFAULT_PORT,
    DEVNET_DEFAULT_TIMEOUT, DEVNET_DEFAULT_TOTAL_ACCOUNTS, MAXIMUM_CONTRACT_BYTECODE_SIZE,
    MAXIMUM_CONTRACT_CLASS_SIZE, MAXIMUM_SIERRA_LENGTH, chargeable_account_initial_balance,
};
use starknet_core::contract_class_choice::{AccountClassWrapper, AccountContractClassChoice};
use starknet_core::random_number_generator::generate_u32_random_number;
use starknet_core::starknet::starknet_config::{
    BlockGenerationOn, ClassSizeConfig, DumpOn, ForkConfig, ProofMode, StarknetConfig,
    StateArchiveCapacity,
};
use starknet_types::chain_id::ChainId;
use starknet_types::num_bigint::BigUint;
use tracing_subscriber::EnvFilter;

use crate::initial_balance_wrapper::InitialBalanceWrapper;
use crate::ip_addr_wrapper::IpAddrWrapper;
use crate::{REQUEST_LOG_ENV_VAR, RESPONSE_LOG_ENV_VAR};

/// Run a local instance of Starknet Devnet
#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "A local testnet for Starknet... in Rust!",
    long_about = "Documentation: https://starknet-io.github.io/starknet-devnet",
    propagate_version = true
)]
pub(crate) struct Args {
    /// Number of accounts
    #[arg(long = "accounts")]
    #[arg(env = "ACCOUNTS")]
    #[arg(value_name = "NUMBER")]
    #[arg(default_value_t = DEVNET_DEFAULT_TOTAL_ACCOUNTS)]
    #[arg(help = "Specify the number of accounts to be predeployed;")]
    accounts_count: u8,

    /// Class used for account predeployment
    #[arg(long = "account-class")]
    #[arg(env = "ACCOUNT_CLASS")]
    #[arg(value_name = "ACCOUNT_CLASS")]
    #[arg(default_value = "cairo1")]
    #[arg(help = "Specify the class used by predeployed accounts;")]
    account_class_choice: AccountContractClassChoice,

    #[arg(long = "account-class-custom")]
    #[arg(env = "ACCOUNT_CLASS_CUSTOM")]
    #[arg(value_name = "PATH")]
    #[arg(conflicts_with = "account_class_choice")]
    #[arg(help = "Specify the path to a Cairo Sierra artifact to be used by predeployed accounts;")]
    account_class_custom: Option<AccountClassWrapper>,

    #[arg(long = "predeclare-argent")]
    #[arg(env = "PREDECLARE_ARGENT")]
    #[arg(help = format!(
        "If set, predeclares Argent account contract classes: regular ({ARGENT_CONTRACT_VERSION}) \
        and multisig ({ARGENT_MULTISIG_CONTRACT_VERSION}); does not affect account predeployment;"
    ))]
    predeclare_argent: bool,

    /// Initial balance of predeployed accounts
    #[arg(long = "initial-balance")]
    #[arg(env = "INITIAL_BALANCE")]
    #[arg(short = 'e')]
    #[arg(value_name = "DECIMAL_VALUE")]
    #[arg(default_value_t = InitialBalanceWrapper::default())]
    #[arg(help = "Specify the initial balance of accounts to be predeployed; the value is \
                  interpreted as WEI for ETH balance and FRI for STRK balance;")]
    initial_balance: InitialBalanceWrapper,

    // Seed for predeployed accounts
    #[arg(long = "seed")]
    #[arg(env = "SEED")]
    #[arg(value_name = "SEED")]
    #[arg(help = "Specify the seed for randomness of accounts to be predeployed; if not \
                  provided, it is randomly generated")]
    seed: Option<u32>,

    // Host address
    #[arg(long = "host")]
    #[arg(env = "HOST")]
    #[arg(value_name = "HOST")]
    #[arg(default_value_t = IpAddrWrapper::LOCALHOST)]
    #[arg(help = "Specify the address to listen at;")]
    host: IpAddrWrapper,

    // Port number
    #[arg(long = "port")]
    #[arg(env = "PORT")]
    #[arg(value_name = "PORT")]
    #[arg(default_value_t = DEVNET_DEFAULT_PORT)]
    #[arg(help = "Specify the port to listen at; If 0, acquires a random free port and prints it;")]
    port: u16,

    // Set start time in seconds
    #[arg(long = "start-time")]
    #[arg(env = "START_TIME")]
    #[arg(value_name = "SECONDS")]
    #[arg(help = "Specify start time in seconds;")]
    start_time: Option<u64>,

    // Server timeout in seconds
    #[arg(long = "timeout")]
    #[arg(env = "TIMEOUT")]
    #[arg(value_name = "SECONDS")]
    #[arg(default_value_t = DEVNET_DEFAULT_TIMEOUT)]
    #[arg(help = "Specify the server timeout in seconds;")]
    timeout: u16,

    // Gas price in wei
    #[arg(long = "gas-price")]
    #[arg(env = "GAS_PRICE")]
    #[arg(value_name = "WEI_PER_GAS_UNIT")]
    #[arg(default_value_t = DEVNET_DEFAULT_L1_GAS_PRICE)]
    #[arg(help = "Specify the gas price in wei per L1 gas unit;")]
    gas_price_wei: NonZeroU128,

    // Gas price in fri
    #[arg(long = "gas-price-fri")]
    #[arg(env = "GAS_PRICE_FRI")]
    #[arg(value_name = "FRI_PER_GAS_UNIT")]
    #[arg(default_value_t = DEVNET_DEFAULT_L1_GAS_PRICE)]
    #[arg(help = "Specify the gas price in fri per L1 gas unit;")]
    gas_price_fri: NonZeroU128,

    // TODO perhaps make this affect only fri and remove -fri flags
    // Gas price in wei
    #[arg(long = "data-gas-price")]
    #[arg(env = "DATA_GAS_PRICE")]
    #[arg(value_name = "WEI_PER_GAS_UNIT")]
    #[arg(default_value_t = DEVNET_DEFAULT_L1_DATA_GAS_PRICE)]
    #[arg(help = "Specify the gas price in wei per L1 data gas unit;")]
    data_gas_price_wei: NonZeroU128,

    // Gas price in fri
    #[arg(long = "data-gas-price-fri")]
    #[arg(env = "DATA_GAS_PRICE_FRI")]
    #[arg(value_name = "FRI_PER_GAS_UNIT")]
    #[arg(default_value_t = DEVNET_DEFAULT_L1_DATA_GAS_PRICE)]
    #[arg(help = "Specify the gas price in fri per L1 data gas unit;")]
    data_gas_price_fri: NonZeroU128,

    // L2 Gas price in wei
    #[arg(long = "l2-gas-price")]
    #[arg(env = "L2_GAS_PRICE")]
    #[arg(value_name = "WEI_PER_GAS_UNIT")]
    #[arg(default_value_t = DEVNET_DEFAULT_L2_GAS_PRICE)]
    #[arg(help = "Specify the gas price in wei per L2 gas unit;")]
    l2_gas_price_wei: NonZeroU128,

    // L2 Gas price in fri
    #[arg(long = "l2-gas-price-fri")]
    #[arg(env = "L2_GAS_PRICE_FRI")]
    #[arg(value_name = "FRI_PER_GAS_UNIT")]
    #[arg(default_value_t = DEVNET_DEFAULT_L2_GAS_PRICE)]
    #[arg(help = "Specify the gas price in fri per L2 gas unit;")]
    l2_gas_price_fri: NonZeroU128,

    #[arg(long = "chain-id")]
    #[arg(env = "CHAIN_ID")]
    #[arg(value_name = "CHAIN_ID")]
    #[arg(default_value = "TESTNET")]
    #[arg(help = "Specify the chain ID. Possible values are:
- \"MAINNET\", \"TESTNET\" - predefined chain IDs
- <USER_SUPPLIED> - custom chain ID, defined by user. Have to contain only ASCII characters.")]
    #[arg(conflicts_with = "fork_network")]
    chain_id: ChainId,

    #[arg(long = "dump-on")]
    #[arg(env = "DUMP_ON")]
    #[arg(value_name = "EVENT")]
    #[arg(value_parser = parse_dump_on)]
    #[arg(help = "Specify when to dump the state of Devnet to --dump-path. Possible values are:
- \"exit\" - on graceful shutdown (e.g. Ctrl+C); the previous file is kept if writing fails
Dumps can also be requested at any time via devnet_dump;")]
    dump_on: Option<DumpOn>,

    #[arg(long = "lite-mode")]
    #[arg(env = "LITE_MODE")]
    #[arg(help = "Specify whether to run in lite mode and skip block hash calculation;")]
    lite_mode: bool,

    #[arg(long = "proof-mode")]
    #[arg(env = "PROOF_MODE")]
    #[arg(default_value = "devnet")]
    #[arg(help = "Specify how to calculate and verify transaction proofs. Possible values are:
- \"full\" - proofs are generated and verified fully
- \"devnet\" - devnet creates fake proof that it later verifies
- \"none\" - proof field of transactions is ignored
")]
    proof_mode: ProofMode,

    // Dump path as string
    #[arg(long = "dump-path")]
    #[arg(env = "DUMP_PATH")]
    #[arg(value_name = "DUMP_PATH")]
    #[arg(help = "Specify the path to dump to. Without --load-path, Devnet also loads from this \
                  path on startup if the file exists;")]
    #[arg(required_if_eq("dump_on", "exit"))]
    dump_path: Option<String>,

    #[arg(long = "load-path")]
    #[arg(env = "LOAD_PATH")]
    #[arg(value_name = "LOAD_PATH")]
    #[arg(help = "Specify the path of a dump to load on startup;")]
    load_path: Option<String>,

    #[arg(long = "convert-legacy-dump")]
    #[arg(value_name = "OUTPUT_PATH")]
    #[arg(requires = "load_path")]
    #[arg(help = "Transitional: load the legacy dump at --load-path, write it in the current \
                  format to OUTPUT_PATH and exit without starting the server. Specify the \
                  options the legacy dump was created with;")]
    convert_legacy_dump: Option<String>,

    #[arg(long = "block-generation-on")]
    #[arg(env = "BLOCK_GENERATION_ON")]
    #[arg(default_value = "transaction")]
    #[arg(help = "Specify when to generate a new block. Possible values are:
- \"transaction\" - new block generated on each transaction
- \"demand\" - new block creatable solely by calling the devnet_createBlock JSON-RPC method
- <INTERVAL> - a positive integer indicating after how many seconds a new block is generated

Calling devnet_createBlock JSON-RPC method is also an option in modes other than \"demand\".")]
    block_generation_on: BlockGenerationOn,

    #[arg(long = "state-archive-capacity")]
    #[arg(env = "STATE_ARCHIVE_CAPACITY")]
    #[arg(value_name = "STATE_ARCHIVE_CAPACITY")]
    #[arg(default_value = "none")]
    #[arg(help = "Specify the state archive capacity;")]
    state_archive: StateArchiveCapacity,

    #[arg(long = "fork-network")]
    #[arg(env = "FORK_NETWORK")]
    #[arg(value_name = "URL")]
    #[arg(help = "Specify the URL of the network to fork;")]
    fork_network: Option<url::Url>,

    #[arg(long = "fork-block")]
    #[arg(env = "FORK_BLOCK")]
    #[arg(value_name = "BLOCK_NUMBER")]
    #[arg(help = "Specify the number of the block to fork at;")]
    #[arg(requires = "fork_network")]
    fork_block: Option<u64>,

    #[arg(long = "fork-upstream-caching")]
    #[arg(env = "FORK_UPSTREAM_CACHING")]
    #[arg(help = "Enable caching of fork upstream data;")]
    #[arg(requires = "fork_network")]
    fork_upstream_caching: Option<bool>,

    #[arg(long = "restrictive-mode")]
    #[arg(env = "RESTRICTIVE_MODE")]
    #[arg(num_args = 0..)]
    #[arg(help = "Use Devnet in restrictive mode; You can specify the methods that will be \
                  forbidden with whitespace-separated values (https://starknet-io.github.io/starknet-devnet/docs/restrictive#with-a-list-of-methods). If nothing is specified for this \
                  argument, then default restricted methods are used (https://starknet-io.github.io/starknet-devnet/docs/restrictive#default-restricted-methods).")]
    restricted_methods: Option<Vec<String>>,

    #[arg(long = "maximum-contract-class-size")]
    #[arg(env = "MAXIMUM_CONTRACT_CLASS_SIZE")]
    #[arg(value_name = "BYTES")]
    #[arg(default_value_t = MAXIMUM_CONTRACT_CLASS_SIZE)]
    maximum_contract_class_size: u64,

    #[arg(long = "maximum-contract-bytecode-size")]
    #[arg(env = "MAXIMUM_CONTRACT_BYTECODE_SIZE")]
    #[arg(value_name = "FELTS")]
    #[arg(default_value_t = MAXIMUM_CONTRACT_BYTECODE_SIZE)]
    maximum_contract_bytecode_size: u64,

    #[arg(long = "maximum-sierra-length")]
    #[arg(env = "MAXIMUM_SIERRA_LENGTH")]
    #[arg(value_name = "FELTS")]
    #[arg(default_value_t = MAXIMUM_SIERRA_LENGTH)]
    maximum_sierra_length: u64,

    #[arg(long = "metrics-host")]
    #[arg(env = "METRICS_HOST")]
    #[arg(value_name = "METRICS_HOST")]
    #[arg(help = "Specify the address for the metrics server to listen at; If not provided, \
                  metrics server is not started;")]
    metrics_host: Option<IpAddrWrapper>,

    #[arg(long = "metrics-port")]
    #[arg(env = "METRICS_PORT")]
    #[arg(value_name = "METRICS_PORT")]
    #[arg(default_value_t = 9090)]
    #[arg(help = "Specify the port for the metrics server to listen at;")]
    metrics_port: u16,

    /// Serve web UI explorer at /ui
    #[arg(long = "ui")]
    #[arg(env = "UI")]
    #[arg(help = "Serve the embedded web UI explorer at /ui;")]
    ui_enabled: bool,
}

fn parse_dump_on(value: &str) -> Result<DumpOn, String> {
    match value {
        "exit" => Ok(DumpOn::Exit),
        "block" | "request" => Err(format!(
            "`--dump-on {value}` was removed; dumps are returned by devnet_dump at any time, and \
             `--dump-on exit` writes one on shutdown"
        )),
        _ => Err("possible values: exit".to_string()),
    }
}

/// Rejects explicitly specified options that differ from the configuration of a dump loaded on
/// startup, whose configuration applies. The forking URL may be replaced and is not checked.
pub(crate) fn assert_options_match_dump(
    matches: &ArgMatches,
    specified: &StarknetConfig,
    dumped: &StarknetConfig,
) -> Result<(), anyhow::Error> {
    let options = [
        ("accounts_count", "--accounts", specified.total_accounts == dumped.total_accounts),
        (
            "account_class_choice",
            "--account-class",
            specified.account_contract_class_hash == dumped.account_contract_class_hash,
        ),
        (
            "account_class_custom",
            "--account-class-custom",
            specified.account_contract_class_hash == dumped.account_contract_class_hash,
        ),
        (
            "predeclare_argent",
            "--predeclare-argent",
            specified.predeclare_argent == dumped.predeclare_argent,
        ),
        (
            "initial_balance",
            "--initial-balance",
            specified.predeployed_accounts_initial_balance
                == dumped.predeployed_accounts_initial_balance,
        ),
        ("seed", "--seed", specified.seed == dumped.seed),
        ("start_time", "--start-time", specified.start_time == dumped.start_time),
        ("gas_price_wei", "--gas-price", specified.gas_price_wei == dumped.gas_price_wei),
        ("gas_price_fri", "--gas-price-fri", specified.gas_price_fri == dumped.gas_price_fri),
        (
            "data_gas_price_wei",
            "--data-gas-price",
            specified.data_gas_price_wei == dumped.data_gas_price_wei,
        ),
        (
            "data_gas_price_fri",
            "--data-gas-price-fri",
            specified.data_gas_price_fri == dumped.data_gas_price_fri,
        ),
        (
            "l2_gas_price_wei",
            "--l2-gas-price",
            specified.l2_gas_price_wei == dumped.l2_gas_price_wei,
        ),
        (
            "l2_gas_price_fri",
            "--l2-gas-price-fri",
            specified.l2_gas_price_fri == dumped.l2_gas_price_fri,
        ),
        ("chain_id", "--chain-id", specified.chain_id.to_felt() == dumped.chain_id.to_felt()),
        ("lite_mode", "--lite-mode", specified.lite_mode == dumped.lite_mode),
        ("proof_mode", "--proof-mode", specified.proof_mode == dumped.proof_mode),
        (
            "block_generation_on",
            "--block-generation-on",
            specified.block_generation_on == dumped.block_generation_on,
        ),
        (
            "state_archive",
            "--state-archive-capacity",
            specified.state_archive == dumped.state_archive,
        ),
        (
            "fork_block",
            "--fork-block",
            specified.fork_config.block_number == dumped.fork_config.block_number,
        ),
        (
            "fork_upstream_caching",
            "--fork-upstream-caching",
            specified.fork_config.caching_enabled == dumped.fork_config.caching_enabled,
        ),
        (
            "maximum_contract_class_size",
            "--maximum-contract-class-size",
            specified.class_size_config.maximum_contract_class_size
                == dumped.class_size_config.maximum_contract_class_size,
        ),
        (
            "maximum_contract_bytecode_size",
            "--maximum-contract-bytecode-size",
            specified.class_size_config.maximum_contract_bytecode_size
                == dumped.class_size_config.maximum_contract_bytecode_size,
        ),
        (
            "maximum_sierra_length",
            "--maximum-sierra-length",
            specified.class_size_config.maximum_sierra_length
                == dumped.class_size_config.maximum_sierra_length,
        ),
    ];

    let conflicting: Vec<&str> = options
        .into_iter()
        .filter(|(id, _, matching)| {
            !matching
                && matches!(
                    matches.value_source(id),
                    Some(ValueSource::CommandLine | ValueSource::EnvVariable)
                )
        })
        .map(|(_, flag, _)| flag)
        .collect();
    if conflicting.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "The loaded dump was created with a different {}; omit the option to use the dump's \
             configuration",
            conflicting.join(", ")
        )
    }
}

impl Args {
    /// Returns where to write a legacy dump converted to the current format, if requested.
    pub(crate) fn legacy_dump_conversion_path(&self) -> Option<&str> {
        self.convert_legacy_dump.as_deref()
    }

    /// Returns the path of the dump to load on startup, if any.
    pub(crate) fn startup_dump_path(&self) -> Result<Option<&str>, anyhow::Error> {
        match (&self.load_path, &self.dump_path) {
            (Some(load_path), _) => {
                if !std::path::Path::new(load_path).exists() {
                    anyhow::bail!("No dump found at --load-path {load_path}");
                }
                Ok(Some(load_path))
            }
            // The dump file is created on exit, so it may not exist yet.
            (None, Some(dump_path)) if std::path::Path::new(dump_path).exists() => {
                Ok(Some(dump_path))
            }
            (None, _) => Ok(None),
        }
    }

    pub(crate) fn get_metrics_addr(&self) -> Option<std::net::SocketAddr> {
        self.metrics_host
            .as_ref()
            .map(|host| std::net::SocketAddr::new(host.inner, self.metrics_port))
    }

    pub(crate) fn to_config(&self) -> Result<(StarknetConfig, ServerConfig), anyhow::Error> {
        // use account-class-custom if specified; otherwise default to predefined account-class
        let account_class_wrapper = match &self.account_class_custom {
            Some(account_class_custom) => account_class_custom.clone(),
            None => self.account_class_choice.get_class_wrapper()?,
        };

        let starknet_config = StarknetConfig {
            seed: match self.seed {
                Some(seed) => seed,
                None => generate_u32_random_number(),
            },
            total_accounts: self.accounts_count,
            account_contract_class: account_class_wrapper.contract_class,
            account_contract_class_hash: account_class_wrapper.class_hash,
            predeployed_accounts_initial_balance: self.initial_balance.0.clone(),
            start_time: self.start_time,
            gas_price_wei: self.gas_price_wei,
            gas_price_fri: self.gas_price_fri,
            data_gas_price_wei: self.data_gas_price_wei,
            data_gas_price_fri: self.data_gas_price_fri,
            l2_gas_price_wei: self.l2_gas_price_wei,
            l2_gas_price_fri: self.l2_gas_price_fri,
            chain_id: self.chain_id,
            dump_on: self.dump_on,
            dump_path: self.dump_path.clone(),
            block_generation_on: self.block_generation_on,
            lite_mode: self.lite_mode,
            proof_mode: self.proof_mode,
            state_archive: self.state_archive,
            fork_config: ForkConfig {
                url: self.fork_network.clone(),
                block_number: self.fork_block,
                block_hash: None,
                recent_blocks: None,
                caching_enabled: self.fork_upstream_caching,
            },
            predeclare_argent: self.predeclare_argent,
            class_size_config: ClassSizeConfig {
                maximum_contract_class_size: self.maximum_contract_class_size,
                maximum_contract_bytecode_size: self.maximum_contract_bytecode_size,
                maximum_sierra_length: self.maximum_sierra_length,
            },
            ..Default::default()
        };

        let RequestResponseLogging { log_request, log_response } =
            RequestResponseLogging::from_rust_log_environment_variable();

        self.validate_total_account_balance()?;

        let server_config = ServerConfig {
            host: self.host.inner,
            port: self.port,
            timeout: self.timeout,
            log_request,
            log_response,
            restricted_methods: self.get_validated_restricted_methods()?,
            ui_enabled: self.ui_enabled,
        };

        Ok((starknet_config, server_config))
    }

    /// n_accounts * balance(single_account) + chargeable_balance < u256 capacity
    fn validate_total_account_balance(&self) -> Result<(), anyhow::Error> {
        let total_supply = self.accounts_count * self.initial_balance.0.clone()
            + chargeable_account_initial_balance();
        if total_supply >= (BigUint::from(1_u32) << 256) {
            anyhow::bail!(
                "Total balance supply in ERC20 contract exceeded. Reduce the number of \
                 predeployed accounts (--accounts) or their initial balance (--initial-balance)."
            )
        }

        Ok(())
    }

    /// Errors if there are unsupported restricted methods/routes.
    fn get_validated_restricted_methods(&self) -> Result<Option<Vec<String>>, anyhow::Error> {
        match &self.restricted_methods {
            None => Ok(None),
            Some(methods) if methods.is_empty() => Ok(Some(
                DEFAULT_RESTRICTED_JSON_RPC_METHODS.iter().map(|m| m.to_string()).collect(),
            )),
            Some(methods) => {
                let json_rpc_methods = JsonRpcRequest::all_variants_serde_renames();
                let wrong_restricted_methods: Vec<_> = methods
                    .iter()
                    .filter(|method| !json_rpc_methods.contains(method))
                    .cloned()
                    .collect();
                if !wrong_restricted_methods.is_empty() {
                    anyhow::bail!(
                        "Restricted methods contain unsupported JSON-RPC methods: {}",
                        wrong_restricted_methods.join(", ")
                    );
                }
                Ok(Some(methods.clone()))
            }
        }
    }
}

struct RequestResponseLogging {
    log_request: bool,
    log_response: bool,
}

impl RequestResponseLogging {
    fn from_rust_log_environment_variable() -> Self {
        let log_env_var = std::env::var(EnvFilter::DEFAULT_ENV).unwrap_or_default().to_lowercase();
        let has_directive =
            |directive: &str| log_env_var.split(',').map(str::trim).any(|item| item == directive);
        let log_request = has_directive(REQUEST_LOG_ENV_VAR);
        let log_response = has_directive(RESPONSE_LOG_ENV_VAR);

        Self { log_request, log_response }
    }
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, FromArgMatches, Parser};
    use starknet_core::constants::CAIRO_1_ACCOUNT_CONTRACT_SIERRA_PATH;
    use starknet_core::starknet::starknet_config::{
        BlockGenerationOn, DumpOn, StateArchiveCapacity,
    };
    use starknet_types::num_bigint::BigUint;
    use tracing_subscriber::EnvFilter;

    use super::{Args, RequestResponseLogging, assert_options_match_dump};
    use crate::ip_addr_wrapper::IpAddrWrapper;

    const CAIRO_0_ERC20_CONTRACT_PATH: &str =
        "../../contracts/test_artifacts/cairo0/ERC20_Mintable_OZ_0.2.0.json";

    #[test]
    fn rejects_mempool_options() {
        for args in [
            ["--", "--block-generation-on", "mempool"],
            ["--", "--mempool-ordering", "fifo"],
            ["--", "--mempool-random-seed", "1"],
            ["--", "--mempool-max-transactions-per-block", "500"],
        ] {
            assert!(Args::try_parse_from(args).is_err(), "accepted {args:?}");
        }
    }

    #[test]
    fn valid_ip_address() {
        Args::parse_from(["--", "--host", "127.0.0.1"]);
    }

    #[test]
    fn invalid_ip_address() {
        let invalid_ip_address = "127.0.0";
        match Args::try_parse_from(["--", "--host", invalid_ip_address]) {
            Err(_) => (),
            Ok(parsed) => panic!("Should have failed; got: {parsed:?}"),
        }
    }

    #[test]
    fn localhost_mapped() {
        let args = Args::parse_from(["--", "--host", "localhost"]);
        assert_eq!(args.host, IpAddrWrapper::LOCALHOST);
    }

    #[test]
    fn invalid_hostname() {
        match Args::try_parse_from(["--", "--host", "invalid"]) {
            Err(_) => (),
            Ok(parsed) => panic!("Should have failed; got: {parsed:?}"),
        }
    }

    #[test]
    fn state_archive_default_none() {
        let args = Args::parse_from(["--"]);
        let (starknet_config, _) = args.to_config().unwrap();
        assert_eq!(starknet_config.state_archive, StateArchiveCapacity::None);
    }

    #[test]
    fn state_archive_none() {
        let args = Args::parse_from(["--", "--state-archive-capacity", "none"]);
        let (starknet_config, _) = args.to_config().unwrap();
        assert_eq!(starknet_config.state_archive, StateArchiveCapacity::None);
    }

    #[test]
    fn state_archive_full() {
        let args = Args::parse_from(["--", "--state-archive-capacity", "full"]);
        let (starknet_config, _) = args.to_config().unwrap();
        assert_eq!(starknet_config.state_archive, StateArchiveCapacity::Full);
    }

    fn get_first_line(text: &str) -> &str {
        text.split('\n').next().unwrap()
    }

    #[test]
    fn not_allowing_account_class_and_account_class_path() {
        match Args::try_parse_from([
            "--",
            "--account-class",
            "cairo1",
            "--account-class-custom",
            CAIRO_1_ACCOUNT_CONTRACT_SIERRA_PATH,
        ]) {
            Err(err) => {
                assert_eq!(
                    get_first_line(&err.to_string()),
                    "error: the argument '--account-class <ACCOUNT_CLASS>' cannot be used with \
                     '--account-class-custom <PATH>'"
                );
            }
            Ok(parsed) => panic!("Should have failed; got: {parsed:?}"),
        }
    }

    #[test]
    fn allowing_if_only_account_class() {
        match Args::try_parse_from(["--", "--account-class", "cairo1"]) {
            Ok(_) => (),
            Err(err) => panic!("Should have passed; got: {err}"),
        }
    }

    #[test]
    fn allowing_if_only_account_class_path() {
        match Args::try_parse_from([
            "--",
            "--account-class-custom",
            CAIRO_1_ACCOUNT_CONTRACT_SIERRA_PATH,
        ]) {
            Ok(_) => (),
            Err(err) => panic!("Should have passed; got: {err}"),
        }
    }

    #[test]
    fn not_allowing_regular_cairo0_contract_as_custom_account() {
        let custom_path = CAIRO_0_ERC20_CONTRACT_PATH;
        match Args::try_parse_from(["--", "--account-class-custom", custom_path]) {
            Err(err) => assert_eq!(
                get_first_line(&err.to_string()),
                format!(
                    "error: invalid value '{custom_path}' for '--account-class-custom <PATH>': \
                     Types error: missing field `kind` at line 1 column 292"
                )
            ),
            Ok(parsed) => panic!("Should have failed; got: {parsed:?}"),
        }
    }

    #[test]
    fn not_allowing_regular_cairo1_contract_as_custom_account() {
        // path to a regular cairo1 contract (not an account)
        let custom_path = "../../contracts/test_artifacts/cairo1/simple_contract/output.sierra";
        match Args::try_parse_from(["--", "--account-class-custom", custom_path]) {
            Err(err) => assert_eq!(
                get_first_line(&err.to_string()),
                format!(
                    "error: invalid value '{custom_path}' for '--account-class-custom <PATH>': \
                     Failed to load ContractClass: Not a valid Sierra account artifact; has \
                     __execute__: false; has __validate__: false"
                )
            ),
            Ok(parsed) => panic!("Should have failed; got: {parsed:?}"),
        }
    }

    #[test]
    fn not_allowing_invalid_url_as_fork_network() {
        for url in ["abc", "", "http://"] {
            match Args::try_parse_from(["--", "--fork-network", url]) {
                Err(_) => (),
                Ok(parsed) => panic!("Should fail for {url}; got: {parsed:?}"),
            }
        }
    }

    #[test]
    fn allowing_valid_url_as_fork_network() {
        for url in [
            "https://free-rpc.nethermind.io/mainnet-juno/v0_6",
            "http://localhost/",
            "http://localhost:5051/",
            "http://127.0.0.1/",
            "http://127.0.0.1:5050/",
            "https://localhost/",
            "https://localhost:5050/",
        ] {
            match Args::try_parse_from(["--", "--fork-network", url]) {
                Ok(args) => assert_eq!(args.fork_network.unwrap().to_string(), url),
                Err(e) => panic!("Should have passed; got: {e}"),
            }
        }
    }

    #[test]
    fn not_allowing_fork_block_without_fork_network() {
        match Args::try_parse_from(["--", "--fork-block", "12"]) {
            Err(_) => (),
            Ok(parsed) => panic!("Should fail when just --fork-block got: {parsed:?}"),
        }
    }

    #[test]
    fn not_allowing_invalid_value_as_fork_block() {
        for number in ["", "abc", "-1"] {
            match Args::try_parse_from([
                "--",
                "--fork-network",
                "http://localhost:5051",
                "--fork-block",
                number,
            ]) {
                Err(_) => (),
                Ok(parsed) => panic!("Should fail for {number}; got: {parsed:?}"),
            }
        }
    }

    #[test]
    fn allowing_number_as_fork_block() {
        for number in [0, 1, 42, 999] {
            match Args::try_parse_from([
                "--",
                "--fork-network",
                "http://localhost:5051",
                "--fork-block",
                &number.to_string(),
            ]) {
                Ok(args) => assert_eq!(args.fork_block, Some(number)),
                Err(e) => panic!("Should have passed; got: {e}"),
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_variants_of_env_var() {
        for (environment_variable, should_log_request, should_log_response) in [
            ("request,response,info", true, true),
            ("request,info", true, false),
            ("response,info", false, true),
            ("info", false, false),
            ("", false, false),
            ("REQUEST,RESPONSE", true, true),
            ("REQUEST", true, false),
            ("RESPONSE", false, true),
            (" request , response ", true, true),
            ("request_handler=trace,response_cache=debug", false, false),
            ("starknet_devnet::request=trace", false, false),
        ] {
            unsafe {
                std::env::set_var(EnvFilter::DEFAULT_ENV, environment_variable);
            }
            let RequestResponseLogging { log_request, log_response } =
                RequestResponseLogging::from_rust_log_environment_variable();

            assert_eq!(log_request, should_log_request);
            assert_eq!(log_response, should_log_response);
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_env_vars_have_same_effect_as_cli_params() {
        let config_source = [
            ("--accounts", "ACCOUNTS", "1"),
            ("--account-class", "ACCOUNT_CLASS", "cairo0"),
            ("--initial-balance", "INITIAL_BALANCE", "1"),
            ("--seed", "SEED", "43"),
            ("--port", "PORT", "1234"),
            ("--start-time", "START_TIME", "123"),
            ("--timeout", "TIMEOUT", "12"),
            ("--gas-price", "GAS_PRICE", "1"),
            ("--gas-price-fri", "GAS_PRICE_FRI", "2"),
            ("--data-gas-price", "DATA_GAS_PRICE", "3"),
            ("--data-gas-price-fri", "DATA_GAS_PRICE_FRI", "4"),
            ("--dump-on", "DUMP_ON", "exit"),
            ("--dump-path", "DUMP_PATH", "dummy-path"),
            ("--state-archive-capacity", "STATE_ARCHIVE_CAPACITY", "full"),
            ("--fork-network", "FORK_NETWORK", "http://dummy.com"),
            ("--fork-block", "FORK_BLOCK", "42"),
            ("--block-generation-on", "BLOCK_GENERATION_ON", "demand"),
        ];

        let mut cli_args = vec!["--"];
        for (cli_param, _, value) in config_source {
            cli_args.push(cli_param);
            cli_args.push(value);
        }

        let config_via_cli = Args::parse_from(cli_args).to_config().unwrap();

        for (_, var_name, value) in config_source {
            unsafe {
                std::env::set_var(var_name, value);
            }
        }
        let config_via_env = Args::parse_from(["--"]).to_config().unwrap();

        assert_eq!(
            serde_json::to_value(config_via_cli).unwrap(),
            serde_json::to_value(config_via_env).unwrap()
        );

        // remove var to avoid collision with other tests
        for (_, var_name, _) in config_source {
            unsafe {
                std::env::remove_var(var_name);
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn test_boolean_param_specification_via_env_vars() {
        let config_source =
            [("--lite-mode", "LITE_MODE"), ("--predeclare-argent", "PREDECLARE_ARGENT")];

        let mut cli_args = vec!["--"];
        for (cli_param, _) in config_source {
            cli_args.push(cli_param);
        }

        let mut config_via_cli =
            serde_json::to_value(Args::parse_from(cli_args).to_config().unwrap()).unwrap();

        for (_, var_name) in config_source {
            unsafe {
                std::env::set_var(var_name, "true");
            }
        }
        let mut config_via_env =
            serde_json::to_value(Args::parse_from(["--"]).to_config().unwrap()).unwrap();

        // Removing seed as it is generated randomly - it would make the compared objects different.
        // to_config returns two config parts, so using index 0 to address starknet_config.
        config_via_cli[0]["seed"].take();
        config_via_env[0]["seed"].take();

        assert_eq!(config_via_cli, config_via_env);

        // remove var to avoid collision with other tests
        for (var_name, _) in config_source {
            unsafe {
                std::env::remove_var(var_name);
            }
        }
    }

    #[test]
    fn not_allowing_invalid_values_as_block_generation_interval() {
        for interval in ["", "0", "-1", "abc"] {
            match Args::try_parse_from(["--", "--block-generation-on", interval]) {
                Err(_) => (),
                Ok(parsed) => panic!("Should fail for {interval}; got: {parsed:?}"),
            }
        }
    }

    #[test]
    fn allowing_valid_values_as_block_generation_interval() {
        match Args::try_parse_from(["--", "--block-generation-on", "1"]) {
            Ok(args) => assert_eq!(args.block_generation_on, BlockGenerationOn::Interval(1)),
            Err(e) => panic!("Should have passed; got: {e}"),
        }

        match Args::try_parse_from(["--", "--block-generation-on", "demand"]) {
            Ok(args) => assert_eq!(args.block_generation_on, BlockGenerationOn::Demand),
            Err(e) => panic!("Should have passed; got: {e}"),
        }

        match Args::try_parse_from(["--", "--block-generation-on", "transaction"]) {
            Ok(args) => assert_eq!(args.block_generation_on, BlockGenerationOn::Transaction),
            Err(e) => panic!("Should have passed; got: {e}"),
        }
    }

    #[test]
    fn test_when_dump_path_flag_required() {
        match Args::try_parse_from(["--", "--dump-on", "exit"]) {
            Ok(args) => panic!("Should have failed; got: {args:?}"),
            Err(e) => assert_eq!(
                get_first_line(&e.to_string()),
                "error: the following required arguments were not provided:"
            ),
        }
        match Args::try_parse_from(["--", "--dump-on", "exit", "--dump-path", "dump.json"]) {
            Ok(Args { dump_on: Some(DumpOn::Exit), .. }) => (),
            other => panic!("Invalid arg parse result: {other:?}"),
        }
    }

    #[test]
    fn invalid_dump_path_not_allowed() {
        match Args::try_parse_from(["--", "--dump-path", "dump_wrong_cli_mode", "--dump-on", "e"]) {
            Ok(args) => panic!("Should have failed; got: {args:?}"),
            Err(e) => assert_eq!(
                get_first_line(&e.to_string()),
                "error: invalid value 'e' for '--dump-on <EVENT>': possible values: exit"
            ),
        }
    }

    #[test]
    fn removed_dump_modes_are_rejected() {
        for mode in ["block", "request"] {
            match Args::try_parse_from(["--", "--dump-on", mode, "--dump-path", "dump.json"]) {
                Ok(args) => panic!("Should have failed; got: {args:?}"),
                Err(e) => assert!(
                    get_first_line(&e.to_string())
                        .contains(&format!("`--dump-on {mode}` was removed")),
                    "{e}"
                ),
            }
        }
    }

    #[test]
    fn legacy_dump_conversion_requires_load_path() {
        match Args::try_parse_from(["--", "--convert-legacy-dump", "converted.json"]) {
            Ok(args) => panic!("Should have failed; got: {args:?}"),
            Err(e) => assert_eq!(
                get_first_line(&e.to_string()),
                "error: the following required arguments were not provided:"
            ),
        }
        let args = Args::try_parse_from([
            "--",
            "--convert-legacy-dump",
            "converted.json",
            "--load-path",
            "legacy.json",
        ])
        .unwrap();
        assert_eq!(args.legacy_dump_conversion_path(), Some("converted.json"));
    }

    #[test]
    fn startup_loads_from_load_path_or_existing_dump_path() {
        let existing = std::env::temp_dir().join(format!("devnet-cli-dump-{}", std::process::id()));
        std::fs::write(&existing, "{}").unwrap();
        let existing = existing.to_str().unwrap();
        let missing = format!("{existing}-missing");

        let path = |args: &[&str]| {
            let args = Args::try_parse_from([&["--"], args].concat()).unwrap();
            args.startup_dump_path().map(|path| path.map(str::to_string))
        };
        assert_eq!(path(&["--load-path", existing]).unwrap().as_deref(), Some(existing));
        assert!(path(&["--load-path", &missing]).is_err());
        assert_eq!(path(&["--dump-path", existing]).unwrap().as_deref(), Some(existing));
        assert_eq!(path(&["--dump-path", &missing]).unwrap(), None);
        assert_eq!(
            path(&["--dump-path", &missing, "--load-path", existing]).unwrap().as_deref(),
            Some(existing)
        );
        std::fs::remove_file(existing).unwrap();
    }

    #[test]
    #[serial_test::serial]
    fn options_conflicting_with_a_loaded_dump_are_rejected() {
        let dumped = Args::parse_from(["--", "--seed", "1"]).to_config().unwrap().0;
        let check = |args: &[&str]| {
            let matches = Args::command().try_get_matches_from([&["--"], args].concat()).unwrap();
            let specified = Args::from_arg_matches(&matches).unwrap().to_config().unwrap().0;
            assert_options_match_dump(&matches, &specified, &dumped)
        };

        // Unspecified options and specified options with the dumped values are accepted.
        check(&[]).unwrap();
        check(&["--seed", "1", "--port", "1234", "--fork-network", "http://localhost:1"]).unwrap();

        let error = check(&["--seed", "2", "--accounts", "3", "--lite-mode"]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "The loaded dump was created with a different --accounts, --seed, --lite-mode; omit \
             the option to use the dump's configuration"
        );
    }

    #[test]
    fn check_if_method_with_incorrect_name_will_produce_an_error() {
        let args = Args::parse_from([
            "--",
            "--restrictive-mode",
            "devnet_dump",
            "devnet_loadd",
            "xyz",
            "/mint",
        ]);
        let err = args.to_config().unwrap_err();
        assert!(err.to_string().contains(
            "Restricted methods contain unsupported JSON-RPC methods: devnet_loadd, xyz, /mint"
        ));
    }

    #[test]
    fn check_if_methods_with_correct_names_will_not_produce_an_error() {
        Args::parse_from(["--", "--restrictive-mode"]).to_config().unwrap();

        Args::parse_from(["--", "--restrictive-mode", "devnet_dump", "devnet_mint"])
            .to_config()
            .unwrap();
    }

    #[test]
    fn should_not_exceed_total_supply() {
        let initial_balance: BigUint = (BigUint::from(1_u32) << 254) - 1_u32;
        Args::parse_from([
            "--",
            "--accounts",
            "2",
            "--initial-balance",
            &initial_balance.to_str_radix(10),
        ])
        .to_config()
        .unwrap();
    }

    #[test]
    /// Fails because 2 * (2 ** 254) + (2 ** 255) = 2 ** 256
    fn should_fail_on_exceeded_total_supply() {
        let initial_balance: BigUint = BigUint::from(1_u32) << 254;
        let err = Args::parse_from([
            "--",
            "--accounts",
            "2",
            "--initial-balance",
            &initial_balance.to_str_radix(10),
        ])
        .to_config()
        .unwrap_err();

        assert!(err.to_string().contains(
            "Total balance supply in ERC20 contract exceeded. Reduce the number of predeployed \
             accounts (--accounts) or their initial balance (--initial-balance)."
        ))
    }
}
