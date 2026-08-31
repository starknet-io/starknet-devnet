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

/// Starknet mempool ordering comparator. It separates priority and fallback branches:
///
/// * **Priority** (`max_l2_gas_price >= current_l2_gas_price`): descending `tip`, with descending
///   `transaction_hash` as the tie-breaker — mirroring the production queue in
///   `apollo_mempool::fee_transaction_queue::PriorityTransaction`, which orders ascending
///   internally and then serves the head via `pop_last`, producing descending `tip` and descending
///   `tx_hash` in selection.
/// * **Non-priority fallback**: FIFO on `arrival_id`.
///
/// Returned `Ordering` matches what `Iterator::max_by` expects: `Greater` means the left
/// argument should be picked (the "maximum"). The fallback deliberately swaps operand
/// order (`right.arrival_id.cmp(&left.arrival_id)`) to produce FIFO under `max_by`.
pub fn starknet_comparator(
    left: &MempoolEntry,
    right: &MempoolEntry,
    current_l2_gas_price: u128,
) -> std::cmp::Ordering {
    let left_priority = left.max_l2_gas_price >= current_l2_gas_price;
    let right_priority = right.max_l2_gas_price >= current_l2_gas_price;
    left_priority.cmp(&right_priority).then_with(|| {
        if left_priority {
            left.tip.cmp(&right.tip).then_with(|| {
                left.transaction
                    .get_transaction_hash()
                    .cmp(right.transaction.get_transaction_hash())
            })
        } else {
            right.arrival_id.cmp(&left.arrival_id)
        }
    })
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

/// Immutable transaction set exposed to an ordering policy.
///
/// The builder constructs this view only from transactions that already passed its eligibility
/// rules. A policy chooses ordering; it does not decide whether a transaction is valid.
pub struct EligibleTransactions<'a> {
    mempool: &'a Mempool,
    hashes: &'a [TransactionHash],
}

impl<'a> EligibleTransactions<'a> {
    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    pub fn hashes(&self) -> &[TransactionHash] {
        self.hashes
    }

    pub fn get(&self, index: usize) -> Option<&'a MempoolEntry> {
        self.hashes.get(index).and_then(|hash| self.mempool.get(hash))
    }

    pub fn iter(&self) -> impl Iterator<Item = &'a MempoolEntry> + '_ {
        self.hashes.iter().filter_map(|hash| self.mempool.get(hash))
    }
}

/// Deterministic block/proposal information supplied to every ordering policy invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SelectionContext {
    pub block_number: u64,
    pub current_l2_gas_price: u128,
    pub proposal_selection_counter: u64,
}

/// Extension point for transaction ordering.
///
/// Implementations must be deterministic for the supplied transactions and context. The block
/// builder rejects hashes outside `eligible`, preserving nonce, validation, and capacity rules for
/// custom policies.
pub trait TransactionOrderingPolicy: Send + Sync {
    fn select(
        &self,
        eligible: &EligibleTransactions<'_>,
        context: &SelectionContext,
    ) -> Option<TransactionHash>;
}

/// Adapter for the ordering policies exposed through Devnet's serialized configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfiguredOrderingPolicy {
    ordering: MempoolOrdering,
    random_seed: u64,
}

impl ConfiguredOrderingPolicy {
    pub fn new(ordering: MempoolOrdering, random_seed: u64) -> Self {
        Self { ordering, random_seed }
    }

    pub fn from_config(config: &MempoolConfig) -> Self {
        Self::new(config.ordering, config.random_seed)
    }
}

impl TransactionOrderingPolicy for ConfiguredOrderingPolicy {
    fn select(
        &self,
        eligible: &EligibleTransactions<'_>,
        context: &SelectionContext,
    ) -> Option<TransactionHash> {
        if eligible.is_empty() {
            return None;
        }

        match self.ordering {
            MempoolOrdering::Fifo => eligible
                .iter()
                .min_by_key(|entry| entry.arrival_id)
                .map(|entry| *entry.transaction.get_transaction_hash()),
            MempoolOrdering::Starknet => eligible
                .iter()
                .max_by(|left, right| {
                    starknet_comparator(left, right, context.current_l2_gas_price)
                })
                .map(|entry| *entry.transaction.get_transaction_hash()),
            MempoolOrdering::Random => {
                let mixed = splitmix64(
                    self.random_seed ^ context.block_number ^ context.proposal_selection_counter,
                );
                eligible
                    .get((mixed as usize) % eligible.len())
                    .map(|entry| *entry.transaction.get_transaction_hash())
            }
        }
    }
}

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

