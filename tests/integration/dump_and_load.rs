use std::path::Path;
use std::sync::Arc;
use std::time;

use serde_json::json;
use starknet_rs_accounts::Account;
use starknet_rs_core::types::{DeclareTransaction, Felt, InvokeTransaction, Transaction};
use starknet_rs_providers::Provider;

use crate::assert_eq_prop;
use crate::common::background_devnet::BackgroundDevnet;
use crate::common::utils::{
    FeeUnit, UniqueAutoDeletableFile, get_events_contract_artifacts, new_contract_factory,
    send_ctrl_c_signal_and_wait,
};

static DUMMY_ADDRESS: u128 = 1;
static DUMMY_AMOUNT: u128 = 1;

#[tokio::test]
async fn dump_load_dump_load_on_exit() -> Result<(), anyhow::Error> {
    let dump_file = UniqueAutoDeletableFile::new("dump_load_dump_load_on_exit");

    // Each run loads the previous dump on startup and dumps again on exit.
    for _ in 0..2 {
        let devnet_dump = BackgroundDevnet::spawn_with_additional_args(&[
            "--dump-path",
            &dump_file.path,
            "--dump-on",
            "exit",
        ])
        .await?;

        devnet_dump.create_block().await.unwrap();
        devnet_dump.mint(DUMMY_ADDRESS, DUMMY_AMOUNT).await;

        send_ctrl_c_signal_and_wait(&devnet_dump.process).await;
    }

    let devnet_load =
        BackgroundDevnet::spawn_with_additional_args(&["--dump-path", &dump_file.path]).await?;

    let last_block = devnet_load.get_latest_block_with_tx_hashes().await?;
    assert_eq_prop!(last_block.block_number, 4)
}

#[tokio::test]
async fn dump_load_via_rpc() {
    let devnet_dump = BackgroundDevnet::spawn().await.expect("Could not start Devnet");
    for _ in 0..2 {
        devnet_dump.create_block().await.unwrap();
        devnet_dump.mint(DUMMY_ADDRESS, DUMMY_AMOUNT).await;
    }
    let dump = devnet_dump.send_custom_rpc("devnet_dump", json!({})).await.unwrap();
    assert_eq!(dump["format"], "starknet-devnet-dump");

    let devnet_load = BackgroundDevnet::spawn().await.expect("Could not start Devnet");
    let response = devnet_load.send_custom_rpc("devnet_load", json!({ "dump": dump })).await;
    assert_eq!(response.unwrap(), json!({}));

    let last_block = devnet_load.get_latest_block_with_tx_hashes().await.unwrap();
    assert_eq!(last_block.block_number, 4);
    assert_eq!(
        last_block,
        devnet_dump.get_latest_block_with_tx_hashes().await.unwrap(),
        "loading restores blocks exactly, including hashes and timestamps"
    );

    let loaded_balance =
        devnet_load.get_balance_latest(&Felt::from(DUMMY_ADDRESS), FeeUnit::Fri).await.unwrap();
    assert_eq!(loaded_balance, Felt::from(DUMMY_AMOUNT * 2));
}

#[tokio::test]
async fn removed_dump_modes_fail_on_startup() {
    for mode in ["block", "request"] {
        let devnet = BackgroundDevnet::spawn_with_additional_args(&[
            "--dump-path",
            "dump",
            "--dump-on",
            mode,
        ])
        .await;
        assert!(devnet.is_err(), "--dump-on {mode} should be rejected");
    }
}

