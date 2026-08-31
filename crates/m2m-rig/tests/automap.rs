//! Matching an incoming rig to a template without using bone names.
//!
//! The test that matters is [`a_rig_with_no_meaningful_names_still_maps`]: it
//! takes our own human rig, renames every bone to `Bone.NNN`, and requires the
//! mapping to come back correct. That is the giraffe supplied as reference,
//! reproduced with an asset we own so CI can run it.
//!
//! Measured against the legacy resolver for comparison: a named humanoid
//! resolves 7 of 7 bones to a slot, and the same shape with bones called
//! `Bone.000` resolves **0 of 17**.

use glam::{Mat4, Quat, Vec3};
use m2m_rig::automap::{map_bones, match_chains, signature_of, Skeleton};
use m2m_rig::template::{ChainKind, Template};

fn asset(relative: &str) -> Vec<u8> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../legacy/static/").to_owned() + relative;
    std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"))
}

fn template(manifest: &str) -> Template {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/templates/").to_owned() + manifest;
    serde_json::from_str(&std::fs::read_to_string(&path).expect("reads")).expect("parses")
}

/// Reads a rig's skeleton: bone names, parents and world rest positions.
fn skeleton_of(relative: &str) -> Skeleton {
    let bytes = asset(relative);
    let document = m2m_io::glb::read(&bytes).expect("reads");
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
    for (index, slot) in world.iter_mut().enumerate() {
        let mut chain = vec![index];
        let mut cursor = index;
        while let Some(parent) = document.nodes[cursor].parent {
            chain.push(parent);
            cursor = parent;
        }
        let mut matrix = Mat4::IDENTITY;
        for &node in chain.iter().rev() {
            matrix *= local[node];
        }
        *slot = matrix;
    }

    let skin = document.skins.first().expect("a skin");
    let joints: std::collections::HashMap<usize, usize> = skin
        .joints
        .iter()
        .enumerate()
        .map(|(slot, &node)| (node, slot))
        .collect();
    Skeleton {
        names: skin
            .joints
            .iter()
            .map(|&j| document.nodes[j].name.clone())
            .collect(),
        parents: skin
            .joints
            .iter()
            .map(|&j| {
                document.nodes[j]
                    .parent
                    .and_then(|p| joints.get(&p).copied())
            })
            .collect(),
        positions: skin
            .joints
            .iter()
            .map(|&j| world[j].transform_point3(Vec3::ZERO))
            .collect(),
    }
}

/// Replaces every bone name with `Bone.NNN`, as a real export can.
fn strip_names(skeleton: &Skeleton) -> Skeleton {
    Skeleton {
        names: (0..skeleton.names.len())
            .map(|i| format!("Bone.{i:03}"))
            .collect(),
        parents: skeleton.parents.clone(),
        positions: skeleton.positions.clone(),
    }
}

/// The decomposition matches the one `tools/glb-chains.py` produces.
#[test]
fn a_skeleton_splits_into_the_chains_it_has() {
    let human = skeleton_of("rigs/rig-human.glb");
    let chains = human.chains();
    let covered: usize = chains.iter().map(Vec::len).sum();
    assert_eq!(
        covered,
        human.names.len(),
        "every bone belongs to one chain"
    );

    // 17: root+pelvis, spine, neck+head, two arms, ten fingers, two legs.
    // Note `pelvis` heads no chain of its own -- it has three children, so it
    // *ends* the chain that starts at `root`. That is why the template's own
    // "spine" chain (pelvis..spine_03) is not a maximal run, and why matching
    // is done maximal-run to maximal-run.
    assert_eq!(chains.len(), 17, "{} chains", chains.len());
    assert!(
        chains.iter().all(|c| !c.is_empty()),
        "a chain with no bones is not a chain"
    );
}

