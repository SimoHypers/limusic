//! Shared helper functions for karaoke integration tests.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub const MODEL_HOSTS: &[&str] =
    &["github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com"];

pub async fn install_runtime() -> (PathBuf, Option<karaoke::runtime::RuntimeReady>) {
    let Some(pkg) = karaoke::runtime_pkg::current() else {
        panic!("no runtime package for this platform");
    };

    let scratch = std::env::temp_dir().join(format!(
        "karaoke_real_runtime_{}",
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let dest = scratch.join(karaoke::runtime_pkg::dir_name());
    std::fs::create_dir_all(&scratch).unwrap();

    let client = reqwest::Client::new();

    for part in pkg.parts {
        println!("processing part: {}", part.name);
        let start_time = std::time::Instant::now();
        match part.kind {
            karaoke::runtime_pkg::ArchiveKind::Tgz | karaoke::runtime_pkg::ArchiveKind::Zip => {
                let part_file = scratch.join(format!("{}.part", part.name));
                let archive_file = scratch.join(format!("{}.archive", part.name));
                let cancel = std::sync::Arc::new(karaoke::download::Cancel::new());
                let src = karaoke::download::Source {
                    url: part.url,
                    size: part.archive_size,
                    sha256: part.archive_sha256,
                    hosts: part.hosts,
                    https_only: true,
                };
                let mut last_percent = 0;
                karaoke::download::fetch(
                    &client,
                    &src,
                    &part_file,
                    &archive_file,
                    karaoke::download::Timeouts::default(),
                    &cancel,
                    |downloaded| {
                        if let Some(pct) = (downloaded * 100).checked_div(src.size) {
                            if pct >= last_percent + 25 || pct == 100 {
                                println!("  downloading {}: {}%", part.name, pct);
                                last_percent = pct;
                            }
                        }
                    },
                )
                .await
                .unwrap();

                let part_static = part;
                let archive_file_clone = archive_file.clone();
                let dest_clone = dest.clone();
                tokio::task::spawn_blocking(move || {
                    karaoke::runtime_pkg::extract_part(
                        part_static,
                        &archive_file_clone,
                        &dest_clone,
                    )
                    .unwrap();
                })
                .await
                .unwrap();
            }
            karaoke::runtime_pkg::ArchiveKind::ZipRange => {
                let cancel = std::sync::Arc::new(karaoke::download::Cancel::new());
                let (total_len, tail) = karaoke::download::fetch_tail(
                    &client,
                    part.url,
                    part.hosts,
                    true,
                    karaoke::runtime_pkg::REMOTE_ZIP_TAIL,
                    karaoke::download::Timeouts::default(),
                    &cancel,
                    &mut |_| {},
                )
                .await
                .unwrap();
                assert_eq!(total_len, part.archive_size);

                let handle = tokio::runtime::Handle::current();
                let client_clone = client.clone();
                let part_url = part.url;
                let part_hosts = part.hosts;
                let cancel_clone = cancel.clone();
                let tail_len = tail.len() as u64;
                let fetched_bytes = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                let fetched_bytes_clone = fetched_bytes.clone();

                let part_static = part;
                let dest_clone = dest.clone();
                let tail_clone = tail.clone();

                tokio::task::spawn_blocking(move || {
                    let mut fetch_closure =
                        |start: u64,
                         end_inclusive: u64|
                         -> Result<Vec<u8>, karaoke::runtime::Unavailable> {
                            let client_c = client_clone.clone();
                            let cancel_c = cancel_clone.clone();
                            let res = handle.block_on(karaoke::download::fetch_range(
                                &client_c,
                                part_url,
                                part_hosts,
                                true,
                                start,
                                end_inclusive,
                                karaoke::download::Timeouts::default(),
                                &cancel_c,
                                &mut |_| {},
                            ));
                            match res {
                                Ok(bytes) => {
                                    fetched_bytes_clone.fetch_add(
                                        bytes.len() as u64,
                                        std::sync::atomic::Ordering::SeqCst,
                                    );
                                    Ok(bytes)
                                }
                                Err(e) => {
                                    Err(karaoke::runtime::Unavailable::LoadFailed(e.to_string()))
                                }
                            }
                        };
                    karaoke::runtime_pkg::extract_remote_zip(
                        part_static,
                        total_len,
                        &tail_clone,
                        &dest_clone,
                        &mut fetch_closure,
                    )
                    .unwrap();
                })
                .await
                .unwrap();

                let fetched = fetched_bytes.load(std::sync::atomic::Ordering::SeqCst);
                println!(
                    "  ZipRange part {} fetched via range: {} bytes (tail: {} bytes)",
                    part.name, fetched, tail_len
                );
                assert!(fetched + tail_len <= part.fetch_size * 2);
            }
        }
        println!("part {} finished in {:?}", part.name, start_time.elapsed());
    }

    let lib_path = karaoke::runtime_pkg::verify_installed(pkg, &dest).unwrap();
    println!("Verified installed library at: {:?}", lib_path);
    for f in karaoke::runtime_pkg::pinned_files(pkg) {
        let file_path = dest.join(f.dest);
        let meta = std::fs::metadata(&file_path).unwrap();
        println!("  pinned file {}: {} bytes", f.dest, meta.len());
    }

    let rt = {
        #[cfg(windows)]
        {
            match karaoke::runtime::load(&dest, None) {
                Ok(rt) => {
                    println!("runtime loaded successfully, directml: {}", rt.directml());
                    Some(rt)
                }
                Err(karaoke::runtime::Unavailable::VcRuntime(detail)) => {
                    println!("refused as expected on this machine: vc runtime error: {detail}");
                    None
                }
                Err(e) => {
                    panic!("unexpected runtime load error: {}", e);
                }
            }
        }
        #[cfg(not(windows))]
        {
            let rt = karaoke::runtime::init_from(&lib_path, false)
                .expect("failed to init runtime from downloaded library");
            println!("runtime initialized successfully from {:?}", lib_path);
            Some(rt)
        }
    };

    (dest, rt)
}

pub async fn download_model(id: karaoke::models::ModelId, dir: &Path) -> PathBuf {
    let spec = id.spec();
    std::fs::create_dir_all(dir).unwrap();
    let final_path = dir.join(spec.file_name);
    if final_path.exists() {
        if let Ok(meta) = std::fs::metadata(&final_path) {
            if meta.len() == spec.size {
                return final_path;
            }
        }
    }
    let part_path = final_path.with_extension(
        final_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!("{e}.part"))
            .unwrap_or_else(|| "part".to_string()),
    );
    let client = reqwest::Client::new();
    let cancel = std::sync::Arc::new(karaoke::download::Cancel::new());
    let source = karaoke::download::Source {
        url: spec.url,
        size: spec.size,
        sha256: spec.sha256,
        hosts: MODEL_HOSTS,
        https_only: true,
    };
    let mut last_percent = 0;
    karaoke::download::fetch(
        &client,
        &source,
        &part_path,
        &final_path,
        karaoke::download::Timeouts::default(),
        &cancel,
        |downloaded| {
            if let Some(pct) = (downloaded * 100).checked_div(spec.size) {
                if pct >= last_percent + 25 || pct == 100 {
                    println!("  downloading model {}: {}%", spec.key, pct);
                    last_percent = pct;
                }
            }
        },
    )
    .await
    .unwrap();
    final_path
}
