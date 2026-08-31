//! Placing a template skeleton onto a target mesh.
//!
//! These run against the real assets, because the whole question is whether a
//! template lands inside the animal it is meant to rig. A fitter tested only on
//! synthetic boxes tells you nothing about that.

use glam::{Mat4, Quat, Vec3};
use m2m_core::mesh::Mesh;
use m2m_core::voxel::{VoxelGrid, VoxelState};
use m2m_rig::fit::{
    ankle_height, body_axis, fit_uniform, ground_bone, refine_spine, BodyAxis, Landmarks, RestPose,
};
use m2m_rig::template::{ChainKind, LimbRole, Posture, Template};

fn asset(relative: &str) -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../legacy/static/").to_owned() + relative;
    std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"))
}

/// Loads a `.glb`'s geometry as one mesh in world space.
///
/// Node transforms are composed rather than assumed to be identity. They happen
/// to be identity on every `model-*.glb` — measured — but a mesh that quietly
/// ignored them would be wrong on the first asset that has one.
fn mesh_of(relative: &str) -> Mesh {
    let bytes = asset(relative);
    let document = m2m_io::glb::read(&bytes).expect("reads");
    let world = world_transforms(&document);

    let mut mesh = Mesh::default();
    for primitive in &document.primitives {
        let transform = primitive.node.map_or(Mat4::IDENTITY, |n| world[n]);
        let base = mesh.positions.len() as u32;
        for chunk in primitive.positions.chunks_exact(3) {
            let local = Vec3::new(chunk[0], chunk[1], chunk[2]);
            mesh.positions.push(transform.transform_point3(local));
        }
        mesh.indices
            .extend(primitive.indices.iter().map(|i| i + base));
    }
    mesh
}

/// World matrix per node, composed down the hierarchy.
fn world_transforms(document: &m2m_io::glb::Document) -> Vec<Mat4> {
    let local: Vec<Mat4> = document
        .nodes
        .iter()
        .map(|n| {
            Mat4::from_scale_rotation_translation(
                Vec3::from(n.transform.scale),
                Quat::from_array(n.transform.rotation),
                Vec3::from(n.transform.translation),
            )
        })
        .collect();

    let mut world = vec![Mat4::IDENTITY; document.nodes.len()];
    let mut done = vec![false; document.nodes.len()];
    // Depth-first from each node, memoised, so a deep chain is composed once.
    fn resolve(
        index: usize,
        nodes: &[m2m_io::glb::Node],
        local: &[Mat4],
        world: &mut [Mat4],
        done: &mut [bool],
    ) -> Mat4 {
        if done[index] {
            return world[index];
        }
        let matrix = match nodes[index].parent {
            Some(parent) => resolve(parent, nodes, local, world, done) * local[index],
            None => local[index],
        };
        world[index] = matrix;
        done[index] = true;
        matrix
    }
    for index in 0..document.nodes.len() {
        resolve(index, &document.nodes, &local, &mut world, &mut done);
    }
    world
}

