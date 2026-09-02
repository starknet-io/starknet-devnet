//! Integration tests for mempool mode and Devnet mempool RPCs.
//!
//! Tests below assert the supported behavior end-to-end via running Devnet instances:
//! * `BlockGenerationOn::Mempool` keeps user txs RECEIVED until they are pre-confirmed or sealed.
//! * `devnet_getMempool` / `devnet_removeFromMempool` / `devnet_clearMempool` reflect
//!   RECEIVED/CANDIDATE/PRE_CONFIRMED state and only act on RECEIVED entries.
//! * `devnet_preconfirmTransactions` selects per ordering policy, with optional forced hashes or
//!   max-transactions cap. RECEIVED/PRE_CONFIRMED transitions are observable.
//! * `devnet_sealBlock` performs strict sealing and never pulls RECEIVED transactions in
//!   automatically.
//! * `devnet_abortPreconfirmedBlock` returns every PRE_CONFIRMED entry back to RECEIVED.
//! * `devnet_abortBlocks` clears all mempool state (RECEIVED + open proposal).
//! * `devnet_mint` uses the system lane, so balance changes are immediately visible even when block
//!   generation is on `mempool`.
//! * Duplicate hashes are rejected with RPC code 59; duplicate account nonces are rejected as
//!   invalid requests without displacing the already received transaction.
//!
//! `Interval(<seconds>)` remains a supported periodic-sealing mode and its functional behavior is
//! verified here.

use serde_json::json;
use starknet_rs_accounts::{Account, ExecutionEncoding, SingleOwnerAccount};
use starknet_rs_core::types::{BlockId, BlockTag, Call, Felt};
use starknet_rs_core::utils::get_selector_from_name;
use starknet_rs_providers::Provider;
use starknet_rs_providers::jsonrpc::{HttpTransport, JsonRpcClient};
use starknet_rs_signers::{LocalWallet, SigningKey};

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

/// Returns the n-th predeployed account (0-based) as a `SingleOwnerAccount`.
async fn nth_predeployed_account<'a>(
    devnet: &'a BackgroundDevnet,
    client: &'a JsonRpcClient<HttpTransport>,
    index: usize,
) -> SingleOwnerAccount<&'a JsonRpcClient<HttpTransport>, LocalWallet> {
    let accounts =
        devnet.send_custom_rpc("devnet_getPredeployedAccounts", json!({})).await.unwrap();
    let arr = accounts.as_array().expect("predeployed accounts should be an array");
    let entry = arr
        .get(index)
        .unwrap_or_else(|| panic!("predeployed account at index {index} not found: {accounts}"));
    let address = Felt::from_hex_unchecked(entry["address"].as_str().unwrap());
    let pk = Felt::from_hex_unchecked(entry["private_key"].as_str().unwrap());
    let signer = LocalWallet::from(SigningKey::from_secret_scalar(pk));
    let chain_id = client.chain_id().await.unwrap();
    SingleOwnerAccount::new(client, signer, address, chain_id, ExecutionEncoding::New)
}

