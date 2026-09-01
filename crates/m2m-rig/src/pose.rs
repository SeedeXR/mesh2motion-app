//! Humanoid arm-pose detection.
//!
//! A character is usually modelled with its arms out (**T-pose**) or angled down
//! and out (**A-pose**), and a clip authored for one sits wrong on the other
//! (see the retargeter). The app must know which it has, and never guess
//! silently — so the pose is read from where the arms actually landed after
//! fitting, and reported.
//!
//! The measure is one angle: how far the shoulder→wrist vector drops below the
//! horizontal. Straight out is 0°, straight down is 90°, A-pose sits between.

use crate::fit::Fitted;
use glam::Vec3;

/// A humanoid's arm pose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pose {
    /// Arms out roughly horizontal — Mixamo's rest pose.
    TPose,
    /// Arms angled down and out — the pose most characters are modelled in.
    APose,
    /// Arms hanging near-vertical.
    ArmsDown,
    /// None of the above: arms raised, only one arm, or bones that are not there.
    Other,
}

impl Pose {
    /// A stable kebab-case label for the UI and IPC.
    pub fn label(self) -> &'static str {
        match self {
            Pose::TPose => "t-pose",
            Pose::APose => "a-pose",
            Pose::ArmsDown => "arms-down",
            Pose::Other => "other",
        }
    }
}

/// Thresholds in degrees below horizontal, chosen so the middle of A-pose (~45°)
/// sits well inside its band and the boundaries fall in the gaps real characters
/// leave between poses.
const TPOSE_MAX_DROP: f32 = 20.0;
const APOSE_MAX_DROP: f32 = 65.0;
/// Arms raised more than this above horizontal are not a rest pose we handle.
const RAISED_LIMIT: f32 = -20.0;

/// How far a shoulder→wrist vector drops below horizontal, in degrees.
///
/// `0` is straight out, `+90` straight down, negative is raised above the
/// horizontal. `None` when the arm has no length or the up axis is unusable.
fn arm_drop_degrees(shoulder: Vec3, wrist: Vec3, up: Vec3) -> Option<f32> {
    let arm = wrist - shoulder;
    let up = up.normalize_or_zero();
    if arm.length() < 1e-4 || up.length() < 0.5 {
        return None;
    }
    let drop = (-arm.normalize().dot(up)).clamp(-1.0, 1.0);
    Some(drop.asin().to_degrees())
}

/// Classifies a drop angle into a [`Pose`].
fn classify_drop(degrees: f32) -> Pose {
    if degrees < RAISED_LIMIT {
        Pose::Other
    } else if degrees < TPOSE_MAX_DROP {
        Pose::TPose
    } else if degrees < APOSE_MAX_DROP {
        Pose::APose
    } else {
        Pose::ArmsDown
    }
}

/// Classifies the pose from one shoulder→wrist vector and the world up axis.
pub fn detect_pose(shoulder: Vec3, wrist: Vec3, up: Vec3) -> Pose {
    match arm_drop_degrees(shoulder, wrist, up) {
        Some(degrees) => classify_drop(degrees),
        None => Pose::Other,
    }
}

/// The pose of a fitted humanoid skeleton, from the average of its two arms.
///
/// Reads the standard human arm bones (`upperarm_*`, `hand_*`). Returns
/// [`Pose::Other`] when neither arm is present — a non-human template, or a rig
/// that names its bones differently.
pub fn pose_of_fitted(fitted: &Fitted, up: Vec3) -> Pose {
    let arm = |shoulder: &str, wrist: &str| -> Option<f32> {
        arm_drop_degrees(
            fitted.position_of(shoulder)?,
            fitted.position_of(wrist)?,
            up,
        )
    };
    let drop = match (arm("upperarm_l", "hand_l"), arm("upperarm_r", "hand_r")) {
        (Some(l), Some(r)) => (l + r) / 2.0,
        (Some(one), None) | (None, Some(one)) => one,
        (None, None) => return Pose::Other,
    };
    classify_drop(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    const UP: Vec3 = Vec3::Y;

    #[test]
    fn arms_straight_out_are_a_t_pose() {
        let shoulder = Vec3::new(0.2, 1.5, 0.0);
        let wrist = Vec3::new(0.8, 1.5, 0.0); // level with the shoulder
        assert_eq!(detect_pose(shoulder, wrist, UP), Pose::TPose);
    }

    #[test]
    fn arms_down_and_out_are_an_a_pose() {
        let shoulder = Vec3::new(0.2, 1.5, 0.0);
        // 45° down and out: equal horizontal reach and vertical drop.
        let wrist = Vec3::new(0.6, 1.1, 0.0);
        assert_eq!(detect_pose(shoulder, wrist, UP), Pose::APose);
    }

    #[test]
    fn arms_hanging_are_arms_down() {
        let shoulder = Vec3::new(0.2, 1.5, 0.0);
        let wrist = Vec3::new(0.22, 1.0, 0.0); // almost straight down
        assert_eq!(detect_pose(shoulder, wrist, UP), Pose::ArmsDown);
    }

    #[test]
    fn raised_arms_are_neither() {
        let shoulder = Vec3::new(0.2, 1.5, 0.0);
        let wrist = Vec3::new(0.6, 2.0, 0.0); // up and out
        assert_eq!(detect_pose(shoulder, wrist, UP), Pose::Other);
    }

    #[test]
    fn a_zero_length_arm_is_not_classified() {
        let p = Vec3::new(0.2, 1.5, 0.0);
        assert_eq!(detect_pose(p, p, UP), Pose::Other);
    }

    #[test]
    fn the_boundary_between_t_and_a_is_where_it_says() {
        let shoulder = Vec3::ZERO;
        // Just under 20° drop is still a T; just over is an A.
        let just_t = Vec3::new(1.0, -(19.0f32.to_radians().tan()), 0.0);
        let just_a = Vec3::new(1.0, -(21.0f32.to_radians().tan()), 0.0);
        assert_eq!(detect_pose(shoulder, just_t, UP), Pose::TPose);
        assert_eq!(detect_pose(shoulder, just_a, UP), Pose::APose);
    }
}
