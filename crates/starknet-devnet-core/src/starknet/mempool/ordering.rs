use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;

use starknet_types::felt::TransactionHash;

use super::{Mempool, MempoolEntry, MempoolOrdering};

/// Starknet mempool ordering comparator for entries that have already qualified against the L2
/// gas-price threshold. Orders by descending `tip`, with descending `transaction_hash` as the
/// tie-breaker.
///
/// Returned `Ordering` matches what `Iterator::max_by` expects: `Greater` means the left
/// argument should be picked (the "maximum").
///
/// Below-threshold entries are filtered out by `StarknetOrderingPolicy::select` before this
/// comparator is invoked.
pub fn starknet_comparator(left: &MempoolEntry, right: &MempoolEntry) -> std::cmp::Ordering {
    left.tip.cmp(&right.tip).then_with(|| {
        left.transaction.get_transaction_hash().cmp(right.transaction.get_transaction_hash())
    })
}

/// Immutable user-transaction set exposed to an ordering policy.
///
/// The builder selects eligible system-lane transactions separately in FIFO order, then constructs
/// this view only from user transactions that already passed its eligibility rules. A policy
/// chooses user ordering; it does not decide whether a transaction is valid.
pub struct EligibleTransactions<'a> {
    mempool: &'a Mempool,
    hashes: &'a [TransactionHash],
}

impl<'a> EligibleTransactions<'a> {
    pub(super) fn new(mempool: &'a Mempool, hashes: &'a [TransactionHash]) -> Self {
        Self { mempool, hashes }
    }

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
    pub random_seed: u64,
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

#[derive(Debug)]
struct FifoOrderingPolicy;

impl TransactionOrderingPolicy for FifoOrderingPolicy {
    fn select(
        &self,
        eligible: &EligibleTransactions<'_>,
        _context: &SelectionContext,
    ) -> Option<TransactionHash> {
        eligible
            .iter()
            .min_by_key(|entry| entry.arrival_id)
            .map(|entry| *entry.transaction.get_transaction_hash())
    }
}

#[derive(Debug)]
struct StarknetOrderingPolicy;

impl TransactionOrderingPolicy for StarknetOrderingPolicy {
    fn select(
        &self,
        eligible: &EligibleTransactions<'_>,
        context: &SelectionContext,
    ) -> Option<TransactionHash> {
        // Skip entries whose `max_l2_gas_price` falls below the L2 gas-price threshold: they
        // are not eligible for selection while the threshold holds. Only qualifying entries
        // participate in tip-priority ordering.
        eligible
            .iter()
            .filter(|entry| entry.max_l2_gas_price >= context.current_l2_gas_price)
            .max_by(|left, right| starknet_comparator(left, right))
            .map(|entry| *entry.transaction.get_transaction_hash())
    }
}

#[derive(Debug)]
struct RandomOrderingPolicy;

impl TransactionOrderingPolicy for RandomOrderingPolicy {
    fn select(
        &self,
        eligible: &EligibleTransactions<'_>,
        context: &SelectionContext,
    ) -> Option<TransactionHash> {
        if eligible.is_empty() {
            return None;
        }
        let mixed = splitmix64(
            context.random_seed ^ context.block_number ^ context.proposal_selection_counter,
        );
        eligible
            .get((mixed as usize) % eligible.len())
            .map(|entry| *entry.transaction.get_transaction_hash())
    }
}

/// Name-to-policy lookup used by block building.
#[derive(Clone)]
pub struct OrderingPolicyRegistry {
    policies: HashMap<MempoolOrdering, Arc<dyn TransactionOrderingPolicy>>,
}

impl OrderingPolicyRegistry {
    pub fn register<P>(&mut self, name: MempoolOrdering, policy: P)
    where
        P: TransactionOrderingPolicy + 'static,
    {
        self.policies.insert(name, Arc::new(policy));
    }

    pub fn contains(&self, name: &MempoolOrdering) -> bool {
        self.policies.contains_key(name)
    }

    pub fn resolve(&self, name: &MempoolOrdering) -> Option<&dyn TransactionOrderingPolicy> {
        self.policies.get(name).map(Arc::as_ref)
    }

    pub fn names(&self) -> impl Iterator<Item = &MempoolOrdering> {
        self.policies.keys()
    }
}

impl Default for OrderingPolicyRegistry {
    fn default() -> Self {
        let mut registry = Self { policies: HashMap::new() };
        registry.register(MempoolOrdering::fifo(), FifoOrderingPolicy);
        registry.register(MempoolOrdering::starknet(), StarknetOrderingPolicy);
        registry.register(MempoolOrdering::random(), RandomOrderingPolicy);
        registry
    }
}

impl Debug for OrderingPolicyRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut names = self.names().map(MempoolOrdering::as_str).collect::<Vec<_>>();
        names.sort_unstable();
        formatter.debug_struct("OrderingPolicyRegistry").field("policies", &names).finish()
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