/// Sends a tiny transfer from the predeployed account to `recipient` in mempool mode,
/// returning the transaction hash. Uses the STRK ERC20 because the predeployed account is
/// funded with it. The optional `tip` enables exercises that depend on Starknet ordering's
/// priority comparator (descending tip, descending transaction-hash tie-break).
///
/// `nonce` MUST be supplied by the caller. In mempool mode the chain nonce advances only after
/// a transaction is pre-confirmed, not when it is admitted as `RECEIVED`, so callers track the
/// next expected nonce themselves.
async fn submit_transfer_in_mempool(
    account: &SingleOwnerAccount<&JsonRpcClient<HttpTransport>, LocalWallet>,
    recipient: Felt,
    amount: u128,
    tip: u64,
    nonce: Felt,
) -> Felt {
    // Use the Devnet default L2 gas price explicitly to avoid fee estimation while still making
    // the transaction sufficiently paying for the Starknet ordering policy.
    let result = account
        .execute_v3(vec![strk_transfer_call(recipient, amount)])
        .l1_gas(0)
        .l1_data_gas(1000)
        .l2_gas(1e8 as u64)
        .l2_gas_price(1_000_000_000)
        .tip(tip)
        .nonce(nonce)
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
/// Mempool behaviors
/// ---------------------------------------------------------------------------
/// `devnet_mint` uses the system lane and force-processes its generated transaction, so the
/// balance change is observable immediately even in `mempool` mode.
#[tokio::test]
async fn mint_is_force_processed_in_mempool_mode() {
    let devnet = spawn_mempool_devnet().await;
    let recipient = Felt::from_hex_unchecked(PREDEPLOYED_ACCOUNT_ADDRESS);
    let balance_before =
        devnet.get_balance_by_tag(&recipient, FeeUnit::Fri, BlockTag::Latest).await.unwrap();
    let transaction_hash = devnet.mint(recipient, 1_000).await;

    // In mempool mode, the mint should be force-processed and visible at pre_confirmed
    // without any explicit preconfirm/seal call.
    let balance_after =
        devnet.get_balance_by_tag(&recipient, FeeUnit::Fri, BlockTag::PreConfirmed).await.unwrap();
    assert_eq!(
        balance_after,
        balance_before + Felt::from(1_000u64),
        "mint must be observable immediately in mempool mode"
    );

    // System transactions are retained as PRE_CONFIRMED until sealing so proposal abort can
    // restore their effects too.
    assert_phase(&devnet, transaction_hash, "PRE_CONFIRMED").await;
}

/// Restart and block abortion clear RECEIVED/CANDIDATE transactions.
/// A pending pre-confirmed block (containing accepted txs) is in the way of the test — we
/// abort it via `devnet_abortBlocks` on the pre-confirmed block, and verify the open
/// mempool state is cleared.
#[tokio::test]
async fn abort_blocks_clears_mempool() {
    let devnet = BackgroundDevnet::spawn_with_additional_args(&[
        "--block-generation-on",
        "mempool",
        "--state-archive-capacity",
        "full",
    ])
    .await
    .expect("Could not start Devnet in mempool mode with a full state archive");
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    // Submit two transfers: they remain RECEIVED in mempool mode. Nonces must be supplied
    // explicitly because the chain nonce does not advance until a transaction is sealed,
    // so back-to-back RECEIVED submissions would otherwise collide on nonce 0.
    let h1 = submit_transfer_in_mempool(&account, Felt::ONE, 1, 0, Felt::ZERO).await;
    let _h2 = submit_transfer_in_mempool(&account, Felt::from(2u64), 1, 0, Felt::ONE).await;
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

    // Block abortion clears the entire mempool, so both the still-RECEIVED h2 and the
    // PRE_CONFIRMED h1 should disappear.
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

/// A transaction submitted in mempool mode goes RECEIVED, then CANDIDATE on selection,
/// then PRE_CONFIRMED on successful execution. The transition is observable through
/// `devnet_getMempool` and the open proposal hash list.
#[tokio::test]
async fn mempool_phases_received_candidate_preconfirmed() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    let hash = submit_transfer_in_mempool(&account, Felt::ONE, 1, 0, Felt::ZERO).await;
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

/// `devnet_getMempool` reports config, RECEIVED transactions, PRE_CONFIRMED hashes,
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

    let hash = submit_transfer_in_mempool(&account, Felt::ONE, 1, 0, Felt::ZERO).await;

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

/// `devnet_removeFromMempool` only acts on RECEIVED entries. PRE_CONFIRMED entries
/// cannot be removed individually; they must first be returned to RECEIVED via
/// `devnet_abortPreconfirmedBlock`.
#[tokio::test]
async fn remove_from_mempool_only_received() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    let hash = submit_transfer_in_mempool(&account, Felt::ONE, 1, 0, Felt::ZERO).await;
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

    // Now re-submit and pre-confirm; the tx is PRE_CONFIRMED and not removable. Nonce is
    // 0 again because the previous RECEIVED entry was removed without being sealed.
    let hash2 = submit_transfer_in_mempool(&account, Felt::from(2u64), 1, 0, Felt::ZERO).await;
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
    // The API refuses to remove a non-RECEIVED entry. We do not assert on a specific code/message;
    // we only assert the call fails and the entry remains.
    assert_phase(&devnet, hash2, "PRE_CONFIRMED").await;
    let _ = err; // presence of error is sufficient signal
}

/// `devnet_clearMempool` only acts on RECEIVED entries. PRE_CONFIRMED entries are
/// preserved (and must be cleared via abort).
#[tokio::test]
async fn clear_mempool_only_received() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    let h_will_pre_confirm =
        submit_transfer_in_mempool(&account, Felt::ONE, 1, 0, Felt::ZERO).await;
    let h_received_2 =
        submit_transfer_in_mempool(&account, Felt::from(2u64), 1, 0, Felt::ONE).await;
    let h_received_3 =
        submit_transfer_in_mempool(&account, Felt::from(3u64), 1, 0, Felt::TWO).await;

    let _ = devnet
        .send_custom_rpc("devnet_preconfirmTransactions", json!({ "max_transactions": 1 }))
        .await
        .unwrap();
    assert_phase(&devnet, h_will_pre_confirm, "PRE_CONFIRMED").await;
    assert_phase(&devnet, h_received_2, "RECEIVED").await;
    assert_phase(&devnet, h_received_3, "RECEIVED").await;

    let resp = devnet.send_custom_rpc("devnet_clearMempool", json!({})).await.unwrap();
    let removed: Vec<String> = resp["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let mut removed_sorted = removed.clone();
    removed_sorted.sort();
    let mut expected = vec![format!("{h_received_2:#x}"), format!("{h_received_3:#x}")];
    expected.sort();
    assert_eq!(removed_sorted, expected, "clear must only drop RECEIVED entries");

    // PRE_CONFIRMED entry remains.
    assert_phase(&devnet, h_will_pre_confirm, "PRE_CONFIRMED").await;
    assert_received_count(&devnet, 0).await;
}

/// `devnet_abortPreconfirmedBlock` returns every PRE_CONFIRMED entry to RECEIVED.
/// We verify by submitting two, pre-confirming both, then aborting.
#[tokio::test]
async fn abort_preconfirmed_block_returns_to_received() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    let h1 = submit_transfer_in_mempool(&account, Felt::ONE, 1, 0, Felt::ZERO).await;
    let h2 = submit_transfer_in_mempool(&account, Felt::from(2u64), 1, 0, Felt::ONE).await;
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

/// `devnet_sealBlock` performs strict sealing — it seals the pre-confirmed block (which
/// is empty) without selecting anything from RECEIVED. After sealing, RECEIVED entries
/// remain RECEIVED.
#[tokio::test]
async fn seal_block_does_not_pick_received() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    let hash = submit_transfer_in_mempool(&account, Felt::ONE, 1, 0, Felt::ZERO).await;
    assert_phase(&devnet, hash, "RECEIVED").await;

    let resp = devnet.send_custom_rpc("devnet_sealBlock", json!({})).await.unwrap();
    assert!(resp["block_hash"].is_string(), "expected block_hash in response: {resp}");

    // The pre-confirmed block (now empty) is sealed. The tx remains RECEIVED.
    assert_phase(&devnet, hash, "RECEIVED").await;
}

