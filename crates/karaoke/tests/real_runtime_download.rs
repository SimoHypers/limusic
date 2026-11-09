//! Run this test with:
//! cargo test -p karaoke --test real_runtime_download -- --ignored --nocapture

mod common;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "downloads the real runtime (about 22 MB on Windows, 8 to 31 MB on Linux/macOS); run it by hand"]
async fn the_real_runtime_downloads_verifies_and_loads() {
    if karaoke::runtime_pkg::current().is_none() {
        eprintln!("no runtime package for this platform");
        return;
    }
    let (dest, _rt) = common::install_runtime().await;
    println!("OK");
    let _ = std::fs::remove_dir_all(dest.parent().unwrap());
}
