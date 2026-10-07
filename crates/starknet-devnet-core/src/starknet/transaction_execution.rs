use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::transaction_execution::Transaction as ExecutableTransaction;
use blockifier::transaction::transactions::ExecutableTransaction as _;
use starknet_types::contract_class::ContractClass;
use starknet_types::felt::{ClassHash, CompiledClassHash, TransactionHash};
use starknet_types::rpc::transactions::TransactionWithHash;

use super::Starknet;
use crate::error::{DevnetResult, Error};
use crate::state::CustomState;

pub(super) struct PendingDeclaration {
    pub class_hash: ClassHash,
    pub casm_hash: Option<CompiledClassHash>,
    pub contract_class: ContractClass,
}

/// Prepared input shared by transaction submission paths, before stateful execution.
pub(super) struct PreparedTransaction {
    transaction: TransactionWithHash,
    executable: ExecutableTransaction,
    declaration: Option<PendingDeclaration>,
}

impl PreparedTransaction {
    pub(super) fn account(
        transaction: TransactionWithHash,
        executable: AccountTransaction,
        declaration: Option<PendingDeclaration>,
    ) -> Self {
        Self { transaction, executable: ExecutableTransaction::Account(executable), declaration }
    }

    pub(super) fn l1_handler(
        transaction: TransactionWithHash,
        executable: starknet_api::executable_transaction::L1HandlerTransaction,
    ) -> Self {
        Self {
            transaction,
            executable: ExecutableTransaction::L1Handler(executable),
            declaration: None,
        }
    }
}

impl Starknet {
    pub(super) fn submit_prepared_transaction(
        &mut self,
        prepared: PreparedTransaction,
    ) -> DevnetResult<TransactionHash> {
        let transaction_hash = *prepared.transaction.get_transaction_hash();
        if self.transactions.get(&transaction_hash).is_some() {
            return Err(Error::DuplicateTransaction { transaction_hash });
        }

        let execution_info = prepared
            .executable
            .execute(&mut self.pre_confirmed_state.state, &self.block_context)?;

        if !execution_info.is_reverted()
            && let Some(declaration) = prepared.declaration
        {
            self.pre_confirmed_state.declare_contract_class(
                declaration.class_hash,
                declaration.casm_hash,
                declaration.contract_class,
            )?;
        }

        self.append_accepted_transaction(prepared.transaction, execution_info)?;
        if !self.config.uses_pre_confirmed_block() {
            self.generate_new_block_and_state();
        }
        Ok(transaction_hash)
    }
}
