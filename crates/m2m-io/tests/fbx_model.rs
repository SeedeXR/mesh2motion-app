//! The Model hierarchy, checked node-for-node against the legacy loader.
//!
//! `legacy/bench/dump-model-fixtures.ts` runs the legacy's own `FBXLoader` over
//! the reference rig headless and records every Model's parent, local matrix
//! and world matrix. Asserting against that is what makes this a port rather
//! than a reimplementation that merely looks plausible: "the hips are about a
//! metre up" would pass with the transform pipeline subtly wrong.

use glam::DMat4;
use m2m_io::fbx::model::{self, ModelTree};
use m2m_io::fbx::{binary, dom::Scene};
use std::collections::HashMap;

const MIXAMO: &[u8] =
    include_bytes!("../../../assets/test-files/retarget testing/mixamo-original-rig.fbx");
const FIXTURE: &[u8] = include_bytes!("fixtures/fbx-models.bin");
const NAMES: &str = include_str!("fixtures/fbx-models-names.txt");

/// id, parentId, local[16], world[16].
const STRIDE: usize = 2 + 32;

struct Expected {
    parent: Option<i64>,
    local: DMat4,
    world: DMat4,
}

fn expected() -> HashMap<i64, Expected> {
    let count = u32::from_le_bytes(FIXTURE[0..4].try_into().expect("header")) as usize;
    let body = &FIXTURE[8..];
    assert_eq!(
        body.len(),
        count * STRIDE * 8,
        "fixture size disagrees with its header — regenerate it"
    );
    let f = |i: usize| f64::from_le_bytes(body[i * 8..i * 8 + 8].try_into().expect("f64"));

    (0..count)
        .map(|c| {
            let base = c * STRIDE;
            let m4 = |o: usize| {
                let mut a = [0.0f64; 16];
                for (k, slot) in a.iter_mut().enumerate() {
                    *slot = f(base + o + k);
                }
                DMat4::from_cols_array(&a)
            };
            let parent = f(base + 1) as i64;
            (
                f(base) as i64,
                Expected {
                    parent: (parent != -1).then_some(parent),
                    local: m4(2),
                    world: m4(18),
                },
            )
        })
        .collect()
}

fn tree() -> ModelTree {
    model::parse_all(&Scene::from_document(
        binary::parse(MIXAMO).expect("parses"),
    ))
}

fn deviation(a: DMat4, b: DMat4) -> f64 {
    a.to_cols_array()
        .iter()
        .zip(b.to_cols_array().iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f64::max)
}

#[test]
fn every_model_matches_the_legacy_hierarchy_and_matrices() {
    let tree = tree();
    let expected = expected();
    assert_eq!(tree.models.len(), 67, "models in the reference rig");
    assert_eq!(expected.len(), 67, "models in the fixture");

    // Names come from the same dump, so a mismatch here means the two sides
    // disagree about which bone an id IS, before any matrix is compared.
    let names: HashMap<i64, &str> = NAMES
        .lines()
        .filter_map(|l| l.split_once(' '))
        .map(|(id, name)| (id.parse().expect("id"), name))
        .collect();

    let mut worst_local = 0.0f64;
    let mut worst_world = 0.0f64;
    let mut worst_name = "";

    for m in &tree.models {
        let e = expected
            .get(&m.id)
            .unwrap_or_else(|| panic!("{} ({}) is not in the fixture", m.id, m.name));
        // three.js strips ':' from names for animation binding; the FBX and
        // this port keep it.
        let legacy_name = names.get(&m.id).copied().unwrap_or("?");
        assert_eq!(
            m.name.replace(':', ""),
            legacy_name,
            "id {} names disagree",
            m.id
        );
        assert_eq!(m.parent, e.parent, "{} parent", m.name);

        let dl = deviation(m.local, e.local);
        let dw = deviation(m.world, e.world);
        if dw > worst_world {
            worst_world = dw;
            worst_name = &m.name;
        }
        worst_local = worst_local.max(dl);

        // Measured worst case is 8.5e-14 local and 2.0e-13 world on a rig
        // that reaches ~165 units, i.e. agreement to about 1e-15 relative --
        // f64 rounding, not an approximation. The bound is set just loose
        // enough to absorb a different multiplication order, and no looser:
        // at 1e-6 a genuinely wrong millimetre would pass.
        assert!(dl < 1e-9, "{} local deviates by {dl}", m.name);
        assert!(dw < 1e-9, "{} world deviates by {dw}", m.name);
    }
    eprintln!("worst local {worst_local:e}, worst world {worst_world:e} ({worst_name})");
}

