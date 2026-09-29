use std::sync::Arc;
use std::time::Duration;

use axum::routing::post;
use axum::{Json, Router};
use futures::future::join_all;
use futures::{StreamExt, poll};
use serde_json::{Value, json};
use starknet_core::starknet::Starknet;
use starknet_core::starknet::starknet_config::{DumpOn, StarknetConfig};
use starknet_types::rpc::block::SubscriptionBlockId;
use tokio::sync::Notify;

use super::models::{
    EventsSubscriptionInput, JsonRpcSubscriptionRequest, SubscriptionBlockIdInput,
};
use super::origin_forwarder::OriginForwarder;
use super::{Api, JsonRpcHandler};
use crate::ServerConfig;
use crate::rpc_core::response::ResponseResult;
use crate::rpc_handler::RpcHandler;
use crate::socket_writer::SocketSender;

fn handler() -> JsonRpcHandler {
    let config = StarknetConfig { dump_on: Some(DumpOn::Request), ..Default::default() };
    JsonRpcHandler::new(Api::new(
        Starknet::new(&config).unwrap(),
        ServerConfig {
            host: std::net::Ipv4Addr::LOCALHOST.into(),
            port: 0,
            timeout: 30,
            log_request: false,
            log_response: false,
            restricted_methods: None,
            ui_enabled: false,
        },
    ))
}

async fn call(handler: &JsonRpcHandler, method: &str, params: Value) -> ResponseResult {
    handler
        .on_call(
            serde_json::from_value(json!({
                "jsonrpc": "2.0", "id": 1, "method": method, "params": params,
            }))
            .unwrap(),
        )
        .await
        .result
}

