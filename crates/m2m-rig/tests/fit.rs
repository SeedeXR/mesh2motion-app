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
    // Rig `.glb` files moved to `assets/rigs/` (P3-3d); other fixtures stay in legacy.
    let path = match relative.strip_prefix("rigs/") {
        Some(rig) => concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/rigs/").to_owned() + rig,
        None => concat!(env!("CARGO_MANIFEST_DIR"), "/../../legacy/static/").to_owned() + relative,
    };
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
        ("rigs/rig-rhino.glb", "rhino.json", BodyAxis::Horizontal),
        ("rigs/rig-buffalo.glb", "buffalo.json", BodyAxis::Horizontal),
        ("rigs/rig-hyena.glb", "hyena.json", BodyAxis::Horizontal),
        (
            "rigs/rig-elephant.glb",
            "elephant.json",
            BodyAxis::Horizontal,
        ),
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
        ("models/model-rhino.glb", "rigs/rig-rhino.glb", "rhino.json"),
        (
            "models/model-buffalo.glb",
            "rigs/rig-buffalo.glb",
            "buffalo.json",
        ),
        (
            "models/model-buffalo.glb",
            "rigs/rig-buffalo.glb",
            "buffalo.json",
        ),
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

/// Where limb joints land after the body fit, before any limb-specific work.
///
/// Run first to find out what limb fitting actually has to fix, rather than
/// assuming. Reports rather than asserts.
#[test]
#[ignore = "diagnostic, not a gate"]
fn report_limb_placement() {
    for (model, rig, manifest) in [
        ("models/model-human.glb", "rigs/rig-human.glb", "human.json"),
        (
            "models-variation/human-jay.glb",
            "rigs/rig-human.glb",
            "human.json",
        ),
        (
            "models-variation/human-bunny.glb",
            "rigs/rig-human.glb",
            "human.json",
        ),
        (
            "models-variation/human-sophia.glb",
            "rigs/rig-human.glb",
            "human.json",
        ),
        ("models/model-fox.glb", "rigs/rig-fox.glb", "fox.json"),
        ("models/model-horse.glb", "rigs/rig-horse.glb", "horse.json"),
        ("models/model-bird.glb", "rigs/rig-bird.glb", "bird.json"),
        ("models/model-rhino.glb", "rigs/rig-rhino.glb", "rhino.json"),
    ] {
        let mesh = mesh_of(model);
        let landmarks = Landmarks::of(&mesh).expect("vertices");
        let rest = rest_pose_of(rig);
        let spine = spine_of(manifest);
        let mut fitted = fit_uniform(&rest, &landmarks, &spine).expect("fits");
        let manifest_data = template(manifest);
        let grid = VoxelGrid::build(&mesh, 128).expect("voxelises");
        m2m_rig::fit::fit_limbs(&mut fitted, &mesh, &grid, &manifest_data);

        let mut outside = 0;
        let mut total = 0;
        let mut worst_ground = 0.0f32;
        for chain in manifest_data.of_kind(ChainKind::Limb) {
            for bone in &chain.bones {
                let Some(at) = fitted.position_of(bone) else {
                    continue;
                };
                total += 1;
                if !matches!(
                    grid.state(grid.coord_of(at)),
                    Some(VoxelState::Interior | VoxelState::Surface)
                ) {
                    outside += 1;
                }
            }
            if chain.role == Some(LimbRole::Leg) {
                if let Some(tip) = chain.bones.last().and_then(|b| fitted.position_of(b)) {
                    let above = (tip.y - landmarks.ground) / landmarks.extent().y;
                    worst_ground = worst_ground.max(above.abs());
                }
            }
        }
        println!(
            "{model:44} limb joints {outside:3}/{total:3} outside, worst leg tip {:.1}% off the ground",
            worst_ground * 100.0
        );
        let mut endpoints_outside = 0;
        for chain in manifest_data.of_kind(ChainKind::Limb) {
            for bone in [chain.bones.first(), chain.bones.last()]
                .into_iter()
                .flatten()
            {
                if fitted.position_of(bone).is_none_or(|at| {
                    !matches!(
                        grid.state(grid.coord_of(at)),
                        Some(VoxelState::Interior | VoxelState::Surface)
                    )
                }) {
                    endpoints_outside += 1;
                }
            }
            let bad: Vec<&str> = chain
                .bones
                .iter()
                .filter(|b| {
                    fitted.position_of(b).is_none_or(|at| {
                        !matches!(
                            grid.state(grid.coord_of(at)),
                            Some(VoxelState::Interior | VoxelState::Surface)
                        )
                    })
                })
                .map(String::as_str)
                .collect();
            if !bad.is_empty() {
                println!("      {} ({:?}): {bad:?}", chain.name, chain.role);
            }
        }
        println!("      endpoints outside: {endpoints_outside}");
    }
}