#[test]
fn the_hierarchy_is_a_single_rooted_skeleton_plus_its_meshes() {
    let tree = tree();

    // 65 bones and 2 meshes, measured; the meshes hang off no bone.
    let bones = tree.models.iter().filter(|m| m.is_bone()).count();
    assert_eq!(bones, 65, "LimbNode models");
    let meshes = tree.models.iter().filter(|m| m.subclass == "Mesh").count();
    assert_eq!(meshes, 2);

    // One skeleton root (the hips) and the two meshes sit at the top.
    assert_eq!(tree.roots.len(), 3, "roots: {:?}", tree.roots);
    let root_bones: Vec<&str> = tree
        .roots
        .iter()
        .filter_map(|id| tree.get(*id))
        .filter(|m| m.is_bone())
        .map(|m| m.name.as_str())
        .collect();
    assert_eq!(root_bones, vec!["mixamorig:Hips"]);

    // Every non-root bone reaches the hips, so the skeleton is connected --
    // a broken parent link would leave an orphan chain that still transforms
    // fine on its own and animates in the wrong place.
    let hips = tree
        .models
        .iter()
        .find(|m| m.name == "mixamorig:Hips")
        .expect("hips");
    for m in tree.models.iter().filter(|m| m.is_bone()) {
        let chain = tree.ancestors(m.id);
        assert_eq!(
            *chain.last().expect("non-empty"),
            hips.id,
            "{} does not reach the hips: {chain:?}",
            m.name
        );
    }
    assert_eq!(tree.report, model::ModelReport::default(), "clean file");
}

#[test]
fn world_positions_are_anatomically_plausible() {
    // The fixture comparison already pins these exactly. This is the
    // independent sanity check the loop asks for: numbers that are
    // self-consistent can still be self-consistently wrong, and a rig where
    // the head sits below the feet would satisfy every matrix assertion above
    // if the fixture were regenerated from a broken loader.
    let tree = tree();
    let pos = |name: &str| {
        tree.models
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("{name}"))
            .world
            .w_axis
            .truncate()
    };

    let hips = pos("mixamorig:Hips");
    let head = pos("mixamorig:Head");
    let foot = pos("mixamorig:LeftFoot");
    let l_hand = pos("mixamorig:LeftHand");
    let r_hand = pos("mixamorig:RightHand");

    // Mixamo exports centimetres; this is a ~165cm human.
    assert!((100.0..110.0).contains(&hips.y), "hip height {} cm", hips.y);
    assert!(head.y > hips.y, "head {} below hips {}", head.y, hips.y);
    assert!(foot.y < 15.0, "foot {} cm off the floor", foot.y);
    assert!(head.y - foot.y > 140.0, "stature {} cm", head.y - foot.y);

    // The hands are mirrored about the body, so they must sit on opposite
    // sides and at a similar height. A dropped PreRotation collapses one arm.
    assert!(
        l_hand.x * r_hand.x < 0.0,
        "hands on the same side: {} and {}",
        l_hand.x,
        r_hand.x
    );
    assert!(
        (l_hand.y - r_hand.y).abs() < 5.0,
        "hands at different heights: {} and {}",
        l_hand.y,
        r_hand.y
    );
    assert!(
        (l_hand.x.abs() - r_hand.x.abs()).abs() < 5.0,
        "arms asymmetric: {} and {}",
        l_hand.x,
        r_hand.x
    );
}

