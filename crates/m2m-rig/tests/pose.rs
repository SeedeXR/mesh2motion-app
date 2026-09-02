//! Pose detection against the real A-pose and T-pose meshes.
//!
//! The unit tests in `src/pose.rs` prove the angle maths on synthetic vectors.
//! This proves the whole path: fit the human template onto a real character and
//! read the pose off where the arms actually landed. The two fixtures are the
//! same human in the two poses, so the classifier has to tell them apart.

use glam::{Mat4, Vec3};
use m2m_core::mesh::Mesh;
use m2m_rig::fit::{fit_template, Fitted, RestPose};
use m2m_rig::pose::{pose_of_fitted, Pose};
use m2m_rig::template::Template;

fn asset(relative: &str) -> Vec<u8> {
    let path = match relative.strip_prefix("rigs/") {
        Some(rig) => concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/rigs/").to_owned() + rig,
        None => concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/").to_owned() + relative,
    };
    std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"))
}

fn mesh_of(relative: &str) -> Mesh {
    let document = m2m_io::glb::read(&asset(relative)).expect("reads");
    let world = document.world_transforms();
    let mut mesh = Mesh::default();
    for primitive in &document.primitives {
        let transform = primitive.node.map_or(Mat4::IDENTITY, |n| world[n]);
        let base = mesh.positions.len() as u32;
        for chunk in primitive.positions.chunks_exact(3) {
            mesh.positions
                .push(transform.transform_point3(Vec3::new(chunk[0], chunk[1], chunk[2])));
        }
        mesh.indices
            .extend(primitive.indices.iter().map(|i| i + base));
    }
    mesh
}

fn rest_pose_of(relative: &str) -> RestPose {
    let document = m2m_io::glb::read(&asset(relative)).expect("reads");
    let world = document.world_transforms();
    let skin = document.skins.first().expect("the template has a skin");
    RestPose {
        bones: skin
            .joints
            .iter()
            .map(|&j| document.nodes[j].name.clone())
            .collect(),
        positions: skin
            .joints
            .iter()
            .map(|&j| world[j].transform_point3(Vec3::ZERO))
            .collect(),
    }
}

fn template(manifest: &str) -> Template {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/templates/").to_owned() + manifest;
    serde_json::from_str(&std::fs::read_to_string(path).expect("reads")).expect("parses")
}

fn fit_human(mesh_path: &str) -> Fitted {
    let human = template("human.json");
    let rest = rest_pose_of("rigs/rig-human.glb");
    let mesh = mesh_of(mesh_path);
    fit_template(&human, &rest, &mesh, 128).expect("fits")
}

#[test]
fn the_t_pose_mesh_reads_as_a_t_pose() {
    let fitted = fit_human("models/model-human.glb");
    assert_eq!(pose_of_fitted(&fitted, Vec3::Y), Pose::TPose);
}

#[test]
fn the_a_pose_mesh_reads_as_an_a_pose() {
    let fitted = fit_human("test-files/bone-correction-tests/human-a-pose.glb");
    let pose = pose_of_fitted(&fitted, Vec3::Y);
    // The whole point of the epic: the same human, arms down, is NOT a T-pose.
    assert_ne!(pose, Pose::TPose, "arms-down must not read as arms-out");
    assert_eq!(pose, Pose::APose);
}
