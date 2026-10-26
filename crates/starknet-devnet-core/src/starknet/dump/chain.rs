//! Encoding of configuration, blocks, transactions, controls and messaging.

use std::collections::{HashMap, HashSet};
use std::num::NonZeroU128;
use std::sync::Arc;

use alloy::primitives::B256;
use blockifier::context::BlockContext;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use starknet_api::block::{BlockHeader, BlockHeaderWithoutHash, BlockNumber, BlockTimestamp};
use starknet_api::core::{
    EventCommitment, ReceiptCommitment, StateDiffCommitment, TransactionCommitment,
};
use starknet_api::hash::PoseidonHash;
use starknet_rs_core::types::{ExecutionResult, Felt};
use starknet_types::chain_id::ChainId;
use starknet_types::contract_address::ContractAddress;
use starknet_types::contract_class::ContractClass;
use starknet_types::felt::felt_from_prefixed_hex;
use starknet_types::patricia_key::StorageKey;
use starknet_types::rpc::block::BlockStatus;
use starknet_types::rpc::gas_modification::GasModification;
use starknet_types::rpc::messaging::MessageToL1;
use starknet_types::rpc::state::{Balance, ReplacedClasses};
use starknet_types::rpc::transactions::{
    TransactionFinalityStatus, TransactionTrace, TransactionWithHash,
};
use url::Url;

use super::state::{ClassDefinitions, ClassTable, invalid_dump, sorted};
use crate::blocks::{StarknetBlock, StarknetBlocks};
use crate::constants::{
    CAIRO_0_ACCOUNT_CONTRACT_HASH, CAIRO_1_ACCOUNT_CONTRACT_SIERRA_HASH,
    ETH_ERC20_CONTRACT_ADDRESS, ETH_ERC20_CONTRACT_CLASS, ETH_ERC20_CONTRACT_CLASS_HASH,
    STRK_ERC20_CONTRACT_ADDRESS, STRK_ERC20_CONTRACT_CLASS, STRK_ERC20_CONTRACT_CLASS_HASH,
};
use crate::contract_class_choice::AccountContractClassChoice;
use crate::error::DevnetResult;
use crate::messaging::MessagingBroker;
use crate::messaging::ethereum::{EthereumMessaging, EthereumMessagingDump};
use crate::starknet::Starknet;
use crate::starknet::cheats::Cheats;
use crate::starknet::starknet_config::{
    BlockGenerationOn, ClassSizeConfig, ForkConfig, ProofMode, StarknetConfig, StateArchiveCapacity,
};
use crate::state::state_diff::StateDiff;
use crate::transactions::{StarknetTransaction, StarknetTransactions, TransactionOutput};

/// Serializes integers wider than JSON-safe numbers as decimal strings.
mod decimal {
    use std::fmt::Display;
    use std::str::FromStr;

    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<T: Display, S: Serializer>(
        value: &T,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_str(value)
    }

