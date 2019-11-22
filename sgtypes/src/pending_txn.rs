use crate::channel_transaction::{ChannelTransaction, ChannelTransactionProposal};
use crate::channel_transaction_sigs::ChannelTransactionSigs;
use libra_crypto::HashValue;
use libra_types::account_address::AccountAddress;
use libra_types::transaction::TransactionOutput;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Every Transaction Proposal should wait for others' signature,
/// TODO: should handle `agree or disagree`
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum PendingTransaction {
    WaitForSig {
        proposal: ChannelTransactionProposal,
        output: TransactionOutput,
        // TODO: or call it vote?
        signatures: BTreeMap<AccountAddress, ChannelTransactionSigs>,
    },

    WaitForApply {
        proposal: ChannelTransactionProposal,
        output: TransactionOutput,
        // TODO: or call it vote?
        signatures: BTreeMap<AccountAddress, ChannelTransactionSigs>,
    },
}

impl PendingTransaction {
    pub fn add_signature(&mut self, sig: ChannelTransactionSigs) {
        match self {
            PendingTransaction::WaitForSig { signatures, .. } => {
                signatures.insert(sig.address, sig);
            }
            PendingTransaction::WaitForApply { .. } => {
                // TODO: debug_assert
            }
        }
    }
    pub fn get_signature(&self, address: &AccountAddress) -> Option<ChannelTransactionSigs> {
        match self {
            PendingTransaction::WaitForSig { signatures, .. } => signatures.get(address).cloned(),
            PendingTransaction::WaitForApply { .. } => signatures.get(address).cloned(),
        }
    }

    pub fn fullfilled(&self) -> bool {
        match self {
            PendingTransaction::WaitForApply { .. } => true,
            _ => false,
        }
    }
    pub fn try_fullfill(&mut self, participants: &[AccountAddress]) {
        match self {
            PendingTransaction::WaitForSig {
                signatures,
                output,
                proposal,
            } => {
                if signatures.len() == participants.len() {
                    if participants
                        .iter()
                        .all(|addr| signatures.contains_key(addr))
                    {
                        *self = PendingTransaction::WaitForApply {
                            proposal: proposal.clone(),
                            signatures: signatures.clone(),
                            output: output.clone(),
                        };
                    }
                }
            }
            PendingTransaction::WaitForApply { .. } => {
                // TODO: debug_assert
            }
        };
        self.fullfilled()
    }

    pub fn request_id(&self) -> HashValue {
        //        match self {
        //            PendingTransaction::WaitForSig { proposal, .. } => proposal.channel_txn.hash(),
        //            PendingTransaction::WaitForApply { proposal, .. } => proposal.channel_txn.hash(),
        //        }
        unimplemented!()
    }
}
