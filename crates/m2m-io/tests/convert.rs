//! FBX reaching the glTF document model — the payload the bulk IPC channel
//! carries (`memory/architecture.md` §4).

use m2m_io::convert::fbx_to_gltf;
use m2m_io::fbx::{binary, dom::Scene};
use m2m_io::glb;

const RIG: &[u8] =
    include_bytes!("../../../legacy/static/test-files/retarget testing/mixamo-original-rig.fbx");

fn scene() -> Scene {
    Scene::from_document(binary::parse(RIG).expect("the reference rig parses"))
}

fn document() -> glb::Document {
    fbx_to_gltf(&scene()).expect("converts")
}

#[test]
fn the_skeleton_and_both_meshes_survive_the_conversion() {
    let document = document();

    // 65 bones plus the two mesh models the FBX also carries as Models.
    assert_eq!(document.nodes.len(), 67);
    assert_eq!(document.primitives.len(), 2);
    assert_eq!(document.skins.len(), 2);
    assert!(document.nodes.iter().any(|n| n.name == "mixamorig:Hips"));
    assert!(document
        .nodes
        .iter()
        .any(|n| n.name == "mixamorig:LeftHand"));
}

#[test]
fn a_node_never_precedes_its_own_parent() {
    // glTF resolves `parent` as an index into this same list, and the writer
    // walks it in order, so a child appearing first is unresolvable.
    for (i, node) in document().nodes.iter().enumerate() {
        if let Some(parent) = node.parent {
            assert!(parent < i, "node {i} ({}) precedes its parent", node.name);
        }
    }
}

#[test]
fn the_character_arrives_in_metres() {
    // FBX counts in the unit `UnitScaleFactor` declares — centimetres for
    // Mixamo — and glTF is defined in metres. Getting this wrong is invisible
    // in every structural assertion and produces a 180-metre character.
    assert_eq!(scene().unit_scale, Some(1.0));

    let document = document();
    let root_scale = document
        .nodes
        .iter()
        .find(|n| n.parent.is_none())
        .expect("a root")
        .transform
        .scale;
    assert!((root_scale[0] - 0.01).abs() < 1e-9, "got {root_scale:?}");

    let tallest = document
        .primitives
        .iter()
        .map(|p| {
            let ys: Vec<f32> = p.positions.chunks_exact(3).map(|v| v[1]).collect();
            let span = ys.iter().copied().fold(f32::MIN, f32::max)
                - ys.iter().copied().fold(f32::MAX, f32::min);
            span * root_scale[1]
        })
        .fold(0.0f32, f32::max);
    assert!(
        (1.2..2.5).contains(&tallest),
        "character is {tallest} m tall"
    );
}

#[test]
fn every_joint_points_at_a_node_that_exists() {
    let document = document();
    for skin in &document.skins {
        assert_eq!(
            skin.joints.len(),
            skin.inverse_bind_matrices.len(),
            "a joint without a bind matrix binds to the identity"
        );
        for &joint in &skin.joints {
            assert!(joint < document.nodes.len(), "joint {joint} has no node");
        }
    }
}

#[test]
fn every_skinned_vertex_keeps_a_full_share_of_weight() {
    // `bind` renormalises, so a vertex summing to anything but one means
    // influences were lost between binding and the glTF arrays.
    let document = document();
    let mut checked = 0usize;
    for primitive in &document.primitives {
        if primitive.weights.is_empty() {
            continue;
        }
        assert_eq!(primitive.joints.len(), primitive.weights.len());
        assert_eq!(primitive.weights.len(), primitive.positions.len() / 3 * 4);
        for vertex in primitive.weights.chunks_exact(4) {
            let total: f32 = vertex.iter().sum();
            assert!((total - 1.0).abs() < 1e-4, "vertex weight sums to {total}");
            checked += 1;
        }
    }
    assert!(checked > 20_000, "only {checked} vertices checked");
}

#[test]
fn the_joint_list_is_every_cluster_in_order() {
    // `Skin::bind` numbers bones by their position in `clusters`, so a joint
    // list shorter than the cluster list would silently deform the mesh to the
    // wrong bones. Counted against the reader's own view of the same file.
    let scene = scene();
    let (skins, _) = m2m_io::fbx::skin::parse_all(&scene);
    let document = fbx_to_gltf(&scene).expect("converts");

    let mut clusters: Vec<usize> = skins.iter().map(|s| s.clusters.len()).collect();
    let mut joints: Vec<usize> = document.skins.iter().map(|s| s.joints.len()).collect();
    clusters.sort_unstable();
    joints.sort_unstable();
    assert_eq!(joints, clusters);
    assert_eq!(clusters, vec![64, 65]);
}