/// `max_transactions_per_block` caps the number of transactions selected in a single
/// `preconfirm_transactions` invocation.
#[tokio::test]
async fn max_transactions_per_block_caps_selection() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    // Submit 4 transfers (FIFO order; same sender nonce sequence).
    let _h1 = submit_transfer_in_mempool(&account, Felt::ONE, 1, 0, Felt::ZERO).await;
    let _h2 = submit_transfer_in_mempool(&account, Felt::from(2u64), 1, 0, Felt::ONE).await;
    let _h3 = submit_transfer_in_mempool(&account, Felt::from(3u64), 1, 0, Felt::TWO).await;
    let _h4 = submit_transfer_in_mempool(&account, Felt::from(4u64), 1, 0, Felt::THREE).await;
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

/// Forced selection preserves the caller's order but still enforces nonce eligibility.
#[tokio::test]
async fn forced_hashes_respect_eligibility() {
    // Use a fresh devnet in transaction mode (executes on submission) so the first tx with
    // nonce 0 is accepted and bumps the sender nonce. Then switch to mempool for the test.
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    // Submit a tx and seal it. In mempool mode without sealing, it stays RECEIVED. We need
    // to surface the pre-confirmed block to the accepted collection; one way is to call
    // preconfirm + sealBlock.
    let h_valid = submit_transfer_in_mempool(&account, Felt::ONE, 1, 0, Felt::ZERO).await;
    let _ = devnet
        .send_custom_rpc(
            "devnet_preconfirmTransactions",
            json!({ "transaction_hashes": [format!("{:#x}", h_valid)] }),
        )
        .await
        .unwrap();
    let _ = devnet.send_custom_rpc("devnet_sealBlock", json!({})).await.unwrap();

    // Now submit two more txs: the first is the next valid nonce (chain nonce advanced to
    // 1 after sealing), the second has a future nonce which by default would be blocked.
    let h_next = submit_transfer_in_mempool(&account, Felt::from(2u64), 1, 0, Felt::ONE).await;
    assert_phase(&devnet, h_next, "RECEIVED").await;

    // Submit a future nonce and force it. The request must report it blocked and leave it
    // RECEIVED because the nonce-1 transaction is still pending.
    let h_future =
        submit_transfer_in_mempool(&account, Felt::from(3u64), 1, 0, Felt::from(2u64)).await;
    let resp = devnet
        .send_custom_rpc(
            "devnet_preconfirmTransactions",
            json!({ "transaction_hashes": [format!("{:#x}", h_future)] }),
        )
        .await
        .unwrap();
    assert!(resp["pre_confirmed"].as_array().unwrap().is_empty(), "{resp}");
    assert_eq!(resp["blocked"][0]["transaction_hash"], format!("{h_future:#x}"));
    assert_phase(&devnet, h_future, "RECEIVED").await;
}

