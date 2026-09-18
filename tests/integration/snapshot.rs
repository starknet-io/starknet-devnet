use std::fs;
use std::path::Path;

use serde_json::json;
use starknet_rs_core::types::{Felt, StarknetError};
use starknet_rs_providers::{Provider, ProviderError};

use crate::common::background_devnet::BackgroundDevnet;
use crate::common::utils::{
    FeeUnit, UniqueAutoDeletableFile, assert_tx_succeeded_accepted, send_text_rpc_via_ws,
};

#[tokio::test]
async fn gauges_follow_restore_then_block_abortion() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let devnet = BackgroundDevnet::spawn_with_additional_args(&[
        "--state-archive-capacity",
        "full",
        "--metrics-host",
        "127.0.0.1",
        "--metrics-port",
        &port.to_string(),
    ])
    .await
    .unwrap();
    let assert_counts = |blocks, transactions| async move {
        let metrics = reqwest::get(format!("http://127.0.0.1:{port}/metrics"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        for (name, expected) in
            [("starknet_block_count", blocks), ("starknet_transaction_count", transactions)]
        {
            assert!(metrics.contains(&format!("# TYPE {name} gauge")));
            let value = metrics
                .lines()
                .find_map(|line| line.strip_prefix(&format!("{name} ")))
                .unwrap()
                .parse::<u64>()
                .unwrap();
            assert_eq!(value, expected, "unexpected {name}");
        }
    };
    assert_counts(1, 0).await;
    devnet.mint(Felt::ONE, 10).await;
    let saved_block = devnet.get_latest_block_with_tx_hashes().await.unwrap().block_hash;
    let snapshot = devnet.send_custom_rpc("devnet_snapshot", json!([])).await.unwrap();
    devnet.mint(Felt::ONE, 20).await;
    assert_counts(3, 2).await;
    devnet.send_custom_rpc("devnet_revert", json!({"snapshot_id": snapshot})).await.unwrap();
    assert_counts(2, 1).await;
    devnet.abort_blocks(&starknet_rs_core::types::BlockId::Hash(saved_block)).await.unwrap();
    assert_counts(1, 0).await;
}

#[tokio::test]
async fn malformed_load_preserves_snapshots() {
    let source = UniqueAutoDeletableFile::new("snapshot_malformed_import");
    fs::write(&source.path, b"{not valid JSON").unwrap();
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    let snapshot = devnet.send_custom_rpc("devnet_snapshot", json!([])).await.unwrap();
    devnet.mint(Felt::ONE, 3).await;
    assert!(devnet.send_custom_rpc("devnet_load", json!({"path": source.path})).await.is_err());
    assert_eq!(devnet.get_balance_latest(&Felt::ONE, FeeUnit::Fri).await.unwrap(), Felt::THREE);
    assert_eq!(
        devnet.send_custom_rpc("devnet_revert", json!({"snapshot_id": snapshot})).await.unwrap(),
        true
    );
    assert_eq!(devnet.get_balance_latest(&Felt::ONE, FeeUnit::Fri).await.unwrap(), Felt::ZERO);
}

#[tokio::test]
async fn older_snapshot_survives_websocket_revert_and_continued_execution() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async(devnet.ws_url()).await.unwrap();
    let older = send_text_rpc_via_ws(&mut ws, "devnet_snapshot", json!([])).await.unwrap();
    assert_eq!(older["result"], "0x1");
    let address = Felt::from(0x987_u64);
    devnet.mint(address, 10).await;
    let newer = send_text_rpc_via_ws(&mut ws, "devnet_snapshot", json!({})).await.unwrap();
    assert_eq!(newer["result"], "0x2");
    devnet.mint(address, 20).await;
    let restored = send_text_rpc_via_ws(&mut ws, "devnet_revert", json!({"snapshot_id": "0x0002"}))
        .await
        .unwrap();
    assert_eq!(restored["result"], true);
    assert_eq!(
        devnet.get_balance_latest(&address, FeeUnit::Fri).await.unwrap(),
        Felt::from(10_u32)
    );
    devnet.mint(address, 5).await;
    let restored = send_text_rpc_via_ws(&mut ws, "devnet_revert", json!({"snapshot_id": "0x1"}))
        .await
        .unwrap();
    assert_eq!(restored["result"], true);
    assert_eq!(devnet.get_balance_latest(&address, FeeUnit::Fri).await.unwrap(), Felt::ZERO);
    assert_eq!(
        devnet.send_custom_rpc("devnet_revert", json!({"snapshot_id": "0x0"})).await.unwrap(),
        false
    );
}

