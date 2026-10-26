use blockifier::state::state_api::StateReader;
use serde_json::{Value, json};
use starknet_api::state::StorageKey;
use starknet_rs_core::types::Felt;
use starknet_types::contract_address::ContractAddress;
use starknet_types::contract_class::deprecated::rpc_contract_class::DeprecatedContractClass;
use starknet_types::contract_class::{Cairo0ContractClass, ContractClass};
use starknet_types::num_bigint::BigUint;
use starknet_types::rpc::block::{BlockId, BlockTag};
use starknet_types::rpc::gas_modification::GasModificationRequest;
use starknet_types::traits::TryHashProducer;

use super::state::layer_json;
use super::{DumpLoadOptions, StateDump};
use crate::constants::STRK_ERC20_CONTRACT_ADDRESS;
use crate::error::Error;
use crate::messaging::ethereum::EthereumMessaging;
use crate::starknet::Starknet;
use crate::starknet::starknet_config::{BlockGenerationOn, StarknetConfig, StateArchiveCapacity};
use crate::state::{CustomState, CustomStateReader};
use crate::utils::exported_test_utils::dummy_cairo_0_contract_class;
use crate::utils::get_storage_var_address;
use crate::utils::test_utils::cairo_0_account_without_validations;

/// Dumps `starknet` through JSON and loads the result.
fn round_trip(starknet: &Starknet) -> Starknet {
    let value = serde_json::to_value(starknet.dump().unwrap()).unwrap();
    let dump = StateDump::from_value(value).unwrap();
    Starknet::from_dump(dump, &DumpLoadOptions::default()).unwrap()
}

fn dump_json(starknet: &Starknet) -> Value {
    serde_json::to_value(starknet.dump().unwrap()).unwrap()
}

/// Asserts that RPC-observable data of both instances is identical.
fn assert_observably_equal(original: &mut Starknet, restored: &mut Starknet) {
    assert_eq!(
        serde_json::to_value(&original.config).unwrap(),
        serde_json::to_value(&restored.config).unwrap()
    );

    let accounts = |starknet: &Starknet| -> Vec<Value> {
        starknet
            .get_predeployed_accounts()
            .iter()
            .map(|account| {
                json!([account.account_address, account.keys.private_key, account.class_hash])
            })
            .collect()
    };
    assert_eq!(accounts(original), accounts(restored));

    let mut block_ids = vec![BlockId::Tag(BlockTag::Latest), BlockId::Tag(BlockTag::PreConfirmed)];
    block_ids.extend(original.blocks.num_to_hash.keys().map(|number| BlockId::Number(number.0)));
    for block_id in &block_ids {
        assert_eq!(
            format!("{:?}", original.get_block_with_receipts(block_id).unwrap()),
            format!("{:?}", restored.get_block_with_receipts(block_id).unwrap()),
            "block {block_id:?}"
        );
        // State updates are built from hash maps, so their element order is not meaningful.
        assert_eq!(
            unordered(serde_json::to_value(original.block_state_update(block_id, None).unwrap())),
            unordered(serde_json::to_value(restored.block_state_update(block_id, None).unwrap())),
            "state update {block_id:?}"
        );
    }

    let transaction_hashes: Vec<Felt> =
        original.transactions.iter().map(|tx| *tx.inner.get_transaction_hash()).collect();
    for hash in transaction_hashes {
        assert_eq!(
            serde_json::to_value(original.get_transaction_receipt_by_hash(&hash).unwrap()).unwrap(),
            serde_json::to_value(restored.get_transaction_receipt_by_hash(&hash).unwrap()).unwrap()
        );
        assert_eq!(
            serde_json::to_value(original.get_transaction_trace_by_hash(hash).unwrap()).unwrap(),
            serde_json::to_value(restored.get_transaction_trace_by_hash(hash).unwrap()).unwrap()
        );
        assert_eq!(
            serde_json::to_value(
                original.get_transaction_execution_and_finality_status(hash).unwrap()
            )
            .unwrap(),
            serde_json::to_value(
                restored.get_transaction_execution_and_finality_status(hash).unwrap()
            )
            .unwrap()
        );
    }

    let class_hashes: Vec<Felt> =
        original.rpc_contract_classes.read().committed.keys().copied().collect();
    for class_hash in class_hashes {
        assert_eq!(
            serde_json::to_value(
                original.get_class(&BlockId::Tag(BlockTag::Latest), class_hash).unwrap()
            )
            .unwrap(),
            serde_json::to_value(
                restored.get_class(&BlockId::Tag(BlockTag::Latest), class_hash).unwrap()
            )
            .unwrap()
        );
    }

    // States rebuilt from diffs must equal the original states.
    assert_eq!(layer_json(&original.latest_state), layer_json(&restored.latest_state));
    assert_eq!(
        layer_json(&original.pre_confirmed_state),
        layer_json(&restored.pre_confirmed_state)
    );
    assert_eq!(original.blocks.hash_to_state.len(), restored.blocks.hash_to_state.len());
    for (block_hash, state) in &original.blocks.hash_to_state {
        assert_eq!(layer_json(state), layer_json(&restored.blocks.hash_to_state[block_hash]));
    }

    // Re-encoding must reproduce the dump, covering state that has no RPC view.
    assert_eq!(dump_json(original), dump_json(restored));
}

