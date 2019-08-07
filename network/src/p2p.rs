use std::{
    net::SocketAddr,
};

use futures::{
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
    io::{AsyncRead, AsyncWrite},
    stream::Stream,
    Future,
};

use crate::error::Error;
use sg_config::config::NetworkConfig;

pub type NetConfig = NetworkConfig;


pub struct TSocket {
    incoming: UnboundedReceiver<Vec<u8>>,
    outgoing: UnboundedSender<Vec<u8>>,
    addr: SocketAddr,
}


pub trait TTcpSteam: AsyncWrite + AsyncRead {}

pub trait Network<T, S, F>
    where T: TTcpSteam, F: Future<Output=T>, S: Stream<Item=T>
{
    fn start(net_cfg: NetConfig) -> Result<(), Error>;
    fn stop() -> Result<(), Error>;
    fn join(forward: bool, peer_id: String) -> Result<(), Error>;
    fn connect(addr: SocketAddr) -> Result<F, Error>;
    fn listen() -> Result<S, Error>;
}

pub fn new_network<T, F, S, N>(net_cfg: NetConfig) -> N
    where T: TTcpSteam, F: Future<Output=T>, S: Stream<Item=T>, N: Network<T, S, F>
{
    unimplemented!()
}