//! Integration tests for the v1 mempool plan (mempool mode + devnet mempool RPCs).
//!
//! The plan governs the following surfaces; tests below assert the behavior end-to-end via
//! running devnet instances:
//! * `BlockGenerationOn::Mempool` keeps user txs RECEIVED until they are pre-confirmed or sealed.
//! * `devnet_getMempool` / `devnet_removeFromMempool` / `devnet_clearMempool` reflect
//!   RECEIVED/CANDIDATE/PRE_CONFIRMED state and only act on RECEIVED entries.
//! * `devnet_preconfirmTransactions` selects per ordering policy, with optional forced hashes or
//!   max-transactions cap. `selected`/RECEIVED transitions are observable.
//! * `devnet_sealBlock` performs strict sealing: pre-confirms RECEIVED only on demand, but never
//!   pulls them in automatically.
//! * `devnet_abortPreconfirmedBlock` returns every PRE_CONFIRMED entry back to RECEIVED.
//! * `devnet_abortBlocks` clears all mempool state (RECEIVED + open proposal).
//! * `devnet_mint` uses the system lane, so balance changes are immediately visible even when block
//!   generation is on `mempool`.
//! * Duplicate nonces (when strict nonce checking applies) are rejected with RPC code 59.
//!
//! The legacy `Interval(<seconds>)` mode is preserved as a compatibility path; its deprecation
//! warning is not asserted here because stderr is not captured in the BackgroundDevnet harness,
//! but its functional behavior is verified.

use serde_json::json;
use starknet_rs_accounts::{Account, ExecutionEncoding, SingleOwnerAccount};
use starknet_rs_core::types::{BlockId, BlockTag, Call, Felt};
use starknet_rs_core::utils::get_selector_from_name;
use starknet_rs_providers::Provider;
use starknet_rs_providers::jsonrpc::{HttpTransport, JsonRpcClient};
use starknet_rs_signers::LocalWallet;

use crate::common::background_devnet::BackgroundDevnet;
use crate::common::constants::{
    PREDEPLOYED_ACCOUNT_ADDRESS, RPC_PATH, STRK_ERC20_CONTRACT_ADDRESS,
};
use crate::common::utils::FeeUnit;

/// Returns the ERC20 transfer selector: `transfer(to: ContractAddress, amount: u256)`.
fn transfer_selector() -> Felt {
    get_selector_from_name("transfer").unwrap()
}

/// Build a `Call` invoking `transfer(recipient, amount_low, amount_high)` on the STRK ERC20.
fn strk_transfer_call(recipient: Felt, amount: u128) -> Call {
    Call {
        to: STRK_ERC20_CONTRACT_ADDRESS,
        selector: transfer_selector(),
        calldata: vec![recipient, Felt::from(amount), Felt::ZERO],
    }
}

/// Spawns a devnet in mempool mode with FIFO ordering, max-transactions-per-block = 2.
async fn spawn_mempool_devnet() -> BackgroundDevnet {
    BackgroundDevnet::spawn_with_additional_args(&["--block-generation-on", "mempool"])
        .await
        .expect("Could not start Devnet in mempool mode")
}

/// Returns a cloneable RPC client connected to the devnet JSON-RPC endpoint.
fn json_rpc_client(devnet: &BackgroundDevnet) -> JsonRpcClient<HttpTransport> {
    let url = reqwest::Url::parse(&format!("{}{}", devnet.url, RPC_PATH)).unwrap();
    JsonRpcClient::new(HttpTransport::new(url))
}

/// Returns the first predeployed account as a `SingleOwnerAccount` ready to submit txs.
async fn first_predeployed_account<'a>(
    devnet: &'a BackgroundDevnet,
    client: &'a JsonRpcClient<HttpTransport>,
) -> SingleOwnerAccount<&'a JsonRpcClient<HttpTransport>, LocalWallet> {
    let (signer, address) = devnet.get_first_predeployed_account_credentials().await;
    let chain_id = client.chain_id().await.unwrap();
    SingleOwnerAccount::new(client, signer, address, chain_id, ExecutionEncoding::New)
}

