use std::collections::HashSet;

use starknet_rs_core::types::Felt;
use starknet_types::felt::TransactionHash;

use super::mempool::{
    BuildFailure, BuildOutcome, ConfiguredOrderingPolicy, MempoolPhase, MempoolSelection,
    SelectionContext, TransactionOrderingPolicy,
};
use super::{Starknet, TransactionEligibility};
use crate::error::{DevnetResult, Error};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockBuilderProgress {
    pub pre_confirmed_transaction_hashes: Vec<TransactionHash>,
    pub remaining_block_capacity: usize,
}

/// Synchronous coordinator for selecting and executing transactions into the open proposal.
///
/// Ordering policies only choose among the immutable eligible view. This type retains ownership of
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
                let policy = ConfiguredOrderingPolicy::from_config(self.starknet.mempool.config());
                self.build_policy_chunk(max_transactions, &policy)
            }
            MempoolSelection::Hashes(hashes) => self.build_forced_chunk(hashes),
        }
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
    /// Returning a hash that is not in the supplied eligible view is rejected before any mutation.
    pub fn build_policy_chunk(
        &mut self,
        max_transactions: Option<usize>,
        policy: &dyn TransactionOrderingPolicy,
    ) -> DevnetResult<BuildOutcome> {
        let mut outcome = BuildOutcome::default();
        self.starknet.evict_stale_received_transactions(&mut outcome)?;
        if self.starknet.mempool.remaining_capacity() == 0 {
            outcome.block_full = true;
            return Ok(outcome);
        }

        let requested_limit = max_transactions.unwrap_or(usize::MAX);
        let limit = requested_limit.min(self.starknet.mempool.remaining_capacity());
        for _ in 0..limit {
            self.starknet.evict_stale_received_transactions(&mut outcome)?;
            let eligible_hashes = self.starknet.eligible_hashes()?;
            let selected = {
                let eligible = self.starknet.mempool.eligible_transactions(&eligible_hashes);
                let context = SelectionContext {
                    block_number: self.starknet.blocks.pre_confirmed_block.block_number().0,
                    current_l2_gas_price: self.starknet.next_block_gas.l2_gas_price_fri.get(),
                    proposal_selection_counter: self
                        .starknet
                        .mempool
                        .open_proposal()
                        .selection_counter(),
                };
                policy.select(&eligible, &context)
            };
            let Some(hash) = selected else { break };
            if !eligible_hashes.contains(&hash) {
                return Err(Error::UnsupportedAction {
                    msg: format!(
                        "Transaction ordering policy selected ineligible transaction {hash:#x}"
                    ),
                });
            }
            self.process_selected(hash, &mut outcome)?;
        }

        outcome.block_full = self.starknet.mempool.remaining_capacity() == 0;
        Ok(outcome)
    }

    fn build_forced_chunk(&mut self, hashes: Vec<TransactionHash>) -> DevnetResult<BuildOutcome> {
        self.preflight_forced_hashes(&hashes)?;
        let mut outcome = BuildOutcome::default();
        if self.starknet.mempool.remaining_capacity() == 0 {
            outcome.block_full = true;
            return Ok(outcome);
        }

        let limit = hashes.len().min(self.starknet.mempool.remaining_capacity());
        for hash in hashes.into_iter().take(limit) {
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
        let hash = Felt::from(0x10);
        let transaction =
            TransactionWithHash::new(hash, Transaction::L1Handler(L1HandlerTransaction::default()));
        starknet
            .mempool
            .admit(PreparedTransaction::system(transaction, Default::default()))
            .unwrap();

        let error =
            starknet.block_builder().build_policy_chunk(Some(1), &IneligiblePolicy).unwrap_err();
        assert!(
            matches!(error, Error::UnsupportedAction { msg } if msg.contains("ineligible transaction"))
        );
        assert_eq!(starknet.mempool.get(&hash).unwrap().phase, MempoolPhase::Received);
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