/// Structure alone recovers the mapping when every name is meaningless.
///
/// **This is the giraffe case**, reproduced with an asset we own so CI can run
/// it: take our own human rig, rename every bone to `Bone.NNN`, and require the
/// mapping to come back as the identity. The legacy resolves 0 of 17 bones on a
/// rig like this, because its slots are parsed from names.
#[test]
fn a_rig_with_no_meaningful_names_still_maps() {
    for rig in [
        "rigs/rig-human.glb",
        "rigs/rig-fox.glb",
        "rigs/rig-bird.glb",
        "rigs/rig-horse.glb",
        "rigs/rig-spider.glb",
        "rigs/rig-shark.glb",
        "rigs/rig-snake.glb",
    ] {
        let reference = skeleton_of(rig);
        let mapping = map_bones(&reference, &strip_names(&reference));

        assert_eq!(
            mapping.len(),
            reference.names.len(),
            "{rig}: {} of {} bones mapped",
            mapping.len(),
            reference.names.len()
        );
        let wrong: Vec<&str> = mapping
            .iter()
            .filter(|(from, to)| from != to)
            .map(|(from, _)| reference.names[*from].as_str())
            .collect();
        assert!(
            wrong.is_empty(),
            "{rig}: {} bones mapped to the wrong bone: {:?}",
            wrong.len(),
            &wrong[..wrong.len().min(8)]
        );
    }
}

/// Left maps to left and right to right, from geometry rather than a suffix.
///
/// A rig that labels its left arm `_r` cannot mislead this, because the labels
/// are never read.
#[test]
fn sides_are_taken_from_the_body_not_the_name() {
    let reference = skeleton_of("rigs/rig-human.glb");
    let anonymous = strip_names(&reference);
    let matches = match_chains(&template("human.json"), &reference, &anonymous);

    for chain in ["arm_l", "leg_l", "arm_r", "leg_r"] {
        let matched = matches
            .iter()
            .find(|m| m.template_chain == chain)
            .unwrap_or_else(|| panic!("{chain} did not match"));
        let x = matched
            .bones
            .iter()
            .map(|&i| anonymous.positions[i].x)
            .sum::<f32>()
            / matched.bones.len() as f32;
        if chain.ends_with("_l") {
            assert!(x > 0.0, "{chain} matched a chain at x {x}");
        } else {
            assert!(x < 0.0, "{chain} matched a chain at x {x}");
        }
    }
}

/// No incoming chain is handed to two template chains.
#[test]
fn a_chain_is_claimed_at_most_once() {
    let reference = skeleton_of("rigs/rig-human.glb");
    let matches = match_chains(
        &template("human.json"),
        &reference,
        &strip_names(&reference),
    );

    let mut seen = std::collections::HashSet::new();
    for matched in &matches {
        for &bone in &matched.bones {
            assert!(
                seen.insert(bone),
                "bone {bone} was claimed twice, second by {}",
                matched.template_chain
            );
        }
    }
}

/// A one-bone chain borrows its direction from its parent rather than reporting
/// none, so stubs are still told apart.
///
/// `rig-human` has none, so this builds one: three bones where the last has two
/// children, leaving each child a chain of one.
#[test]
fn a_single_bone_chain_still_has_a_direction() {
    let skeleton = Skeleton {
        names: vec!["a".into(), "b".into(), "c".into(), "d".into()],
        parents: vec![None, Some(0), Some(1), Some(1)],
        positions: vec![
            Vec3::ZERO,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.5, 1.5, 0.0),
            Vec3::new(-0.5, 1.5, 0.0),
        ],
    };
    let stubs: Vec<Vec<usize>> = skeleton
        .chains()
        .into_iter()
        .filter(|c| c.len() == 1)
        .collect();
    assert_eq!(stubs.len(), 2, "two children means two stubs");
    for stub in stubs {
        let signature = signature_of(&skeleton, &stub).expect("a signature");
        assert!(
            signature.direction.length() > 0.5,
            "a stub reported no direction: {:?}",
            signature.direction
        );
    }
}

/// Matching the fox template onto the fox rig recovers its four legs.
#[test]
fn a_quadruped_maps_its_four_legs() {
    let reference = skeleton_of("rigs/rig-fox.glb");
    let fox = template("fox.json");
    let matches = match_chains(&fox, &reference, &strip_names(&reference));

    let legs: Vec<&m2m_rig::automap::Match> = matches
        .iter()
        .filter(|m| m.kind == ChainKind::Limb)
        .collect();
    assert_eq!(legs.len(), 4, "a fox has four legs");
    for leg in legs {
        let expected = fox
            .chains
            .iter()
            .find(|c| c.name == leg.template_chain)
            .expect("the chain");
        let got: Vec<&str> = leg
            .bones
            .iter()
            .map(|&i| reference.names[i].as_str())
            .collect();
        assert_eq!(got, expected.bones, "{} mapped wrongly", leg.template_chain);
    }
}

