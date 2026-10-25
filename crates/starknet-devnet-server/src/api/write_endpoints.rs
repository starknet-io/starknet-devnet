use std::sync::Arc;

use serde_json::Value;
use starknet_core::starknet::Starknet;
use starknet_core::starknet::dump::{DumpLoadOptions, StateDump};
use starknet_core::starknet::starknet_config::StarknetConfig;
use starknet_rs_core::types::TransactionExecutionStatus;
use starknet_types::contract_address::ContractAddress;
use starknet_types::felt::{TransactionHash, felt_from_prefixed_hex};
use starknet_types::messaging::{MessageToL1, MessageToL2};
use starknet_types::rpc::block::{BlockId, BlockTag};
use starknet_types::rpc::gas_modification::GasModificationRequest;
use starknet_types::rpc::transaction_receipt::FeeUnit;
use starknet_types::rpc::transactions::l1_handler_transaction::L1HandlerTransaction;
use starknet_types::rpc::transactions::{
    BroadcastedDeclareTransaction, BroadcastedDeployAccountTransaction,
    BroadcastedInvokeTransaction,
};

use super::error::{ApiError, StrictRpcResult};
use super::models::{
    DeclareTransactionOutput, DeployAccountTransactionOutput, DevnetResponse, JsonRpcResponse,
    StarknetResponse, TransactionHashOutput,
};
use crate::api::account_helpers::{get_balance, get_erc20_fee_unit_address};
use crate::api::models::{
    AbortedBlocks, AbortingBlocks, AcceptOnL1Request, AcceptedOnL1Blocks, CreatedBlock,
    FlushParameters, FlushedMessages, IncreaseTime, IncreaseTimeResponse, LoadRequest, MessageHash,
    MessagingLoadAddress, MintTokensRequest, MintTokensResponse, PostmanLoadL1MessagingContract,
    RestartParameters, RevertRequest, SetTime, SetTimeResponse,
};
use crate::api::{Api, JsonRpcHandler};
use crate::rpc_core::error::RpcError;
use crate::rpc_core::request::RpcMethodCall;
use crate::rpc_core::response::ResponseResult;

impl JsonRpcHandler {
    /// devnet_applyStateDiff
    pub async fn apply_state_diff(
        &self,
        request: starknet_types::rpc::state_mutation::ApplyStateDiffRequest,
    ) -> StrictRpcResult {
        let response = self.api.starknet.lock().await.apply_state_diff(request).map_err(
            |error| match error {
                starknet_core::error::Error::InvalidStateDiff(message) => {
                    ApiError::RpcError(RpcError::invalid_params(message))
                }
                starknet_core::error::Error::ContractNotFound => ApiError::ContractNotFound,
                starknet_core::error::Error::StateError(
                    starknet_core::error::StateError::NoneClassHash(_),
                ) => ApiError::ClassHashNotFound,
                starknet_core::error::Error::TypesError(_) => ApiError::InvalidContractClass,
                other => ApiError::StarknetDevnetError(other),
            },
        )?;
        Ok(DevnetResponse::AppliedStateDiff(response).into())
    }

    /// devnet_snapshot
    pub async fn snapshot(&self) -> StrictRpcResult {
        let mut snapshots = self.api.snapshots.lock().await;
        let snapshot_id = snapshots.last_id.checked_add(1).ok_or_else(|| {
            ApiError::RpcError(RpcError::internal_error_with("Snapshot ID space exhausted"))
        })?;
        let origin_accepted_on_l1_through = match &self.origin_caller {
            Some(origin) => origin.acceptance_boundary().await,
            None => None,
        };
        let core = self.api.starknet.lock().await.checkpoint();

        snapshots.last_id = snapshot_id;
        snapshots.checkpoints.insert(
            snapshot_id,
            crate::api::ServerCheckpoint { core, origin_accepted_on_l1_through },
        );

        Ok(DevnetResponse::SnapshotId(format!("0x{snapshot_id:x}")).into())
    }

