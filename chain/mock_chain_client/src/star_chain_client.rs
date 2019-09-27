use std::sync::Arc;
use crate::ChainClient;
use failure::prelude::*;
use types::{account_config::{association_address, AccountResource}, account_address::AccountAddress,
            transaction::{Version, SignedTransaction, RawTransaction, SignedTransactionWithProof},
            proof::SparseMerkleProof, get_with_proof::RequestItem, account_state_blob::AccountStateBlob,
            proto::get_with_proof::{ResponseItem, UpdateToLatestLedgerRequest, UpdateToLatestLedgerResponse}};
use admission_control_proto::proto::{admission_control::SubmitTransactionRequest,
                                     admission_control_client::AdmissionControlClientTrait,
                                     admission_control_grpc::AdmissionControlClient};
use admission_control_service::admission_control_client::AdmissionControlClient as MockAdmissionControlClient;
use core::borrow::Borrow;
use proto_conv::{IntoProto, FromProto};
use std::convert::TryInto;
use vm_genesis::{encode_transfer_script, encode_create_account_script, GENESIS_KEYPAIR};
use std::time::Duration;
use grpcio::{EnvBuilder, ChannelBuilder};
use config::trusted_peers::ConfigHelpers;
use executable_helpers::helpers::{
    setup_executable, ARG_CONFIG_PATH, ARG_DISABLE_LOGGING, ARG_PEER_ID, load_configs_from_args,
};
use crate::mock_star_node::{setup_environment, StarHandle};
use clap::ArgMatches;
use mempool::core_mempool_client::CoreMemPoolClient;
use vm_validator::vm_validator::VMValidator;

pub struct StarChainClient<C> {
    ac_client: Arc<C>
}

impl<C> StarChainClient<C> where C: AdmissionControlClientTrait {
    pub fn new(c: C) -> Self {
        StarChainClient { ac_client: Arc::new(c) }
    }

    fn do_request(&self, req: &UpdateToLatestLedgerRequest) -> UpdateToLatestLedgerResponse {
        self.ac_client.update_to_latest_ledger(req).expect("Call update_to_latest_ledger err.")
    }

    fn get_account_state_with_proof_inner(&self, account_address: &AccountAddress, version: Option<Version>)
                                          -> Result<(Version, Option<Vec<u8>>, SparseMerkleProof)> {
        let req = RequestItem::GetAccountState { address: account_address.clone() };
        let resp = parse_response(self.do_request(&build_request(req, version)));
        let proof = resp.get_get_account_state_response().get_account_state_with_proof();
        let blob = if proof.has_blob() {
            Some(proof.get_blob().get_blob().to_vec())
        } else {
            None
        };
        Ok((proof.version, blob, SparseMerkleProof::from_proto(
            proof.get_proof().get_transaction_info_to_account_proof().clone())
            .expect("SparseMerkleProof parse from proto err.")))
    }

    fn account_exist(&self, account_address: &AccountAddress, version: Option<Version>) -> bool {
        match self.get_account_state_with_proof_inner(account_address, version).expect("get account state err.").1 {
            Some(blob) => true,
            None => false
        }
    }

    pub fn account_sequence_number(&self, account_address: &AccountAddress) -> Option<Version> {
        match self.get_account_state_with_proof_inner(account_address, None).expect("get account state err.").1 {
            Some(blob) => {
                let a_s_b = AccountStateBlob::from(blob);
                let account_btree = a_s_b.borrow().try_into().expect("blob to btree err.");
                let account_resource = AccountResource::make_from(&account_btree).expect("make account resource err.");
                Some(account_resource.sequence_number())
            }
            None => None
        }
    }
}

impl<C> ChainClient for StarChainClient<C> where C: AdmissionControlClientTrait {
    fn get_account_state_with_proof(&self, account_address: &AccountAddress, version: Option<Version>)
                                    -> Result<(Version, Option<Vec<u8>>, SparseMerkleProof)> {
        self.get_account_state_with_proof_inner(account_address, version)
    }

    fn faucet(&self, receiver: AccountAddress, amount: u64) -> Result<()> {
        let exist_flag = self.account_exist(&receiver, None);
        let script = if !exist_flag {
            encode_create_account_script(&receiver, amount)
        } else {
            encode_transfer_script(&receiver, amount)
        };

        let sender = association_address();
        let s_n = self.account_sequence_number(&sender).expect("seq num is none.");
        let signed_tx = RawTransaction::new_script(
            sender.clone(),
            s_n,
            script,
            1000_000 as u64,
            1 as u64,
            Duration::from_secs(u64::max_value()),
        ).sign(&GENESIS_KEYPAIR.0, GENESIS_KEYPAIR.1.clone())
            .unwrap()
            .into_inner();

        self.submit_transaction(signed_tx);
        Ok(())
    }

    fn submit_transaction(&self, signed_transaction: SignedTransaction) -> Result<()> {
        let mut req = SubmitTransactionRequest::new();
        req.set_signed_txn(signed_transaction.into_proto());
        self.ac_client.submit_transaction(&req).expect("submit txn err.");
        Ok(())
    }

    fn watch_transaction(&self, address: &AccountAddress, seq: u64) -> Result<Option<SignedTransactionWithProof>> {
        unimplemented!()
    }

    fn get_transaction_by_seq_num(&self, account_address: &AccountAddress, seq_num: u64) -> Result<Option<SignedTransactionWithProof>> {
        let req = RequestItem::GetAccountTransactionBySequenceNumber { account: account_address.clone(), sequence_number: seq_num, fetch_events: false };
        let mut resp = parse_response(self.do_request(&build_request(req, None)));
        let mut tmp = resp.take_get_account_transaction_by_sequence_number_response();
        if tmp.has_signed_transaction_with_proof() {
            let proof = tmp.take_signed_transaction_with_proof();
            Ok(Some(SignedTransactionWithProof::from_proto(proof).expect("SignedTransaction parse from proto err.")))
        } else {
            Ok(None)
        }
    }
}

pub fn build_request(req: RequestItem, ver: Option<Version>) -> UpdateToLatestLedgerRequest {
    let mut repeated = ::protobuf::RepeatedField::new();
    repeated.push(req.into_proto());
    let mut req = UpdateToLatestLedgerRequest::new();
    req.set_requested_items(repeated);
    match ver {
        Some(v) => req.set_client_known_version(v),
        None => {}
    }

    req
}

pub fn parse_response(resp: UpdateToLatestLedgerResponse) -> ResponseItem {
    resp.get_response_items().get(0).expect("response item is none.").clone()
}

pub fn create_star_client(host: &str, port: u32) -> AdmissionControlClient {
    let conn_addr = format!("{}:{}", host, port);
    let env = Arc::new(EnvBuilder::new().name_prefix("ac-grpc-client-").build());
    let ch = ChannelBuilder::new(env).connect(&conn_addr);
    AdmissionControlClient::new(ch)
}

pub fn mock_star_client() -> (MockAdmissionControlClient<CoreMemPoolClient, VMValidator>, StarHandle) {
    let args = ArgMatches::default();
    let mut config = load_configs_from_args(&args);
    if config.consensus.get_consensus_peers().len() == 0 {
        let (_, single_peer_consensus_config) = ConfigHelpers::get_test_consensus_config(1, None);
        config.consensus.consensus_peers = single_peer_consensus_config;
        let genesis_path = star_node::genesis::genesis_blob();
        config.execution.genesis_file_location = genesis_path;
    }

    setup_environment(&mut config)
}