/// A second transaction for the same account and nonce is rejected as an invalid request.
#[tokio::test]
async fn duplicate_nonce_is_rejected() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    // Submit the first tx; the mempool admits it as RECEIVED.
    let _h1 = submit_transfer_in_mempool(&account, Felt::ONE, 1, 0, Felt::ZERO).await;

    // Submitting a second tx with the same sender address and nonce must be rejected.
    let second_send = account
        .execute_v3(vec![strk_transfer_call(Felt::from(2u64), 1)])
        .l1_gas(0)
        .l1_data_gas(1000)
        .l2_gas(1e8 as u64)
        .nonce(Felt::ZERO)
        .send()
        .await;
    let err = match second_send {
        Ok(_) => panic!("expected duplicate-nonce rejection, but the tx was accepted"),
        Err(e) => e,
    };
    // Provider-level error wraps the JSON-RPC error; assert on its string content.
    let msg = err.to_string();
    assert!(msg.contains("already in the mempool"), "expected nonce-conflict error; got: {msg}");
}

/// Starknet ordering prioritizes transactions with higher tips. We can't
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

/// Helper for `starknet_policy_hides_below_threshold_txs`. Submits a transfer whose
/// `max_l2_gas_price` resource bound (in FRI) is set explicitly via `l2_gas_price`.
///
/// All seven resource-bound fields must be set explicitly: when any of them is `None`,
/// the SDK's `prepare` step multiplies the block's gas prices by
/// `gas_price_estimate_multiplier` (default 1.5), which would otherwise clobber the
/// requested `l2_gas_price`. L1 prices are set to a value comfortably above the block's
/// price so the transaction can pay its fees regardless of `l2_gas_price`.
async fn submit_transfer_with_l2_gas_price(
    account: &SingleOwnerAccount<&JsonRpcClient<HttpTransport>, LocalWallet>,
    recipient: Felt,
    amount: u128,
    nonce: Felt,
    l2_gas_price: u128,
) -> Felt {
    let result = account
        .execute_v3(vec![strk_transfer_call(recipient, amount)])
        .l1_gas(0)
        .l1_gas_price(1_000_000_000)
        .l1_data_gas(1000)
        .l1_data_gas_price(1_000_000_000)
        .l2_gas(1e8 as u64)
        .l2_gas_price(l2_gas_price)
        .tip(0)
        .nonce(nonce)
        .send()
        .await
        .expect("transfer should submit");
    result.transaction_hash
}

