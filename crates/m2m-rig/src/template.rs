//! The template format: a skeleton described as typed chains of bones.
//!
//! # Why chains rather than a bone list
//!
//! The nine shipped templates are `.glb` files holding a flat list of bones.
//! Nothing in them says *what* a run of bones is, so every stage downstream has
//! to guess from names: the legacy retargeter carries 32 name categories and a
//! tokenizer to do exactly that. A template that states "these five bones are a
//! digitigrade leg" makes the guessing unnecessary, gives fitting a per-kind
//! rule to apply, and turns "support another creature" into new data rather
//! than new code — which `crate`'s design rule requires.
//!
//! # What a chain is
//!
//! A contiguous parent-to-child run of bones with one kind. Contiguity is
//! enforced, not assumed: `bones[i + 1]` must be a child of `bones[i]`. A
//! manifest that claims otherwise describes a skeleton that does not exist.
//!
//! # On the kind vocabulary
//!
//! [`ChainKind`] is deliberately small and was derived by reading the actual
//! templates, not by guessing at anatomy — human, bird, snake, spider and shark
//! between them cover the extremes we ship (a 21-bone snake body, a spider's
//! eight legs behind anchor bones, a bird's feather chains hanging off wings).
//! All nine templates describe themselves with these kinds and no others.
//!
//! Adding a kind is a format change and needs the same justification: a real
//! template that cannot be described without it. Adding a *creature* must never
//! need one.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// What a run of bones is, structurally.
///
/// These name a chain's role in the body, not the species. A bird's wing, a
/// human's arm and a shark's fin are all [`ChainKind::Limb`]: a chain hanging
/// off the body axis whose far end is what a fitter has to place. What
/// separates them is [`Limb::role`] and the bone count, not a separate kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainKind {
    /// The single bone the whole skeleton hangs from. Exactly one per template.
    Root,
    /// The body axis: pelvis through chest, or a snake's whole length.
    Spine,
    /// Between the spine and the head.
    Neck,
    /// The head, including any tip bone that only marks its end.
    Head,
    /// A jaw, mouth or fang chain. Bird, snake, spider and shark all have one;
    /// the human template does not.
    Jaw,
    /// A limb: arm, leg, wing, fin, or one of a spider's eight.
    Limb,
    /// A chain hanging off a limb — finger, toe, or wing feather.
    ///
    /// Not necessarily off its *end*: a bird's four feather chains hang off
    /// `wing_2` through `wing_5`, part-way along the wing.
    Digit,
    /// A tail.
    Tail,
    /// A chain with no special fitting behaviour: it follows whatever it hangs
    /// from. Ears, horns, a belly bone, decorative strands.
    ///
    /// **Added because the fox template needs it**, which is the bar this
    /// vocabulary is meant to hold to: `Ear_L -> Ear_Tip_L` and
    /// `Stomach -> Stomach_tip` are none of the kinds above, and calling an ear
    /// a digit to avoid admitting the gap would put a wrong rule on it later.
    /// Rigify reaches the same conclusion from the other direction — its
    /// `basic.super_copy` does exactly this and its bird metarig uses it twenty
    /// times.
    ///
    /// This is the escape hatch, so prefer a real kind when one fits: anything
    /// marked `Accessory` gets no fitting rule beyond following its parent.
    Accessory,
}

/// Which side of the body a chain is on.
///
/// Stored rather than parsed out of a name. Names are not a reliable signal
/// (`_l`, `_L`, `.L`, `Left` all appear across the templates), and a wrong
/// guess mirrors a limb onto the wrong side, which is worse than not knowing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The creature's left.
    Left,
    /// The creature's right.
    Right,
}

/// What a limb is for, which is what makes two limbs fit differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimbRole {
    /// A forelimb that manipulates rather than bears weight.
    Arm,
    /// A weight-bearing limb.
    Leg,
    /// A wing.
    Wing,
    /// A fin.
    Fin,
}

