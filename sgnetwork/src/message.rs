use network_libp2p::CustomMessage;
use parity_codec::{Encode, Decode};
use crypto::{
    hash::{
        CryptoHash, CryptoHasher, TestOnlyHasher,
    },
    HashValue,
};
use types::account_address::AccountAddress;
use crate::helper::get_unix_ts;

#[derive(Clone, Debug)]
pub struct NetworkMessage {
    pub peer_id: AccountAddress,
    pub msg: Message,
}

impl CryptoHash for NetworkMessage {
    type Hasher = TestOnlyHasher;

    fn hash(&self) -> HashValue {
        let mut state = Self::Hasher::default();
        let mut bytes_vec = self.peer_id.to_vec();
        bytes_vec.extend_from_slice(&self.msg.clone().into_bytes());
        state.write(&bytes_vec);
        state.finish()
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Encode, Decode)]
pub enum Message {
    ACK(u128),
    Payload(PayloadMsg),
}

#[derive(Debug, PartialEq, Eq, Clone, Encode, Decode)]
pub struct PayloadMsg {
    pub id: u128,
    pub data: Vec<u8>,
}

impl CustomMessage for Message
    where Self: Decode + Encode
{
    fn into_bytes(self) -> Vec<u8> {
        self.encode()
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, ()> where Self: Sized {
        Decode::decode(&mut &bytes[..]).ok_or(())
    }
}

impl Message {
    pub fn new_ack(message_id: u128) -> Message {
        Message::ACK(message_id)
    }

    pub fn new_payload(data: Vec<u8>) -> (Message, u128) {
        let message_id = get_unix_ts();
        (Message::Payload(PayloadMsg { id: message_id, data }), message_id)
    }
    pub fn new_message(data: Vec<u8>) -> Message {
        Message::Payload(PayloadMsg { id: 0, data })
    }
}
