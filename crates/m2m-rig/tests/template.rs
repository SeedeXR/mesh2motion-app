//! The template format, checked against the skeletons it describes.
//!
//! The important test is `the_human_template_describes_its_skeleton_exactly`:
//! it reads the real `.glb` and requires the manifest to account for every bone
//! in it. A format tested only against hand-written fixtures drifts away from
//! the files it exists to describe.

use m2m_rig::template::{
    Chain, ChainKind, LimbRole, Posture, Side, Skeleton, Template, TemplateProblem,
};

/// Reads a template skeleton's bones and their parents out of its `.glb`.
///
/// Only skin joints count as bones. The `Armature` node holding the root is
/// part of the scene graph, not the skeleton, so the root bone's parent is
/// reported as `None` rather than as a bone that does not exist.
fn skeleton_of(relative: &str) -> (Vec<(String, Option<String>)>, usize) {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/rigs/").to_owned() + relative;
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    let document = m2m_io::glb::read(&bytes).expect("the template reads");
    let skin = document.skins.first().expect("the template has a skin");

    let joints: std::collections::HashSet<usize> = skin.joints.iter().copied().collect();
    let bones: Vec<(String, Option<String>)> = skin
        .joints
        .iter()
        .map(|&j| {
            let parent = document.nodes[j]
                .parent
                .filter(|p| joints.contains(p))
                .map(|p| document.nodes[p].name.clone());
            (document.nodes[j].name.clone(), parent)
        })
        .collect();
    let count = skin.joints.len();
    (bones, count)
}

fn load(manifest: &str) -> Template {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/templates/").to_owned() + manifest;
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parsing {path}: {e}"))
}

fn borrow(bones: &[(String, Option<String>)]) -> Skeleton<'_> {
    Skeleton::new(
        bones
            .iter()
            .map(|(bone, parent)| (bone.as_str(), parent.as_deref())),
    )
}

