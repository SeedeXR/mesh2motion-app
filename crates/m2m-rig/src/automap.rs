//! Matching an incoming rig's bones to a template's chains, without using names.
//!
//! # Why not names
//!
//! The legacy resolves a bone to a canonical slot by parsing its name and then
//! uses the hierarchy to propagate side and depth
//! (`legacy/src/retarget/bone-automap/BoneChainResolver.ts`). That works well
//! when the names mean something, and not at all when they do not. Measured
//! against the legacy resolver itself:
//!
//! | rig | bones resolved to a slot |
//! |---|---|
//! | named humanoid (`hips`, `spine`, `upperarm_l`, ...) | **7 of 7** |
//! | same shape, bones called `Bone.000`.. | **0 of 17** |
//!
//! That second row is not hypothetical. A giraffe supplied as reference has
//! every bone named `Bone.027` through `Bone.055`, with many parented to
//! nothing — 31 chains for 48 bones.
//!
//! So this matches on what a skeleton *is* rather than what it is called:
//! chain length, depth, direction, reach, and which side of the body it is on.
//! Names are not consulted at all, which also means a rig that names its left
//! arm "right" cannot mislead it.

use std::collections::HashMap;

use glam::Vec3;

use crate::template::{ChainKind, Template};

/// A skeleton to match: each bone's name, parent and rest position.
///
/// Names are carried so a caller can report a mapping, never to decide one.
#[derive(Debug, Clone, PartialEq)]
pub struct Skeleton {
    /// Bone names, indexed as the caller indexes them.
    pub names: Vec<String>,
    /// Parent of each bone, or `None` for a root.
    pub parents: Vec<Option<usize>>,
    /// World-space rest position of each bone.
    pub positions: Vec<Vec3>,
}

impl Skeleton {
    /// Splits the skeleton into maximal parent-to-child runs.
    ///
    /// A run ends where the skeleton branches: a bone with two or more children
    /// finishes its chain and each child starts a new one. The same
    /// decomposition `tools/glb-chains.py` performs, and the unit a template
    /// describes.
    pub fn chains(&self) -> Vec<Vec<usize>> {
        let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
        for (index, parent) in self.parents.iter().enumerate() {
            if let Some(parent) = parent {
                children.entry(*parent).or_default().push(index);
            }
        }
        let starts: Vec<usize> = (0..self.names.len())
            .filter(|index| match self.parents[*index] {
                None => true,
                Some(parent) => children.get(&parent).map_or(0, Vec::len) > 1,
            })
            .collect();

        starts
            .into_iter()
            .map(|start| {
                let mut run = vec![start];
                while let Some(only) = children
                    .get(run.last().expect("non-empty"))
                    .filter(|kids| kids.len() == 1)
                    .map(|kids| kids[0])
                {
                    run.push(only);
                }
                run
            })
            .collect()
    }

    /// Overall height, used to express reaches as proportions.
    fn height(&self) -> f32 {
        let (lo, hi) = self.bounds();
        (hi.y - lo.y).max(f32::EPSILON)
    }

    fn bounds(&self) -> (Vec3, Vec3) {
        let first = self.positions.first().copied().unwrap_or(Vec3::ZERO);
        self.positions
            .iter()
            .fold((first, first), |(lo, hi), &p| (lo.min(p), hi.max(p)))
    }

    /// Midpoint of the X extent: the plane left and right are measured against.
    fn symmetry_x(&self) -> f32 {
        let (lo, hi) = self.bounds();
        (lo.x + hi.x) * 0.5
    }
}

/// What a chain looks like, with no reference to what it is called.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Signature {
    /// How many bones it holds.
    pub bones: usize,
    /// How many ancestors its first bone has.
    pub depth: usize,
    /// Direction from its first bone to its last, normalised.
    pub direction: Vec3,
    /// Distance from its first bone to its last, over the skeleton's height.
    pub reach: f32,
    /// Where its midpoint sits across the body, over the skeleton's height.
    /// Negative is one side, positive the other; near zero is the midline.
    pub lateral: f32,
    /// Height of its first bone above the skeleton's lowest point, over height.
    pub attachment_height: f32,
}