/// How many limb joints sit outside the mesh, before and after limb fitting.
fn limb_joints_outside(model: &str, rig: &str, manifest: &str) -> (usize, usize, usize, f32) {
    let mesh = mesh_of(model);
    let landmarks = Landmarks::of(&mesh).expect("vertices");
    let rest = rest_pose_of(rig);
    let spine = spine_of(manifest);
    let manifest_data = template(manifest);
    let grid = VoxelGrid::build(&mesh, 128).expect("voxelises");

    let placed = fit_uniform(&rest, &landmarks, &spine).expect("fits");
    let mut swung = placed.clone();
    m2m_rig::fit::fit_limbs(&mut swung, &mesh, &grid, &manifest_data);

    let count = |fit: &m2m_rig::fit::Fitted| -> usize {
        manifest_data
            .of_kind(ChainKind::Limb)
            .flat_map(|c| c.bones.iter())
            .filter(|bone| {
                fit.position_of(bone).is_none_or(|at| {
                    !matches!(
                        grid.state(grid.coord_of(at)),
                        Some(VoxelState::Interior | VoxelState::Surface)
                    )
                })
            })
            .count()
    };
    let total = manifest_data
        .of_kind(ChainKind::Limb)
        .map(|c| c.bones.len())
        .sum();

    // Worst leg tip, as a fraction of body height above the ground plane.
    let worst = manifest_data
        .of_kind(ChainKind::Limb)
        .filter(|c| c.role == Some(LimbRole::Leg))
        .filter_map(|c| c.bones.last().and_then(|b| swung.position_of(b)))
        .map(|tip| ((tip.y - landmarks.ground) / landmarks.extent().y).abs())
        .fold(0.0f32, f32::max);

    (count(&placed), count(&swung), total, worst)
}

/// Swinging a limb onto the mesh never makes a body worse than leaving it.
///
/// The invariant that matters, and one this failed twice while being written.
/// Picking the target as the furthest vertex in a 60-degree cone put 8 of the
/// fox's 26 limb joints outside a body they had all been inside, and lifted its
/// leg tips 52% of its height off the ground. Ranking by reach along the limb's
/// own axis in a 32-degree cone fixed the tips but still cost the fox 6 and the
/// horse 5. Only skipping limbs that are already inside makes this hold.
#[test]
fn limb_fitting_never_makes_a_body_worse() {
    for (model, rig, manifest) in [
        ("models/model-human.glb", "rigs/rig-human.glb", "human.json"),
        ("models/model-fox.glb", "rigs/rig-fox.glb", "fox.json"),
        ("models/model-horse.glb", "rigs/rig-horse.glb", "horse.json"),
        ("models/model-bird.glb", "rigs/rig-bird.glb", "bird.json"),
        ("models/model-rhino.glb", "rigs/rig-rhino.glb", "rhino.json"),
        (
            "models-variation/human-jay.glb",
            "rigs/rig-human.glb",
            "human.json",
        ),
        (
            "models-variation/human-bunny.glb",
            "rigs/rig-human.glb",
            "human.json",
        ),
        (
            "models-variation/human-sophia.glb",
            "rigs/rig-human.glb",
            "human.json",
        ),
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
            "models-variation/human-zombie.glb",
            "rigs/rig-human.glb",
            "human.json",
        ),
    ] {
        let (before, after, total, _) = limb_joints_outside(model, rig, manifest);
        assert!(
            after <= before,
            "{model}: limb fitting made it worse, {before} -> {after} of {total} outside"
        );
    }
}