#[tokio::test]
async fn empty_journal_restore_allows_next_append_and_load_preserves_import_source() {
    let dump = UniqueAutoDeletableFile::new("snapshot_empty_journal");
    let source = UniqueAutoDeletableFile::new("snapshot_import_source");
    let devnet = BackgroundDevnet::spawn_with_additional_args(&[
        "--dump-on",
        "block",
        "--dump-path",
        &dump.path,
    ])
    .await
    .unwrap();
    let snapshot = devnet.send_custom_rpc("devnet_snapshot", json!([])).await.unwrap();
    devnet.mint(Felt::ONE, 10).await;
    let exported = fs::read(&dump.path).unwrap();
    fs::write(&source.path, &exported).unwrap();
    assert_eq!(
        devnet.send_custom_rpc("devnet_revert", json!({"snapshot_id": snapshot})).await.unwrap(),
        true
    );
    assert!(!Path::new(&dump.path).exists());
    devnet.mint(Felt::ONE, 2).await;
    let events: Vec<serde_json::Value> =
        serde_json::from_slice(&fs::read(&dump.path).unwrap()).unwrap();
    assert_eq!(events.len(), 1);

    devnet.send_custom_rpc("devnet_load", json!({"path": source.path})).await.unwrap();
    assert_eq!(fs::read(&source.path).unwrap(), exported);
    assert_eq!(
        devnet.get_balance_latest(&Felt::ONE, FeeUnit::Fri).await.unwrap(),
        Felt::from(10_u32)
    );
    let events: Vec<serde_json::Value> =
        serde_json::from_slice(&fs::read(&dump.path).unwrap()).unwrap();
    assert_eq!(events.len(), 1);
}

#[tokio::test]
async fn restores_state_and_consumes_nested_snapshots() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    let address = Felt::from(0x987_u64);

    let first = devnet.send_custom_rpc("devnet_snapshot", json!([])).await.unwrap();
    assert_eq!(first, json!("0x1"));

    let first_tx = devnet.mint(address, 10).await;
    let second = devnet.send_custom_rpc("devnet_snapshot", json!({})).await.unwrap();
    assert_eq!(second, json!("0x2"));
    let second_tx = devnet.mint(address, 20).await;
    assert_eq!(devnet.get_balance_latest(&address, FeeUnit::Fri).await.unwrap(), Felt::from(30_u8));

    assert_eq!(
        devnet.send_custom_rpc("devnet_revert", json!({ "snapshot_id": "0x1" })).await.unwrap(),
        json!(true)
    );
    assert_eq!(devnet.get_balance_latest(&address, FeeUnit::Fri).await.unwrap(), Felt::ZERO);

    for transaction_hash in [first_tx, second_tx] {
        assert!(matches!(
            devnet.json_rpc_client.get_transaction_by_hash(transaction_hash, None).await,
            Err(ProviderError::StarknetError(StarknetError::TransactionHashNotFound))
        ));
    }

    assert_eq!(
        devnet.send_custom_rpc("devnet_revert", json!({ "snapshot_id": "0x1" })).await.unwrap(),
        json!(false)
    );
    assert_eq!(
        devnet.send_custom_rpc("devnet_revert", json!({ "snapshot_id": "0x2" })).await.unwrap(),
        json!(false)
    );
    assert_eq!(devnet.send_custom_rpc("devnet_snapshot", json!([])).await.unwrap(), json!("0x3"));
}

#[tokio::test]
async fn restart_invalidates_snapshots_without_reusing_ids() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    assert_eq!(devnet.send_custom_rpc("devnet_snapshot", json!({})).await.unwrap(), json!("0x1"));
    devnet.restart().await;
    assert_eq!(
        devnet.send_custom_rpc("devnet_revert", json!({ "snapshot_id": "0x1" })).await.unwrap(),
        json!(false)
    );
    assert_eq!(devnet.send_custom_rpc("devnet_snapshot", json!({})).await.unwrap(), json!("0x2"));
}

#[tokio::test]
async fn validates_snapshot_id_format() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    for snapshot_id in ["1", "0x", "0xg", "0x10000000000000000"] {
        let error = devnet
            .send_custom_rpc("devnet_revert", json!({ "snapshot_id": snapshot_id }))
            .await
            .unwrap_err();
        assert_eq!(error.code, -32602);
    }
}

