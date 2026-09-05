//! Maya bridge tests.
//!
//! The parse test is CI-safe (no Maya). The round-trip is `#[ignore]`d: it needs
//! a local Maya install, so CI (which has none) skips it, and it is run by hand
//! on a machine that has one.

use m2m_bridge::maya;
use m2m_io::fbx::{build, encode};

/// A unit cube (8 corners, 12 triangles) skinned to a two-bone skeleton, encoded
/// through OUR fbx writer. Minimal, but it exercises exactly what Maya is strict
/// about: the footer CRC and every Model's DefaultAttributeIndex — without which
/// Maya imports zero joints and no mesh.
fn our_rigged_cube_fbx() -> Vec<u8> {
    const POSITIONS: [f32; 24] = [
        0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0,
    ];
    const TRIANGLES: [u32; 36] = [
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, //
        2, 3, 7, 2, 7, 6, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5,
    ];
    const IDENTITY: [f64; 16] = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let bones = [
        build::Bone {
            name: "Root",
            parent: None,
            translation: [0.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            pre_rotation: [0.0, 0.0, 0.0],
        },
        build::Bone {
            name: "Tip",
            parent: Some(0),
            translation: [0.0, 1.0, 0.0],
            rotation: [0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            pre_rotation: [0.0, 0.0, 0.0],
        },
    ];
    let indices: Vec<u32> = (0..8).collect();
    let weights = vec![1.0_f64; 8];
    let clusters = [build::Cluster {
        bone: 0,
        indices: &indices,
        weights: &weights,
        transform: IDENTITY,
        transform_link: IDENTITY,
    }];
    let skins = [build::Skin {
        mesh: 0,
        clusters: &clusters,
    }];
    let mesh = build::Mesh {
        name: "Cube",
        positions: &POSITIONS,
        normals: &[],
        uvs: &[],
        material: None,
        faces: build::Faces::Triangles(&TRIANGLES),
    };
    let document = build::build(&build::Scene {
        meshes: &[mesh],
        bones: &bones,
        skins: &skins,
        materials: &[],
        clips: &[],
        time_mode: 6,
    });
    encode::encode(&document).expect("our encoder writes the cube")
}

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

/// Maya reads OUR encoder's output — the check that catches the footer-CRC and
/// DefaultAttributeIndex conformance bugs that only Maya rejects. Ignored
/// because it needs a local Maya; run with `--ignored`.
#[test]
#[ignore = "needs a local Maya install (mayapy)"]
fn maya_reads_our_own_encoder_output() {
    let mayapy = match maya::mayapy_path() {
        Ok(path) => path,
        Err(_) => return,
    };
    let report = maya::inspect(&our_rigged_cube_fbx(), &mayapy).expect("inspects");
    assert!(report.imported, "Maya rejected our fbx: {:?}", report.error);
    // Two joints bound, one mesh, one skin — a flat count of zero here is the
    // DefaultAttributeIndex regression, which is the whole point of the check.
    assert_eq!(report.joints, Some(2), "our joints did not bind in Maya");
    assert_eq!(report.meshes, Some(1), "our mesh did not import in Maya");
    assert_eq!(report.skin_clusters, Some(1));
    assert_eq!(report.mesh_vertices, vec![8]);
    assert_eq!(report.weighted_vertices, Some(8));
    assert_eq!(report.weight_total, Some(8.0));
    assert_eq!(report.root_joints, vec!["Root"]);
}

/// The full check: a real human rig re-encoded through OUR fbx writer
/// (`fbx::roundtrip::reencode`, the same path the rebuild_rig example uses) must
/// still open in Maya with its skeleton, meshes and skinning intact — the O9
/// acceptance test carried all the way to the strict reader. Ignored; needs Maya.
#[test]
#[ignore = "needs a local Maya install (mayapy)"]
fn maya_reads_our_reencoded_human_rig() {
    let mayapy = match maya::mayapy_path() {
        Ok(path) => path,
        Err(_) => return,
    };
    let source = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/test-files/retarget testing/mixamo-original-rig.fbx"
    ))
    .expect("the reference fbx");
    let ours = m2m_io::fbx::roundtrip::reencode(&source).expect("our encoder re-encodes the rig");

    let report = maya::inspect(&ours, &mayapy).expect("inspects");
    assert!(
        report.imported,
        "Maya rejected our re-encoded rig: {:?}",
        report.error
    );
    assert_eq!(
        report.joints,
        Some(65),
        "skeleton did not survive our re-encode"
    );
    assert_eq!(
        report.meshes,
        Some(2),
        "meshes did not survive our re-encode"
    );
    assert_eq!(
        report.skin_clusters,
        Some(2),
        "skinning did not survive our re-encode"
    );
    assert_eq!(report.mesh_vertices, vec![10514, 14232]);
    assert_eq!(report.root_joints, vec!["mixamorig:Hips"]);
}
