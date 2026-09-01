use std::collections::HashMap;

use indexmap::IndexMap;
use serde::Serialize;
use starknet_api::core::Nonce;
use starknet_types::contract_address::ContractAddress;
use starknet_types::felt::TransactionHash;

use crate::error::{DevnetResult, Error};

mod config;
mod entry;
mod ordering;

pub use config::{MempoolConfig, MempoolConfigUpdate, MempoolOrdering};
pub use entry::{MempoolEntry, MempoolLane, MempoolPhase};
pub(crate) use entry::{PendingDeclaration, PreparedTransaction};
pub use ordering::{
    EligibleTransactions, OrderingPolicyRegistry, SelectionContext, TransactionOrderingPolicy,
    starknet_comparator,
};

/// Transactions already appended to the live pre-confirmed block and deterministic selection
/// state associated with that proposal.
#[derive(Debug, Default)]
pub struct OpenProposal {
    transaction_hashes: Vec<TransactionHash>,
    selection_counter: u64,
}

impl OpenProposal {
    pub fn transaction_hashes(&self) -> &[TransactionHash] {
        &self.transaction_hashes
    }

    pub fn transaction_count(&self) -> usize {
        self.transaction_hashes.len()
    }

    pub fn selection_counter(&self) -> u64 {
        self.selection_counter
    }

    fn append(&mut self, hash: TransactionHash) {
        self.transaction_hashes.push(hash);
    }

    fn record_selection(&mut self) {
        self.selection_counter = self.selection_counter.saturating_add(1);
    }

    fn take_hashes(&mut self) -> Vec<TransactionHash> {
        self.selection_counter = 0;
        std::mem::take(&mut self.transaction_hashes)
    }

    fn clear(&mut self) {
        self.transaction_hashes.clear();
        self.selection_counter = 0;
    }
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

#[derive(Debug)]
pub struct Mempool {
    entries: IndexMap<TransactionHash, MempoolEntry>,
    account_nonce_index: HashMap<(ContractAddress, Nonce), TransactionHash>,
    next_arrival_id: u64,
    proposal: OpenProposal,
    config: MempoolConfig,
    ordering_policies: OrderingPolicyRegistry,
}

impl Mempool {
    pub fn new(config: MempoolConfig) -> DevnetResult<Self> {
        Self::with_policy_registry(config, OrderingPolicyRegistry::default())
    }

    pub fn with_policy_registry(
        config: MempoolConfig,
        ordering_policies: OrderingPolicyRegistry,
    ) -> DevnetResult<Self> {
        if !ordering_policies.contains(&config.ordering) {
            return Err(unknown_policy_error(&config.ordering, &ordering_policies));
        }
        Ok(Self::empty(config, ordering_policies))
    }

    fn empty(config: MempoolConfig, ordering_policies: OrderingPolicyRegistry) -> Self {
        Self {
            entries: IndexMap::new(),
            account_nonce_index: HashMap::new(),
            next_arrival_id: 0,
            proposal: OpenProposal::default(),
            config,
            ordering_policies,
        }
    }

    pub fn config(&self) -> &MempoolConfig {
        &self.config
    }

    pub fn ordering_policies(&self) -> &OrderingPolicyRegistry {
        &self.ordering_policies
    }

    pub fn entries(&self) -> impl Iterator<Item = (&TransactionHash, &MempoolEntry)> {
        self.entries.iter()
    }

    pub fn get(&self, hash: &TransactionHash) -> Option<&MempoolEntry> {
        self.entries.get(hash)
    }

    pub fn proposal_hashes(&self) -> &[TransactionHash] {
        self.proposal.transaction_hashes()
    }

    pub fn open_proposal(&self) -> &OpenProposal {
        &self.proposal
    }

    pub fn remaining_capacity(&self) -> usize {
        self.config.max_transactions_per_block.saturating_sub(self.proposal.transaction_count())
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
    }

    pub(crate) fn remove_entry(&mut self, hash: &TransactionHash) -> Option<MempoolEntry> {
        let entry = self.entries.shift_remove(hash)?;
        if let (Some(address), Some(nonce)) = (entry.account_address, entry.nonce) {
            self.account_nonce_index.remove(&(address, nonce));
        }
        Some(entry)
    }

    #[cfg(test)]
    pub(crate) fn select_policy(
        &self,
        eligible: &[TransactionHash],
        block_number: u64,
        current_l2_gas_price: u128,
    ) -> Option<TransactionHash> {
        self.select_configured_policy(
            &self.eligible_transactions(eligible),
            &SelectionContext {
                block_number,
                current_l2_gas_price,
                proposal_selection_counter: self.proposal.selection_counter(),
                random_seed: self.config.random_seed,
            },
        )
        .unwrap()
    }