/// How a leg meets the ground.
///
/// The distinction a fox rig needs and a human rig does not: a digitigrade leg
/// stands on its toes with a raised ankle, so the joint a fitter should put on
/// the ground is not the one a plantigrade leg uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Posture {
    /// Sole on the ground, ankle low — human, bear.
    Plantigrade,
    /// Standing on the toes, ankle raised — dog, cat, bird.
    Digitigrade,
    /// Standing on hoofed toe tips — horse, deer.
    ///
    /// **Added because the horse template needs it.** It is a real third
    /// category, not a shade of digitigrade: the ground contact is the hoof at
    /// the very end of the limb, so a fitter grounding the foot bone puts the
    /// horse through the floor. Same bar as [`ChainKind::Accessory`] — a real
    /// template that cannot be described without it.
    Unguligrade,
}

/// One typed run of bones.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chain {
    /// Identifies the chain within its template, e.g. `arm_l`.
    pub name: String,
    /// What this run of bones is.
    pub kind: ChainKind,
    /// Bone names, parent first, each the parent of the next.
    pub bones: Vec<String>,
    /// Which side, for chains that come in pairs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
    /// What a [`ChainKind::Limb`] is for. Absent on other kinds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<LimbRole>,
    /// How a leg meets the ground. Absent when it does not apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posture: Option<Posture>,
}

/// A creature template: its skeleton, described as chains.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Template {
    /// Identifies the template, e.g. `human`.
    pub name: String,
    /// The `.glb` the bones live in, relative to the template directory.
    pub skeleton: String,
    /// A creature-specific tip shown at the Fit step (design.md §7).
    ///
    /// Lives with the template, not in the UI, so a new creature carries its own
    /// guidance. `#[serde(default)]` keeps an older manifest parsing; the test
    /// suite requires every shipped one to fill it.
    #[serde(default)]
    pub guidance: String,
    /// The chains, in no particular order.
    pub chains: Vec<Chain>,
}

// Generated by `build.rs`: every manifest in `templates/`, name and contents.
include!(concat!(env!("OUT_DIR"), "/manifests.rs"));

/// Every creature template that ships with the app.
///
/// The manifests are embedded, so nothing has to be found on disk at runtime
/// and nothing has to be packaged alongside the binary. They are globbed at
/// build time, so a new creature is still a new JSON file and no code change.
///
/// # Errors
///
/// A manifest that does not parse. These are our own files and CI validates
/// them against their skeletons, so this is a build error that escaped —
/// reported rather than silently dropping a creature from the list.
pub fn all() -> Result<Vec<Template>, serde_json::Error> {
    MANIFESTS
        .iter()
        .map(|(_, json)| serde_json::from_str(json))
        .collect()
}

/// Why a template does not describe its skeleton.
///
/// Every variant names a way a manifest and a skeleton can disagree. They are
/// collected rather than returned one at a time, because fixing a manifest one
/// error per run is miserable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateProblem {
    /// A chain names a bone the skeleton does not have.
    #[error("chain {chain:?} names bone {bone:?}, which the skeleton does not have")]
    UnknownBone {
        /// The chain that named it.
        chain: String,
        /// The name it used.
        bone: String,
    },
    /// A bone belongs to no chain, so nothing downstream knows what it is.
    #[error("bone {bone:?} belongs to no chain")]
    UnclaimedBone {
        /// The orphaned bone.
        bone: String,
    },
    /// Two chains claim the same bone.
    #[error("bone {bone:?} is claimed by both {first:?} and {second:?}")]
    DoublyClaimedBone {
        /// The contested bone.
        bone: String,
        /// The chain that claimed it first.
        first: String,
        /// The chain that claimed it again.
        second: String,
    },
    /// Consecutive bones in a chain are not parent and child.
    #[error("in chain {chain:?}, {child:?} is not a child of {parent:?}")]
    BrokenChain {
        /// The chain.
        chain: String,
        /// The bone that should be the parent.
        parent: String,
        /// The bone that should be its child.
        child: String,
    },
    /// A chain has no bones in it.
    #[error("chain {chain:?} is empty")]
    EmptyChain {
        /// The chain.
        chain: String,
    },
    /// Two chains share a name, so neither can be referred to.
    #[error("more than one chain is called {chain:?}")]
    DuplicateChainName {
        /// The repeated name.
        chain: String,
    },
    /// A template needs exactly one root chain.
    #[error("expected exactly one root chain, found {found}")]
    RootCount {
        /// How many there were.
        found: usize,
    },
}