/// A document with the given Model bodies and connections.
///
/// Models are `LimbNode`s numbered by the caller; `body` is the inside of a
/// `Properties70` block.
fn synthetic(models: &[(i64, &str, &str)], connections: &str) -> Scene {
    let mut text = String::from("FBXVersion: 7400\nObjects:  {\n");
    for (id, name, props) in models {
        text.push_str(&format!(
            "\tModel: {id}, \"Model::{name}\", \"LimbNode\" {{\n\t\tProperties70:  {{\n{props}\t\t}}\n\t}}\n"
        ));
    }
    text.push_str("}\nConnections:  {\n");
    text.push_str(connections);
    text.push_str("}\n");
    Scene::from_document(m2m_io::fbx::text::parse(&text).expect("ascii parses"))
}

const TRANSLATE_10Y: &str =
    "\t\t\tP: \"Lcl Translation\", \"Lcl Translation\", \"\", \"A\",0,10,0\n";

#[test]
fn a_model_naming_two_parents_keeps_the_last_and_says_so() {
    // The reference rig has none, so nothing above can distinguish this. The
    // choice is arbitrary either way -- it is matched to three.js's
    // `Object3D.add`, which detaches the child from its previous parent, and
    // counted so a caller knows the file was ambiguous.
    let tree = model::parse_all(&synthetic(
        &[(10, "a", ""), (20, "b", ""), (30, "child", TRANSLATE_10Y)],
        "\tC: \"OO\",30,10\n\tC: \"OO\",30,20\n",
    ));

    assert_eq!(tree.report.multiple_parents, 1);
    assert_eq!(
        tree.get(30).expect("child").parent,
        Some(20),
        "the last one"
    );
    assert_eq!(tree.get(20).expect("b").children, vec![30]);
    assert!(tree.get(10).expect("a").children.is_empty());
}

#[test]
fn a_parent_cycle_is_cut_rather_than_followed_forever() {
    // A hostile or truncated file can close a loop in the connection graph.
    // Without cutting it these Models are unreachable from any root, so they
    // would silently keep the identity transform -- and `ancestors` would not
    // terminate.
    let tree = model::parse_all(&synthetic(
        &[
            (10, "a", TRANSLATE_10Y),
            (20, "b", TRANSLATE_10Y),
            (30, "c", TRANSLATE_10Y),
        ],
        // a -> b -> c -> a
        "\tC: \"OO\",10,20\n\tC: \"OO\",20,30\n\tC: \"OO\",30,10\n",
    ));

    assert_eq!(tree.report.cycles_broken, 1, "exactly one link cut");
    assert_eq!(
        tree.roots.len(),
        1,
        "the cut makes one root: {:?}",
        tree.roots
    );

    // Every Model is now reachable, and every walk terminates.
    for m in &tree.models {
        let chain = tree.ancestors(m.id);
        assert!(
            chain.len() <= tree.models.len(),
            "{} loops: {chain:?}",
            m.name
        );
        assert_eq!(*chain.last().expect("non-empty"), tree.roots[0]);
    }
    // And they were actually composed, not left at the identity: the chain
    // stacks three 10-unit translations.
    let deepest = tree
        .models
        .iter()
        .map(|m| m.world.w_axis.y)
        .fold(f64::MIN, f64::max);
    assert!((deepest - 30.0).abs() < 1e-9, "deepest world y = {deepest}");
}

#[test]
fn a_parent_collapsed_to_zero_scale_defaults_its_children_and_is_counted() {
    // glam returns NaN from `inverse()` on a singular matrix, so without this
    // the whole subtree becomes NaN with nothing to say where it began.
    let zero_scale = "\t\t\tP: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",0,0,0\n";
    let tree = model::parse_all(&synthetic(
        &[(10, "flat", zero_scale), (20, "child", TRANSLATE_10Y)],
        "\tC: \"OO\",20,10\n",
    ));

    assert_eq!(tree.report.transforms_defaulted, 1, "the child");
    let child = tree.get(20).expect("child");
    assert_eq!(child.local, DMat4::IDENTITY, "defaulted, not NaN");
    assert!(child.world.is_finite(), "world must not be NaN");
    // The parent itself composed fine — it is only the child that could not.
    assert!(tree.get(10).expect("flat").local.is_finite());
}