    /// devnet_revert
    pub async fn revert(&self, request: RevertRequest) -> StrictRpcResult {
        let snapshot_id = parse_snapshot_id(&request.snapshot_id)?;
        if snapshot_id == 0 {
            return Ok(DevnetResponse::Reverted(false).into());
        }

        let mut snapshots = self.api.snapshots.lock().await;
        let std::collections::btree_map::Entry::Occupied(entry) =
            snapshots.checkpoints.entry(snapshot_id)
        else {
            return Ok(DevnetResponse::Reverted(false).into());
        };
        // Acquire every destination before publication. Cancellation during preparation must
        // leave the checkpoint and live state untouched.
        let mut starknet = self.api.starknet.lock().await;
        let mut origin_acceptance = match &self.origin_caller {
            Some(origin) => Some(origin.lock_acceptance().await),
            None => None,
        };
        let sockets = self.api.sockets.lock().await;
        let checkpoint = entry.get();
        let origin_accepted_on_l1_through = checkpoint.origin_accepted_on_l1_through;
        let notifications = Self::revert_notifications(&starknet, checkpoint.core.as_starknet())?;

        // Commit: no await or fallible operation until the restore is complete, including
        // snapshot consumption.
        let checkpoint = entry.remove();
        starknet.restore_checkpoint(checkpoint.core);
        if let Some(acceptance) = &mut origin_acceptance {
            acceptance.restore(origin_accepted_on_l1_through);
        }
        snapshots.checkpoints.retain(|id, _| *id < snapshot_id);
        drop(origin_acceptance);
        drop(starknet);
        drop(snapshots);
        sockets.notify_subscribers(&notifications);
        Ok(DevnetResponse::Reverted(true).into())
    }

    pub async fn add_declare_transaction(
        &self,
        request: BroadcastedDeclareTransaction,
    ) -> StrictRpcResult {
        let (transaction_hash, class_hash) =
            self.api.starknet.lock().await.add_declare_transaction(request).map_err(
                |err| match err {
                    starknet_core::error::Error::CompiledClassHashMismatch => {
                        ApiError::CompiledClassHashMismatch
                    }
                    starknet_core::error::Error::ClassAlreadyDeclared { .. } => {
                        ApiError::ClassAlreadyDeclared
                    }
                    starknet_core::error::Error::ContractClassSizeIsTooLarge => {
                        ApiError::ContractClassSizeIsTooLarge
                    }
                    unknown_error => ApiError::StarknetDevnetError(unknown_error),
                },
            )?;

        Ok(StarknetResponse::AddDeclareTransaction(DeclareTransactionOutput {
            transaction_hash,
            class_hash,
        })
        .into())
    }

    pub async fn add_deploy_account_transaction(
        &self,
        request: BroadcastedDeployAccountTransaction,
    ) -> StrictRpcResult {
        let (transaction_hash, contract_address) =
            self.api.starknet.lock().await.add_deploy_account_transaction(request).map_err(
                |err| match err {
                    starknet_core::error::Error::StateError(
                        starknet_core::error::StateError::NoneClassHash(_),
                    ) => ApiError::ClassHashNotFound,
                    unknown_error => ApiError::StarknetDevnetError(unknown_error),
                },
            )?;

        Ok(StarknetResponse::AddDeployAccountTransaction(DeployAccountTransactionOutput {
            transaction_hash,
            contract_address,
        })
        .into())
    }

    pub async fn add_invoke_transaction(
        &self,
        request: BroadcastedInvokeTransaction,
    ) -> StrictRpcResult {
        let transaction_hash = self.api.starknet.lock().await.add_invoke_transaction(request)?;

        Ok(StarknetResponse::TransactionHash(TransactionHashOutput { transaction_hash }).into())
    }