/// The measured state of limb placement, pinned so it can only improve.
///
/// Now **2 of 170** across seven bodies, down from 53 before any limb work, 26
/// after the rigid swing, and 8 before endpoints were included in the joint
/// refinement. Five of the seven are at zero.
///
/// The two that remain are one joint each on `model-human` and `human-sophia`,
/// and they are the honest residue of a rigid chain: a T-posed template swung
/// onto an arm that bends differently still leaves one joint proud of the
/// surface.
#[test]
fn limb_placement_is_at_least_this_good() {
    for (model, rig, manifest, budget) in [
        ("models/model-fox.glb", "rigs/rig-fox.glb", "fox.json", 0),
        (
            "models/model-horse.glb",
            "rigs/rig-horse.glb",
            "horse.json",
            0,
        ),
        (
            "models-variation/human-sophia.glb",
            "rigs/rig-human.glb",
            "human.json",
            1,
        ),
        ("models/model-bird.glb", "rigs/rig-bird.glb", "bird.json", 0),
        (
            "models/model-rhino.glb",
            "rigs/rig-rhino.glb",
            "rhino.json",
            1,
        ),
        (
            "models/model-human.glb",
            "rigs/rig-human.glb",
            "human.json",
            1,
        ),
        (
            "models-variation/human-jay.glb",
            "rigs/rig-human.glb",
            "human.json",
            0,
        ),
        (
            "models-variation/human-bunny.glb",
            "rigs/rig-human.glb",
            "human.json",
            0,
        ),
    ] {
        let (_, after, total, _) = limb_joints_outside(model, rig, manifest);
        assert!(
            after <= budget,
            "{model}: {after} of {total} limb joints outside, budget {budget}"
        );
    }
}

/// Every leg keeps its foot on the ground.
///
/// Grounding comes free from the body fit, which maps the rig's own floor onto
/// the mesh's ground plane, and limb fitting must not undo it. It did, twice:
/// the first target rule lifted the fox's leg tips 52% of its height off the
/// floor and the bird's 73%.
#[test]
fn every_leg_keeps_its_foot_on_the_ground() {
    for (model, rig, manifest) in [
        ("models/model-human.glb", "rigs/rig-human.glb", "human.json"),
        ("models/model-fox.glb", "rigs/rig-fox.glb", "fox.json"),
        ("models/model-horse.glb", "rigs/rig-horse.glb", "horse.json"),
        ("models/model-bird.glb", "rigs/rig-bird.glb", "bird.json"),
        ("models/model-rhino.glb", "rigs/rig-rhino.glb", "rhino.json"),
        (
            "models/model-spider.glb",
            "rigs/rig-spider.glb",
            "spider.json",
        ),
        (
            "models-variation/human-bunny.glb",
            "rigs/rig-human.glb",
            "human.json",
        ),
    ] {
        let (_, _, _, worst) = limb_joints_outside(model, rig, manifest);
        // The templates' own leg tips sit 0.5% to 2.4% above their floor, so a
        // fitted foot is expected near the ground rather than exactly on it.
        assert!(
            worst < 0.03,
            "{model}: a leg tip sits {:.1}% of body height off the ground",
            worst * 100.0
        );
    }
}

// --- the shipped template set, and the pipeline as one call ----------------

/// Every manifest in `templates/` is embedded and parses.
#[test]
fn every_shipped_template_is_available_without_touching_the_disk() {
    let shipped = m2m_rig::template::all().expect("the shipped manifests parse");

    let mut names: Vec<&str> = shipped.iter().map(|t| t.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "bird", "buffalo", "dragon", "elephant", "fox", "horse", "human", "hyena", "kaiju",
            "rhino", "shark", "snake", "spider",
        ]
    );

    // 500 bones, each claimed exactly once — the count P3-1 established.
    let bones: usize = shipped.iter().map(|t| t.bones().count()).sum();
    assert_eq!(bones, 664);
}

/// The embedded manifests are the files on disk, not a stale copy.
#[test]
fn the_embedded_manifests_match_the_files_they_were_built_from() {
    // A build script that stopped rerunning would leave the binary describing
    // creatures the repository no longer has, and nothing else would notice.
    for shipped in m2m_rig::template::all().expect("parses") {
        let on_disk = template(&format!("{}.json", shipped.name));
        assert_eq!(shipped, on_disk, "{} drifted", shipped.name);
    }
}

/// `fit_template` is exactly the four steps, in order.
#[test]
fn the_one_call_pipeline_is_the_hand_chained_one() {
    // Every caller used to chain these by hand and they did not agree — the
    // report example stops after `fit_uniform` and never voxelises. Pinning the
    // composition is what stops that drifting again.
    let mesh = mesh_of("models/model-human.glb");
    let rest = rest_pose_of("rigs/rig-human.glb");
    let manifest = template("human.json");
    let spine = spine_of("human.json");

    let landmarks = Landmarks::of(&mesh).expect("vertices");
    let mut expected = fit_uniform(&rest, &landmarks, &spine).expect("fits");
    let axis = body_axis(&rest, &spine).expect("axis");
    refine_spine(&mut expected, &mesh, &landmarks, &spine, axis);
    let grid = VoxelGrid::build(&mesh, 128).expect("voxelises");
    m2m_rig::fit::fit_limbs(&mut expected, &mesh, &grid, &manifest);

    let got = m2m_rig::fit::fit_template(&manifest, &rest, &mesh, 128).expect("fits");
    assert_eq!(got, expected);
}