    pub(super) fn deserialize<'de, T, D>(deserializer: D) -> Result<T, D::Error>
    where
        T: FromStr,
        T::Err: Display,
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ChainIdDump {
    Mainnet,
    Testnet,
    Custom(Felt),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ForkDump {
    url: Option<String>,
    block_number: Option<u64>,
    block_hash: Option<Felt>,
    recent_blocks: Option<Vec<(u64, Felt)>>,
    caching_enabled: Option<bool>,
}

/// Chain configuration. Classes are referenced by hash and stored once in the class table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConfigDump {
    seed: u32,
    total_accounts: u8,
    account_contract_class_hash: Felt,
    #[serde(with = "decimal")]
    predeployed_accounts_initial_balance: Balance,
    start_time: Option<u64>,
    #[serde(with = "decimal")]
    gas_price_wei: NonZeroU128,
    #[serde(with = "decimal")]
    gas_price_fri: NonZeroU128,
    #[serde(with = "decimal")]
    data_gas_price_wei: NonZeroU128,
    #[serde(with = "decimal")]
    data_gas_price_fri: NonZeroU128,
    #[serde(with = "decimal")]
    l2_gas_price_wei: NonZeroU128,
    #[serde(with = "decimal")]
    l2_gas_price_fri: NonZeroU128,
    chain_id: ChainIdDump,
    block_generation_on: BlockGenerationOn,
    lite_mode: bool,
    proof_mode: ProofMode,
    state_archive: StateArchiveCapacity,
    fork: ForkDump,
    eth_erc20_class_hash: Felt,
    strk_erc20_class_hash: Felt,
    predeclare_argent: bool,
    class_size_config: ClassSizeConfig,
}

impl ConfigDump {
    pub(super) fn encode(config: &StarknetConfig, table: &mut ClassTable) -> DevnetResult<Self> {
        let StarknetConfig {
            seed,
            total_accounts,
            account_contract_class,
            account_contract_class_hash,
            predeployed_accounts_initial_balance,
            start_time,
            gas_price_wei,
            gas_price_fri,
            data_gas_price_wei,
            data_gas_price_fri,
            l2_gas_price_wei,
            l2_gas_price_fri,
            chain_id,
            // Settings of the running instance, supplied on load.
            dump_on: _,
            dump_path: _,
            block_generation_on,
            lite_mode,
            proof_mode,
            state_archive,
            fork_config,
            eth_erc20_class_hash,
            strk_erc20_class_hash,
            eth_erc20_contract_class,
            strk_erc20_contract_class,
            predeclare_argent,
            class_size_config,
        } = config;

        // Classes built into Devnet are restored from the binary; others are stored.
        if built_in_account_class(*account_contract_class_hash)?.is_none() {
            table.insert(*account_contract_class_hash, account_contract_class);
        }
        for (class_hash, class_json) in [
            (eth_erc20_class_hash, eth_erc20_contract_class),
            (strk_erc20_class_hash, strk_erc20_contract_class),
        ] {
            if built_in_erc20_class(*class_hash).is_none() {
                let class =
                    ContractClass::Cairo1(ContractClass::cairo_1_from_sierra_json_str(class_json)?);
                table.insert(*class_hash, &class);
            }
        }

        let ForkConfig { url, block_number, block_hash, recent_blocks, caching_enabled } =
            fork_config;

        Ok(Self {
            seed: *seed,
            total_accounts: *total_accounts,
            account_contract_class_hash: *account_contract_class_hash,
            predeployed_accounts_initial_balance: predeployed_accounts_initial_balance.clone(),
            start_time: *start_time,
            gas_price_wei: *gas_price_wei,
            gas_price_fri: *gas_price_fri,
            data_gas_price_wei: *data_gas_price_wei,
            data_gas_price_fri: *data_gas_price_fri,
            l2_gas_price_wei: *l2_gas_price_wei,
            l2_gas_price_fri: *l2_gas_price_fri,
            chain_id: match chain_id {
                ChainId::Mainnet => ChainIdDump::Mainnet,
                ChainId::Testnet => ChainIdDump::Testnet,
                ChainId::Custom(felt) => ChainIdDump::Custom(*felt),
            },
            block_generation_on: *block_generation_on,
            lite_mode: *lite_mode,
            proof_mode: *proof_mode,
            state_archive: *state_archive,
            fork: ForkDump {
                url: url.as_ref().map(Url::to_string),
                block_number: *block_number,
                block_hash: *block_hash,
                recent_blocks: recent_blocks.as_ref().map(|blocks| sorted(blocks.clone())),
                caching_enabled: *caching_enabled,
            },
            eth_erc20_class_hash: *eth_erc20_class_hash,
            strk_erc20_class_hash: *strk_erc20_class_hash,
            predeclare_argent: *predeclare_argent,
            class_size_config: class_size_config.clone(),
        })
    }

    /// Returns the hashes of the classes the configuration refers to.
    pub(super) fn class_hashes(&self) -> [Felt; 3] {
        [self.account_contract_class_hash, self.eth_erc20_class_hash, self.strk_erc20_class_hash]
    }

