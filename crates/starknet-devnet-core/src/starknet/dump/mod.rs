//! Versioned persistence of all Devnet-owned state.
//!
//! A dump holds the configuration and everything that happened after genesis. Loading
//! rebuilds the genesis with [`Starknet::new`] and applies the dumped blocks, state diffs,
//! transactions and controls on top of it, without replaying transactions. A dump loads only into
//! a Devnet whose genesis for the configuration matches, which in practice means the same release.
//!
//! Every field of [`Starknet`] is destructured exhaustively, so adding a field does not compile
//! until it is either persisted or derived on load.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use starknet_api::block::BlockNumber;
use starknet_rs_core::types::Felt;
use url::Url;

use self::chain::{BlocksDump, ConfigDump, ControlsDump, MessagingDump, TransactionDump};
pub(super) use self::state::genesis_fingerprint;
use self::state::{
    ClassTable, ClassesDump, CompiledClasses, Genesis, LatestDump, invalid_dump, rebuild_states,
};
use super::Starknet;
use super::starknet_config::{DumpOn, StarknetConfig};
use crate::error::DevnetResult;

mod chain;
mod state;

/// Identifies dumps.
pub const DUMP_FORMAT: &str = "starknet-devnet-dump";
/// The dump format version produced and accepted by this Devnet.
pub const DUMP_FORMAT_VERSION: u64 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateDump {
    format: String,
    version: u64,
    /// The Devnet version that produced the dump. Informational only.
    devnet_version: String,
    /// Identifies the genesis the dump applies to.
    genesis_fingerprint: Felt,
    /// The last forking-origin block known to be accepted on L1. Owned by the server.
    pub origin_accepted_on_l1_through: Option<u64>,
    config: ConfigDump,
    classes: ClassesDump,
    latest: LatestDump,
    blocks: BlocksDump,
    transactions: Vec<TransactionDump>,
    controls: ControlsDump,
    messaging: MessagingDump,
}

/// Settings of the loading instance that a dump does not carry.
#[derive(Debug, Clone, Default)]
pub struct DumpLoadOptions {
    pub dump_on: Option<DumpOn>,
    pub dump_path: Option<String>,
    /// Replaces the origin URL recorded in a dump of a forked instance.
    pub fork_url: Option<Url>,
}

impl StateDump {
    /// Parses a dump, rejecting other formats and unsupported versions before decoding the
    /// rest of the document.
    pub fn from_value(value: Value) -> DevnetResult<Self> {
        let object = value.as_object().ok_or_else(|| invalid_dump("expected a JSON object"))?;
        match object.get("format") {
            Some(Value::String(format)) if format == DUMP_FORMAT => {}
            Some(format) => {
                return Err(invalid_dump(format!(
                    "unsupported format {format}; expected \"{DUMP_FORMAT}\""
                )));
            }
            None => {
                return Err(invalid_dump(format!(
                    "missing \"format\"; expected \"{DUMP_FORMAT}\""
                )));
            }
        }
        match object.get("version") {
            Some(version) if version.as_u64() == Some(DUMP_FORMAT_VERSION) => {}
            Some(version) => {
                return Err(invalid_dump(format!(
                    "unsupported version {version}; supported: {DUMP_FORMAT_VERSION}"
                )));
            }
            None => return Err(invalid_dump("missing \"version\"")),
        }

        serde_json::from_value(value).map_err(|e| invalid_dump(e.to_string()))
    }

    pub fn devnet_version(&self) -> &str {
        &self.devnet_version
    }

    /// Returns the configuration that loading the dump with `options` applies.
    pub fn starknet_config(&self, options: &DumpLoadOptions) -> DevnetResult<StarknetConfig> {
        let definitions = self.classes.definitions_of(&self.config.class_hashes());
        self.config.clone().decode(&definitions, options)
    }
}

