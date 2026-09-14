use std::collections::HashSet;

use starknet_api::block::FeeType;
use starknet_rs_core::types::Felt;
use starknet_types::felt::TransactionHash;

use super::mempool::{
    BuildFailure, BuildOutcome, ForcedHashSelection, MempoolLane, MempoolPhase, MempoolSelection,
    PolicySelection, SelectionContext, TransactionOrderingPolicy,
};
use super::{Starknet, TransactionEligibility};
use crate::error::{DevnetResult, Error};

const POLICY_SELECTION_ROUND_SIZE: usize = 100;

#[derive(Debug, Default)]
struct PolicySelectionRound {
    user_hashes: Vec<TransactionHash>,
    selections: usize,
    exhausted: bool,
}

impl PolicySelectionRound {
    fn should_refresh(&self) -> bool {
        self.user_hashes.is_empty()
            || self.selections >= POLICY_SELECTION_ROUND_SIZE
            || self.exhausted
    }

    fn refresh(&mut self, user_hashes: Vec<TransactionHash>) {
        self.user_hashes = user_hashes;
        self.selections = 0;
        self.exhausted = false;
    }

    fn retain_eligible(&mut self, eligible_hashes: &[TransactionHash]) {
        self.user_hashes.retain(|hash| eligible_hashes.contains(hash));
    }

    fn record_selection(&mut self, hash: TransactionHash) -> bool {
        let Some(position) = self.user_hashes.iter().position(|candidate| *candidate == hash)
        else {
            return false;
        };
        self.user_hashes.remove(position);
        self.selections += 1;
        self.exhausted = false;
        true
    }

    fn mark_exhausted(&mut self) {
        self.exhausted = true;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockBuilderProgress {
    pub pre_confirmed_transaction_hashes: Vec<TransactionHash>,
    pub remaining_block_capacity: usize,
}

/// Synchronous coordinator for selecting and executing transactions into the open proposal.
///
/// System-lane transactions are selected FIFO before user ordering policies are consulted. User
/// ordering policies choose from a fixed eligible-head snapshot for up to 100 selections; account
/// successors exposed by execution enter the next selection round. This type retains ownership of
/// capacity enforcement, nonce eligibility, execution, and lifecycle transitions.
pub struct BlockBuilder<'a> {
    starknet: &'a mut Starknet,
}

impl<'a> BlockBuilder<'a> {
    pub(crate) fn new(starknet: &'a mut Starknet) -> Self {
        Self { starknet }
    }

    pub fn build_chunk(&mut self, selection: MempoolSelection) -> DevnetResult<BuildOutcome> {
        match selection {
            MempoolSelection::Policy { max_transactions } => {
                self.build_configured_policy_chunk(max_transactions)
            }
            MempoolSelection::Hashes(forced) => self.build_forced_chunk(forced),
        }
    }

    fn build_configured_policy_chunk(
        &mut self,
        max_transactions: Option<usize>,
    ) -> DevnetResult<BuildOutcome> {
        self.build_policy_chunk_inner(max_transactions, None)
    }

    pub fn progress(&self) -> BlockBuilderProgress {
        BlockBuilderProgress {
            pre_confirmed_transaction_hashes: self
                .starknet
                .mempool
                .open_proposal()
                .transaction_hashes()
                .to_vec(),
            remaining_block_capacity: self.starknet.mempool.remaining_capacity(),
        }
    }

    /// Seals exactly the current open proposal without selecting more transactions.
    pub fn seal(self) -> Felt {
        self.starknet.generate_new_block_and_state()
    }

    /// Builds a chunk with a caller-defined ordering rule.
    ///
    /// Returning a hash that is not in the supplied eligible user view is rejected before any
    /// mutation. Eligible system-lane transactions are always selected FIFO before this policy is
    /// consulted.
    pub fn build_policy_chunk(
        &mut self,
        max_transactions: Option<usize>,
        policy: &dyn TransactionOrderingPolicy,
    ) -> DevnetResult<BuildOutcome> {
        self.build_policy_chunk_inner(max_transactions, Some(policy))
    }

