use serde_json::json;
use starknet_rs_core::types::{BlockId, BlockTag, Felt, FunctionCall};
use starknet_rs_core::utils::{
    get_selector_from_name, get_storage_var_address, parse_cairo_short_string,
};
use starknet_rs_providers::Provider;

use crate::common::background_devnet::BackgroundDevnet;
use crate::common::constants::{
    CAIRO_1_ACCOUNT_CONTRACT_SIERRA_HASH, ETH_ERC20_CONTRACT_ADDRESS,
    ETH_ERC20_CONTRACT_CLASS_HASH, STRK_ERC20_CONTRACT_ADDRESS, STRK_ERC20_CONTRACT_CLASS_HASH,
};
use crate::common::utils::{UniqueAutoDeletableFile, to_hex_felt};

#[tokio::test]
/// Asserts that a background instance can be spawned
async fn background_devnet_can_be_spawned() {
    BackgroundDevnet::spawn().await.expect("Could not start Devnet");
}

#[tokio::test]
async fn background_devnets_at_different_ports_with_random_acquisition() {
    let devnet_args = ["--port", "0"];
    let devnet1 = BackgroundDevnet::spawn_with_additional_args(&devnet_args).await.unwrap();
    let devnet2 = BackgroundDevnet::spawn_with_additional_args(&devnet_args).await.unwrap();

    assert_ne!(devnet1.url, devnet2.url);
}

#[tokio::test]
async fn test_config() {
    // random values
    let dump_file = UniqueAutoDeletableFile::new("dummy");
    let expected_config = json!({
        "seed": 1,
        "total_accounts": 2,
        "account_contract_class_hash": Felt::from_hex_unchecked(CAIRO_1_ACCOUNT_CONTRACT_SIERRA_HASH),
        "predeployed_accounts_initial_balance": "3",
        "start_time": 4,
        "gas_price_wei": 5,
        "gas_price_fri": 7,
        "data_gas_price_wei": 6,
        "data_gas_price_fri": 8,
        "l2_gas_price_wei": 9,
        "l2_gas_price_fri": 10,
        "chain_id": "SN_MAIN",
        "dump_on": "exit",
        "dump_path": dump_file.path,
        "state_archive": "full",
        "fork_config": {
            "url": null,
            "block_number": null,
            "caching_enabled": null
        },
        "server_config": {
            "host": "0.0.0.0",
            "port": 0, // default value in tests, config not modified upon finding a free port
            "timeout": 121,
            "restricted_methods": null,
            "ui_enabled": false,
        },
        "block_generation_on": "demand",
        "lite_mode": false,
        "proof_mode": "devnet",
        "eth_erc20_class_hash": to_hex_felt(&ETH_ERC20_CONTRACT_CLASS_HASH),
        "strk_erc20_class_hash": to_hex_felt(&STRK_ERC20_CONTRACT_CLASS_HASH),
        "class_size_config": {
            "maximum_contract_class_size": 1_000_000,
            "maximum_contract_bytecode_size": 200_000,
            "maximum_sierra_length": 300_000,
        }
    });

    let devnet = BackgroundDevnet::spawn_with_additional_args(&[
        "--seed",
        &expected_config["seed"].to_string(),
        "--accounts",
        &expected_config["total_accounts"].to_string(),
        "--initial-balance",
        expected_config["predeployed_accounts_initial_balance"].as_str().unwrap(),
        "--start-time",
        &expected_config["start_time"].to_string(),
        "--gas-price",
        &expected_config["gas_price_wei"].to_string(),
        "--gas-price-fri",
        &expected_config["gas_price_fri"].to_string(),
        "--data-gas-price",
        &expected_config["data_gas_price_wei"].to_string(),
        "--data-gas-price-fri",
        &expected_config["data_gas_price_fri"].to_string(),
        "--l2-gas-price",
        &expected_config["l2_gas_price_wei"].to_string(),
        "--l2-gas-price-fri",
        &expected_config["l2_gas_price_fri"].to_string(),
        "--chain-id",
        "MAINNET",
        "--dump-on",
        expected_config["dump_on"].as_str().unwrap(),
        "--dump-path",
        expected_config["dump_path"].as_str().unwrap(),
        "--block-generation-on",
        "demand",
        "--state-archive-capacity",
        expected_config["state_archive"].as_str().unwrap(),
        "--host",
        expected_config["server_config"]["host"].as_str().unwrap(),
        "--timeout",
        &expected_config["server_config"]["timeout"].to_string(),
        "--maximum-contract-class-size",
        &expected_config["class_size_config"]["maximum_contract_class_size"].to_string(),
        "--maximum-contract-bytecode-size",
        &expected_config["class_size_config"]["maximum_contract_bytecode_size"].to_string(),
        "--maximum-sierra-length",
        &expected_config["class_size_config"]["maximum_sierra_length"].to_string(),
    ])
    .await
    .unwrap();

    let fetched_config = devnet.get_config().await;
    assert_eq!(fetched_config, expected_config);
}