/// A template skeleton's rest pose, in world space.
fn rest_pose_of(relative: &str) -> RestPose {
    let bytes = asset(relative);
    let document = m2m_io::glb::read(&bytes).expect("reads");
    let world = world_transforms(&document);
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

/// The spine bones a template declares.
fn spine_of(manifest: &str) -> Vec<String> {
    template(manifest)
        .of_kind(ChainKind::Spine)
        .flat_map(|c| c.bones.clone())
        .collect()
}

fn template(manifest: &str) -> Template {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/templates/").to_owned() + manifest;
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    serde_json::from_str(&text).expect("parses")
}

/// Landmarks describe the mesh they were measured from.
#[test]
fn landmarks_measure_the_mesh() {
    let mesh = mesh_of("models/model-human.glb");
    let landmarks = Landmarks::of(&mesh).expect("the mesh has vertices");

    // Measured from the accessor bounds of the file itself.
    assert!(
        (landmarks.extent().y - 1.830).abs() < 0.01,
        "{:?}",
        landmarks.extent()
    );
    assert_eq!(landmarks.ground, landmarks.min.y);
    assert!((landmarks.symmetry_x - (landmarks.min.x + landmarks.max.x) / 2.0).abs() < 1e-6);

    // A humanoid is close to symmetric about its midline. The threshold is
    // loose on purpose: this reports asymmetry, it does not gate on it.
    let error = landmarks.symmetry_error(&mesh);
    assert!(
        error < 0.02,
        "symmetry error {error} is high for a human mesh"
    );
}

/// The mesh's widest extent is **not** its body axis, which is why the body
/// axis comes from the template.
///
/// Pinned as a test because it is the assumption the fitter would otherwise
/// make, and it is wrong for four of the nine shipped models.
#[test]
fn the_widest_extent_is_not_the_body_axis() {
    for (model, widest_is_x) in [
        ("models/model-human.glb", true),  // arm span beats height
        ("models/model-bird.glb", true),   // wingspan
        ("models/model-spider.glb", true), // leg spread
        ("models/model-fox.glb", false),   // body length, along Z
    ] {
        let mesh = mesh_of(model);
        let extent = Landmarks::of(&mesh).expect("vertices").extent();
        assert_eq!(
            extent.x > extent.y && extent.x > extent.z,
            widest_is_x,
            "{model} extent {extent:?}"
        );
    }
}

/// Each template's spine says which way that creature's body runs.
///
/// The classification the whole fit depends on, asserted directly rather than
/// only through its consequences.
#[test]
fn each_template_reports_the_body_axis_its_spine_implies() {
    // Measured from each rest pose, not assumed. The spider caught me out: I
    // expected Upright because it walks on legs, and its spine actually runs
    // +0.002 in Y against +0.299 in Z -- a spider's body is as horizontal as a
    // fox's. The kaiju is the only quadruped-looking one that is upright, its
    // spine climbing 1.043 while moving 0.861 forward.
    for (rig, manifest, expected) in [
        ("rigs/rig-human.glb", "human.json", BodyAxis::Upright),
        ("rigs/rig-kaiju.glb", "kaiju.json", BodyAxis::Upright),
        ("rigs/rig-spider.glb", "spider.json", BodyAxis::Horizontal),
        ("rigs/rig-fox.glb", "fox.json", BodyAxis::Horizontal),
        ("rigs/rig-horse.glb", "horse.json", BodyAxis::Horizontal),
        ("rigs/rig-shark.glb", "shark.json", BodyAxis::Horizontal),
        ("rigs/rig-snake.glb", "snake.json", BodyAxis::Horizontal),
        ("rigs/rig-bird.glb", "bird.json", BodyAxis::Horizontal),
        ("rigs/rig-dragon.glb", "dragon.json", BodyAxis::Horizontal),
    ] {
        let rest = rest_pose_of(rig);
        let axis = body_axis(&rest, &spine_of(manifest));
        assert_eq!(axis, Some(expected), "{manifest}");
    }
}

/// A spine too short to give a direction reports none rather than guessing.
#[test]
fn a_spine_without_a_direction_reports_none() {
    let rest = rest_pose_of("rigs/rig-human.glb");
    assert_eq!(body_axis(&rest, &[]), None, "no bones");
    assert_eq!(
        body_axis(&rest, &["pelvis".to_string()]),
        None,
        "one bone is a point, not a direction"
    );
    assert_eq!(
        body_axis(
            &rest,
            &["nonexistent".to_string(), "also_missing".to_string()]
        ),
        None,
        "bones the rest pose does not have"
    );
}

/// The fitted skeleton stands on the mesh's ground plane at the right size.
#[test]
fn a_fitted_skeleton_stands_on_the_ground_at_mesh_height() {
    let mesh = mesh_of("models/model-human.glb");
    let landmarks = Landmarks::of(&mesh).expect("vertices");
    let rest = rest_pose_of("rigs/rig-human.glb");
    let fitted = fit_uniform(&rest, &landmarks, &spine_of("human.json")).expect("fits");

    let lowest = fitted
        .positions
        .iter()
        .map(|p| p.y)
        .fold(f32::INFINITY, f32::min);
    let highest = fitted
        .positions
        .iter()
        .map(|p| p.y)
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        (lowest - landmarks.ground).abs() < 1e-4,
        "feet at {lowest}, ground at {}",
        landmarks.ground
    );
    assert!(
        (highest - landmarks.max.y).abs() < 1e-4,
        "head at {highest}, top of mesh at {}",
        landmarks.max.y
    );
    assert!(fitted.scale > 0.0);
}

