use serde_json::json;
use starknet_rs_core::types::{BlockTag, Felt, StarknetError};
use starknet_rs_providers::{Provider, ProviderError};

use crate::common::background_devnet::BackgroundDevnet;
use crate::common::utils::{FeeUnit, assert_tx_succeeded_accepted, send_text_rpc_via_ws};

#[tokio::test]
async fn restores_pre_confirmed_state_in_demand_and_interval_modes() {
    for archive in ["none", "full"] {
        for mode in ["demand", "60"] {
            let devnet = BackgroundDevnet::spawn_with_additional_args(&[
                "--block-generation-on",
                mode,
                "--state-archive-capacity",
                archive,
            ])
            .await
            .unwrap();
            let address = Felt::from(0x987_u64);
            let retained_tx = devnet.mint(address, 10).await;
            let before = devnet.get_pre_confirmed_block_with_tx_hashes().await.unwrap();
            assert_eq!(before.transactions, vec![retained_tx]);
            let snapshot = devnet.send_custom_rpc("devnet_snapshot", json!([])).await.unwrap();

            let removed_tx = devnet.mint(address, 20).await;
            assert_eq!(
                devnet
                    .get_balance_by_tag(&address, FeeUnit::Fri, BlockTag::PreConfirmed)
                    .await
                    .unwrap(),
                Felt::from(30_u8)
            );
            assert_eq!(
                devnet
                    .send_custom_rpc("devnet_revert", json!({ "snapshot_id": snapshot }))
                    .await
                    .unwrap(),
                true
            );

            let restored = devnet.get_pre_confirmed_block_with_tx_hashes().await.unwrap();
            assert_eq!(restored.transactions, before.transactions, "{archive} {mode}");
            assert_eq!(restored.timestamp, before.timestamp, "{archive} {mode}");
            assert_eq!(
                devnet
                    .get_balance_by_tag(&address, FeeUnit::Fri, BlockTag::PreConfirmed)
                    .await
                    .unwrap(),
                Felt::from(10_u8),
                "{archive} {mode}"
            );
            assert!(matches!(
                devnet.json_rpc_client.get_transaction_by_hash(removed_tx, None).await,
                Err(ProviderError::StarknetError(StarknetError::TransactionHashNotFound))
            ));
            devnet.create_block().await.unwrap();
            assert_eq!(
                devnet.get_balance_latest(&address, FeeUnit::Fri).await.unwrap(),
                Felt::from(10_u8),
                "{archive} {mode}"
            );
        }
    }
}

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
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    let snapshot = devnet.send_custom_rpc("devnet_snapshot", json!([])).await.unwrap();
    devnet.mint(Felt::ONE, 3).await;
    let incomplete = json!({ "format": "starknet-devnet-dump", "version": 1 });
    let error =
        devnet.send_custom_rpc("devnet_load", json!({ "dump": incomplete })).await.unwrap_err();
    assert_eq!(error.code, -32602);
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
async fn load_invalidates_snapshots_without_reusing_ids() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    assert_eq!(devnet.send_custom_rpc("devnet_snapshot", json!([])).await.unwrap(), json!("0x1"));
    devnet.mint(Felt::ONE, 10).await;
    let dump = devnet.send_custom_rpc("devnet_dump", json!({})).await.unwrap();
    devnet.mint(Felt::ONE, 2).await;

    devnet.send_custom_rpc("devnet_load", json!({ "dump": dump })).await.unwrap();
    assert_eq!(
        devnet.get_balance_latest(&Felt::ONE, FeeUnit::Fri).await.unwrap(),
        Felt::from(10_u32)
    );
    assert_eq!(
        devnet.send_custom_rpc("devnet_revert", json!({ "snapshot_id": "0x1" })).await.unwrap(),
        json!(false)
    );
    assert_eq!(devnet.send_custom_rpc("devnet_snapshot", json!([])).await.unwrap(), json!("0x2"));
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
async fn failed_loads_preserve_snapshots_and_state() {
    let devnet = BackgroundDevnet::spawn().await.unwrap();
    let address = Felt::from(0x987_u64);

    // State and transactions created before the snapshot must be recovered by the revert.
    let mint_before_snapshot = devnet.mint(address, 10).await;
    let block_before_snapshot = devnet.get_latest_block_with_tx_hashes().await.unwrap();
    let first = devnet.send_custom_rpc("devnet_snapshot", json!([])).await.unwrap();
    assert_eq!(first, json!("0x1"));

    // Mutate both state and transactions after the snapshot.
    let mint_after_snapshot = devnet.mint(address, 20).await;
    assert_eq!(devnet.get_balance_latest(&address, FeeUnit::Fri).await.unwrap(), Felt::from(30_u8));

    // An invalid native dump is rejected without changing the state or the snapshot.
    let invalid = json!({ "format": "starknet-devnet-dump", "version": 2 });
    assert!(devnet.send_custom_rpc("devnet_load", json!({ "dump": invalid })).await.is_err());
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

    // A legacy dump whose replay fails is replayed in isolation, so it leaves the state and the
    // snapshot untouched too.
    let failing_events = json!([{
        "jsonrpc": "2.0",
        "method": "devnet_unknown",
        "params": [],
        "id": 1
    }]);
    assert!(
        devnet.send_custom_rpc("devnet_load", json!({ "dump": failing_events })).await.is_err()
    );
    assert_eq!(devnet.get_balance_latest(&address, FeeUnit::Fri).await.unwrap(), Felt::from(15_u8));
    assert_eq!(
        devnet.send_custom_rpc("devnet_revert", json!({ "snapshot_id": second })).await.unwrap(),
        json!(true)
    );
    assert_eq!(devnet.get_balance_latest(&address, FeeUnit::Fri).await.unwrap(), Felt::from(10_u8));
}
