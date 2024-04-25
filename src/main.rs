// cargo run port 3000

use std::env;

use sqlite::State;

use automerge::ActorId;
use autosurgeon::{hydrate, reconcile, Hydrate, Reconcile};

#[derive(Debug, Clone, Reconcile, Hydrate, PartialEq)]
struct Path {
    name: String,
    path: String,
    description: String,
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();

    let port = &args[2];

    let peer = &args[4];

    // 创建内存sqlite数据库
    let db = sqlite::open(":memory:").unwrap();

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

    db.execute(query).unwrap();

    loop {
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap();
        let input = input.trim();

        if input == "exit" {
            break;
        }

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
        let query = format!(
            "UPDATE paths SET name = '{}' WHERE id = 1;",
            path.name
        );

        db.execute(&query).unwrap();

        // 查询数据库的 test数据
        let query = "SELECT * FROM paths";
        let mut stmt = db.prepare(query).unwrap();

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