#[tokio::test]
async fn dump_and_load_blocks_generation_on_demand() {
    let dump_file = UniqueAutoDeletableFile::new("dump_load_on_demand");

    let total_iterations = 2;
    for _ in 0..total_iterations {
        let devnet_dump = BackgroundDevnet::spawn_with_additional_args(&[
            "--dump-path",
            &dump_file.path,
            "--dump-on",
            "exit",
            "--block-generation-on",
            "demand",
        ])
        .await
        .expect("Could not start Devnet");

        devnet_dump.mint(DUMMY_ADDRESS, DUMMY_AMOUNT).await;
        devnet_dump.mint(DUMMY_ADDRESS, DUMMY_AMOUNT).await;
        devnet_dump.create_block().await.unwrap();

        send_ctrl_c_signal_and_wait(&devnet_dump.process).await;
    }

    // The block generation mode is taken from the dump.
    let devnet_load =
        BackgroundDevnet::spawn_with_additional_args(&["--dump-path", &dump_file.path])
            .await
            .expect("Could not start Devnet");

    let last_block = devnet_load.get_latest_block_with_tx_hashes().await.unwrap();
    assert_eq!(last_block.block_number, total_iterations);
    assert_eq!(last_block.transactions.len(), total_iterations as usize);
    assert_eq!(devnet_load.get_config().await["block_generation_on"], "demand");
}

#[tokio::test]
async fn mint_dump_on_exit_and_load() {
    let dump_file = UniqueAutoDeletableFile::new("dump_on_exit");
    let devnet_dump = BackgroundDevnet::spawn_with_additional_args(&[
        "--dump-path",
        dump_file.path.as_str(),
        "--dump-on",
        "exit",
    ])
    .await
    .expect("Could not start Devnet");
    let devnet_dump_pid = devnet_dump.process.id();
    let mint_tx_hash = devnet_dump.mint(DUMMY_ADDRESS, DUMMY_AMOUNT).await;

    send_ctrl_c_signal_and_wait(&devnet_dump.process).await;

    let devnet_load =
        BackgroundDevnet::spawn_with_additional_args(&["--load-path", &dump_file.path])
            .await
            .expect("Could not start Devnet");
    let devnet_load_pid = devnet_load.process.id();
    assert_ne!(devnet_dump_pid, devnet_load_pid); // if PID's are different SIGINT signal worked
    let loaded_transaction =
        devnet_load.json_rpc_client.get_transaction_by_hash(mint_tx_hash, None).await.unwrap();
    if let Transaction::Invoke(InvokeTransaction::V3(tx)) = loaded_transaction {
        assert_eq!(tx.transaction_hash, mint_tx_hash);
    } else {
        panic!("Could not unpack the transaction from {loaded_transaction:?}");
    }
}

#[tokio::test]
async fn declare_deploy() {
    let devnet = BackgroundDevnet::spawn().await.expect("Could not start Devnet");

    let predeployed_account = devnet.get_first_predeployed_account_owned().await;

    let (cairo_1_contract, casm_class_hash) = get_events_contract_artifacts();

    // declare the contract
    let declaration_result = predeployed_account
        .declare_v3(Arc::new(cairo_1_contract), casm_class_hash)
        .l1_gas(0)
        .l1_data_gas(1000)
        .l2_gas(1e8 as u64)
        .send()
        .await
        .unwrap();

    let predeployed_account = Arc::new(predeployed_account);

    // deploy the contract
    let contract_factory =
        new_contract_factory(declaration_result.class_hash, predeployed_account.clone());
    let deploy_result = contract_factory
        .deploy_v3(vec![], Felt::ZERO, false)
        .l1_gas(0)
        .l1_data_gas(1000)
        .l2_gas(5e7 as u64)
        .send()
        .await
        .unwrap();

    let dump = devnet.send_custom_rpc("devnet_dump", json!({})).await.unwrap();
    let devnet_load = BackgroundDevnet::spawn().await.expect("Could not start Devnet");
    devnet_load.send_custom_rpc("devnet_load", json!({ "dump": dump })).await.unwrap();

    // check declare transaction
    let loaded_declare = devnet_load
        .json_rpc_client
        .get_transaction_by_hash(declaration_result.transaction_hash, None)
        .await
        .unwrap();
    if let Transaction::Declare(DeclareTransaction::V3(tx)) = loaded_declare {
        assert_eq!(tx.transaction_hash, declaration_result.transaction_hash);
    } else {
        panic!("Could not unpack the transaction from {loaded_declare:?}");
    }

    // check deploy transaction
    let loaded_deploy = devnet_load
        .json_rpc_client
        .get_transaction_by_hash(deploy_result.transaction_hash, None)
        .await
        .unwrap();
    if let Transaction::Invoke(InvokeTransaction::V3(tx)) = loaded_deploy {
        assert_eq!(tx.transaction_hash, deploy_result.transaction_hash);
    } else {
        panic!("Could not unpack the transaction from {loaded_deploy:?}");
    }

    // the declared class is restored too
    let class_hash = declaration_result.class_hash;
    let block_id = starknet_rs_core::types::BlockId::Tag(starknet_rs_core::types::BlockTag::Latest);
    assert_eq!(
        devnet_load.json_rpc_client.get_class(block_id, class_hash).await.unwrap(),
        devnet.json_rpc_client.get_class(block_id, class_hash).await.unwrap()
    );
}

