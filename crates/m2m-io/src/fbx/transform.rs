//! The FBX local-transform pipeline.
//!
//! FBX does not store a node's local matrix. It stores nine separate
//! components — translation, three rotations, scale, and two pivot/offset
//! pairs — that must be composed in a fixed order, then reconciled with how
//! the node inherits its parent's rotation and scale. Autodesk documents the
//! composition; this mirrors it, by way of the legacy's `generateTransform`.
//!
//! # Why this is checked against fixtures rather than reasoned about
//!
//! Two conventions here are easy to invert and impossible to notice by eye:
//!
//! - three.js Euler order strings are the **reverse** of FBX's extrinsic order
//!   integers. FBX order `0` is XYZ-extrinsic, which three.js calls `'ZYX'`.
//! - three.js composes order `'XYZ'` as the literal product `Rx · Ry · Rz`
//!   (verified against `three.core.js`, `Matrix4.makeRotationFromEuler`).
//!
//! Get either backwards and the rotations are still smooth, still invertible,
//! and wrong. So `tests/fbx_transform.rs` asserts this against 49 cases dumped
//! from the legacy's own `generateTransform` (`legacy/bench/dump-transform-fixtures.ts`),
//! including the pivots and Euler orders that no file in the corpus carries.

use glam::{DMat3, DMat4, DVec3};

/// Euler rotation order, named as three.js names it.
///
/// The name says the order the axis rotations are **multiplied** in: `Xyz` is
/// `Rx · Ry · Rz`. FBX files store an extrinsic order integer instead, and the
/// two conventions are reverses of one another — see [`Self::from_fbx`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EulerOrder {
    /// FBX order 0, XYZ-extrinsic. The default when a Model says nothing.
    #[default]
    Zyx,
    /// FBX order 1, XZY-extrinsic.
    Yzx,
    /// FBX order 2, YZX-extrinsic.
    Xzy,
    /// FBX order 3, YXZ-extrinsic.
    Zxy,
    /// FBX order 4, ZXY-extrinsic.
    Yxz,
    /// FBX order 5, ZYX-extrinsic.
    Xyz,
}

/// Which axis a single rotation turns about.
#[derive(Debug, Clone, Copy)]
enum Axis {
    X,
    Y,
    Z,
}

impl EulerOrder {
    /// Maps an FBX `RotationOrder` integer.
    ///
    /// Order 6 is Spherical XYZ, which is not an Euler triple at all and
    /// cannot be represented; the legacy warns and falls back to order 0, so
    /// this does the same rather than inventing a different wrong answer.
    pub fn from_fbx(order: i64) -> Self {
        match order {
            1 => Self::Yzx,
            2 => Self::Xzy,
            3 => Self::Zxy,
            4 => Self::Yxz,
            5 => Self::Xyz,
            _ => Self::Zyx,
        }
    }

    /// The axes in multiplication order.
    fn axes(self) -> [Axis; 3] {
        match self {
            Self::Zyx => [Axis::Z, Axis::Y, Axis::X],
            Self::Yzx => [Axis::Y, Axis::Z, Axis::X],
            Self::Xzy => [Axis::X, Axis::Z, Axis::Y],
            Self::Zxy => [Axis::Z, Axis::X, Axis::Y],
            Self::Yxz => [Axis::Y, Axis::X, Axis::Z],
            Self::Xyz => [Axis::X, Axis::Y, Axis::Z],
        }
    }
}

/// How a node inherits its parent's rotation and scale.
///
/// The three modes differ only in where the parent's scale enters the product,
/// so they agree exactly whenever the parent is uniformly scaled — which is
/// why a test needs a parent with non-uniform scale to tell them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InheritType {
    /// `RrSs` (FBX 0): parent rotation, then local rotation, then both scales.
    #[default]
    RrSs,
    /// `RSrs` (FBX 1): both parent transforms first, then the local ones.
    RSrs,
    /// `Rrs` (FBX 2): the parent's **local** scale is excluded.
    Rrs,
}

