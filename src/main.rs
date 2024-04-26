// cargo run port 3000

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use sqlite::State;

use autosurgeon::{hydrate, reconcile, Hydrate, Reconcile};
use libp2p::{
    futures::{AsyncReadExt, AsyncWriteExt, StreamExt},
    mdns, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, PeerId, StreamProtocol,
};
use libp2p_stream as stream;
use tokio::io::{self, AsyncBufReadExt};

const SYNC_PROTOCOL: StreamProtocol = StreamProtocol::new("/sync");

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq, Serialize, Deserialize)]
struct Path {
    name: String,
    path: String,
    description: String,
}

#[derive(NetworkBehaviour)]
struct MyBehaviour {
    mdns: mdns::tokio::Behaviour,
    stream: stream::Behaviour,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Invite {
    pub data: Vec<u8>,
}

#[tokio::main]
async fn main() {
    // let args: Vec<String> = env::args().collect();

    // let port = &args[2];

    // let peer = &args[4];

    // 创建内存sqlite数据库
    let db = sqlite::open(":memory:").unwrap();

    let db_arc = Arc::new(Mutex::new(db));

    let mut peers: Vec<PeerId> = vec![];

    // create a path table with:name path description
    let query = "
        CREATE TABLE IF NOT EXISTS paths (
            id INTEGER PRIMARY KEY, 
            name TEXT NOT NULL,
            path TEXT NOT NULL, 
            description TEXT NOT NULL
        );
        INSERT INTO paths VALUES (NULL, 'test', 'test', 'test');
    ";
    // 记录 test 记录的 doc
    let mut doc: automerge::AutoCommit = automerge::AutoCommit::new();
    let path = Path {
        name: "test".to_string(),
        path: "test".to_string(),
        description: "test".to_string(),
    };
    reconcile(&mut doc, &path).unwrap();

    let conn = db_arc.lock().unwrap();

    conn.execute(query).unwrap();

    drop(conn);

    // 运行p2p服务
    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .unwrap()
        .with_quic()
        .with_behaviour(|key| {
            let mdns =
                mdns::tokio::Behaviour::new(mdns::Config::default(), key.public().to_peer_id())?;
            let stream = stream::Behaviour::new();
            Ok(MyBehaviour { mdns, stream })
        })
        .unwrap()
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    swarm
        .listen_on("/ip4/0.0.0.0/tcp/0".parse().unwrap())
        .unwrap();

    let mut stdin = io::BufReader::new(io::stdin()).lines();

    let mut incoming_streams = swarm
        .behaviour()
        .stream
        .new_control()
        .accept(SYNC_PROTOCOL)
        .unwrap();

    let db_arc_copy = db_arc.clone();
    tokio::spawn(async move {
        while let Some((peer, mut stream)) = incoming_streams.next().await {
            println!("Incoming stream from: {peer}");
            // 读取stream数据
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).await.unwrap();
            // 解析数据
            let invite: Invite = serde_json::from_slice(&buf).unwrap();
            let data = invite.data;
            // 将data序列化到doc
            let other_doc = automerge::AutoCommit::load(&data).unwrap();
            // 拿到数据
            let path: Path = hydrate(&other_doc).unwrap();
            println!("path: {path:#?}");

            // 插入数据库
            let query = format!(
                "INSERT INTO paths VALUES (NULL, '{}', '{}', '{}');",
                path.name, path.path, path.description
            );

            let conn = db_arc_copy.lock().unwrap();

            conn.execute(&query).unwrap();            

            // 查询数据库
            let query = "SELECT * FROM paths";
            let mut stmt = conn.prepare(query).unwrap();

            while let State::Row = stmt.next().unwrap() {
                let id: i64 = stmt.read(0).unwrap();
                let name: String = stmt.read(1).unwrap();
                let path: String = stmt.read(2).unwrap();
                let description: String = stmt.read(3).unwrap();

                println!(
                    "id: {}, name: {}, path: {}, description: {}",
                    id, name, path, description
                );
            }
        }
    });

    let db_loop = db_arc.clone();
    loop {
        tokio::select! {
            Ok(Some(line)) = stdin.next_line() => {
                let input = line.trim();
                match input {
                    "exit" => break,
                    "sync" => {

                        let peer_id = peers[0];

                        // 发送 doc 数据
                        let sync_request = serde_json::to_vec(&Invite {
                             data: doc.save(),
                        }).expect("Failed to serialize sync request");

                        // open new stream
                        let mut stream = swarm.behaviour().stream.new_control().open_stream(peer_id, SYNC_PROTOCOL).await.expect("Failed to open stream");
                        // println!("Opened stream to: {peer_id}");
                        // write data to stream
                        stream.write_all(&sync_request).await.unwrap();
                        println!("Sent sync request to: {peer_id}")
                    },
                    input => {
                        // 拿到test数据
                        let path: Path = hydrate(&doc).unwrap();
                        // input 修改数据的 name
                        let path = Path {
                            name: input.to_string(),
                            path: path.path,
                            description: path.description,
                        };
                        // 更新doc
                        reconcile(&mut doc, &path).unwrap();

                        // 更新数据库
                        let query = format!("UPDATE paths SET name = '{}' WHERE id = 1;", path.name);

                        let conn = db_loop.lock().unwrap();

                        conn.execute(&query).unwrap();

                        // 查询数据库的 test数据
                        let query = "SELECT * FROM paths";
                        let mut stmt = conn.prepare(query).unwrap();

                        while let State::Row = stmt.next().unwrap() {
                            let id: i64 = stmt.read(0).unwrap();
                            let name: String = stmt.read(1).unwrap();
                            let path: String = stmt.read(2).unwrap();
                            let description: String = stmt.read(3).unwrap();

                            println!(
                                "id: {}, name: {}, path: {}, description: {}",
                                id, name, path, description
                            );
                        }
                    }
                }
            }
            event = swarm.select_next_some() => match event{
                SwarmEvent::Behaviour(MyBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                    for (peer_id, _multiaddr) in list {
                        println!("mDNS discovered a new peer: {peer_id}");
                        // 连接peer
                        let _ = swarm.dial(peer_id);
                        println!("Dialed peer: {peer_id}");

                        // 添加peer到peers
                        peers.push(peer_id);
                    }
                },
                SwarmEvent::Behaviour(MyBehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                    for (peer_id, _multiaddr) in list {
                        println!("mDNS discover peer has expired: {peer_id}");
                    }
                },
                _ => {}
            }
        }
    }
}