#[test]
fn inherit_type_is_read_from_the_file_and_changes_the_result() {
    // Every scale in the reference rig is 1, and the three inheritance modes
    // are identical under a uniformly scaled parent -- so no assertion against
    // that rig can tell whether InheritType is read at all. This is the only
    // place that distinction is visible.
    let non_uniform = "\t\t\tP: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",1,2,4\n";
    let child_with = |inherit: i64| {
        let props = format!(
            "{TRANSLATE_10Y}\t\t\tP: \"Lcl Rotation\", \"Lcl Rotation\", \"\", \"A\",10,20,30\n\t\t\tP: \"InheritType\", \"enum\", \"\", \"\",{inherit}\n"
        );
        let tree = model::parse_all(&synthetic(
            &[(10, "parent", non_uniform), (20, "child", &props)],
            "\tC: \"OO\",20,10\n",
        ));
        tree.get(20).expect("child").local
    };

    let (a, b, c) = (child_with(0), child_with(1), child_with(2));
    assert!(deviation(a, b) > 0.01, "0 and 1 agree: {}", deviation(a, b));
    assert!(deviation(a, c) > 0.01, "0 and 2 agree: {}", deviation(a, c));
    assert!(deviation(b, c) > 0.01, "1 and 2 agree: {}", deviation(b, c));

    // Absent means 0, not "some other default".
    let absent = model::parse_all(&synthetic(
        &[
            (10, "parent", non_uniform),
            (
                20,
                "child",
                &format!("{TRANSLATE_10Y}\t\t\tP: \"Lcl Rotation\", \"Lcl Rotation\", \"\", \"A\",10,20,30\n"),
            ),
        ],
        "\tC: \"OO\",20,10\n",
    ));
    assert!(deviation(absent.get(20).expect("child").local, a) < 1e-15);
}