impl Starknet {
    /// Encodes all Devnet-owned state that the genesis of the configuration does not produce.
    /// Does not modify the instance.
    pub fn dump(&self) -> DevnetResult<StateDump> {
        let Starknet {
            latest_state,
            // Rebuilt from the latest state and the pre-confirmed state diff.
            pre_confirmed_state: _,
            pre_confirmed_state_diff,
            // Produced by the genesis.
            predeployed_accounts: _,
            block_context,
            blocks,
            transactions,
            config,
            pre_confirmed_block_timestamp_shift,
            next_block_timestamp,
            next_block_gas,
            messaging,
            rpc_contract_classes,
            cheats,
            genesis_fingerprint,
        } = self;

        let mut table = ClassTable::default();
        let classes = ClassesDump::encode(
            &rpc_contract_classes.read(),
            blocks.starting_block_number,
            &mut table,
        );
        let config = ConfigDump::encode(config, &mut table)?;

        Ok(StateDump {
            format: DUMP_FORMAT.to_string(),
            version: DUMP_FORMAT_VERSION,
            devnet_version: env!("CARGO_PKG_VERSION").to_string(),
            genesis_fingerprint: *genesis_fingerprint,
            origin_accepted_on_l1_through: None,
            config,
            classes: classes.finish(table),
            latest: LatestDump::encode(latest_state, pre_confirmed_state_diff),
            blocks: BlocksDump::encode(blocks, pre_confirmed_state_diff)?,
            transactions: chain::encode_transactions(transactions),
            controls: ControlsDump::encode(
                *pre_confirmed_block_timestamp_shift,
                *next_block_timestamp,
                next_block_gas,
                block_context,
                cheats,
            )?,
            messaging: MessagingDump::encode(messaging),
        })
    }

    /// Builds a new instance from a dump without replaying transactions. Fails without
    /// affecting any existing instance's state; building the genesis does update the global
    /// block and transaction metrics, which the caller resynchronizes for the instance it keeps.
    pub fn from_dump(dump: StateDump, options: &DumpLoadOptions) -> DevnetResult<Self> {
        let StateDump {
            format: _,
            version: _,
            devnet_version: _,
            genesis_fingerprint,
            origin_accepted_on_l1_through: _,
            config,
            classes,
            latest,
            blocks,
            transactions,
            controls,
            messaging,
        } = dump;

        let (definitions, post_genesis_classes) = classes.decode()?;
        let config = config.decode(&definitions, options)?;

        let genesis = Starknet::new(&config)?;
        if genesis.genesis_fingerprint != genesis_fingerprint {
            return Err(invalid_dump(
                "the dump was created by a Devnet with a different genesis for its configuration; \
                 dumps load only into the same Devnet release",
            ));
        }
        let Starknet {
            latest_state: genesis_latest,
            pre_confirmed_state: _,
            pre_confirmed_state_diff: _,
            predeployed_accounts,
            block_context: _,
            blocks: mut genesis_blocks,
            transactions: _,
            config,
            pre_confirmed_block_timestamp_shift: _,
            next_block_timestamp: _,
            next_block_gas: _,
            messaging: _,
            rpc_contract_classes,
            cheats: _,
            genesis_fingerprint,
        } = genesis;

        post_genesis_classes.add_to(&mut rpc_contract_classes.write(), &definitions)?;
        let (mut blocks, pre_confirmed_state_diff) = blocks.decode(&genesis_blocks)?;
        let genesis_archived = genesis_blocks
            .num_to_hash
            .get(&BlockNumber(genesis_blocks.starting_block_number))
            .copied()
            .and_then(|genesis_hash| genesis_blocks.hash_to_state.remove(&genesis_hash));

        let genesis = Genesis {
            latest: &genesis_latest,
            archived: genesis_archived,
            compiled: CompiledClasses::new(&definitions, &genesis_latest),
        };
        let states = rebuild_states(genesis, &blocks, &pre_confirmed_state_diff, latest)?;
        blocks.hash_to_state.extend(states.archive);
        let controls = controls.decode(&config)?;

        Ok(Starknet {
            latest_state: states.latest,
            pre_confirmed_state: states.pre_confirmed,
            pre_confirmed_state_diff,
            predeployed_accounts,
            block_context: controls.block_context,
            blocks,
            transactions: chain::decode_transactions(transactions)?,
            config,
            pre_confirmed_block_timestamp_shift: controls.pre_confirmed_block_timestamp_shift,
            next_block_timestamp: controls.next_block_timestamp,
            next_block_gas: controls.next_block_gas,
            messaging: messaging.decode()?,
            rpc_contract_classes,
            cheats: controls.cheats,
            genesis_fingerprint,
        })
    }
}

#[cfg(test)]
mod tests;