#[tokio::test]
async fn dump_on_exit_without_transactions_loads_genesis() {
    let dump_file = UniqueAutoDeletableFile::new("dump_without_transaction");
    let devnet_dump = BackgroundDevnet::spawn_with_additional_args(&[
        "--dump-path",
        &dump_file.path,
        "--dump-on",
        "exit",
    ])
    .await
    .expect("Could not start Devnet");
    let genesis = devnet_dump.get_latest_block_with_tx_hashes().await.unwrap();

    send_ctrl_c_signal_and_wait(&devnet_dump.process).await;
    assert!(Path::new(&dump_file.path).exists());

    let devnet_load =
        BackgroundDevnet::spawn_with_additional_args(&["--load-path", &dump_file.path])
            .await
            .expect("Could not start Devnet");
    assert_eq!(devnet_load.get_latest_block_with_tx_hashes().await.unwrap(), genesis);
}

#[tokio::test]
async fn failed_exit_dump_fails_the_process() {
    let mut devnet = BackgroundDevnet::spawn_with_additional_args(&[
        "--dump-path",
        "/nonexistent-devnet-directory/dump.json",
        "--dump-on",
        "exit",
    ])
    .await
    .expect("Could not start Devnet");

    send_ctrl_c_signal_and_wait(&devnet.process).await;
    let status = devnet.process.process.try_wait().unwrap().expect("Devnet did not exit");
    assert!(!status.success());
}

