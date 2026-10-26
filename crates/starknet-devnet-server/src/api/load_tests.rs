use serde_json::{Value, json};
use starknet_core::starknet::Starknet;
use starknet_core::starknet::starknet_config::{BlockGenerationOn, StarknetConfig};

use super::{Api, JsonRpcHandler};
use crate::ServerConfig;
use crate::rpc_core::error::RpcError;
use crate::rpc_core::response::ResponseResult;
use crate::rpc_handler::RpcHandler;

fn handler_with(config: StarknetConfig) -> JsonRpcHandler {
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

fn handler() -> JsonRpcHandler {
    handler_with(StarknetConfig { lite_mode: true, ..Default::default() })
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

async fn success(handler: &JsonRpcHandler, method: &str, params: Value) -> Value {
    match call(handler, method, params).await {
        ResponseResult::Success(value) => value,
        ResponseResult::Error(error) => panic!("{method} failed: {error:?}"),
    }
}

async fn error(handler: &JsonRpcHandler, method: &str, params: Value) -> RpcError {
    match call(handler, method, params).await {
        ResponseResult::Error(error) => error,
        ResponseResult::Success(value) => panic!("{method} succeeded with {value}"),
    }
}

async fn latest_block_number(handler: &JsonRpcHandler) -> u64 {
    handler.api.starknet.lock().await.get_latest_block().unwrap().block_number().0
}

#[tokio::test(flavor = "multi_thread")]
async fn dump_round_trips_and_invalidates_snapshots_without_reusing_ids() {
    let origin = handler();
    success(&origin, "devnet_createBlock", json!([])).await;
    success(&origin, "devnet_createBlock", json!([])).await;
    let dump = success(&origin, "devnet_dump", json!([])).await;
    assert_eq!(dump["format"], "starknet-devnet-dump");

    let target = handler();
    assert_eq!(success(&target, "devnet_snapshot", json!([])).await, json!("0x1"));
    assert_eq!(success(&target, "devnet_load", json!({ "dump": dump })).await, json!({}));

    assert_eq!(latest_block_number(&target).await, 2);
    assert_eq!(success(&target, "devnet_dump", json!({})).await, dump);
    assert_eq!(
        success(&target, "devnet_revert", json!({ "snapshot_id": "0x1" })).await,
        json!(false)
    );
    assert_eq!(success(&target, "devnet_snapshot", json!([])).await, json!("0x2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_loads_preserve_state_and_snapshots() {
    let target = handler();
    success(&target, "devnet_createBlock", json!([])).await;
    assert_eq!(success(&target, "devnet_snapshot", json!([])).await, json!("0x1"));

    let mut tampered = success(&handler(), "devnet_dump", json!([])).await;
    tampered["genesis_fingerprint"] = json!("0x1");
    let failing_replay = json!([
        { "jsonrpc": "2.0", "id": 1, "method": "devnet_createBlock" },
        { "jsonrpc": "2.0", "id": 2, "method": "devnet_revert", "params": { "snapshot_id": "zz" } },
    ]);
    for dump in [tampered, failing_replay, json!({ "format": "other" }), json!("dump")] {
        error(&target, "devnet_load", json!({ "dump": dump })).await;
        assert_eq!(latest_block_number(&target).await, 1);
        assert!(target.api.snapshots.lock().await.checkpoints.contains_key(&1));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn legacy_dump_is_replayed() {
    let target = handler();
    let events = json!([
        { "jsonrpc": "2.0", "id": 1, "method": "devnet_createBlock" },
        { "jsonrpc": "2.0", "id": 2, "method": "devnet_createBlock" },
    ]);
    success(&target, "devnet_load", json!({ "dump": events })).await;
    assert_eq!(latest_block_number(&target).await, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn legacy_dumps_with_dump_or_load_events_are_rejected() {
    let target = handler();
    for method in ["devnet_dump", "devnet_load"] {
        let events = json!([
            { "jsonrpc": "2.0", "id": 1, "method": "devnet_createBlock" },
            { "jsonrpc": "2.0", "id": 2, "method": method, "params": { "path": "dump.json" } },
        ]);
        let error = error(&target, "devnet_load", json!({ "dump": events })).await;
        assert!(error.message.contains(&format!("containing {method}")), "{error:?}");
        assert_eq!(latest_block_number(&target).await, 0);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn removed_parameters_are_rejected() {
    let target = handler();
    for (method, params, removed) in [
        ("devnet_dump", json!({ "path": "dump.json" }), "path"),
        ("devnet_dump", json!({ "inline": true }), "inline"),
        ("devnet_load", json!({ "path": "dump.json" }), "path"),
        ("devnet_load", json!({ "events": [] }), "events"),
    ] {
        let error = error(&target, method, params).await;
        assert!(error.message.contains(&format!("`{removed}` parameter")), "{error:?}");
    }
    error(&target, "devnet_load", json!({})).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn dumps_that_change_block_generation_load_only_at_startup() {
    let demand = handler_with(StarknetConfig {
        lite_mode: true,
        block_generation_on: BlockGenerationOn::Demand,
        ..Default::default()
    });
    let dump = success(&demand, "devnet_dump", json!([])).await;

    let target = handler();
    let error = error(&target, "devnet_load", json!({ "dump": dump })).await;
    assert!(error.message.contains("load it at startup instead"), "{error:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn config_reports_the_loaded_configuration() {
    let origin = handler_with(StarknetConfig { lite_mode: true, seed: 7, ..Default::default() });
    let dump = success(&origin, "devnet_dump", json!([])).await;

    let target = handler();
    assert_ne!(success(&target, "devnet_getConfig", json!({})).await["seed"], 7);
    success(&target, "devnet_load", json!({ "dump": dump })).await;
    assert_eq!(success(&target, "devnet_getConfig", json!({})).await["seed"], 7);
}