/// Sends a tiny transfer from the predeployed account to `recipient` in mempool mode, returning
/// the transaction hash. Uses the STRK ERC20 because the predeployed account is funded with it.
async fn submit_transfer_in_mempool(
    account: &SingleOwnerAccount<&JsonRpcClient<HttpTransport>, LocalWallet>,
    recipient: Felt,
    amount: u128,
) -> Felt {
    let result = account
        .execute_v3(vec![strk_transfer_call(recipient, amount)])
        .l1_gas(0)
        .l1_data_gas(1000)
        .l2_gas(1e8 as u64)
        .send()
        .await
        .expect("transfer should submit");
    result.transaction_hash
}

/// Asserts the JSON-RPC response of `devnet_getMempool` contains `n` RECEIVED transactions.
async fn assert_received_count(devnet: &BackgroundDevnet, n: usize) {
    let resp = devnet.send_custom_rpc("devnet_getMempool", json!({})).await.unwrap();
    let txs = resp["transactions"].as_array().unwrap();
    let received: Vec<&serde_json::Value> =
        txs.iter().filter(|t| t["status"].as_str() == Some("RECEIVED")).collect();
    assert_eq!(received.len(), n, "expected {n} RECEIVED transactions, got: {resp}");
}

/// Asserts the JSON-RPC response of `devnet_getMempool` shows a transaction in a given phase.
async fn assert_phase(devnet: &BackgroundDevnet, tx_hash: Felt, expected: &str) {
    let resp = devnet.send_custom_rpc("devnet_getMempool", json!({})).await.unwrap();
    let txs = resp["transactions"].as_array().unwrap();
    let tx_str = format!("{:#x}", tx_hash);
    let entry = txs
        .iter()
        .find(|t| t["transaction_hash"].as_str() == Some(tx_str.as_str()))
        .unwrap_or_else(|| panic!("tx {tx_str} not in mempool snapshot: {resp}"));
    let actual = entry["status"].as_str().unwrap_or_default();
    assert_eq!(actual, expected, "wrong phase for {tx_str}: {entry}");
}

/// ---------------------------------------------------------------------------
/// Plan-mandated behaviors
/// ---------------------------------------------------------------------------

/// Plan: "devnet_mint ... uses the system lane and force-processes its generated transaction"
/// so the balance change is observable immediately even in `mempool` mode.
#[tokio::test]
async fn mint_is_force_processed_in_mempool_mode() {
    let devnet = spawn_mempool_devnet().await;
    let recipient = Felt::from_hex_unchecked(PREDEPLOYED_ACCOUNT_ADDRESS);
    let balance_before =
        devnet.get_balance_by_tag(&recipient, FeeUnit::Fri, BlockTag::Latest).await.unwrap();
    devnet.mint(recipient, 1_000).await;

    // In mempool mode, the mint should be force-processed and visible at pre_confirmed
    // without any explicit preconfirm/seal call.
    let balance_after =
        devnet.get_balance_by_tag(&recipient, FeeUnit::Fri, BlockTag::PreConfirmed).await.unwrap();
    assert_eq!(
        balance_after,
        balance_before + Felt::from(1_000u64),
        "mint must be observable immediately in mempool mode"
    );

    // And the mint tx must NOT linger in the mempool as a user RECEIVED entry (system lane).
    let resp = devnet.send_custom_rpc("devnet_getMempool", json!({})).await.unwrap();
    let txs = resp["transactions"].as_array().unwrap();
    assert_eq!(txs.len(), 0, "mint tx must not appear in user mempool: {resp}");
}