/// Requires a manifest to account for every bone of a real skeleton, with every
/// chain a real parent-to-child run.
fn assert_template_matches(manifest: &str, skeleton: &str, expected_joints: usize) {
    let (bones, joints) = skeleton_of(skeleton);
    assert_eq!(joints, expected_joints, "{skeleton} changed");

    let template = load(manifest);
    let problems = template.check(&borrow(&bones));
    assert!(
        problems.is_empty(),
        "{manifest} does not describe {skeleton}:\n{}",
        problems
            .iter()
            .map(|p| format!("  - {p}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert_eq!(
        template.bones().count(),
        expected_joints,
        "every bone exactly once"
    );
}

/// The manifest accounts for every bone of the real skeleton, and every chain
/// is a real parent-to-child run.
#[test]
fn the_human_template_describes_its_skeleton_exactly() {
    assert_template_matches("human.json", "rig-human.glb", 66);
}

/// The chains are the ones a human actually has, not merely *a* set that adds
/// up: putting all 66 bones in one chain would satisfy the check above.
#[test]
fn the_human_template_has_the_chains_a_human_has() {
    let template = load("human.json");
    assert_eq!(template.of_kind(ChainKind::Root).count(), 1);
    assert_eq!(template.of_kind(ChainKind::Spine).count(), 1);
    assert_eq!(template.of_kind(ChainKind::Neck).count(), 1);
    assert_eq!(template.of_kind(ChainKind::Head).count(), 1);
    assert_eq!(
        template.of_kind(ChainKind::Limb).count(),
        4,
        "two arms, two legs"
    );
    assert_eq!(
        template.of_kind(ChainKind::Digit).count(),
        10,
        "five fingers a hand"
    );
    assert_eq!(
        template.of_kind(ChainKind::Jaw).count(),
        0,
        "no jaw in this rig"
    );
    assert_eq!(template.of_kind(ChainKind::Tail).count(), 0);

    // Sides are paired, and legs record how they meet the ground -- the fox rig
    // will differ there, which is the reason to record it at all.
    let limbs: Vec<&Chain> = template.of_kind(ChainKind::Limb).collect();
    assert_eq!(
        limbs.iter().filter(|c| c.side == Some(Side::Left)).count(),
        2
    );
    assert_eq!(
        limbs.iter().filter(|c| c.side == Some(Side::Right)).count(),
        2
    );
    for leg in limbs.iter().filter(|c| c.role == Some(LimbRole::Leg)) {
        assert_eq!(leg.posture, Some(Posture::Plantigrade), "{}", leg.name);
    }
    for arm in limbs.iter().filter(|c| c.role == Some(LimbRole::Arm)) {
        assert_eq!(arm.posture, None, "an arm has no ground posture");
    }
}

/// A three-bone skeleton, for the failure cases.
fn toy() -> Vec<(String, Option<String>)> {
    vec![
        ("root".into(), None),
        ("spine".into(), Some("root".into())),
        ("head".into(), Some("spine".into())),
    ]
}

fn chain(name: &str, kind: ChainKind, bones: &[&str]) -> Chain {
    Chain {
        name: name.into(),
        kind,
        bones: bones.iter().map(|b| (*b).to_string()).collect(),
        side: None,
        role: None,
        posture: None,
    }
}

fn template_of(chains: Vec<Chain>) -> Template {
    Template {
        name: "toy".into(),
        skeleton: "toy.glb".into(),
        chains,
    }
}

/// A bone no chain claims is reported. This is the error that actually happens:
/// a skeleton gains a bone, the manifest is not updated, and that bone silently
/// has no kind for every stage that asks what it is.
#[test]
fn a_bone_no_chain_claims_is_reported() {
    let bones = toy();
    let template = template_of(vec![
        chain("root", ChainKind::Root, &["root"]),
        chain("spine", ChainKind::Spine, &["spine"]),
    ]);
    assert_eq!(
        template.check(&borrow(&bones)),
        vec![TemplateProblem::UnclaimedBone {
            bone: "head".into()
        }]
    );
}

/// Two chains claiming one bone is reported, naming both.
#[test]
fn a_bone_two_chains_claim_is_reported() {
    let bones = toy();
    let template = template_of(vec![
        chain("root", ChainKind::Root, &["root"]),
        chain("a", ChainKind::Spine, &["spine", "head"]),
        chain("b", ChainKind::Neck, &["head"]),
    ]);
    assert_eq!(
        template.check(&borrow(&bones)),
        vec![TemplateProblem::DoublyClaimedBone {
            bone: "head".into(),
            first: "a".into(),
            second: "b".into(),
        }]
    );
}

/// A chain naming a bone the skeleton lacks is reported once, as unknown -- not
/// again as a broken link.
#[test]
fn a_chain_naming_a_missing_bone_is_reported_once() {
    let bones = toy();
    let template = template_of(vec![
        chain("root", ChainKind::Root, &["root"]),
        chain("spine", ChainKind::Spine, &["spine", "tail", "head"]),
    ]);
    assert_eq!(
        template.check(&borrow(&bones)),
        vec![TemplateProblem::UnknownBone {
            chain: "spine".into(),
            bone: "tail".into(),
        }]
    );
}

/// Bones listed in an order the skeleton does not have are reported. Without
/// this a manifest could claim every bone exactly once and still describe a
/// hierarchy that does not exist.
#[test]
fn a_chain_that_is_not_a_parent_to_child_run_is_reported() {
    let bones = toy();
    let template = template_of(vec![
        chain("root", ChainKind::Root, &["root"]),
        // head is a child of spine, so this is backwards.
        chain("spine", ChainKind::Spine, &["head", "spine"]),
    ]);
    assert_eq!(
        template.check(&borrow(&bones)),
        vec![TemplateProblem::BrokenChain {
            chain: "spine".into(),
            parent: "head".into(),
            child: "spine".into(),
        }]
    );
}

/// A skipped generation is not a chain either: `root` is not the parent of
/// `head`, even though both lie on one path.
#[test]
fn a_chain_that_skips_a_bone_is_reported() {
    let bones = toy();
    let template = template_of(vec![
        chain("root", ChainKind::Root, &["root", "head"]),
        chain("spine", ChainKind::Spine, &["spine"]),
    ]);
    let problems = template.check(&borrow(&bones));
    assert!(
        problems.contains(&TemplateProblem::BrokenChain {
            chain: "root".into(),
            parent: "root".into(),
            child: "head".into(),
        }),
        "got {problems:?}"
    );
}

/// Exactly one root chain, no more and no fewer.
#[test]
fn a_template_needs_exactly_one_root() {
    let bones = toy();
    for (chains, found) in [
        (
            vec![
                chain("a", ChainKind::Root, &["root"]),
                chain("b", ChainKind::Root, &["spine"]),
                chain("c", ChainKind::Head, &["head"]),
            ],
            2,
        ),
        (
            vec![
                chain("a", ChainKind::Spine, &["root", "spine"]),
                chain("c", ChainKind::Head, &["head"]),
            ],
            0,
        ),
    ] {
        let problems = template_of(chains).check(&borrow(&bones));
        assert!(
            problems.contains(&TemplateProblem::RootCount { found }),
            "expected a root-count problem, got {problems:?}"
        );
    }
}

/// Every problem is reported in one pass, so a manifest can be fixed at once
/// rather than one error per run.
#[test]
fn all_the_problems_are_reported_together() {
    let bones = toy();
    let template = template_of(vec![
        chain("dup", ChainKind::Root, &["root"]),
        chain("dup", ChainKind::Spine, &["nope"]),
        chain("empty", ChainKind::Tail, &[]),
    ]);
    let problems = template.check(&borrow(&bones));
    assert!(problems.len() >= 5, "got only {problems:?}");
    assert!(problems.contains(&TemplateProblem::DuplicateChainName {
        chain: "dup".into()
    }));
    assert!(problems.contains(&TemplateProblem::EmptyChain {
        chain: "empty".into()
    }));
    assert!(problems.contains(&TemplateProblem::UnknownBone {
        chain: "dup".into(),
        bone: "nope".into()
    }));
    assert!(problems.contains(&TemplateProblem::UnclaimedBone {
        bone: "spine".into()
    }));
    assert!(problems.contains(&TemplateProblem::UnclaimedBone {
        bone: "head".into()
    }));
}

/// The fox describes its own skeleton too.
///
/// The point of a second species: it is what tells you whether the format
/// generalises or was fitted to one file. It also forced
/// [`ChainKind::Accessory`] into existence — a fox has ears and a belly bone,
/// and none of the kinds derived from the human, bird, snake, spider and shark
/// rigs covered them.
#[test]
fn the_fox_template_describes_its_skeleton_exactly() {
    assert_template_matches("fox.json", "rig-fox.glb", 49);
}

/// A fox stands on its toes and a human does not, and the templates say so.
///
/// This is the whole reason posture is recorded rather than inferred: the two
/// rigs are otherwise described with the same kinds, and a fitter that put a
/// fox's ankle on the ground would be wrong in a way no bone count reveals.
#[test]
fn the_fox_is_digitigrade_where_the_human_is_plantigrade() {
    let fox = load("fox.json");
    let human = load("human.json");

    let postures = |t: &Template| -> Vec<Option<Posture>> {
        let mut p: Vec<Option<Posture>> = t
            .of_kind(ChainKind::Limb)
            .filter(|c| c.role == Some(LimbRole::Leg))
            .map(|c| c.posture)
            .collect();
        p.dedup();
        p
    };
    assert_eq!(postures(&fox), vec![Some(Posture::Digitigrade)]);
    assert_eq!(postures(&human), vec![Some(Posture::Plantigrade)]);

    // The fox has four legs and no arms; the human has two of each.
    let legs = |t: &Template| {
        t.of_kind(ChainKind::Limb)
            .filter(|c| c.role == Some(LimbRole::Leg))
            .count()
    };
    assert_eq!(legs(&fox), 4);
    assert_eq!(legs(&human), 2);
    assert_eq!(fox.of_kind(ChainKind::Tail).count(), 1, "a fox has a tail");
    assert_eq!(fox.of_kind(ChainKind::Jaw).count(), 1, "and a jaw");
}

/// The bird, whose wings carry feather chains part-way along their length.
///
/// One of the two templates picked to stress the format before the easy ones.
/// It needed no new kind: a feather is a [`ChainKind::Digit`], the same as a
/// finger, and both jaws are [`ChainKind::Jaw`] — this rig parents
/// `mouth_lower` to `mouth_upper`, so they are two chains rather than one.
#[test]
fn the_bird_template_describes_its_skeleton_exactly() {
    assert_template_matches("bird.json", "rig-bird.glb", 55);
}

/// The spider: eight legs, each behind an anchor bone.
///
/// The other stress case, and it needed no new kind either. Each leg chain
/// starts at its `legs_anchor_N` exactly as the human arm chain starts at its
/// clavicle — the bone that attaches a limb belongs to the limb. The two
/// per-side hubs the anchors themselves hang from are `Accessory`.
#[test]
fn the_spider_template_describes_its_skeleton_exactly() {
    assert_template_matches("spider.json", "rig-spider.glb", 56);
}

/// The bird and spider have the chains those creatures have.
#[test]
fn the_stress_templates_have_the_chains_those_creatures_have() {
    let bird = load("bird.json");
    assert_eq!(
        bird.of_kind(ChainKind::Limb).count(),
        4,
        "two wings, two legs"
    );
    assert_eq!(
        bird.of_kind(ChainKind::Limb)
            .filter(|c| c.role == Some(LimbRole::Wing))
            .count(),
        2
    );
    assert_eq!(
        bird.of_kind(ChainKind::Digit).count(),
        8,
        "four feather chains a wing"
    );
    assert_eq!(bird.of_kind(ChainKind::Jaw).count(), 2, "upper and lower");

    let spider = load("spider.json");
    let legs: Vec<&Chain> = spider
        .of_kind(ChainKind::Limb)
        .filter(|c| c.role == Some(LimbRole::Leg))
        .collect();
    assert_eq!(legs.len(), 8, "a spider has eight legs");
    assert_eq!(
        legs.iter().filter(|c| c.side == Some(Side::Left)).count(),
        4
    );
    assert_eq!(
        legs.iter().filter(|c| c.side == Some(Side::Right)).count(),
        4
    );
    assert!(
        legs.iter().all(|c| c.bones.len() == 5),
        "each leg is its anchor plus four bones"
    );

    // Posture is left unset here, deliberately. Plantigrade and digitigrade
    // describe how a mammal's foot meets the ground; an arthropod leg is
    // neither, and recording a wrong value would be worse than recording none.
    assert!(
        legs.iter().all(|c| c.posture.is_none()),
        "a spider leg is neither plantigrade nor digitigrade"
    );
    assert_eq!(spider.of_kind(ChainKind::Jaw).count(), 2, "two fangs");
    assert_eq!(
        spider.of_kind(ChainKind::Accessory).count(),
        2,
        "the per-side hubs the leg anchors hang from"
    );
}

/// Every shipped template describes its own skeleton.
///
/// Table-driven so a new template cannot be added without being checked here,
/// and so a failure names which creature broke.
#[test]
fn every_template_describes_its_skeleton_exactly() {
    for (manifest, skeleton, joints) in [
        ("human.json", "rig-human.glb", 66),
        ("fox.json", "rig-fox.glb", 49),
        ("bird.json", "rig-bird.glb", 55),
        ("spider.json", "rig-spider.glb", 56),
        ("snake.json", "rig-snake.glb", 28),
        ("shark.json", "rig-shark.glb", 33),
        ("horse.json", "rig-horse.glb", 56),
        ("kaiju.json", "rig-kaiju.glb", 58),
        ("dragon.json", "rig-dragon.glb", 99),
    ] {
        assert_template_matches(manifest, skeleton, joints);
    }
}

/// There is a manifest for every rig in `assets/rigs`, and no manifest without
/// one.
///
/// Without this, adding a tenth rig and forgetting its manifest would pass
/// every other test in this file, because they all iterate the manifests.
#[test]
fn every_rig_has_a_manifest_and_every_manifest_a_rig() {
    let rigs = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/rigs");
    let mut skeletons: Vec<String> = std::fs::read_dir(rigs)
        .expect("the rigs directory")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("rig-") && n.ends_with(".glb"))
        .collect();
    skeletons.sort();

    let templates = concat!(env!("CARGO_MANIFEST_DIR"), "/templates");
    let mut described: Vec<String> = std::fs::read_dir(templates)
        .expect("the templates directory")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".json"))
        .map(|n| load(&n).skeleton)
        .collect();
    described.sort();

    assert_eq!(
        described, skeletons,
        "every rig needs a manifest and every manifest a rig"
    );
}