    pub(super) fn decode(
        self,
        definitions: &ClassDefinitions,
        options: &super::DumpLoadOptions,
    ) -> DevnetResult<StarknetConfig> {
        let Self {
            seed,
            total_accounts,
            account_contract_class_hash,
            predeployed_accounts_initial_balance,
            start_time,
            gas_price_wei,
            gas_price_fri,
            data_gas_price_wei,
            data_gas_price_fri,
            l2_gas_price_wei,
            l2_gas_price_fri,
            chain_id,
            block_generation_on,
            lite_mode,
            proof_mode,
            state_archive,
            fork,
            eth_erc20_class_hash,
            strk_erc20_class_hash,
            predeclare_argent,
            class_size_config,
        } = self;

        let erc20_class_json = |class_hash: Felt| {
            if let Some(class_json) = built_in_erc20_class(class_hash) {
                return Ok(class_json.to_string());
            }
            match definitions.get(class_hash)? {
                ContractClass::Cairo1(sierra) => serde_json::to_string(sierra)
                    .map_err(|e| crate::error::Error::SerializationError { origin: e.to_string() }),
                ContractClass::Cairo0(_) => Err(invalid_dump(format!(
                    "fee token class {class_hash:#x} must be a Cairo 1 class"
                ))),
            }
        };
        let account_contract_class = match built_in_account_class(account_contract_class_hash)? {
            Some(choice) => choice.get_class_wrapper()?.contract_class,
            None => definitions.get(account_contract_class_hash)?.clone(),
        };

        let ForkDump { url, block_number, block_hash, recent_blocks, caching_enabled } = fork;
        let url = match (url, &options.fork_url) {
            (Some(_), Some(override_url)) => Some(override_url.clone()),
            (Some(url), None) => Some(
                Url::parse(&url)
                    .map_err(|e| invalid_dump(format!("Invalid fork URL '{url}': {e}")))?,
            ),
            (None, Some(_)) => {
                return Err(invalid_dump("A fork URL was supplied for a dump that is not forked"));
            }
            (None, None) => None,
        };

        Ok(StarknetConfig {
            seed,
            total_accounts,
            account_contract_class,
            account_contract_class_hash,
            predeployed_accounts_initial_balance,
            start_time,
            gas_price_wei,
            gas_price_fri,
            data_gas_price_wei,
            data_gas_price_fri,
            l2_gas_price_wei,
            l2_gas_price_fri,
            chain_id: match chain_id {
                ChainIdDump::Mainnet => ChainId::Mainnet,
                ChainIdDump::Testnet => ChainId::Testnet,
                ChainIdDump::Custom(felt) => ChainId::Custom(felt),
            },
            dump_on: options.dump_on,
            dump_path: options.dump_path.clone(),
            block_generation_on,
            lite_mode,
            proof_mode,
            state_archive,
            fork_config: ForkConfig {
                url,
                block_number,
                block_hash,
                recent_blocks: recent_blocks.map(|blocks| blocks.into_iter().collect()),
                caching_enabled,
            },
            eth_erc20_class_hash,
            strk_erc20_class_hash,
            eth_erc20_contract_class: erc20_class_json(eth_erc20_class_hash)?,
            strk_erc20_contract_class: erc20_class_json(strk_erc20_class_hash)?,
            predeclare_argent,
            class_size_config,
        })
    }
}

/// Returns the built-in account class choice with `class_hash`, if there is one.
fn built_in_account_class(class_hash: Felt) -> DevnetResult<Option<AccountContractClassChoice>> {
    Ok(if class_hash == felt_from_prefixed_hex(CAIRO_0_ACCOUNT_CONTRACT_HASH)? {
        Some(AccountContractClassChoice::Cairo0)
    } else if class_hash == felt_from_prefixed_hex(CAIRO_1_ACCOUNT_CONTRACT_SIERRA_HASH)? {
        Some(AccountContractClassChoice::Cairo1)
    } else {
        None
    })
}

/// Returns the built-in fee token class with `class_hash`, if there is one.
fn built_in_erc20_class(class_hash: Felt) -> Option<&'static str> {
    [
        (ETH_ERC20_CONTRACT_CLASS_HASH, ETH_ERC20_CONTRACT_CLASS),
        (STRK_ERC20_CONTRACT_CLASS_HASH, STRK_ERC20_CONTRACT_CLASS),
    ]
    .into_iter()
    .find_map(|(built_in_hash, class_json)| (built_in_hash == class_hash).then_some(class_json))
}