/// Sorts every array recursively, for comparing values whose element order is arbitrary.
fn unordered(value: serde_json::Result<Value>) -> Value {
    fn sort(value: Value) -> Value {
        match value {
            Value::Array(items) => {
                let mut items: Vec<Value> = items.into_iter().map(sort).collect();
                items.sort_by_key(|item| item.to_string());
                Value::Array(items)
            }
            Value::Object(entries) => {
                Value::Object(entries.into_iter().map(|(key, value)| (key, sort(value))).collect())
            }
            other => other,
        }
    }
    sort(value.unwrap())
}

fn mintable_address(starknet: &Starknet) -> ContractAddress {
    starknet.get_predeployed_accounts()[0].account_address
}

fn strk() -> ContractAddress {
    ContractAddress::new(STRK_ERC20_CONTRACT_ADDRESS).unwrap()
}

fn balance_key(address: ContractAddress) -> starknet_types::patricia_key::PatriciaKey {
    get_storage_var_address("ERC20_balances", &[Felt::from(address)]).unwrap()
}

fn gas_price_request(value: u128) -> GasModificationRequest {
    GasModificationRequest {
        gas_price_wei: None,
        data_gas_price_wei: None,
        gas_price_fri: std::num::NonZeroU128::new(value),
        data_gas_price_fri: None,
        l2_gas_price_wei: None,
        l2_gas_price_fri: None,
        generate_block: None,
    }
}

#[test]
fn round_trips_fresh_instance() {
    let mut original = Starknet::new(&StarknetConfig::default()).unwrap();
    let mut restored = round_trip(&original);
    assert_observably_equal(&mut original, &mut restored);
}

#[tokio::test(flavor = "multi_thread")]
async fn round_trips_history_pre_confirmed_state_and_controls() {
    let config = StarknetConfig {
        block_generation_on: BlockGenerationOn::Demand,
        state_archive: StateArchiveCapacity::Full,
        ..Default::default()
    };
    let mut original = Starknet::new(&config).unwrap();
    let address = mintable_address(&original);

    original.mint(address, BigUint::from(10_u32), strk()).await.unwrap();
    // Pass the stored block hash buffer, so creating a block pre-processes the next one.
    for _ in 0..blockifier::abi::constants::STORED_BLOCK_HASH_BUFFER {
        original.create_block();
    }
    original.mint(address, BigUint::from(20_u32), strk()).await.unwrap();
    original.set_time(1_000, true);
    // Leave a transaction in the pre-confirmed block.
    original.mint(address, BigUint::from(30_u32), strk()).await.unwrap();
    original.set_next_block_gas(gas_price_request(123)).unwrap();
    original.cheats.impersonate_account(address);
    original.cheats.set_auto_impersonate(true);
    original.messaging.last_local_block = 7;
    original.messaging.l2_to_l1_messages_hashes.insert(alloy::primitives::B256::repeat_byte(1), 2);

    // The latest state includes the next block's pre-processing write.
    assert_ne!(dump_json(&original)["latest"]["storage"], json!([]));

    let mut restored = round_trip(&original);
    assert_observably_equal(&mut original, &mut restored);
    assert!(restored.cheats.is_impersonated(&address));
    assert_eq!(restored.messaging.last_local_block, 7);

    // Historical storage reads match at every archived block.
    for number in original.blocks.num_to_hash.keys().map(|number| number.0).collect::<Vec<_>>() {
        let block_id = BlockId::Number(number);
        assert_eq!(
            original.contract_storage_at_block(&block_id, strk(), balance_key(address)).unwrap(),
            restored.contract_storage_at_block(&block_id, strk(), balance_key(address)).unwrap(),
        );
    }

    // The restored instance continues where the original left off.
    let latest_before = restored.get_latest_block().unwrap().block_number();
    restored.mint(address, BigUint::from(40_u32), strk()).await.unwrap();
    restored.create_block();
    let latest = restored.get_latest_block().unwrap();
    assert_eq!(latest.block_number().0, latest_before.0 + 1);
    assert_eq!(latest.get_transactions().len(), 2);
    assert_eq!(latest.header.block_header_without_hash.l1_gas_price.price_in_fri.0, 123);

    // Snapshot/revert and block abortion keep working.
    let checkpoint = restored.checkpoint();
    restored.create_block();
    restored.restore_checkpoint(checkpoint);
    assert_eq!(restored.get_latest_block().unwrap().block_number(), latest.block_number());
    restored.abort_blocks(BlockId::Number(latest.block_number().0)).unwrap();

    // An instance with aborted blocks round trips too; its latest state equals the archived one.
    assert_eq!(dump_json(&restored)["latest"]["storage"], json!([]));
    let mut restored_after_abort = round_trip(&restored);
    assert_observably_equal(&mut restored, &mut restored_after_abort);
}