impl Signature {
    /// How unlike another signature this is. Zero is identical.
    ///
    /// The weights say what matters when two chains compete. Direction and
    /// side dominate: an arm and a leg of the same length differ by pointing
    /// opposite ways, and a left arm differs from a right one only in `lateral`.
    /// Bone count is deliberately weak — a template's arm has four bones and an
    /// incoming rig's may have three or five without being anything else.
    pub fn distance(&self, other: &Signature) -> f32 {
        let direction = (self.direction - other.direction).length();
        let lateral = (self.lateral - other.lateral).abs();
        let reach = (self.reach - other.reach).abs();
        let attachment = (self.attachment_height - other.attachment_height).abs();
        let bones = (self.bones as f32 - other.bones as f32).abs() / 8.0;
        let depth = (self.depth as f32 - other.depth as f32).abs() / 8.0;

        2.0 * direction + 2.0 * lateral + reach + attachment + 0.5 * bones + 0.5 * depth
    }
}

/// Describes one chain of a skeleton, by index into `Skeleton`.
pub fn signature_of(skeleton: &Skeleton, chain: &[usize]) -> Option<Signature> {
    let (&first, &last) = (chain.first()?, chain.last()?);
    let height = skeleton.height();
    let (lo, _) = skeleton.bounds();
    let symmetry_x = skeleton.symmetry_x();

    let start = skeleton.positions[first];
    let end = skeleton.positions[last];
    let span = end - start;
    // A one-bone chain has no direction of its own, so it borrows the direction
    // it leaves its parent in. Otherwise every stub would look identical.
    let direction = if span.length_squared() > f32::EPSILON {
        span.normalize()
    } else {
        match skeleton.parents[first] {
            Some(parent) => (start - skeleton.positions[parent])
                .try_normalize()
                .unwrap_or(Vec3::ZERO),
            None => Vec3::ZERO,
        }
    };

    let mut depth = 0;
    let mut cursor = skeleton.parents[first];
    while let Some(index) = cursor {
        depth += 1;
        cursor = skeleton.parents[index];
        if depth > skeleton.names.len() {
            break; // a cycle: refuse to hang rather than trust the file
        }
    }

    Some(Signature {
        bones: chain.len(),
        depth,
        direction,
        reach: span.length() / height,
        lateral: ((start.x + end.x) * 0.5 - symmetry_x) / height,
        attachment_height: (start.y - lo.y) / height,
    })
}

/// Two chains that describe the same part of two skeletons.
#[derive(Debug, Clone, PartialEq)]
pub struct ChainMatch {
    /// Bone indices in the reference skeleton, in chain order.
    pub reference: Vec<usize>,
    /// Bone indices in the incoming skeleton, in chain order.
    pub incoming: Vec<usize>,
    /// How unlike each other they are. Lower is better.
    pub cost: f32,
}

