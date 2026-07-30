use std::num::NonZeroUsize;
use std::sync::Arc;

use lru::LruCache;
use serde_json::json;
use starknet_rs_core::types::{
    BlockId as ImportedBlockId, BlockStatus as ImportedBlockStatus, BlockTag as ImportedBlockTag,
    BlockWithTxHashes, Felt, MaybePreConfirmedBlockWithTxHashes,
};
use starknet_rs_providers::jsonrpc::HttpTransport;
use starknet_rs_providers::{JsonRpcClient, Provider};
use starknet_types::rpc::block::{BlockId, BlockTag};
use tokio::sync::{Mutex, RwLock};

use super::error::ApiError;
use crate::rpc_core::error::RpcError;
use crate::rpc_core::request::{RequestParams, RpcMethodCall};
use crate::rpc_core::response::{ResponseResult, RpcResponse};

const TRANSACTION_BLOCK_NUMBER_CACHE_CAPACITY: usize = 1024;

struct OriginAcceptance {
    accepted_on_l1_through: Option<u64>,
    transaction_block_numbers: LruCache<Felt, u64>,
}

impl Default for OriginAcceptance {
    fn default() -> Self {
        Self {
            accepted_on_l1_through: None,
            transaction_block_numbers: LruCache::new(
                NonZeroUsize::new(TRANSACTION_BLOCK_NUMBER_CACHE_CAPACITY)
                    .unwrap_or(NonZeroUsize::MIN),
            ),
        }
    }
}

/// Used for forwarding requests to origin in case of:
/// - BlockNotFound
/// - TransactionNotFound
/// - NoStateAtBlock
/// - ClassHashNotFound
///
/// Basic contract-wise interaction is handled by `BlockingOriginReader`
#[derive(Clone)]
pub struct OriginForwarder {
    reqwest_client: reqwest::Client,
    url: Arc<String>,
    block_number: u64,
    pub(crate) starknet_client: JsonRpcClient<HttpTransport>,
    acceptance: Arc<RwLock<OriginAcceptance>>,
    pub(crate) acceptance_lock: Arc<Mutex<()>>,
}

impl OriginForwarder {
    pub fn new(url: url::Url, block_number: u64) -> Self {
        Self {
            reqwest_client: reqwest::Client::new(),
            url: Arc::new(url.to_string()),
            block_number,
            starknet_client: JsonRpcClient::new(HttpTransport::new(url)),
            acceptance: Default::default(),
            acceptance_lock: Default::default(),
        }
    }

    pub fn fork_block_number(&self) -> u64 {
        self.block_number
    }

    /// In case block tag "pre_confirmed" or "latest" is a part of the request, it is replaced with
    /// the numeric block id of the forked block. Both JSON-RPC 1 and 2 semantics is covered
    fn clone_call_with_origin_block_id(
        &self,
        rpc_call: &RpcMethodCall,
        accepted_on_l1_through: Option<u64>,
    ) -> RpcMethodCall {
        let mut new_rpc_call = rpc_call.clone();
        let origin_block_id = json!({ "block_number": self.block_number });
        let l1_accepted_block_id =
            json!({ "block_number": accepted_on_l1_through.unwrap_or(self.block_number) });

        match new_rpc_call.params {
            crate::rpc_core::request::RequestParams::None => (),
            crate::rpc_core::request::RequestParams::Array(ref mut params) => {
                for param in params.iter_mut() {
                    match param.as_str() {
                        Some("latest" | "pre_confirmed") => {
                            *param = origin_block_id;
                            break;
                        }
                        Some("l1_accepted") => {
                            if accepted_on_l1_through.is_none() {
                                tracing::warn!("Assuming fork block is ACCEPTED_ON_L1");
                            }
                            *param = l1_accepted_block_id;
                            break;
                        }
                        _ => (),
                    }
                }
            }
            crate::rpc_core::request::RequestParams::Object(ref mut params) => {
                if let Some(block_id) = params.get_mut("block_id") {
                    match block_id.as_str() {
                        Some("latest" | "pre_confirmed") => {
                            *block_id = origin_block_id;
                        }
                        Some("l1_accepted") => {
                            if accepted_on_l1_through.is_none() {
                                tracing::warn!("Assuming fork block is ACCEPTED_ON_L1");
                            }
                            *block_id = l1_accepted_block_id;
                        }
                        _ => (),
                    }
                }
            }
        }
        new_rpc_call
    }