#[tokio::test]
async fn startup_load_rejects_conflicting_options() {
    let dump_file = UniqueAutoDeletableFile::new("dump_startup_options");
    let devnet_dump = BackgroundDevnet::spawn_with_additional_args(&[
        "--dump-path",
        &dump_file.path,
        "--dump-on",
        "exit",
        "--seed",
        "1",
        "--accounts",
        "3",
    ])
    .await
    .expect("Could not start Devnet");
    send_ctrl_c_signal_and_wait(&devnet_dump.process).await;

    // The dumped configuration applies; matching options are accepted.
    let devnet_load = BackgroundDevnet::spawn_with_additional_args(&[
        "--load-path",
        &dump_file.path,
        "--seed",
        "1",
    ])
    .await
    .expect("Could not start Devnet");
    let accounts = devnet_load.send_custom_rpc("devnet_getPredeployedAccounts", json!({})).await;
    assert_eq!(accounts.unwrap().as_array().unwrap().len(), 3);

    for conflicting in [["--seed", "2"], ["--accounts", "4"]] {
        let args = [&["--load-path", dump_file.path.as_str()], conflicting.as_slice()].concat();
        assert!(BackgroundDevnet::spawn_with_additional_args(&args).await.is_err());
    }
    let missing = UniqueAutoDeletableFile::new("dump_startup_missing");
    assert!(
        BackgroundDevnet::spawn_with_additional_args(&["--load-path", &missing.path])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn legacy_dumps_load_via_rpc_and_on_startup() {
    let events = json!([
        { "jsonrpc": "2.0", "id": 1, "method": "devnet_createBlock" },
        {
            "jsonrpc": "2.0", "id": 2, "method": "devnet_mint",
            "params": { "address": format!("{DUMMY_ADDRESS:#x}"), "amount": DUMMY_AMOUNT, "unit": "FRI" }
        },
    ]);

    let devnet = BackgroundDevnet::spawn().await.unwrap();
    devnet.send_custom_rpc("devnet_load", json!({ "dump": events })).await.unwrap();
    assert_eq!(devnet.get_latest_block_with_tx_hashes().await.unwrap().block_number, 2);

    let dump_file = UniqueAutoDeletableFile::new("legacy_dump");
    std::fs::write(&dump_file.path, events.to_string()).unwrap();
    let devnet_load =
        BackgroundDevnet::spawn_with_additional_args(&["--load-path", &dump_file.path])
            .await
            .unwrap();
    assert_eq!(devnet_load.get_latest_block_with_tx_hashes().await.unwrap().block_number, 2);
    let balance =
        devnet_load.get_balance_latest(&Felt::from(DUMMY_ADDRESS), FeeUnit::Fri).await.unwrap();
    assert_eq!(balance, Felt::from(DUMMY_AMOUNT));

    // Dumps produced after loading a legacy dump are native.
    let dump = devnet_load.send_custom_rpc("devnet_dump", json!({})).await.unwrap();
    assert_eq!(dump["format"], "starknet-devnet-dump");
}

#[tokio::test]
async fn dump_endpoint_works_without_startup_options() {
    let devnet = BackgroundDevnet::spawn().await.expect("Could not start Devnet");
    for params in [json!({}), json!([]), serde_json::Value::Null] {
        let dump = devnet.send_custom_rpc("devnet_dump", params).await.unwrap();
        assert_eq!(dump["format"], "starknet-devnet-dump");
    }
}

#[tokio::test]
async fn dump_endpoint_fail_with_wrong_request() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    let err = devnet.send_custom_rpc("devnet_dump", json!({ "test": "" })).await.unwrap_err();
    assert_eq!(err.code, -32602);
}

#[tokio::test]
async fn dump_and_load_endpoints_reject_removed_parameters() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    for (method, params, removed) in [
        ("devnet_dump", json!({ "path": "dump.json" }), "path"),
        ("devnet_dump", json!({ "inline": true }), "inline"),
        ("devnet_load", json!({ "path": "dump.json" }), "path"),
        ("devnet_load", json!({ "events": [] }), "events"),
    ] {
        let err = devnet.send_custom_rpc(method, params).await.unwrap_err();
        assert_eq!(err.code, -32602);
        assert!(err.message.contains(&format!("`{removed}` parameter")), "{err:?}");
    }
}

#[tokio::test]
async fn load_endpoint_fail_with_wrong_request() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    let err = devnet.send_custom_rpc("devnet_load", json!({ "test": "" })).await.unwrap_err();
    assert_eq!(err.code, -32602);
}

#[tokio::test]
async fn load_endpoint_rejects_dumps_that_change_block_generation() {
    let demand = BackgroundDevnet::spawn_with_additional_args(&["--block-generation-on", "demand"])
        .await
        .unwrap();
    let dump = demand.send_custom_rpc("devnet_dump", json!({})).await.unwrap();

    let devnet = BackgroundDevnet::spawn().await.unwrap();
    let err = devnet.send_custom_rpc("devnet_load", json!({ "dump": dump })).await.unwrap_err();
    assert_eq!(err.code, -32602);
    assert!(err.message.contains("load it at startup instead"), "{err:?}");
}

#[tokio::test]
async fn mint_and_dump_and_load_on_same_devnet() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();

    let unit = FeeUnit::Wei;

    devnet.mint_unit(DUMMY_ADDRESS, DUMMY_AMOUNT, unit).await;
    let balance_before_dump = devnet.get_balance_latest(&DUMMY_ADDRESS.into(), unit).await.unwrap();
    assert_eq!(balance_before_dump, Felt::from(DUMMY_AMOUNT));

    let dump = devnet.send_custom_rpc("devnet_dump", json!({})).await.unwrap();

    devnet.mint_unit(DUMMY_ADDRESS, DUMMY_AMOUNT, unit).await;
    let balance_after_dump = devnet.get_balance_latest(&DUMMY_ADDRESS.into(), unit).await.unwrap();
    assert_eq!(balance_after_dump, balance_before_dump + Felt::from(DUMMY_AMOUNT));

    devnet.send_custom_rpc("devnet_load", json!({ "dump": dump })).await.unwrap();

    let balance_after_load = devnet.get_balance_latest(&DUMMY_ADDRESS.into(), unit).await.unwrap();
    assert_eq!(balance_after_load, balance_before_dump);

    devnet.mint_unit(DUMMY_ADDRESS, DUMMY_AMOUNT, unit).await;
    let balance_after_mint_on_loaded =
        devnet.get_balance_latest(&DUMMY_ADDRESS.into(), unit).await.unwrap();
    assert_eq!(balance_after_mint_on_loaded, balance_after_load + Felt::from(DUMMY_AMOUNT));
}