#[test]
fn a_child_receives_its_parents_local_matrix_not_its_world() {
    // Only `InheritType::Rrs` reads the parent's local matrix, and it divides
    // by that matrix's scale. Passing the world matrix instead is invisible
    // unless the two differ in scale -- which needs a grandparent that is
    // itself scaled.
    let scaled = |s: &str| format!("\t\t\tP: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",{s}\n");
    let rrs = format!("{TRANSLATE_10Y}\t\t\tP: \"InheritType\", \"enum\", \"\", \"\",2\n");
    let tree = model::parse_all(&synthetic(
        &[
            (10, "grandparent", &scaled("5,5,5")),
            (20, "parent", &scaled("1,2,4")),
            (30, "child", &rrs),
        ],
        "\tC: \"OO\",20,10\n\tC: \"OO\",30,20\n",
    ));

    // The parent's world scale is (5,10,20) while its local scale is (1,2,4),
    // so Rrs -- which removes the parent's LOCAL scale only -- must leave the
    // grandparent's factor of 5 in place.
    let child = tree.get(30).expect("child");
    assert!(child.world.is_finite());
    let scale = glam::DVec3::new(
        child.world.x_axis.truncate().length(),
        child.world.y_axis.truncate().length(),
        child.world.z_axis.truncate().length(),
    );
    // Grandparent 5 on every axis, parent's own local scale divided out.
    assert!(
        (scale.x - 5.0).abs() < 1e-9 && (scale.y - 5.0).abs() < 1e-9 && (scale.z - 5.0).abs() < 1e-9,
        "child world scale {scale:?} — the parent's local scale should be gone and the grandparent's kept"
    );
}

#[test]
fn children_are_in_ascending_id_order_and_stable_across_parses() {
    // A child's position in this list becomes a bone index downstream, so the
    // order is part of the output. The connection graph has no inherent order
    // and the id map is a HashMap, so without the sort this would be free to
    // vary between runs -- producing a rig whose bones are numbered
    // differently each time it is loaded.
    let tree = tree();
    let mut with_siblings = 0;
    for m in &tree.models {
        let mut sorted = m.children.clone();
        sorted.sort_unstable();
        assert_eq!(m.children, sorted, "{} children out of order", m.name);
        if m.children.len() > 1 {
            with_siblings += 1;
        }
    }
    // The assertion above is vacuous for a node with fewer than two children,
    // and a chain of single children would satisfy it trivially.
    assert_eq!(
        with_siblings, 4,
        "measured: Hips (3), Spine2 (3), and each hand (5 fingers) — if this rig \
         ever stops branching, the ordering assertion above becomes vacuous"
    );

    let again = model::parse_all(&Scene::from_document(
        binary::parse(MIXAMO).expect("parses"),
    ));
    assert_eq!(
        again
            .models
            .iter()
            .map(|m| (m.id, m.children.clone()))
            .collect::<Vec<_>>(),
        tree.models
            .iter()
            .map(|m| (m.id, m.children.clone()))
            .collect::<Vec<_>>(),
        "two parses of the same bytes must produce the same tree"
    );
}

#[test]
fn malformed_documents_produce_a_tree_rather_than_a_panic_or_a_lost_node() {
    // The trust boundary: these all arrive as bytes from a file. None may
    // panic, and — the quieter failure — none may leave a Model unreachable
    // from any root, because an unreachable node is never composed and keeps
    // the identity transform while looking like a perfectly ordinary bone.
    let cases: Vec<(&str, Scene)> = vec![
        ("no models at all", synthetic(&[], "")),
        (
            "one model, no connections",
            synthetic(&[(10, "lonely", TRANSLATE_10Y)], ""),
        ),
        (
            "a model parented to itself",
            synthetic(&[(10, "ouroboros", TRANSLATE_10Y)], "\tC: \"OO\",10,10\n"),
        ),
        (
            "a connection to an id that is not a Model",
            synthetic(&[(10, "a", TRANSLATE_10Y)], "\tC: \"OO\",10,999\n"),
        ),
        (
            "two separate cycles",
            synthetic(
                &[(10, "a", ""), (20, "b", ""), (30, "c", ""), (40, "d", "")],
                "\tC: \"OO\",10,20\n\tC: \"OO\",20,10\n\tC: \"OO\",30,40\n\tC: \"OO\",40,30\n",
            ),
        ),
        (
            "a duplicate connection between the same pair",
            synthetic(
                &[(10, "a", ""), (20, "b", TRANSLATE_10Y)],
                "\tC: \"OO\",20,10\n\tC: \"OO\",20,10\n",
            ),
        ),
    ];

    for (what, scene) in cases {
        let tree = model::parse_all(&scene);

        for m in &tree.models {
            // Terminates, and ends at a root.
            let chain = tree.ancestors(m.id);
            assert!(
                chain.len() <= tree.models.len(),
                "{what}: {} has a chain longer than the tree: {chain:?}",
                m.name
            );
            let last = *chain.last().expect("a node is its own first ancestor");
            assert!(
                tree.roots.contains(&last),
                "{what}: {} climbs to {last}, which is not a root",
                m.name
            );
            // Composed, not skipped: an unreachable node would still be
            // sitting at the identity it was constructed with.
            assert!(
                m.local.is_finite() && m.world.is_finite(),
                "{what}: {} is NaN",
                m.name
            );
        }
        assert_eq!(
            tree.roots.is_empty(),
            tree.models.is_empty(),
            "{what}: a non-empty tree must have at least one root"
        );
    }
}

#[test]
fn ancestors_terminates_even_on_a_tree_whose_parents_were_rewritten() {
    // `parse_all` cuts every cycle, so no tree it returns can loop — which
    // means nothing above can reach this guard. But `models` is a public
    // field: a caller that re-parents nodes (a retarget step reordering a
    // skeleton, say) can put a loop back. An unbounded walk would hang there,
    // and a hang leaves nothing to debug, unlike a wrong answer.
    let mut tree = model::parse_all(&synthetic(
        &[(10, "a", ""), (20, "b", "")],
        "\tC: \"OO\",20,10\n",
    ));
    assert_eq!(tree.report.cycles_broken, 0, "this document is acyclic");

    // Close the loop behind parse_all's back.
    tree.models[0].parent = Some(20);

    let chain = tree.ancestors(10);
    assert!(
        chain.len() <= tree.models.len() + 1,
        "ancestors did not stop: {chain:?}"
    );
}

#[test]
fn a_root_subclass_model_counts_as_a_bone() {
    // The whole reference corpus is `LimbNode`, so the fixture comparison
    // cannot see this. 3ds Max Biped exports name the skeleton root `Root`,
    // and the legacy builds a `Bone` for both subclasses.
    let mut text = String::from("FBXVersion: 7400\nObjects:  {\n");
    text.push_str("\tModel: 10, \"Model::skeleton_root\", \"Root\" {\n\t}\n");
    text.push_str("\tModel: 20, \"Model::limb\", \"LimbNode\" {\n\t}\n");
    text.push_str("\tModel: 30, \"Model::a_mesh\", \"Mesh\" {\n\t}\n");
    text.push_str("\tModel: 40, \"Model::a_null\", \"Null\" {\n\t}\n");
    text.push_str("}\nConnections:  {\n\tC: \"OO\",20,10\n}\n");
    let tree = model::parse_all(&Scene::from_document(
        m2m_io::fbx::text::parse(&text).expect("ascii parses"),
    ));

    let bone = |name: &str| {
        tree.models
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("{name}"))
            .is_bone()
    };
    assert!(bone("skeleton_root"), "a Root subclass is a bone");
    assert!(bone("limb"));
    assert!(!bone("a_mesh"), "a Mesh is not");
    assert!(!bone("a_null"), "a Null is not");
}

