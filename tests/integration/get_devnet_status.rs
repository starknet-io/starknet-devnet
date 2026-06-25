use serde_json::json;

use crate::common::background_devnet::BackgroundDevnet;

#[tokio::test]
async fn devnet_get_status_initial_state() {
    let devnet = BackgroundDevnet::spawn().await.expect("Could not start Devnet");

    let status = devnet.send_custom_rpc("devnet_getStatus", json!({})).await.unwrap();

    // Verify structure and initial values
    assert_eq!(status["block_count"].as_u64().unwrap(), 1, "Genesis block should be present");
    assert_eq!(status["transaction_count"].as_u64().unwrap(), 0, "No transactions initially");
    assert_eq!(status["pre_confirmed_tx_count"].as_u64().unwrap(), 0);
    assert!(status["chain_id"].as_str().unwrap().contains("SEPOLIA"));
    assert_eq!(status["protocol_version"].as_str().unwrap(), "0.10.2");
    assert!(!status["is_forked"].as_bool().unwrap());
    // fork_config is only present when forking
    assert!(
        status.get("fork_config").is_none_or(|v| v.is_null()),
        "fork_config should be absent/omitted when not forking"
    );
    assert!(status["impersonated_accounts"].as_array().unwrap().is_empty());
    assert!(!status["auto_impersonate"].as_bool().unwrap());
}

#[tokio::test]
async fn devnet_get_status_after_interaction() {
    let devnet = BackgroundDevnet::spawn().await.expect("Could not start Devnet");

    // Create a block to change block count
    devnet.create_block().await.unwrap();

    let status = devnet.send_custom_rpc("devnet_getStatus", json!({})).await.unwrap();
    assert!(
        status["block_count"].as_u64().unwrap() >= 2,
        "Block count should increase after creating a block"
    );

    // Mint tokens to generate a transaction
    let (_, account_address) = devnet.get_first_predeployed_account().await;
    let mint_amount = 1_000_000u128;
    devnet.mint(account_address, mint_amount).await;

    let status = devnet.send_custom_rpc("devnet_getStatus", json!({})).await.unwrap();
    assert!(
        status["transaction_count"].as_u64().unwrap() >= 1,
        "Transaction count should increase after minting"
    );
}