#[tokio::test]
async fn restores_block_dump_and_preserves_snapshot_after_rewrite_failure() {
    let dump_file = UniqueAutoDeletableFile::new("snapshot_block_dump");
    let devnet = BackgroundDevnet::spawn_with_additional_args(&[
        "--dump-on",
        "block",
        "--dump-path",
        &dump_file.path,
    ])
    .await
    .unwrap();
    devnet.mint(Felt::ONE, 1).await;
    let snapshot_id = devnet.send_custom_rpc("devnet_snapshot", json!([])).await.unwrap();
    devnet.mint(Felt::ONE, 2).await;

    fs::remove_file(&dump_file.path).unwrap();
    fs::create_dir(&dump_file.path).unwrap();
    assert!(
        devnet
            .send_custom_rpc("devnet_revert", json!({ "snapshot_id": snapshot_id }))
            .await
            .is_err()
    );

    fs::remove_dir(&dump_file.path).unwrap();
    assert_eq!(
        devnet
            .send_custom_rpc("devnet_revert", json!({ "snapshot_id": snapshot_id }))
            .await
            .unwrap(),
        json!(true)
    );
    let events: Vec<serde_json::Value> =
        serde_json::from_slice(&fs::read(&dump_file.path).unwrap()).unwrap();
    assert_eq!(events.len(), 1);
    assert!(Path::new(&dump_file.path).is_file());
}

#[tokio::test]
async fn load_invalidates_only_after_its_destructive_reset_begins() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    let missing = UniqueAutoDeletableFile::new("missing_snapshot_load");
    let address = Felt::from(0x987_u64);

    // State and transactions created before the snapshot must be recovered by the revert.
    let mint_before_snapshot = devnet.mint(address, 10).await;
    let block_before_snapshot = devnet.get_latest_block_with_tx_hashes().await.unwrap();
    let first = devnet.send_custom_rpc("devnet_snapshot", json!([])).await.unwrap();
    assert_eq!(first, json!("0x1"));

    // Mutate both state and transactions after the snapshot.
    let mint_after_snapshot = devnet.mint(address, 20).await;
    assert_eq!(devnet.get_balance_latest(&address, FeeUnit::Fri).await.unwrap(), Felt::from(30_u8));

    // Reading a missing file fails before any destructive action, so the snapshot survives and the
    // live state is left untouched.
    assert!(devnet.send_custom_rpc("devnet_load", json!({ "path": missing.path })).await.is_err());
    assert_eq!(devnet.get_balance_latest(&address, FeeUnit::Fri).await.unwrap(), Felt::from(30_u8));

    assert_eq!(
        devnet.send_custom_rpc("devnet_revert", json!({ "snapshot_id": first })).await.unwrap(),
        json!(true)
    );

    // Balance, latest block and transaction history are all restored to the snapshot.
    assert_eq!(devnet.get_balance_latest(&address, FeeUnit::Fri).await.unwrap(), Felt::from(10_u8));
    let restored_block = devnet.get_latest_block_with_tx_hashes().await.unwrap();
    assert_eq!(restored_block.block_number, block_before_snapshot.block_number);
    assert_eq!(restored_block.block_hash, block_before_snapshot.block_hash);
    assert_eq!(restored_block.transactions, block_before_snapshot.transactions);
    assert_tx_succeeded_accepted(&mint_before_snapshot, &devnet.json_rpc_client).await.unwrap();
    assert!(matches!(
        devnet.json_rpc_client.get_transaction_by_hash(mint_after_snapshot, None).await,
        Err(ProviderError::StarknetError(StarknetError::TransactionHashNotFound))
    ));

    // Take a second snapshot on top of the restored state and make it diverge again.
    let second = devnet.send_custom_rpc("devnet_snapshot", json!([])).await.unwrap();
    assert_eq!(second, json!("0x2"));
    devnet.mint(address, 5).await;
    assert_eq!(devnet.get_balance_latest(&address, FeeUnit::Fri).await.unwrap(), Felt::from(15_u8));

    assert!(
        devnet
            .send_custom_rpc(
                "devnet_load",
                json!({
                    "events": [{
                        "jsonrpc": "2.0",
                        "method": "devnet_unknown",
                        "params": [],
                        "id": 1
                    }]
                }),
            )
            .await
            .is_err()
    );

    // The failing load already performed its destructive reset, so the second checkpoint is
    // invalidated and the revert is a no-op reporting false; the reset must be observable.
    assert_eq!(devnet.get_balance_latest(&address, FeeUnit::Fri).await.unwrap(), Felt::ZERO);
    assert!(matches!(
        devnet.json_rpc_client.get_transaction_by_hash(mint_before_snapshot, None).await,
        Err(ProviderError::StarknetError(StarknetError::TransactionHashNotFound))
    ));
    assert_eq!(
        devnet.send_custom_rpc("devnet_revert", json!({ "snapshot_id": second })).await.unwrap(),
        json!(false)
    );
}