impl InheritType {
    /// Maps an FBX `InheritType` integer.
    ///
    /// Only 0 and 1 are named; **everything else becomes [`Self::Rrs`]**,
    /// including values FBX does not define. That is not a guess — the legacy
    /// branches `if (inheritType === 0) … else if (=== 1) … else { Rrs }`, so
    /// a corrupt or vendor-specific 3 composes through `Rrs` there. Defaulting
    /// to `RrSs` instead would give every child of such a node a different,
    /// perfectly finite, plausible-looking matrix.
    ///
    /// A **missing** property is a separate question and is not this
    /// function's: the legacy reads absent as 0, which `TransformData`'s
    /// `Default` also does.
    pub fn from_fbx(value: i64) -> Self {
        match value {
            0 => Self::RrSs,
            1 => Self::RSrs,
            _ => Self::Rrs,
        }
    }
}

/// The parent's matrices, as the child needs them.
#[derive(Debug, Clone, Copy)]
pub struct ParentTransform {
    /// The parent's own local matrix. Only [`InheritType::Rrs`] reads it.
    pub local: DMat4,
    /// The parent's world matrix.
    pub world: DMat4,
}

/// One Model's transform components, straight from its `Properties70`.
///
/// Angles are **degrees**, as FBX stores them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransformData {
    /// `Lcl_Translation`.
    pub translation: DVec3,
    /// `PreRotation`, applied before [`Self::rotation`]. Maya writes joint
    /// orientation here, which is why it always uses the default Euler order.
    pub pre_rotation: DVec3,
    /// `Lcl_Rotation`. The only rotation [`Self::euler_order`] applies to.
    pub rotation: DVec3,
    /// `PostRotation`, applied after [`Self::rotation`] — and **inverted**,
    /// which is the one term in the pipeline that is not used as written.
    pub post_rotation: DVec3,
    /// `Lcl_Scaling`. Defaults to one, not zero.
    pub scale: DVec3,
    /// `ScalingOffset`, a translation applied with the scale.
    pub scaling_offset: DVec3,
    /// `ScalingPivot`, the point scaling happens about; undone afterwards.
    pub scaling_pivot: DVec3,
    /// `RotationOffset`, a translation applied with the rotation.
    pub rotation_offset: DVec3,
    /// `RotationPivot`, the point rotation happens about; undone afterwards.
    pub rotation_pivot: DVec3,
    /// `RotationOrder`, governing [`Self::rotation`] only.
    pub euler_order: EulerOrder,
    /// `InheritType`, governing how the parent's scale reaches this node.
    pub inherit_type: InheritType,
}

impl Default for TransformData {
    fn default() -> Self {
        Self {
            translation: DVec3::ZERO,
            pre_rotation: DVec3::ZERO,
            rotation: DVec3::ZERO,
            post_rotation: DVec3::ZERO,
            // The one component whose identity is not zero.
            scale: DVec3::ONE,
            scaling_offset: DVec3::ZERO,
            scaling_pivot: DVec3::ZERO,
            rotation_offset: DVec3::ZERO,
            rotation_pivot: DVec3::ZERO,
            euler_order: EulerOrder::Zyx,
            inherit_type: InheritType::RrSs,
        }
    }
}

/// A rotation about one axis, in radians.
fn axis_rotation(axis: Axis, radians: f64) -> DMat4 {
    match axis {
        Axis::X => DMat4::from_rotation_x(radians),
        Axis::Y => DMat4::from_rotation_y(radians),
        Axis::Z => DMat4::from_rotation_z(radians),
    }
}

/// Builds a rotation from Euler angles in **degrees**.
///
/// The order names the multiplication sequence, but each angle always belongs
/// to its own axis: the x component turns about X whether it is applied first
/// or last.
fn euler_matrix(degrees: DVec3, order: EulerOrder) -> DMat4 {
    let angle = |axis: Axis| {
        match axis {
            Axis::X => degrees.x,
            Axis::Y => degrees.y,
            Axis::Z => degrees.z,
        }
        .to_radians()
    };
    let [a, b, c] = order.axes();
    axis_rotation(a, angle(a)) * axis_rotation(b, angle(b)) * axis_rotation(c, angle(c))
}

/// three.js `Matrix4.copyPosition` applied to an identity: the translation
/// alone, with rotation and scale discarded.
fn translation_only(m: DMat4) -> DMat4 {
    DMat4::from_translation(m.w_axis.truncate())
}