    pub async fn call(&self, rpc_call: &RpcMethodCall) -> ResponseResult {
        match self.try_call(rpc_call).await {
            Ok(result) => result,
            Err(e) => ResponseResult::Error(RpcError::internal_error_with::<String>(format!(
                "Error in interacting with origin: {e}"
            ))),
        }
    }

    async fn try_call(&self, rpc_call: &RpcMethodCall) -> Result<ResponseResult, anyhow::Error> {
        let accepted_on_l1_through = self.acceptance.read().await.accepted_on_l1_through;
        let interception =
            AcceptanceResponseKind::from_method(&rpc_call.method).zip(accepted_on_l1_through);
        let forwarded_call = self.clone_call_with_origin_block_id(rpc_call, accepted_on_l1_through);
        let origin_rpc_resp: RpcResponse = self
            .reqwest_client
            .post(self.url.to_string())
            .json(&forwarded_call)
            .send()
            .await?
            .json()
            .await?;

        match interception {
            Some((response_kind, accepted_on_l1_through)) => Ok(self
                .intercept_response(
                    rpc_call,
                    origin_rpc_resp.result,
                    response_kind,
                    accepted_on_l1_through,
                )
                .await),
            None => Ok(origin_rpc_resp.result),
        }
    }

    async fn get_l1_accepted_block(&self) -> Result<BlockWithTxHashes, ApiError> {
        let tag = ImportedBlockId::Tag(ImportedBlockTag::L1Accepted);
        match self.starknet_client.get_block_with_tx_hashes(tag).await {
            Ok(MaybePreConfirmedBlockWithTxHashes::Block(block)) => Ok(block),
            other => Err(ApiError::StarknetDevnetError(
                starknet_core::error::Error::UnexpectedInternalError {
                    msg: format!(
                        "Failed retrieval of l1_accepted block from forking origin. Got: {other:?}"
                    ),
                },
            )),
        }
    }

    async fn get_origin_block(
        &self,
        block_id: ImportedBlockId,
    ) -> Result<BlockWithTxHashes, ApiError> {
        match self.starknet_client.get_block_with_tx_hashes(block_id).await {
            Ok(MaybePreConfirmedBlockWithTxHashes::Block(block)) => Ok(block),
            Ok(MaybePreConfirmedBlockWithTxHashes::PreConfirmedBlock(_)) => {
                Err(ApiError::StarknetDevnetError(starknet_core::error::Error::UnsupportedAction {
                    msg: "Pre-confirmed block cannot be accepted on L1".into(),
                }))
            }
            Err(starknet_rs_providers::ProviderError::StarknetError(
                starknet_rs_core::types::StarknetError::BlockNotFound,
            )) => Err(ApiError::StarknetDevnetError(starknet_core::error::Error::NoBlock)),
            Err(error) => Err(ApiError::StarknetDevnetError(
                starknet_core::error::Error::UnexpectedInternalError {
                    msg: format!("Error in retrieving a block from forking origin: {error}"),
                },
            )),
        }
    }

    pub(crate) async fn resolve_origin_block_number(
        &self,
        block_id: BlockId,
    ) -> Result<u64, ApiError> {
        let starting_block_id = match block_id {
            BlockId::Tag(BlockTag::Latest | BlockTag::PreConfirmed) => {
                ImportedBlockId::Number(self.block_number)
            }
            other => ImportedBlockId::from(other),
        };
        let block = self.get_origin_block(starting_block_id).await?;

        if block.block_number > self.block_number {
            return Err(ApiError::StarknetDevnetError(starknet_core::error::Error::NoBlock));
        }

        let previously_accepted_through = self.acceptance.read().await.accepted_on_l1_through;
        if previously_accepted_through.is_some_and(|number| block.block_number <= number) {
            return Err(ApiError::StarknetDevnetError(
                starknet_core::error::Error::UnsupportedAction {
                    msg: "Block already accepted on L1".into(),
                },
            ));
        }

        match block.status {
            ImportedBlockStatus::AcceptedOnL2 => Ok(block.block_number),
            ImportedBlockStatus::AcceptedOnL1 => {
                Err(ApiError::StarknetDevnetError(starknet_core::error::Error::UnsupportedAction {
                    msg: "Block already accepted on L1".into(),
                }))
            }
            _ => {
                Err(ApiError::StarknetDevnetError(starknet_core::error::Error::UnsupportedAction {
                    msg: "Only blocks accepted on L2 can be accepted on L1".into(),
                }))
            }
        }
    }

