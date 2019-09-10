#[cfg(test)]
mod tests {
    use std::{str::FromStr, thread, time::{Instant, Duration}};
    use futures::{
        future::Future,
        stream,
        stream::Stream,
        sync::mpsc::{UnboundedReceiver, UnboundedSender},
    };
    use hex;
    use libp2p::multihash;
    use rand::prelude::*;
    use tokio::{prelude::Async, runtime::{TaskExecutor, Runtime}, timer::{Delay, Interval}};

    use crypto::{
        ed25519::{compat, Ed25519PrivateKey, Ed25519PublicKey},
        test_utils::KeyPair,
        Uniform,
    };
    use logger::prelude::*;
    use network_libp2p::{CustomMessage, identity, NodeKeyConfig, PeerId, PublicKey, Secret};
    use types::account_address::AccountAddress;

    use crate::{
        build_network_service, convert_account_address_to_peer_id,
        convert_peer_id_to_account_address, Message, NetworkComponent, NetworkMessage,
        NetworkService,
    };
    use crate::helper::convert_boot_nodes;
    use crate::message::PayloadMsg;

    fn build_test_network_pair(executor: TaskExecutor) -> (NetworkComponent, NetworkComponent) {
        let mut l = build_test_network_services(2, 50400, executor).into_iter();
        let a = l.next().unwrap();
        let b = l.next().unwrap();
        (a, b)
    }

    fn build_test_network_services(
        num: usize,
        base_port: u16,
        executor: TaskExecutor,
    ) -> Vec<(
        NetworkService,
        UnboundedSender<NetworkMessage>,
        UnboundedReceiver<NetworkMessage>,
    )> {
        let mut result: Vec<(
            NetworkService,
            UnboundedSender<NetworkMessage>,
            UnboundedReceiver<NetworkMessage>,
        )> = Vec::with_capacity(num);
        let mut first_addr = None::<String>;
        for index in 0..num {
            let mut boot_nodes = Vec::new();
            let key_pair = {
                let mut rng: StdRng = SeedableRng::seed_from_u64(index as u64);
                KeyPair::<Ed25519PrivateKey, Ed25519PublicKey>::generate_for_testing(&mut rng)
            };

            if let Some(first_addr) = first_addr.as_ref() {
                boot_nodes.push(format!(
                    "{}/p2p/{}",
                    first_addr,
                    hex::encode(result[0].0.identify())
                ));
            }
            let config = sg_config::config::NetworkConfig {
                listen: format!("/ip4/127.0.0.1/tcp/{}", base_port + index as u16),
                seeds: boot_nodes,
            };
            println!("listen:{:?},boots {:?}", config.listen, config.seeds);
            if first_addr.is_none() {
                first_addr = Some(config.listen.clone().parse().unwrap());
            }

            let server = build_network_service(&config, key_pair, executor.clone());
            result.push({
                let c: NetworkComponent = server;
                c
            });
        }
        result
    }

    #[test]
    fn test_send_receive_1() {
        ::logger::init_for_e2e_testing();
        env_logger::init();

        let rt = Runtime::new().unwrap();
        let executor = rt.executor();
        let ((service1, tx1, rx1), (service2, tx2, rx2)) =
            build_test_network_pair(executor.clone());
        let msg_peer_id = service1.identify();
        // Once sender has been droped, the select_all will return directly. clone it to prevent it.
        let tx22 = tx2.clone();
        let sender_fut = Interval::new(Instant::now(), Duration::from_millis(50))
            .take(3)
            .map_err(|e| ())
            .for_each(move |_| {
                let (message, _) = Message::new_payload(vec![1, 0]);
                match tx2.unbounded_send(NetworkMessage {
                    peer_id: msg_peer_id,
                    msg: message,
                }) {
                    Ok(()) => Ok(()),
                    Err(e) => Err(()),
                }
            });
        let receive_fut = rx1.for_each(|msg| {
            println!("{:?}", msg);
            Ok(())
        });
        executor.clone().spawn(receive_fut);
        executor.clone().spawn(sender_fut);
        let task = Delay::new(Instant::now() + Duration::from_millis(1000))
            .and_then(move |_| {
                drop(service1);
                drop(service2);
                Ok(())
            })
            .map_err(|e| panic!("delay errored; err={:?}", e));
        executor.spawn(task);
        rt.shutdown_on_idle().wait().unwrap();
    }