/// three.js `Matrix4.extractRotation`: each basis column normalised to unit
/// length, which strips scale but leaves shear.
///
/// Returns the identity for a degenerate matrix, matching three.js's
/// `determinantAffine() === 0` guard — and, unlike a bare normalisation, never
/// divides by zero.
fn extract_rotation(m: DMat4) -> DMat4 {
    if DMat3::from_mat4(m).determinant() == 0.0 {
        return DMat4::IDENTITY;
    }
    DMat4::from_cols(
        m.x_axis.truncate().normalize().extend(0.0),
        m.y_axis.truncate().normalize().extend(0.0),
        m.z_axis.truncate().normalize().extend(0.0),
        glam::DVec4::W,
    )
}

/// three.js `Vector3.setFromMatrixScale`: the column lengths.
///
/// Unsigned, so a mirrored matrix reports positive scale — three.js only
/// recovers the sign in `decompose`, and the FBX pipeline does not call it.
fn matrix_scale(m: DMat4) -> DVec3 {
    DVec3::new(
        m.x_axis.truncate().length(),
        m.y_axis.truncate().length(),
        m.z_axis.truncate().length(),
    )
}

/// Composes a node's local matrix, relative to its parent.
///
/// `None` when the parent's world matrix cannot be inverted — a parent
/// collapsed to zero scale. The legacy produces an all-zero matrix there
/// (three.js `invert()` zeroes a singular matrix), which silently collapses
/// the node and everything under it; reporting it is the deliberate
/// divergence, since `glam`'s `inverse()` would hand back NaN instead.
pub fn generate_transform(d: &TransformData, parent: Option<ParentTransform>) -> Option<DMat4> {
    let (parent_local, parent_world) = match parent {
        Some(p) => (p.local, p.world),
        None => (DMat4::IDENTITY, DMat4::IDENTITY),
    };
    if DMat3::from_mat4(parent_world).determinant() == 0.0 {
        return None;
    }

    let translation_m = DMat4::from_translation(d.translation);
    // Pre- and post-rotation always use the DEFAULT order, even when the node
    // declares another: a node's RotationOrder governs Lcl_Rotation only. Maya
    // writes joint orientation into PreRotation and depends on this.
    let pre_rotation_m = euler_matrix(d.pre_rotation, EulerOrder::default());
    let rotation_m = euler_matrix(d.rotation, d.euler_order);
    let post_rotation_m = euler_matrix(d.post_rotation, EulerOrder::default()).inverse();
    let scaling_m = DMat4::from_scale(d.scale);

    let scaling_offset_m = DMat4::from_translation(d.scaling_offset);
    let scaling_pivot_m = DMat4::from_translation(d.scaling_pivot);
    let rotation_offset_m = DMat4::from_translation(d.rotation_offset);
    let rotation_pivot_m = DMat4::from_translation(d.rotation_pivot);

    let local_rotation = pre_rotation_m * rotation_m * post_rotation_m;

    // Split the parent's world matrix into rotation and scale/shear, so the
    // inheritance mode can reassemble them in its own order.
    let parent_rotation = extract_rotation(parent_world);
    let parent_translation = translation_only(parent_world);
    let parent_rot_scale = parent_translation.inverse() * parent_world;
    let parent_scale = parent_rotation.inverse() * parent_rot_scale;

    let global_rs = match d.inherit_type {
        InheritType::RrSs => parent_rotation * local_rotation * parent_scale * scaling_m,
        InheritType::RSrs => parent_rotation * parent_scale * local_rotation * scaling_m,
        InheritType::Rrs => {
            let parent_local_scale = DMat4::from_scale(matrix_scale(parent_local));
            // A zero-scale parent makes this uninvertible on its own, even
            // when the world matrix was fine.
            if parent_local_scale.determinant() == 0.0 {
                return None;
            }
            let without_local = parent_scale * parent_local_scale.inverse();
            parent_rotation * local_rotation * without_local * scaling_m
        }
    };

    // The pivot/offset sandwich: rotate about the rotation pivot, scale about
    // the scaling pivot, each undone afterwards.
    let local = translation_m
        * rotation_offset_m
        * rotation_pivot_m
        * pre_rotation_m
        * rotation_m
        * post_rotation_m
        * rotation_pivot_m.inverse()
        * scaling_offset_m
        * scaling_pivot_m
        * scaling_m
        * scaling_pivot_m.inverse();

    // Position comes from the full pivot-aware product, orientation and scale
    // from the inheritance-aware one; then back into the parent's space.
    let global_translation = translation_only(parent_world * translation_only(local));
    Some(parent_world.inverse() * (global_translation * global_rs))
}