#[test]
fn round_trips_state_mutation_and_cairo_0_classes() {
    let mut original = Starknet::new(&StarknetConfig {
        block_generation_on: BlockGenerationOn::Demand,
        ..Default::default()
    })
    .unwrap();
    let address = mintable_address(&original);
    original
        .apply_state_diff(
            serde_json::from_value(json!({ "state_diff": {
                "nonces": [{ "contract_address": address, "nonce": "0x5" }],
                "storage_diffs": [{ "address": address, "storage_entries": [{ "key": "0x55", "value": "0x7" }] }]
            }}))
            .unwrap(),
        )
        .unwrap();

    // Declare Cairo 0 classes in both representations Devnet stores them in.
    let rpc_class = DeprecatedContractClass::rpc_from_path(
        "../starknet-devnet-types/test_data/cairo_0_rpc.json",
    )
    .unwrap();
    let json_class = dummy_cairo_0_contract_class();
    for class in [Cairo0ContractClass::Rpc(rpc_class), json_class] {
        let class_hash = class.try_generate_hash().unwrap();
        original
            .pre_confirmed_state
            .declare_contract_class(class_hash, None, ContractClass::Cairo0(class))
            .unwrap();
    }
    original.commit_diff().unwrap();
    original.create_block();
    // Leave a state mutation in the pre-confirmed block.
    original
        .apply_state_diff(
            serde_json::from_value(json!({ "state_diff": {
                "storage_diffs": [{ "address": address, "storage_entries": [{ "key": "0x66", "value": "0x2" }] }]
            }}))
            .unwrap(),
        )
        .unwrap();

    let mut restored = round_trip(&original);
    assert_observably_equal(&mut original, &mut restored);
    let api_address = address.into();
    let key = StorageKey::try_from(Felt::from(0x66_u32)).unwrap();
    assert_eq!(restored.pre_confirmed_state.get_nonce_at(api_address).unwrap().0, Felt::from(5));
    assert_eq!(restored.pre_confirmed_state.get_storage_at(api_address, key).unwrap(), Felt::TWO);
}

#[test]
fn round_trips_l1_connection_without_contacting_it() {
    let mut original = Starknet::new(&StarknetConfig::default()).unwrap();
    original.messaging.ethereum = Some(
        EthereumMessaging::from_dump(
            serde_json::from_value(json!({
                "node_url": "http://127.0.0.1:1/",
                "messaging_contract_address": "0x5fbdb2315678afecb367f032d93f642f64180aa3",
                "last_fetched_block": 12,
                "chain_id": 31337,
                "deployer_private_key": "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"
            }))
            .unwrap(),
        )
        .unwrap(),
    );

    let restored = round_trip(&original);
    assert_eq!(dump_json(&original), dump_json(&restored));
    assert_eq!(dump_json(&restored)["messaging"]["ethereum"]["last_fetched_block"], 12);
}

#[test]
fn restored_instance_restarts_with_dumped_configuration() {
    let original =
        Starknet::new(&StarknetConfig { seed: 42, total_accounts: 3, ..Default::default() })
            .unwrap();
    let mut restored = round_trip(&original);
    restored.restart(false).unwrap();

    let addresses = |starknet: &Starknet| -> Vec<ContractAddress> {
        starknet.get_predeployed_accounts().iter().map(|account| account.account_address).collect()
    };
    assert_eq!(addresses(&restored), addresses(&original));
    for address in addresses(&restored) {
        assert!(restored.pre_confirmed_state.is_contract_deployed(address).unwrap());
    }
}

