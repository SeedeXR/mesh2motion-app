//! The FBX transform pipeline, checked against the legacy's own output.
//!
//! Every case here was produced by running three.js's `generateTransform` in
//! `legacy/bench/dump-transform-fixtures.ts` and recording its result. That is
//! the point: the conventions this code has to match (three.js Euler order
//! strings being the reverse of FBX's integers, and `'XYZ'` meaning the
//! literal product `Rx·Ry·Rz`) are invisible to inspection. A port that
//! inverts either one still produces smooth, invertible, plausible rotations.

use glam::{DMat4, DVec3};
use m2m_io::fbx::transform::{
    generate_transform, EulerOrder, InheritType, ParentTransform, TransformData,
};

const FIXTURE: &[u8] = include_bytes!("fixtures/fbx-transform.bin");

/// Fields per case: nine vec3s, three scalars, three 4x4 matrices.
const STRIDE: usize = 27 + 3 + 48;

struct Case {
    data: TransformData,
    parent: Option<ParentTransform>,
    expected: DMat4,
}

fn cases() -> Vec<Case> {
    let count = u32::from_le_bytes(FIXTURE[0..4].try_into().expect("header")) as usize;
    // The 4-byte pad after the count is what makes the f64 body 8-byte
    // aligned; without it the dumper could not have written a Float64Array.
    let body = &FIXTURE[8..];
    assert_eq!(
        body.len(),
        count * STRIDE * 8,
        "fixture size disagrees with its own header — regenerate it"
    );

    let f = |i: usize| f64::from_le_bytes(body[i * 8..i * 8 + 8].try_into().expect("f64"));

    (0..count)
        .map(|c| {
            let base = c * STRIDE;
            let v3 = |o: usize| DVec3::new(f(base + o), f(base + o + 1), f(base + o + 2));
            let m4 = |o: usize| {
                let mut a = [0.0f64; 16];
                for (k, slot) in a.iter_mut().enumerate() {
                    *slot = f(base + o + k);
                }
                DMat4::from_cols_array(&a)
            };
            // The fixture carries the FBX RotationOrder INTEGER, so reaching
            // the expected matrix requires our own mapping to be right. An
            // earlier version stored three.js's string index instead, which
            // bypassed `from_fbx` entirely and left it unverified.
            let order = EulerOrder::from_fbx(f(base + 27) as i64);

            Case {
                data: TransformData {
                    translation: v3(0),
                    pre_rotation: v3(3),
                    rotation: v3(6),
                    post_rotation: v3(9),
                    scale: v3(12),
                    scaling_offset: v3(15),
                    scaling_pivot: v3(18),
                    rotation_offset: v3(21),
                    rotation_pivot: v3(24),
                    euler_order: order,
                    inherit_type: InheritType::from_fbx(f(base + 28) as i64),
                },
                parent: (f(base + 29) != 0.0).then(|| ParentTransform {
                    local: m4(30),
                    world: m4(46),
                }),
                expected: m4(62),
            }
        })
        .collect()
}

/// Largest absolute component difference between two matrices.
fn deviation(a: DMat4, b: DMat4) -> f64 {
    a.to_cols_array()
        .iter()
        .zip(b.to_cols_array().iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f64::max)
}

#[test]
fn every_case_reproduces_the_legacy_matrix() {
    let cases = cases();
    // If the dumper ever silently writes nothing, the loop below passes
    // vacuously. It does not.
    assert_eq!(cases.len(), 49, "fixture case count");

    let mut worst = 0.0f64;
    let mut worst_case = 0;
    for (i, case) in cases.iter().enumerate() {
        let got = generate_transform(&case.data, case.parent)
            .unwrap_or_else(|| panic!("case {i}: parent should be invertible"));
        let d = deviation(got, case.expected);
        if d > worst {
            worst = d;
            worst_case = i;
        }
        assert!(
            d < 1e-9,
            "case {i} deviates by {d}\n  data: {:?}\n  parent: {:?}\n  got:      {:?}\n  expected: {:?}",
            case.data,
            case.parent.is_some(),
            got.to_cols_array(),
            case.expected.to_cols_array()
        );
    }
    // Translations here run to ±100, so this is a relative agreement of ~1e-16
    // — the two implementations are doing the same arithmetic, not merely
    // arriving somewhere similar.
    assert!(worst < 1e-9, "worst {worst} at case {worst_case}");
    eprintln!(
        "worst deviation {worst:e} at case {worst_case} of {}",
        cases.len()
    );
}

