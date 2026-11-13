#![cfg(windows)]

#[test]
fn the_binary_requests_common_controls_v6() {
    let exe = std::env::current_exe().unwrap();
    let bytes = std::fs::read(exe).unwrap();
    let needle = b"Microsoft.Windows.Common-Controls";
    assert!(
        bytes.windows(needle.len()).any(|window| window == needle),
        "comctl32 v6 is not in the manifest; the process will fail to load TaskDialogIndirect"
    );
}
