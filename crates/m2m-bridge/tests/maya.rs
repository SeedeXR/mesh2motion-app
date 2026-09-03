//! Maya bridge tests.
//!
//! The parse test is CI-safe (no Maya). The round-trip is `#[ignore]`d: it needs
//! a local Maya install, so CI (which has none) skips it, and it is run by hand
//! on a machine that has one.

use m2m_bridge::maya;

#[test]
fn a_successful_import_report_parses_with_its_counts() {
    let json = r#"{
        "file": "rig.fbx",
        "imported": true,
        "joints": 65,
        "meshes": 2,
        "skin_clusters": 2,
        "joint_names": ["Hips", "Spine"],
        "joint_parents": ["Hips<-", "Spine<-Hips"],
        "root_joints": ["Hips"],
        "mesh_vertices": [10514, 14232],
        "weighted_vertices": 24746,
        "weight_total": 24746.0
    }"#;
    let report = maya::parse_report(json).expect("parses");
    assert!(report.imported);
    assert_eq!(report.joints, Some(65));
    assert_eq!(report.meshes, Some(2));
    assert_eq!(report.skin_clusters, Some(2));
    assert_eq!(report.mesh_vertices, vec![10514, 14232]);
    assert_eq!(report.weight_total, Some(24746.0));
    assert_eq!(report.root_joints, vec!["Hips"]);
    assert_eq!(report.error, None);
}

#[test]
fn a_failed_import_parses_with_its_error() {
    let json = r#"{"file": "bad.fbx", "imported": false, "error": "RuntimeError: nope"}"#;
    let report = maya::parse_report(json).expect("parses");
    assert!(!report.imported);
    assert_eq!(report.error.as_deref(), Some("RuntimeError: nope"));
    assert_eq!(report.joints, None);
}

/// A real headless round trip through Maya. Ignored by default because CI has no
/// Maya; run it with `--ignored` on a machine that does.
#[test]
#[ignore = "needs a local Maya install (mayapy)"]
fn maya_reads_a_real_rig_end_to_end() {
    let mayapy = match maya::mayapy_path() {
        Ok(path) => path,
        Err(_) => return,
    };
    let fbx = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/test-files/retarget testing/mixamo-original-rig.fbx"
    ))
    .expect("the reference fbx");

    let report = maya::inspect(&fbx, &mayapy).expect("inspects");
    assert!(report.imported, "Maya failed to import: {:?}", report.error);
    assert_eq!(report.joints, Some(65));
    assert_eq!(report.meshes, Some(2));
    assert_eq!(report.skin_clusters, Some(2));
    assert_eq!(report.mesh_vertices, vec![10514, 14232]);
    // Every vertex normalised to one, so the total equals the weighted count —
    // the same invariant the Blender report checks on the same rig.
    assert_eq!(report.weighted_vertices, Some(24746));
    assert_eq!(report.weight_total, Some(24746.0));
    assert_eq!(report.root_joints, vec!["mixamorig:Hips"]);
}
