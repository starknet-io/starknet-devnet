use std::collections::{HashMap, HashSet};

use serde_json::json;
use starknet_rs_core::types::BlockId;
use tokio_tungstenite::connect_async;

use crate::common::background_devnet::BackgroundDevnet;
use crate::common::utils::{
    SubscriptionId, assert_no_notifications, receive_notification, receive_rpc_via_ws, subscribe,
    unsubscribe,
};

#[tokio::test]
async fn snapshot_revert_reports_status_changes_without_replacing_blocks() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    let transaction_hash = devnet.mint(starknet_rs_core::types::Felt::ONE, 1).await;
    let accepted_block = devnet.get_latest_block_with_tx_hashes().await.unwrap();
    let unchanged_tx = devnet.mint(starknet_rs_core::types::Felt::TWO, 1).await;
    let saved_head = devnet.get_latest_block_with_tx_hashes().await.unwrap();
    let snapshot_id = devnet.send_custom_rpc("devnet_snapshot", json!([])).await.unwrap();
    devnet.accept_on_l1(&BlockId::Hash(accepted_block.block_hash)).await.unwrap();

    let (mut ws, _) = connect_async(devnet.ws_url()).await.unwrap();
    let status_id = subscribe(
        &mut ws,
        "starknet_subscribeTransactionStatus",
        json!({"transaction_hash": transaction_hash}),
    )
    .await
    .unwrap();
    let initial = receive_rpc_via_ws(&mut ws).await.unwrap();
    assert_eq!(initial["params"]["result"]["status"]["finality_status"], "ACCEPTED_ON_L1");
    subscribe(
        &mut ws,
        "starknet_subscribeTransactionStatus",
        json!({"transaction_hash": unchanged_tx}),
    )
    .await
    .unwrap();
    let initial = receive_rpc_via_ws(&mut ws).await.unwrap();
    assert_eq!(initial["params"]["result"]["status"]["finality_status"], "ACCEPTED_ON_L2");
    subscribe(&mut ws, "starknet_subscribeNewHeads", json!({})).await.unwrap();

    assert_eq!(
        devnet.send_custom_rpc("devnet_revert", json!({"snapshot_id": snapshot_id})).await.unwrap(),
        true
    );
    let restored_head = devnet.get_latest_block_with_tx_hashes().await.unwrap();
    assert_eq!(restored_head.block_hash, saved_head.block_hash);
    let status = devnet
        .send_custom_rpc(
            "starknet_getTransactionStatus",
            json!({"transaction_hash": transaction_hash}),
        )
        .await
        .unwrap();
    assert_eq!(status["finality_status"], "ACCEPTED_ON_L2");
    let notification =
        receive_notification(&mut ws, "starknet_subscriptionTransactionStatus", status_id)
            .await
            .unwrap();
    assert_eq!(notification["transaction_hash"], json!(transaction_hash));
    assert_eq!(notification["status"], status);
    // Unchanged statuses and retained headers must not produce duplicate notifications or reorgs.
    assert_no_notifications(&mut ws).await.unwrap();
}