    fn build_policy_chunk_inner(
        &mut self,
        max_transactions: Option<usize>,
        policy: Option<&dyn TransactionOrderingPolicy>,
    ) -> DevnetResult<BuildOutcome> {
        let mut outcome = BuildOutcome::default();
        self.starknet.evict_stale_received_transactions(&mut outcome)?;
        if self.starknet.mempool.remaining_capacity() == 0 {
            outcome.block_full = true;
            return Ok(outcome);
        }

        let requested_limit = max_transactions.unwrap_or(usize::MAX);
        let mut selection_round = PolicySelectionRound::default();
        let mut attempts: usize = 0;
        while attempts < requested_limit {
            self.starknet.evict_stale_received_transactions(&mut outcome)?;
            if self.starknet.mempool.remaining_capacity() == 0 {
                break;
            }

            let eligible_hashes = self.starknet.eligible_hashes()?;
            if let Some(system_hash) = self.oldest_eligible_system_hash(&eligible_hashes) {
                self.process_selected(system_hash, &mut outcome)?;
                attempts = attempts.saturating_add(1);
                continue;
            }

            selection_round.retain_eligible(&eligible_hashes);
            if selection_round.should_refresh() {
                let new_view = self.eligible_user_hashes(&eligible_hashes);
                if selection_round.exhausted && selection_round.user_hashes == new_view {
                    break;
                }
                selection_round.refresh(new_view);
            }

            let selected = {
                let eligible =
                    self.starknet.mempool.eligible_transactions(&selection_round.user_hashes);
                let context = SelectionContext {
                    block_number: self.starknet.blocks.pre_confirmed_block.block_number().0,
                    current_l2_gas_price: self
                        .starknet
                        .block_context
                        .block_info()
                        .gas_prices
                        .l2_gas_price(&FeeType::Strk)
                        .get()
                        .0,
                    proposal_selection_counter: self
                        .starknet
                        .mempool
                        .open_proposal()
                        .selection_counter(),
                    random_seed: self.starknet.mempool.config().random_seed,
                };
                match policy {
                    Some(policy) => policy.select_in_round(&eligible, &context),
                    None => self.starknet.mempool.select_configured_policy(&eligible, &context)?,
                }
            };
            let hash = match selected {
                PolicySelection::Transaction(hash) => hash,
                PolicySelection::Stop => break,
                PolicySelection::RoundExhausted => {
                    selection_round.mark_exhausted();
                    continue;
                }
            };
            if !selection_round.record_selection(hash) {
                return Err(Error::UnsupportedAction {
                    msg: format!(
                        "Transaction ordering policy selected ineligible transaction {hash:#x} \
                         outside the current selection round"
                    ),
                });
            }
            self.process_selected(hash, &mut outcome)?;
            attempts = attempts.saturating_add(1);
        }

        outcome.block_full = self.starknet.mempool.remaining_capacity() == 0;
        Ok(outcome)
    }

    fn eligible_user_hashes(&self, eligible_hashes: &[TransactionHash]) -> Vec<TransactionHash> {
        eligible_hashes
            .iter()
            .copied()
            .filter(|hash| {
                self.starknet.mempool.get(hash).is_some_and(|entry| entry.lane == MempoolLane::User)
            })
            .collect()
    }

    fn oldest_eligible_system_hash(
        &self,
        eligible_hashes: &[TransactionHash],
    ) -> Option<TransactionHash> {
        eligible_hashes
            .iter()
            .filter_map(|hash| {
                let entry = self.starknet.mempool.get(hash)?;
                (entry.lane == MempoolLane::System).then_some((entry.arrival_id, *hash))
            })
            .min_by_key(|(arrival_id, _)| *arrival_id)
            .map(|(_, hash)| hash)
    }

    fn build_forced_chunk(&mut self, forced: ForcedHashSelection) -> DevnetResult<BuildOutcome> {
        let ForcedHashSelection { transaction_hashes, swept_stale_hashes } = forced;
        self.preflight_forced_hashes(&transaction_hashes)?;
        self.preflight_forced_hashes(&swept_stale_hashes)?;
        let mut swept_failures = Vec::with_capacity(swept_stale_hashes.len());
        for hash in &swept_stale_hashes {
            if transaction_hashes.contains(hash) {
                return Err(Error::UnsupportedAction {
                    msg: format!("Transaction {hash:#x} cannot be both selected and swept"),
                });
            }
            match self.starknet.eligibility(*hash)? {
                TransactionEligibility::Stale(reason) => {
                    swept_failures.push(BuildFailure { transaction_hash: *hash, reason });
                }
                _ => {
                    return Err(Error::UnsupportedAction {
                        msg: format!("Transaction {hash:#x} is not stale"),
                    });
                }
            }
        }
        let mut outcome = BuildOutcome::default();

        for failure in swept_failures {
            self.starknet.mempool.remove_entry(&failure.transaction_hash);
            outcome.swept_stale_hashes.push(failure.transaction_hash);
            outcome.rejected.push(failure);
        }

        for hash in transaction_hashes {
            if self.starknet.mempool.remaining_capacity() == 0 {
                break;
            }
            self.process_selected(hash, &mut outcome)?;
        }
        outcome.block_full = self.starknet.mempool.remaining_capacity() == 0;
        Ok(outcome)
    }