/// A skeleton to check a template against: each bone, and its parent.
///
/// Deliberately not tied to the glTF reader. The check is about names and
/// parentage, and taking the smaller input keeps `m2m-io` out of this crate's
/// dependencies and makes the tests readable.
pub struct Skeleton<'a> {
    parents: HashMap<&'a str, Option<&'a str>>,
}

impl<'a> Skeleton<'a> {
    /// Builds a skeleton from `(bone, parent)` pairs. A root bone has `None`.
    pub fn new(bones: impl IntoIterator<Item = (&'a str, Option<&'a str>)>) -> Self {
        Self {
            parents: bones.into_iter().collect(),
        }
    }

    /// How many bones it has.
    pub fn len(&self) -> usize {
        self.parents.len()
    }

    /// Whether it has no bones.
    pub fn is_empty(&self) -> bool {
        self.parents.is_empty()
    }

    fn has(&self, bone: &str) -> bool {
        self.parents.contains_key(bone)
    }

    fn parent_of(&self, bone: &str) -> Option<&'a str> {
        self.parents.get(bone).copied().flatten()
    }
}

impl Template {
    /// Checks that this template describes the given skeleton exactly.
    ///
    /// Returns every problem found, so a manifest can be fixed in one pass. An
    /// empty result means every bone is claimed by exactly one chain and every
    /// chain is a real parent-to-child run.
    ///
    /// **Both directions matter.** A chain naming a bone that does not exist is
    /// the obvious error; a bone that no chain claims is the one that actually
    /// happens, because a skeleton gains a bone and the manifest is not
    /// updated. That bone then silently has no kind, and every stage that asks
    /// "what is this" gets no answer.
    pub fn check(&self, skeleton: &Skeleton<'_>) -> Vec<TemplateProblem> {
        let mut problems = Vec::new();
        let mut claimed: HashMap<&str, &str> = HashMap::new();
        let mut names: HashMap<&str, ()> = HashMap::new();

        for chain in &self.chains {
            if names.insert(chain.name.as_str(), ()).is_some() {
                problems.push(TemplateProblem::DuplicateChainName {
                    chain: chain.name.clone(),
                });
            }
            if chain.bones.is_empty() {
                problems.push(TemplateProblem::EmptyChain {
                    chain: chain.name.clone(),
                });
                continue;
            }

            for bone in &chain.bones {
                if !skeleton.has(bone) {
                    problems.push(TemplateProblem::UnknownBone {
                        chain: chain.name.clone(),
                        bone: bone.clone(),
                    });
                    continue;
                }
                if let Some(first) = claimed.insert(bone.as_str(), chain.name.as_str()) {
                    problems.push(TemplateProblem::DoublyClaimedBone {
                        bone: bone.clone(),
                        first: first.to_owned(),
                        second: chain.name.clone(),
                    });
                }
            }

            // Contiguity. Checked only where both bones exist, so a missing
            // bone reports once as unknown rather than again as a broken link.
            for pair in chain.bones.windows(2) {
                let (parent, child) = (&pair[0], &pair[1]);
                if !skeleton.has(parent) || !skeleton.has(child) {
                    continue;
                }
                if skeleton.parent_of(child) != Some(parent.as_str()) {
                    problems.push(TemplateProblem::BrokenChain {
                        chain: chain.name.clone(),
                        parent: parent.clone(),
                        child: child.clone(),
                    });
                }
            }
        }

        let roots = self
            .chains
            .iter()
            .filter(|c| c.kind == ChainKind::Root)
            .count();
        if roots != 1 {
            problems.push(TemplateProblem::RootCount { found: roots });
        }

        let mut unclaimed: Vec<&str> = skeleton
            .parents
            .keys()
            .copied()
            .filter(|bone| !claimed.contains_key(bone))
            .collect();
        unclaimed.sort_unstable();
        problems.extend(
            unclaimed
                .into_iter()
                .map(|bone| TemplateProblem::UnclaimedBone {
                    bone: bone.to_owned(),
                }),
        );

        problems
    }

    /// Every bone the template claims, in chain order.
    pub fn bones(&self) -> impl Iterator<Item = &str> {
        self.chains
            .iter()
            .flat_map(|c| c.bones.iter().map(String::as_str))
    }

    /// The chains of one kind.
    pub fn of_kind(&self, kind: ChainKind) -> impl Iterator<Item = &Chain> {
        self.chains.iter().filter(move |c| c.kind == kind)
    }
}
