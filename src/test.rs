use std::time::Duration;

use automerge::ActorId;
use tokio::time::sleep;
use uhlc::{Timestamp, NTP64};
use uuid::Uuid;

// 多生产，多消费 广播
use tokio::sync::broadcast;

// 控制并发数， crdt太多，cpu会爆炸
use tokio::sync::Semaphore;

use autosurgeon::{hydrate, reconcile, Hydrate, Reconcile};

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
struct AssetObject {
    hash: String,
    size: u64,
    mime_type: String,
    file_path: FilePath,
    media_data: MediaData,
}

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
struct FilePath {
    name: String,
    description: String,
}

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
struct MediaData {
    width: u32,
    height: u32,
    duration: u32,
    bitrate: u32,
    has_audio: bool,
}

#[tokio::test]
async fn test_asset_object_merge() {
    let uuid: Uuid = Uuid::new_v4();
    println!("uuid: {}", uuid.to_string());

    // 如果是共享的是文件 就不包含路径
    let mut object1 = AssetObject {
        hash: "hash".to_string(),
        size: 1024,
        mime_type: "image/png".to_string(),
        file_path: FilePath {
            name: "test".to_string(),
            description: "test".to_string(),
        },
        media_data: MediaData {
            width: 1920,
            height: 1080,
            duration: 60,
            bitrate: 1024,
            has_audio: true,
        },
    };

    // 创建一个新的文档 文档是一个自动提交的文档 文档是automerge的一个最小单元
    let mut doc: automerge::AutoCommit = automerge::AutoCommit::new();

    reconcile(&mut doc, &object1).unwrap();

    // 从文档中恢复对象
    let object2: AssetObject = hydrate(&doc).unwrap();

    assert_eq!(object1, object2);

    // 其他人fork了这个文档，然后修改了这个文档
    let actor_id: ActorId = automerge::ActorId::random();
    let mut doc3: automerge::AutoCommit = doc.fork().with_actor(actor_id);
    let mut object3: AssetObject = hydrate(&doc3).unwrap();
    object3.file_path.name = "test3".to_string();
    reconcile(&mut doc3, &object3).unwrap();

    // 第一个object 再修改了一次
    sleep(Duration::from_secs(1)).await;
    object1.file_path.name = "test2".to_string();
    reconcile(&mut doc, &object1).unwrap();

    // 合并文档
    doc.merge(&mut doc3).unwrap();

    // 从文档中恢复对象
    let object4: AssetObject = hydrate(&doc).unwrap();
    // println!("file_path4: {:#?}", object4);
    println!("doc: {:?}", doc);
}

// 分享文件夹
#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
struct Folder {
    pub_id: Uuid,
    name: String,
    description: String,
    materialized_path: String,
    files: Vec<AssetObject>,
    folders: Vec<Folder>,
}

#[tokio::test]
async fn test_folder_merge() {
    // 如果分享的是一个文件夹，就包含路径
    let mut folder1 = Folder {
        pub_id: Uuid::new_v4(),             // 分布式全局唯一
        materialized_path: "/".to_string(), // 自己的路径不需要，只需要子文件夹的路径
        name: "test1".to_string(),
        description: "test1".to_string(),
        files: vec![AssetObject {
            hash: "hash".to_string(),
            size: 1024,
            mime_type: "image/png".to_string(),
            file_path: FilePath {
                name: "test".to_string(),
                description: "test".to_string(),
            },
            media_data: MediaData {
                width: 1920,
                height: 1080,
                duration: 60,
                bitrate: 1024,
                has_audio: true,
            },
        }],
        folders: vec![Folder {
            pub_id: Uuid::new_v4(),
            materialized_path: "/test1".to_string(),
            name: "test2".to_string(),
            description: "test2".to_string(),
            files: vec![],
            folders: vec![],
        }],
    };

    // 创建一个新的文档 文档是一个自动提交的文档 文档是automerge的一个最小单元
    let mut doc: automerge::AutoCommit = automerge::AutoCommit::new();

    reconcile(&mut doc, &folder1).unwrap();

    // fork 一个文档
    let actor_id: ActorId = automerge::ActorId::random();
    let mut doc2: automerge::AutoCommit = doc.fork().with_actor(actor_id);
    let mut folder2: Folder = hydrate(&doc2).unwrap();

    folder2.files.push(AssetObject {
        hash: "hash2".to_string(),
        size: 1024,
        mime_type: "image/png".to_string(),
        file_path: FilePath {
            name: "test".to_string(),
            description: "test".to_string(),
        },
        media_data: MediaData {
            width: 1920,
            height: 1080,
            duration: 60,
            bitrate: 1024,
            has_audio: true,
        },
    });

    reconcile(&mut doc2, &folder2).unwrap();

    // 第一个object 再修改了一次
    sleep(Duration::from_secs(1)).await;
    folder1.files.push(AssetObject {
        hash: "hash3".to_string(),
        size: 1024,
        mime_type: "image/png".to_string(),
        file_path: FilePath {
            name: "test".to_string(),
            description: "test".to_string(),
        },
        media_data: MediaData {
            width: 1920,
            height: 1080,
            duration: 60,
            bitrate: 1024,
            has_audio: true,
        },
    });
    reconcile(&mut doc, &folder1).unwrap();

    // 合并文档
    doc.merge(&mut doc2).unwrap();

    // 从文档中恢复对象
    let folder3: Folder = hydrate(&doc).unwrap();
    println!("folder3: {:#?}", folder3);
}