#[derive(Debug, Default)]
pub struct Mempool {
    entries: IndexMap<TransactionHash, MempoolEntry>,
    account_nonce_index: HashMap<(ContractAddress, Nonce), TransactionHash>,
    next_arrival_id: u64,
    proposal: OpenProposal,
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
        ConfiguredOrderingPolicy::from_config(&self.config).select(
            &self.eligible_transactions(eligible),
            &SelectionContext {
                block_number,
                current_l2_gas_price,
                proposal_selection_counter: self.proposal.selection_counter(),
            },
        )
    }

    pub(crate) fn eligible_transactions<'a>(
        &'a self,
        hashes: &'a [TransactionHash],
    ) -> EligibleTransactions<'a> {
        EligibleTransactions { mempool: self, hashes }
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
    use starknet_rs_core::types::Felt;
    use starknet_types::rpc::transactions::l1_handler_transaction::L1HandlerTransaction;
    use starknet_types::rpc::transactions::{Transaction, TransactionWithHash};

    use super::{
        EligibleTransactions, Mempool, MempoolConfig, MempoolConfigUpdate, MempoolEntry,
        MempoolOrdering, MempoolPhase, PreparedTransaction, SelectionContext,
        TransactionOrderingPolicy, starknet_comparator,
    };

    struct NewestFirst;

    impl TransactionOrderingPolicy for NewestFirst {
        fn select(
            &self,
            eligible: &EligibleTransactions<'_>,
            _context: &SelectionContext,
        ) -> Option<Felt> {
            eligible
                .iter()
                .max_by_key(|entry| entry.arrival_id)
                .map(|entry| *entry.transaction.get_transaction_hash())
        }
    }

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

    /// Builds a `MempoolEntry` suitable for exercising the Starknet comparator. Only the
    /// fields the comparator consults (`transaction`, `arrival_id`, `tip`,
    /// `max_l2_gas_price`) need to be meaningful; `prepared` is a stub because the
    /// comparator never reads it.
    fn entry_with(hash: Felt, arrival_id: u64, tip: u64, max_l2_gas_price: u128) -> MempoolEntry {
        let tx =
            TransactionWithHash::new(hash, Transaction::L1Handler(L1HandlerTransaction::default()));
        let prepared = PreparedTransaction::system(tx.clone(), Default::default());
        MempoolEntry {
            transaction: tx,
            phase: MempoolPhase::Received,
            arrival_id,
            account_address: None,
            nonce: None,
            tip,
            max_l2_gas_price,
            lane: super::MempoolLane::User,
            prepared,
        }
    }

    /// Inserts `entry` into `pool.entries`, bypassing `admit()` because constructing a
    /// full `PreparedTransaction` from outside the mempool crate would pull in heavy
    /// blockifier types. The comparator is a pure function over the entry, so direct
    /// state insertion is sufficient.
    fn install(pool: &mut Mempool, hash: Felt, entry: MempoolEntry) {
        pool.entries.insert(hash, entry);
        pool.next_arrival_id = pool.next_arrival_id.saturating_add(1);
    }

    #[test]
    fn starknet_comparator_priority_orders_by_descending_tip() {
        // All entries are priority (`max_l2 >= threshold`). Highest tip wins.
        let low = entry_with(Felt::from(0x10), 1, 1, 1_000);
        let high = entry_with(Felt::from(0x20), 2, 5, 1_000);
        let mid = entry_with(Felt::from(0x30), 3, 3, 1_000);

        let threshold = 500_u128;
        // `max_by` picks the entry whose comparator yields `Greater` against every other
        // candidate. Assert via pairwise checks rather than relying on `max_by`'s
        // reduction mechanics: a `Greater` result means the *left* argument should win.
        assert_eq!(starknet_comparator(&high, &mid, threshold), std::cmp::Ordering::Greater);
        assert_eq!(starknet_comparator(&high, &low, threshold), std::cmp::Ordering::Greater);
        assert_eq!(starknet_comparator(&mid, &low, threshold), std::cmp::Ordering::Greater);
        // Reverse direction must be `Less` so `max_by` keeps the higher tip.
        assert_eq!(starknet_comparator(&low, &high, threshold), std::cmp::Ordering::Less);
    }

    #[test]
    fn starknet_comparator_priority_tie_break_prefers_larger_hash() {
        // Same tip → tie-break on transaction_hash. The comparator is consumed by
        // `Iterator::max_by`, so the entry whose comparator yields `Greater` against
        // the other wins. We pick the entry that should be selected first; this
        // mirrors `apollo_mempool`'s `pop_last` semantics (descending tip, descending
        // tx_hash in selection).
        let smaller_hash = entry_with(Felt::from(0x10), 1, 7, 1_000);
        let larger_hash = entry_with(Felt::from(0x20), 2, 7, 1_000);
        let threshold = 500_u128;
        assert_eq!(
            starknet_comparator(&larger_hash, &smaller_hash, threshold),
            std::cmp::Ordering::Greater,
            "larger hash must win when tips are equal (descending-hash selection)"
        );
        assert_eq!(
            starknet_comparator(&smaller_hash, &larger_hash, threshold),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn starknet_comparator_fallback_is_fifo_on_arrival_id() {
        // All entries are non-priority (`max_l2 < threshold`). Earlier arrival must win.
        let first = entry_with(Felt::from(0x10), 1, /* tip= */ 99, /* max_l2= */ 0);
        let second = entry_with(Felt::from(0x20), 2, /* tip= */ 0, /* max_l2= */ 0);
        let third = entry_with(Felt::from(0x30), 3, /* tip= */ 5_000, /* max_l2= */ 0);

        let threshold = 1_000_u128;
        // Ties must hold even when later arrivals carry higher tips or different hashes
        // — the fallback branch ignores both.
        assert_eq!(
            starknet_comparator(&first, &second, threshold),
            std::cmp::Ordering::Greater,
            "earlier arrival must win in fallback"
        );
        assert_eq!(starknet_comparator(&first, &third, threshold), std::cmp::Ordering::Greater);
        assert_eq!(starknet_comparator(&second, &third, threshold), std::cmp::Ordering::Greater);
        assert_eq!(starknet_comparator(&third, &first, threshold), std::cmp::Ordering::Less);
    }

    #[test]
    fn starknet_comparator_priority_beats_fallback() {
        // A non-priority entry never outranks a priority entry, regardless of arrival or
        // hash. This is the partition guarantee the policy relies on.
        let priority_old =
            entry_with(Felt::from(0x10), 100, /* tip= */ 0, /* max_l2= */ 1_000);
        let fallback_new =
            entry_with(Felt::from(0xff), 1, /* tip= */ 9_999, /* max_l2= */ 0);
        let threshold = 500_u128;
        assert_eq!(
            starknet_comparator(&priority_old, &fallback_new, threshold),
            std::cmp::Ordering::Greater,
            "priority entry must win regardless of arrival order or tip"
        );
        assert_eq!(
            starknet_comparator(&fallback_new, &priority_old, threshold),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn starknet_select_policy_drains_fallback_in_arrival_order() {
        // End-to-end through `select_policy`: submit four non-priority entries with
        // insertion order distinct from hash order and verify the policy picks them in
        // arrival order across repeated calls.
        let mut pool = Mempool::new(MempoolConfig {
            ordering: MempoolOrdering::Starknet,
            random_seed: 0,
            max_transactions_per_block: 500,
        });

        // Hash values are intentionally non-monotonic so arrival order is the only
        // discriminator — any reliance on hash for ordering would surface here.
        let h_arrival_0 = Felt::from(0xee);
        let h_arrival_1 = Felt::from(0xdd);
        let h_arrival_2 = Felt::from(0x11);
        let h_arrival_3 = Felt::from(0x22);

        install(&mut pool, h_arrival_0, entry_with(h_arrival_0, 0, 0, 0));
        install(&mut pool, h_arrival_1, entry_with(h_arrival_1, 1, 0, 0));
        install(&mut pool, h_arrival_2, entry_with(h_arrival_2, 2, 0, 0));
        install(&mut pool, h_arrival_3, entry_with(h_arrival_3, 3, 0, 0));

        let threshold = 1_000_u128;
        let mut remaining = vec![h_arrival_2, h_arrival_0, h_arrival_3, h_arrival_1];
        let mut drained: Vec<Felt> = Vec::new();
        while let Some(picked) = pool.select_policy(&remaining, 0, threshold) {
            drained.push(picked);
            remaining.retain(|h| *h != picked);
            if remaining.is_empty() {
                break;
            }
        }
        assert_eq!(drained, vec![h_arrival_0, h_arrival_1, h_arrival_2, h_arrival_3]);
    }

    #[test]
    fn forced_selection_advances_the_random_sequence() {
        let config = MempoolConfig {
            ordering: MempoolOrdering::Random,
            random_seed: 42,
            max_transactions_per_block: 500,
        };
        let mut original = Mempool::new(config.clone());
        let mut replay = Mempool::new(config);
        let hashes = [Felt::from(0x10), Felt::from(0x20), Felt::from(0x30)];

        for (arrival_id, hash) in hashes.iter().copied().enumerate() {
            install(&mut original, hash, entry_with(hash, arrival_id as u64, 0, 1_000));
            install(&mut replay, hash, entry_with(hash, arrival_id as u64, 0, 1_000));
        }

        // The original request uses the random policy. Replay uses the canonical forced hash,
        // but both must leave the next random selection at the same sequence position.
        let first = original.select_policy(&hashes, 1, 1_000).unwrap();
        original.record_selection();
        replay.record_selection();

        let remaining = hashes.into_iter().filter(|hash| *hash != first).collect::<Vec<_>>();
        assert_eq!(
            original.select_policy(&remaining, 1, 1_000),
            replay.select_policy(&remaining, 1, 1_000)
        );
    }

    #[test]
    fn caller_defined_policy_can_order_the_eligible_view() {
        let mut pool = Mempool::new(MempoolConfig::default());
        let first = Felt::from(0x10);
        let last = Felt::from(0x20);
        install(&mut pool, first, entry_with(first, 0, 0, 1_000));
        install(&mut pool, last, entry_with(last, 1, 0, 1_000));
        let hashes = [first, last];
        let eligible = pool.eligible_transactions(&hashes);

        assert_eq!(
            NewestFirst.select(
                &eligible,
                &SelectionContext {
                    block_number: 1,
                    current_l2_gas_price: 1_000,
                    proposal_selection_counter: 0,
                }
            ),
            Some(last)
        );
    }

    #[test]
    fn open_proposal_owns_hashes_and_selection_state() {
        let mut proposal = super::OpenProposal::default();
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