/// Changes introduced by a block, or accumulated in the pre-confirmed block.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StateDiffDump {
    storage_updates: Vec<(Felt, Vec<(Felt, Felt)>)>,
    nonces: Vec<(Felt, Felt)>,
    class_hashes: Vec<(Felt, Felt)>,
    compiled_class_hashes: Vec<(Felt, Felt)>,
    declared_classes: Vec<Felt>,
    cairo_0_declared_classes: Vec<Felt>,
    replaced_classes: Vec<ReplacedClasses>,
}

impl StateDiffDump {
    pub(super) fn encode(state_diff: &StateDiff) -> Self {
        let StateDiff {
            storage_updates,
            address_to_nonce,
            address_to_class_hash,
            class_hash_to_compiled_class_hash,
            declared_contracts,
            cairo_0_declared_contracts,
            replaced_classes,
        } = state_diff;

        Self {
            storage_updates: sorted(storage_updates.iter().map(|(address, updates)| {
                let updates = sorted(updates.iter().map(|(key, value)| (key.to_felt(), *value)));
                (Felt::from(*address), updates)
            })),
            nonces: sorted(
                address_to_nonce.iter().map(|(address, nonce)| (Felt::from(*address), *nonce)),
            ),
            class_hashes: sorted(
                address_to_class_hash
                    .iter()
                    .map(|(address, class_hash)| (Felt::from(*address), *class_hash)),
            ),
            compiled_class_hashes: sorted(
                class_hash_to_compiled_class_hash.iter().map(|(class_hash, c)| (*class_hash, *c)),
            ),
            declared_classes: declared_contracts.clone(),
            cairo_0_declared_classes: cairo_0_declared_contracts.clone(),
            replaced_classes: replaced_classes.clone(),
        }
    }

    pub(super) fn decode(self) -> DevnetResult<StateDiff> {
        let Self {
            storage_updates,
            nonces,
            class_hashes,
            compiled_class_hashes,
            declared_classes,
            cairo_0_declared_classes,
            replaced_classes,
        } = self;

        let mut decoded_storage_updates = HashMap::with_capacity(storage_updates.len());
        for (address, updates) in storage_updates {
            let mut decoded_updates = HashMap::with_capacity(updates.len());
            for (key, value) in updates {
                decoded_updates.insert(StorageKey::new(key)?, value);
            }
            decoded_storage_updates.insert(ContractAddress::new(address)?, decoded_updates);
        }
        let mut address_to_nonce = HashMap::with_capacity(nonces.len());
        for (address, nonce) in nonces {
            address_to_nonce.insert(ContractAddress::new(address)?, nonce);
        }
        let mut address_to_class_hash = HashMap::with_capacity(class_hashes.len());
        for (address, class_hash) in class_hashes {
            address_to_class_hash.insert(ContractAddress::new(address)?, class_hash);
        }

        Ok(StateDiff {
            storage_updates: decoded_storage_updates,
            address_to_nonce,
            address_to_class_hash,
            class_hash_to_compiled_class_hash: compiled_class_hashes.into_iter().collect(),
            declared_contracts: declared_classes,
            cairo_0_declared_contracts: cairo_0_declared_classes,
            replaced_classes,
        })
    }
}

/// A block header. `starknet_api` skips the commitments and counts when serializing its header,
/// so they are stored explicitly.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockHeaderDump {
    block_hash: Felt,
    block_header_without_hash: BlockHeaderWithoutHash,
    state_diff_commitment: Option<Felt>,
    state_diff_length: Option<usize>,
    transaction_commitment: Option<Felt>,
    event_commitment: Option<Felt>,
    n_transactions: usize,
    n_events: usize,
    receipt_commitment: Option<Felt>,
}

impl From<&BlockHeader> for BlockHeaderDump {
    fn from(header: &BlockHeader) -> Self {
        let BlockHeader {
            block_hash,
            block_header_without_hash,
            state_diff_commitment,
            state_diff_length,
            transaction_commitment,
            event_commitment,
            n_transactions,
            n_events,
            receipt_commitment,
        } = header;
        Self {
            block_hash: block_hash.0,
            block_header_without_hash: block_header_without_hash.clone(),
            state_diff_commitment: state_diff_commitment.map(|commitment| commitment.0.0),
            state_diff_length: *state_diff_length,
            transaction_commitment: transaction_commitment.map(|commitment| commitment.0),
            event_commitment: event_commitment.map(|commitment| commitment.0),
            n_transactions: *n_transactions,
            n_events: *n_events,
            receipt_commitment: receipt_commitment.map(|commitment| commitment.0),
        }
    }
}

