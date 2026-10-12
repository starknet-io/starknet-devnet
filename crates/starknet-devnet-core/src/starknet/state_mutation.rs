use std::collections::HashSet;
use std::hash::Hash;

use blockifier::execution::contract_class::RunnableCompiledClass;
use blockifier::state::errors::StateError as BlockifierStateError;
use blockifier::state::state_api::{State, StateReader};
use starknet_api::contract_class::compiled_class_hash::{HashVersion, HashableCompiledClass};
use starknet_api::core::ClassHash as ApiClassHash;
use starknet_rs_core::types::Felt;
use starknet_types::compile_sierra_contract;
use starknet_types::contract_class::ContractClass;
use starknet_types::rpc::state_mutation::{
    AppliedClass, ApplyStateDiffRequest, ApplyStateDiffResponse,
};
use starknet_types::traits::TryHashProducer;

use super::Starknet;
use crate::error::{DevnetResult, Error};
use crate::state::{CustomState, CustomStateReader};

impl Starknet {
    /// Applies out-of-band changes without executing a transaction or sealing a block.
    pub fn apply_state_diff(
        &mut self,
        request: ApplyStateDiffRequest,
    ) -> DevnetResult<ApplyStateDiffResponse> {
        let checkpoint = self.checkpoint();
        match self.apply_state_diff_inner(request) {
            Ok(response) => Ok(response),
            Err(error) => {
                self.restore_checkpoint(checkpoint);
                Err(match error {
                    Error::BlockifierStateError(BlockifierStateError::UndeclaredClassHash(
                        hash,
                    )) => Error::StateError(crate::error::StateError::NoneClassHash(hash.0)),
                    other => other,
                })
            }
        }
    }

