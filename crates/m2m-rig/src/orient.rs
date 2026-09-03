//! Re-orienting a fitted skeleton's rest rotations to match its positions.
//!
//! Fitting moves joints but leaves each bone carrying the *template's* local
//! rest rotation. When the template and the mesh share a pose that is fine —
//! the orientations already match the positions. When they do not (a T-pose
//! template fitted onto an A-pose character), the arm bones end up pointing
//! down while their rotations still say "out", and every clip retargeted onto
//! that rig turns the arms around the wrong axis.
//!
//! The fix reorients each bone by the *minimal* rotation that carries its
//! template bone→child direction onto its fitted one, then keeps the template's
//! own orientation (its roll and its relationship to the parent) on top of that.
//! When the fit matches the template every such rotation is the identity, so the
//! result is exactly the template rotations — the correction cannot regress a
//! rig whose pose already agrees with its skeleton.

use crate::template::{ChainKind, Template};
use glam::{Quat, Vec3};
use std::collections::HashMap;

/// The aim map for [`pose_matched_local_rotations`]: within each **limb** chain
/// (arm, leg, wing, fin) every bone aims at the next, and everything else — the
/// spine, the head, and the fingers and toes hanging off a limb — aims at
/// nothing and keeps its template rotation.
///
/// Limbs are the only chains that are both pose-ambiguous (A-pose vs T-pose) and
/// placed reliably enough by the fitter to aim along. Accessory chains land
/// poorly (a fitted finger can sit above its hand), so aiming at them would turn
/// bones from noise.
pub fn limb_aims(template: &Template, bones: &[String]) -> Vec<Option<usize>> {
    let index_of: HashMap<&str, usize> = bones
        .iter()
        .enumerate()
        .map(|(i, b)| (b.as_str(), i))
        .collect();
    let mut aim = vec![None; bones.len()];
    for chain in &template.chains {
        if chain.kind != ChainKind::Limb {
            continue;
        }
        for pair in chain.bones.windows(2) {
            if let (Some(&a), Some(&b)) = (
                index_of.get(pair[0].as_str()),
                index_of.get(pair[1].as_str()),
            ) {
                aim[a] = Some(b);
            }
        }
    }
    aim
}

/// Composes local rotations down the hierarchy into world rotations.
///
/// `parents` must be topologically ordered (every parent precedes its child),
/// which is how skeletons come out of a glTF/FBX node graph.
fn world_rotations(parents: &[Option<usize>], local: &[Quat]) -> Vec<Quat> {
    let mut world = vec![Quat::IDENTITY; parents.len()];
    for bone in 0..parents.len() {
        let l = local.get(bone).copied().unwrap_or(Quat::IDENTITY);
        world[bone] = match parents[bone] {
            Some(parent) => world[parent] * l,
            None => l,
        };
    }
    world
}

/// New **local** rest rotations that make the skeleton's orientations agree with
/// its fitted positions.
///
/// `aim[bone]` names the bone this one points along — its successor in the same
/// limb chain — or `None` for a bone that should keep the template's own
/// orientation (a chain end, a finger, the spine). Only limb bones are aimed,
/// because only they are both pose-ambiguous and placed reliably by the fitter;
/// aiming a bone at a poorly-fitted child (fingers land above the hand) would
/// invent a rotation from noise.
///
/// Returns the template rotations unchanged wherever the fitted bone directions
/// already match the template's — so a rig fitted onto a mesh of its own pose is
/// untouched.
pub fn pose_matched_local_rotations(
    parents: &[Option<usize>],
    template_positions: &[Vec3],
    template_local_rotations: &[Quat],
    fitted_positions: &[Vec3],
    aim: &[Option<usize>],
) -> Vec<Quat> {
    let n = parents.len();
    let template_world = world_rotations(parents, template_local_rotations);

    // New world rotation per bone, filled parent-first so a child can read its
    // parent's corrected world rotation.
    let mut new_world = vec![Quat::IDENTITY; n];
    for bone in 0..n {
        let template_local = match parents[bone] {
            Some(parent) => template_world[parent].inverse() * template_world[bone],
            None => template_world[bone],
        };
        match aim.get(bone).copied().flatten() {
            Some(target) => {
                // The minimal rotation carrying the template aim onto the fitted
                // aim, applied on top of the template's world orientation.
                let template_dir = direction(template_positions, bone, target);
                let fitted_dir = direction(fitted_positions, bone, target);
                let delta = match (template_dir, fitted_dir) {
                    (Some(t), Some(f)) => Quat::from_rotation_arc(t, f),
                    _ => Quat::IDENTITY,
                };
                new_world[bone] = delta * template_world[bone];
            }
            // Not aimed: ride the (already corrected) parent rigidly, keeping the
            // template's local rotation.
            None => {
                new_world[bone] = match parents[bone] {
                    Some(parent) => new_world[parent] * template_local,
                    None => template_world[bone],
                };
            }
        }
    }

    // Back to local space against the corrected parents.
    (0..n)
        .map(|bone| match parents[bone] {
            Some(parent) => (new_world[parent].inverse() * new_world[bone]).normalize(),
            None => new_world[bone].normalize(),
        })
        .collect()
}