    /// devnet_impersonateAccount
    pub async fn impersonate_account(&self, address: ContractAddress) -> StrictRpcResult {
        let mut starknet = self.api.starknet.lock().await;
        starknet.impersonate_account(address)?;
        Ok(JsonRpcResponse::Empty)
    }

    /// devnet_stopImpersonateAccount
    pub async fn stop_impersonating_account(&self, address: ContractAddress) -> StrictRpcResult {
        let mut starknet = self.api.starknet.lock().await;
        starknet.stop_impersonating_account(&address);
        Ok(JsonRpcResponse::Empty)
    }

    /// devnet_autoImpersonate | devnet_stopAutoImpersonate
    pub async fn set_auto_impersonate(&self, auto_impersonation: bool) -> StrictRpcResult {
        let mut starknet = self.api.starknet.lock().await;
        starknet.set_auto_impersonate_account(auto_impersonation)?;
        Ok(JsonRpcResponse::Empty)
    }

    /// devnet_dump
    pub async fn dump(&self) -> StrictRpcResult {
        Ok(DevnetResponse::DevnetDump(Box::new(self.native_dump().await?)).into())
    }

    /// Encodes the instance, including the forking origin's L1-acceptance boundary.
    pub async fn native_dump(&self) -> Result<StateDump, ApiError> {
        let origin_accepted_on_l1_through = match &self.origin_caller {
            Some(origin) => origin.acceptance_boundary().await,
            None => None,
        };
        let mut dump = self.api.starknet.lock().await.dump()?;
        dump.origin_accepted_on_l1_through = origin_accepted_on_l1_through;
        Ok(dump)
    }

    /// Restores the forking origin's L1-acceptance boundary recorded in a dump loaded on startup.
    pub async fn restore_origin_acceptance(&self, accepted_on_l1_through: Option<u64>) {
        if let Some(origin) = &self.origin_caller {
            origin.lock_acceptance().await.restore(accepted_on_l1_through);
        }
    }

    /// devnet_load
    pub async fn load(&self, request: LoadRequest) -> StrictRpcResult {
        self.load_dump(request.dump).await?;
        Ok(JsonRpcResponse::Empty)
    }

    /// Replaces the instance state with a native dump or a legacy event array. On failure, the
    /// state, snapshots and subscriptions are left unchanged.
    pub async fn load_dump(&self, dump: Value) -> Result<(), ApiError> {
        let result = match dump {
            Value::Array(_) => self.load_legacy_dump(dump).await,
            dump => self.load_native_dump(dump).await,
        };
        if result.is_err() {
            // Building a replacement instance updates the global metrics.
            self.api.starknet.lock().await.sync_metrics();
        }
        result
    }

    async fn load_native_dump(&self, dump: Value) -> Result<(), ApiError> {
        let dump = StateDump::from_value(dump).map_err(load_error)?;
        let options = DumpLoadOptions {
            dump_on: self.api.config.dump_on,
            dump_path: self.api.config.dump_path.clone(),
            fork_url: None,
        };
        let dumped_config = dump.starknet_config(&options).map_err(load_error)?;
        self.assert_loadable_without_reconfiguration(&dumped_config)?;

        let origin_accepted_on_l1_through = dump.origin_accepted_on_l1_through;
        let loaded = Starknet::from_dump(dump, &options).map_err(load_error)?;
        self.replace_state(loaded, origin_accepted_on_l1_through).await;
        Ok(())
    }