    fn preflight_forced_hashes(&self, hashes: &[TransactionHash]) -> DevnetResult<()> {
        let mut unique = HashSet::new();
        for hash in hashes {
            if !unique.insert(*hash) {
                return Err(Error::UnsupportedAction {
                    msg: format!("Transaction hash {hash:#x} is duplicated"),
                });
            }
            let entry = self.starknet.mempool.get(hash).ok_or(Error::NoTransaction)?;
            if entry.phase != MempoolPhase::Received {
                return Err(Error::UnsupportedAction {
                    msg: format!("Transaction {hash:#x} is not RECEIVED"),
                });
            }
        }
        Ok(())
    }

    fn process_selected(
        &mut self,
        hash: TransactionHash,
        outcome: &mut BuildOutcome,
    ) -> DevnetResult<()> {
        outcome.selected.push(hash);
        self.starknet.mempool.record_selection();

        match self.starknet.eligibility(hash)? {
            TransactionEligibility::Eligible => {
                match self.starknet.execute_mempool_transaction(hash) {
                    Ok(()) => outcome.pre_confirmed.push(hash),
                    Err(error) => {
                        self.starknet.mempool.remove_entry(&hash);
                        outcome.rejected.push(BuildFailure {
                            transaction_hash: hash,
                            reason: error.to_string(),
                        });
                    }
                }
            }
            TransactionEligibility::Blocked(reason) => {
                outcome.blocked.push(BuildFailure { transaction_hash: hash, reason });
            }
            TransactionEligibility::Stale(reason) => {
                self.starknet.mempool.remove_entry(&hash);
                outcome.rejected.push(BuildFailure { transaction_hash: hash, reason });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use starknet_types::rpc::transactions::l1_handler_transaction::L1HandlerTransaction;
    use starknet_types::rpc::transactions::{Transaction, TransactionWithHash};

    use super::*;
    use crate::starknet::mempool::{EligibleTransactions, PreparedTransaction};

    struct IneligiblePolicy;

    impl TransactionOrderingPolicy for IneligiblePolicy {
        fn select(
            &self,
            _eligible: &EligibleTransactions<'_>,
            _context: &SelectionContext,
        ) -> Option<TransactionHash> {
            Some(Felt::from(0x999))
        }
    }

    #[test]
    fn custom_policy_cannot_select_outside_the_eligible_view() {
        let mut starknet = Starknet::default();
        let error =
            starknet.block_builder().build_policy_chunk(Some(1), &IneligiblePolicy).unwrap_err();
        assert!(
            matches!(error, Error::UnsupportedAction { msg } if msg.contains("ineligible transaction"))
        );
        assert_eq!(starknet.mempool.entries().count(), 0);
    }

    #[test]
    fn system_lane_is_selected_fifo_before_user_policy() {
        let mut starknet = Starknet::default();
        let first = Felt::from(0x10);
        let second = Felt::from(0x20);
        for hash in [first, second] {
            let transaction = TransactionWithHash::new(
                hash,
                Transaction::L1Handler(L1HandlerTransaction::default()),
            );
            starknet
                .mempool
                .admit(PreparedTransaction::system(transaction, Default::default()))
                .unwrap();
        }

        let outcome =
            starknet.block_builder().build_policy_chunk(Some(2), &IneligiblePolicy).unwrap();
        assert_eq!(outcome.selected, vec![first, second]);
    }

    #[test]
    fn selection_round_defers_new_hashes_until_refresh() {
        let first = Felt::from(0x10);
        let second = Felt::from(0x20);
        let successor = Felt::from(0x30);
        let mut round = PolicySelectionRound::default();
        round.refresh(vec![first, second]);

        assert!(round.record_selection(first));
        round.retain_eligible(&[second, successor]);
        assert_eq!(round.user_hashes, vec![second]);

        assert!(round.record_selection(second));
        assert!(round.should_refresh());
        round.refresh(vec![successor]);
        assert_eq!(round.user_hashes, vec![successor]);
    }

    #[test]
    fn selection_round_refreshes_after_fixed_selection_limit() {
        let hashes = (0..=POLICY_SELECTION_ROUND_SIZE)
            .map(|value| Felt::from(value as u64))
            .collect::<Vec<_>>();
        let mut round = PolicySelectionRound::default();
        round.refresh(hashes.clone());

        for hash in hashes.into_iter().take(POLICY_SELECTION_ROUND_SIZE) {
            assert!(round.record_selection(hash));
        }

        assert_eq!(round.user_hashes.len(), 1);
        assert!(round.should_refresh());
    }

    #[test]
    fn custom_policy_none_stops_before_refreshing_successors() {
        use starknet_rs_core::utils::get_selector_from_name;

        use super::super::starknet_config::BlockGenerationOn;
        use super::super::tests::setup_starknet_with_no_signature_check_account;
        use crate::constants::ETH_ERC20_CONTRACT_ADDRESS;
        use crate::traits::Deployed;
        use crate::utils::test_utils::{resource_bounds_with_price_1, test_invoke_transaction_v3};

        struct SelectWithPeer;
        impl TransactionOrderingPolicy for SelectWithPeer {
            fn select(
                &self,
                eligible: &EligibleTransactions<'_>,
                _context: &SelectionContext,
            ) -> Option<TransactionHash> {
                if eligible.len() < 2 { None } else { eligible.hashes().first().copied() }
            }
        }

        for configured in [false, true] {
            let (mut starknet, account) =
                setup_starknet_with_no_signature_check_account(1_000_000_000);
            starknet.config.block_generation_on = BlockGenerationOn::Mempool;
            if configured {
                let mut config = starknet.mempool.config().clone();
                config.ordering = "select-with-peer".parse().unwrap();
                let mut registry = super::super::mempool::OrderingPolicyRegistry::default();
                registry.register(config.ordering.clone(), SelectWithPeer);
                starknet.mempool =
                    super::super::mempool::Mempool::with_policy_registry(config, registry).unwrap();
            }
            let mut hashes = Vec::new();
            for nonce in 0..2 {
                let tx = test_invoke_transaction_v3(
                    account.get_address(),
                    starknet_types::contract_address::ContractAddress::new(
                        ETH_ERC20_CONTRACT_ADDRESS,
                    )
                    .unwrap(),
                    get_selector_from_name("balanceOf").unwrap(),
                    &[account.get_address().into()],
                    nonce,
                    resource_bounds_with_price_1(1_000_000, 1_000_000, 100_000_000),
                );
                hashes.push(starknet.add_invoke_transaction(tx).unwrap());
            }
            let peer = Felt::from(0x999);
            let mut prepared = PreparedTransaction::system(
                TransactionWithHash::new(
                    peer,
                    Transaction::L1Handler(L1HandlerTransaction::default()),
                ),
                Default::default(),
            );
            prepared.lane = MempoolLane::User;
            starknet.mempool.admit(prepared).unwrap();
            let outcome = if configured {
                starknet.preconfirm_transactions(MempoolSelection::default()).unwrap()
            } else {
                starknet.preconfirm_transactions_with_policy(None, &SelectWithPeer).unwrap()
            };
            assert_eq!(outcome.pre_confirmed, vec![hashes[0]]);
            assert_eq!(starknet.mempool.get(&hashes[1]).unwrap().phase, MempoolPhase::Received);
            assert_eq!(starknet.mempool.get(&peer).unwrap().phase, MempoolPhase::Received);
        }
    }

    #[test]
    fn progress_reports_open_proposal_and_capacity() {
        let mut starknet = Starknet::default();
        let progress = starknet.block_builder().progress();
        assert!(progress.pre_confirmed_transaction_hashes.is_empty());
        assert_eq!(
            progress.remaining_block_capacity,
            starknet.config.mempool_config.max_transactions_per_block
        );
    }
}
