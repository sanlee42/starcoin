/// Sync message which outbound
use crate::pool::TTLPool;
use crate::{do_duration, DELAY_TIME};
use actix::prelude::*;
use actix::{
    fut::wrap_future, fut::FutureWrap, Actor, Addr, AsyncContext, Context, Handler,
    ResponseActFuture,
};
use anyhow::{Error, Result};
use atomic_refcell::AtomicRefCell;
use bus::{Bus, BusActor, Subscription};
use chain::{ChainActor, ChainActorRef};
use crypto::hash::CryptoHash;
use futures::compat::Future01CompatExt;
use futures_locks::{Mutex, RwLock};
use futures_timer::Delay;
use itertools;
use network::sync_messages::{
    BatchBodyMsg, BatchHashByNumberMsg, BatchHeaderMsg, BlockBody, DataType, DownloadMessage,
    GetDataByHashMsg, GetHashByNumberMsg, HashWithBlockHeader, HashWithNumber, LatestStateMsg,
    ProcessMessage,
};
use network::{NetworkAsyncService, RPCMessage, RPCRequest, RPCResponse};
use std::borrow::BorrowMut;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use traits::{AsyncChain, ChainAsyncService};
use txpool::TxPoolRef;
use types::{
    block::{Block, BlockHeader},
    peer_info::PeerInfo,
};

#[derive(Clone)]
pub struct DownloadActor {
    downloader: Arc<RwLock<Downloader>>,
    peer_info: Arc<PeerInfo>,
    network: NetworkAsyncService<TxPoolRef>,
    bus: Addr<BusActor>,
}

impl DownloadActor {
    pub fn launch(
        peer_info: Arc<PeerInfo>,
        chain_reader: ChainActorRef<ChainActor>,
        network: NetworkAsyncService<TxPoolRef>,
        bus: Addr<BusActor>,
    ) -> Result<Addr<DownloadActor>> {
        let download_actor = DownloadActor {
            downloader: Arc::new(RwLock::new(Downloader::new(chain_reader))),
            peer_info,
            network,
            bus,
        };
        Ok(download_actor.start())
    }
}

impl Actor for DownloadActor {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        println!("download actor started.")
    }
}

impl Handler<DownloadMessage> for DownloadActor {
    type Result = ResponseActFuture<Self, Result<()>>;

