use serde_json::{Value, json};
use starknet_rs_accounts::Account;
use starknet_rs_core::types::contract::legacy::LegacyContractClass;
use starknet_rs_core::types::{BlockId, BlockTag, Call, Felt, FunctionCall};
use starknet_rs_core::utils::{get_selector_from_name, get_storage_var_address};
use starknet_rs_providers::Provider;
use tokio_tungstenite::connect_async;

use crate::common::background_devnet::BackgroundDevnet;
use crate::common::constants::PREDEPLOYED_ACCOUNT_ADDRESS;
use crate::common::utils::{
    UniqueAutoDeletableFile, assert_no_notifications, get_events_contract_artifacts,
    get_simple_contract_artifacts, send_text_rpc_via_ws, subscribe_new_heads, subscribe_new_txs,
};

fn storage_diff(address: Felt, key: Felt, value: Felt) -> Value {
    json!({ "storage_diffs": [{ "address": address, "storage_entries": [{ "key": key, "value": value }] }] })
}

async fn apply(devnet: &BackgroundDevnet, diff: Value) -> Value {
    devnet.send_custom_rpc("devnet_applyStateDiff", json!({ "state_diff": diff })).await.unwrap()
}

async fn storage(devnet: &BackgroundDevnet, address: Felt, key: Felt, block_id: Value) -> Felt {
    let value = devnet
        .send_custom_rpc(
            "starknet_getStorageAt",
            json!({
                "contract_address": address, "key": key, "block_id": block_id
            }),
        )
        .await
        .unwrap();
    serde_json::from_value(value).unwrap()
}

async fn state_diff(devnet: &BackgroundDevnet, block_id: Value) -> Value {
    devnet
        .send_custom_rpc("starknet_getStateUpdate", json!({ "block_id": block_id }))
        .await
        .unwrap()["state_diff"]
        .clone()
}

async fn supply_simple_contract(devnet: &BackgroundDevnet, address: Felt) -> Felt {
    let (class, compiled_hash) = get_simple_contract_artifacts();
    let hash = class.class_hash();
    let result = devnet.send_custom_rpc("devnet_applyStateDiff", json!({
        "classes": [{ "contract_class": class, "class_hash": hash, "compiled_class_hash": compiled_hash }],
        "state_diff": {
            "declared_classes": [{ "class_hash": hash, "compiled_class_hash": compiled_hash }],
            "deployed_contracts": [{ "address": address, "class_hash": hash }],
            "nonces": [{ "contract_address": address, "nonce": "0x0" }],
            "storage_diffs": [{ "address": address, "storage_entries": [{ "key": "0x55", "value": "0x1" }] }]
        }
    })).await.unwrap();
    assert_eq!(
        result,
        json!({ "classes": [{ "class_hash": hash, "compiled_class_hash": compiled_hash }] })
    );
    hash
}

#[tokio::test]
async fn visibility_and_transaction_free_sealing_in_all_modes() {
    let address = Felt::from_hex_unchecked(PREDEPLOYED_ACCOUNT_ADDRESS);
    let key = Felt::from(0x55);
    for mode in ["transaction", "demand", "60"] {
        let devnet = BackgroundDevnet::spawn_with_additional_args(&[
            "--block-generation-on",
            mode,
            "--state-archive-capacity",
            "full",
        ])
        .await
        .unwrap();
        let original = devnet.get_latest_block_with_tx_hashes().await.unwrap();
        assert_eq!(apply(&devnet, json!({})).await, json!({ "classes": [] }));
        for value in [Felt::ONE, Felt::TWO, Felt::ZERO, Felt::from(3)] {
            apply(&devnet, storage_diff(address, key, value)).await;
            assert_eq!(storage(&devnet, address, key, json!("pre_confirmed")).await, value);
            assert_eq!(storage(&devnet, address, key, json!("latest")).await, Felt::ZERO);
        }
        assert_eq!(
            devnet.get_latest_block_with_tx_hashes().await.unwrap().block_hash,
            original.block_hash
        );
        assert_eq!(
            state_diff(&devnet, json!("pre_confirmed")).await["storage_diffs"],
            storage_diff(address, key, Felt::from(3))["storage_diffs"]
        );
        assert!(
            devnet.get_pre_confirmed_block_with_tx_hashes().await.unwrap().transactions.is_empty()
        );
        let hash = devnet.create_block().await.unwrap();
        assert_eq!(storage(&devnet, address, key, json!("latest")).await, Felt::from(3));
        assert_eq!(
            storage(&devnet, address, key, json!({ "block_hash": original.block_hash })).await,
            Felt::ZERO
        );
        let diff = state_diff(&devnet, json!({ "block_hash": hash })).await;
        assert_eq!(
            diff["storage_diffs"],
            storage_diff(address, key, Felt::from(3))["storage_diffs"]
        );
        assert!(devnet.get_latest_block_with_tx_hashes().await.unwrap().transactions.is_empty());
        devnet.create_block().await.unwrap();
        assert_eq!(state_diff(&devnet, json!("latest")).await["storage_diffs"], json!([]));
    }
}