    pub(crate) async fn set_accepted_on_l1_through(&self, block_number: u64) {
        let mut acceptance = self.acceptance.write().await;
        acceptance.accepted_on_l1_through = Some(
            acceptance
                .accepted_on_l1_through
                .map_or(block_number, |current| current.max(block_number)),
        );
    }

    pub(crate) async fn reset_acceptance(&self) {
        *self.acceptance.write().await = OriginAcceptance::default();
    }

    async fn intercept_response(
        &self,
        rpc_call: &RpcMethodCall,
        mut response: ResponseResult,
        response_kind: AcceptanceResponseKind,
        accepted_on_l1_through: u64,
    ) -> ResponseResult {
        let ResponseResult::Success(value) = &mut response else {
            return response;
        };

        if response_kind == AcceptanceResponseKind::MessagesStatus {
            self.intercept_messages_status(value, accepted_on_l1_through).await;
            return response;
        }

        let block_number = if let Some(block_number) = block_number_from_response(value) {
            Some(block_number)
        } else if response_kind == AcceptanceResponseKind::TransactionStatus {
            match transaction_hash_from_call(rpc_call) {
                Some(transaction_hash) => self.transaction_block_number(transaction_hash).await,
                None => None,
            }
        } else {
            None
        };

        if block_number.is_none_or(|number| number > accepted_on_l1_through) {
            return response;
        }

        match response_kind {
            AcceptanceResponseKind::Block => promote_status(value, "status"),
            AcceptanceResponseKind::BlockWithReceipts => {
                promote_status(value, "status");
                if let Some(transactions) =
                    value.get_mut("transactions").and_then(serde_json::Value::as_array_mut)
                {
                    for transaction in transactions {
                        if let Some(receipt) = transaction.get_mut("receipt") {
                            promote_status(receipt, "finality_status");
                        }
                    }
                }
            }
            AcceptanceResponseKind::TransactionReceipt
            | AcceptanceResponseKind::TransactionStatus => {
                promote_status(value, "finality_status");
            }
            AcceptanceResponseKind::MessagesStatus => {}
        }

        response
    }

    async fn intercept_messages_status(
        &self,
        value: &mut serde_json::Value,
        accepted_on_l1_through: u64,
    ) {
        let transaction_hashes = value
            .as_array()
            .map(|statuses| {
                statuses
                    .iter()
                    .enumerate()
                    .filter_map(|(index, status)| {
                        status
                            .get("transaction_hash")
                            .cloned()
                            .and_then(|hash| serde_json::from_value(hash).ok())
                            .map(|hash| (index, hash))
                    })
                    .collect::<Vec<(usize, Felt)>>()
            })
            .unwrap_or_default();

        for (index, transaction_hash) in transaction_hashes {
            if self
                .transaction_block_number(transaction_hash)
                .await
                .is_some_and(|number| number <= accepted_on_l1_through)
                && let Some(status) = value.get_mut(index)
            {
                promote_status(status, "finality_status");
            }
        }
    }

    async fn transaction_block_number(&self, transaction_hash: Felt) -> Option<u64> {
        let cached_block_number =
            self.acceptance.write().await.transaction_block_numbers.get(&transaction_hash).copied();
        if let Some(block_number) = cached_block_number {
            return Some(block_number);
        }

        let receipt = match self.starknet_client.get_transaction_receipt(transaction_hash).await {
            Ok(receipt) => receipt,
            Err(error) => {
                tracing::warn!(
                    %transaction_hash,
                    %error,
                    "Could not resolve origin transaction block number for L1 acceptance"
                );
                return None;
            }
        };
        let block_number = receipt.block.block_number();
        self.acceptance.write().await.transaction_block_numbers.put(transaction_hash, block_number);

        Some(block_number)
    }