    fn handle(&mut self, msg: DownloadMessage, ctx: &mut Self::Context) -> Self::Result {
        let downloader = self.downloader.clone();
        let my_peer_info = self.peer_info.id.clone();
        let network = self.network.clone();
        let fut = async move {
            match msg {
                DownloadMessage::LatestStateMsg(peer_info, latest_state_msg) => {
                    println!(
                        "latest_state_msg: {:?}",
                        &latest_state_msg.hash_header.header.number()
                    );
                    Downloader::handle_latest_state_msg(
                        downloader.clone(),
                        peer_info.clone(),
                        latest_state_msg,
                    )
                    .await;
                    let send_get_hash_by_number_msg =
                        Downloader::send_get_hash_by_number_msg(downloader.clone()).await;
                    match send_get_hash_by_number_msg {
                        Some((best_peer, get_hash_by_number_msg)) => {
                            let get_hash_by_number_req = RPCRequest::GetHashByNumberMsg(
                                ProcessMessage::GetHashByNumberMsg(get_hash_by_number_msg),
                            );
                            println!(
                                "best peer: {:?} : {:?} : {:?}",
                                best_peer.id.clone(),
                                my_peer_info,
                                peer_info.clone()
                            );
                            if let RPCResponse::BatchHashByNumberMsg(batch_hash_by_number_msg) =
                                network
                                    .clone()
                                    .send_request(
                                        best_peer.id.clone(),
                                        get_hash_by_number_req.clone(),
                                        do_duration(DELAY_TIME),
                                    )
                                    .await
                                    .unwrap()
                            {
                                println!("batch_hash_by_number_msg:{:?}", batch_hash_by_number_msg);
                                let hash_with_number = Downloader::find_ancestor(
                                    downloader.clone(),
                                    best_peer.clone(),
                                    batch_hash_by_number_msg,
                                )
                                .await;
                                println!("hash_with_number:{:?}", hash_with_number);
                                match hash_with_number {
                                    Some(_) => {
                                        let send_get_header_by_hash_msg =
                                            Downloader::send_get_header_by_hash_msg(
                                                downloader.clone(),
                                            )
                                            .await;
                                        match send_get_header_by_hash_msg {
                                            Some(get_data_by_hash_msg) => {
                                                let get_data_by_hash_req =
                                                    RPCRequest::GetDataByHashMsg(
                                                        ProcessMessage::GetDataByHashMsg(
                                                            get_data_by_hash_msg,
                                                        ),
                                                    );

                                                if let RPCResponse::BatchHeaderAndBodyMsg(
                                                    _,
                                                    headers,
                                                    bodies,
                                                ) = network
                                                    .clone()
                                                    .send_request(
                                                        best_peer.id.clone(),
                                                        get_data_by_hash_req.clone(),
                                                        do_duration(DELAY_TIME),
                                                    )
                                                    .await
                                                    .unwrap()
                                                {
                                                    Downloader::do_blocks(
                                                        downloader.clone(),
                                                        headers.headers,
                                                        bodies.bodies,
                                                    )
                                                    .await;
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        _ => {}
                    }
                }
                DownloadMessage::NewBlock(block) => {
                    println!("new block: {:?}", block.header().id());
                    Downloader::do_block(downloader.clone(), block).await;
                }
                _ => {}
            }

            Ok(())
        };

        Box::new(wrap_future::<_, Self>(fut))
    }
}

/// Send download message
pub struct Downloader {
    hash_pool: TTLPool<HashWithNumber>,
    header_pool: TTLPool<HashWithBlockHeader>,
    body_pool: TTLPool<BlockBody>,
    //    _network: Addr<NetworkActor>,
    peers: HashMap<PeerInfo, LatestStateMsg>,
    chain_reader: ChainActorRef<ChainActor>,
}

const HEAD_CT: u64 = 100;

impl Downloader {
    pub fn new(chain_reader: ChainActorRef<ChainActor>) -> Self {
        Downloader {
            hash_pool: TTLPool::new(),
            header_pool: TTLPool::new(),
            body_pool: TTLPool::new(),
            //            _network: network,
            peers: HashMap::new(),
            chain_reader,
        }
    }

    pub async fn handle_latest_state_msg(
        downloader: Arc<RwLock<Downloader>>,
        peer: PeerInfo,
        latest_state_msg: LatestStateMsg,
    ) {
        // let hash_num = HashWithNumber {
        //     hash: latest_state_msg.hash_header.hash.clone(),
        //     number: latest_state_msg.hash_header.header.number(),
        // };
        //        self.hash_pool
        //            .insert(peer.clone(), latest_state_msg.header.number(), hash_num);
        downloader
            .write()
            .compat()
            .await
            .unwrap()
            .peers
            .insert(peer, latest_state_msg.clone());
    }

    async fn best_peer(downloader: Arc<RwLock<Downloader>>) -> PeerInfo {
        let lock = downloader.read().compat().await.unwrap();
        assert!(lock.peers.len() > 0);
        let mut peer = None;
        lock.peers.keys().for_each(|p| peer = Some(p.clone()));

        peer.take().expect("best peer is none.")
    }

    pub async fn send_get_hash_by_number_msg(
        downloader: Arc<RwLock<Downloader>>,
    ) -> Option<(PeerInfo, GetHashByNumberMsg)> {
        let best_peer = Self::best_peer(downloader.clone()).await;
        let lock = downloader.read().compat().await.unwrap();
        //todo：binary search

        let latest_number = lock
            .chain_reader
            .clone()
            .current_header()
            .await
            .unwrap()
            .number();
        let number = lock
            .peers
            .get(&best_peer)
            .expect("Latest state is none.")
            .hash_header
            .header
            .number();
        if latest_number < number {
            let mut numbers = Vec::new();
            if number < HEAD_CT {
                for i in 0..(number + 1) {
                    numbers.push(i);
                }
            } else {
                for i in 0..HEAD_CT {
                    numbers.push((number - HEAD_CT + i + 1));
                }
            };

            Some((best_peer, GetHashByNumberMsg { numbers }))
        } else {
            None
        }
    }

    pub async fn find_ancestor(
        downloader: Arc<RwLock<Downloader>>,
        peer: PeerInfo,
        batch_hash_by_number_msg: BatchHashByNumberMsg,
    ) -> Option<HashWithNumber> {
        let mut lock = downloader.write().compat().await.unwrap();
        //TODO
        let mut exist_ancestor = false;
        let mut ancestor = None;
        let mut hashs = batch_hash_by_number_msg.hashs.clone();
        let mut not_exist_hash = Vec::new();
        hashs.reverse();
        let id = lock
            .chain_reader
            .clone()
            .current_header()
            .await
            .unwrap()
            .id();
        println!("find_ancestor header hash : {:?}", id);
        for hash in hashs {
            if lock
                .chain_reader
                .clone()
                .get_block_by_hash(&hash.hash)
                .await
                .is_some()
            {
                exist_ancestor = true;
                println!("find_ancestor is hash : {:?}", hash);
                ancestor = Some(hash);
                break;
            } else {
                println!("find_ancestor not hash : {:?}", hash);
                not_exist_hash.push(hash);
            }
        }

        if exist_ancestor {
            for hash in not_exist_hash {
                lock.borrow_mut()
                    .hash_pool
                    .insert(peer.clone(), hash.number.clone(), hash);
            }
        }
        ancestor
    }

    pub async fn send_get_header_by_hash_msg(
        downloader: Arc<RwLock<Downloader>>,
    ) -> Option<GetDataByHashMsg> {
        let mut lock = downloader.write().compat().await.unwrap();
        let hash_vec = lock.borrow_mut().hash_pool.take(100);
        if !hash_vec.is_empty() {
            let mut hashs = hash_vec.iter().map(|hash| hash.hash).collect();
            Some(GetDataByHashMsg {
                hashs,
                data_type: DataType::HEADER,
            })
        } else {
            None
        }
    }

    pub async fn handle_batch_header_msg(
        downloader: Arc<RwLock<Downloader>>,
        peer: PeerInfo,
        batch_header_msg: BatchHeaderMsg,
    ) {
        let mut lock = downloader.write().compat().await.unwrap();
        if !batch_header_msg.headers.is_empty() {
            for header in batch_header_msg.headers {
                lock.header_pool
                    .borrow_mut()
                    .insert(peer.clone(), header.header.number(), header);
            }
        }
    }

    pub async fn send_get_body_by_hash_msg(
        downloader: Arc<RwLock<Downloader>>,
    ) -> Option<GetDataByHashMsg> {
        let mut lock = downloader.write().compat().await.unwrap();
        let header_vec = lock.borrow_mut().header_pool.take(100);
        if !header_vec.is_empty() {
            let mut hashs = header_vec.iter().map(|header| header.hash).collect();
            Some(GetDataByHashMsg {
                hashs,
                data_type: DataType::BODY,
            })
        } else {
            None
        }
    }

    pub async fn do_blocks(
        downloader: Arc<RwLock<Downloader>>,
        headers: Vec<HashWithBlockHeader>,
        bodies: Vec<BlockBody>,
    ) {
        for (header, body) in itertools::zip_eq(headers, bodies) {
            let block = Block::new(header.header, body.transactions);
            //todo:verify block
            let _ = Self::do_block(downloader.clone(), block).await;
        }
    }

    pub async fn do_block(downloader: Arc<RwLock<Downloader>>, block: Block) {
        println!("do block {:?}", block.header().id());
        let lock = downloader.write().compat().await.unwrap();
        //todo:verify block
        let _ = lock.chain_reader.clone().try_connect(block).await;
    }
}