#[tokio::test]
async fn interval_timer_seals_manual_changes() {
    let devnet = BackgroundDevnet::spawn_with_additional_args(&["--block-generation-on", "1"])
        .await
        .unwrap();
    let address = Felt::from_hex_unchecked(PREDEPLOYED_ACCOUNT_ADDRESS);
    let key = Felt::from(0x55);
    apply(&devnet, storage_diff(address, key, Felt::from(9))).await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if storage(&devnet, address, key, json!("latest")).await == Felt::from(9) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    assert!(devnet.get_latest_block_with_tx_hashes().await.unwrap().transactions.is_empty());
}

#[tokio::test]
async fn transactions_calls_and_estimation_observe_mutations_without_trace_leaks() {
    for mode in ["transaction", "demand", "60"] {
        let devnet = BackgroundDevnet::spawn_with_additional_args(&["--block-generation-on", mode])
            .await
            .unwrap();
        let address = Felt::from(0xdead);
        supply_simple_contract(&devnet, address).await;
        let key = get_storage_var_address("balance", &[]).unwrap();
        let unrelated_key = Felt::from(0x55);
        let mut account = devnet.get_first_predeployed_account().await;
        account.set_block_id(BlockId::Tag(BlockTag::PreConfirmed));
        apply(
            &devnet,
            json!({
                "nonces": [{ "contract_address": account.address(), "nonce": "0xa" }],
                "storage_diffs": [{ "address": address, "storage_entries": [
                    { "key": key, "value": "0xa" }, { "key": unrelated_key, "value": "0x99" }
                ] }]
            }),
        )
        .await;
        let call = FunctionCall {
            contract_address: address,
            entry_point_selector: get_selector_from_name("get_balance").unwrap(),
            calldata: vec![],
        };
        assert_eq!(
            devnet.json_rpc_client.call(call, BlockId::Tag(BlockTag::PreConfirmed)).await.unwrap(),
            vec![Felt::from(10)]
        );
        let increase = Call {
            to: address,
            selector: get_selector_from_name("increase_balance").unwrap(),
            calldata: vec![Felt::from(10), Felt::ZERO],
        };
        account
            .execute_v3(vec![increase.clone()])
            .nonce(Felt::from(10))
            .estimate_fee()
            .await
            .unwrap();
        let first =
            account.execute_v3(vec![increase.clone()]).nonce(Felt::from(10)).send().await.unwrap();
        assert_eq!(storage(&devnet, address, key, json!("pre_confirmed")).await, Felt::from(20));
        let receipt_before = devnet
            .send_custom_rpc(
                "starknet_getTransactionReceipt",
                json!({ "transaction_hash": first.transaction_hash }),
            )
            .await
            .unwrap();
        let trace_before = devnet
            .send_custom_rpc(
                "starknet_traceTransaction",
                json!({ "transaction_hash": first.transaction_hash }),
            )
            .await
            .unwrap();
        assert_eq!(trace_before["state_diff"]["declared_classes"], json!([]));
        assert_eq!(trace_before["state_diff"]["deployed_contracts"], json!([]));
        let entries = trace_before["state_diff"]["storage_diffs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["address"] == json!(address))
            .unwrap()["storage_entries"]
            .clone();
        assert_eq!(entries, json!([{ "key": key, "value": Felt::from(20) }]));
        apply(&devnet, storage_diff(address, key, Felt::from(30))).await;
        assert_eq!(
            devnet
                .send_custom_rpc(
                    "starknet_getTransactionReceipt",
                    json!({ "transaction_hash": first.transaction_hash })
                )
                .await
                .unwrap(),
            receipt_before
        );
        assert_eq!(
            devnet
                .send_custom_rpc(
                    "starknet_traceTransaction",
                    json!({ "transaction_hash": first.transaction_hash })
                )
                .await
                .unwrap(),
            trace_before
        );
        let second = account.execute_v3(vec![increase]).nonce(Felt::from(11)).send().await.unwrap();
        assert_eq!(storage(&devnet, address, key, json!("pre_confirmed")).await, Felt::from(40));
        if mode != "transaction" {
            assert_eq!(
                devnet.get_pre_confirmed_block_with_tx_hashes().await.unwrap().transactions,
                vec![first.transaction_hash, second.transaction_hash]
            );
            devnet.create_block().await.unwrap();
        }
        assert_eq!(storage(&devnet, address, key, json!("latest")).await, Felt::from(40));
        let diff = state_diff(&devnet, json!("latest")).await;
        let updates = diff["storage_diffs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["address"] == json!(address))
            .unwrap()["storage_entries"]
            .as_array()
            .unwrap();
        assert_eq!(
            updates.iter().filter(|item| item["key"] == json!(key)).collect::<Vec<_>>(),
            vec![&json!({ "key": key, "value": Felt::from(40) })]
        );
    }
}

#[tokio::test]
async fn declarations_deployments_replacements_and_historical_classes() {
    let devnet = BackgroundDevnet::spawn_with_additional_args(&[
        "--block-generation-on",
        "demand",
        "--state-archive-capacity",
        "full",
    ])
    .await
    .unwrap();
    let address = Felt::from(0xdead);
    let sierra_hash = supply_simple_contract(&devnet, address).await;
    let class: LegacyContractClass = serde_json::from_str(
        &std::fs::read_to_string(
            "../../contracts/test_artifacts/cairo0/account_without_validations/account.json",
        )
        .unwrap(),
    )
    .unwrap();
    let legacy_hash = class.class_hash().unwrap();
    let compressed = class.compress().unwrap();
    let response = devnet
        .send_custom_rpc(
            "devnet_applyStateDiff",
            json!({
                "classes": [{ "contract_class": compressed }],
                "state_diff": {
                    "deprecated_declared_classes": [legacy_hash],
                    "replaced_classes": [{ "contract_address": address, "class_hash": legacy_hash }]
                }
            }),
        )
        .await
        .unwrap();
    assert_eq!(response, json!({ "classes": [{ "class_hash": legacy_hash }] }));
    let diff = state_diff(&devnet, json!("pre_confirmed")).await;
    assert_eq!(
        diff["deployed_contracts"],
        json!([{ "address": address, "class_hash": legacy_hash }])
    );
    assert_eq!(diff["replaced_classes"], json!([]));
    assert_eq!(
        devnet
            .send_custom_rpc(
                "starknet_getClass",
                json!({ "block_id": "latest", "class_hash": legacy_hash })
            )
            .await
            .unwrap_err()
            .code,
        28
    );
    let declared_block = devnet.create_block().await.unwrap();
    for hash in [sierra_hash, legacy_hash, sierra_hash] {
        apply(
            &devnet,
            json!({ "replaced_classes": [{ "contract_address": address, "class_hash": hash }] }),
        )
        .await;
    }
    let diff = state_diff(&devnet, json!("pre_confirmed")).await;
    assert_eq!(
        diff["replaced_classes"],
        json!([{ "contract_address": address, "class_hash": sierra_hash }])
    );
    assert_eq!(diff["deployed_contracts"], json!([]));
    devnet
        .send_custom_rpc(
            "devnet_applyStateDiff",
            json!({ "state_diff": {}, "classes": [{ "contract_class": compressed }] }),
        )
        .await
        .unwrap();
    devnet.create_block().await.unwrap();
    assert!(
        devnet
            .send_custom_rpc(
                "starknet_getClass",
                json!({ "block_id": { "block_hash": declared_block }, "class_hash": legacy_hash })
            )
            .await
            .is_ok()
    );
    assert_eq!(
        state_diff(&devnet, json!("latest")).await["deprecated_declared_classes"],
        json!([])
    );
    devnet.abort_blocks(&BlockId::Hash(declared_block)).await.unwrap();
    assert_eq!(
        devnet
            .send_custom_rpc(
                "starknet_getClass",
                json!({ "block_id": "pre_confirmed", "class_hash": legacy_hash })
            )
            .await
            .unwrap_err()
            .code,
        28
    );
}

#[tokio::test]
async fn atomic_errors_leave_state_classes_and_journal_unchanged() {
    let dump = UniqueAutoDeletableFile::new("apply_state_diff_atomic");
    let devnet = BackgroundDevnet::spawn_with_additional_args(&[
        "--dump-on",
        "block",
        "--dump-path",
        &dump.path,
        "--block-generation-on",
        "demand",
    ])
    .await
    .unwrap();
    let address = Felt::from_hex_unchecked(PREDEPLOYED_ACCOUNT_ADDRESS);
    let (class, compiled_hash) = get_simple_contract_artifacts();
    let class_hash = class.class_hash();
    apply(&devnet, storage_diff(address, Felt::from(0x55), Felt::ONE)).await;
    let before = std::fs::read(&dump.path).unwrap();
    let before_diff = state_diff(&devnet, json!("pre_confirmed")).await;
    let invalid_class = devnet
        .send_custom_rpc(
            "devnet_applyStateDiff",
            json!({
                "state_diff": {},
                "classes": [{ "contract_class": {
                    "program": "eA==", "abi": [],
                    "entry_points_by_type": { "CONSTRUCTOR": [], "EXTERNAL": [], "L1_HANDLER": [] }
                } }]
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(invalid_class.code, 50);
    assert_eq!(std::fs::read(&dump.path).unwrap(), before);
    for invalid in [
        json!({ "classes": [{ "contract_class": class }], "state_diff": {
            "storage_diffs": [{ "address": address, "storage_entries": [{ "key": "0x55", "value": "0x2" }] }],
            "nonces": [{ "contract_address": address, "nonce": "0xa" }, { "contract_address": "0xdead", "nonce": "0x1" }]
        } }),
        json!({ "state_diff": { "replaced_classes": [{ "contract_address": address, "class_hash": "0xdead" }] } }),
        json!({ "classes": [{ "contract_class": class, "class_hash": "0x1" }], "state_diff": {} }),
        json!({ "classes": [{ "contract_class": class, "compiled_class_hash": "0x1" }], "state_diff": {} }),
        json!({ "classes": [{ "contract_class": class }], "state_diff": { "declared_classes": [{ "class_hash": class_hash, "compiled_class_hash": compiled_hash + Felt::ONE }] } }),
    ] {
        assert!(devnet.send_custom_rpc("devnet_applyStateDiff", invalid).await.is_err());
        assert_eq!(std::fs::read(&dump.path).unwrap(), before);
        assert_eq!(state_diff(&devnet, json!("pre_confirmed")).await, before_diff);
        assert_eq!(
            devnet
                .json_rpc_client
                .get_nonce(BlockId::Tag(BlockTag::PreConfirmed), address)
                .await
                .unwrap(),
            Felt::ZERO
        );
        assert_eq!(
            devnet
                .send_custom_rpc(
                    "starknet_getClass",
                    json!({ "block_id": "pre_confirmed", "class_hash": class_hash })
                )
                .await
                .unwrap_err()
                .code,
            28
        );
    }
}

#[tokio::test]
async fn snapshots_replay_restart_and_abort_restore_mutations() {
    let dump = UniqueAutoDeletableFile::new("apply_state_diff_replay");
    let args = [
        "--dump-on",
        "block",
        "--dump-path",
        &dump.path,
        "--block-generation-on",
        "demand",
        "--state-archive-capacity",
        "full",
    ];
    let devnet = BackgroundDevnet::spawn_with_additional_args(&args).await.unwrap();
    let address = Felt::from_hex_unchecked(PREDEPLOYED_ACCOUNT_ADDRESS);
    let key = Felt::from(0x55);
    apply(&devnet, storage_diff(address, key, Felt::ONE)).await;
    let retained_tx = devnet.mint(Felt::from(0xabcd), 1).await;
    let snapshot = devnet.send_custom_rpc("devnet_snapshot", json!([])).await.unwrap();
    let contract = Felt::from(0xdead);
    let hash = supply_simple_contract(&devnet, contract).await;
    apply(&devnet, storage_diff(address, key, Felt::TWO)).await;
    devnet.send_custom_rpc("devnet_revert", json!({ "snapshot_id": snapshot })).await.unwrap();
    assert_eq!(storage(&devnet, address, key, json!("pre_confirmed")).await, Felt::ONE);
    assert_eq!(
        devnet.get_pre_confirmed_block_with_tx_hashes().await.unwrap().transactions,
        vec![retained_tx]
    );
    assert_eq!(
        devnet
            .send_custom_rpc(
                "starknet_getClass",
                json!({ "block_id": "pre_confirmed", "class_hash": hash })
            )
            .await
            .unwrap_err()
            .code,
        28
    );
    supply_simple_contract(&devnet, contract).await;
    let block = devnet.create_block().await.unwrap();
    apply(&devnet, storage_diff(address, key, Felt::from(3))).await;
    let replay = BackgroundDevnet::spawn_with_additional_args(&[
        "--block-generation-on",
        "demand",
        "--state-archive-capacity",
        "full",
    ])
    .await
    .unwrap();
    replay.send_custom_rpc("devnet_load", json!({ "path": dump.path })).await.unwrap();
    assert_eq!(storage(&replay, address, key, json!("latest")).await, Felt::ONE);
    assert_eq!(storage(&replay, address, key, json!("pre_confirmed")).await, Felt::from(3));
    assert_eq!(
        replay.get_latest_block_with_tx_hashes().await.unwrap().transactions,
        vec![retained_tx]
    );
    assert!(
        replay
            .send_custom_rpc(
                "starknet_getClass",
                json!({ "block_id": "latest", "class_hash": hash })
            )
            .await
            .is_ok()
    );
    assert_eq!(
        replay
            .json_rpc_client
            .get_class_hash_at(BlockId::Tag(BlockTag::Latest), contract)
            .await
            .unwrap(),
        hash
    );
    let (unsealed_class, _) = get_events_contract_artifacts();
    let unsealed_hash = unsealed_class.class_hash();
    devnet
        .send_custom_rpc(
            "devnet_applyStateDiff",
            json!({ "state_diff": {}, "classes": [{ "contract_class": unsealed_class }] }),
        )
        .await
        .unwrap();
    devnet.abort_blocks(&BlockId::Hash(block)).await.unwrap();
    assert_eq!(storage(&devnet, address, key, json!("pre_confirmed")).await, Felt::ZERO);
    assert_eq!(
        devnet
            .send_custom_rpc(
                "starknet_getClass",
                json!({ "block_id": "pre_confirmed", "class_hash": hash })
            )
            .await
            .unwrap_err()
            .code,
        28
    );
    apply(&devnet, storage_diff(address, key, Felt::from(4))).await;
    assert_eq!(
        devnet
            .send_custom_rpc(
                "starknet_getClass",
                json!({ "block_id": "pre_confirmed", "class_hash": unsealed_hash })
            )
            .await
            .unwrap_err()
            .code,
        28
    );
    supply_simple_contract(&devnet, contract).await;
    devnet.restart().await;
    assert_eq!(storage(&devnet, address, key, json!("pre_confirmed")).await, Felt::ZERO);
    assert_eq!(
        devnet
            .send_custom_rpc(
                "starknet_getClass",
                json!({ "block_id": "pre_confirmed", "class_hash": hash })
            )
            .await
            .unwrap_err()
            .code,
        28
    );
}

#[tokio::test]
async fn websocket_and_restrictive_mode_without_notifications() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    let (mut ws, _) = connect_async(devnet.ws_url()).await.unwrap();
    subscribe_new_txs(&mut ws, json!({ "finality_status": ["PRE_CONFIRMED", "ACCEPTED_ON_L2"] }))
        .await
        .unwrap();
    subscribe_new_heads(&mut ws, json!({})).await.unwrap();
    let address = Felt::from_hex_unchecked(PREDEPLOYED_ACCOUNT_ADDRESS);
    let params = json!({ "state_diff": storage_diff(address, Felt::from(0x55), Felt::ONE) });
    let response =
        send_text_rpc_via_ws(&mut ws, "devnet_applyStateDiff", params.clone()).await.unwrap();
    assert_eq!(response["result"], json!({ "classes": [] }));
    assert_no_notifications(&mut ws).await.unwrap();
    let restricted =
        BackgroundDevnet::spawn_with_additional_args(&["--restrictive-mode"]).await.unwrap();
    assert_eq!(
        restricted.send_custom_rpc("devnet_applyStateDiff", params.clone()).await.unwrap_err().code,
        -32604
    );
    let (mut ws, _) = connect_async(restricted.ws_url()).await.unwrap();
    assert_eq!(
        send_text_rpc_via_ws(&mut ws, "devnet_applyStateDiff", params).await.unwrap()["error"]
            ["code"],
        -32604
    );
}

#[tokio::test]
async fn fork_origin_classes_and_contracts_support_local_overrides() {
    let origin = BackgroundDevnet::spawn_forkable_devnet().await.unwrap();
    let address = Felt::from(0xdead);
    let hash = supply_simple_contract(&origin, address).await;
    let key = get_storage_var_address("balance", &[]).unwrap();
    apply(&origin, storage_diff(address, key, Felt::ONE)).await;
    origin.create_block().await.unwrap();
    let fork = origin.fork().await.unwrap();
    apply(&fork, storage_diff(address, key, Felt::TWO)).await;
    assert_eq!(storage(&fork, address, key, json!("pre_confirmed")).await, Felt::TWO);
    assert_eq!(storage(&fork, address, key, json!("latest")).await, Felt::ONE);
    assert_eq!(storage(&origin, address, key, json!("latest")).await, Felt::ONE);
    let deployed = Felt::from(0xbeef);
    let (_, compiled_hash) = get_simple_contract_artifacts();
    apply(
        &fork,
        json!({
            "declared_classes": [{ "class_hash": hash, "compiled_class_hash": compiled_hash }],
            "deployed_contracts": [{ "address": deployed, "class_hash": hash }]
        }),
    )
    .await;
    let call = FunctionCall {
        contract_address: deployed,
        entry_point_selector: get_selector_from_name("get_balance").unwrap(),
        calldata: vec![],
    };
    assert_eq!(
        fork.json_rpc_client.call(call, BlockId::Tag(BlockTag::PreConfirmed)).await.unwrap(),
        vec![Felt::ZERO]
    );
    let before = state_diff(&fork, json!("pre_confirmed")).await;
    assert!(fork.send_custom_rpc("devnet_applyStateDiff", json!({ "state_diff": {
        "storage_diffs": [{ "address": address, "storage_entries": [{ "key": key, "value": "0x99" }] }],
        "nonces": [{ "contract_address": "0x123456", "nonce": "0x1" }]
    } })).await.is_err());
    assert_eq!(state_diff(&fork, json!("pre_confirmed")).await, before);
    assert_eq!(storage(&fork, address, key, json!("pre_confirmed")).await, Felt::TWO);
}
