use starknet_core::error::Error;
use starknet_rs_core::types::{
    BlockId as ImportedBlockId, Felt, L1DataAvailabilityMode as ImportedL1DataAvailabilityMode,
    MaybePreConfirmedBlockWithTxHashes,
};
use starknet_rs_providers::{Provider, ProviderError};
use starknet_types::contract_address::ContractAddress;
use starknet_types::emitted_event::{EmittedEvent, SubscriptionEmittedEvent};
use starknet_types::felt::TransactionHash;
use starknet_types::rpc::block::{BlockHeader, BlockId, BlockStatus, BlockTag};
use starknet_types::rpc::transactions::TransactionFinalityStatus;
use starknet_types::starknet_api::block::{BlockNumber, BlockTimestamp};
use starknet_types::starknet_api::core::{
    EventCommitment, ReceiptCommitment, StateDiffCommitment, TransactionCommitment,
};
use starknet_types::starknet_api::data_availability::L1DataAvailabilityMode;
use starknet_types::starknet_api::hash::PoseidonHash;

use super::JsonRpcHandler;
use super::error::ApiError;
use super::models::{
    EventsSubscriptionInput, SubscriptionIdInput, TransactionHashInput,
    TransactionReceiptSubscriptionInput, TransactionSubscriptionInput,
};
use crate::api::models::JsonRpcSubscriptionRequest;
use crate::rpc_core::request::Id;
use crate::subscribe::{
    AddressFilter, NewTransactionStatus, NotificationData, SocketId, StatusFilter, Subscription,
};

pub(crate) enum PreparedWsSubscription {
    NewHeads(PreparedNewHeads),
    Events(PreparedEvents),
    Other(JsonRpcSubscriptionRequest),
}

pub(crate) struct PreparedNewHeads {
    block_id: BlockId,
    origin_header: Option<BlockHeader>,
    origin_range: Option<(u64, u64)>,
    origin_headers: Vec<BlockHeader>,
}

pub(crate) struct PreparedEvents {
    input: Option<EventsSubscriptionInput>,
    block_id: BlockId,
    origin_header: Option<BlockHeader>,
    origin_range: Option<(u64, u64)>,
    origin_events: Vec<EmittedEvent>,
}

