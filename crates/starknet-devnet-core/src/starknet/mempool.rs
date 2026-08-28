use std::collections::HashMap;

use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::transaction_execution::Transaction as ExecutableTransaction;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use starknet_api::core::Nonce;
use starknet_types::contract_address::ContractAddress;
use starknet_types::contract_class::ContractClass;
use starknet_types::felt::{ClassHash, CompiledClassHash, TransactionHash};
use starknet_types::rpc::transactions::TransactionWithHash;

use crate::error::{DevnetResult, Error};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MempoolOrdering {
    #[default]
    Fifo,
    Starknet,
    Random,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MempoolConfig {
    pub ordering: MempoolOrdering,
    pub random_seed: u64,
    pub max_transactions_per_block: usize,
}

impl Default for MempoolConfig {
    fn default() -> Self {
        Self { ordering: MempoolOrdering::Fifo, random_seed: 0, max_transactions_per_block: 500 }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct MempoolConfigUpdate {
    pub ordering: Option<MempoolOrdering>,
    pub random_seed: Option<u64>,
    pub max_transactions_per_block: Option<usize>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MempoolPhase {
    Received,
    Candidate,
    PreConfirmed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MempoolLane {
    User,
    System,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingDeclaration {
    pub class_hash: ClassHash,
    pub casm_hash: Option<CompiledClassHash>,
    pub contract_class: ContractClass,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedTransaction {
    pub transaction: TransactionWithHash,
    pub executable: ExecutableTransaction,
    pub declaration: Option<PendingDeclaration>,
    pub account_address: Option<ContractAddress>,
    pub nonce: Option<Nonce>,
    pub tip: u64,
    pub max_l2_gas_price: u128,
    pub lane: MempoolLane,
}

impl PreparedTransaction {
    pub(crate) fn account(
        transaction: TransactionWithHash,
        executable: AccountTransaction,
        declaration: Option<PendingDeclaration>,
    ) -> Self {
        let account_address = Some(executable.sender_address().into());
        let nonce = Some(executable.nonce());
        let tip = executable.tip().0;
        let max_l2_gas_price = executable.resource_bounds().get_l2_bounds().max_price_per_unit.0;
        Self {
            transaction,
            executable: ExecutableTransaction::Account(executable),
            declaration,
            account_address,
            nonce,
            tip,
            max_l2_gas_price,
            lane: MempoolLane::User,
        }
    }

    pub(crate) fn system(
        transaction: TransactionWithHash,
        executable: starknet_api::executable_transaction::L1HandlerTransaction,
    ) -> Self {
        Self {
            transaction,
            executable: ExecutableTransaction::L1Handler(executable),
            declaration: None,
            account_address: None,
            nonce: None,
            tip: 0,
            max_l2_gas_price: 0,
            lane: MempoolLane::System,
        }
    }

    /// Admin/system account-tx variant used by `devnet_mint` and similar administrative actions.
    /// Behaves identically to `account` except for the lane marker, which causes
    /// `submit_system_prepared_transaction` to execute regardless of the configured block
    /// generation mode, so balance changes remain immediately observable in `mempool` mode.
    pub(crate) fn system_account(
        transaction: TransactionWithHash,
        executable: AccountTransaction,
        declaration: Option<PendingDeclaration>,
    ) -> Self {
        let account_address = Some(executable.sender_address().into());
        let nonce = Some(executable.nonce());
        let tip = executable.tip().0;
        let max_l2_gas_price = executable.resource_bounds().get_l2_bounds().max_price_per_unit.0;
        Self {
            transaction,
            executable: ExecutableTransaction::Account(executable),
            declaration,
            account_address,
            nonce,
            tip,
            max_l2_gas_price,
            lane: MempoolLane::System,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MempoolEntry {
    pub transaction: TransactionWithHash,
    pub phase: MempoolPhase,
    pub arrival_id: u64,
    pub account_address: Option<ContractAddress>,
    pub nonce: Option<Nonce>,
    pub tip: u64,
    pub max_l2_gas_price: u128,
    pub lane: MempoolLane,
    pub(crate) prepared: PreparedTransaction,
}

#[derive(Clone, Debug)]
pub enum MempoolSelection {
    Policy { max_transactions: Option<usize> },
    Hashes(Vec<TransactionHash>),
}

impl Default for MempoolSelection {
    fn default() -> Self {
        Self::Policy { max_transactions: None }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BuildFailure {
    pub transaction_hash: TransactionHash,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct BuildOutcome {
    pub selected: Vec<TransactionHash>,
    pub pre_confirmed: Vec<TransactionHash>,
    pub rejected: Vec<BuildFailure>,
    pub blocked: Vec<BuildFailure>,
    pub block_full: bool,
}

impl BuildOutcome {
    pub fn made_progress(&self) -> bool {
        !self.pre_confirmed.is_empty() || !self.rejected.is_empty()
    }
}

#[derive(Debug, Default)]
pub struct Mempool {
    entries: IndexMap<TransactionHash, MempoolEntry>,
    account_nonce_index: HashMap<(ContractAddress, Nonce), TransactionHash>,
    next_arrival_id: u64,
    proposal: Vec<TransactionHash>,
    selection_counter: u64,
    config: MempoolConfig,
}

impl Mempool {
    pub fn new(config: MempoolConfig) -> Self {
        Self { config, ..Self::default() }
    }

    pub fn config(&self) -> &MempoolConfig {
        &self.config
    }

    pub fn entries(&self) -> impl Iterator<Item = (&TransactionHash, &MempoolEntry)> {
        self.entries.iter()
    }

    pub fn get(&self, hash: &TransactionHash) -> Option<&MempoolEntry> {
        self.entries.get(hash)
    }

    pub fn proposal_hashes(&self) -> &[TransactionHash] {
        &self.proposal
    }

    pub fn remaining_capacity(&self) -> usize {
        self.config.max_transactions_per_block.saturating_sub(self.proposal.len())
    }

    pub(crate) fn admit(&mut self, prepared: PreparedTransaction) -> DevnetResult<TransactionHash> {
        let hash = *prepared.transaction.get_transaction_hash();
        if self.entries.contains_key(&hash) {
            return Err(Error::DuplicateTransaction { transaction_hash: hash });
        }
        if let (Some(address), Some(nonce)) = (prepared.account_address, prepared.nonce)
            && self.account_nonce_index.contains_key(&(address, nonce))
        {
            return Err(Error::NonceConflict { address, nonce });
        }

        let entry = MempoolEntry {
            transaction: prepared.transaction.clone(),
            phase: MempoolPhase::Received,
            arrival_id: self.next_arrival_id,
            account_address: prepared.account_address,
            nonce: prepared.nonce,
            tip: prepared.tip,
            max_l2_gas_price: prepared.max_l2_gas_price,
            lane: prepared.lane,
            prepared,
        };
        self.next_arrival_id = self.next_arrival_id.saturating_add(1);
        if let (Some(address), Some(nonce)) = (entry.account_address, entry.nonce) {
            self.account_nonce_index.insert((address, nonce), hash);
        }
        self.entries.insert(hash, entry);
        Ok(hash)
    }

    pub(crate) fn remove_received(&mut self, hash: &TransactionHash) -> DevnetResult<MempoolEntry> {
        let phase = self.entries.get(hash).ok_or(Error::NoTransaction)?.phase;
        if phase != MempoolPhase::Received {
            return Err(Error::UnsupportedAction {
                msg: format!("Transaction {hash:#x} is {phase:?} and cannot be removed"),
            });
        }
        self.remove_entry(hash).ok_or(Error::NoTransaction)
    }

    pub(crate) fn clear_received(&mut self) -> Vec<MempoolEntry> {
        let hashes = self
            .entries
            .iter()
            .filter_map(|(hash, entry)| (entry.phase == MempoolPhase::Received).then_some(*hash))
            .collect::<Vec<_>>();
        hashes.iter().filter_map(|hash| self.remove_entry(hash)).collect()
    }

    /// Drop every entry, the open proposal, and the policy selection counter.
    /// Used by accepted-block abortion and restart to model a complete reset of pool state.
    pub(crate) fn clear_all(&mut self) {
        self.entries.clear();
        self.account_nonce_index.clear();
        self.proposal.clear();
        self.selection_counter = 0;
    }

    pub(crate) fn remove_entry(&mut self, hash: &TransactionHash) -> Option<MempoolEntry> {
        let entry = self.entries.shift_remove(hash)?;
        if let (Some(address), Some(nonce)) = (entry.account_address, entry.nonce) {
            self.account_nonce_index.remove(&(address, nonce));
        }
        Some(entry)
    }

    pub(crate) fn select_policy(
        &mut self,
        eligible: &[TransactionHash],
        block_number: u64,
        current_l2_gas_price: u128,
    ) -> Option<TransactionHash> {
        if eligible.is_empty() {
            return None;
        }
        let selected = match self.config.ordering {
            MempoolOrdering::Fifo => {
                eligible.iter().min_by_key(|hash| self.entries[*hash].arrival_id)
            }
            MempoolOrdering::Starknet => eligible.iter().max_by(|left, right| {
                let left = &self.entries[*left];
                let right = &self.entries[*right];
                let left_priority = left.max_l2_gas_price >= current_l2_gas_price;
                let right_priority = right.max_l2_gas_price >= current_l2_gas_price;
                left_priority.cmp(&right_priority).then_with(|| {
                    if left_priority {
                        left.tip.cmp(&right.tip).then_with(|| {
                            right
                                .transaction
                                .get_transaction_hash()
                                .cmp(left.transaction.get_transaction_hash())
                        })
                    } else {
                        right.arrival_id.cmp(&left.arrival_id)
                    }
                })
            }),
            MempoolOrdering::Random => {
                let mixed =
                    splitmix64(self.config.random_seed ^ block_number ^ self.selection_counter);
                eligible.get((mixed as usize) % eligible.len())
            }
        }
        .copied();
        self.selection_counter = self.selection_counter.saturating_add(1);
        selected
    }

    pub(crate) fn mark_candidate(
        &mut self,
        hash: &TransactionHash,
    ) -> DevnetResult<PreparedTransaction> {
        let entry = self.entries.get_mut(hash).ok_or(Error::NoTransaction)?;
        if entry.phase != MempoolPhase::Received {
            return Err(Error::UnsupportedAction {
                msg: format!("Transaction {hash:#x} is not RECEIVED"),
            });
        }
        entry.phase = MempoolPhase::Candidate;
        Ok(entry.prepared.clone())
    }

    pub(crate) fn mark_pre_confirmed(&mut self, hash: &TransactionHash) -> DevnetResult<()> {
        let entry = self.entries.get_mut(hash).ok_or(Error::NoTransaction)?;
        if entry.phase != MempoolPhase::Candidate {
            return Err(Error::UnexpectedInternalError {
                msg: format!("Transaction {hash:#x} was not selected"),
            });
        }
        entry.phase = MempoolPhase::PreConfirmed;
        self.proposal.push(*hash);
        Ok(())
    }

    pub(crate) fn return_to_received(&mut self, hash: &TransactionHash) {
        if let Some(entry) = self.entries.get_mut(hash) {
            entry.phase = MempoolPhase::Received;
        }
    }

    pub(crate) fn commit_proposal(&mut self) {
        let hashes = std::mem::take(&mut self.proposal);
        for hash in hashes {
            self.remove_entry(&hash);
        }
        self.selection_counter = 0;
    }

    pub(crate) fn abort_proposal(&mut self) -> Vec<TransactionHash> {
        let hashes = std::mem::take(&mut self.proposal);
        for hash in &hashes {
            self.return_to_received(hash);
        }
        for entry in self.entries.values_mut() {
            if entry.phase == MempoolPhase::Candidate {
                entry.phase = MempoolPhase::Received;
            }
        }
        self.selection_counter = 0;
        hashes
    }

    pub(crate) fn set_config(
        &mut self,
        update: MempoolConfigUpdate,
    ) -> DevnetResult<MempoolConfig> {
        if update.max_transactions_per_block == Some(0) {
            return Err(Error::UnsupportedAction {
                msg: "Mempool block capacity must be positive".into(),
            });
        }
        if let Some(ordering) = update.ordering {
            self.config.ordering = ordering;
        }
        if let Some(seed) = update.random_seed {
            self.config.random_seed = seed;
        }
        if let Some(capacity) = update.max_transactions_per_block {
            self.config.max_transactions_per_block = capacity;
        }
        Ok(self.config.clone())
    }
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = value;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::{MempoolConfig, MempoolConfigUpdate, MempoolOrdering};

    #[test]
    fn capacity_cannot_be_zero() {
        let mut pool = super::Mempool::new(MempoolConfig::default());
        assert!(
            pool.set_config(MempoolConfigUpdate {
                max_transactions_per_block: Some(0),
                ..Default::default()
            })
            .is_err()
        );
    }

    #[test]
    fn config_can_be_updated_partially() {
        let mut pool = super::Mempool::new(MempoolConfig::default());
        let config = pool
            .set_config(MempoolConfigUpdate {
                ordering: Some(MempoolOrdering::Random),
                random_seed: Some(17),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(config.ordering, MempoolOrdering::Random);
        assert_eq!(config.random_seed, 17);
        assert_eq!(config.max_transactions_per_block, 500);
    }
}