    /// Legacy dumps are replayed on a separate instance, so a failed replay cannot affect this
    /// one and the replay notifies no subscribers.
    async fn load_legacy_dump(&self, dump: Value) -> Result<(), ApiError> {
        let events: Vec<RpcMethodCall> = serde_json::from_value(dump).map_err(|error| {
            ApiError::RpcError(RpcError::invalid_params(format!("Invalid legacy dump: {error}")))
        })?;
        if let Some(event) = events
            .iter()
            .find(|event| matches!(event.method.as_str(), "devnet_dump" | "devnet_load"))
        {
            return Err(ApiError::RpcError(RpcError::invalid_params(format!(
                "Legacy dumps containing {} cannot be loaded",
                event.method
            ))));
        }

        let config = self.api.starknet.lock().await.config.clone();
        let replay = JsonRpcHandler::new(Api {
            config: Arc::new(config.clone()),
            server_config: self.api.server_config.clone(),
            starknet: Arc::new(tokio::sync::Mutex::new(Starknet::new(&config)?)),
            sockets: Default::default(),
            snapshots: Default::default(),
            lifecycle: Default::default(),
        });
        replay.re_execute_locked(&events).await.map_err(ApiError::RpcError)?;

        let loaded = std::mem::take(&mut *replay.api.starknet.lock().await);
        let origin_accepted_on_l1_through = match &replay.origin_caller {
            Some(origin) => origin.acceptance_boundary().await,
            None => None,
        };
        self.replace_state(loaded, origin_accepted_on_l1_through).await;
        Ok(())
    }

    /// The server is wired to its startup block generation and forking origin, so dumps that
    /// change them load only at startup.
    fn assert_loadable_without_reconfiguration(
        &self,
        dumped: &StarknetConfig,
    ) -> Result<(), ApiError> {
        let running = &self.api.config;
        let mismatch = if dumped.block_generation_on != running.block_generation_on {
            Some(format!(
                "block generation {:?}, but this Devnet uses {:?}",
                dumped.block_generation_on, running.block_generation_on
            ))
        } else if (&dumped.fork_config.url, dumped.fork_config.block_number)
            != (&running.fork_config.url, running.fork_config.block_number)
        {
            let describe = |config: &StarknetConfig| match (
                &config.fork_config.url,
                config.fork_config.block_number,
            ) {
                (Some(url), Some(block_number)) => {
                    format!("a fork of {url} at block {block_number}")
                }
                _ => "no fork".to_string(),
            };
            Some(format!("{}, but this Devnet uses {}", describe(dumped), describe(running)))
        } else {
            None
        };

        match mismatch {
            Some(mismatch) => Err(ApiError::RpcError(RpcError::invalid_params(format!(
                "The dump uses {mismatch}; load it at startup instead"
            )))),
            None => Ok(()),
        }
    }

    /// Replaces the instance state, invalidating snapshots and clearing subscriptions while
    /// keeping WebSocket connections.
    async fn replace_state(&self, loaded: Starknet, origin_accepted_on_l1_through: Option<u64>) {
        let mut snapshots = self.api.snapshots.lock().await;
        let mut starknet = self.api.starknet.lock().await;
        let mut origin_acceptance = match &self.origin_caller {
            Some(origin) => Some(origin.lock_acceptance().await),
            None => None,
        };
        let mut sockets = self.api.sockets.lock().await;

        *starknet = loaded;
        starknet.sync_metrics();
        if let Some(acceptance) = &mut origin_acceptance {
            acceptance.restore(origin_accepted_on_l1_through);
        }
        snapshots.checkpoints.clear();
        sockets.clear();
    }

    /// devnet_postmanLoad
    pub async fn postman_load(&self, data: PostmanLoadL1MessagingContract) -> StrictRpcResult {
        let mut starknet = self.api.starknet.lock().await;
        let messaging_contract_address = starknet
            .configure_messaging(
                &data.network_url,
                data.messaging_contract_address.as_deref(),
                data.deployer_account_private_key.as_deref(),
            )
            .await?;

        Ok(DevnetResponse::MessagingContractAddress(MessagingLoadAddress {
            messaging_contract_address,
        })
        .into())
    }