#[tokio::test]
async fn reorg_notification_for_all_subscriptions() {
    let devnet_args = ["--state-archive-capacity", "full"];
    let devnet = BackgroundDevnet::spawn_with_additional_args(&devnet_args).await.unwrap();

    // create blocks for later abortion
    let starting_block_hash = devnet.create_block().await.unwrap();
    let ending_block_hash = devnet.create_block().await.unwrap();

    let mut notifiable_subscribers = HashMap::new();
    for (subscription_method, subscription_params) in [
        ("starknet_subscribeNewHeads", json!({})),
        ("starknet_subscribeTransactionStatus", json!({ "transaction_hash": "0x1" })),
        ("starknet_subscribeEvents", json!({})),
        ("starknet_subscribeNewTransactions", json!({})),
        ("starknet_subscribeNewTransactionReceipts", json!({})),
    ] {
        let (mut ws, _) = connect_async(devnet.ws_url()).await.unwrap();
        let subscription_id =
            subscribe(&mut ws, subscription_method, subscription_params).await.unwrap();
        notifiable_subscribers.insert(subscription_id, ws);
    }

    // assert that block-, tx_status- and event-subscribers got notified; unsubscribe them
    devnet.abort_blocks(&BlockId::Hash(starting_block_hash)).await.unwrap();
    for (subscription_id, ws) in notifiable_subscribers.iter_mut() {
        let notification = receive_rpc_via_ws(ws).await.unwrap();
        assert_eq!(
            notification,
            json!({
                "jsonrpc": "2.0",
                "method": "starknet_subscriptionReorg",
                "params": {
                    "result": {
                        "starting_block_hash": starting_block_hash,
                        "starting_block_number": 1,
                        "ending_block_hash": ending_block_hash,
                        "ending_block_number": 2,
                    },
                    "subscription_id": subscription_id,
                }
            })
        );
        unsubscribe(ws, subscription_id.clone()).await.unwrap();
    }

    // now that all sockets are unsubscribed, abort a new block and assert no notifications
    let additional_block_hash = devnet.create_block().await.unwrap();
    devnet.abort_blocks(&BlockId::Hash(additional_block_hash)).await.unwrap();
    for (_, mut ws) in notifiable_subscribers {
        assert_no_notifications(&mut ws).await.unwrap();
    }
}

#[tokio::test]
async fn socket_with_n_subscriptions_should_get_n_reorg_notifications() {
    let devnet_args = ["--state-archive-capacity", "full"];
    let devnet = BackgroundDevnet::spawn_with_additional_args(&devnet_args).await.unwrap();

    let created_block_hash = devnet.create_block().await.unwrap();

    // Create one socket with n subscriptions.
    let (mut ws, _) = connect_async(devnet.ws_url()).await.unwrap();
    let mut subscription_ids = vec![];
    for subscription_method in ["starknet_subscribeNewHeads", "starknet_subscribeEvents"] {
        let subscription_id = subscribe(&mut ws, subscription_method, json!({})).await.unwrap();
        subscription_ids.push(subscription_id);
    }

    // Trigger reorg.
    devnet.abort_blocks(&BlockId::Hash(created_block_hash)).await.unwrap();

    // Assert n reorg notifications received. The notifications only differ in subscription_id.
    let mut notification_ids = HashSet::new();
    for _ in subscription_ids.iter() {
        let mut notification = receive_rpc_via_ws(&mut ws).await.unwrap();

        // Reorg notifications may be received in any order. To assert one reorg subscription
        // was received per subscription_id, we extract the IDs from notifications, store them
        // in a set, and later assert equality with the set of expected subscription IDs.
        // .take() method removes the property from serde_json::Value.
        // This is intentional, because notifications do not come in deterministic order
        // and we cant assert the exact notification id in the loop.
        let notification_id = notification["params"]["subscription_id"]
            .take()
            .as_str()
            .unwrap()
            .parse::<SubscriptionId>()
            .unwrap();

        notification_ids.insert(notification_id);

        assert_eq!(
            notification,
            json!({
                "jsonrpc": "2.0",
                "method": "starknet_subscriptionReorg",
                "params": {
                    "result": {
                        "starting_block_hash": created_block_hash,
                        "starting_block_number": 1,
                        "ending_block_hash": created_block_hash,
                        "ending_block_number": 1,
                    },
                    "subscription_id": null,
                }
            })
        );
    }

    assert_eq!(notification_ids, HashSet::from_iter(subscription_ids));

    assert_no_notifications(&mut ws).await.unwrap();
}