    /// Only use with confirmed block ID
    pub(crate) async fn get_block_number_from_block_id(
        &self,
        block_id: BlockId,
    ) -> Result<u64, ApiError> {
        if block_id == BlockId::Tag(BlockTag::L1Accepted) {
            if let Some(block_number) = self.acceptance.read().await.accepted_on_l1_through {
                return Ok(block_number);
            }

            let l1_accepted_block = self.get_l1_accepted_block().await?;
            return Ok(std::cmp::min(l1_accepted_block.block_number, self.fork_block_number()));
        }

        match self.starknet_client.get_block_with_tx_hashes(ImportedBlockId::from(block_id)).await {
            Ok(MaybePreConfirmedBlockWithTxHashes::Block(block)) => Ok(block.block_number),
            Ok(MaybePreConfirmedBlockWithTxHashes::PreConfirmedBlock(block)) => {
                Err(ApiError::StarknetDevnetError(
                    starknet_core::error::Error::UnexpectedInternalError {
                        msg: format!("Impossible: expected a confirmed block; got: {block:?}"),
                    },
                ))
            }
            Err(error) => Err(ApiError::StarknetDevnetError(
                starknet_core::error::Error::UnexpectedInternalError {
                    msg: format!("Invalid origin response in retrieving block number: {error}"),
                },
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AcceptanceResponseKind {
    Block,
    BlockWithReceipts,
    TransactionReceipt,
    TransactionStatus,
    MessagesStatus,
}

impl AcceptanceResponseKind {
    fn from_method(method: &str) -> Option<Self> {
        match method {
            "starknet_getBlockWithTxHashes" | "starknet_getBlockWithTxs" => Some(Self::Block),
            "starknet_getBlockWithReceipts" => Some(Self::BlockWithReceipts),
            "starknet_getTransactionReceipt" => Some(Self::TransactionReceipt),
            "starknet_getTransactionStatus" => Some(Self::TransactionStatus),
            "starknet_getMessagesStatus" => Some(Self::MessagesStatus),
            _ => None,
        }
    }
}

fn block_number_from_response(value: &serde_json::Value) -> Option<u64> {
    value.get("block_number").and_then(serde_json::Value::as_u64)
}

fn promote_status(value: &mut serde_json::Value, field: &str) {
    if value.get(field).and_then(serde_json::Value::as_str) == Some("ACCEPTED_ON_L2") {
        value[field] = serde_json::Value::String("ACCEPTED_ON_L1".into());
    }
}

fn transaction_hash_from_call(rpc_call: &RpcMethodCall) -> Option<Felt> {
    let value = match &rpc_call.params {
        RequestParams::Array(params) => params.first(),
        RequestParams::Object(params) => params.get("transaction_hash"),
        RequestParams::None => None,
    }?;

    serde_json::from_value(value.clone()).ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use starknet_rs_core::types::Felt;

    use super::{AcceptanceResponseKind, OriginForwarder, TRANSACTION_BLOCK_NUMBER_CACHE_CAPACITY};
    use crate::rpc_core::request::RpcMethodCall;
    use crate::rpc_core::response::ResponseResult;

    #[test]
    fn test_replacing_block_id() {
        let block_number = 10;
        let forwarder =
            OriginForwarder::new(url::Url::parse("http://dummy.com").unwrap(), block_number);

        let common_body = json!({
            "method": "starknet_dummyMethod",
            "id": 1,
        });
        for (jsonrpc_value, orig_params, replaced_params) in [
            ("2.0", json!(null), json!(null)),
            ("1.0", json!(["a", 1, "latest", 2]), json!(["a", 1, { "block_number": 10 }, 2])),
            (
                "1.0",
                json!(["a", 1, "pre_confirmed", 2]),
                json!(["a", 1, { "block_number": 10 }, 2]),
            ),
            (
                "2.0",
                json!({ "param1": "a", "param2": 1, "block_id": "latest", "param3": 2 }),
                json!({ "param1": "a", "param2": 1, "block_id": { "block_number": 10 }, "param3": 2 }),
            ),
            (
                "2.0",
                json!({ "param1": "a", "param2": 1, "block_id": "pre_confirmed", "param3": 2 }),
                json!({ "param1": "a", "param2": 1, "block_id": { "block_number": 10 }, "param3": 2 }),
            ),
        ] {
            let mut orig_body = common_body.clone();
            orig_body["jsonrpc"] = serde_json::Value::String(jsonrpc_value.into());
            orig_body["params"] = orig_params;

            let request: RpcMethodCall = serde_json::from_value(orig_body).unwrap();
            let replaced_request = forwarder.clone_call_with_origin_block_id(&request, None);
            let replaced_request_json = serde_json::to_value(replaced_request).unwrap();

            let mut expected_body = common_body.clone();
            expected_body["jsonrpc"] = serde_json::Value::String(jsonrpc_value.into());
            expected_body["params"] = replaced_params;

            assert_eq!(replaced_request_json, expected_body);
        }
    }

    #[test]
    fn l1_accepted_tag_uses_simulated_boundary_when_present() {
        let forwarder = OriginForwarder::new(url::Url::parse("http://dummy.com").unwrap(), 10);
        let request: RpcMethodCall = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "method": "starknet_getBlockWithTxHashes",
            "params": { "block_id": "l1_accepted" },
            "id": 1,
        }))
        .unwrap();

        let unchanged = forwarder.clone_call_with_origin_block_id(&request, None);
        let simulated = forwarder.clone_call_with_origin_block_id(&request, Some(7));

        assert_eq!(
            serde_json::to_value(unchanged).unwrap()["params"]["block_id"],
            json!({ "block_number": 10 })
        );
        assert_eq!(
            serde_json::to_value(simulated).unwrap()["params"]["block_id"],
            json!({ "block_number": 7 })
        );
    }

    #[test]
    fn acceptance_response_subset_is_explicit() {
        for method in [
            "starknet_getBlockWithTxHashes",
            "starknet_getBlockWithTxs",
            "starknet_getBlockWithReceipts",
            "starknet_getTransactionReceipt",
            "starknet_getTransactionStatus",
            "starknet_getMessagesStatus",
        ] {
            assert!(AcceptanceResponseKind::from_method(method).is_some());
        }
        assert!(AcceptanceResponseKind::from_method("starknet_getTransactionByHash").is_none());
    }

    #[tokio::test]
    async fn interception_is_dormant_until_acceptance_boundary_is_set() {
        let forwarder = OriginForwarder::new(url::Url::parse("http://dummy.com").unwrap(), 10);
        let request: RpcMethodCall = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "method": "starknet_getBlockWithTxHashes",
            "params": { "block_id": { "block_number": 5 } },
            "id": 1,
        }))
        .unwrap();
        let response = ResponseResult::Success(json!({
            "block_number": 5,
            "status": "ACCEPTED_ON_L2",
        }));