/// Matching between two genuinely different rigs, not a skeleton to itself.
///
/// Identity recovery under renaming is the easy half: both sides have the same
/// shape. This maps our 66-bone human template rig onto a Mixamo rig
/// with different names, different proportions and a different bone count.
///
/// There is no ground-truth pairing to assert, so what is checked is what must
/// hold of *any* correct mapping: a left chain cannot map to a right one, and a
/// midline chain cannot map to a limb.
#[test]
fn mapping_between_two_different_rigs_keeps_sides_and_the_midline() {
    let ours = skeleton_of("rigs/rig-human.glb");
    let theirs = skeleton_of("test-files/retarget testing/mixamo-sample-rig.glb");
    assert_ne!(ours.names.len(), 0);
    assert_ne!(theirs.names.len(), 0);

    let mapping = map_bones(&ours, &theirs);
    assert!(
        mapping.len() * 2 > ours.names.len(),
        "only {} of {} bones mapped",
        mapping.len(),
        ours.names.len()
    );

    let side = |skeleton: &Skeleton, bone: usize| -> f32 {
        let xs: Vec<f32> = skeleton.positions.iter().map(|p| p.x).collect();
        let lo = xs.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let width = (hi - lo).max(f32::EPSILON);
        (skeleton.positions[bone].x - (lo + hi) * 0.5) / width
    };

    let mut crossed = Vec::new();
    for (&from, &to) in &mapping {
        let (a, b) = (side(&ours, from), side(&theirs, to));
        // Only judge bones clearly off the midline; a spine bone near zero may
        // land either side of it without being wrong.
        if a.abs() > 0.05 && b.abs() > 0.05 && a.signum() != b.signum() {
            crossed.push(ours.names[from].as_str());
        }
    }
    assert!(
        crossed.is_empty(),
        "{} bones crossed the midline: {:?}",
        crossed.len(),
        &crossed[..crossed.len().min(8)]
    );
}

/// Mapping a rig onto its own mirror image must swap left for right.
///
/// **Written because two mutations survived without it.** Zeroing the side term
/// in the distance, and zeroing the direction term, both passed every other test
/// here — because identity recovery is exact whatever the weights are: the
/// correct pairing scores zero on every feature, so any weighting still picks
/// it. A fixture that cannot distinguish a mutation proves nothing about it.
///
/// Mirroring in X moves each chain to the opposite side while leaving its shape
/// alone, so the only thing that can tell `arm_l` from `arm_r` is which side of
/// the body it is on. A side-blind matcher maps each arm to itself and fails.
#[test]
fn mapping_onto_a_mirrored_rig_swaps_the_sides() {
    let ours = skeleton_of("rigs/rig-human.glb");
    let mirrored = Skeleton {
        names: ours.names.clone(),
        parents: ours.parents.clone(),
        positions: ours
            .positions
            .iter()
            .map(|p| Vec3::new(-p.x, p.y, p.z))
            .collect(),
    };

    let mapping = map_bones(&ours, &mirrored);
    let index_of = |name: &str| ours.names.iter().position(|n| n == name);

    // Each of these must map to its opposite number, not to itself.
    for (left, right) in [
        ("upperarm_l", "upperarm_r"),
        ("lowerarm_l", "lowerarm_r"),
        ("hand_l", "hand_r"),
        ("thigh_l", "thigh_r"),
        ("calf_l", "calf_r"),
    ] {
        let (from, want) = (index_of(left).expect(left), index_of(right).expect(right));
        let got = mapping.get(&from).copied();
        assert_eq!(
            got,
            Some(want),
            "{left} should map to {right}, got {:?}",
            got.map(|g| ours.names[g].as_str())
        );
    }
}