#[test]
fn a_local_matrix_keeps_the_shear_the_legacy_would_have_discarded() {
    // A deliberate divergence, recorded because no fixture can catch it.
    //
    // The legacy stores this matrix on an `Object3D` via `applyMatrix4` and
    // then `updateWorldMatrix`, which recomposes it from position/quaternion/
    // scale -- a representation with no room for shear. Measured directly
    // against three.js r185: a sheared matrix put through that round trip
    // comes back changed by 0.39.
    //
    // FBX genuinely produces shear here: with a non-uniformly scaled ancestor,
    // the local matrix contains GSM⁻¹·R·GSM, which is not orthogonal. A
    // `DMat4` can hold that and an `Object3D` cannot, so this port keeps the
    // information the reference implementation had to throw away.
    //
    // Consequence for the fixture test: it agrees to 8.5e-14 only because
    // every scale in the reference rig is 1.0, so no shear arises. Regenerate
    // the fixtures from a rig with a non-uniformly scaled ancestor and
    // `every_model_matches_the_legacy_hierarchy_and_matrices` will fail --
    // correctly, and this comment is the explanation.
    let tree = model::parse_all(&synthetic(
        &[
            (
                10,
                "parent",
                "\t\t\tP: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",1,2,4\n",
            ),
            (
                20,
                "child",
                "\t\t\tP: \"Lcl Rotation\", \"Lcl Rotation\", \"\", \"A\",0,0,40\n",
            ),
        ],
        "\tC: \"OO\",20,10\n",
    ));

    let local = tree.get(20).expect("child").local;
    // Shear shows up as basis columns that are no longer perpendicular.
    let x = local.x_axis.truncate().normalize();
    let y = local.y_axis.truncate().normalize();
    let skew = x.dot(y).abs();
    assert!(
        skew > 0.01,
        "expected a sheared local basis, got columns {skew} from perpendicular — \
         if this is now ~0, the raw product is being recomposed somewhere"
    );
    assert!(local.is_finite());
}