/// Each creature is described with the kinds it actually has, so a manifest
/// cannot pass by lumping bones into whatever chain is convenient.
#[test]
fn each_creature_has_the_chains_it_should() {
    // (template, limbs, digits, tails, jaws)
    for (manifest, limbs, digits, tails, jaws) in [
        // shark has no Tail chain: its "tail_*" bones are its body axis, the
        // same call made for the snake. A fox's tail is an appendage behind a
        // separate spine; a shark's is the spine.
        ("human.json", 4, 10, 0, 0),
        ("fox.json", 4, 0, 1, 1),
        ("bird.json", 4, 8, 1, 2),
        ("spider.json", 8, 0, 1, 2),
        ("snake.json", 0, 0, 0, 1),
        ("shark.json", 4, 0, 0, 1),
        ("horse.json", 4, 0, 1, 1),
        ("kaiju.json", 4, 6, 1, 1),
        ("dragon.json", 6, 10, 1, 2),
    ] {
        let t = load(manifest);
        assert_eq!(
            t.of_kind(ChainKind::Limb).count(),
            limbs,
            "{manifest} limbs"
        );
        assert_eq!(
            t.of_kind(ChainKind::Digit).count(),
            digits,
            "{manifest} digits"
        );
        assert_eq!(
            t.of_kind(ChainKind::Tail).count(),
            tails,
            "{manifest} tails"
        );
        assert_eq!(t.of_kind(ChainKind::Jaw).count(), jaws, "{manifest} jaws");
        assert_eq!(t.of_kind(ChainKind::Root).count(), 1, "{manifest} root");
    }
}

