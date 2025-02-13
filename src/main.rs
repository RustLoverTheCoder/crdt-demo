// cargo run port 3000

use std::{
    collections::HashMap,
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

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub enum PeerPermission {
    #[default]
    ReadOnly, // peer只读，只能其他人的变更
    ReadWrite, // peer可读写，接收和广播变更
    Owner,     // 可读写，接收和广播变更
}

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq, Serialize, Deserialize)]
struct Path {
    pub_id: uuid::Uuid,
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
    pub id: uuid::Uuid,
    pub data: Vec<u8>,
    pub permission: PeerPermission,
}

#[tokio::main]
async fn main() {
    let state: Arc<Mutex<HashMap<uuid::Uuid, automerge::AutoCommit>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // let args: Vec<String> = env::args().collect();

    // let port = &args[2];

    // let peer = &args[4];

    // 创建内存sqlite数据库
    let db = sqlite::open(":memory:").unwrap();

    let db_arc = Arc::new(Mutex::new(db));

    let mut peers: Vec<PeerId> = vec![];

    // doc 对应的 id
    let id: uuid::Uuid = uuid::Uuid::new_v4();
    println!("doc id: {id}");

    // create a path table with:name path description
    let query = format!(
        "
    CREATE TABLE IF NOT EXISTS paths (
        id INTEGER PRIMARY KEY, 
        pub_id TEXT NOT NULL,
        name TEXT NOT NULL,
        path TEXT NOT NULL, 
        description TEXT NOT NULL
    );
    INSERT INTO paths VALUES (NULL, '{}', 'test', 'test', 'test');
",
        id.to_string()
    );

    // 记录 test 记录的 doc
    let mut doc: automerge::AutoCommit = automerge::AutoCommit::new();

    let path = Path {
        pub_id: id,
        name: "test".to_string(),
        path: "test".to_string(),
        description: "test".to_string(),
    };
    reconcile(&mut doc, &path).unwrap();

    {
        // 存入state
        state.lock().unwrap().insert(id, doc.clone());
    }

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
    let state_copy = state.clone();

    tokio::spawn(async move {
        while let Some((_peer, mut stream)) = incoming_streams.next().await {
            // 读取stream数据
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).await.unwrap();
            // 解析数据
            let invite: Invite = serde_json::from_slice(&buf).unwrap();
            let data = invite.data;
            let id = invite.id;

            // state中是否存在id
            if state_copy.lock().unwrap().contains_key(&id) {
                let state_copy2 = state_copy.clone();
                // 存在则更新
                // 从state中拿到doc
                let state_lock = state_copy2.lock().unwrap();

                let mut doc = state_lock.get(&id).unwrap().clone();
                // 更新doc
                let other_doc = automerge::AutoCommit::load(&data).unwrap();
                let mut doc_fork = doc.fork();
                // other_doc 读取数据
                let other_doc_path: Path = hydrate(&other_doc).unwrap();
                // 更新 doc_fork
                reconcile(&mut doc_fork, &other_doc_path).unwrap();
                doc.merge(&mut doc_fork).expect("Failed to merge doc");

                // 更新数据库
                update_db(db_arc_copy.clone(), doc.clone());
            } else {
                let other_doc = automerge::AutoCommit::load(&data).unwrap();
                // 插入数据库
                insert_db(db_arc_copy.clone(), other_doc.clone());
                // 不存在则插入
                state_copy.lock().unwrap().insert(id, other_doc);
            }
        }
    });

    let db_loop = db_arc.clone();
    let state_loop = state.clone();
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
                            id,
                            data: doc.save(),
                            permission: PeerPermission::ReadWrite,
                        }).expect("Failed to serialize sync request");

                        // open new stream
                        let mut stream = swarm.behaviour().stream.new_control().open_stream(peer_id, SYNC_PROTOCOL).await.expect("Failed to open stream");
                        // println!("Opened stream to: {peer_id}");
                        // write data to stream
                        stream.write_all(&sync_request).await.unwrap();
                        println!("Sent sync request to: {peer_id}")
                    },
                    "update" => {
                        // 更新数据
                        // 请再输入新的name
                        println!("Please enter doc id: ");
                        match stdin.next_line().await {
                            Ok(Some(line)) => {
                                let input = line.trim();
                                println!("Updating doc id: {input}");
                                let id = uuid::Uuid::parse_str(input).unwrap();

                                // 拿到doc

                                match state_loop.lock().unwrap().clone().get_mut(&id){
                                    Some(doc) => {
                                        // 从doc拿到数据
                                        let path: Path = hydrate(doc).unwrap();
                                        // 更新数据
                                        let path = Path {
                                            pub_id: path.pub_id,
                                            name: "update".to_string(),
                                            path: path.path,
                                            description: path.description,
                                        };
                                        // 更新doc
                                        reconcile(doc, &path).unwrap();
                                        // 更新数据库
                                        update_db(db_arc.clone(), doc.clone());

                                        // 发送同步数据
                                        let peer_id = peers[0];
                                        // 发送 doc 数据
                                        let sync_request = serde_json::to_vec(&Invite {
                                            id,
                                            data: doc.save(),
                                            permission: PeerPermission::ReadWrite,
                                        }).expect("Failed to serialize sync request");

                                        // open new stream
                                        let mut stream = swarm.behaviour().stream.new_control().open_stream(peer_id, SYNC_PROTOCOL).await.expect("Failed to open stream");
                                        // println!("Opened stream to: {peer_id}");
                                        // write data to stream
                                        stream.write_all(&sync_request).await.unwrap();
                                        // println!("Sent sync request to: {peer_id}")
                                    },
                                    None => {
                                        println!("Doc not found");
                                    }
                                }
                            },
                            _ => {}
                        }
                    },
                    input => {
                        // 拿到test数据
                        let path: Path = hydrate(&doc).unwrap();
                        // input 修改数据的 name
                        let path = Path {
                            pub_id: path.pub_id,
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
                            let pub_id: String = stmt.read(1).unwrap();
                            let name: String = stmt.read(2).unwrap();
                            let path: String = stmt.read(3).unwrap();
                            let description: String = stmt.read(4).unwrap();

                            println!(
                                "id: {},pub_id:{}, name: {}, path: {}, description: {}",
                                id,pub_id, name, path, description
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

pub fn insert_db(conn: Arc<Mutex<sqlite::Connection>>, doc: automerge::AutoCommit) {
    // 将data序列化到doc
    // 拿到数据
    let path: Path = hydrate(&doc).unwrap();
    // 插入数据库
    let query = format!(
        "INSERT INTO paths VALUES (NULL,'{}', '{}', '{}', '{}');",
        path.pub_id, path.name, path.path, path.description
    );

    let conn = conn.lock().unwrap();

    conn.execute(&query).unwrap();

    // 查询数据库
    let query = "SELECT * FROM paths";
    let mut stmt = conn.prepare(query).unwrap();

    while let State::Row = stmt.next().unwrap() {
        let id: i64 = stmt.read(0).unwrap();
        let pub_id: String = stmt.read(1).unwrap();
        let name: String = stmt.read(2).unwrap();
        let path: String = stmt.read(3).unwrap();
        let description: String = stmt.read(4).unwrap();

        println!(
            "id: {},pub_id: {},  name: {}, path: {}, description: {}",
            id, pub_id, name, path, description
        );
    }
}

pub fn update_db(conn: Arc<Mutex<sqlite::Connection>>, doc: automerge::AutoCommit) {
    // 将data序列化到doc
    // 拿到数据
    let path: Path = hydrate(&doc).unwrap();
    // path pub_id
    let pub_id = path.pub_id;

    // 更新数据库 pub_id
    // let query = format!("UPDATE paths SET name = '{}' WHERE p = 2;", path.name);
    let query = format!(
        "UPDATE paths SET name = '{}', path = '{}', description = '{}' WHERE pub_id = '{}';",
        path.name, path.path, path.description, pub_id
    );

    let conn = conn.lock().unwrap();

    conn.execute(&query).unwrap();

    // 查询数据库
    let query = "SELECT * FROM paths";
    let mut stmt = conn.prepare(query).unwrap();

    while let State::Row = stmt.next().unwrap() {
        let id: i64 = stmt.read(0).unwrap();
        let pub_id: String = stmt.read(1).unwrap();
        let name: String = stmt.read(2).unwrap();
        let path: String = stmt.read(3).unwrap();
        let description: String = stmt.read(4).unwrap();

        println!(
            "id: {}, pub: {} name: {}, path: {}, description: {}",
            id, pub_id, name, path, description
        );
    }
}