/// The normalised direction from bone `a` to bone `b`, or `None` if they sit on
/// top of each other.
fn direction(positions: &[Vec3], a: usize, b: usize) -> Option<Vec3> {
    let (Some(&from), Some(&to)) = (positions.get(a), positions.get(b)) else {
        return None;
    };
    let d = to - from;
    (d.length() > 1e-6).then(|| d.normalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-bone arm: shoulder → hand. Template arm points +X (a T-pose);
    /// fitting drops it 45° down and out.
    #[test]
    fn a_dropped_arm_gets_a_rotation_that_aims_it_down() {
        let parents = vec![None, Some(0)];
        let template_positions = vec![Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)];
        let template_local = vec![Quat::IDENTITY, Quat::IDENTITY];
        // Hand moved down and out: the shoulder→hand direction is now (1,-1,0).
        let fitted_positions = vec![Vec3::ZERO, Vec3::new(0.7, -0.7, 0.0)];

        let aim = vec![Some(1), None];
        let local = pose_matched_local_rotations(
            &parents,
            &template_positions,
            &template_local,
            &fitted_positions,
            &aim,
        );

        // The shoulder's corrected world rotation must carry the template aim
        // (+X) onto the fitted aim (down-and-out).
        let world = world_rotations(&parents, &local);
        let aimed = world[0] * Vec3::X;
        let expected = Vec3::new(0.7, -0.7, 0.0).normalize();
        assert!(
            aimed.distance(expected) < 1e-3,
            "shoulder should aim down-out, got {aimed:?}"
        );
    }

    /// The whole point of the no-regression guarantee: a skeleton fitted onto a
    /// mesh of its own pose keeps the template rotations exactly.
    #[test]
    fn a_matching_fit_leaves_the_rotations_untouched() {
        let parents = vec![None, Some(0), Some(1)];
        let positions = vec![
            Vec3::ZERO,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 2.0, 0.0),
        ];
        // Some non-trivial template rotations to prove they are preserved.
        let template_local = vec![
            Quat::from_rotation_z(0.3),
            Quat::from_rotation_x(0.5),
            Quat::from_rotation_y(0.2),
        ];

        let aim = vec![Some(1), Some(2), None];
        let local =
            pose_matched_local_rotations(&parents, &positions, &template_local, &positions, &aim);

        for (before, after) in template_local.iter().zip(&local) {
            assert!(
                before.abs_diff_eq(*after, 1e-4) || before.abs_diff_eq(-*after, 1e-4),
                "matching fit changed a rotation: {before:?} -> {after:?}"
            );
        }
    }

    /// A leaf bone follows its reoriented parent and contributes no aim of its
    /// own — it must not blow up when it has no child.
    #[test]
    fn a_leaf_keeps_its_local_rotation_relative_to_its_parent() {
        let parents = vec![None, Some(0)];
        let template_positions = vec![Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)];
        let template_local = vec![Quat::IDENTITY, Quat::from_rotation_z(0.4)];
        let fitted_positions = vec![Vec3::ZERO, Vec3::new(0.0, -1.0, 0.0)];

        let aim = vec![Some(1), None];
        let local = pose_matched_local_rotations(
            &parents,
            &template_positions,
            &template_local,
            &fitted_positions,
            &aim,
        );
        // The leaf (bone 1) keeps its template local rotation.
        assert!(local[1].abs_diff_eq(template_local[1], 1e-4));
    }
}