#[test]
fn the_converted_document_can_be_written_and_read_back() {
    // The whole point: this document exists to become a `.glb` on the wire.
    let document = document();
    let bytes = glb::write(&document).expect("writes");
    let back = glb::read(&bytes).expect("reads back");

    assert_eq!(back.nodes.len(), document.nodes.len());
    assert_eq!(back.skins.len(), document.skins.len());
    assert_eq!(back.mesh_count(), document.primitives.len());
    assert_eq!(
        back.skins.iter().map(|s| s.joints.len()).sum::<usize>(),
        document.skins.iter().map(|s| s.joints.len()).sum::<usize>()
    );
}

#[test]
fn only_the_roots_carry_the_unit_scale() {
    // The unit factor goes on the roots so everything below inherits it once.
    // Putting it on every node compounds it down the chain, and no assertion
    // about the root's own scale would notice.
    for node in document().nodes.iter().filter(|n| n.parent.is_some()) {
        assert_eq!(
            node.transform.scale,
            [1.0, 1.0, 1.0],
            "{} carries a scale of its own",
            node.name
        );
    }
}

#[test]
fn the_node_hierarchy_reproduces_the_bind_pose_the_file_recorded() {
    // An inverse bind matrix cannot be checked by counting anything, and
    // Blender's import report would not notice a wrong one — it reports bones
    // and weights, not deformation. The file states the bind pose twice, in
    // the node transforms and in each cluster's `TransformLink`, so the two
    // can be held against each other.
    use glam::{Mat4, Quat, Vec3};

    let scene = scene();
    let models = m2m_io::fbx::model::parse_all(&scene);
    let (fbx_skins, _) = m2m_io::fbx::skin::parse_all(&scene);
    let document = fbx_to_gltf(&scene).expect("converts");

    let mut world = vec![Mat4::IDENTITY; document.nodes.len()];
    for (i, node) in document.nodes.iter().enumerate() {
        let local = Mat4::from_scale_rotation_translation(
            Vec3::from(node.transform.scale),
            Quat::from_array(node.transform.rotation),
            Vec3::from(node.transform.translation),
        );
        world[i] = node.parent.map_or(local, |p| world[p] * local);
    }

    let mut worst = 0.0f32;
    for skin in &document.skins {
        let names: Vec<&str> = skin
            .joints
            .iter()
            .map(|&j| document.nodes[j].name.as_str())
            .collect();

        // Matched by joint names in order, not by index: the converter emits
        // skins in node order and `parse_all` returns them in its own, so
        // pairing them positionally silently compares the wrong two.
        let matches: Vec<_> = fbx_skins
            .iter()
            .filter(|f| {
                f.clusters.len() == names.len()
                    && f.clusters
                        .iter()
                        .zip(&names)
                        .all(|(c, name)| models.get(c.bone_id).is_some_and(|m| m.name == *name))
            })
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "no single FBX skin matches this joint list"
        );

        for (joint, cluster) in skin.joints.iter().zip(&matches[0].clusters) {
            // `TransformLink` is the bone's world at bind, in file units. The
            // unit factor is applied as a matrix, not as a scalar: scaling all
            // sixteen elements would scale the homogeneous row too.
            let recorded = Mat4::from_scale(Vec3::splat(0.01)) * cluster.transform_link.as_mat4();
            for (a, b) in world[*joint]
                .to_cols_array()
                .iter()
                .zip(recorded.to_cols_array().iter())
            {
                worst = worst.max((a - b).abs());
            }

            // And the matrix itself. At bind the bone's matrix *is*
            // `TransformLink`, so `jointWorld · IBM` collapses to the mesh
            // transform the exporter recorded — which for this rig is not the
            // identity, so the assertion has something to fail against.
            let ibm = Mat4::from_cols_array(
                &skin.inverse_bind_matrices
                    [skin.joints.iter().position(|j| j == joint).expect("joint")],
            );
            let bound = Mat4::from_scale(Vec3::splat(0.01)) * cluster.transform.as_mat4();
            for (a, b) in (world[*joint] * ibm)
                .to_cols_array()
                .iter()
                .zip(bound.to_cols_array().iter())
            {
                worst = worst.max((a - b).abs());
            }
        }
    }
    assert!(worst < 1e-5, "bind pose differs by {worst}");
}

#[test]
fn a_joint_is_only_deforming_for_the_skin_it_belongs_to() {
    // A joint index is an offset into ONE skin's joint list and means nothing
    // outside it, so asking which of a skin's joints are idle has to look only
    // at the meshes that skin deforms. The converted rig has two skins over two
    // meshes, which is the smallest case where the difference shows.
    let document = document();
    assert_eq!(document.skins.len(), 2, "the fixture needs two skins");

    let deforming: Vec<usize> = (0..document.skins.len())
        .map(|i| document.skins[i].joints.len() - document.non_deforming_joints(i).len())
        .collect();

    // Measured, not chosen: the body mesh recruits more bones than the joint
    // caps do. Counting every primitive instead reports 57 and 58.
    assert_eq!(deforming, vec![39, 50]);
}
