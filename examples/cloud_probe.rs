//! Exercises a cloud account end to end, without the UI: make a folder,
//! upload, list, download (and compare), rename, delete.
//!
//! ```sh
//! PROBE_ACCOUNT='{"id":"probe","name":"NAS","kind":"WebDav","address":"http://127.0.0.1:8080"}' \
//! PROBE_SECRET=pw XDG_DATA_HOME=/tmp/probe-data \
//! cargo run --example cloud_probe -- some.pdf
//! ```
//!
//! `PROBE_UPLOAD_DIR=<local folder>` uploads a whole folder and lists it.
//! `PROBE_LIST=<folder>` only lists that folder ("" for the top) and changes
//! nothing. Set `XDG_DATA_HOME` to a scratch folder: downloads land under it.
//! Everything happens inside a `paperview-probe` folder on the account.

use std::sync::mpsc;
use std::time::Duration;

use paperview::cloud::{self, Account, CloudEvent, CloudOp};

fn run(acct: &Account, op: CloudOp) -> Result<CloudEvent, String> {
    let (tx, rx) = mpsc::channel();
    cloud::spawn(acct.clone(), op, tx, eframe::egui::Context::default());
    loop {
        match rx.recv_timeout(Duration::from_secs(120)) {
            Ok(CloudEvent::Error(e)) => return Err(e),
            Ok(CloudEvent::Progress { done, total, bps }) => {
                eprintln!("    … {done}/{total} at {}/s", cloud::fmt_size(bps as u64));
            }
            Ok(CloudEvent::Status(s)) => eprintln!("    {s}"),
            Ok(e) if e.is_terminal() => return Ok(e),
            Ok(_) => {}
            Err(e) => return Err(format!("no answer: {e}")),
        }
    }
}

fn names(e: &CloudEvent) -> Vec<String> {
    match e {
        CloudEvent::Listed { files, .. } => files
            .iter()
            .map(|f| {
                if f.is_dir {
                    format!("{}/", f.path)
                } else {
                    format!("{} ({} B)", f.path, f.size)
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn main() {
    let acct: Account =
        serde_json::from_str(&std::env::var("PROBE_ACCOUNT").expect("PROBE_ACCOUNT"))
            .expect("PROBE_ACCOUNT is an account as JSON");
    if let Ok(secret) = std::env::var("PROBE_SECRET") {
        cloud::secrets::remember(&acct.id, &secret);
    }
    // PROBE_UPLOAD_DIR=<local folder>: upload it into the top folder, then
    // list what arrived.
    if let Ok(dir) = std::env::var("PROBE_UPLOAD_DIR") {
        let dir = std::path::PathBuf::from(dir);
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        match run(&acct, CloudOp::UploadFolder(dir, name.clone())) {
            Ok(CloudEvent::FolderUploaded {
                files,
                failed,
                secs,
                ..
            }) => {
                println!("uploaded {files} in {secs:.1}s, failed {failed:?}");
            }
            Ok(_) => println!("unexpected reply"),
            Err(e) => {
                eprintln!("FAILED: {e}");
                std::process::exit(1);
            }
        }
        let mut todo = vec![name];
        while let Some(d) = todo.pop() {
            if let Ok(CloudEvent::Listed { files, .. }) = run(&acct, CloudOp::List(d)) {
                for f in files {
                    if f.is_dir {
                        todo.push(f.path);
                    } else {
                        println!("  {} ({} B)", f.path, f.size);
                    }
                }
            }
        }
        return;
    }
    // PROBE_LIST=dir: only list that folder (read-only).
    if let Ok(dir) = std::env::var("PROBE_LIST") {
        match run(&acct, CloudOp::List(dir)) {
            Ok(e) => {
                for n in names(&e) {
                    println!("{n}");
                }
                return;
            }
            Err(e) => {
                eprintln!("FAILED: {e}");
                std::process::exit(1);
            }
        }
    }
    let file = std::path::PathBuf::from(std::env::args().nth(1).expect("a document to upload"));
    let original = std::fs::read(&file).expect("readable document");
    let dir = "paperview-probe";
    let first = format!("{dir}/Probe ü & co.pdf");
    let second = format!("{dir}/Renamed probe.pdf");

    let step = |what: &str, op: CloudOp| -> CloudEvent {
        eprintln!("• {what}");
        match run(&acct, op) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("  FAILED: {e}");
                std::process::exit(1);
            }
        }
    };

    let top = step("list the top folder", CloudOp::List(String::new()));
    eprintln!("  {:?}", names(&top));
    if !names(&top).iter().any(|n| n == &format!("{dir}/")) {
        step("make a folder", CloudOp::MakeFolder(dir.into()));
    }
    step("upload", CloudOp::Upload(file.clone(), first.clone()));
    let listed = step("list the folder", CloudOp::List(dir.into()));
    eprintln!("  {:?}", names(&listed));
    assert!(names(&listed).contains(&format!("{first} ({} B)", original.len())));

    match step(
        "download",
        CloudOp::Download {
            path: first.clone(),
            size: original.len() as u64,
            open: false,
        },
    ) {
        CloudEvent::Downloaded {
            path, bytes, secs, ..
        } => {
            let got = std::fs::read(&path).expect("downloaded file");
            assert_eq!(got, original, "downloaded bytes differ");
            eprintln!(
                "  {} in {secs:.2}s → {}, identical",
                cloud::fmt_size(bytes),
                path.display()
            );
        }
        other => panic!("unexpected {:?}", names(&other)),
    }

    let id = match &listed {
        CloudEvent::Listed { files, .. } => files
            .iter()
            .find(|f| f.path == first)
            .map(|f| f.id.clone())
            .unwrap_or_default(),
        _ => String::new(),
    };
    step(
        "rename",
        CloudOp::Rename {
            from: first.clone(),
            id,
            to: second.clone(),
        },
    );
    let after = step("list again", CloudOp::List(dir.into()));
    eprintln!("  {:?}", names(&after));
    assert!(names(&after).iter().any(|n| n.starts_with(&second)));
    assert!(!names(&after).iter().any(|n| n.starts_with(&first)));

    step(
        "delete",
        CloudOp::Delete(vec![(second.clone(), String::new())]),
    );
    let last = step("list once more", CloudOp::List(dir.into()));
    eprintln!("  {:?}", names(&last));
    assert!(names(&last).is_empty(), "folder should be empty");
    eprintln!("✓ all steps passed for {}", acct.summary());
}