/// Chains of different lengths pair end to end, not index to index.
///
/// **Written because a mutation survived**: pairing bone `i` with bone `i`
/// passed everything, because no fixture asserted *which* bone inside a chain a
/// bone maps to. A template's four-bone arm has to map onto an incoming
/// three-bone arm somehow, and the ends are the part that must line up — a
/// shoulder is a shoulder and a hand is a hand whatever lies between.
#[test]
fn chains_of_different_lengths_pair_end_to_end() {
    let long = Skeleton {
        names: (0..5).map(|i| format!("a{i}")).collect(),
        parents: vec![None, Some(0), Some(1), Some(2), Some(3)],
        positions: (0..5)
            .map(|i| Vec3::new(0.0, i as f32 * 0.25, 0.0))
            .collect(),
    };
    let short = Skeleton {
        names: (0..3).map(|i| format!("b{i}")).collect(),
        parents: vec![None, Some(0), Some(1)],
        positions: (0..3)
            .map(|i| Vec3::new(0.0, i as f32 * 0.5, 0.0))
            .collect(),
    };

    let mapping = map_bones(&long, &short);
    assert_eq!(
        mapping.get(&0),
        Some(&0),
        "the first bone maps to the first"
    );
    assert_eq!(mapping.get(&4), Some(&2), "the last maps to the last");
    // The middle is what actually separates proportional pairing from index
    // pairing: the ends agree either way, because an index past the end is
    // clamped to it. Bone 2 of 5 is halfway along, so it belongs on bone 1 of 3.
    assert_eq!(
        mapping.get(&2),
        Some(&1),
        "the middle bone should land halfway, not at its own index"
    );
    assert_eq!(mapping.len(), 5, "every bone of the longer chain is mapped");
}

/// Two chains alike in every way but direction are told apart by it.
///
/// **Written because a mutation survived**: zeroing the direction term passed
/// every other test. The real rigs do not isolate it — an arm and a leg differ
/// in side, height and reach as well — so this builds the case where direction
/// is the only difference: two chains of equal length, on the midline, leaving
/// the same joint, one up and one down.
#[test]
fn two_chains_differing_only_in_direction_are_told_apart() {
    // Both branches leave the same joint at the same height on the midline, so
    // side, reach and attachment height are identical between them and only the
    // direction they travel in can tell them apart. An earlier version of this
    // had one branch above the other, which attachment height alone separated.
    let make = |forward_first: bool| Skeleton {
        names: vec![
            "root".into(),
            "hub".into(),
            "a0".into(),
            "a1".into(),
            "b0".into(),
            "b1".into(),
        ],
        parents: vec![None, Some(0), Some(1), Some(2), Some(1), Some(4)],
        positions: {
            let (a, b) = if forward_first {
                (1.0, -1.0)
            } else {
                (-1.0, 1.0)
            };
            vec![
                Vec3::ZERO,
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 1.0, a * 0.5),
                Vec3::new(0.0, 1.0, a * 1.0),
                Vec3::new(0.0, 1.0, b * 0.5),
                Vec3::new(0.0, 1.0, b * 1.0),
            ]
        },
    };
    let reference = make(true);
    // The same skeleton with the branches swapped in space: what ran forward now
    // runs back. The forward chain must map to whichever chain now runs forward.
    let flipped = make(false);

    let mapping = map_bones(&reference, &flipped);
    assert_eq!(
        mapping.get(&2),
        Some(&4),
        "the forward chain should map to whichever chain now runs forward"
    );
    assert_eq!(mapping.get(&4), Some(&2), "and the backward one likewise");
}

fn known_rigs() -> Vec<m2m_rig::automap::KnownRig> {
    ["mixamo.json", "rigify.json"]
        .iter()
        .map(|file| {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/known-rigs/").to_owned() + file;
            serde_json::from_str(&std::fs::read_to_string(&path).expect("reads")).expect("parses")
        })
        .collect()
}

/// Names that differ only in punctuation or case are the same bone.
///
/// The sample rig writes `mixamorig:Hips` and the legacy's table writes
/// `mixamorigHips`. The separator is an exporter's habit, not information.
#[test]
fn bone_names_compare_without_punctuation_or_case() {
    use m2m_rig::automap::normalised_bone_name as n;
    assert_eq!(n("mixamorig:Hips"), n("mixamorigHips"));
    assert_eq!(n("mixamorig:Hips"), n("MIXAMORIG_HIPS"));
    assert_eq!(n("DEF-spine.001"), n("defspine001"));
    assert_ne!(n("spine_01"), n("spine_02"));
}

