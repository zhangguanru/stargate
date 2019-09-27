// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

use admission_control_proto::proto::admission_control_grpc::create_admission_control;
use admission_control_service::{admission_control_service::AdmissionControlService,
                                admission_control_client::AdmissionControlClient};
use config::config::{NetworkConfig, NodeConfig, RoleType};
use crypto::{ed25519::*, ValidKey, HashValue};
use execution_proto::proto::execution_grpc;
use execution_service::ExecutionService;
use futures03::future::{FutureExt, TryFutureExt};
use grpc_helpers::ServerHandle;
use grpcio::{ChannelBuilder, EnvBuilder, ServerBuilder};
use grpcio_sys;
use logger::prelude::*;
use mempool::{core_mempool_client::CoreMemPoolClient, proto::{mempool_grpc::MempoolClient, mempool::{TransactionExclusion, GetBlockRequest}}, MempoolRuntime};
use metrics::metric_server;
use std::{
    cmp::min,
    convert::{TryFrom, TryInto},
    str::FromStr,
    sync::Arc,
    thread,
    time::{Instant, Duration},
};
use storage_client::{StorageRead, StorageWrite, StorageReadServiceClient, StorageWriteServiceClient};
use storage_service::start_storage_service_and_return_service;
use tokio::runtime::{Builder, Runtime};
use types::proto::{transaction::SignedTransactionsBlock, validator_set::ValidatorSet, ledger_info::{LedgerInfoWithSignatures, LedgerInfo}};
use execution_proto::proto::execution::{CommitBlockRequest, ExecuteBlockRequest};
use vm_validator::vm_validator::VMValidator;
use tokio_timer::Interval;
use futures::{Stream, Future};
use mempool::proto::mempool_client::MempoolClientTrait;
use crypto::hash::GENESIS_BLOCK_ID;
use admission_control_proto::proto::admission_control_client::AdmissionControlClientTrait;
use types::{account_state_blob::AccountStateBlob, proof::SparseMerkleProof, transaction::{SignedTransactionWithProof, RawTransaction, SignedTransaction, Version},
            account_config::{association_address, AccountResource}, account_address::AccountAddress, get_with_proof::RequestItem};
use core::borrow::Borrow;
use proto_conv::FromProto;
use lazy_static::lazy_static;
use std::sync::Mutex;
use std::thread::sleep;

pub struct StarHandle {
    _storage: ServerHandle,
}

lazy_static! {
    static ref LATEST_BLOCK_HASH: Mutex<Vec<HashValue>> = Mutex::new(vec![*GENESIS_BLOCK_ID]);
}

fn setup_ac<R>(config: &NodeConfig, r: Arc<R>) -> (AdmissionControlClient<CoreMemPoolClient, VMValidator>, CoreMemPoolClient) where R: StorageRead + Clone + 'static {
    let env = Arc::new(
        EnvBuilder::new()
            .name_prefix("grpc-ac-")
            .cq_count(unsafe { min(grpcio_sys::gpr_cpu_num_cores() as usize * 2, 32) })
            .build(),
    );
    let port = config.admission_control.admission_control_service_port;
    let mempool = CoreMemPoolClient::new(&config);
    let mempool_client = Some(Arc::new(mempool.clone()));

    let storage_read_client = Arc::clone(&r);
    let vm_validator = Arc::new(VMValidator::new(&config, storage_read_client));

    let storage_read_client = Arc::clone(&r);
    let handle = AdmissionControlService::new(
        mempool_client,
        storage_read_client,
        vm_validator,
        config
            .admission_control
            .need_to_check_mempool_before_validation,
    );

    (AdmissionControlClient::new(handle), mempool)
}

fn setup_executor<R, W>(config: &NodeConfig, r: Arc<R>, w: Arc<W>) -> ExecutionService where R: StorageRead + 'static, W: StorageWrite + 'static {
    let client_env = Arc::new(EnvBuilder::new().name_prefix("grpc-exe-sto-").build());
    ExecutionService::new(r, w, config)
}