#[tokio::test]
async fn snapshot_revert_reports_equal_height_branch_replacement() {
    let devnet =
        BackgroundDevnet::spawn_with_additional_args(&["--state-archive-capacity", "full"])
            .await
            .unwrap();
    let old_block_hash = devnet.create_block().await.unwrap();
    let snapshot_id = devnet.send_custom_rpc("devnet_snapshot", json!([])).await.unwrap();
    devnet.abort_blocks(&BlockId::Hash(old_block_hash)).await.unwrap();
    devnet.mint(starknet_rs_core::types::Felt::from(999_u64), 1).await;
    let displaced_block_hash = devnet.get_latest_block_with_tx_hashes().await.unwrap().block_hash;
    assert_ne!(old_block_hash, displaced_block_hash);

    let (mut ws, _) = connect_async(devnet.ws_url()).await.unwrap();
    let subscription_id =
        subscribe(&mut ws, "starknet_subscribeNewHeads", json!({})).await.unwrap();
    assert_eq!(
        devnet
            .send_custom_rpc("devnet_revert", json!({ "snapshot_id": snapshot_id }))
            .await
            .unwrap(),
        json!(true)
    );

    let reorg = receive_rpc_via_ws(&mut ws).await.unwrap();
    assert_eq!(
        reorg,
        json!({
            "jsonrpc": "2.0",
            "method": "starknet_subscriptionReorg",
            "params": {
                "result": {
                    "starting_block_hash": displaced_block_hash,
                    "starting_block_number": 1,
                    "ending_block_hash": displaced_block_hash,
                    "ending_block_number": 1
                },
                "subscription_id": subscription_id
            }
        })
    );
    let restored_head = receive_rpc_via_ws(&mut ws).await.unwrap();
    assert_eq!(restored_head["method"], "starknet_subscriptionNewHeads");
    assert_eq!(restored_head["params"]["result"]["block_hash"], json!(old_block_hash));
}

#[tokio::test]
async fn snapshot_revert_replays_restored_blocks_and_their_notifications_in_order() {
    let devnet =
        BackgroundDevnet::spawn_with_additional_args(&["--state-archive-capacity", "full"])
            .await
            .unwrap();
    let first_hash = devnet.create_block().await.unwrap();
    let restored_tx = devnet.mint(starknet_rs_core::types::Felt::from(42_u64), 1).await;
    let second_hash = devnet.get_latest_block_with_tx_hashes().await.unwrap().block_hash;
    let third_hash = devnet.create_block().await.unwrap();
    let snapshot_id = devnet.send_custom_rpc("devnet_snapshot", json!([])).await.unwrap();

    devnet.abort_blocks(&BlockId::Hash(first_hash)).await.unwrap();
    let displaced_tx = devnet.mint(starknet_rs_core::types::Felt::from(43_u64), 1).await;
    assert_ne!(displaced_tx, restored_tx);

    let mut subscriptions = HashMap::new();
    for (method, params) in [
        ("starknet_subscribeNewHeads", json!({})),
        ("starknet_subscribeNewTransactions", json!({})),
        ("starknet_subscribeNewTransactionReceipts", json!({})),
        ("starknet_subscribeEvents", json!({})),
        ("starknet_subscribeTransactionStatus", json!({ "transaction_hash": restored_tx })),
    ] {
        let (mut ws, _) = connect_async(devnet.ws_url()).await.unwrap();
        let subscription_id = subscribe(&mut ws, method, params).await.unwrap();
        subscriptions.insert(method, (ws, subscription_id));
    }

    // Events subscriptions backfill the current latest block on registration.
    let (events_ws, events_id) = subscriptions.get_mut("starknet_subscribeEvents").unwrap();
    let mut displaced_event_count = 0;
    loop {
        match receive_rpc_via_ws(events_ws).await {
            Ok(notification) => {
                assert_eq!(notification["method"], "starknet_subscriptionEvents");
                assert_eq!(notification["params"]["subscription_id"], json!(events_id));
                assert_eq!(
                    notification["params"]["result"]["transaction_hash"],
                    json!(displaced_tx)
                );
                displaced_event_count += 1;
            }
            Err(error) if error.to_string().contains("deadline has elapsed") => break,
            Err(error) => panic!("Unexpected WebSocket error: {error}"),
        }
    }
    assert!(displaced_event_count > 0);

    assert_eq!(
        devnet
            .send_custom_rpc("devnet_revert", json!({ "snapshot_id": snapshot_id }))
            .await
            .unwrap(),
        json!(true)
    );

    for (ws, subscription_id) in subscriptions.values_mut() {
        let notification = receive_rpc_via_ws(ws).await.unwrap();
        assert_eq!(notification["method"], "starknet_subscriptionReorg");
        assert_eq!(notification["params"]["subscription_id"], json!(subscription_id));
    }

    let (ws, subscription_id) = subscriptions.get_mut("starknet_subscribeNewHeads").unwrap();
    for (number, hash) in [(1, first_hash), (2, second_hash), (3, third_hash)] {
        let head =
            receive_notification(ws, "starknet_subscriptionNewHeads", subscription_id.clone())
                .await
                .unwrap();
        assert_eq!(head["block_number"], json!(number));
        assert_eq!(head["block_hash"], json!(hash));
    }
    assert_no_notifications(ws).await.unwrap();

    for (method, notification_method) in [
        ("starknet_subscribeNewTransactions", "starknet_subscriptionNewTransaction"),
        ("starknet_subscribeNewTransactionReceipts", "starknet_subscriptionNewTransactionReceipts"),
        ("starknet_subscribeEvents", "starknet_subscriptionEvents"),
        ("starknet_subscribeTransactionStatus", "starknet_subscriptionTransactionStatus"),
    ] {
        let (ws, subscription_id) = subscriptions.get_mut(method).unwrap();
        let result =
            receive_notification(ws, notification_method, subscription_id.clone()).await.unwrap();
        assert_eq!(result["transaction_hash"], json!(restored_tx));
        if method == "starknet_subscribeEvents" {
            assert_eq!(result["block_hash"], json!(second_hash));
            assert_eq!(result["block_number"], json!(2));
        } else {
            assert_no_notifications(ws).await.unwrap();
        }
    }
}