/// Limbs carry the role that creature's limbs have.
///
/// **Found by mutation**: relabelling the shark's four fins as legs passed
/// every other test in this file, because the counts only asked how many limbs
/// there are, never what they are for. Role is what tells a fitter that a fin
/// sweeps and an arm reaches.
#[test]
fn limbs_carry_the_role_that_creature_has() {
    // (template, arms, legs, wings, fins)
    for (manifest, arms, legs, wings, fins) in [
        ("human.json", 2, 2, 0, 0),
        ("fox.json", 0, 4, 0, 0),
        ("bird.json", 0, 2, 2, 0),
        ("spider.json", 0, 8, 0, 0),
        ("snake.json", 0, 0, 0, 0),
        ("shark.json", 0, 0, 0, 4),
        ("horse.json", 0, 4, 0, 0),
        ("kaiju.json", 2, 2, 0, 0),
        ("dragon.json", 0, 4, 2, 0),
    ] {
        let t = load(manifest);
        let count = |role: LimbRole| {
            t.of_kind(ChainKind::Limb)
                .filter(|c| c.role == Some(role))
                .count()
        };
        assert_eq!(count(LimbRole::Arm), arms, "{manifest} arms");
        assert_eq!(count(LimbRole::Leg), legs, "{manifest} legs");
        assert_eq!(count(LimbRole::Wing), wings, "{manifest} wings");
        assert_eq!(count(LimbRole::Fin), fins, "{manifest} fins");
        assert_eq!(
            t.of_kind(ChainKind::Limb)
                .filter(|c| c.role.is_none())
                .count(),
            0,
            "{manifest}: every limb needs a role"
        );
    }
}