    /// devnet_postmanFlush
    pub async fn postman_flush(&self, data: Option<FlushParameters>) -> StrictRpcResult {
        let is_dry_run = if let Some(params) = data { params.dry_run } else { false };

        // Need to handle L1 to L2 first in case those messages create L2 to L1 messages.
        let mut messages_to_l2 = vec![];
        let mut generated_l2_transactions = vec![];
        if !is_dry_run {
            // Fetch and execute messages to L2.
            // It is important that self.api.starknet is dropped immediately to allow rpc execution
            messages_to_l2 =
                self.api.starknet.lock().await.fetch_messages_to_l2().await.map_err(|e| {
                    ApiError::RpcError(RpcError::internal_error_with(format!(
                        "Error in fetching messages to L2: {e}"
                    )))
                })?;

            for message in &messages_to_l2 {
                let rpc_call = message.try_into().map_err(|e| {
                    ApiError::RpcError(RpcError::internal_error_with(format!(
                        "Error in converting message to L2 RPC call: {e}"
                    )))
                })?;
                let tx_hash = execute_rpc_tx(self, rpc_call).await.map_err(ApiError::RpcError)?;
                generated_l2_transactions.push(tx_hash);
            }
        };

        // Collect and send messages to L1.
        let mut starknet = self.api.starknet.lock().await;
        let messages_to_l1 = starknet.collect_messages_to_l1().await.map_err(|e| {
            ApiError::RpcError(RpcError::internal_error_with(format!(
                "Error in collecting messages to L1: {e}"
            )))
        })?;

        let l1_provider = if is_dry_run {
            "dry run".to_string()
        } else {
            starknet.send_messages_to_l1().await.map_err(|e| {
                ApiError::RpcError(RpcError::internal_error_with(format!(
                    "Error in sending messages to L1: {e}"
                )))
            })?;
            starknet.get_ethereum_url().unwrap_or("Not set".to_string())
        };

        let flushed_messages = FlushedMessages {
            messages_to_l1,
            messages_to_l2,
            generated_l2_transactions,
            l1_provider,
        };

        Ok(DevnetResponse::FlushedMessages(flushed_messages).into())
    }

    /// devnet_postmanSendMessageToL2
    pub async fn postman_send_message_to_l2(&self, message: MessageToL2) -> StrictRpcResult {
        let transaction = L1HandlerTransaction::try_from_message_to_l2(message)?;
        let transaction_hash =
            self.api.starknet.lock().await.add_l1_handler_transaction(transaction)?;
        Ok(DevnetResponse::TransactionHash(TransactionHashOutput { transaction_hash }).into())
    }

    /// devnet_postmanConsumeMessageFromL2
    pub async fn postman_consume_message_from_l2(&self, message: MessageToL1) -> StrictRpcResult {
        let message_hash =
            self.api.starknet.lock().await.consume_l2_to_l1_message(&message).await?;
        Ok(DevnetResponse::MessageHash(MessageHash { message_hash }).into())
    }

    /// devnet_createBlock
    pub async fn create_block(&self) -> StrictRpcResult {
        let mut starknet = self.api.starknet.lock().await;

        starknet.create_block();
        let block = starknet.get_latest_block()?;

        Ok(DevnetResponse::CreatedBlock(CreatedBlock { block_hash: block.block_hash() }).into())
    }

    /// devnet_abortBlocks
    pub async fn abort_blocks(&self, data: AbortingBlocks) -> StrictRpcResult {
        let aborted = self.api.starknet.lock().await.abort_blocks(data.starting_block_id)?;
        Ok(DevnetResponse::AbortedBlocks(AbortedBlocks { aborted }).into())
    }