impl From<BlockHeaderDump> for BlockHeader {
    fn from(header: BlockHeaderDump) -> Self {
        let BlockHeaderDump {
            block_hash,
            block_header_without_hash,
            state_diff_commitment,
            state_diff_length,
            transaction_commitment,
            event_commitment,
            n_transactions,
            n_events,
            receipt_commitment,
        } = header;
        Self {
            block_hash: starknet_api::block::BlockHash(block_hash),
            block_header_without_hash,
            state_diff_commitment: state_diff_commitment
                .map(|commitment| StateDiffCommitment(PoseidonHash(commitment))),
            state_diff_length,
            transaction_commitment: transaction_commitment.map(TransactionCommitment),
            event_commitment: event_commitment.map(EventCommitment),
            n_transactions,
            n_events,
            receipt_commitment: receipt_commitment.map(ReceiptCommitment),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockDump {
    header: BlockHeaderDump,
    status: BlockStatus,
    transaction_hashes: Vec<Felt>,
}

impl BlockDump {
    fn encode(block: &StarknetBlock) -> Self {
        Self {
            header: BlockHeaderDump::from(&block.header),
            status: *block.status(),
            transaction_hashes: block.get_transactions().clone(),
        }
    }

    fn decode(self) -> StarknetBlock {
        let Self { header, status, transaction_hashes } = self;
        StarknetBlock::from_parts(header.into(), transaction_hashes, status)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptedBlockDump {
    block: BlockDump,
    state_diff: StateDiffDump,
}

/// Accepted blocks, in block order, and the pre-confirmed block.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BlocksDump {
    last_block_hash: Option<Felt>,
    last_accepted_on_l1: Option<Felt>,
    aborted_blocks: Vec<Felt>,
    /// The genesis block. Its state diff is produced by the genesis of the configuration.
    genesis: BlockDump,
    /// Blocks accepted after genesis, each with the state diff it introduced.
    accepted: Vec<AcceptedBlockDump>,
    pre_confirmed: BlockDump,
    pre_confirmed_state_diff: StateDiffDump,
}

impl BlocksDump {
    pub(super) fn encode(
        blocks: &StarknetBlocks,
        pre_confirmed_state_diff: &StateDiff,
    ) -> DevnetResult<Self> {
        let StarknetBlocks {
            num_to_hash,
            hash_to_block,
            pre_confirmed_block,
            last_block_hash,
            hash_to_state_diff,
            hash_to_state,
            aborted_blocks,
            starting_block_number,
            last_accepted_on_l1,
        } = blocks;

        // Archived states are rebuilt from the genesis and the block state diffs, so either every
        // accepted block or none has one.
        if hash_to_block.len() != num_to_hash.len()
            || hash_to_state_diff.len() != num_to_hash.len()
            || !(hash_to_state.is_empty() || hash_to_state.len() == num_to_hash.len())
        {
            return Err(inconsistent_blocks());
        }

        let mut accepted_hashes = num_to_hash.iter();
        let genesis = match accepted_hashes.next() {
            Some((number, hash)) if number.0 == *starting_block_number => {
                hash_to_block.get(hash).ok_or_else(inconsistent_blocks)?
            }
            _ => return Err(inconsistent_blocks()),
        };
        let mut accepted = Vec::with_capacity(num_to_hash.len() - 1);
        for (_, block_hash) in accepted_hashes {
            let block = hash_to_block.get(block_hash).ok_or_else(inconsistent_blocks)?;
            let state_diff = hash_to_state_diff.get(block_hash).ok_or_else(inconsistent_blocks)?;
            accepted.push(AcceptedBlockDump {
                block: BlockDump::encode(block),
                state_diff: StateDiffDump::encode(state_diff),
            });
        }

        Ok(Self {
            last_block_hash: *last_block_hash,
            last_accepted_on_l1: *last_accepted_on_l1,
            aborted_blocks: aborted_blocks.clone(),
            genesis: BlockDump::encode(genesis),
            accepted,
            pre_confirmed: BlockDump::encode(pre_confirmed_block),
            pre_confirmed_state_diff: StateDiffDump::encode(pre_confirmed_state_diff),
        })
    }

    /// Returns the blocks, without archived states, and the pre-confirmed state diff.
    /// `genesis_blocks` are the blocks produced by the genesis of the configuration.
    pub(super) fn decode(
        self,
        genesis_blocks: &StarknetBlocks,
    ) -> DevnetResult<(StarknetBlocks, StateDiff)> {
        let Self {
            last_block_hash,
            last_accepted_on_l1,
            aborted_blocks,
            genesis,
            accepted,
            pre_confirmed,
            pre_confirmed_state_diff,
        } = self;

        let genesis = genesis.decode();
        let starting_block_number = genesis_blocks.starting_block_number;
        let genesis_state_diff = genesis_blocks
            .num_to_hash
            .get(&BlockNumber(starting_block_number))
            .and_then(|hash| genesis_blocks.hash_to_state_diff.get(hash))
            .ok_or_else(|| invalid_dump("the genesis of this Devnet has no genesis block"))?;
        if genesis.block_number().0 != starting_block_number {
            return Err(invalid_dump(format!(
                "genesis block number {} differs from {starting_block_number}",
                genesis.block_number()
            )));
        }

        let mut num_to_hash = IndexMap::with_capacity(accepted.len() + 1);
        let mut hash_to_block = HashMap::with_capacity(accepted.len() + 1);
        let mut hash_to_state_diff = HashMap::with_capacity(accepted.len() + 1);
        let mut blocks_and_diffs = Vec::with_capacity(accepted.len() + 1);
        blocks_and_diffs.push((genesis, genesis_state_diff.clone()));
        for AcceptedBlockDump { block, state_diff } in accepted {
            blocks_and_diffs.push((block.decode(), state_diff.decode()?));
        }
        for (block, state_diff) in blocks_and_diffs {
            let block_hash = block.block_hash();
            if num_to_hash.insert(block.block_number(), block_hash).is_some()
                || hash_to_state_diff.insert(block_hash, state_diff).is_some()
            {
                return Err(invalid_dump(format!(
                    "block {block_hash:#x} is listed more than once"
                )));
            }
            hash_to_block.insert(block_hash, block);
        }

        let blocks = StarknetBlocks {
            num_to_hash,
            hash_to_block,
            pre_confirmed_block: pre_confirmed.decode(),
            last_block_hash,
            hash_to_state_diff,
            hash_to_state: HashMap::new(),
            aborted_blocks,
            starting_block_number,
            last_accepted_on_l1,
        };
        Ok((blocks, pre_confirmed_state_diff.decode()?))
    }
}

fn inconsistent_blocks() -> crate::error::Error {
    crate::error::Error::UnexpectedInternalError {
        msg: "Cannot dump inconsistent block collections".to_string(),
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TransactionDump {
    transaction: TransactionWithHash,
    finality_status: TransactionFinalityStatus,
    execution_result: ExecutionResult,
    block_hash: Option<Felt>,
    block_number: Option<u64>,
    output: TransactionOutput,
    trace: Option<TransactionTrace>,
}

pub(super) fn encode_transactions(transactions: &StarknetTransactions) -> Vec<TransactionDump> {
    transactions
        .iter()
        .map(|transaction| {
            let StarknetTransaction {
                inner,
                finality_status,
                execution_result,
                block_hash,
                block_number,
                output,
                trace,
            } = transaction;
            TransactionDump {
                transaction: inner.clone(),
                finality_status: *finality_status,
                execution_result: execution_result.clone(),
                block_hash: *block_hash,
                block_number: block_number.map(|number| number.0),
                output: output.as_ref().clone(),
                trace: trace.as_deref().cloned(),
            }
        })
        .collect()
}

pub(super) fn decode_transactions(
    transactions: Vec<TransactionDump>,
) -> DevnetResult<StarknetTransactions> {
    let mut decoded = StarknetTransactions::default();
    for TransactionDump {
        transaction,
        finality_status,
        execution_result,
        block_hash,
        block_number,
        output,
        trace,
    } in transactions
    {
        let transaction_hash = *transaction.get_transaction_hash();
        if decoded.get(&transaction_hash).is_some() {
            return Err(invalid_dump(format!(
                "transaction {transaction_hash:#x} is listed more than once"
            )));
        }
        decoded.insert(
            &transaction_hash,
            StarknetTransaction {
                inner: transaction,
                finality_status,
                execution_result,
                block_hash,
                block_number: block_number.map(BlockNumber),
                output: Arc::new(output),
                trace: trace.map(Arc::new),
            },
        );
    }
    Ok(decoded)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GasPricesDump {
    #[serde(with = "decimal")]
    gas_price_wei: NonZeroU128,
    #[serde(with = "decimal")]
    data_gas_price_wei: NonZeroU128,
    #[serde(with = "decimal")]
    gas_price_fri: NonZeroU128,
    #[serde(with = "decimal")]
    data_gas_price_fri: NonZeroU128,
    #[serde(with = "decimal")]
    l2_gas_price_wei: NonZeroU128,
    #[serde(with = "decimal")]
    l2_gas_price_fri: NonZeroU128,
}

impl From<&GasModification> for GasPricesDump {
    fn from(gas: &GasModification) -> Self {
        let GasModification {
            gas_price_wei,
            data_gas_price_wei,
            gas_price_fri,
            data_gas_price_fri,
            l2_gas_price_wei,
            l2_gas_price_fri,
        } = gas;
        Self {
            gas_price_wei: *gas_price_wei,
            data_gas_price_wei: *data_gas_price_wei,
            gas_price_fri: *gas_price_fri,
            data_gas_price_fri: *data_gas_price_fri,
            l2_gas_price_wei: *l2_gas_price_wei,
            l2_gas_price_fri: *l2_gas_price_fri,
        }
    }
}

impl From<GasPricesDump> for GasModification {
    fn from(gas: GasPricesDump) -> Self {
        let GasPricesDump {
            gas_price_wei,
            data_gas_price_wei,
            gas_price_fri,
            data_gas_price_fri,
            l2_gas_price_wei,
            l2_gas_price_fri,
        } = gas;
        Self {
            gas_price_wei,
            data_gas_price_wei,
            gas_price_fri,
            data_gas_price_fri,
            l2_gas_price_wei,
            l2_gas_price_fri,
        }
    }
}

/// The parts of the execution context that are not fixed by the configuration.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockContextDump {
    block_number: u64,
    block_timestamp: u64,
    gas_prices: GasPricesDump,
}

impl BlockContextDump {
    fn encode(block_context: &BlockContext) -> DevnetResult<Self> {
        let block_info = block_context.block_info();
        let price = |price: starknet_api::block::NonzeroGasPrice| {
            NonZeroU128::new(price.get().0).ok_or_else(|| {
                crate::error::Error::UnexpectedInternalError {
                    msg: "Block context has a zero gas price".to_string(),
                }
            })
        };
        let eth = &block_info.gas_prices.eth_gas_prices;
        let strk = &block_info.gas_prices.strk_gas_prices;
        Ok(Self {
            block_number: block_info.block_number.0,
            block_timestamp: block_info.block_timestamp.0,
            gas_prices: GasPricesDump {
                gas_price_wei: price(eth.l1_gas_price)?,
                data_gas_price_wei: price(eth.l1_data_gas_price)?,
                gas_price_fri: price(strk.l1_gas_price)?,
                data_gas_price_fri: price(strk.l1_data_gas_price)?,
                l2_gas_price_wei: price(eth.l2_gas_price)?,
                l2_gas_price_fri: price(strk.l2_gas_price)?,
            },
        })
    }

    fn decode(self, config: &StarknetConfig) -> BlockContext {
        let Self { block_number, block_timestamp, gas_prices } = self;
        let GasPricesDump {
            gas_price_wei,
            data_gas_price_wei,
            gas_price_fri,
            data_gas_price_fri,
            l2_gas_price_wei,
            l2_gas_price_fri,
        } = gas_prices;

        let mut block_context = Starknet::init_block_context(
            gas_price_wei,
            gas_price_fri,
            data_gas_price_wei,
            data_gas_price_fri,
            l2_gas_price_wei,
            l2_gas_price_fri,
            ETH_ERC20_CONTRACT_ADDRESS,
            STRK_ERC20_CONTRACT_ADDRESS,
            config.chain_id,
            block_number,
        );
        Starknet::update_block_context_block_timestamp(
            &mut block_context,
            BlockTimestamp(block_timestamp),
        );
        block_context
    }
}

/// Time, gas and impersonation controls, and the execution context derived from them.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ControlsDump {
    pre_confirmed_block_timestamp_shift: i64,
    next_block_timestamp: Option<u64>,
    next_block_gas: GasPricesDump,
    block_context: BlockContextDump,
    impersonated_accounts: Vec<Felt>,
    auto_impersonate: bool,
}

/// Controls decoded from a dump.
pub(super) struct Controls {
    pub(super) pre_confirmed_block_timestamp_shift: i64,
    pub(super) next_block_timestamp: Option<u64>,
    pub(super) next_block_gas: GasModification,
    pub(super) block_context: BlockContext,
    pub(super) cheats: Cheats,
}

impl ControlsDump {
    pub(super) fn encode(
        pre_confirmed_block_timestamp_shift: i64,
        next_block_timestamp: Option<u64>,
        next_block_gas: &GasModification,
        block_context: &BlockContext,
        cheats: &Cheats,
    ) -> DevnetResult<Self> {
        Ok(Self {
            pre_confirmed_block_timestamp_shift,
            next_block_timestamp,
            next_block_gas: next_block_gas.into(),
            block_context: BlockContextDump::encode(block_context)?,
            impersonated_accounts: sorted(
                cheats.get_impersonated_accounts().iter().map(|address| Felt::from(*address)),
            ),
            auto_impersonate: cheats.is_auto_impersonate(),
        })
    }

    pub(super) fn decode(self, config: &StarknetConfig) -> DevnetResult<Controls> {
        let Self {
            pre_confirmed_block_timestamp_shift,
            next_block_timestamp,
            next_block_gas,
            block_context,
            impersonated_accounts,
            auto_impersonate,
        } = self;

        let mut accounts = HashSet::with_capacity(impersonated_accounts.len());
        for address in impersonated_accounts {
            accounts.insert(ContractAddress::new(address)?);
        }

        Ok(Controls {
            pre_confirmed_block_timestamp_shift,
            next_block_timestamp,
            next_block_gas: next_block_gas.into(),
            block_context: block_context.decode(config),
            cheats: Cheats::from_parts(accounts, auto_impersonate),
        })
    }
}

/// Devnet-owned L1-L2 messaging state.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MessagingDump {
    ethereum: Option<EthereumMessagingDump>,
    last_local_block: u64,
    l2_to_l1_messages_hashes: Vec<(B256, u64)>,
    l2_to_l1_messages_to_flush: Vec<MessageToL1>,
    l1_to_l2_tx_hashes: Vec<(B256, Vec<Felt>)>,
}

impl MessagingDump {
    pub(super) fn encode(messaging: &MessagingBroker) -> Self {
        let MessagingBroker {
            ethereum,
            last_local_block,
            l2_to_l1_messages_hashes,
            l2_to_l1_messages_to_flush,
            l1_to_l2_tx_hashes,
        } = messaging;

        Self {
            ethereum: ethereum.as_ref().map(EthereumMessaging::to_dump),
            last_local_block: *last_local_block,
            l2_to_l1_messages_hashes: sorted(
                l2_to_l1_messages_hashes.iter().map(|(hash, count)| (*hash, *count)),
            ),
            l2_to_l1_messages_to_flush: l2_to_l1_messages_to_flush.clone(),
            l1_to_l2_tx_hashes: sorted(
                l1_to_l2_tx_hashes.iter().map(|(hash, l2_hashes)| (*hash, l2_hashes.clone())),
            ),
        }
    }

    pub(super) fn decode(self) -> DevnetResult<MessagingBroker> {
        let Self {
            ethereum,
            last_local_block,
            l2_to_l1_messages_hashes,
            l2_to_l1_messages_to_flush,
            l1_to_l2_tx_hashes,
        } = self;

        Ok(MessagingBroker {
            ethereum: ethereum.map(EthereumMessaging::from_dump).transpose()?,
            last_local_block,
            l2_to_l1_messages_hashes: l2_to_l1_messages_hashes.into_iter().collect(),
            l2_to_l1_messages_to_flush,
            l1_to_l2_tx_hashes: l1_to_l2_tx_hashes.into_iter().collect(),
        })
    }
}