    fn apply_state_diff_inner(
        &mut self,
        request: ApplyStateDiffRequest,
    ) -> DevnetResult<ApplyStateDiffResponse> {
        let diff = request.state_diff;
        if diff.migrated_compiled_classes.as_ref().is_some_and(|classes| !classes.is_empty()) {
            return Err(invalid("state_diff.migrated_compiled_classes is unsupported"));
        }
        unique(diff.storage_diffs.iter().map(|item| item.address), "storage_diffs.address")?;
        for item in &diff.storage_diffs {
            unique(item.storage_entries.iter().map(|entry| entry.key), "storage_entries.key")?;
        }
        unique(diff.nonces.iter().map(|item| item.contract_address), "nonces.contract_address")?;
        unique(
            diff.deployed_contracts.iter().map(|item| item.address),
            "deployed_contracts.address",
        )?;
        unique(
            diff.replaced_classes.iter().map(|item| item.contract_address),
            "replaced_classes.contract_address",
        )?;
        unique(
            diff.declared_classes.iter().map(|item| item.class_hash),
            "declared_classes.class_hash",
        )?;
        unique(diff.deprecated_declared_classes.iter().copied(), "deprecated_declared_classes")?;
        let deployments: HashSet<_> =
            diff.deployed_contracts.iter().map(|item| item.address).collect();
        if diff.replaced_classes.iter().any(|item| deployments.contains(&item.contract_address)) {
            return Err(invalid("deployed_contracts and replaced_classes overlap"));
        }
        if diff
            .declared_classes
            .iter()
            .any(|item| diff.deprecated_declared_classes.contains(&item.class_hash))
        {
            return Err(invalid("declared_classes and deprecated_declared_classes overlap"));
        }

        let mut classes = Vec::with_capacity(request.classes.len());
        let mut supplied_hashes = HashSet::new();
        for (index, supplied) in request.classes.into_iter().enumerate() {
            let contract_class: ContractClass = supplied.contract_class.try_into()?;
            let class_hash = contract_class.try_generate_hash()?;
            if !supplied_hashes.insert(class_hash) {
                return Err(invalid(format!("classes[{index}]: duplicate class definition")));
            }
            if supplied.class_hash.is_some_and(|hash| hash != class_hash) {
                return Err(invalid(format!(
                    "classes[{index}].class_hash does not match definition"
                )));
            }
            let compiled_class_hash = match &contract_class {
                ContractClass::Cairo0(_) => None,
                ContractClass::Cairo1(sierra) => {
                    Some(compile_sierra_contract(sierra)?.hash(&HashVersion::V2).0)
                }
            };
            if supplied.compiled_class_hash.is_some()
                && supplied.compiled_class_hash != compiled_class_hash
            {
                return Err(invalid(format!(
                    "classes[{index}].compiled_class_hash does not match definition"
                )));
            }
            match self.pre_confirmed_state.get_compiled_class(ApiClassHash(class_hash)) {
                Ok(known) => {
                    if compiled_hash(&known) != compiled_class_hash {
                        return Err(invalid(format!(
                            "classes[{index}]: conflicts with known class"
                        )));
                    }
                }
                Err(BlockifierStateError::UndeclaredClassHash(_)) => {
                    self.pre_confirmed_state.declare_contract_class(
                        class_hash,
                        compiled_class_hash,
                        contract_class,
                    )?;
                }
                Err(error) => return Err(error.into()),
            }
            classes.push(AppliedClass { class_hash, compiled_class_hash });
        }

        for item in &diff.declared_classes {
            let class =
                self.pre_confirmed_state.get_compiled_class(ApiClassHash(item.class_hash))?;
            if compiled_hash(&class) != Some(item.compiled_class_hash) {
                return Err(invalid(
                    "declared_classes.compiled_class_hash does not match a Sierra class",
                ));
            }
        }
        for hash in &diff.deprecated_declared_classes {
            let class = self.pre_confirmed_state.get_compiled_class(ApiClassHash(*hash))?;
            if !matches!(class, RunnableCompiledClass::V0(_)) {
                return Err(invalid("deprecated_declared_classes must reference Cairo 0 classes"));
            }
        }

        for item in &diff.deployed_contracts {
            if item.address == starknet_types::contract_address::ContractAddress::zero()
                || self.pre_confirmed_state.is_contract_deployed(item.address)?
            {
                return Err(invalid("deployed_contracts.address must be nonzero and unused"));
            }
            self.pre_confirmed_state.get_compiled_class(ApiClassHash(item.class_hash))?;
            // Read before writing so Blockifier can calculate the change from the parent state.
            self.pre_confirmed_state
                .set_class_hash_at(item.address.into(), ApiClassHash(item.class_hash))?;
        }
        for item in &diff.replaced_classes {
            self.pre_confirmed_state.assert_contract_deployed(item.contract_address)?;
            self.pre_confirmed_state.get_compiled_class(ApiClassHash(item.class_hash))?;
            self.pre_confirmed_state
                .set_class_hash_at(item.contract_address.into(), ApiClassHash(item.class_hash))?;
        }
        for item in diff.storage_diffs {
            self.pre_confirmed_state.assert_contract_deployed(item.address)?;
            for entry in item.storage_entries {
                self.pre_confirmed_state.set_storage_at(
                    item.address.into(),
                    entry.key.into(),
                    entry.value,
                )?;
            }
        }
        for item in diff.nonces {
            self.pre_confirmed_state.assert_contract_deployed(item.contract_address)?;
            self.pre_confirmed_state.set_nonce(item.contract_address, item.nonce)?;
        }
        self.commit_diff()?;
        Ok(ApplyStateDiffResponse { classes })
    }
}