/// Fitting a template onto its **own** base model barely moves it.
///
/// Worth pinning, because it is the reason the base pairs are a weak fixture:
/// the rigs were authored against these meshes, so the fit is nearly the
/// identity and would hide almost any mistake. A mutation that dropped ground
/// alignment entirely moved the human skeleton by 0.0004 units and the
/// inside-the-mesh check never noticed.
#[test]
fn a_base_model_and_its_own_rig_are_already_almost_aligned() {
    for (model, rig, manifest, scale) in [
        (
            "models/model-human.glb",
            "rigs/rig-human.glb",
            "human.json",
            1.117,
        ),
        (
            "models/model-fox.glb",
            "rigs/rig-fox.glb",
            "fox.json",
            1.014,
        ),
    ] {
        let mesh = mesh_of(model);
        let landmarks = Landmarks::of(&mesh).expect("vertices");
        let rest = rest_pose_of(rig);
        let fitted = fit_uniform(&rest, &landmarks, &spine_of(manifest)).expect("fits");
        assert!(
            (fitted.scale - scale).abs() < 0.01,
            "{model}: scale {} is not the expected {scale}",
            fitted.scale
        );
        assert!(
            fitted.offset.length() < 0.05,
            "{model}: offset {:?} is larger than expected",
            fitted.offset
        );
    }
}

/// Every fitted spine joint lands inside the mesh.
///
/// This is the acceptance test the whole increment exists for: a skeleton whose
/// spine floats outside the body is not a fit, however plausible its bounding
/// box looks. Voxels are the arbiter rather than a distance heuristic, because
/// "inside" is exactly what a voxelisation answers.
///
/// The base pairs are checked first, then the **variation** meshes, which is
/// where the test gets its teeth: they range from 0.99x to 2.30x the base
/// height with quite different proportions, so a fitter that only works on the
/// mesh a rig was authored against fails here.
#[test]
fn the_fitted_spine_lands_inside_the_mesh() {
    // `budget` is how far outside the mesh a spine joint may sit, as a
    // fraction of body height. Zero means every joint must be inside.
    //
    // Only `sintel` needs one: her spine_03 sits 0.031 out on a 1.82-tall body,
    // about 1.7% of her height. That is the limit of a single global scale, and
    // it is what this test exists to record -- **`refine_spine` removes it**,
    // and `refinement_puts_every_spine_joint_inside_every_body` asserts zero
    // budget for the same eight bodies. This one stays because knowing what the
    // uniform placement alone achieves is what says whether refinement is
    // earning anything.
    for (model, rig, manifest, budget) in [
        (
            "models/model-human.glb",
            "rigs/rig-human.glb",
            "human.json",
            0.0,
        ),
        ("models/model-fox.glb", "rigs/rig-fox.glb", "fox.json", 0.0),
        (
            "models-variation/human-female.glb",
            "rigs/rig-human.glb",
            "human.json",
            0.0,
        ),
        (
            "models-variation/human-sophia.glb",
            "rigs/rig-human.glb",
            "human.json",
            0.0,
        ),
        (
            "models-variation/human-jay.glb",
            "rigs/rig-human.glb",
            "human.json",
            0.0,
        ),
        (
            "models-variation/human-zombie.glb",
            "rigs/rig-human.glb",
            "human.json",
            0.0,
        ),
        (
            "models-variation/human-bunny.glb",
            "rigs/rig-human.glb",
            "human.json",
            0.0,
        ),
        (
            "models-variation/human-sintel.glb",
            "rigs/rig-human.glb",
            "human.json",
            0.02,
        ),
    ] {
        let mesh = mesh_of(model);
        let landmarks = Landmarks::of(&mesh).expect("vertices");
        let rest = rest_pose_of(rig);
        let spine = spine_of(manifest);
        let fitted = fit_uniform(&rest, &landmarks, &spine).expect("fits");
        let grid = VoxelGrid::build(&mesh, 128).expect("voxelises");

        let spine: Vec<&str> = spine.iter().map(String::as_str).collect();
        assert!(!spine.is_empty(), "{manifest} has no spine");

        // A joint counts as placed when its voxel is inside or on the surface.
        // A joint the voxels call outside is then measured against the mesh:
        // grazing the surface by a fraction of a voxel is not a misplacement,
        // and demanding strict interior would be asserting a precision a single
        // uniform scale cannot promise. `sintel`'s spine_03 sits 0.002 outside
        // on a 1.82-tall body -- 0.1% of its height, about a seventh of a
        // voxel. A genuinely misplaced joint is off by tens of times that.
        let height = landmarks.extent().y;
        let tolerance = height * budget;
        let misplaced: Vec<(&str, f32)> = spine
            .iter()
            .filter_map(|bone| {
                let at = fitted.position_of(bone)?;
                if matches!(
                    grid.state(grid.coord_of(at)),
                    Some(VoxelState::Interior | VoxelState::Surface)
                ) {
                    return None;
                }
                let nearest = mesh
                    .positions
                    .iter()
                    .map(|v| v.distance(at))
                    .fold(f32::INFINITY, f32::min);
                (nearest > tolerance).then_some((*bone, nearest))
            })
            .collect();
        assert!(
            misplaced.is_empty(),
            "{model}: spine joints further than {tolerance:.4} from the mesh \
             (budget {budget} of {height:.3} height): {misplaced:?}"
        );
    }
}

