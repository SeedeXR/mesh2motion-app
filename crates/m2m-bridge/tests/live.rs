//! Live-mode round trip against a real Blender (P4-1b).
//!
//! `#[ignore]`d: it needs a Blender install, so CI (which has none) skips it,
//! exactly like the headless live test in `report.rs`. Run it by hand with
//! `cargo test -p m2m-bridge --release -- --ignored`.
//!
//! It launches the companion add-on in its headless single-shot mode
//! (`blender -b --python mesh2motion_bridge.py -- <port>`), waits for the
//! server to come up, pushes a rig through `inspect_live`, and checks Blender
//! made the same sense of it that the headless path does.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

const PORT: u16 = 47830;

fn rig_bytes() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/rigs/rig-human.glb"
    ))
    .expect("the reference rig")
}

#[test]
#[ignore = "needs a local Blender install with the add-on"]
fn a_rig_pushed_to_a_live_blender_reads_back() {
    let blender = match m2m_bridge::blender_path() {
        Ok(path) => path,
        Err(_) => return, // no Blender here; the ignore covers CI
    };
    let addon = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../blender-addon/mesh2motion_bridge.py"
    );

    // The add-on's headless single-shot mode: serve one request, then exit.
    let mut child = Command::new(&blender)
        .args([
            "-b",
            "--factory-startup",
            "--python",
            addon,
            "--",
            &PORT.to_string(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawns Blender");

    // Wait for the server's own "listening" line rather than probing the port —
    // the server accepts exactly one connection, so a probe would BE the request
    // and leave nothing for the real one.
    let stdout = child.stdout.take().expect("piped stdout");
    let mut lines = BufReader::new(stdout).lines();
    let ready = lines
        .by_ref()
        .take(400)
        .map_while(Result::ok)
        .any(|line| line.contains("M2M_BRIDGE listening"));
    assert!(ready, "the bridge server never announced itself");

    let report =
        m2m_bridge::live::inspect_live(&format!("127.0.0.1:{PORT}"), "rig-human.glb", &rig_bytes())
            .expect("the live round trip succeeds");

    let _ = child.wait();

    assert!(
        report.imported,
        "Blender did not import the rig: {:?}",
        report.error
    );
    assert_eq!(report.bones, Some(66), "human rig has 66 bones");
    assert_eq!(report.meshes, Some(0), "a rig file carries no mesh");
    assert_eq!(report.armatures, Some(1));
}