#[test]
fn the_fixture_actually_exercises_the_branches_it_claims_to() {
    // A fixture that quietly lost its interesting cases would leave every
    // assertion above passing on nothing but identities.
    let cases = cases();

    let with_parent = cases.iter().filter(|c| c.parent.is_some()).count();
    assert!(with_parent >= 15, "cases with a parent: {with_parent}");

    for inherit in [InheritType::RrSs, InheritType::RSrs, InheritType::Rrs] {
        let n = cases
            .iter()
            .filter(|c| c.data.inherit_type == inherit && c.parent.is_some())
            .count();
        assert!(n > 0, "no parented case uses {inherit:?}");
    }
    for order in [
        EulerOrder::Zyx,
        EulerOrder::Yzx,
        EulerOrder::Xzy,
        EulerOrder::Zxy,
        EulerOrder::Yxz,
        EulerOrder::Xyz,
    ] {
        let n = cases
            .iter()
            .filter(|c| c.data.euler_order == order && c.data.rotation != DVec3::ZERO)
            .count();
        assert!(n > 0, "no rotating case uses {order:?}");
    }
    // Pivots and offsets appear in no FBX in the corpus, so these cases are
    // their only coverage anywhere in the project.
    let pivoted = cases
        .iter()
        .filter(|c| c.data.rotation_pivot != DVec3::ZERO || c.data.scaling_pivot != DVec3::ZERO)
        .count();
    assert!(pivoted >= 10, "cases exercising pivots: {pivoted}");
    let post = cases
        .iter()
        .filter(|c| c.data.post_rotation != DVec3::ZERO)
        .count();
    assert!(post >= 10, "cases exercising PostRotation: {post}");
}

#[test]
fn the_three_inheritance_modes_actually_disagree() {
    // They differ only in where the parent's scale enters, so a uniformly
    // scaled parent makes all three identical -- and a test built on one would
    // pass with the match arms swapped.
    let parent = ParentTransform {
        local: DMat4::from_rotation_y(0.4) * DMat4::from_scale(DVec3::new(2.0, 3.0, 4.0)),
        world: DMat4::from_rotation_x(0.3)
            * DMat4::from_rotation_y(0.4)
            * DMat4::from_scale(DVec3::new(2.0, 3.0, 4.0)),
    };
    let base = TransformData {
        translation: DVec3::new(1.0, 2.0, 3.0),
        rotation: DVec3::new(10.0, 20.0, 30.0),
        scale: DVec3::new(1.5, 2.5, 3.5),
        ..Default::default()
    };

    let of = |inherit| {
        generate_transform(
            &TransformData {
                inherit_type: inherit,
                ..base
            },
            Some(parent),
        )
        .expect("invertible")
    };
    let (a, b, c) = (
        of(InheritType::RrSs),
        of(InheritType::RSrs),
        of(InheritType::Rrs),
    );

    assert!(
        deviation(a, b) > 0.1,
        "RrSs and RSrs agree: {}",
        deviation(a, b)
    );
    assert!(
        deviation(a, c) > 0.1,
        "RrSs and Rrs agree: {}",
        deviation(a, c)
    );
    assert!(
        deviation(b, c) > 0.1,
        "RSrs and Rrs agree: {}",
        deviation(b, c)
    );

    // And with a UNIFORMLY scaled parent they must coincide -- the property
    // that makes the assertions above meaningful rather than accidental.
    let uniform = ParentTransform {
        local: DMat4::from_rotation_y(0.4) * DMat4::from_scale(DVec3::splat(2.0)),
        world: DMat4::from_rotation_x(0.3)
            * DMat4::from_rotation_y(0.4)
            * DMat4::from_scale(DVec3::splat(2.0)),
    };
    let u = |inherit| {
        generate_transform(
            &TransformData {
                inherit_type: inherit,
                ..base
            },
            Some(uniform),
        )
        .expect("invertible")
    };
    assert!(deviation(u(InheritType::RrSs), u(InheritType::RSrs)) < 1e-12);
}