/// Refinement puts every spine joint inside every body, budget-free.
///
/// The uniform placement left `human-sintel`'s spine_03 0.031 outside her chest
/// and needed a stated budget to pass. Following the midline at each joint's own
/// height removes it: this asserts **zero** allowance for all eight bodies, so
/// the budget the previous increment carried is gone rather than relaxed.
#[test]
fn refinement_puts_every_spine_joint_inside_every_body() {
    for (model, rig, manifest) in [
        ("models/model-human.glb", "rigs/rig-human.glb", "human.json"),
        ("models/model-fox.glb", "rigs/rig-fox.glb", "fox.json"),
        ("models/model-horse.glb", "rigs/rig-horse.glb", "horse.json"),
        (
            "models-variation/human-female.glb",
            "rigs/rig-human.glb",
            "human.json",
        ),
        (
            "models-variation/human-sintel.glb",
            "rigs/rig-human.glb",
            "human.json",
        ),
        (
            "models-variation/human-sophia.glb",
            "rigs/rig-human.glb",
            "human.json",
        ),
        (
            "models-variation/human-jay.glb",
            "rigs/rig-human.glb",
            "human.json",
        ),
        (
            "models-variation/human-zombie.glb",
            "rigs/rig-human.glb",
            "human.json",
        ),
        (
            "models-variation/human-bunny.glb",
            "rigs/rig-human.glb",
            "human.json",
        ),
    ] {
        let mesh = mesh_of(model);
        let landmarks = Landmarks::of(&mesh).expect("vertices");
        let rest = rest_pose_of(rig);
        let spine = spine_of(manifest);
        let mut fitted = fit_uniform(&rest, &landmarks, &spine).expect("fits");
        let axis = body_axis(&rest, &spine).expect("an axis");
        refine_spine(&mut fitted, &mesh, &landmarks, &spine, axis);

        let grid = VoxelGrid::build(&mesh, 128).expect("voxelises");
        let outside: Vec<&str> = spine
            .iter()
            .filter(|bone| {
                fitted.position_of(bone).is_none_or(|at| {
                    !matches!(
                        grid.state(grid.coord_of(at)),
                        Some(VoxelState::Interior | VoxelState::Surface)
                    )
                })
            })
            .map(String::as_str)
            .collect();
        assert!(
            outside.is_empty(),
            "{model}: spine joints still outside after refinement: {outside:?}"
        );
    }
}

/// Refinement moves the joint that was wrong and leaves the rest roughly alone.
///
/// A refinement that shifted every joint by a lot would also pass the test
/// above, by dragging the whole spine to the mesh's centre regardless of what
/// the template said.
#[test]
fn refinement_is_a_correction_not_a_relocation() {
    let mesh = mesh_of("models-variation/human-sintel.glb");
    let landmarks = Landmarks::of(&mesh).expect("vertices");
    let rest = rest_pose_of("rigs/rig-human.glb");
    let spine = spine_of("human.json");
    let before = fit_uniform(&rest, &landmarks, &spine).expect("fits");
    let mut after = before.clone();
    let axis = body_axis(&rest, &spine).expect("an axis");
    refine_spine(&mut after, &mesh, &landmarks, &spine, axis);

    let height = landmarks.extent().y;
    for bone in &spine {
        let (a, b) = (
            before.position_of(bone).expect("placed"),
            after.position_of(bone).expect("placed"),
        );
        assert_eq!(a.y, b.y, "{bone}: refinement must not change height");
        assert!(
            a.distance(b) < height * 0.10,
            "{bone} moved {:.3}, more than a correction",
            a.distance(b)
        );
    }
}