#[tokio::test]
async fn snapshot_revert_preserves_l1_finality_in_restored_notifications() {
    let devnet =
        BackgroundDevnet::spawn_with_additional_args(&["--state-archive-capacity", "full"])
            .await
            .unwrap();
    let restored_tx = devnet.mint(starknet_rs_core::types::Felt::from(42_u64), 1).await;
    let restored_block_hash = devnet.get_latest_block_with_tx_hashes().await.unwrap().block_hash;
    devnet.accept_on_l1(&BlockId::Hash(restored_block_hash)).await.unwrap();
    let snapshot_id = devnet.send_custom_rpc("devnet_snapshot", json!([])).await.unwrap();

    devnet.abort_blocks(&BlockId::Hash(restored_block_hash)).await.unwrap();
    let displaced_block_hash = devnet.create_block().await.unwrap();
    assert_ne!(displaced_block_hash, restored_block_hash);

    let (mut tx_ws, _) = connect_async(devnet.ws_url()).await.unwrap();
    let tx_subscription_id =
        subscribe(&mut tx_ws, "starknet_subscribeNewTransactions", json!({})).await.unwrap();
    let (mut events_ws, _) = connect_async(devnet.ws_url()).await.unwrap();
    let events_subscription_id = subscribe(
        &mut events_ws,
        "starknet_subscribeEvents",
        json!({ "finality_status": "ACCEPTED_ON_L1" }),
    )
    .await
    .unwrap();

    assert_eq!(
        devnet
            .send_custom_rpc("devnet_revert", json!({ "snapshot_id": snapshot_id }))
            .await
            .unwrap(),
        json!(true)
    );

    for (ws, subscription_id) in
        [(&mut tx_ws, &tx_subscription_id), (&mut events_ws, &events_subscription_id)]
    {
        let reorg = receive_rpc_via_ws(ws).await.unwrap();
        assert_eq!(reorg["method"], "starknet_subscriptionReorg");
        assert_eq!(reorg["params"]["subscription_id"], json!(subscription_id));
    }

    assert_no_notifications(&mut tx_ws).await.unwrap();

    let event =
        receive_notification(&mut events_ws, "starknet_subscriptionEvents", events_subscription_id)
            .await
            .unwrap();
    assert_eq!(event["transaction_hash"], json!(restored_tx));
    assert_eq!(event["finality_status"], "ACCEPTED_ON_L1");
}