    /// devnet_acceptOnL1
    pub async fn accept_on_l1(&self, data: AcceptOnL1Request) -> StrictRpcResult {
        let Some(origin_caller) = &self.origin_caller else {
            let accepted = self.api.starknet.lock().await.accept_on_l1(data.starting_block_id)?;
            return Ok(DevnetResponse::AcceptedOnL1Blocks(AcceptedOnL1Blocks { accepted }).into());
        };

        let _acceptance_guard = origin_caller.acceptance_lock.lock().await;
        let local_acceptance = self.api.starknet.lock().await.accept_on_l1(data.starting_block_id);

        let accepted = match local_acceptance {
            Ok(accepted) => {
                origin_caller.set_accepted_on_l1_through(origin_caller.fork_block_number()).await;
                accepted
            }
            Err(starknet_core::error::Error::NoBlock) => {
                let origin_block_number =
                    origin_caller.resolve_origin_block_number(data.starting_block_id).await?;
                origin_caller.set_accepted_on_l1_through(origin_block_number).await;
                Vec::new()
            }
            Err(error) => return Err(ApiError::StarknetDevnetError(error)),
        };

        Ok(DevnetResponse::AcceptedOnL1Blocks(AcceptedOnL1Blocks { accepted }).into())
    }

    /// devnet_setGasPrice
    pub async fn set_gas_price(&self, data: GasModificationRequest) -> StrictRpcResult {
        let modified_gas =
            self.api.starknet.lock().await.set_next_block_gas(data).map_err(ApiError::from)?;

        Ok(DevnetResponse::GasModification(modified_gas).into())
    }

    /// devnet_restart
    pub async fn restart(&self, data: Option<RestartParameters>) -> StrictRpcResult {
        // Invalidate immediately before the first destructive reset. The allocator deliberately
        // remains monotonic for the lifetime of the process.
        self.api.snapshots.lock().await.checkpoints.clear();

        let _acceptance_guard = if let Some(origin_caller) = &self.origin_caller {
            Some(origin_caller.acceptance_lock.lock().await)
        } else {
            None
        };
        let restart_params = data.unwrap_or_default();
        self.api.starknet.lock().await.restart(restart_params.restart_l1_to_l2_messaging)?;
        if let Some(origin_caller) = &self.origin_caller {
            origin_caller.reset_acceptance().await;
        }

        self.api.sockets.lock().await.clear();

        Ok(JsonRpcResponse::Empty)
    }

    /// devnet_setTime
    pub async fn set_time(&self, data: SetTime) -> StrictRpcResult {
        let mut starknet = self.api.starknet.lock().await;
        let generate_block = data.generate_block.unwrap_or(true);
        starknet.set_time(data.time, generate_block);
        let block_hash = if generate_block {
            let last_block = starknet.get_latest_block()?;
            Some(last_block.block_hash())
        } else {
            None
        };
        Ok(DevnetResponse::SetTime(SetTimeResponse { block_timestamp: data.time, block_hash })
            .into())
    }

    /// devnet_increaseTime
    pub async fn increase_time(&self, data: IncreaseTime) -> StrictRpcResult {
        let mut starknet = self.api.starknet.lock().await;
        starknet.increase_time(data.time);

        let last_block = starknet.get_latest_block()?;

        Ok(DevnetResponse::IncreaseTime(IncreaseTimeResponse {
            timestamp_increased_by: data.time,
            block_hash: last_block.block_hash(),
        })
        .into())
    }

    /// devnet_mint
    pub async fn mint(&self, request: MintTokensRequest) -> StrictRpcResult {
        let mut starknet = self.api.starknet.lock().await;
        let unit = request.unit.unwrap_or(FeeUnit::FRI);
        let erc20_address = get_erc20_fee_unit_address(&unit);

        // increase balance
        let tx_hash = starknet.mint(request.address, request.amount, erc20_address).await?;

        let tx = starknet.get_transaction_execution_and_finality_status(tx_hash)?;
        match tx.execution_status {
            TransactionExecutionStatus::Succeeded => {
                let new_balance = get_balance(
                    &mut starknet,
                    request.address,
                    erc20_address,
                    BlockId::Tag(BlockTag::PreConfirmed),
                )?;
                let new_balance = new_balance.to_str_radix(10);

                Ok(DevnetResponse::MintTokens(MintTokensResponse { new_balance, unit, tx_hash })
                    .into())
            }
            TransactionExecutionStatus::Reverted => Err(ApiError::MintingReverted {
                tx_hash,
                revert_reason: tx.failure_reason.as_deref().map(|reason| {
                    if reason.contains("u256_add Overflow") {
                        "The requested minting amount overflows the token contract's total_supply."
                            .into()
                    } else {
                        reason.to_string()
                    }
                }),
            }),
        }
    }
}