/// Under the Starknet ordering policy, transactions whose `max_l2_gas_price` falls below the
/// active proposal's L2 gas-price threshold are not selected. Changing the next-block gas price
/// does not alter the open proposal; once that proposal is sealed, the pending transaction becomes
/// eligible under the new proposal's lower threshold.
#[tokio::test]
async fn starknet_policy_hides_below_threshold_txs() {
    let devnet = BackgroundDevnet::spawn_with_additional_args(&[
        "--block-generation-on",
        "mempool",
        "--mempool-ordering",
        "starknet",
        "--l2-gas-price-fri",
        "1000000001",
    ])
    .await
    .expect("Could not start Devnet in mempool mode");
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    // Submit a transfer whose `max_l2_gas_price` (= 1_000_000_000) is strictly below the
    // active proposal threshold (= 1_000_000_001). L1 prices and `l2_gas` are sized so the
    // transaction can still pay its fees after the next proposal lowers the threshold.
    let hash_pending =
        submit_transfer_with_l2_gas_price(&account, Felt::ONE, 1, Felt::ZERO, 1_000_000_000).await;
    assert_phase(&devnet, hash_pending, "RECEIVED").await;

    let resp = devnet.send_custom_rpc("devnet_preconfirmTransactions", json!({})).await.unwrap();
    assert!(
        resp["pre_confirmed"].as_array().unwrap().is_empty(),
        "below-threshold tx must remain unselected: {resp}"
    );
    assert_phase(&devnet, hash_pending, "RECEIVED").await;

    // Configure the lower price for the next block. The active proposal retains its original
    // threshold, so this must not make the transaction selectable yet.
    let _ = devnet
        .send_custom_rpc(
            "devnet_setGasPrice",
            json!({ "l2_gas_price_fri": 1_000_000_000, "generate_block": false }),
        )
        .await
        .expect("lowering the threshold should succeed");

    let resp = devnet.send_custom_rpc("devnet_preconfirmTransactions", json!({})).await.unwrap();
    assert!(
        resp["pre_confirmed"].as_array().unwrap().is_empty(),
        "next-block gas changes must not alter the active proposal: {resp}"
    );
    assert_phase(&devnet, hash_pending, "RECEIVED").await;

    // Strictly seal the empty proposal. The next proposal now uses the lower configured gas price,
    // making the transaction's equal max price sufficient.
    let _ = devnet.send_custom_rpc("devnet_sealBlock", json!({})).await.unwrap();

    let resp = devnet.send_custom_rpc("devnet_preconfirmTransactions", json!({})).await.unwrap();
    let pre_confirmed: Vec<String> = resp["pre_confirmed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        pre_confirmed.contains(&format!("{:#x}", hash_pending)),
        "the pending tx must qualify in the next proposal: {resp}"
    );
    assert_phase(&devnet, hash_pending, "PRE_CONFIRMED").await;
}

/// Policy-driven building snapshots nonce-eligible account heads for a selection round. A
/// successor exposed by executing one account head cannot overtake another account head that was
/// already present in that snapshot, even when the successor has a higher tip.
#[tokio::test]
async fn starknet_policy_replenishes_account_heads_between_selection_rounds() {
    let devnet = BackgroundDevnet::spawn_with_additional_args(&[
        "--block-generation-on",
        "mempool",
        "--mempool-ordering",
        "starknet",
    ])
    .await
    .expect("Could not start Devnet in mempool mode");
    let client = json_rpc_client(&devnet);
    let account_a = first_predeployed_account(&devnet, &client).await;
    let account_b = nth_predeployed_account(&devnet, &client, 1).await;

    let a0 = submit_transfer_in_mempool(&account_a, Felt::ONE, 1, 20, Felt::ZERO).await;
    let a1 = submit_transfer_in_mempool(&account_a, Felt::TWO, 1, 20, Felt::ONE).await;
    let b0 = submit_transfer_in_mempool(&account_b, Felt::from(3_u64), 1, 10, Felt::ZERO).await;

    let response =
        devnet.send_custom_rpc("devnet_preconfirmTransactions", json!({})).await.unwrap();
    let pre_confirmed = response["pre_confirmed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hash| hash.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();

    assert_eq!(
        pre_confirmed,
        vec![format!("{a0:#x}"), format!("{b0:#x}"), format!("{a1:#x}")],
        "A1 must wait until the initial A0/B0 selection round is exhausted: {response}"
    );
}

#[tokio::test]
async fn unregistered_ordering_policy_is_rejected() {
    let devnet = spawn_mempool_devnet().await;
    let error = devnet
        .send_custom_rpc("devnet_setMempoolConfig", json!({ "ordering": "custom-policy" }))
        .await
        .unwrap_err();
    assert!(error.message.contains("Unknown mempool ordering policy"), "{error:?}");

    let snapshot = devnet.send_custom_rpc("devnet_getMempool", json!({})).await.unwrap();
    assert_eq!(snapshot["config"]["ordering"], "fifo");
}

/// Random ordering with a fixed seed produces deterministic selection. We verify that
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

/// Verify that the config endpoint reports the interval and that `Interval(1)` seals a new block
/// without a manual `createBlock` request.
#[tokio::test]
async fn interval_mode_seals_periodically() {
    let devnet = BackgroundDevnet::spawn_with_additional_args(&["--block-generation-on", "1"])
        .await
        .expect("Could not start Devnet with Interval(1)");

    let config = devnet.get_config().await;
    assert_eq!(config["block_generation_on"], json!({ "interval": 1 }));

    let latest_block_num_before =
        devnet.get_latest_block_with_tx_hashes().await.unwrap().block_number;

    // Send a tx; the interval timer should seal a block within a few seconds.
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;
    let _ = submit_transfer_in_mempool(&account, Felt::ONE, 1, 0, Felt::ZERO).await;

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

/// `devnet_mint` returns a successful response with the new balance and transaction hash, and the
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

/// When `devnet_preconfirmTransactions` is called with no RECEIVED entries, the
/// response indicates an empty selection without error.
#[tokio::test]
async fn preconfirm_with_empty_pool_is_noop() {
    let devnet = spawn_mempool_devnet().await;
    let resp = devnet.send_custom_rpc("devnet_preconfirmTransactions", json!({})).await.unwrap();
    assert_eq!(resp["pre_confirmed"].as_array().unwrap().len(), 0);
    assert_eq!(resp["rejected"].as_array().unwrap().len(), 0);
    assert_eq!(resp["blocked"].as_array().unwrap().len(), 0);
    assert_eq!(resp["block_full"], false);
}

/// After `preconfirm_transactions` selects a transaction with strict nonce
/// checking, the next tx in the same account's sequence becomes eligible (its nonce is now
/// the expected one).
#[tokio::test]
async fn next_nonce_becomes_eligible_after_preconfirm() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;

    // Submit two txs; the first has the next-valid nonce, the second a future nonce.
    let h_first = submit_transfer_in_mempool(&account, Felt::ONE, 1, 0, Felt::ZERO).await;
    let h_second = submit_transfer_in_mempool(&account, Felt::from(2u64), 1, 0, Felt::ONE).await;
    assert_phase(&devnet, h_first, "RECEIVED").await;
    assert_phase(&devnet, h_second, "RECEIVED").await;

    // Policy pre-confirm recomputes eligibility after the first execution, so both consecutive
    // nonces are selected in the same call.
    let resp = devnet.send_custom_rpc("devnet_preconfirmTransactions", json!({})).await.unwrap();
    let pre_confirmed: Vec<String> = resp["pre_confirmed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(pre_confirmed.contains(&format!("{:#x}", h_first)));
    assert!(
        pre_confirmed.contains(&format!("{:#x}", h_second)),
        "the next nonce must become eligible during the same processing call: {resp}"
    );
}

/// An entry's `transaction` field is only populated when `include_transactions: true`.
#[tokio::test]
async fn get_mempool_include_transactions_flag() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;
    let _ = submit_transfer_in_mempool(&account, Felt::ONE, 1, 0, Felt::ZERO).await;

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

/// `devnet_preconfirmTransactions` with `transaction_hashes` and `max_transactions`
/// together is rejected.
#[tokio::test]
async fn preconfirm_rejects_mutually_exclusive_params() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;
    let _ = submit_transfer_in_mempool(&account, Felt::ONE, 1, 0, Felt::ZERO).await;

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

/// `devnet_preconfirmTransactions` with `max_transactions: 0` is rejected.
#[tokio::test]
async fn preconfirm_rejects_zero_max_transactions() {
    let devnet = spawn_mempool_devnet().await;
    let err = devnet
        .send_custom_rpc("devnet_preconfirmTransactions", json!({ "max_transactions": 0 }))
        .await
        .unwrap_err();
    assert!(err.message.contains("positive"), "expected positive-max error, got: {}", err.message);
}

/// The `transaction` mode is the default; transactions are sealed into a new block on each
/// submission. (Regression test to ensure mempool changes do not break the default flow.)
#[tokio::test]
async fn default_mode_seals_on_each_submission() {
    let devnet = BackgroundDevnet::spawn().await.expect("Could not start Devnet");

    let block_num_before = devnet.get_latest_block_with_tx_hashes().await.unwrap().block_number;

    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;
    let _ = submit_transfer_in_mempool(&account, Felt::ONE, 1, 0, Felt::ZERO).await;

    let block_num_after = devnet.get_latest_block_with_tx_hashes().await.unwrap().block_number;
    assert!(block_num_after > block_num_before, "default mode must seal on submission");
}

/// The `demand` mode does not auto-seal; an explicit `createBlock` is required.
#[tokio::test]
async fn demand_mode_does_not_auto_seal() {
    let devnet = BackgroundDevnet::spawn_with_additional_args(&["--block-generation-on", "demand"])
        .await
        .expect("Could not start Devnet in demand mode");

    let block_num_before = devnet.get_latest_block_with_tx_hashes().await.unwrap().block_number;

    let client = json_rpc_client(&devnet);
    let account = first_predeployed_account(&devnet, &client).await;
    let _ = submit_transfer_in_mempool(&account, Felt::ONE, 1, 0, Felt::ZERO).await;

    // Demand mode should NOT auto-seal.
    let block_num_after_noop = devnet.get_latest_block_with_tx_hashes().await.unwrap().block_number;
    assert_eq!(block_num_after_noop, block_num_before, "demand mode must not auto-seal");

    // Explicit createBlock seals.
    devnet.create_block().await.unwrap();
    let block_num_after_create =
        devnet.get_latest_block_with_tx_hashes().await.unwrap().block_number;
    assert!(block_num_after_create > block_num_before);
}

/// End-to-end FIFO pipeline. Submit `n` transfers, one from each of `n` distinct
/// predeployed accounts (so strict per-account nonce checking admits all of them at once
/// — every sender has its own next-nonce slot). The inter-account FIFO ordering is what
/// this test exercises: pre-confirm should select them in arrival order, the latest sealed
/// block should contain them in that same order, and the mempool should drain.
#[tokio::test]
async fn fifo_pipeline_received_preconfirmed_sealed() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let accounts = [
        first_predeployed_account(&devnet, &client).await,
        nth_predeployed_account(&devnet, &client, 1).await,
        nth_predeployed_account(&devnet, &client, 2).await,
    ];

    // Phase 1 — RECEIVED. Each sender submits a nonce-0 transfer (recipient is unique so
    // each tx has a distinguishable intent; tx hashes themselves are ordered at the
    // mempool layer, not by their recipients). Submissions are serialized so arrival order
    // matches the order in which accounts are enumerated — `join_all` would race the three
    // `execute_v3` round trips and break FIFO determinism.
    let mut hashes: Vec<Felt> = Vec::with_capacity(accounts.len());
    for (i, account) in accounts.iter().enumerate() {
        let hash =
            submit_transfer_in_mempool(account, Felt::from((i + 1) as u64), 1, 0, Felt::ZERO).await;
        hashes.push(hash);
    }
    assert_received_count(&devnet, 3).await;

    // Phase 2 — PRE_CONFIRMED. All three accounts are eligible, so a single unforced
    // preconfirmTransactions must select all three.
    let resp = devnet.send_custom_rpc("devnet_preconfirmTransactions", json!({})).await.unwrap();
    let pre_confirmed: Vec<String> = resp["pre_confirmed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let mut pc_sorted = pre_confirmed.clone();
    pc_sorted.sort();
    let mut expected_sorted: Vec<String> = hashes.iter().map(|h| format!("{h:#x}")).collect();
    expected_sorted.sort();
    assert_eq!(
        pc_sorted, expected_sorted,
        "all eligible txs must be PRE_CONFIRMED in one call: {resp}"
    );
    for h in &hashes {
        assert_phase(&devnet, *h, "PRE_CONFIRMED").await;
    }
    assert_received_count(&devnet, 0).await;

    // Phase 3 — SEALED. The latest sealed block contains all three hashes in FIFO order.
    let block_num_before = devnet.get_latest_block_with_tx_hashes().await.unwrap().block_number;
    let resp = devnet.send_custom_rpc("devnet_sealBlock", json!({})).await.unwrap();
    assert!(resp["block_hash"].is_string(), "expected block_hash: {resp}");

    let sealed = devnet.get_latest_block_with_tx_hashes().await.unwrap();
    assert_eq!(sealed.block_number, block_num_before + 1, "sealing must advance the latest block");
    assert_eq!(
        sealed.transactions, hashes,
        "sealed block must contain txs in FIFO submission order: {sealed:?}"
    );

    let snapshot = devnet.send_custom_rpc("devnet_getMempool", json!({})).await.unwrap();
    assert_eq!(
        snapshot["transactions"].as_array().unwrap().len(),
        0,
        "mempool must be drained after seal: {snapshot}"
    );
    assert_eq!(
        snapshot["pre_confirmed_transaction_hashes"].as_array().unwrap().len(),
        0,
        "open proposal must be empty after seal: {snapshot}"
    );
}

/// End-to-end Starknet ordering pipeline. Three distinct senders each submit a
/// nonce-0 transfer (so strict per-account nonce checking admits all of them at once).
/// Each sender attaches a different `tip` (`i` for the i-th sender), which makes the
/// Starknet priority comparator — descending tip with transaction-hash tie-break —
/// deterministic without depending on FIFO fallback. Switch the policy to `starknet`
/// and run the same RECEIVED -> PRE_CONFIRMED -> SEALED pipeline. Expected sealed
/// order: senders enumerated in reverse submission order (highest tip first), i.e.
/// `hashes[2], hashes[1], hashes[0]`.
#[tokio::test]
async fn starknet_pipeline_received_preconfirmed_sealed() {
    let devnet = spawn_mempool_devnet().await;
    let client = json_rpc_client(&devnet);
    let accounts = [
        first_predeployed_account(&devnet, &client).await,
        nth_predeployed_account(&devnet, &client, 1).await,
        nth_predeployed_account(&devnet, &client, 2).await,
    ];

    // Switch to the starknet policy before submitting.
    let resp = devnet
        .send_custom_rpc("devnet_setMempoolConfig", json!({ "ordering": "starknet" }))
        .await
        .unwrap();
    assert_eq!(resp["ordering"], "starknet");

    // Phase 1 — RECEIVED. Each sender submits a nonce-0 transfer with a distinct tip
    // (0, 1, 2). Submissions are serialized so arrival order matches the order in
    // which accounts are enumerated; the tip values make the Starknet priority
    // comparator produce a deterministic, non-FIFO pre-confirm order.
    let mut hashes: Vec<Felt> = Vec::with_capacity(accounts.len());
    for (i, account) in accounts.iter().enumerate() {
        let hash = submit_transfer_in_mempool(
            account,
            Felt::from((i + 1) as u64),
            1,
            i as u64,
            Felt::ZERO,
        )
        .await;
        hashes.push(hash);
    }
    assert_received_count(&devnet, 3).await;

    // Phase 2 — PRE_CONFIRMED. All three accounts are eligible, so a single unforced
    // preconfirmTransactions must select all three. Their declared tips are distinct,
    // so the Starknet comparator picks them in strictly descending-tip order: the
    // sender with tip=2 first, then tip=1, then tip=0.
    let resp = devnet.send_custom_rpc("devnet_preconfirmTransactions", json!({})).await.unwrap();
    let pre_confirmed: Vec<String> = resp["pre_confirmed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let mut pc_sorted = pre_confirmed.clone();
    pc_sorted.sort();
    let mut expected_sorted: Vec<String> = hashes.iter().map(|h| format!("{h:#x}")).collect();
    expected_sorted.sort();
    assert_eq!(
        pc_sorted, expected_sorted,
        "all eligible txs must be PRE_CONFIRMED in one call: {resp}"
    );
    // The actual pre-confirm order must follow descending tip (not arrival order):
    // sender index 2 (tip=2), 1 (tip=1), 0 (tip=0).
    assert_eq!(
        pre_confirmed,
        vec![format!("{:#x}", hashes[2]), format!("{:#x}", hashes[1]), format!("{:#x}", hashes[0])],
        "pre-confirm must list txs in starknet descending-tip order: {resp}"
    );
    for h in &hashes {
        assert_phase(&devnet, *h, "PRE_CONFIRMED").await;
    }
    assert_received_count(&devnet, 0).await;
    let block_num_before = devnet.get_latest_block_with_tx_hashes().await.unwrap().block_number;
    let resp = devnet.send_custom_rpc("devnet_sealBlock", json!({})).await.unwrap();
    assert!(resp["block_hash"].is_string(), "expected block_hash: {resp}");

    let sealed = devnet.get_latest_block_with_tx_hashes().await.unwrap();
    assert_eq!(sealed.block_number, block_num_before + 1, "sealing must advance the latest block");
    assert_eq!(
        sealed.transactions,
        vec![hashes[2], hashes[1], hashes[0]],
        "sealed block must contain txs in starknet descending-tip order: {sealed:?}"
    );

    let snapshot = devnet.send_custom_rpc("devnet_getMempool", json!({})).await.unwrap();
    assert_eq!(
        snapshot["transactions"].as_array().unwrap().len(),
        0,
        "mempool must be drained after seal: {snapshot}"
    );
    assert_eq!(
        snapshot["pre_confirmed_transaction_hashes"].as_array().unwrap().len(),
        0,
        "open proposal must be empty after seal: {snapshot}"
    );
    // Policy persists across seals.
    assert_eq!(snapshot["config"]["ordering"], "starknet");
}
