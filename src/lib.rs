use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, RwLock},
};

use automerge::ActorId;

use libp2p::PeerId;

// crdt 操作 只要 创建，更新，删除
pub enum CrdtOperation {
    Create,
    Update(uuid::Uuid, Update),
    Delete,
}

// 更新操作
pub enum Update {
    // 文件名字更新
    Name(String),
    // 文件描述更新
    Description(String),
    // 文件路径更新 需要共享的是文件夹，如果共享的是文件就不需要路径 注意：路径是相对路径，如果移动到了共享文件夹外，算是删除
    Path(String),
}

// peer权限
#[derive(Debug, Default, Clone, PartialEq)]
pub enum PeerPermission {
    #[default]
    ReadOnly, // peer只读，只能其他人的变更
    ReadWrite, // peer可读写，接收和广播变更
    Owner,     // 可读写，接收和广播变更
}

#[derive(Clone, Debug)]
pub enum SyncMessage {
    Ingested,
    Created,
}

pub struct Manager {
    // 已经共享的doc
    pub shared: BTreeMap<ActorId, DocInfo>,

    // 发送者
    pub sender: tokio::sync::broadcast::Sender<automerge::sync::Message>,

    // 时间锁， 防止并发，crdt太多，cpu会爆炸
    pub timestamp_lock: tokio::sync::Semaphore,

    // 失败的共享消息
    pub failed_messages: Vec<automerge::sync::Message>,
}

pub struct DocInfo {
    // doc id
    pub doc_id: ActorId,
    // doc的权限
    pub permission: PeerPermission,
    // doc的crdt
    pub crdt: automerge::AutoCommit,
    // 分享状态
    pub shared: automerge::sync::State,
}