#[tokio::test(flavor = "multi_thread")]
async fn latest_subscription_preparation_does_not_read_moving_state() {
    let handler = handler();
    let _lifecycle = handler.api.lifecycle.write().await;
    let _starknet = handler.api.starknet.lock().await;
    let requests = [
        JsonRpcSubscriptionRequest::NewHeads(None),
        JsonRpcSubscriptionRequest::NewHeads(Some(SubscriptionBlockIdInput {
            block_id: SubscriptionBlockId::Latest,
        })),
        JsonRpcSubscriptionRequest::Events(None),
        JsonRpcSubscriptionRequest::Events(Some(EventsSubscriptionInput {
            block_id: Some(SubscriptionBlockId::Latest),
            from_address: None,
            keys: None,
            finality_status: None,
        })),
    ];

    for request in requests {
        tokio::time::timeout(Duration::from_secs(1), handler.prepare_ws_subscription(request))
            .await
            .expect("latest preparation waited for local state")
            .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_during_revert_preparation_preserves_snapshot_and_journal() {
    let handler = handler();
    assert_eq!(
        call(&handler, "devnet_snapshot", json!([])).await,
        ResponseResult::Success(json!("0x1"))
    );
    assert!(matches!(
        call(&handler, "devnet_createBlock", json!([])).await,
        ResponseResult::Success(_)
    ));

    // Force a real suspension after revert has found the checkpoint but before publication.
    let sockets = handler.api.sockets.lock().await;
    let mut revert = Box::pin(call(&handler, "devnet_revert", json!({"snapshot_id": "0x1"})));
    assert!(poll!(&mut revert).is_pending());
    drop(revert);
    drop(sockets);
    assert!(handler.api.snapshots.lock().await.checkpoints.contains_key(&1));
    assert_eq!(handler.api.dumpable_events.lock().await.len(), 1);
    assert_eq!(handler.api.starknet.lock().await.get_latest_block().unwrap().block_number().0, 1);

    assert_eq!(
        call(&handler, "devnet_revert", json!({"snapshot_id": "0x1"})).await,
        ResponseResult::Success(json!(true))
    );
    assert!(handler.api.dumpable_events.lock().await.is_empty());
    assert_eq!(handler.api.starknet.lock().await.get_latest_block().unwrap().block_number().0, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_writes_and_snapshots_keep_journal_positions_consistent() {
    let handler = handler();
    let calls = (0..12).map(|index| {
        call(
            &handler,
            if index % 2 == 0 { "devnet_createBlock" } else { "devnet_snapshot" },
            json!([]),
        )
    });
    let lifecycle = handler.api.lifecycle.write().await;
    let mut calls = Box::pin(join_all(calls));
    assert!(poll!(&mut calls).is_pending());
    drop(lifecycle);
    for result in calls.await {
        assert!(matches!(result, ResponseResult::Success(_)));
    }
    let snapshots = handler.api.snapshots.lock().await;
    assert_eq!(snapshots.checkpoints.len(), 6);
    for checkpoint in snapshots.checkpoints.values() {
        assert_eq!(
            checkpoint.core.latest_block().unwrap().block_number().0 as usize,
            checkpoint.dump_event_count
        );
    }
    assert_eq!(handler.api.dumpable_events.lock().await.len(), 6);
}

#[tokio::test(flavor = "multi_thread")]
async fn allocator_exhaustion_preserves_existing_checkpoints() {
    let handler = handler();
    call(&handler, "devnet_snapshot", json!([])).await;
    handler.api.snapshots.lock().await.last_id = u64::MAX - 1;
    assert_eq!(
        call(&handler, "devnet_snapshot", json!([])).await,
        ResponseResult::Success(json!("0xffffffffffffffff"))
    );
    assert!(matches!(call(&handler, "devnet_snapshot", json!([])).await, ResponseResult::Error(_)));
    assert_eq!(handler.api.snapshots.lock().await.last_id, u64::MAX);
    assert_eq!(
        call(&handler, "devnet_revert", json!({"snapshot_id": "0xFFFFFFFFFFFFFFFF"})).await,
        ResponseResult::Success(json!(true))
    );
    assert_eq!(
        call(&handler, "devnet_revert", json!({"snapshot_id": "0x1"})).await,
        ResponseResult::Success(json!(true))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn slow_origin_fallback_does_not_block_revert_and_uses_captured_acceptance() {
    let mut handler = handler();
    let received = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let origin = Router::new().route(
        "/",
        post({
            let received = received.clone();
            let release = release.clone();
            move || {
                let received = received.clone();
                let release = release.clone();
                async move {
                    received.notify_one();
                    release.notified().await;
                    Json(json!({"jsonrpc": "2.0", "id": 1, "result": {
                        "block_number": 5, "finality_status": "ACCEPTED_ON_L2"
                    }}))
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap()).parse().unwrap();
    let origin_task = tokio::spawn(async move {
        axum::serve(listener, origin).await.unwrap();
    });
    let forwarder = OriginForwarder::new(url, 10);
    handler.origin_caller = Some(forwarder.clone());
    call(&handler, "devnet_snapshot", json!([])).await;
    forwarder.set_accepted_on_l1_through(5).await;

    let request = tokio::spawn({
        let handler = handler.clone();
        async move {
            call(&handler, "starknet_getTransactionReceipt", json!({"transaction_hash": "0x123"}))
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(5), received.notified()).await.unwrap();
    let restored = tokio::time::timeout(
        Duration::from_secs(5),
        call(&handler, "devnet_revert", json!({"snapshot_id": "0x1"})),
    )
    .await
    .unwrap();
    assert_eq!(restored, ResponseResult::Success(json!(true)));
    assert_eq!(forwarder.acceptance_boundary().await, None);
    release.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(5), request).await.unwrap().unwrap();
    assert_eq!(
        result,
        ResponseResult::Success(json!({
            "block_number": 5, "finality_status": "ACCEPTED_ON_L1"
        }))
    );
    origin_task.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn slow_subscription_backfill_does_not_block_writes_or_lose_new_heads() {
    let mut handler = handler();
    let received = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let origin = Router::new().route(
        "/",
        post({
            let received = received.clone();
            let release = release.clone();
            move |Json(request): Json<Value>| {
                let received = received.clone();
                let release = release.clone();
                async move {
                    received.notify_one();
                    release.notified().await;
                    let result = if request["method"] == "starknet_getEvents" {
                        json!({"events": [], "continuation_token": null})
                    } else {
                        json!({
                            "status": "ACCEPTED_ON_L2",
                            "block_hash": "0x1",
                            "parent_hash": "0x0",
                            "block_number": 0,
                            "new_root": "0x0",
                            "timestamp": 0,
                            "sequencer_address": "0x0",
                            "l1_gas_price": {"price_in_wei": "0x0", "price_in_fri": "0x0"},
                            "l2_gas_price": {"price_in_wei": "0x0", "price_in_fri": "0x0"},
                            "l1_data_gas_price": {"price_in_wei": "0x0", "price_in_fri": "0x0"},
                            "l1_da_mode": "CALLDATA",
                            "starknet_version": "0.13.0",
                            "event_commitment": "0x0",
                            "transaction_commitment": "0x0",
                            "receipt_commitment": "0x0",
                            "state_diff_commitment": "0x0",
                            "event_count": 0,
                            "transaction_count": 0,
                            "state_diff_length": 0,
                            "transactions": []
                        })
                    };
                    Json(json!({"jsonrpc": "2.0", "id": request["id"], "result": result}))
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url: url::Url = format!("http://{}", listener.local_addr().unwrap()).parse().unwrap();
    let origin_task = tokio::spawn(async move { axum::serve(listener, origin).await.unwrap() });
    let config = Arc::make_mut(&mut handler.api.config);
    config.fork_config.url = Some(url.clone());
    config.fork_config.block_number = Some(0);
    handler.origin_caller = Some(OriginForwarder::new(url, 0));

    let (sender, writer) = SocketSender::channel();
    let (output, mut messages) = futures::channel::mpsc::unbounded();
    let writer_task = tokio::spawn(writer.run(output));
    let socket_id = handler.api.sockets.lock().await.insert(sender);

    for (method, create_block) in
        [("starknet_subscribeNewHeads", true), ("starknet_subscribeEvents", false)]
    {
        let request = serde_json::from_value(json!({
            "jsonrpc": "2.0", "id": 1, "method": method,
            "params": {"block_id": {"block_number": 0}},
        }))
        .unwrap();
        let subscription = tokio::spawn({
            let handler = handler.clone();
            async move { handler.on_websocket_rpc_call(&request, socket_id).await }
        });
        tokio::time::timeout(Duration::from_secs(5), received.notified()).await.unwrap();
        assert!(handler.api.sockets.try_lock().is_ok());
        let snapshot = tokio::time::timeout(
            Duration::from_secs(5),
            call(&handler, "devnet_snapshot", json!([])),
        )
        .await
        .unwrap();
        let ResponseResult::Success(snapshot_id) = snapshot else {
            panic!("snapshot failed during origin backfill");
        };
        if create_block {
            let block = tokio::time::timeout(
                Duration::from_secs(5),
                call(&handler, "devnet_createBlock", json!([])),
            )
            .await
            .unwrap();
            assert!(matches!(block, ResponseResult::Success(_)));
            let reverted = tokio::time::timeout(
                Duration::from_secs(5),
                call(&handler, "devnet_revert", json!({"snapshot_id": snapshot_id})),
            )
            .await
            .unwrap();
            assert_eq!(reverted, ResponseResult::Success(json!(true)));
            let replacement = tokio::time::timeout(
                Duration::from_secs(5),
                call(&handler, "devnet_createBlock", json!([])),
            )
            .await
            .unwrap();
            assert!(matches!(replacement, ResponseResult::Success(_)));
        }
        release.notify_one();
        tokio::time::timeout(Duration::from_secs(5), subscription).await.unwrap().unwrap().unwrap();

        let confirmation =
            tokio::time::timeout(Duration::from_secs(5), messages.next()).await.unwrap().unwrap();
        let axum::extract::ws::Message::Text(confirmation) = confirmation else {
            panic!("expected subscription confirmation");
        };
        assert!(serde_json::from_str::<Value>(&confirmation).unwrap()["result"].is_string());
        if create_block {
            for expected_number in 0..=1 {
                let notification = tokio::time::timeout(Duration::from_secs(5), messages.next())
                    .await
                    .unwrap()
                    .unwrap();
                let axum::extract::ws::Message::Text(notification) = notification else {
                    panic!("expected new-head notification");
                };
                assert_eq!(
                    serde_json::from_str::<Value>(&notification).unwrap()["params"]["result"]
                        ["block_number"],
                    json!(expected_number)
                );
            }
        }
    }

    writer_task.abort();
    origin_task.abort();
}