/// A Mixamo rig is recognised and mapped through its table.
#[test]
fn a_mixamo_rig_is_recognised_and_mapped_by_name() {
    let ours = skeleton_of("test-files/retarget testing/m2m-sample-rig.glb");
    let theirs = skeleton_of("test-files/retarget testing/mixamo-sample-rig.glb");
    let rigs = known_rigs();

    let mixamo = rigs.iter().find(|r| r.name == "mixamo").expect("the table");
    let coverage = mixamo.coverage(&theirs);
    assert!(coverage > 0.9, "mixamo table covers only {coverage:.2}");

    let (mapping, strategy) = m2m_rig::automap::map_bones_best(&ours, &theirs, &rigs, 0.5);
    assert_eq!(strategy, m2m_rig::automap::Strategy::Known("mixamo".into()));

    // The bones structural matching got wrong are exactly the fingers, so check
    // one of those: it must now be right.
    let from = ours
        .names
        .iter()
        .position(|n| n == "middle_01_l")
        .expect("bone");
    let to = mapping.get(&from).copied().expect("mapped");
    assert_eq!(
        m2m_rig::automap::normalised_bone_name(&theirs.names[to]),
        m2m_rig::automap::normalised_bone_name("mixamorigLeftHandMiddle1"),
        "got {}",
        theirs.names[to]
    );
}

/// A rig with Rigify's deform names is recognised as Rigify, not Mixamo.
#[test]
fn a_rigify_named_rig_is_recognised_as_rigify() {
    let ours = skeleton_of("test-files/retarget testing/m2m-sample-rig.glb");
    let theirs = skeleton_of("test-files/import custom animations/m2m-wrong-bone-names.glb");
    let rigs = known_rigs();

    // This fixture is named as a Rigify export: DEF-hips, DEF-spine.001, ...
    assert!(theirs.names.iter().any(|n| n.starts_with("DEF-")));

    let (_, strategy) = m2m_rig::automap::map_bones_best(&ours, &theirs, &rigs, 0.5);
    assert_eq!(strategy, m2m_rig::automap::Strategy::Known("rigify".into()));
}

/// A rig no table recognises falls back to structure rather than failing.
///
/// The giraffe case: `Bone.000`, `Bone.001`, ... A table matches none of it, and
/// the fallback still recovers the mapping.
#[test]
fn an_unrecognised_rig_falls_back_to_structure() {
    let ours = skeleton_of("rigs/rig-human.glb");
    let anonymous = strip_names(&ours);
    let rigs = known_rigs();

    for rig in &rigs {
        assert_eq!(
            rig.coverage(&anonymous),
            0.0,
            "{} matched nothing",
            rig.name
        );
    }
    let (mapping, strategy) = m2m_rig::automap::map_bones_best(&ours, &anonymous, &rigs, 0.5);
    assert_eq!(strategy, m2m_rig::automap::Strategy::Structural);
    assert_eq!(
        mapping.len(),
        ours.names.len(),
        "the fallback mapped everything"
    );
}

/// Where the two strategies disagree is the fingers, and only the fingers.
///
/// Pinned because it is the measured reason the tables exist. If structure ever
/// learns to order fingers, this test is how that will show up.
#[test]
fn structure_and_the_table_differ_only_on_fingers() {
    let ours = skeleton_of("test-files/retarget testing/m2m-sample-rig.glb");
    let theirs = skeleton_of("test-files/retarget testing/mixamo-sample-rig.glb");
    let rigs = known_rigs();
    let mixamo = rigs.iter().find(|r| r.name == "mixamo").expect("the table");

    let by_table = mixamo.map_bones(&ours, &theirs);
    let by_structure = map_bones(&ours, &theirs);

    let mut disagreements = Vec::new();
    for (from, table_to) in &by_table {
        if by_structure.get(from).is_some_and(|s| s != table_to) {
            disagreements.push(ours.names[*from].as_str());
        }
    }
    assert!(
        !disagreements.is_empty(),
        "they agreed everywhere, which is new"
    );
    let non_finger: Vec<&&str> = disagreements
        .iter()
        .filter(|name| {
            !["thumb", "index", "middle", "ring", "pinky"]
                .iter()
                .any(|finger| name.starts_with(finger))
        })
        .collect();
    assert!(
        non_finger.is_empty(),
        "structure and the table now differ outside the fingers: {non_finger:?}"
    );
}
