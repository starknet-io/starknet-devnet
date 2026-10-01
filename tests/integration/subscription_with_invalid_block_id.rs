use serde_json::json;
use starknet_rs_core::types::{BlockId, Felt};
use tokio_tungstenite::connect_async;

use crate::common::background_devnet::BackgroundDevnet;
use crate::common::utils::{assert_no_notifications, send_text_rpc_via_ws};

fn block_not_found_error() -> serde_json::Value {
    json!({ "jsonrpc": "2.0", "id": 0, "error": { "code": 24, "message": "Block not found" } })
}

fn call_on_pre_confirmed_error() -> serde_json::Value {
    json!({ "jsonrpc": "2.0", "id": 0, "error": { "code": -32602, "message": "Subscription block cannot be 'pre_confirmed'" }})
}

fn call_on_l1_accepted_error() -> serde_json::Value {
    json!({ "jsonrpc": "2.0", "id": 0, "error": { "code": -32602, "message": "Subscription block cannot be 'l1_accepted'" }})
}

#[tokio::test]
async fn test_post_fork_origin_hash_not_subscribable() {
    let origin = BackgroundDevnet::spawn_forkable_devnet().await.unwrap();
    origin.mint(Felt::ONE, 10).await;
    let fork = origin.fork().await.unwrap();
    fork.mint(Felt::TWO, 20).await;
    let local_head = fork.get_latest_block_with_tx_hashes().await.unwrap();
    origin.mint(Felt::THREE, 30).await;
    origin.create_block().await.unwrap();
    let origin_head = origin.get_latest_block_with_tx_hashes().await.unwrap();
    assert_eq!(origin_head.block_number, local_head.block_number);
    assert_ne!(origin_head.block_hash, local_head.block_hash);

    let (mut ws, _) = connect_async(fork.ws_url()).await.unwrap();
    for method in ["starknet_subscribeNewHeads", "starknet_subscribeEvents"] {
        let response = send_text_rpc_via_ws(
            &mut ws,
            method,
            json!({
                "block_id": BlockId::Hash(origin_head.block_hash),
            }),
        )
        .await
        .unwrap();
        assert_eq!(response, block_not_found_error(), "{method}");
    }
    assert_no_notifications(&mut ws).await.unwrap();
}

#[tokio::test]
async fn test_subscribing_to_non_existent_block() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    let (mut ws, _) = connect_async(devnet.ws_url()).await.unwrap();

    // Cartesian product: subscription_method x invalid_block_id
    for subscription_method in ["starknet_subscribeNewHeads", "starknet_subscribeEvents"] {
        for block_id in [BlockId::Number(1), BlockId::Hash(Felt::ONE)] {
            let subscription_resp =
                send_text_rpc_via_ws(&mut ws, subscription_method, json!({ "block_id": block_id }))
                    .await
                    .unwrap();

            assert_eq!(subscription_resp, block_not_found_error())
        }
    }

    assert_no_notifications(&mut ws).await.unwrap();
}

#[tokio::test]
async fn test_aborted_blocks_not_subscribable() {
    let devnet_args = ["--state-archive-capacity", "full"];
    let devnet = BackgroundDevnet::spawn_with_additional_args(&devnet_args).await.unwrap();
    let (mut ws, _) = connect_async(devnet.ws_url()).await.unwrap();

    let created_block_hash = devnet.create_block().await.unwrap();
    devnet.abort_blocks(&BlockId::Hash(created_block_hash)).await.unwrap();

    // Cartesian product: subscription_method x invalid_block_id
    for subscription_method in ["starknet_subscribeNewHeads", "starknet_subscribeEvents"] {
        for block_id in [BlockId::Number(1), BlockId::Hash(created_block_hash)] {
            let subscription_resp =
                send_text_rpc_via_ws(&mut ws, subscription_method, json!({ "block_id": block_id }))
                    .await
                    .unwrap();

            assert_eq!(subscription_resp, block_not_found_error())
        }
    }

    assert_no_notifications(&mut ws).await.unwrap();
}

#[tokio::test]
async fn test_pre_confirmed_block_not_allowed_in_block_and_event_subscription() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    let (mut ws, _) = connect_async(devnet.ws_url()).await.unwrap();

    for subscription_method in ["starknet_subscribeNewHeads", "starknet_subscribeEvents"] {
        let subscription_resp = send_text_rpc_via_ws(
            &mut ws,
            subscription_method,
            json!({ "block_id": "pre_confirmed" }),
        )
        .await
        .unwrap();

        assert_eq!(
            subscription_resp,
            call_on_pre_confirmed_error(),
            "Method: {subscription_method}"
        );
    }
}

#[tokio::test]
async fn test_l1_accepted_block_not_allowed_in_block_and_event_subscription() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    let (mut ws, _) = connect_async(devnet.ws_url()).await.unwrap();

    for subscription_method in ["starknet_subscribeNewHeads", "starknet_subscribeEvents"] {
        let subscription_resp = send_text_rpc_via_ws(
            &mut ws,
            subscription_method,
            json!({ "block_id": "l1_accepted" }),
        )
        .await
        .unwrap();

        assert_eq!(subscription_resp, call_on_l1_accepted_error(), "Method: {subscription_method}");
    }
}