    #[test]
    fn test_send_receive_2() {
        ::logger::init_for_e2e_testing();
        let rt = Runtime::new().unwrap();
        let executor = rt.executor();
        let ((service1, tx1, rx1), (mut service2, tx2, rx2)) =
            build_test_network_pair(executor.clone());
        let msg_peer_id = service1.identify();
        let receive_fut = rx1.for_each(|msg| {
            println!("{:?}", msg);
            Ok(())
        });
        executor.clone().spawn(receive_fut);

        //wait the network started.
        thread::sleep(Duration::from_secs(1));
        for x in 0..10 {
            let _ = service2.send_message_block(msg_peer_id, "starcoiniscoming".into());
        }
    }

    #[test]
    fn test_spawn() {
        let rt = Runtime::new().unwrap();
        let executor = rt.executor();
        let task = Delay::new(Instant::now() + Duration::from_millis(1000))
            .and_then(move |_| {
                println!("hello spawn");
                Ok(())
            })
            .map_err(|e| panic!("delay errored; err={:?}", e));
        executor.spawn(task);
        thread::sleep(Duration::from_secs(2));
        //rt.shutdown_on_idle().wait().unwrap();
    }

    #[test]
    fn test_generate_account_and_peer_id() {
        let (private_key, public_key) = compat::generate_keypair(None);

        let mut cfg = network_libp2p::NetworkConfiguration::new();
        let seckey = identity::ed25519::SecretKey::from_bytes(&mut private_key.to_bytes()).unwrap();
        cfg.node_key = NodeKeyConfig::Ed25519(Secret::Input(seckey));
        let libp2p_public_key = cfg.node_key.into_keypair().unwrap().public();
        let libp2p_public_key_byte;
        if let PublicKey::Ed25519(key) = libp2p_public_key {
            libp2p_public_key_byte = key.encode();
            assert_eq!(libp2p_public_key_byte, public_key.to_bytes());
        } else {
            panic!("failed");
        }

        let address = AccountAddress::from_public_key(&public_key).to_vec();
        let peer_id = multihash::encode(multihash::Hash::SHA3256, &public_key.to_bytes())
            .unwrap()
            .into_bytes();
        PeerId::from_bytes(peer_id.clone()).unwrap();
        assert_eq!(address, &peer_id[2..]);
    }

    #[test]
    fn test_connected_nodes() {
        let rt = Runtime::new().unwrap();
        let executor = rt.executor();
        let (service1, service2) = build_test_network_pair(executor);
        thread::sleep(Duration::new(1, 0));
        for (peer_id, peer) in service1.0.libp2p_service.lock().state().connected_peers {
            println!("id: {:?}, peer: {:?}", peer_id, peer);
            assert_eq!(peer.open, true);
        }
        assert_eq!(
            AccountAddress::from_str(&hex::encode(service1.0.identify())).unwrap(),
            service1.0.identify()
        );
    }

    #[test]
    fn test_convert_address_peer_id() {
        let (private_key, public_key) = compat::generate_keypair(Option::None);
        let seckey = identity::ed25519::SecretKey::from_bytes(&mut private_key.to_bytes()).unwrap();
        let node_public_key = NodeKeyConfig::Ed25519(Secret::Input(seckey))
            .into_keypair()
            .unwrap()
            .public();
        let account_address = AccountAddress::from_public_key(&public_key);
        let peer_id = PeerId::from_public_key(node_public_key.clone());

        if let PublicKey::Ed25519(key) = node_public_key.clone() {
            assert_eq!(key.encode(), public_key.to_bytes());
        };
        assert_eq!(node_public_key.clone().into_peer_id(), peer_id.clone());
        assert_eq!(
            convert_account_address_to_peer_id(account_address).unwrap(),
            peer_id
        );
    }

    fn generate_account_address() -> String {
        let (private_key, public_key) = compat::generate_keypair(Option::None);
        let account_address = AccountAddress::from_public_key(&public_key);
        hex::encode(account_address)
    }

    #[test]
    fn test_boot_nodes() {
        let mut boot_nodes = Vec::new();

        boot_nodes.push(
            format!(
                "/ip4/127.0.0.1/tcp/5000/p2p/{:}",
                generate_account_address()
            )
                .to_string(),
        );
        boot_nodes.iter().for_each(|x| println!("{}", x));

        let boot_nodes = convert_boot_nodes(boot_nodes);
        boot_nodes.iter().for_each(|x| println!("{}", x));
    }
}