pub fn setup_environment(node_config: &mut NodeConfig) -> (AdmissionControlClient<CoreMemPoolClient, VMValidator>, StarHandle) {
    crash_handler::setup_panic_handler();

    let mut instant = Instant::now();
    let (storage, storage_service) = start_storage_service_and_return_service(&node_config);
    debug!(
        "Storage service started in {} ms",
        instant.elapsed().as_millis()
    );

    instant = Instant::now();
    let execution_service = setup_executor(&node_config, Arc::clone(&storage_service), Arc::clone(&storage_service));
    debug!(
        "Execution service started in {} ms",
        instant.elapsed().as_millis()
    );

    // Initialize and start AC.
    instant = Instant::now();
    let (ac_client, mempool_client) = setup_ac(&node_config, Arc::clone(&storage_service));
    debug!("AC started in {} ms", instant.elapsed().as_millis());

    commit_block(ac_client.clone(), mempool_client, execution_service);
    let star_handle = StarHandle {
        _storage: storage,
    };
    (ac_client, star_handle)
}

fn commit_block(ac_client: AdmissionControlClient<CoreMemPoolClient, VMValidator>, mempool_client: CoreMemPoolClient, execution_service: ExecutionService) {
    let task = Interval::new(Instant::now(), Duration::from_secs(3))
        .for_each(move |_| {
            let mut block_req = GetBlockRequest::new();
            block_req.set_max_block_size(1);
            let block_resp = mempool_client.get_block(&block_req).expect("get_block err.");
            let block = block_resp.get_block();
            let mut txns = block.get_transactions();

            println!("txn size: {:?} of current block.", txns.len());

            if txns.len() > 0 {
                let mut tmp_txn_vec = vec![];
                let mut txn_exc_vec = vec![];
                txns.clone().iter().for_each(|txn| {
                    let tmp = SignedTransaction::from_proto(txn.clone()).expect("from pb err.");

                    let mut txn_exc = TransactionExclusion::new();
                    txn_exc.set_sender(tmp.sender().to_vec());
                    txn_exc.set_sequence_number(tmp.sequence_number());
                    txn_exc_vec.push(txn_exc);

                    tmp_txn_vec.push(tmp);
                });

                // exe
                let repeated = ::protobuf::RepeatedField::from_vec(txns.to_vec());
                let mut exe_req = ExecuteBlockRequest::new();
                let block_id = HashValue::random();
                let pre_block_id = HashValue::random();
                exe_req.set_transactions(repeated);

                let len = LATEST_BLOCK_HASH.lock().unwrap().len();
                println!("block hight: {:?}", len);
                let latest_hash = LATEST_BLOCK_HASH.lock().unwrap().get(len - 1).unwrap().clone();

                println!("new block hash: {:?}", latest_hash);

                exe_req.set_parent_block_id(latest_hash.to_vec());


                exe_req.set_block_id(block_id.to_vec());
                let exe_resp = execution_service.execute_block_inner(exe_req.clone());

                LATEST_BLOCK_HASH.lock().unwrap().push(block_id);

                // commit
                let mut info = LedgerInfo::new();
                info.set_version(exe_resp.get_version());
                info.set_consensus_block_id(exe_req.get_block_id().to_vec());
                info.set_consensus_data_hash(HashValue::random().to_vec());
                info.set_epoch_num(0);
                info.set_next_validator_set(ValidatorSet::default());
                info.set_timestamp_usecs(u64::max_value());
                info.set_transaction_accumulator_hash(exe_resp.get_root_hash().to_vec());
                let mut info_sign = LedgerInfoWithSignatures::new();
                //        exe_resp.get_validators()
                //        info.set_signatures()
                info_sign.set_ledger_info(info);
                let mut req = CommitBlockRequest::new();
                req.set_ledger_info_with_sigs(info_sign.clone());
                execution_service.commit_block_inner(req);

                // remove from mem pool
                let mut remove_req = GetBlockRequest::new();

                let repeated_txn_exc = ::protobuf::RepeatedField::from_vec(txn_exc_vec);
                remove_req.set_transactions(repeated_txn_exc);

                mempool_client.remove_txn(&remove_req);
            }
            Ok(())
        }).map_err(|e| { panic!("interval errored; err={:?}", e) });

    thread::spawn(move || { tokio::run(task) });
}