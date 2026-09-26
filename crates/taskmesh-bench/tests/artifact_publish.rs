use std::fs;
use std::sync::{Arc, Barrier};

use taskmesh_bench::artifact::write_new;

#[test]
fn concurrent_publish_has_one_winner_and_never_replaces_bytes() {
    let directory = std::env::temp_dir().join(format!("taskmesh-artifact-{}", std::process::id()));
    fs::create_dir(&directory).expect("fresh test directory");
    let path = directory.join("raw.json");
    let barrier = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (0..8u8)
        .map(|value| {
            let barrier = barrier.clone();
            let path = path.clone();
            std::thread::spawn(move || {
                barrier.wait();
                (value, write_new(&path, &[value; 4096]))
            })
        })
        .collect();
    let outcomes: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    let winners: Vec<_> = outcomes
        .iter()
        .filter(|(_, result)| result.is_ok())
        .collect();
    assert_eq!(winners.len(), 1);
    assert_eq!(fs::read(&path).unwrap(), vec![winners[0].0; 4096]);
    assert!(outcomes
        .iter()
        .filter(|(_, result)| result.is_err())
        .all(|(_, result)| {
            result.as_ref().unwrap_err().kind() == std::io::ErrorKind::AlreadyExists
        }));
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
    assert!(write_new(&path, b"replacement").is_err());
    assert_eq!(fs::read(&path).unwrap(), vec![winners[0].0; 4096]);
    fs::remove_dir_all(directory).unwrap();
}