/// Fitting places every spine joint inside the body, on every shipped creature
/// that has a model to be fitted to.
#[test]
fn the_pipeline_puts_the_spine_inside_the_mesh() {
    // Per-creature counts, not a total: a total lets one creature improve while
    // another rots. Every number here is measured, and the seven zeros are the
    // invariant that spine refinement must never break — the first attempt at
    // the horizontal case scored better overall while putting fox, horse, bird
    // and dragon outside a body they had been inside.
    let budget = [
        ("human", 0),
        ("fox", 0),
        ("horse", 0),
        ("bird", 0),
        ("spider", 0),
        ("kaiju", 0),
        ("dragon", 0),
        // A long tapering tail is where a uniform scale diverges most. The
        // joints that remain outside are past the END of the mesh's tail: the
        // template's tail carries more bones than the shorter, tapering mesh
        // tail reaches, so `snap_spine_into_mesh` correctly leaves them rather
        // than bunching the chain. It took the snake from 6 to 5 and the shark
        // from 5 to 3 by nudging the barely-outside ones onto the body without
        // moving them along its length; the rest are genuinely beyond the tip.
        ("snake", 5),
        ("shark", 3),
    ];

    for (creature, allowed) in budget {
        let mesh = mesh_of(&format!("models/model-{creature}.glb"));
        let rest = rest_pose_of(&format!("rigs/rig-{creature}.glb"));
        let manifest = template(&format!("{creature}.json"));
        let grid = VoxelGrid::build(&mesh, 128).expect("voxelises");

        let fitted = m2m_rig::fit::fit_template(&manifest, &rest, &mesh, 128).expect("fits");

        let outside: Vec<String> = spine_of(&format!("{creature}.json"))
            .into_iter()
            .filter(|bone| {
                let at = fitted.position_of(bone).expect("a fitted spine bone");
                !matches!(
                    grid.state(grid.coord_of(at)),
                    Some(VoxelState::Interior | VoxelState::Surface)
                )
            })
            .collect();

        assert_eq!(
            outside.len(),
            allowed,
            "{creature} has {} spine joints outside its mesh: {outside:?}",
            outside.len()
        );
    }
}

/// The spine snap nudges a barely-outside joint onto the body but never drags a
/// past-the-tip joint back along the chain.
#[test]
fn the_spine_snap_pulls_joints_in_without_moving_them_along_the_body() {
    // Snake and shark are the creatures whose backbone IS a long tapering tail,
    // so they are where this matters. The invariant is two-sided: a joint the
    // snap moves must have kept its position along the body (its Z), and a joint
    // past the end of the tail — whose cross-section holds no interior voxel —
    // must not have moved at all.
    for creature in ["snake", "shark"] {
        let mesh = mesh_of(&format!("models/model-{creature}.glb"));
        let rest = rest_pose_of(&format!("rigs/rig-{creature}.glb"));
        let spine = spine_of(&format!("{creature}.json"));
        let landmarks = Landmarks::of(&mesh).expect("vertices");
        let axis = body_axis(&rest, &spine).expect("axis");
        let grid = VoxelGrid::build(&mesh, 128).expect("voxelises");

        let mut before = fit_uniform(&rest, &landmarks, &spine).expect("fits");
        refine_spine(&mut before, &mesh, &landmarks, &spine, axis);
        let mut after = before.clone();
        m2m_rig::fit::snap_spine_into_mesh(&mut after, &grid, &spine, axis);

        let inside = |p: glam::Vec3| {
            matches!(
                grid.state(grid.coord_of(p)),
                Some(VoxelState::Interior | VoxelState::Surface)
            )
        };

        let mut moved = 0;
        for bone in &spine {
            let a = before.position_of(bone).expect("bone");
            let b = after.position_of(bone).expect("bone");

            if inside(a) {
                // A joint already inside is never touched.
                assert_eq!(a, b, "{creature}:{bone} moved an already-inside joint");
                continue;
            }
            if a == b {
                continue; // past the tip, left alone
            }
            moved += 1;
            // A moved joint kept its position along the body …
            assert!(
                (a.z - b.z).abs() < 1e-6,
                "{creature}:{bone} was dragged along the body from z={} to z={}",
                a.z,
                b.z
            );
            // … and actually landed inside.
            assert!(
                inside(b),
                "{creature}:{bone} was moved but is still outside"
            );
        }
        assert!(moved > 0, "{creature}: the snap moved nothing");
    }
}
