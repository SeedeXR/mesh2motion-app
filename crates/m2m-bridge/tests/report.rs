//! The report parsing, which CI can cover, and a live Blender check that only
//! runs where Blender is installed.

use m2m_bridge::{parse_report, BlenderReport, BridgeError};

#[test]
fn a_successful_import_report_parses_with_its_counts() {
    let json = r#"{
        "file": "rigged.glb",
        "imported": true,
        "bones": 66,
        "meshes": 1,
        "armatures": 1,
        "actions": ["Chest_Open"],
        "mesh_vertices": [10514, 14232],
        "weighted_vertices": 24746,
        "weight_total": 24746.0,
        "action_detail": ["Chest_Open:curves=462,keys=9468,range=0.00-33.00"]
    }"#;

    let report = parse_report(json).expect("parses");
    assert!(report.imported);
    assert_eq!(report.bones, Some(66));
    assert_eq!(report.meshes, Some(1));
    assert_eq!(report.mesh_vertices, vec![10514, 14232]);
    assert_eq!(report.weight_total, Some(24746.0));
    assert_eq!(report.actions, vec!["Chest_Open"]);
    assert!(report.action_detail[0].contains("range=0.00-33.00"));
    assert_eq!(report.error, None);
}

#[test]
fn a_failed_import_report_parses_without_the_optional_counts() {
    let json = r#"{"file": "broken.fbx", "imported": false, "error": "RuntimeError: bad file"}"#;

    let report: BlenderReport = parse_report(json).expect("parses");
    assert!(!report.imported);
    assert_eq!(report.error.as_deref(), Some("RuntimeError: bad file"));
    assert_eq!(report.bones, None);
    assert!(report.mesh_vertices.is_empty());
}

#[test]
fn garbage_is_a_bad_report_error_not_a_panic() {
    assert!(matches!(
        parse_report("Blender 4.x progress...\n{not json"),
        Err(BridgeError::BadReport(_))
    ));
}

/// A real headless round trip. Ignored by default because CI has no Blender.
#[test]
#[ignore = "needs a local Blender install"]
fn blender_reads_a_real_rig_end_to_end() {
    let blender = match m2m_bridge::blender_path() {
        Ok(path) => path,
        Err(_) => return,
    };
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../legacy/static/rigs/rig-human.glb"
    ))
    .expect("the reference rig");

    let report = m2m_bridge::inspect(&bytes, "glb", &blender).expect("inspects");
    assert!(
        report.imported,
        "Blender failed to import: {:?}",
        report.error
    );
    assert_eq!(report.bones, Some(66));
    assert_eq!(report.meshes, Some(0));
    assert_eq!(report.armatures, Some(1));
}
