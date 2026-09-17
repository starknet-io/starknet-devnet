use std::sync::Arc;

use parking_lot::RwLock;

use super::Starknet;

/// An isolated, process-local copy of all mutable state owned by [`Starknet`].
///
/// This type is opaque so capture remains the single inventory of checkpointed core state.
pub struct StarknetCheckpoint(Starknet);

impl StarknetCheckpoint {
    pub fn accepted_block_hashes(&self) -> Vec<(u64, starknet_types::felt::BlockHash)> {
        self.0.accepted_block_hashes()
    }

    pub fn latest_block(&self) -> crate::error::DevnetResult<crate::StarknetBlock> {
        self.0.get_latest_block()
    }

    pub fn capture(starknet: &Starknet) -> Self {
        let Starknet {
            latest_state,
            pre_confirmed_state,
            pre_confirmed_state_diff,
            predeployed_accounts,
            block_context,
            blocks,
            transactions,
            mempool,
            config,
            pre_confirmed_block_timestamp_shift,
            next_block_timestamp,
            next_block_gas,
            messaging,
            rpc_contract_classes,
            cheats,
        } = starknet;

        let rpc_contract_classes = Arc::new(RwLock::new(rpc_contract_classes.read().clone()));
        let mut latest_state = latest_state.clone();
        latest_state.relink_rpc_contract_classes(rpc_contract_classes.clone());
        let mut pre_confirmed_state = pre_confirmed_state.clone();
        pre_confirmed_state.relink_rpc_contract_classes(rpc_contract_classes.clone());
        let mut blocks = blocks.clone();
        blocks.relink_rpc_contract_classes(rpc_contract_classes.clone());

        Self(Starknet {
            latest_state,
            pre_confirmed_state,
            pre_confirmed_state_diff: pre_confirmed_state_diff.clone(),
            predeployed_accounts: predeployed_accounts.clone(),
            block_context: block_context.clone(),
            blocks,
            transactions: transactions.clone(),
            mempool: mempool.clone(),
            config: config.clone(),
            pre_confirmed_block_timestamp_shift: *pre_confirmed_block_timestamp_shift,
            next_block_timestamp: *next_block_timestamp,
            next_block_gas: next_block_gas.clone(),
            messaging: messaging.clone(),
            rpc_contract_classes,
            cheats: cheats.clone(),
        })
    }

    pub fn restore(self, starknet: &mut Starknet) {
        *starknet = self.0;
    }
}

impl Starknet {
    pub fn checkpoint(&self) -> StarknetCheckpoint {
        StarknetCheckpoint::capture(self)
    }

    pub fn restore_checkpoint(&mut self, checkpoint: StarknetCheckpoint) {
        checkpoint.restore(self);
        self.sync_metrics();
    }
}

#[cfg(test)]
mod tests {
    use blockifier::state::state_api::{State, StateReader};
    use starknet_api::state::StorageKey;
    use starknet_rs_core::types::Felt;

    use super::Starknet;
    use crate::state::{CustomState, CustomStateReader};
    use crate::utils::test_utils::{dummy_cairo_1_contract_class, dummy_felt};

    #[test]
    fn restores_cached_pre_confirmed_writes_in_isolation() {
        let mut starknet = Starknet::default();
        let address = starknet_api::core::ContractAddress::from(123_u32);
        let key = StorageKey::try_from(Felt::from(456_u32)).unwrap();
        starknet.pre_confirmed_state.set_storage_at(address, key, Felt::ONE).unwrap();

        let checkpoint = starknet.checkpoint();
        starknet.pre_confirmed_state.set_storage_at(address, key, Felt::TWO).unwrap();
        assert_eq!(starknet.pre_confirmed_state.get_storage_at(address, key).unwrap(), Felt::TWO);

        starknet.restore_checkpoint(checkpoint);
        assert_eq!(starknet.pre_confirmed_state.get_storage_at(address, key).unwrap(), Felt::ONE);
    }

    #[test]
    fn declarations_after_capture_do_not_leak_into_checkpoint() {
        let mut starknet = Starknet::new(&Default::default()).unwrap();
        let class_hash = dummy_felt();
        let checkpoint = starknet.checkpoint();

        starknet
            .pre_confirmed_state
            .declare_contract_class(
                class_hash,
                Some(dummy_felt()),
                dummy_cairo_1_contract_class().into(),
            )
            .unwrap();
        assert!(starknet.pre_confirmed_state.is_contract_declared(class_hash));

        starknet.restore_checkpoint(checkpoint);
        assert!(!starknet.pre_confirmed_state.is_contract_declared(class_hash));
    }
}
