use std::sync::Arc;

use parking_lot::RwLock;

use super::Starknet;

/// An isolated, process-local copy of all mutable state owned by [`Starknet`].
///
/// This type is opaque so capture remains the single inventory of checkpointed core state.
pub struct StarknetCheckpoint(Starknet);

impl StarknetCheckpoint {
    pub fn as_starknet(&self) -> &Starknet {
        &self.0
    }

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
        starknet.sync_metrics();
    }
}

impl Starknet {
    pub fn checkpoint(&self) -> StarknetCheckpoint {
        StarknetCheckpoint::capture(self)
    }

    pub fn restore_checkpoint(&mut self, checkpoint: StarknetCheckpoint) {
        checkpoint.restore(self);
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU128;

    use alloy::primitives::B256;
    use blockifier::state::state_api::{State, StateReader};
    use starknet_api::state::StorageKey;
    use starknet_rs_core::types::Felt;
    use starknet_types::contract_address::ContractAddress;
    use starknet_types::rpc::block::{BlockId, BlockTag};
    use starknet_types::rpc::gas_modification::GasModificationRequest;

    use super::Starknet;
    use crate::starknet::starknet_config::StarknetConfig;
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
        let pre_confirmed = BlockId::Tag(BlockTag::PreConfirmed);
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
        assert!(starknet.get_class(&pre_confirmed, class_hash).is_ok());

        starknet.restore_checkpoint(checkpoint);
        assert!(!starknet.pre_confirmed_state.is_contract_declared(class_hash));
        assert!(starknet.get_class(&pre_confirmed, class_hash).is_err());
    }

    #[test]
    fn restores_controls_impersonation_and_messaging_queues() {
        let mut starknet =
            Starknet::new(&StarknetConfig { lite_mode: true, ..Default::default() }).unwrap();
        let retained_account = ContractAddress::new(Felt::from(0x123_u64)).unwrap();
        let discarded_account = ContractAddress::new(Felt::from(0x456_u64)).unwrap();
        let message_hash = B256::repeat_byte(7);

        starknet.set_time(42, false);
        starknet.set_next_block_gas(gas_price_request(123)).unwrap();
        starknet.cheats.impersonate_account(retained_account);
        starknet.messaging.last_local_block = 11;
        starknet.messaging.l2_to_l1_messages_hashes.insert(message_hash, 2);
        let checkpoint = starknet.checkpoint();

        starknet.set_time(91, false);
        starknet.set_next_block_gas(gas_price_request(456)).unwrap();
        starknet.cheats.stop_impersonating_account(&retained_account);
        starknet.cheats.impersonate_account(discarded_account);
        starknet.cheats.set_auto_impersonate(true);
        starknet.messaging.last_local_block = 27;
        starknet.messaging.l2_to_l1_messages_hashes.insert(message_hash, 5);

        starknet.restore_checkpoint(checkpoint);
        assert!(starknet.cheats.is_impersonated(&retained_account));
        assert!(!starknet.cheats.is_impersonated(&discarded_account));
        assert!(!starknet.cheats.is_auto_impersonate());
        assert_eq!(starknet.messaging.last_local_block, 11);
        assert_eq!(starknet.messaging.l2_to_l1_messages_hashes.get(&message_hash), Some(&2));

        starknet.create_block();
        let header = &starknet.get_latest_block().unwrap().header.block_header_without_hash;
        assert_eq!(header.timestamp.0, 42);
        assert_eq!(header.l1_gas_price.price_in_fri.0, 123);
    }

    fn gas_price_request(value: u128) -> GasModificationRequest {
        GasModificationRequest {
            gas_price_wei: None,
            data_gas_price_wei: None,
            gas_price_fri: NonZeroU128::new(value),
            data_gas_price_fri: None,
            l2_gas_price_wei: None,
            l2_gas_price_fri: None,
            generate_block: None,
        }
    }
}