fn compiled_hash(class: &RunnableCompiledClass) -> Option<Felt> {
    match class {
        RunnableCompiledClass::V0(_) => None,
        RunnableCompiledClass::V1(class) => Some(class.hash(&HashVersion::V2).0),
        #[cfg(feature = "cairo_native")]
        RunnableCompiledClass::V1Native(class) => Some(class.casm().hash(&HashVersion::V2).0),
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidStateDiff(message.into())
}

fn unique<T: Eq + Hash>(items: impl Iterator<Item = T>, field: &str) -> DevnetResult<()> {
    let mut seen = HashSet::new();
    for item in items {
        if !seen.insert(item) {
            return Err(invalid(format!("state_diff.{field}: duplicate entry")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use starknet_types::rpc::block::{BlockId, BlockTag};

    use super::*;
    use crate::starknet::starknet_config::{BlockGenerationOn, StarknetConfig};
    use crate::utils::test_utils::{
        cairo_0_account_without_validations, dummy_cairo_1_contract_class,
    };

    fn setup() -> Starknet {
        Starknet::new(&StarknetConfig {
            block_generation_on: BlockGenerationOn::Demand,
            ..Default::default()
        })
        .unwrap()
    }

    fn apply(
        starknet: &mut Starknet,
        diff: serde_json::Value,
    ) -> DevnetResult<ApplyStateDiffResponse> {
        starknet.apply_state_diff(serde_json::from_value(json!({ "state_diff": diff })).unwrap())
    }

    #[test]
    fn absolute_nonces_and_zero_storage_preserve_latest() {
        let mut starknet = setup();
        let address = starknet.predeployed_accounts.get_accounts()[0].account_address;
        for nonce in [Felt::from(10), Felt::from(3), Felt::ZERO] {
            apply(&mut starknet, json!({
                "nonces": [{ "contract_address": address, "nonce": nonce }],
                "storage_diffs": [{ "address": address, "storage_entries": [{ "key": "0x55", "value": nonce }] }]
            })).unwrap();
            assert_eq!(starknet.pre_confirmed_state.get_nonce_at(address.into()).unwrap().0, nonce);
            assert_eq!(starknet.latest_state.get_nonce_at(address.into()).unwrap().0, Felt::ZERO);
            assert_eq!(
                starknet
                    .block_state_update(&BlockId::Tag(BlockTag::PreConfirmed), None)
                    .unwrap()
                    .get_state_diff()
                    .nonces[0]
                    .nonce,
                nonce
            );
        }
        assert_eq!(starknet.pre_confirmed_state_diff.address_to_nonce[&address], Felt::ZERO);
        assert_eq!(starknet.transactions.len(), 0);
        starknet.generate_new_block_and_state();
        let diff = starknet.block_state_update(&BlockId::Tag(BlockTag::Latest), None).unwrap();
        assert_eq!(diff.get_state_diff().nonces[0].nonce, Felt::ZERO);
        starknet.generate_new_block_and_state();
        assert!(
            starknet
                .block_state_update(&BlockId::Tag(BlockTag::Latest), None)
                .unwrap()
                .get_state_diff()
                .is_empty()
        );
    }

    #[test]
    fn failed_request_restores_storage_nonce_and_supplied_class_registry() {
        let mut starknet = setup();
        let address = starknet.predeployed_accounts.get_accounts()[0].account_address;
        let class = ContractClass::Cairo1(dummy_cairo_1_contract_class());
        let hash = class.try_generate_hash().unwrap();
        let codegen: starknet_rs_core::types::ContractClass = class.try_into().unwrap();
        let before = starknet
            .block_state_update(&BlockId::Tag(BlockTag::PreConfirmed), None)
            .unwrap()
            .get_state_diff()
            .clone();
        let request = serde_json::from_value(json!({
            "classes": [{ "contract_class": codegen }],
            "state_diff": {
                "storage_diffs": [{ "address": address, "storage_entries": [{ "key": "0x55", "value": "0x5" }] }],
                "nonces": [
                    { "contract_address": address, "nonce": "0x10" },
                    { "contract_address": "0xdead", "nonce": "0x1" }
                ]
            }
        })).unwrap();
        assert!(starknet.apply_state_diff(request).is_err());
        assert_eq!(
            starknet.pre_confirmed_state.get_nonce_at(address.into()).unwrap().0,
            Felt::ZERO
        );
        assert_eq!(
            starknet
                .pre_confirmed_state
                .get_storage_at(
                    address.into(),
                    starknet_api::state::StorageKey::try_from(Felt::from(0x55)).unwrap()
                )
                .unwrap(),
            Felt::ZERO
        );
        assert!(!starknet.pre_confirmed_state.is_contract_declared(hash));
        assert!(starknet.get_class(&BlockId::Tag(BlockTag::PreConfirmed), hash).is_err());
        assert_eq!(
            starknet
                .block_state_update(&BlockId::Tag(BlockTag::PreConfirmed), None)
                .unwrap()
                .get_state_diff(),
            &before
        );
    }

    #[test]
    fn supplied_classes_auto_declare_and_known_classes_are_noops() {
        let mut starknet = setup();
        for class in [
            ContractClass::Cairo0(cairo_0_account_without_validations()),
            ContractClass::Cairo1(dummy_cairo_1_contract_class()),
        ] {
            let hash = class.try_generate_hash().unwrap();
            let codegen: starknet_rs_core::types::ContractClass = class.try_into().unwrap();
            let request: ApplyStateDiffRequest = serde_json::from_value(json!({
                "state_diff": {}, "classes": [{ "contract_class": codegen, "class_hash": hash }]
            }))
            .unwrap();
            let response = starknet.apply_state_diff(request.clone()).unwrap();
            assert_eq!(response.classes[0].class_hash, hash);
            assert!(starknet.get_class(&BlockId::Tag(BlockTag::Latest), hash).is_err());
            starknet.generate_new_block_and_state();
            starknet.apply_state_diff(request).unwrap();
            let diff: starknet_types::rpc::state::ThinStateDiff =
                starknet.pre_confirmed_state_diff.clone().into();
            assert!(diff.is_empty());
            assert!(starknet.get_class(&BlockId::Tag(BlockTag::Latest), hash).is_ok());
        }
    }

    #[test]
    fn invalid_categories_and_conflicts_are_rejected() {
        let mut starknet = setup();
        let address = starknet.predeployed_accounts.get_accounts()[0].account_address;
        for diff in [
            json!({ "nonces": [{ "contract_address": address, "nonce": "0x1" }, { "contract_address": address, "nonce": "0x2" }] }),
            json!({ "storage_diffs": [{ "address": address, "storage_entries": [] }, { "address": address, "storage_entries": [] }] }),
            json!({ "storage_diffs": [{ "address": address, "storage_entries": [{ "key": "0x1", "value": "0x1" }, { "key": "0x1", "value": "0x2" }] }] }),
            json!({ "deployed_contracts": [{ "address": "0xdead", "class_hash": "0x1" }, { "address": "0xdead", "class_hash": "0x2" }] }),
            json!({ "replaced_classes": [{ "contract_address": address, "class_hash": "0x1" }, { "contract_address": address, "class_hash": "0x2" }] }),
            json!({ "declared_classes": [{ "class_hash": "0x1", "compiled_class_hash": "0x2" }, { "class_hash": "0x1", "compiled_class_hash": "0x3" }] }),
            json!({ "deprecated_declared_classes": ["0x1", "0x1"] }),
            json!({ "deployed_contracts": [{ "address": address, "class_hash": "0x1" }] }),
            json!({ "deployed_contracts": [{ "address": "0x0", "class_hash": "0x1" }] }),
            json!({ "deployed_contracts": [{ "address": "0xdead", "class_hash": "0x1" }], "replaced_classes": [{ "contract_address": "0xdead", "class_hash": "0x1" }] }),
            json!({ "declared_classes": [{ "class_hash": "0x1", "compiled_class_hash": "0x2" }], "deprecated_declared_classes": ["0x1"] }),
            json!({ "migrated_compiled_classes": [{ "class_hash": "0x1", "compiled_class_hash": "0x2" }] }),
        ] {
            assert!(matches!(apply(&mut starknet, diff), Err(Error::InvalidStateDiff(_))));
        }
        assert!(apply(&mut starknet, json!({})).unwrap().classes.is_empty());
        assert!(apply(&mut starknet, json!({ "migrated_compiled_classes": [] })).is_ok());
    }

    #[test]
    fn supplied_hashes_and_explicit_declaration_types_are_validated() {
        let mut starknet = setup();
        let legacy = ContractClass::Cairo0(cairo_0_account_without_validations());
        let hash = legacy.try_generate_hash().unwrap();
        let codegen: starknet_rs_core::types::ContractClass = legacy.try_into().unwrap();
        for fields in [json!({ "class_hash": "0x1" }), json!({ "compiled_class_hash": "0x1" })] {
            let mut supplied = fields;
            supplied["contract_class"] = serde_json::to_value(&codegen).unwrap();
            let request =
                serde_json::from_value(json!({ "state_diff": {}, "classes": [supplied] })).unwrap();
            assert!(matches!(starknet.apply_state_diff(request), Err(Error::InvalidStateDiff(_))));
            assert!(!starknet.pre_confirmed_state.is_contract_declared(hash));
        }
        let request = serde_json::from_value(json!({
            "classes": [{ "contract_class": codegen }],
            "state_diff": { "declared_classes": [{ "class_hash": hash, "compiled_class_hash": "0x1" }] }
        })).unwrap();
        assert!(matches!(starknet.apply_state_diff(request), Err(Error::InvalidStateDiff(_))));
        assert!(!starknet.pre_confirmed_state.is_contract_declared(hash));
        let duplicate = serde_json::from_value(json!({
            "state_diff": {}, "classes": [{ "contract_class": codegen }, { "contract_class": codegen }]
        })).unwrap();
        assert!(matches!(starknet.apply_state_diff(duplicate), Err(Error::InvalidStateDiff(_))));
        assert!(!starknet.pre_confirmed_state.is_contract_declared(hash));
    }
}
