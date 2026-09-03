use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::transaction_execution::Transaction as ExecutableTransaction;
use serde::{Deserialize, Serialize};
use starknet_api::core::Nonce;
use starknet_types::contract_address::ContractAddress;
use starknet_types::contract_class::ContractClass;
use starknet_types::felt::{ClassHash, CompiledClassHash};
use starknet_types::rpc::transactions::TransactionWithHash;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MempoolPhase {
    Received,
    Candidate,
    PreConfirmed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MempoolLane {
    User,
    System,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingDeclaration {
    pub class_hash: ClassHash,
    pub casm_hash: Option<CompiledClassHash>,
    pub contract_class: ContractClass,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedTransaction {
    pub transaction: TransactionWithHash,
    pub executable: ExecutableTransaction,
    pub declaration: Option<PendingDeclaration>,
    pub account_address: Option<ContractAddress>,
    pub nonce: Option<Nonce>,
    pub tip: u64,
    pub max_l2_gas_price: u128,
    pub lane: MempoolLane,
}

impl PreparedTransaction {
    pub(crate) fn account(
        transaction: TransactionWithHash,
        executable: AccountTransaction,
        declaration: Option<PendingDeclaration>,
    ) -> Self {
        let account_address = Some(executable.sender_address().into());
        let nonce = Some(executable.nonce());
        let tip = executable.tip().0;
        let max_l2_gas_price = executable.resource_bounds().get_l2_bounds().max_price_per_unit.0;
        Self {
            transaction,
            executable: ExecutableTransaction::Account(executable),
            declaration,
            account_address,
            nonce,
            tip,
            max_l2_gas_price,
            lane: MempoolLane::User,
        }
    }

    pub(crate) fn system(
        transaction: TransactionWithHash,
        executable: starknet_api::executable_transaction::L1HandlerTransaction,
    ) -> Self {
        Self {
            transaction,
            executable: ExecutableTransaction::L1Handler(executable),
            declaration: None,
            account_address: None,
            nonce: None,
            tip: 0,
            max_l2_gas_price: 0,
            lane: MempoolLane::System,
        }
    }

    /// Admin/system account-tx variant used by `devnet_mint` and similar administrative actions.
    /// Behaves identically to `account` except for the lane marker, which causes
    /// `submit_system_prepared_transaction` to execute regardless of the configured block
    /// generation mode, so balance changes remain immediately observable in `mempool` mode.
    pub(crate) fn system_account(
        transaction: TransactionWithHash,
        executable: AccountTransaction,
        declaration: Option<PendingDeclaration>,
    ) -> Self {
        let account_address = Some(executable.sender_address().into());
        let nonce = Some(executable.nonce());
        let tip = executable.tip().0;
        let max_l2_gas_price = executable.resource_bounds().get_l2_bounds().max_price_per_unit.0;
        Self {
            transaction,
            executable: ExecutableTransaction::Account(executable),
            declaration,
            account_address,
            nonce,
            tip,
            max_l2_gas_price,
            lane: MempoolLane::System,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MempoolEntry {
    pub transaction: TransactionWithHash,
    pub phase: MempoolPhase,
    pub arrival_id: u64,
    pub account_address: Option<ContractAddress>,
    pub nonce: Option<Nonce>,
    pub tip: u64,
    pub max_l2_gas_price: u128,
    pub lane: MempoolLane,
    pub(crate) prepared: PreparedTransaction,
}