/// Matches two skeletons chain by chain, on structure alone.
///
/// Both sides are decomposed into **maximal** parent-to-child runs and paired
/// greedily, cheapest first, each chain used at most once.
///
/// Maximal runs are the right unit here and template chains are not: a template
/// splits by meaning, so `human.json` puts `pelvis` at the head of its spine,
/// while topologically `pelvis` has three children and therefore ends the chain
/// that starts at `root`. Matching structure to structure keeps both sides
/// speaking the same language; [`map_bones`] then carries a template's own
/// naming across.
pub fn match_skeletons(reference: &Skeleton, incoming: &Skeleton) -> Vec<ChainMatch> {
    let (left, right) = (reference.chains(), incoming.chains());
    let left_signatures: Vec<Option<Signature>> =
        left.iter().map(|c| signature_of(reference, c)).collect();
    let right_signatures: Vec<Option<Signature>> =
        right.iter().map(|c| signature_of(incoming, c)).collect();

    let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
    for (l, left_signature) in left_signatures.iter().enumerate() {
        for (r, right_signature) in right_signatures.iter().enumerate() {
            if let (Some(a), Some(b)) = (left_signature, right_signature) {
                pairs.push((a.distance(b), l, r));
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut taken_left = vec![false; left.len()];
    let mut taken_right = vec![false; right.len()];
    let mut matches = Vec::new();
    for (cost, l, r) in pairs {
        if taken_left[l] || taken_right[r] {
            continue;
        }
        taken_left[l] = true;
        taken_right[r] = true;
        matches.push(ChainMatch {
            reference: left[l].clone(),
            incoming: right[r].clone(),
            cost,
        });
    }
    matches
}

/// Maps each reference bone to a bone of the incoming skeleton.
///
/// Within a matched pair of chains, bones are paired by their position along
/// the chain, proportionally, so a four-bone arm can map onto a three-bone one.
/// A reference bone whose chain found no partner is absent from the result
/// rather than guessed at.
pub fn map_bones(reference: &Skeleton, incoming: &Skeleton) -> HashMap<usize, usize> {
    let mut mapping = HashMap::new();
    for matched in match_skeletons(reference, incoming) {
        let (from, to) = (matched.reference, matched.incoming);
        if to.is_empty() {
            continue;
        }
        for (position, &bone) in from.iter().enumerate() {
            let fraction = if from.len() > 1 {
                position as f32 / (from.len() - 1) as f32
            } else {
                0.0
            };
            let index = (fraction * (to.len() - 1) as f32).round() as usize;
            mapping.insert(bone, to[index.min(to.len() - 1)]);
        }
    }
    mapping
}

/// One template chain matched to one incoming chain.
#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    /// The template chain's name.
    pub template_chain: String,
    /// What the template says that chain is.
    pub kind: ChainKind,
    /// Indices into the incoming skeleton, in chain order.
    pub bones: Vec<usize>,
    /// How unlike the template's chain the match is. Lower is better.
    pub cost: f32,
}

/// Matches a template's chains onto an incoming skeleton, using structure only.
///
/// `reference` is the template's own skeleton — the rig the template describes —
/// which supplies the shape each template chain is expected to have.
///
/// Greedy: every template chain is paired with its best remaining candidate,
/// best pairs first. An incoming chain is used at most once, so two template
/// chains cannot both claim the same bones.
pub fn match_chains(template: &Template, reference: &Skeleton, incoming: &Skeleton) -> Vec<Match> {
    // The shape of each template chain, taken from the rig it describes.
    let by_name: HashMap<&str, usize> = reference
        .names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.as_str(), index))
        .collect();

    let mut wanted: Vec<(&str, ChainKind, Signature)> = Vec::new();
    for chain in &template.chains {
        let indices: Vec<usize> = chain
            .bones
            .iter()
            .filter_map(|bone| by_name.get(bone.as_str()).copied())
            .collect();
        if indices.len() != chain.bones.len() {
            continue; // the template names a bone the reference does not have
        }
        if let Some(signature) = signature_of(reference, &indices) {
            wanted.push((chain.name.as_str(), chain.kind, signature));
        }
    }

    let candidates: Vec<Vec<usize>> = incoming.chains();
    let signatures: Vec<Option<Signature>> = candidates
        .iter()
        .map(|chain| signature_of(incoming, chain))
        .collect();

    // Every pairing, cheapest first.
    let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
    for (w, (_, _, want)) in wanted.iter().enumerate() {
        for (c, candidate) in signatures.iter().enumerate() {
            if let Some(candidate) = candidate {
                pairs.push((want.distance(candidate), w, c));
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut taken_template = vec![false; wanted.len()];
    let mut taken_candidate = vec![false; candidates.len()];
    let mut matches = Vec::new();
    for (cost, w, c) in pairs {
        if taken_template[w] || taken_candidate[c] {
            continue;
        }
        taken_template[w] = true;
        taken_candidate[c] = true;
        matches.push(Match {
            template_chain: wanted[w].0.to_owned(),
            kind: wanted[w].1,
            bones: candidates[c].clone(),
            cost,
        });
    }
    matches.sort_by(|a, b| a.template_chain.cmp(&b.template_chain));
    matches
}