        assert!(forwarder.acceptance.read().await.accepted_on_l1_through.is_none());

        forwarder.set_accepted_on_l1_through(5).await;

        assert_eq!(forwarder.acceptance.read().await.accepted_on_l1_through, Some(5));
        assert!(forwarder.acceptance.read().await.transaction_block_numbers.is_empty());
        assert_eq!(
            forwarder
                .intercept_response(&request, response, AcceptanceResponseKind::Block, 5,)
                .await,
            ResponseResult::Success(json!({
                "block_number": 5,
                "status": "ACCEPTED_ON_L1",
            }))
        );
    }

    #[tokio::test]
    async fn interception_promotes_origin_transaction_receipts_and_statuses() {
        let forwarder = OriginForwarder::new(url::Url::parse("http://dummy.com").unwrap(), 10);
        let transaction_hash = Felt::from(123_u64);
        forwarder.set_accepted_on_l1_through(5).await;

        let receipt_request: RpcMethodCall = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "method": "starknet_getTransactionReceipt",
            "params": { "transaction_hash": transaction_hash },
            "id": 1,
        }))
        .unwrap();
        let receipt = ResponseResult::Success(json!({
            "block_number": 5,
            "finality_status": "ACCEPTED_ON_L2",
        }));
        assert_eq!(
            forwarder
                .intercept_response(
                    &receipt_request,
                    receipt,
                    AcceptanceResponseKind::TransactionReceipt,
                    5,
                )
                .await,
            ResponseResult::Success(json!({
                "block_number": 5,
                "finality_status": "ACCEPTED_ON_L1",
            }))
        );

        forwarder.acceptance.write().await.transaction_block_numbers.put(transaction_hash, 5);
        let status_request: RpcMethodCall = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "method": "starknet_getTransactionStatus",
            "params": { "transaction_hash": transaction_hash },
            "id": 1,
        }))
        .unwrap();
        let status = ResponseResult::Success(json!({
            "finality_status": "ACCEPTED_ON_L2",
            "execution_status": "SUCCEEDED",
        }));
        assert_eq!(
            forwarder
                .intercept_response(
                    &status_request,
                    status,
                    AcceptanceResponseKind::TransactionStatus,
                    5,
                )
                .await,
            ResponseResult::Success(json!({
                "finality_status": "ACCEPTED_ON_L1",
                "execution_status": "SUCCEEDED",
            }))
        );
    }

    #[tokio::test]
    async fn interception_promotes_receipts_nested_in_accepted_origin_block() {
        let forwarder = OriginForwarder::new(url::Url::parse("http://dummy.com").unwrap(), 10);
        forwarder.set_accepted_on_l1_through(5).await;
        let request: RpcMethodCall = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "method": "starknet_getBlockWithReceipts",
            "params": { "block_id": { "block_number": 5 } },
            "id": 1,
        }))
        .unwrap();
        let response = ResponseResult::Success(json!({
            "block_number": 5,
            "status": "ACCEPTED_ON_L2",
            "transactions": [{
                "transaction": {},
                "receipt": { "finality_status": "ACCEPTED_ON_L2" },
            }],
        }));

        assert_eq!(
            forwarder
                .intercept_response(
                    &request,
                    response,
                    AcceptanceResponseKind::BlockWithReceipts,
                    5,
                )
                .await,
            ResponseResult::Success(json!({
                "block_number": 5,
                "status": "ACCEPTED_ON_L1",
                "transactions": [{
                    "transaction": {},
                    "receipt": { "finality_status": "ACCEPTED_ON_L1" },
                }],
            }))
        );
    }

    #[tokio::test]
    async fn interception_promotes_only_covered_message_statuses() {
        let forwarder = OriginForwarder::new(url::Url::parse("http://dummy.com").unwrap(), 10);
        let covered_transaction_hash = Felt::from(123_u64);
        let uncovered_transaction_hash = Felt::from(456_u64);
        {
            let mut acceptance = forwarder.acceptance.write().await;
            acceptance.transaction_block_numbers.put(covered_transaction_hash, 5);
            acceptance.transaction_block_numbers.put(uncovered_transaction_hash, 6);
        }
        let request: RpcMethodCall = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "method": "starknet_getMessagesStatus",
            "params": { "transaction_hash": Felt::ONE },
            "id": 1,
        }))
        .unwrap();
        let response = ResponseResult::Success(json!([
            {
                "transaction_hash": covered_transaction_hash,
                "finality_status": "ACCEPTED_ON_L2",
            },
            {
                "transaction_hash": uncovered_transaction_hash,
                "finality_status": "ACCEPTED_ON_L2",
            },
        ]));

        assert_eq!(
            forwarder
                .intercept_response(&request, response, AcceptanceResponseKind::MessagesStatus, 5,)
                .await,
            ResponseResult::Success(json!([
                {
                    "transaction_hash": covered_transaction_hash,
                    "finality_status": "ACCEPTED_ON_L1",
                },
                {
                    "transaction_hash": uncovered_transaction_hash,
                    "finality_status": "ACCEPTED_ON_L2",
                },
            ]))
        );
    }

    #[tokio::test]
    async fn transaction_block_number_cache_is_bounded() {
        let forwarder = OriginForwarder::new(url::Url::parse("http://dummy.com").unwrap(), 10);
        let mut acceptance = forwarder.acceptance.write().await;

        for number in 0..=TRANSACTION_BLOCK_NUMBER_CACHE_CAPACITY as u64 {
            acceptance.transaction_block_numbers.put(Felt::from(number), number);
        }

        assert_eq!(
            acceptance.transaction_block_numbers.len(),
            TRANSACTION_BLOCK_NUMBER_CACHE_CAPACITY
        );
        assert!(!acceptance.transaction_block_numbers.contains(&Felt::ZERO));
        assert!(
            acceptance
                .transaction_block_numbers
                .contains(&Felt::from(TRANSACTION_BLOCK_NUMBER_CACHE_CAPACITY as u64))
        );
    }
}