#[tokio::test]
async fn predeployed_erc20_tokens_have_expected_storage() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    for (token_address, var_name, expected_value) in [
        (ETH_ERC20_CONTRACT_ADDRESS, "ERC20_name", "Ether"),
        (ETH_ERC20_CONTRACT_ADDRESS, "ERC20_symbol", "ETH"),
        (STRK_ERC20_CONTRACT_ADDRESS, "ERC20_name", "StarkNet Token"),
        (STRK_ERC20_CONTRACT_ADDRESS, "ERC20_symbol", "STRK"),
    ] {
        let actual_value = devnet
            .json_rpc_client
            .get_storage_at(
                token_address,
                get_storage_var_address(var_name, &[]).unwrap(),
                BlockId::Tag(BlockTag::Latest),
                None,
            )
            .await
            .unwrap()
            .value();

        assert_eq!(actual_value, expected_value.len().into());
    }
}

#[tokio::test]
async fn predeployed_erc20_tokens_return_expected_values_from_property_getters() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    for (token_address, getter_name, expected_value) in [
        (ETH_ERC20_CONTRACT_ADDRESS, "name", "Ether"),
        (ETH_ERC20_CONTRACT_ADDRESS, "symbol", "ETH"),
        (STRK_ERC20_CONTRACT_ADDRESS, "name", "StarkNet Token"),
        (STRK_ERC20_CONTRACT_ADDRESS, "symbol", "STRK"),
    ] {
        let actual_felts = devnet
            .json_rpc_client
            .call(
                FunctionCall {
                    contract_address: token_address,
                    entry_point_selector: get_selector_from_name(getter_name).unwrap(),
                    calldata: vec![],
                },
                BlockId::Tag(BlockTag::Latest),
            )
            .await
            .unwrap();
        assert_eq!(actual_felts.len(), 3);
        assert_eq!(parse_cairo_short_string(&actual_felts[1]).unwrap(), expected_value);
    }
}

#[tokio::test]
async fn mempool_methods_are_unavailable() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    for method in [
        "devnet_getMempool",
        "devnet_removeFromMempool",
        "devnet_clearMempool",
        "devnet_setMempoolConfig",
        "devnet_preconfirmTransactions",
        "devnet_sealBlock",
        "devnet_abortPreconfirmedBlock",
    ] {
        let error = devnet.send_custom_rpc(method, json!([])).await.unwrap_err();
        assert_eq!(error.code, -32601, "unexpected error for {method}: {error:?}");
    }
    assert!(devnet.get_config().await.get("mempool_config").is_none());
}

#[tokio::test]
async fn interval_mode_seals_periodically() {
    let devnet = BackgroundDevnet::spawn_with_additional_args(&["--block-generation-on", "1"])
        .await
        .unwrap();
    assert_eq!(
        devnet.get_config().await["block_generation_on"],
        serde_json::json!({"interval": 1})
    );
    let transaction_hash = devnet.mint(Felt::ONE, 1).await;
    for _ in 0..30 {
        let status = devnet
            .send_custom_rpc(
                "starknet_getTransactionStatus",
                json!({ "transaction_hash": transaction_hash }),
            )
            .await
            .unwrap();
        assert_eq!(status["execution_status"], "SUCCEEDED");
        if status["finality_status"] == "ACCEPTED_ON_L2" {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    panic!("Interval(1) did not seal the submitted transaction");
}
