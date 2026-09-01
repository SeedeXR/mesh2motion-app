//! Manual live-session round trip (P4-1b).
//!
//! Unlike the headless `live.rs` test, this one talks to a Blender the artist
//! already has open: enable the add-on, run *Start Mesh2Motion Bridge Server*,
//! then `cargo test -p m2m-bridge --release --test live_mcp -- --ignored`. It
//! pushes a rig to the running session and prints what came back. Non-blocking
//! on the Blender side (background accept thread + a main-thread timer), and —
//! since the fix in session 071 — non-destructive: the artist's scene is left
//! alone and the report describes only the pushed rig.

#[test]
#[ignore = "manual: needs a live Blender with the add-on server on the default port"]
fn a_rig_pushed_to_a_live_session_reads_back() {
    let rig = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/rigs/rig-human.glb"
    ))
    .expect("the reference rig");
    let addr = format!("127.0.0.1:{}", m2m_bridge::live::DEFAULT_PORT);
    let report = m2m_bridge::live::inspect_live(&addr, "rig-human.glb", &rig)
        .expect("the live round trip succeeds");
    eprintln!(
        "live report: imported={} bones={:?} armatures={:?} error={:?}",
        report.imported, report.bones, report.armatures, report.error
    );
    assert!(
        report.imported,
        "Blender did not import: {:?}",
        report.error
    );
    assert_eq!(report.bones, Some(66));
    assert_eq!(report.armatures, Some(1));
}