    pub(crate) fn select_configured_policy(
        &self,
        eligible: &EligibleTransactions<'_>,
        context: &SelectionContext,
    ) -> DevnetResult<Option<TransactionHash>> {
        let policy = self
            .ordering_policies
            .resolve(&self.config.ordering)
            .ok_or_else(|| unknown_policy_error(&self.config.ordering, &self.ordering_policies))?;
        Ok(policy.select(eligible, context))
    }

    pub(crate) fn eligible_transactions<'a>(
        &'a self,
        hashes: &'a [TransactionHash],
    ) -> EligibleTransactions<'a> {
        EligibleTransactions::new(self, hashes)
    }

    /// Advances the deterministic selection sequence after any attempted selection, including a
    /// caller-forced hash. This keeps a replayed, canonicalized forced-hash request aligned with
    /// the original policy-driven request that selected the same hashes.
    pub(crate) fn record_selection(&mut self) {
        self.proposal.record_selection();
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
        self.proposal.append(*hash);
        Ok(())
    }

    pub(crate) fn return_to_received(&mut self, hash: &TransactionHash) {
        if let Some(entry) = self.entries.get_mut(hash) {
            entry.phase = MempoolPhase::Received;
        }
    }

    pub(crate) fn commit_proposal(&mut self) {
        let hashes = self.proposal.take_hashes();
        for hash in hashes {
            self.remove_entry(&hash);
        }
    }

    pub(crate) fn abort_proposal(&mut self) -> Vec<TransactionHash> {
        let hashes = self.proposal.take_hashes();
        for hash in &hashes {
            self.return_to_received(hash);
        }
        for entry in self.entries.values_mut() {
            if entry.phase == MempoolPhase::Candidate {
                entry.phase = MempoolPhase::Received;
            }
        }
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
            if !self.ordering_policies.contains(&ordering) {
                return Err(unknown_policy_error(&ordering, &self.ordering_policies));
            }
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

impl Default for Mempool {
    fn default() -> Self {
        Self::empty(MempoolConfig::default(), OrderingPolicyRegistry::default())
    }
}

fn unknown_policy_error(name: &MempoolOrdering, registry: &OrderingPolicyRegistry) -> Error {
    let mut available = registry.names().map(MempoolOrdering::as_str).collect::<Vec<_>>();
    available.sort_unstable();
    Error::UnsupportedAction {
        msg: format!(
            "Unknown mempool ordering policy '{name}'. Available policies: {}",
            available.join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use starknet_rs_core::types::Felt;

    use super::{Mempool, MempoolConfig, MempoolConfigUpdate, MempoolOrdering, OpenProposal};

    #[test]
    fn capacity_cannot_be_zero() {
        let mut pool = Mempool::new(MempoolConfig::default()).unwrap();
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
        let mut pool = Mempool::new(MempoolConfig::default()).unwrap();
        let config = pool
            .set_config(MempoolConfigUpdate {
                ordering: Some(MempoolOrdering::random()),
                random_seed: Some(17),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(config.ordering, MempoolOrdering::random());
        assert_eq!(config.random_seed, 17);
        assert_eq!(config.max_transactions_per_block, 500);
    }

    #[test]
    fn unknown_configured_policy_is_rejected_without_changing_active_policy() {
        let unknown: MempoolOrdering = "not-registered".parse().unwrap();
        assert!(
            Mempool::new(MempoolConfig { ordering: unknown.clone(), ..MempoolConfig::default() })
                .is_err()
        );

        let mut pool = Mempool::new(MempoolConfig::default()).unwrap();
        assert!(
            pool.set_config(MempoolConfigUpdate {
                ordering: Some(unknown),
                ..MempoolConfigUpdate::default()
            })
            .is_err()
        );
        assert_eq!(pool.config().ordering, MempoolOrdering::fifo());
    }

    #[test]
    fn open_proposal_owns_hashes_and_selection_state() {
        let mut proposal = OpenProposal::default();
        let hash = Felt::from(0x10);
        proposal.record_selection();
        proposal.append(hash);

        assert_eq!(proposal.transaction_hashes(), &[hash]);
        assert_eq!(proposal.transaction_count(), 1);
        assert_eq!(proposal.selection_counter(), 1);
        assert_eq!(proposal.take_hashes(), vec![hash]);
        assert!(proposal.transaction_hashes().is_empty());
        assert_eq!(proposal.selection_counter(), 0);
    }
}