/// The ground contact is the chain's lowest bone, in every posture.
///
/// Pinned because the first implementation guessed it from posture by counting
/// back from the end of the chain, and the rest poses say all three ground at
/// the last bone.
#[test]
fn the_ground_contact_is_the_lowest_bone_of_the_chain() {
    for (rig, manifest, chain_name) in [
        ("rigs/rig-human.glb", "human.json", "leg_l"),
        ("rigs/rig-fox.glb", "fox.json", "back_leg_l"),
        ("rigs/rig-horse.glb", "horse.json", "back_leg_l"),
    ] {
        let rest = rest_pose_of(rig);
        let manifest_data = template(manifest);
        let chain = manifest_data
            .chains
            .iter()
            .find(|c| c.name == chain_name)
            .expect("the chain");
        let bone = ground_bone(chain, &rest).expect("a ground bone");
        assert_eq!(
            Some(bone),
            chain.bones.last().map(String::as_str),
            "{manifest} {chain_name}"
        );
    }
}

/// Posture separates the three ankle heights, and separates them widely.
///
/// This is what posture is actually for. Measured on the rest poses: a
/// plantigrade human's ankle sits at 6% of its height, a digitigrade fox's at
/// 21%, an unguligrade horse's at 26%.
#[test]
fn posture_separates_ankle_height() {
    let ratio = |rig: &str, manifest: &str, chain_name: &str| -> f32 {
        let rest = rest_pose_of(rig);
        let manifest_data = template(manifest);
        let chain = manifest_data
            .chains
            .iter()
            .find(|c| c.name == chain_name)
            .expect("the chain");
        let (lo, hi) = rest.bounds().expect("bones");
        ankle_height(chain, &rest).expect("an ankle") / (hi.y - lo.y)
    };
    let human = ratio("rigs/rig-human.glb", "human.json", "leg_l");
    let fox = ratio("rigs/rig-fox.glb", "fox.json", "back_leg_l");
    let horse = ratio("rigs/rig-horse.glb", "horse.json", "back_leg_l");

    assert!((human - 0.06).abs() < 0.02, "human ankle ratio {human}");
    assert!((fox - 0.21).abs() < 0.03, "fox ankle ratio {fox}");
    assert!((horse - 0.263).abs() < 0.02, "horse ankle ratio {horse}");
    assert!(
        human < fox && fox < horse,
        "the three postures should order by ankle height: {human} {fox} {horse}"
    );
}

/// A limb with no posture reports no ankle height rather than a mammal's.
///
/// A spider's leg is an arthropod's and a wing does not touch the ground, so
/// both must come back empty instead of being handed a default.
#[test]
fn limbs_without_posture_report_no_ankle_height() {
    for (rig, manifest, role) in [
        ("rigs/rig-spider.glb", "spider.json", LimbRole::Leg),
        ("rigs/rig-bird.glb", "bird.json", LimbRole::Wing),
        ("rigs/rig-shark.glb", "shark.json", LimbRole::Fin),
    ] {
        let rest = rest_pose_of(rig);
        let manifest_data = template(manifest);
        let mut checked = 0;
        for chain in manifest_data
            .of_kind(ChainKind::Limb)
            .filter(|c| c.role == Some(role))
        {
            assert_eq!(chain.posture, None, "{manifest} {}", chain.name);
            assert_eq!(
                ankle_height(chain, &rest),
                None,
                "{manifest} {}: no posture must mean no ankle height",
                chain.name
            );
            checked += 1;
        }
        assert!(checked > 0, "{manifest} has no {role:?} limbs to check");
    }
}

/// Posture is only ever one of the three, and the templates that carry it are
/// the ones with legs on the ground.
#[test]
fn only_legs_carry_a_posture() {
    for manifest in [
        "human.json",
        "fox.json",
        "bird.json",
        "spider.json",
        "snake.json",
        "shark.json",
        "horse.json",
        "kaiju.json",
        "dragon.json",
    ] {
        for chain in template(manifest).of_kind(ChainKind::Limb) {
            if chain.posture.is_some() {
                assert_eq!(
                    chain.role,
                    Some(LimbRole::Leg),
                    "{manifest} {}: only a leg meets the ground",
                    chain.name
                );
                assert!(matches!(
                    chain.posture,
                    Some(Posture::Plantigrade | Posture::Digitigrade | Posture::Unguligrade)
                ));
            }
        }
    }
}