#[test]
fn a_parent_collapsed_to_zero_scale_is_reported_rather_than_returning_nan() {
    // glam's `inverse()` on a singular matrix returns NaN in every component
    // rather than panicking, and NaN would propagate through every descendant
    // of this node without anything to show where it started.
    let dead = ParentTransform {
        local: DMat4::ZERO,
        world: DMat4::ZERO,
    };
    assert!(generate_transform(&TransformData::default(), Some(dead)).is_none());

    // Premise: this is a real hazard in this glam build, not a hypothetical.
    assert!(!DMat4::ZERO.inverse().is_finite());

    // A parent whose WORLD matrix is fine but whose LOCAL scale is zero only
    // matters for Rrs, which is the one mode that divides by it.
    let flat = ParentTransform {
        local: DMat4::from_scale(DVec3::new(1.0, 0.0, 1.0)),
        world: DMat4::IDENTITY,
    };
    assert!(generate_transform(
        &TransformData {
            inherit_type: InheritType::Rrs,
            ..Default::default()
        },
        Some(flat)
    )
    .is_none());
    // The other two modes never touch the parent's local matrix.
    assert!(generate_transform(&TransformData::default(), Some(flat)).is_some());
}

#[test]
fn no_parent_and_an_identity_parent_are_the_same_thing() {
    let d = TransformData {
        translation: DVec3::new(3.0, -4.0, 5.0),
        rotation: DVec3::new(15.0, 25.0, 35.0),
        scale: DVec3::new(2.0, 2.0, 2.0),
        ..Default::default()
    };
    let none = generate_transform(&d, None).expect("no parent");
    let identity = generate_transform(
        &d,
        Some(ParentTransform {
            local: DMat4::IDENTITY,
            world: DMat4::IDENTITY,
        }),
    )
    .expect("identity parent");
    assert!(deviation(none, identity) < 1e-15);
}

#[test]
fn an_unrecognised_inherit_type_follows_the_legacy_into_rrs() {
    // The legacy branches `if (0) … else if (1) … else { Rrs }`, so every
    // value it does not name -- a corrupt 3, a negative, a vendor extension --
    // composes through Rrs. Falling back to the *default* instead would give
    // each child of such a node a different, finite, plausible matrix.
    assert_eq!(InheritType::from_fbx(0), InheritType::RrSs);
    assert_eq!(InheritType::from_fbx(1), InheritType::RSrs);
    for odd in [2, 3, 7, 99, -1] {
        assert_eq!(
            InheritType::from_fbx(odd),
            InheritType::Rrs,
            "InheritType {odd}"
        );
    }
    // And the three modes really are distinguishable, so this is not a
    // distinction without a difference.
    assert_ne!(InheritType::RrSs, InheritType::Rrs);
}

#[test]
fn the_quaternion_and_matrix_forms_of_an_euler_triple_agree() {
    // `euler_matrix` (used by the transform pipeline) and `axis_quats` (used by
    // the animation tracks) are each checked against three.js separately, so
    // they could drift apart and both fixtures would still pass — leaving a
    // rig whose rest pose and whose animation disagree about what a rotation
    // means, which reads as "the animation is subtly off" and is very hard to
    // trace back here.
    let orders = [
        EulerOrder::Zyx,
        EulerOrder::Yzx,
        EulerOrder::Xzy,
        EulerOrder::Zxy,
        EulerOrder::Yxz,
        EulerOrder::Xyz,
    ];
    let triples = [
        [0.0, 0.0, 0.0],
        [30.0, 45.0, 60.0],
        [-170.0, 20.0, 95.0],
        [12.5, -160.0, -33.25],
    ];

    let mut worst = 0.0f64;
    for order in orders {
        for t in triples {
            let [a, b, c] = order.axis_quats(t);
            let from_quat = DMat4::from_quat(a * b * c);
            // The pipeline's rotation term with everything else at identity.
            let from_matrix = generate_transform(
                &TransformData {
                    rotation: DVec3::from(t),
                    euler_order: order,
                    ..Default::default()
                },
                None,
            )
            .expect("no parent");
            let d = deviation(from_quat, from_matrix);
            worst = worst.max(d);
            assert!(d < 1e-12, "{order:?} {t:?} disagree by {d}");
        }
    }
    // Not vacuous: these triples produce genuinely different matrices.
    let [a, b, c] = EulerOrder::Zyx.axis_quats([30.0, 45.0, 60.0]);
    let [d, e, f] = EulerOrder::Xyz.axis_quats([30.0, 45.0, 60.0]);
    assert!(
        deviation(DMat4::from_quat(a * b * c), DMat4::from_quat(d * e * f)) > 0.1,
        "the orders should not agree on this triple"
    );
    eprintln!("worst quat/matrix disagreement {worst:e}");
}