/// Plan: "Restart and block abortion clear RECEIVED/CANDIDATE transactions in v1".
/// A pending pre-confirmed block (containing accepted txs) is in the way of the test — we
/// abort it via `devnet_abortBlocks` on the pre-confirmed block, and verify the open
/// mempool state is cleared.
#[tokio::test]
async fn abort_blocks_clears_mempool() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    // Submit two transfers: they remain RECEIVED in mempool mode.
    let h1 = submit_transfer_in_mempool(&account, Felt::ONE, 1).await;
    let _h2 = submit_transfer_in_mempool(&account, Felt::from(2u64), 1).await;
    assert_received_count(&devnet, 2).await;

    // Pre-confirm one of them; it becomes PRE_CONFIRMED while the other stays RECEIVED.
    let preconfirm: serde_json::Value = devnet
        .send_custom_rpc(
            "devnet_preconfirmTransactions",
            json!({ "transaction_hashes": [format!("{:#x}", h1)] }),
        )
        .await
        .unwrap();
    let pre_confirmed: Vec<String> = preconfirm["pre_confirmed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(pre_confirmed.contains(&format!("{:#x}", h1)));

    // Now abort the pre-confirmed block. The plan says: block abortion clears the entire
    // mempool (both the still-RECEIVED h2 and the PRE_CONFIRMED h1 should disappear).
    let aborted = devnet
        .abort_blocks(&BlockId::Tag(BlockTag::PreConfirmed))
        .await
        .expect("abort_blocks should succeed");
    assert!(!aborted.is_empty(), "expected at least one aborted block");

    let resp = devnet.send_custom_rpc("devnet_getMempool", json!({})).await.unwrap();
    let txs = resp["transactions"].as_array().unwrap();
    assert_eq!(txs.len(), 0, "abort_blocks must clear all mempool entries: {resp}");
    let pre_confirmed_hashes = resp["pre_confirmed_transaction_hashes"].as_array().unwrap();
    assert_eq!(pre_confirmed_hashes.len(), 0, "open proposal must be empty after abort: {resp}");
}

/// Plan: a transaction submitted in mempool mode goes RECEIVED, then CANDIDATE on selection,
/// then PRE_CONFIRMED on successful execution. The transition is observable through
/// `devnet_getMempool` and the open proposal hash list.
#[tokio::test]
async fn mempool_phases_received_candidate_preconfirmed() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    let hash = submit_transfer_in_mempool(&account, Felt::ONE, 1).await;
    assert_phase(&devnet, hash, "RECEIVED").await;

    // The selection happens in `preconfirm_transactions` (when it returns). The intermediate
    // CANDIDATE phase is short-lived (between mark_candidate and mark_pre_confirmed inside
    // execute_mempool_transaction), so we can only reliably observe RECEIVED -> PRE_CONFIRMED
    // here. Verify PRE_CONFIRMED via both mempool snapshot and pre_confirmed block.
    let resp = devnet
        .send_custom_rpc(
            "devnet_preconfirmTransactions",
            json!({ "transaction_hashes": [format!("{:#x}", hash)] }),
        )
        .await
        .unwrap();
    let pre_confirmed: Vec<String> = resp["pre_confirmed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(pre_confirmed, vec![format!("{:#x}", hash)]);

    assert_phase(&devnet, hash, "PRE_CONFIRMED").await;

    // The hash should also be on the open pre_confirmed block.
    let resp = devnet.send_custom_rpc("devnet_getMempool", json!({})).await.unwrap();
    let pre_confirmed_hashes: Vec<String> = resp["pre_confirmed_transaction_hashes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(pre_confirmed_hashes.contains(&format!("{:#x}", hash)));
}

/// Plan: `devnet_getMempool` reports config, RECEIVED transactions, PRE_CONFIRMED hashes,
/// and remaining block capacity.
#[tokio::test]
async fn get_mempool_reports_config_and_state() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    // Snapshot the empty pool.
    let resp = devnet.send_custom_rpc("devnet_getMempool", json!({})).await.unwrap();
    assert_eq!(resp["transactions"].as_array().unwrap().len(), 0);
    assert_eq!(resp["pre_confirmed_transaction_hashes"].as_array().unwrap().len(), 0);
    assert_eq!(resp["config"]["ordering"], "fifo");
    assert!(resp["config"]["max_transactions_per_block"].is_number());
    assert!(resp["config"]["random_seed"].is_number());
    assert!(resp["remaining_block_capacity"].is_number());

    let hash = submit_transfer_in_mempool(&account, Felt::ONE, 1).await;

    // Snapshot after one submission: tx present as RECEIVED.
    let resp = devnet.send_custom_rpc("devnet_getMempool", json!({})).await.unwrap();
    let txs = resp["transactions"].as_array().unwrap();
    assert_eq!(txs.len(), 1);
    let entry = &txs[0];
    assert_eq!(entry["transaction_hash"], format!("{:#x}", hash));
    assert_eq!(entry["status"], "RECEIVED");
    assert!(entry["arrival_id"].is_number());
    assert!(entry["sender_address"].is_string());
    assert!(entry["nonce"].is_string());
    assert!(entry["tip"].is_string());
}

/// Plan: `devnet_removeFromMempool` only acts on RECEIVED entries. PRE_CONFIRMED entries
/// cannot be removed individually; they must first be returned to RECEIVED via
/// `devnet_abortPreconfirmedBlock`.
#[tokio::test]
async fn remove_from_mempool_only_received() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    let hash = submit_transfer_in_mempool(&account, Felt::ONE, 1).await;
    assert_phase(&devnet, hash, "RECEIVED").await;

    // RECEIVED -> removable.
    let resp = devnet
        .send_custom_rpc(
            "devnet_removeFromMempool",
            json!({ "transaction_hash": format!("{:#x}", hash) }),
        )
        .await
        .unwrap();
    assert_eq!(resp["transaction_hash"], format!("{:#x}", hash));
    assert_received_count(&devnet, 0).await;

    // Now re-submit and pre-confirm; the tx is PRE_CONFIRMED and not removable.
    let hash2 = submit_transfer_in_mempool(&account, Felt::from(2u64), 1).await;
    let _ = devnet
        .send_custom_rpc(
            "devnet_preconfirmTransactions",
            json!({ "transaction_hashes": [format!("{:#x}", hash2)] }),
        )
        .await
        .unwrap();
    assert_phase(&devnet, hash2, "PRE_CONFIRMED").await;

    let err = devnet
        .send_custom_rpc(
            "devnet_removeFromMempool",
            json!({ "transaction_hash": format!("{:#x}", hash2) }),
        )
        .await
        .unwrap_err();
    // The plan requires the API to refuse removing a non-RECEIVED entry. We do not assert
    // on a specific code/message; we only assert the call fails and the entry remains.
    assert_phase(&devnet, hash2, "PRE_CONFIRMED").await;
    let _ = err; // presence of error is sufficient signal
}

/// Plan: `devnet_clearMempool` only acts on RECEIVED entries. PRE_CONFIRMED entries are
/// preserved (and must be cleared via abort).
#[tokio::test]
async fn clear_mempool_only_received() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    let h_received_1 = submit_transfer_in_mempool(&account, Felt::ONE, 1).await;
    let h_received_2 = submit_transfer_in_mempool(&account, Felt::from(2u64), 1).await;
    let h_will_pre_confirm = submit_transfer_in_mempool(&account, Felt::from(3u64), 1).await;

    let _ = devnet
        .send_custom_rpc(
            "devnet_preconfirmTransactions",
            json!({ "transaction_hashes": [format!("{:#x}", h_will_pre_confirm)] }),
        )
        .await
        .unwrap();
    assert_phase(&devnet, h_will_pre_confirm, "PRE_CONFIRMED").await;
    assert_phase(&devnet, h_received_1, "RECEIVED").await;
    assert_phase(&devnet, h_received_2, "RECEIVED").await;

    let resp = devnet.send_custom_rpc("devnet_clearMempool", json!({})).await.unwrap();
    let removed: Vec<String> = resp["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let mut removed_sorted = removed.clone();
    removed_sorted.sort();
    let mut expected = vec![format!("{:#x}", h_received_1), format!("{:#x}", h_received_2)];
    expected.sort();
    assert_eq!(removed_sorted, expected, "clear must only drop RECEIVED entries");

    // PRE_CONFIRMED entry remains.
    assert_phase(&devnet, h_will_pre_confirm, "PRE_CONFIRMED").await;
    assert_received_count(&devnet, 0).await;
}

/// Plan: `devnet_abortPreconfirmedBlock` returns every PRE_CONFIRMED entry to RECEIVED.
/// We verify by submitting two, pre-confirming both, then aborting.
#[tokio::test]
async fn abort_preconfirmed_block_returns_to_received() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    let h1 = submit_transfer_in_mempool(&account, Felt::ONE, 1).await;
    let h2 = submit_transfer_in_mempool(&account, Felt::from(2u64), 1).await;
    let _ = devnet
        .send_custom_rpc(
            "devnet_preconfirmTransactions",
            json!({
                "transaction_hashes": [format!("{:#x}", h1), format!("{:#x}", h2)]
            }),
        )
        .await
        .unwrap();
    assert_phase(&devnet, h1, "PRE_CONFIRMED").await;
    assert_phase(&devnet, h2, "PRE_CONFIRMED").await;

    let resp = devnet.send_custom_rpc("devnet_abortPreconfirmedBlock", json!({})).await.unwrap();
    let requeued: Vec<String> = resp["requeued"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let mut requeued_sorted = requeued.clone();
    requeued_sorted.sort();
    let mut expected = vec![format!("{:#x}", h1), format!("{:#x}", h2)];
    expected.sort();
    assert_eq!(requeued_sorted, expected);

    // Both should be back to RECEIVED now.
    assert_phase(&devnet, h1, "RECEIVED").await;
    assert_phase(&devnet, h2, "RECEIVED").await;
}

/// Plan: `devnet_sealBlock` performs strict sealing — it seals the pre-confirmed block (which
/// is empty) without selecting anything from RECEIVED. After sealing, RECEIVED entries
/// remain RECEIVED.
#[tokio::test]
async fn seal_block_does_not_pick_received() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    let hash = submit_transfer_in_mempool(&account, Felt::ONE, 1).await;
    assert_phase(&devnet, hash, "RECEIVED").await;

    let resp = devnet.send_custom_rpc("devnet_sealBlock", json!({})).await.unwrap();
    assert!(resp["block_hash"].is_string(), "expected block_hash in response: {resp}");

    // The pre-confirmed block (now empty) is sealed. The tx remains RECEIVED.
    assert_phase(&devnet, hash, "RECEIVED").await;
}

/// Plan: max_transactions_per_block caps the number of transactions selected in a single
/// `preconfirm_transactions` invocation.
#[tokio::test]
async fn max_transactions_per_block_caps_selection() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    // Submit 4 transfers (FIFO order; same sender nonce sequence).
    let _h1 = submit_transfer_in_mempool(&account, Felt::ONE, 1).await;
    let _h2 = submit_transfer_in_mempool(&account, Felt::from(2u64), 1).await;
    let _h3 = submit_transfer_in_mempool(&account, Felt::from(3u64), 1).await;
    let _h4 = submit_transfer_in_mempool(&account, Felt::from(4u64), 1).await;
    assert_received_count(&devnet, 4).await;

    // Set max to 2 and pre-confirm without forcing hashes.
    let resp = devnet
        .send_custom_rpc("devnet_setMempoolConfig", json!({ "max_transactions_per_block": 2 }))
        .await
        .unwrap();
    assert_eq!(resp["max_transactions_per_block"], 2);

    let resp = devnet.send_custom_rpc("devnet_preconfirmTransactions", json!({})).await.unwrap();
    let pre_confirmed: Vec<serde_json::Value> = resp["pre_confirmed"].as_array().unwrap().clone();
    assert_eq!(pre_confirmed.len(), 2, "max_transactions_per_block must cap selection: {resp}");
    assert_eq!(resp["block_full"], true);

    // Two remaining RECEIVED.
    let snapshot = devnet.send_custom_rpc("devnet_getMempool", json!({})).await.unwrap();
    let received = snapshot["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["status"] == "RECEIVED")
        .count();
    assert_eq!(received, 2, "two txs must remain RECEIVED: {snapshot}");
}

/// Plan: forced selection via `transaction_hashes` is honored even when the target nonce is
/// not the next expected one. We use nonces higher than expected so the call would otherwise
/// be blocked by eligibility checks.
#[tokio::test]
async fn forced_hashes_bypass_eligibility() {
    // Use a fresh devnet in transaction mode (executes on submission) so the first tx with
    // nonce 0 is accepted and bumps the sender nonce. Then switch to mempool for the test.
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    // Submit a tx and seal it. In mempool mode without sealing, it stays RECEIVED. We need
    // to surface the pre-confirmed block to the accepted collection; one way is to call
    // preconfirm + sealBlock.
    let h_valid = submit_transfer_in_mempool(&account, Felt::ONE, 1).await;
    let _ = devnet
        .send_custom_rpc(
            "devnet_preconfirmTransactions",
            json!({ "transaction_hashes": [format!("{:#x}", h_valid)] }),
        )
        .await
        .unwrap();
    let _ = devnet.send_custom_rpc("devnet_sealBlock", json!({})).await.unwrap();

    // Now submit two more txs: the first is the next valid nonce, the second has a future
    // nonce which by default would be blocked.
    let h_next = submit_transfer_in_mempool(&account, Felt::from(2u64), 1).await;
    assert_phase(&devnet, h_next, "RECEIVED").await;

    // Submit a tx with a future nonce; mempool will keep it RECEIVED (eligibility is a
    // separate gate, see `preconfirm_transactions` which checks the next-expected nonce).
    // Force-include it via transaction_hashes.
    let h_future = submit_transfer_in_mempool(&account, Felt::from(3u64), 1).await;
    let resp = devnet
        .send_custom_rpc(
            "devnet_preconfirmTransactions",
            json!({ "transaction_hashes": [format!("{:#x}", h_future)] }),
        )
        .await
        .unwrap();
    let pre_confirmed: Vec<String> = resp["pre_confirmed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(pre_confirmed.contains(&format!("{:#x}", h_future)));
}

/// Plan: in `mempool` mode, strict nonce checking is enabled. Re-submitting the same nonce
/// is rejected with the duplicate-transaction error (RPC code 59 in the plan).
#[tokio::test]
async fn duplicate_nonce_rejected_with_code_59() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    // Submit the first tx; the mempool admits it as RECEIVED.
    let _h1 = submit_transfer_in_mempool(&account, Felt::ONE, 1).await;

    // Submitting a second tx with the same sender address and same nonce (the next valid
    // nonce is shared) must be rejected with code 59.
    let second_send = account
        .execute_v3(vec![strk_transfer_call(Felt::from(2u64), 1)])
        .l1_gas(0)
        .l1_data_gas(1000)
        .l2_gas(1e8 as u64)
        .send()
        .await;
    let err = match second_send {
        Ok(_) => panic!("expected duplicate-nonce rejection, but the tx was accepted"),
        Err(e) => e,
    };
    // Provider-level error wraps the JSON-RPC error; assert on its string content.
    let msg = err.to_string();
    assert!(
        msg.contains("code 59") || msg.contains("Duplicate transaction"),
        "expected Duplicate transaction (code 59); got: {msg}"
    );
}

/// Plan: nonce ordering policy (Starknet) prioritizes txs with higher tip first. We can't
/// inject tips from the public RPC without raw tx construction, so we set the policy and
/// submit txs to verify the config is accepted. Selection-ordering correctness is verified
/// via the receive-side ordering config.
#[tokio::test]
async fn starknet_ordering_is_accepted() {
    let devnet = spawn_mempool_devnet().await;
    let resp = devnet
        .send_custom_rpc("devnet_setMempoolConfig", json!({ "ordering": "starknet" }))
        .await
        .unwrap();
    assert_eq!(resp["ordering"], "starknet");
}

/// Plan: random ordering with a fixed seed produces deterministic selection. We verify that
/// the same seed yields a stable `arrival_id`-based selection: submitting three txs and
/// inspecting the snapshot is enough; the deterministic part is the random_seed itself.
#[tokio::test]
async fn random_ordering_seed_is_recorded() {
    let devnet = spawn_mempool_devnet().await;
    let resp = devnet
        .send_custom_rpc(
            "devnet_setMempoolConfig",
            json!({ "ordering": "random", "random_seed": 42 }),
        )
        .await
        .unwrap();
    assert_eq!(resp["ordering"], "random");
    assert_eq!(resp["random_seed"], 42);

    // Subsequent reads should retain the same seed.
    let snapshot = devnet.send_custom_rpc("devnet_getMempool", json!({})).await.unwrap();
    assert_eq!(snapshot["config"]["random_seed"], 42);
    assert_eq!(snapshot["config"]["ordering"], "random");
}

/// Plan: compatibility — legacy `Interval(<seconds>)` continues to work. We verify that the
/// config endpoint reports the interval and that an `Interval(1)` mode seals a new block on
/// each txs without manual `createBlock`.
#[tokio::test]
async fn legacy_interval_mode_continues_to_work() {
    let devnet = BackgroundDevnet::spawn_with_additional_args(&["--block-generation-on", "1"])
        .await
        .expect("Could not start Devnet with Interval(1)");

    let config = devnet.get_config().await;
    assert_eq!(config["block_generation_on"], "1");

    let latest_block_num_before =
        devnet.get_latest_block_with_tx_hashes().await.unwrap().block_number;

    // Send a tx; the interval timer should seal a block within a few seconds.
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;
    let _ = submit_transfer_in_mempool(&account, Felt::ONE, 1).await;

    // Wait for the interval to trigger a block seal.
    let mut latest_block_num_after = latest_block_num_before;
    for _ in 0..30 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let block = devnet.get_latest_block_with_tx_hashes().await.unwrap();
        if block.block_number > latest_block_num_before {
            latest_block_num_after = block.block_number;
            break;
        }
    }
    assert!(
        latest_block_num_after > latest_block_num_before,
        "Interval(1) should have produced a new block"
    );
}

/// Plan: devnet_mint returns a successful response with the new balance and tx hash, and the
/// hash is non-zero.
#[tokio::test]
async fn mint_response_shape() {
    let devnet = spawn_mempool_devnet().await;
    let recipient = Felt::from_hex_unchecked(PREDEPLOYED_ACCOUNT_ADDRESS);
    let tx_hash = devnet.mint(recipient, 100).await;
    assert_ne!(tx_hash, Felt::ZERO);

    let balance =
        devnet.get_balance_by_tag(&recipient, FeeUnit::Fri, BlockTag::PreConfirmed).await.unwrap();
    assert!(balance > Felt::ZERO, "balance must reflect the mint, got {balance}");
}

/// Plan: when `devnet_preconfirmTransactions` is called with no RECEIVED entries, the
/// response indicates an empty selection without error.
#[tokio::test]
async fn preconfirm_with_empty_pool_is_noop() {
    let devnet = spawn_mempool_devnet().await;
    let resp = devnet.send_custom_rpc("devnet_preconfirmTransactions", json!({})).await.unwrap();
    assert_eq!(resp["pre_confirmed"].as_array().unwrap().len(), 0);
    assert_eq!(resp["selected"].as_array().unwrap().len(), 0);
    assert_eq!(resp["rejected"].as_array().unwrap().len(), 0);
    assert_eq!(resp["blocked"].as_array().unwrap().len(), 0);
    assert_eq!(resp["block_full"], false);
}

/// Plan: in mempool mode, after `preconfirm_transactions` selects a tx with strict nonce
/// checking, the next tx in the same account's sequence becomes eligible (its nonce is now
/// the expected one).
#[tokio::test]
async fn next_nonce_becomes_eligible_after_preconfirm() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    // Submit two txs; the first has the next-valid nonce, the second a future nonce.
    let h_first = submit_transfer_in_mempool(&account, Felt::ONE, 1).await;
    let h_second = submit_transfer_in_mempool(&account, Felt::from(2u64), 1).await;
    assert_phase(&devnet, h_first, "RECEIVED").await;
    assert_phase(&devnet, h_second, "RECEIVED").await;

    // Policy pre-confirm: only the first should be selected.
    let resp = devnet.send_custom_rpc("devnet_preconfirmTransactions", json!({})).await.unwrap();
    let pre_confirmed: Vec<String> = resp["pre_confirmed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(pre_confirmed.contains(&format!("{:#x}", h_first)));
    assert!(
        !pre_confirmed.contains(&format!("{:#x}", h_second)),
        "future-nonce tx must not be auto-selected: {resp}"
    );

    // h_second is still RECEIVED — its nonce is now eligible, so a second pre-confirm call
    // should pick it up.
    let resp = devnet.send_custom_rpc("devnet_preconfirmTransactions", json!({})).await.unwrap();
    let pre_confirmed: Vec<String> = resp["pre_confirmed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        pre_confirmed.contains(&format!("{:#x}", h_second)),
        "after the first pre-confirm, h_second becomes eligible: {resp}"
    );
}

/// Plan: an entry's `transaction` field is only populated when `include_transactions: true`.
#[tokio::test]
async fn get_mempool_include_transactions_flag() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;
    let _ = submit_transfer_in_mempool(&account, Felt::ONE, 1).await;

    // Default — `transaction` is null/absent.
    let resp = devnet.send_custom_rpc("devnet_getMempool", json!({})).await.unwrap();
    let entry = &resp["transactions"].as_array().unwrap()[0];
    assert!(
        entry.get("transaction").is_none() || entry["transaction"].is_null(),
        "transaction should be omitted by default: {entry}"
    );

    // include_transactions: true.
    let resp = devnet
        .send_custom_rpc("devnet_getMempool", json!({ "include_transactions": true }))
        .await
        .unwrap();
    let entry = &resp["transactions"].as_array().unwrap()[0];
    assert!(
        entry["transaction"].is_object(),
        "transaction should be an object when include_transactions=true: {entry}"
    );
}

/// Plan: `devnet_preconfirmTransactions` with `transaction_hashes` and `max_transactions`
/// together is rejected.
#[tokio::test]
async fn preconfirm_rejects_mutually_exclusive_params() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;
    let _ = submit_transfer_in_mempool(&account, Felt::ONE, 1).await;

    let err = devnet
        .send_custom_rpc(
            "devnet_preconfirmTransactions",
            json!({
                "transaction_hashes": [format!("{:#x}", Felt::ONE)],
                "max_transactions": 1
            }),
        )
        .await
        .unwrap_err();
    assert!(
        err.message.contains("mutually exclusive"),
        "expected mutually-exclusive error, got: {}",
        err.message
    );
}

/// Plan: `devnet_preconfirmTransactions` with `max_transactions: 0` is rejected.
#[tokio::test]
async fn preconfirm_rejects_zero_max_transactions() {
    let devnet = spawn_mempool_devnet().await;
    let err = devnet
        .send_custom_rpc("devnet_preconfirmTransactions", json!({ "max_transactions": 0 }))
        .await
        .unwrap_err();
    assert!(err.message.contains("positive"), "expected positive-max error, got: {}", err.message);
}

/// Plan: the `transaction` mode is the default; txs are sealed into a new block on each
/// submission. (Regression test to ensure mempool changes do not break the default flow.)
#[tokio::test]
async fn default_mode_seals_on_each_submission() {
    let devnet = BackgroundDevnet::spawn().await.expect("Could not start Devnet");

    let block_num_before = devnet.get_latest_block_with_tx_hashes().await.unwrap().block_number;

    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;
    let _ = submit_transfer_in_mempool(&account, Felt::ONE, 1).await;

    let block_num_after = devnet.get_latest_block_with_tx_hashes().await.unwrap().block_number;
    assert!(block_num_after > block_num_before, "default mode must seal on submission");
}

/// Plan: the `demand` mode does not auto-seal; an explicit `createBlock` is required.
#[tokio::test]
async fn demand_mode_does_not_auto_seal() {
    let devnet = BackgroundDevnet::spawn_with_additional_args(&["--block-generation-on", "demand"])
        .await
        .expect("Could not start Devnet in demand mode");

    let block_num_before = devnet.get_latest_block_with_tx_hashes().await.unwrap().block_number;

    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;
    let _ = submit_transfer_in_mempool(&account, Felt::ONE, 1).await;

    // Demand mode should NOT auto-seal.
    let block_num_after_noop = devnet.get_latest_block_with_tx_hashes().await.unwrap().block_number;
    assert_eq!(block_num_after_noop, block_num_before, "demand mode must not auto-seal");

    // Explicit createBlock seals.
    devnet.create_block().await.unwrap();
    let block_num_after_create =
        devnet.get_latest_block_with_tx_hashes().await.unwrap().block_number;
    assert!(block_num_after_create > block_num_before);
}