fn parse_snapshot_id(value: &str) -> Result<u64, ApiError> {
    let digits = value.strip_prefix("0x").ok_or_else(|| {
        ApiError::RpcError(RpcError::invalid_params("snapshot_id must be 0x-prefixed hexadecimal"))
    })?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::RpcError(RpcError::invalid_params(
            "snapshot_id must be 0x-prefixed hexadecimal",
        )));
    }
    u64::from_str_radix(digits, 16).map_err(|_| {
        ApiError::RpcError(RpcError::invalid_params("snapshot_id is outside the u64 range"))
    })
}

async fn execute_rpc_tx(
    rpc_handler: &JsonRpcHandler,
    rpc_call: RpcMethodCall,
) -> Result<TransactionHash, RpcError> {
    match Box::pin(rpc_handler.on_call_with_lifecycle(rpc_call, false)).await.result {
        ResponseResult::Success(result) => {
            let tx_hash_hex = result
                .get("transaction_hash")
                .ok_or(RpcError::internal_error_with(format!(
                    "Message execution did not yield a transaction hash: {result:?}"
                )))?
                .as_str()
                .ok_or(RpcError::internal_error_with(format!(
                    "Message execution result contains invalid transaction hash: {result:?}"
                )))?;
            let tx_hash = felt_from_prefixed_hex(tx_hash_hex).map_err(|e| {
                RpcError::internal_error_with(format!(
                    "Message execution resulted in an invalid tx hash: {tx_hash_hex}: {e}"
                ))
            })?;
            Ok(tx_hash)
        }
        ResponseResult::Error(e) => Err(e),
    }
}

/// Maps a failure to read a dump; invalid dumps are reported as invalid params.
fn load_error(error: starknet_core::error::Error) -> ApiError {
    match error {
        starknet_core::error::Error::InvalidDump(msg) => {
            ApiError::RpcError(RpcError::invalid_params(format!("Invalid dump: {msg}")))
        }
        other => ApiError::StarknetDevnetError(other),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_snapshot_id;
    use crate::api::models::BroadcastedDeployAccountTransactionEnumWrapper;

    #[test]
    fn parses_snapshot_ids() {
        assert_eq!(parse_snapshot_id("0x1").unwrap(), 1);
        assert_eq!(parse_snapshot_id("0x000A").unwrap(), 10);
        assert!(parse_snapshot_id("1").is_err());
        assert!(parse_snapshot_id("0x").is_err());
        assert!(parse_snapshot_id("0xg").is_err());
        assert!(parse_snapshot_id("0x10000000000000000").is_err());
    }

    #[test]
    fn check_correct_deserialization_of_deploy_account_transaction_request() {
        let _: BroadcastedDeployAccountTransactionEnumWrapper = serde_json::from_str(
            r#"{
                "type":"DEPLOY_ACCOUNT",
                "resource_bounds": {
                    "l1_gas": {
                        "max_amount": "0x1",
                        "max_price_per_unit": "0x2"
                    },
                    "l1_data_gas": {
                        "max_amount": "0x1",
                        "max_price_per_unit": "0x2"
                    },
                    "l2_gas": {
                        "max_amount": "0x1",
                        "max_price_per_unit": "0x2"
                    }
                },
                "tip": "0xabc",
                "paymaster_data": [],
                "version": "0x3",
                "signature": ["0xFF", "0xAA"],
                "nonce": "0x0",
                "contract_address_salt": "0x01",
                "class_hash": "0x01",
                "constructor_calldata": ["0x01"],
                "nonce_data_availability_mode": "L1",
                "fee_data_availability_mode": "L1"
            }"#,
        )
        .unwrap();
    }
}