    use super::*;
    use crate::starknet::mempool::{MempoolConfig, MempoolLane, MempoolPhase, PreparedTransaction};

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
            lane: MempoolLane::User,
            prepared,
        }
    }

    fn install(pool: &mut Mempool, hash: Felt, entry: MempoolEntry) {
        pool.entries.insert(hash, entry);
        pool.next_arrival_id = pool.next_arrival_id.saturating_add(1);
    }

    #[test]
    fn registry_resolves_a_policy_by_configured_name() {
        let name: MempoolOrdering = "newest-first".parse().unwrap();
        let mut registry = OrderingPolicyRegistry::default();
        registry.register(name.clone(), NewestFirst);
        let mut pool = Mempool::with_policy_registry(
            MempoolConfig { ordering: name, ..MempoolConfig::default() },
            registry,
        )
        .unwrap();
        let first = Felt::from(0x10);
        let last = Felt::from(0x20);
        install(&mut pool, first, entry_with(first, 0, 0, 1_000));
        install(&mut pool, last, entry_with(last, 1, 0, 1_000));

        assert_eq!(pool.select_policy(&[first, last], 1, 1_000), Some(last));
    }

    #[test]
    fn starknet_comparator_orders_by_descending_tip() {
        let low = entry_with(Felt::from(0x10), 1, 1, 1_000);
        let high = entry_with(Felt::from(0x20), 2, 5, 1_000);
        let mid = entry_with(Felt::from(0x30), 3, 3, 1_000);

        assert_eq!(starknet_comparator(&high, &mid), std::cmp::Ordering::Greater);
        assert_eq!(starknet_comparator(&high, &low), std::cmp::Ordering::Greater);
        assert_eq!(starknet_comparator(&mid, &low), std::cmp::Ordering::Greater);
        assert_eq!(starknet_comparator(&low, &high), std::cmp::Ordering::Less);
    }

    #[test]
    fn starknet_comparator_tie_break_prefers_larger_hash() {
        let smaller_hash = entry_with(Felt::from(0x10), 1, 7, 1_000);
        let larger_hash = entry_with(Felt::from(0x20), 2, 7, 1_000);

        assert_eq!(starknet_comparator(&larger_hash, &smaller_hash), std::cmp::Ordering::Greater);
        assert_eq!(starknet_comparator(&smaller_hash, &larger_hash), std::cmp::Ordering::Less);
    }

    #[test]
    fn starknet_select_policy_returns_none_when_all_entries_below_threshold() {
        let mut pool = Mempool::new(MempoolConfig {
            ordering: MempoolOrdering::starknet(),
            random_seed: 0,
            max_transactions_per_block: 500,
        })
        .unwrap();
        // Every entry is below the threshold of 1_000.
        let first = Felt::from(0x10);
        let second = Felt::from(0x20);
        install(&mut pool, first, entry_with(first, 0, 100, 100));
        install(&mut pool, second, entry_with(second, 1, 50, 100));

        assert_eq!(pool.select_policy(&[first, second], 0, 1_000), None);
    }

    #[test]
    fn starknet_select_policy_skips_below_threshold_entries_among_priority() {
        let mut pool = Mempool::new(MempoolConfig {
            ordering: MempoolOrdering::starknet(),
            random_seed: 0,
            max_transactions_per_block: 500,
        })
        .unwrap();
        let priority_low_tip = Felt::from(0x10);
        let priority_high_tip = Felt::from(0x20);
        let pending_high_tip = Felt::from(0x30);
        let pending_low_tip = Felt::from(0x40);
        // Threshold: 500. Priority entries are >= 500, pending are < 500.
        install(&mut pool, priority_low_tip, entry_with(priority_low_tip, 0, 1, 500));
        install(&mut pool, priority_high_tip, entry_with(priority_high_tip, 1, 10, 500));
        install(&mut pool, pending_high_tip, entry_with(pending_high_tip, 2, 9_999, 1));
        install(&mut pool, pending_low_tip, entry_with(pending_low_tip, 3, 0, 0));

        // First pick: priority_high_tip (highest tip among qualifying entries).
        assert_eq!(
            pool.select_policy(
                &[priority_low_tip, priority_high_tip, pending_high_tip, pending_low_tip],
                0,
                500,
            ),
            Some(priority_high_tip)
        );

        // Then priority_low_tip; pending entries remain invisible.
        assert_eq!(
            pool.select_policy(&[priority_low_tip, pending_high_tip, pending_low_tip], 0, 500,),
            Some(priority_low_tip)
        );

        // Only pending remain — selection returns None.
        assert_eq!(pool.select_policy(&[pending_high_tip, pending_low_tip], 0, 500), None);
    }

    #[test]
    fn starknet_select_policy_picks_below_threshold_after_threshold_drops() {
        // Lowering the threshold below a previously-pending entry's `max_l2_gas_price` makes
        // it eligible for selection.
        let mut pool = Mempool::new(MempoolConfig {
            ordering: MempoolOrdering::starknet(),
            random_seed: 0,
            max_transactions_per_block: 500,
        })
        .unwrap();
        let pending = Felt::from(0x10);
        install(&mut pool, pending, entry_with(pending, 0, 5, 700));

        // Threshold above the tx's price: hidden.
        assert_eq!(pool.select_policy(&[pending], 0, 1_000), None);

        // Threshold dropped below the tx's price: visible, returns the tx.
        assert_eq!(pool.select_policy(&[pending], 0, 100), Some(pending));
    }

    #[test]
    fn forced_selection_advances_the_random_sequence() {
        let config = MempoolConfig {
            ordering: MempoolOrdering::random(),
            random_seed: 42,
            max_transactions_per_block: 500,
        };
        let mut original = Mempool::new(config.clone()).unwrap();
        let mut replay = Mempool::new(config).unwrap();
        let hashes = [Felt::from(0x10), Felt::from(0x20), Felt::from(0x30)];

        for (arrival_id, hash) in hashes.iter().copied().enumerate() {
            install(&mut original, hash, entry_with(hash, arrival_id as u64, 0, 1_000));
            install(&mut replay, hash, entry_with(hash, arrival_id as u64, 0, 1_000));
        }

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
        let mut pool = Mempool::new(MempoolConfig::default()).unwrap();
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
                    random_seed: 0,
                }
            ),
            Some(last)
        );
    }
}