/// Posture is recorded where it means something and left unset where it does
/// not, rather than defaulted to whatever is nearest.
///
/// Three of the nine creatures meet the ground three different ways, and a
/// spider meets it in none of them.
#[test]
fn posture_is_recorded_only_where_it_applies() {
    let posture_of = |manifest: &str| -> Vec<Option<Posture>> {
        let t = load(manifest);
        let mut p: Vec<Option<Posture>> = t
            .of_kind(ChainKind::Limb)
            .filter(|c| c.role == Some(LimbRole::Leg))
            .map(|c| c.posture)
            .collect();
        p.dedup();
        p
    };
    assert_eq!(posture_of("human.json"), vec![Some(Posture::Plantigrade)]);
    assert_eq!(posture_of("fox.json"), vec![Some(Posture::Digitigrade)]);
    assert_eq!(posture_of("horse.json"), vec![Some(Posture::Unguligrade)]);
    // A spider is an arthropod: none of the three describe it, so none is
    // recorded. Wings and fins carry no posture either.
    assert_eq!(posture_of("spider.json"), vec![None]);
    for manifest in ["bird.json", "dragon.json"] {
        let t = load(manifest);
        for limb in t.of_kind(ChainKind::Limb) {
            if limb.role != Some(LimbRole::Leg) {
                assert_eq!(limb.posture, None, "{manifest} {}", limb.name);
            }
        }
    }
}

/// The manifest survives a round trip through JSON, so an editor and this crate
/// agree on the format.
#[test]
fn a_template_round_trips_through_json() {
    let template = load("human.json");
    let text = serde_json::to_string(&template).expect("serialises");
    let back: Template = serde_json::from_str(&text).expect("deserialises");
    assert_eq!(back, template);
}