#[test]
fn load_options_replace_instance_settings() {
    let original = Starknet::new(&StarknetConfig::default()).unwrap();
    let dump = StateDump::from_value(dump_json(&original)).unwrap();
    let options = DumpLoadOptions { dump_path: Some("out.json".to_string()), ..Default::default() };
    let restored = Starknet::from_dump(dump, &options).unwrap();
    assert_eq!(restored.config.dump_path.as_deref(), Some("out.json"));

    let dump = StateDump::from_value(dump_json(&original)).unwrap();
    let options = DumpLoadOptions {
        fork_url: Some("http://127.0.0.1:1".parse().unwrap()),
        ..Default::default()
    };
    match Starknet::from_dump(dump, &options) {
        Err(Error::InvalidDump(msg)) => assert!(msg.contains("not forked"), "{msg}"),
        other => panic!("unexpected result: {:?}", other.map(|_| ())),
    }
}

#[test]
fn rejects_other_formats_and_versions() {
    let starknet = Starknet::new(&StarknetConfig::default()).unwrap();
    let valid = dump_json(&starknet);

    let mut wrong_format = valid.clone();
    wrong_format["format"] = json!("something-else");
    let mut newer_version = valid.clone();
    newer_version["version"] = json!(2);
    let mut string_version = valid.clone();
    string_version["version"] = json!("1");
    let mut unknown_field = valid.clone();
    unknown_field["unexpected"] = json!(true);
    let mut missing_section = valid.clone();
    missing_section.as_object_mut().unwrap().remove("blocks");

    for (input, expected) in [
        (json!([]), "expected a JSON object"),
        (json!({ "version": 1 }), "missing \"format\""),
        (wrong_format, "unsupported format \"something-else\""),
        (json!({ "format": "starknet-devnet-dump" }), "missing \"version\""),
        (newer_version, "unsupported version 2; supported: 1"),
        (string_version, "unsupported version \"1\""),
        (unknown_field, "unknown field `unexpected`"),
        (missing_section, "missing field `blocks`"),
    ] {
        match StateDump::from_value(input) {
            Err(Error::InvalidDump(msg)) => assert!(msg.contains(expected), "{msg}"),
            other => panic!("expected {expected:?}, got {:?}", other.map(|_| ())),
        }
    }
}

#[test]
fn fresh_instance_dump_stores_no_classes_and_is_small() {
    let starknet = Starknet::new(&StarknetConfig::default()).unwrap();
    let dump = dump_json(&starknet);
    assert_eq!(dump["classes"]["definitions"], json!([]));
    assert_eq!(dump["blocks"]["accepted"], json!([]));
    assert_eq!(dump["transactions"], json!([]));
    let size = dump.to_string().len();
    assert!(size < 8_000, "a fresh dump takes {size} bytes");
}

#[test]
fn round_trips_custom_account_class() {
    let class: ContractClass = cairo_0_account_without_validations().into();
    let class_hash = class.try_generate_hash().unwrap();
    let mut original = Starknet::new(&StarknetConfig {
        account_contract_class: class,
        account_contract_class_hash: class_hash,
        total_accounts: 2,
        ..Default::default()
    })
    .unwrap();

    let dump = dump_json(&original);
    let definitions = dump["classes"]["definitions"].as_array().unwrap();
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0]["class_hash"], json!(class_hash));

    let mut restored = round_trip(&original);
    assert_observably_equal(&mut original, &mut restored);
    assert_eq!(restored.config.account_contract_class_hash, class_hash);
    assert!(
        restored.get_predeployed_accounts().iter().all(|account| account.class_hash == class_hash)
    );
}

#[test]
fn rejects_dump_of_a_different_genesis() {
    let starknet = Starknet::new(&StarknetConfig::default()).unwrap();
    let mut dump = dump_json(&starknet);
    dump["genesis_fingerprint"] = json!("0x1");

    let dump = StateDump::from_value(dump).unwrap();
    match Starknet::from_dump(dump, &DumpLoadOptions::default()) {
        Err(Error::InvalidDump(msg)) => assert!(msg.contains("different genesis"), "{msg}"),
        other => panic!("unexpected result: {:?}", other.map(|_| ())),
    }
}