/// The definitions of JSON-RPC read endpoints defined in starknet_ws_api.json
impl JsonRpcHandler {
    /// Fetch immutable fork history before entering lifecycle coordination. The local range is
    /// checked again when publishing, so writes and reverts during this phase are included.
    pub(crate) async fn prepare_ws_subscription(
        &self,
        request: JsonRpcSubscriptionRequest,
    ) -> Result<PreparedWsSubscription, ApiError> {
        match request {
            JsonRpcSubscriptionRequest::NewHeads(input) => {
                let block_id = input
                    .map(|input| input.block_id.into())
                    .unwrap_or(BlockId::Tag(BlockTag::Latest));
                let (query_block_number, origin_header) =
                    self.resolve_start_block(block_id).await?;
                let (_, _, origin_range) =
                    self.validate_block_number_range(query_block_number).await?;
                let origin_headers = if !matches!(block_id, BlockId::Tag(_)) {
                    if let Some((start, end)) = origin_range {
                        self.fetch_origin_heads(start, end).await?
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
                Ok(PreparedWsSubscription::NewHeads(PreparedNewHeads {
                    block_id,
                    origin_header,
                    origin_range,
                    origin_headers,
                }))
            }
            JsonRpcSubscriptionRequest::Events(input) => {
                let block_id = input
                    .as_ref()
                    .and_then(|input| input.block_id.as_ref().map(BlockId::from))
                    .unwrap_or(BlockId::Tag(BlockTag::Latest));
                let (query_block_number, origin_header) =
                    self.resolve_start_block(block_id).await?;
                let (_, _, origin_range) =
                    self.validate_block_number_range(query_block_number).await?;
                let origin_events = if let Some((start, end)) = origin_range {
                    self.fetch_origin_events(
                        start,
                        end,
                        input.as_ref().and_then(|input| input.from_address.clone()),
                        input.as_ref().and_then(|input| input.keys.clone()),
                    )
                    .await?
                } else {
                    Vec::new()
                };
                Ok(PreparedWsSubscription::Events(PreparedEvents {
                    input,
                    block_id,
                    origin_header,
                    origin_range,
                    origin_events,
                }))
            }
            other => Ok(PreparedWsSubscription::Other(other)),
        }
    }

    pub(crate) async fn execute_ws_subscription(
        &self,
        request: PreparedWsSubscription,
        rpc_request_id: Id,
        socket_id: SocketId,
    ) -> Result<(), ApiError> {
        match request {
            PreparedWsSubscription::NewHeads(prepared) => {
                self.subscribe_new_heads(prepared, rpc_request_id, socket_id).await
            }
            PreparedWsSubscription::Events(prepared) => {
                self.subscribe_events(prepared, rpc_request_id, socket_id).await
            }
            PreparedWsSubscription::Other(JsonRpcSubscriptionRequest::TransactionStatus(
                TransactionHashInput { transaction_hash },
            )) => self.subscribe_tx_status(transaction_hash, rpc_request_id, socket_id).await,
            PreparedWsSubscription::Other(JsonRpcSubscriptionRequest::NewTransactions(data)) => {
                self.subscribe_new_txs(data, rpc_request_id, socket_id).await
            }
            PreparedWsSubscription::Other(JsonRpcSubscriptionRequest::NewTransactionReceipts(
                data,
            )) => self.subscribe_new_tx_receipts(data, rpc_request_id, socket_id).await,
            PreparedWsSubscription::Other(JsonRpcSubscriptionRequest::Unsubscribe(
                SubscriptionIdInput { subscription_id },
            )) => {
                let mut sockets = self.api.sockets.lock().await;
                let socket_context = sockets.get_mut(&socket_id)?;
                socket_context.unsubscribe(rpc_request_id, subscription_id)
            }
            PreparedWsSubscription::Other(
                JsonRpcSubscriptionRequest::NewHeads(_) | JsonRpcSubscriptionRequest::Events(_),
            ) => unreachable!("historical subscriptions are prepared above"),
        }
    }

    async fn get_origin_block_header_by_id(&self, id: BlockId) -> Result<BlockHeader, ApiError> {
        let origin_caller = self.origin_caller.as_ref().ok_or_else(|| {
            ApiError::StarknetDevnetError(Error::UnexpectedInternalError {
                msg: "No origin caller available".into(),
            })
        })?;
        match origin_caller
            .starknet_client
            .get_block_with_tx_hashes(ImportedBlockId::from(id))
            .await
        {
            Ok(MaybePreConfirmedBlockWithTxHashes::Block(origin_block)) => {
                let origin_header = BlockHeader {
                    block_hash: origin_block.block_hash,
                    parent_hash: origin_block.parent_hash,
                    block_number: BlockNumber(origin_block.block_number),
                    l1_gas_price: origin_block.l1_gas_price.into(),
                    l2_gas_price: origin_block.l2_gas_price.into(),
                    new_root: origin_block.new_root,
                    sequencer_address: ContractAddress::new_unchecked(
                        origin_block.sequencer_address,
                    ),
                    timestamp: BlockTimestamp(origin_block.timestamp),
                    starknet_version: origin_block.starknet_version,
                    l1_data_gas_price: origin_block.l1_data_gas_price.into(),
                    l1_da_mode: match origin_block.l1_da_mode {
                        ImportedL1DataAvailabilityMode::Calldata => {
                            L1DataAvailabilityMode::Calldata
                        }
                        ImportedL1DataAvailabilityMode::Blob => L1DataAvailabilityMode::Blob,
                    },
                    n_transactions: origin_block.transaction_count,
                    n_events: origin_block.event_count,
                    state_diff_length: origin_block.state_diff_length,
                    state_diff_commitment: StateDiffCommitment(PoseidonHash(
                        origin_block.state_diff_commitment,
                    )),
                    transaction_commitment: TransactionCommitment(
                        origin_block.transaction_commitment,
                    ),
                    event_commitment: EventCommitment(origin_block.event_commitment),
                    receipt_commitment: ReceiptCommitment(origin_block.receipt_commitment),
                };
                Ok(origin_header)
            }
            Err(ProviderError::StarknetError(
                starknet_rs_core::types::StarknetError::BlockNotFound,
            )) => Err(ApiError::BlockNotFound),
            other => Err(ApiError::StarknetDevnetError(
                starknet_core::error::Error::UnexpectedInternalError {
                    msg: format!("Failed retrieval of block from forking origin. Got: {other:?}"),
                },
            )),
        }
    }

    async fn get_local_block_header_by_id(&self, id: &BlockId) -> Result<BlockHeader, ApiError> {
        let starknet = self.api.starknet.lock().await;

        let block = match starknet.get_block(id) {
            Ok(block) => match block.status() {
                BlockStatus::Rejected => return Err(ApiError::BlockNotFound),
                _ => Ok::<_, ApiError>(block),
            },
            Err(Error::NoBlock) => Err(ApiError::BlockNotFound),
            Err(other) => Err(ApiError::StarknetDevnetError(other)),
        }?;

        Ok(block.into())
    }

    async fn resolve_start_block(
        &self,
        starting_block_id: BlockId,
    ) -> Result<(u64, Option<BlockHeader>), ApiError> {
        let starting_block_id = match starting_block_id {
            BlockId::Tag(BlockTag::PreConfirmed) => BlockId::Tag(BlockTag::Latest),
            other => other,
        };

        match starting_block_id {
            BlockId::Number(n) => Ok((n, None)),
            block_id => match self.get_local_block_header_by_id(&block_id).await {
                Ok(block) => Ok((block.block_number.0, None)),
                Err(ApiError::BlockNotFound) if self.origin_caller.is_some() => {
                    let origin_header = self.get_origin_block_header_by_id(block_id).await?;
                    Ok((origin_header.block_number.0, Some(origin_header)))
                }
                Err(other) => Err(other),
            },
        }
    }

    /// Rechecks the start block after origin I/O, without another origin request. A local hash
    /// removed by a concurrent revert is rejected instead of silently selecting its old height.
    async fn revalidate_start_block(
        &self,
        starting_block_id: BlockId,
        origin_header: Option<&BlockHeader>,
    ) -> Result<u64, ApiError> {
        let starting_block_id = match starting_block_id {
            BlockId::Tag(BlockTag::PreConfirmed) => BlockId::Tag(BlockTag::Latest),
            other => other,
        };
        match starting_block_id {
            BlockId::Number(n) => Ok(n),
            block_id => match self.get_local_block_header_by_id(&block_id).await {
                Ok(block) => Ok(block.block_number.0),
                Err(ApiError::BlockNotFound) => {
                    origin_header.map(|header| header.block_number.0).ok_or(ApiError::BlockNotFound)
                }
                Err(other) => Err(other),
            },
        }
    }

    /// Returns (starting local block number, latest block number, origin range). Checks both
    /// the initial range and the range at publication after origin I/O completes.
    async fn validate_block_number_range(
        &self,
        query_block_number: u64,
    ) -> Result<(u64, u64, Option<(u64, u64)>), ApiError> {
        let starknet = self.api.starknet.lock().await;
        let latest_block_number =
            starknet.get_block(&BlockId::Tag(BlockTag::Latest))?.block_number().0;
        drop(starknet);

        let (fork_url, fork_block_number) =
            (self.api.config.fork_config.url.clone(), self.api.config.fork_config.block_number);

        if query_block_number > latest_block_number {
            return Err(ApiError::BlockNotFound);
        }
        if latest_block_number - query_block_number > 1024 {
            return Err(ApiError::TooManyBlocksBack);
        }

        // Check if forking is configured and return the block range from the forking origin
        let origin_block_range = match (fork_url, fork_block_number) {
            (Some(_url), Some(fork_block_number)) => {
                // If the query block number is less than or equal to the fork block number,
                // we need to fetch blocks from the origin
                if query_block_number <= fork_block_number {
                    Some((query_block_number, fork_block_number))
                } else {
                    None
                }
            }
            _ => None, // No fork configuration or block number
        };

        let validated_start_block_number =
            if let Some(origin) = origin_block_range { origin.1 + 1 } else { query_block_number };

        Ok((validated_start_block_number, latest_block_number, origin_block_range))
    }

    async fn fetch_origin_heads(
        &self,
        start_block: u64,
        end_block: u64,
    ) -> Result<Vec<BlockHeader>, ApiError> {
        let mut headers = Vec::new();
        for block_n in start_block..=end_block {
            let block_id = BlockId::Number(block_n);
            headers.push(self.get_origin_block_header_by_id(block_id).await?);
        }
        Ok(headers)
    }

    /// starknet_subscribeNewHeads
    /// Checks if an optional block ID is provided. Validates that the block exists and is not too
    /// many blocks in the past. If it is a valid block, the user is notified of all blocks from the
    /// old up to the latest, and subscribed to new ones. If no block ID specified, the user is just
    /// subscribed to new blocks.
    async fn subscribe_new_heads(
        &self,
        prepared: PreparedNewHeads,
        rpc_request_id: Id,
        socket_id: SocketId,
    ) -> Result<(), ApiError> {
        let PreparedNewHeads {
            block_id,
            origin_header,
            origin_range: prepared_origin_range,
            mut origin_headers,
        } = prepared;
        let query_block_number =
            self.revalidate_start_block(block_id, origin_header.as_ref()).await?;
        let (local_start, latest_block_number, origin_range) =
            self.validate_block_number_range(query_block_number).await?;
        if origin_range != prepared_origin_range {
            return Err(ApiError::BlockNotFound);
        }

        // Notifying of old blocks. latest_block_number inclusive?
        // Yes, only if block_id != latest/pre-confirmed.
        if !matches!(block_id, BlockId::Tag(_)) {
            let starknet = self.api.starknet.lock().await;
            for block_n in local_start..=latest_block_number {
                let old_block = starknet
                    .get_block(&BlockId::Number(block_n))
                    .map_err(ApiError::StarknetDevnetError)?;
                origin_headers.push(old_block.into());
            }
        }

        let mut sockets = self.api.sockets.lock().await;
        let socket_context = sockets.get_mut(&socket_id)?;
        let subscription_id = socket_context.subscribe(rpc_request_id, Subscription::NewHeads);
        for header in origin_headers {
            let notification = NotificationData::NewHeads(header);
            socket_context.notify(subscription_id, &Subscription::NewHeads, notification);
        }

        Ok(())
    }

    /// Does not return TOO_MANY_ADDRESSES_IN_FILTER
    pub async fn subscribe_new_txs(
        &self,
        maybe_subscription_input: Option<TransactionSubscriptionInput>,
        rpc_request_id: Id,
        socket_id: SocketId,
    ) -> Result<(), ApiError> {
        let status_filter = StatusFilter::new(
            maybe_subscription_input
                .as_ref()
                .and_then(|input| input.finality_status.as_ref())
                .map_or_else(
                    || vec![TransactionFinalityStatus::AcceptedOnL2],
                    |statuses| {
                        statuses.iter().cloned().map(TransactionFinalityStatus::from).collect()
                    },
                ),
        );

        let address_filter = AddressFilter::new(
            maybe_subscription_input
                .as_ref()
                .and_then(|subscription_input| subscription_input.sender_address.clone())
                .unwrap_or_default(),
        );

        let tags = maybe_subscription_input
            .and_then(|subscription_input| subscription_input.tags)
            .unwrap_or_default();

        let mut sockets = self.api.sockets.lock().await;
        let socket_context = sockets.get_mut(&socket_id)?;

        let subscription = Subscription::NewTransactions { address_filter, status_filter, tags };
        socket_context.subscribe(rpc_request_id, subscription);

        Ok(())
    }

    /// Does not return TOO_MANY_ADDRESSES_IN_FILTER
    pub async fn subscribe_new_tx_receipts(
        &self,
        maybe_subscription_input: Option<TransactionReceiptSubscriptionInput>,
        rpc_request_id: Id,
        socket_id: SocketId,
    ) -> Result<(), ApiError> {
        let status_filter = StatusFilter::new(
            maybe_subscription_input
                .as_ref()
                .and_then(|input| input.finality_status.as_ref())
                .map_or_else(
                    || vec![TransactionFinalityStatus::AcceptedOnL2],
                    |statuses| {
                        statuses.iter().cloned().map(TransactionFinalityStatus::from).collect()
                    },
                ),
        );

        let address_filter = AddressFilter::new(
            maybe_subscription_input
                .and_then(|subscription_input| subscription_input.sender_address)
                .unwrap_or_default(),
        );

        let mut sockets = self.api.sockets.lock().await;
        let socket_context = sockets.get_mut(&socket_id)?;

        let subscription = Subscription::NewTransactionReceipts { address_filter, status_filter };
        socket_context.subscribe(rpc_request_id, subscription);

        Ok(())
    }

    async fn subscribe_tx_status(
        &self,
        transaction_hash: TransactionHash,
        rpc_request_id: Id,
        socket_id: SocketId,
    ) -> Result<(), ApiError> {
        // perform the actual subscription
        let mut sockets = self.api.sockets.lock().await;
        let socket_context = sockets.get_mut(&socket_id)?;

        let subscription = Subscription::TransactionStatus { transaction_hash };
        let subscription_id = socket_context.subscribe(rpc_request_id, subscription.clone());

        let starknet = self.api.starknet.lock().await;

        if let Ok(status) = starknet.get_transaction_execution_and_finality_status(transaction_hash)
        {
            let notification = NotificationData::TransactionStatus(NewTransactionStatus {
                transaction_hash,
                status,
            });
            socket_context.notify(subscription_id, &subscription, notification);
        } else if let Some(phase) = starknet.get_queued_transaction_phase(&transaction_hash) {
            use starknet_core::starknet::mempool::MempoolPhase;
            use starknet_types::rpc::transactions::{TransactionFinalityStatus, TransactionStatus};

            let finality_status = match phase {
                MempoolPhase::Received => TransactionFinalityStatus::Received,
                MempoolPhase::Candidate => TransactionFinalityStatus::Candidate,
                MempoolPhase::PreConfirmed => TransactionFinalityStatus::PreConfirmed,
            };
            let notification = NotificationData::TransactionStatus(NewTransactionStatus {
                transaction_hash,
                status: TransactionStatus::pre_execution(finality_status),
            });
            socket_context.notify(subscription_id, &subscription, notification);
        } else {
            tracing::debug!("Tx status subscription: tx not yet received")
        }

        Ok(())
    }

    async fn fetch_origin_events(
        &self,
        from_block: u64,
        to_block: u64,
        addresses: Option<Vec<ContractAddress>>,
        keys_filter: Option<Vec<Vec<Felt>>>,
    ) -> Result<Vec<EmittedEvent>, ApiError> {
        const DEFAULT_CHUNK_SIZE: u64 = 1000;
        let mut continuation_token: Option<String> = None;
        let mut all_events = Vec::new();

        // Fetch all events with pagination
        loop {
            let events_chunk = self
                .fetch_origin_events_chunk(
                    from_block,
                    to_block,
                    continuation_token,
                    addresses.clone(),
                    keys_filter.clone(),
                    DEFAULT_CHUNK_SIZE,
                )
                .await?;

            // Extend our collection with events from this chunk
            all_events.extend(events_chunk.events);

            // Update continuation token or break if done
            match events_chunk.continuation_token {
                Some(token) if token == "0" => break,
                Some(token) => continuation_token = Some(token),
                None => break,
            }
        }

        Ok(all_events)
    }

    async fn subscribe_events(
        &self,
        prepared: PreparedEvents,
        rpc_request_id: Id,
        socket_id: SocketId,
    ) -> Result<(), ApiError> {
        let PreparedEvents {
            input: maybe_subscription_input,
            block_id: starting_block_id,
            origin_header,
            origin_range: prepared_origin_range,
            origin_events,
        } = prepared;
        let addresses = maybe_subscription_input
            .as_ref()
            .and_then(|subscription_input| subscription_input.from_address.clone());

        let query_block_number =
            self.revalidate_start_block(starting_block_id, origin_header.as_ref()).await?;
        let (validated_start_block_number, _, origin_range) =
            self.validate_block_number_range(query_block_number).await?;
        if origin_range != prepared_origin_range {
            return Err(ApiError::BlockNotFound);
        }

        let keys_filter = maybe_subscription_input
            .as_ref()
            .and_then(|subscription_input| subscription_input.keys.clone());

        let finality_status = maybe_subscription_input
            .and_then(|subscription_input| subscription_input.finality_status)
            .unwrap_or(TransactionFinalityStatus::AcceptedOnL2);

        let subscription = Subscription::Events {
            addresses: addresses.clone(),
            keys_filter: keys_filter.clone(),
            status_filter: StatusFilter::new(vec![finality_status]),
        };

        // Get events from local chain
        let local_events = self.api.starknet.lock().await.get_unlimited_events(
            Some(BlockId::Number(validated_start_block_number)),
            Some(BlockId::Tag(BlockTag::PreConfirmed)), // Last block; filtering by status
            addresses,
            keys_filter,
            Some(finality_status),
        )?;

        let mut sockets = self.api.sockets.lock().await;
        let socket_context = sockets.get_mut(&socket_id)?;
        let subscription_id = socket_context.subscribe(rpc_request_id, subscription.clone());

        // Process origin events first (chronological order)
        for event in origin_events {
            let notification_data = NotificationData::Event(SubscriptionEmittedEvent {
                emitted_event: event,
                finality_status,
            });
            socket_context.notify(subscription_id, &subscription, notification_data);
        }

        // Process local events after origin events
        for event in local_events {
            let notification_data = NotificationData::Event(SubscriptionEmittedEvent {
                emitted_event: event,
                finality_status,
            });
            socket_context.notify(subscription_id, &subscription, notification_data);
        }

        Ok(())
    }
}