#[tokio::test]
async fn set_time_with_later_block_generation_dump_and_load() {
    let dump_file = UniqueAutoDeletableFile::new("dump_set_time");
    let devnet_dump = BackgroundDevnet::spawn_with_additional_args(&[
        "--dump-path",
        &dump_file.path,
        "--dump-on",
        "exit",
    ])
    .await
    .expect("Could not start Devnet");

    // set time in past without block generation
    let past_time = 1;
    devnet_dump
        .send_custom_rpc("devnet_setTime", json!({ "time": past_time, "generate_block": false }))
        .await
        .unwrap();

    // wait 1 second
    tokio::time::sleep(time::Duration::from_secs(1)).await;

    devnet_dump.create_block().await.unwrap();
    devnet_dump.get_latest_block_with_tx_hashes().await.unwrap();

    // dump and load
    send_ctrl_c_signal_and_wait(&devnet_dump.process).await;

    // load and assert
    let devnet_load =
        BackgroundDevnet::spawn_with_additional_args(&["--dump-path", &dump_file.path])
            .await
            .expect("Could not start Devnet");

    let latest_block = devnet_load.get_latest_block_with_tx_hashes().await.unwrap();

    assert_eq!(latest_block.block_number, 1);
    assert_eq!(latest_block.timestamp, past_time);
}

#[tokio::test]
async fn test_dumping_after_restart() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();

    // mint, restart, assert dump has no transactions
    devnet.mint(Felt::ONE, 100).await;
    devnet.restart().await;

    let dump = devnet.send_custom_rpc("devnet_dump", serde_json::Value::Null).await.unwrap();
    assert_eq!(dump["transactions"], json!([]));
    assert_eq!(dump["blocks"]["accepted"], json!([]));
}

#[tokio::test]
async fn legacy_dump_converts_to_native_format() {
    let legacy_file = UniqueAutoDeletableFile::new("legacy_dump_to_convert");
    let native_file = UniqueAutoDeletableFile::new("converted_native_dump");
    let events = json!([
        { "jsonrpc": "2.0", "id": 1, "method": "devnet_createBlock" },
        {
            "jsonrpc": "2.0", "id": 2, "method": "devnet_mint",
            "params": { "address": format!("{DUMMY_ADDRESS:#x}"), "amount": DUMMY_AMOUNT, "unit": "FRI" }
        },
    ]);
    std::fs::write(&legacy_file.path, events.to_string()).unwrap();

    let output = BackgroundDevnet::run_to_completion(&[
        "--load-path",
        &legacy_file.path,
        "--convert-legacy-dump",
        &native_file.path,
    ])
    .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

    let converted: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&native_file.path).unwrap()).unwrap();
    assert_eq!(converted["format"], "starknet-devnet-dump");

    let devnet = BackgroundDevnet::spawn_with_additional_args(&["--load-path", &native_file.path])
        .await
        .unwrap();
    assert_eq!(devnet.get_latest_block_with_tx_hashes().await.unwrap().block_number, 2);
    let balance =
        devnet.get_balance_latest(&Felt::from(DUMMY_ADDRESS), FeeUnit::Fri).await.unwrap();
    assert_eq!(balance, Felt::from(DUMMY_AMOUNT));

    // Converting a native dump is refused.
    let output = BackgroundDevnet::run_to_completion(&[
        "--load-path",
        &native_file.path,
        "--convert-legacy-dump",
        &legacy_file.path,
    ])
    .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("already in the native format"));
}
