//! Placing a template skeleton onto a target mesh.
//!
//! # The division of labour
//!
//! The **template** knows the creature's plan — that a human's spine runs up
//! and a fox's runs forward, where the legs hang from, which end the head is.
//! The **mesh** knows this particular animal's proportions. Fitting combines
//! them; neither can do it alone.
//!
//! That sounds obvious and is easy to get wrong. The first instinct is to take
//! the body axis from the mesh's longest bounding-box extent, and **measuring
//! the nine shipped models shows that is wrong for four of them**: the human's
//! widest extent is its arm span (1.933 across against 1.830 tall), the bird's
//! and dragon's are their wingspans, and the spider's is its leg spread. Only
//! the quadrupeds and the fish are longest along the body. The template's own
//! rest pose answers the question the mesh cannot.
//!
//! # What this does today
//!
//! An initial placement: uniform scale plus translation, aligning the ground
//! plane and the symmetry plane, sized so the template's body reaches the same
//! proportion of the mesh that it does of its own. Per-chain refinement — the
//! part where posture and role matter — comes next, and this is the pose it
//! starts from.

use glam::Vec3;
use m2m_core::mesh::Mesh;
use m2m_core::voxel::{VoxelGrid, VoxelState};

/// glTF fixes +Y as up, so this is a property of the format rather than a guess
/// about the models. Every asset here is glTF.
pub const UP: Vec3 = Vec3::Y;

/// What a target mesh says about the animal it describes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Landmarks {
    /// Axis-aligned bounds.
    pub min: Vec3,
    /// Axis-aligned bounds.
    pub max: Vec3,
    /// Height of the lowest geometry: the plane the animal stands on.
    pub ground: f32,
    /// Where the body's midline sits on Z: the median depth of the vertices
    /// closest to the symmetry plane.
    ///
    /// **Not the bounding box's centre**, which is set by whatever sticks out
    /// furthest. Fitting the human rig onto `human-sophia.glb` with the
    /// bounding-box centre put the whole lower spine *behind* the body: her
    /// mesh reaches back to z = -0.549 where the torso at pelvis height only
    /// spans [-0.157, +0.135], so the box centre sat 0.18 behind the body it
    /// was meant to describe. Vertices near the symmetry plane are the ones
    /// actually on the midline, and their median ignores the outliers.
    pub medial_z: f32,
    /// Where the left-right symmetry plane sits on X.
    ///
    /// Taken as the midpoint of the X extent rather than fitted, because every
    /// shipped model is modelled about its own centre and a fitted plane on a
    /// symmetric mesh returns the midpoint anyway. A mesh that is genuinely
    /// asymmetric needs a real fit, and [`Landmarks::symmetry_error`] is what
    /// says whether this one is.
    pub symmetry_x: f32,
}

impl Landmarks {
    /// Measures a mesh. `None` when it has no vertices.
    pub fn of(mesh: &Mesh) -> Option<Self> {
        let (min, max) = mesh.bounds()?;
        let symmetry_x = (min.x + max.x) * 0.5;
        let half_width = (max.x - min.x) * 0.05;
        let mut midline: Vec<f32> = mesh
            .positions
            .iter()
            .filter(|p| (p.x - symmetry_x).abs() <= half_width)
            .map(|p| p.z)
            .collect();
        // Falls back to the box centre only when nothing sits near the plane,
        // which a mesh modelled off-axis could manage.
        let medial_z = if midline.is_empty() {
            (min.z + max.z) * 0.5
        } else {
            midline.sort_by(f32::total_cmp);
            midline[midline.len() / 2]
        };
        Some(Self {
            min,
            max,
            ground: min.y,
            symmetry_x,
            medial_z,
        })
    }

    /// Size along each axis.
    pub fn extent(&self) -> Vec3 {
        self.max - self.min
    }

    /// How badly the mesh disagrees with its own symmetry plane, as a fraction
    /// of its width.
    ///
    /// Each vertex is mirrored across the plane and matched to the nearest
    /// vertex on the other side; this is the mean of those distances. Zero for
    /// a perfectly symmetric mesh. It is a *report*, not a gate: an asymmetric
    /// animal is not a broken one, and the caller decides what to do about it.
    ///
    /// Costs a scan per vertex against a grid of the opposite side, so it is
    /// not something to call in a loop.
    pub fn symmetry_error(&self, mesh: &Mesh) -> f32 {
        let width = self.extent().x;
        if width <= f32::EPSILON || mesh.positions.is_empty() {
            return 0.0;
        }
        // Bucket the right side by (y, z) so the mirror lookup is local rather
        // than a scan of every vertex for every vertex.
        let cell = width * 0.02;
        let key = |p: Vec3| ((p.y / cell) as i32, (p.z / cell) as i32);
        let mut right: std::collections::HashMap<(i32, i32), Vec<Vec3>> =
            std::collections::HashMap::new();
        for &p in &mesh.positions {
            if p.x >= self.symmetry_x {
                right.entry(key(p)).or_default().push(p);
            }
        }

        let mut total = 0.0f32;
        let mut counted = 0usize;
        for &p in mesh.positions.iter().filter(|p| p.x < self.symmetry_x) {
            let mirrored = Vec3::new(2.0 * self.symmetry_x - p.x, p.y, p.z);
            let (ky, kz) = key(mirrored);
            let mut best = f32::INFINITY;
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &q in right.get(&(ky + dy, kz + dz)).into_iter().flatten() {
                        best = best.min(mirrored.distance_squared(q));
                    }
                }
            }
            if best.is_finite() {
                total += best.sqrt();
                counted += 1;
            }
        }
        if counted == 0 {
            return 0.0;
        }
        total / counted as f32 / width
    }
}

/// A template skeleton in its own rest pose: one world-space position per bone.
#[derive(Debug, Clone, PartialEq)]
pub struct RestPose {
    /// Bone names, in the order the positions follow.
    pub bones: Vec<String>,
    /// World-space rest position of each bone's head.
    pub positions: Vec<Vec3>,
}

impl RestPose {
    /// Axis-aligned bounds of the rest pose, or `None` when it has no bones.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let first = *self.positions.first()?;
        Some(
            self.positions
                .iter()
                .fold((first, first), |(lo, hi), &p| (lo.min(p), hi.max(p))),
        )
    }

    /// The rest position of a named bone.
    pub fn position_of(&self, bone: &str) -> Option<Vec3> {
        let index = self.bones.iter().position(|b| b == bone)?;
        self.positions.get(index).copied()
    }
}

/// A template skeleton placed onto a mesh.
#[derive(Debug, Clone, PartialEq)]
pub struct Fitted {
    /// Bone names, matching `positions`.
    pub bones: Vec<String>,
    /// World-space position of each bone after fitting.
    pub positions: Vec<Vec3>,
    /// The uniform scale applied to the template.
    pub scale: f32,
    /// The translation applied after scaling.
    pub offset: Vec3,
}

/// Which way a creature's body runs, taken from its template's spine.
///
/// This is the distinction that decides what the Z axis *means*. For an upright
/// creature Z is depth — front to back — and the body's midline is worth
/// measuring. For a creature on all fours, Z is body *length*, and the same
/// measurement slides the skeleton from nose to tail.
///
/// Getting this from the mesh is not possible: a human's widest extent is its
/// arm span and a bird's is its wingspan. The template knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyAxis {
    /// The spine runs mostly up — human, kaiju, spider.
    Upright,
    /// The spine runs mostly along the ground — fox, horse, shark, snake.
    Horizontal,
}

/// Reads the body axis off a template's spine in its rest pose.
///
/// `None` when the spine has fewer than two bones to give a direction, or when
/// its bones are missing from the rest pose.
pub fn body_axis(rest: &RestPose, spine: &[String]) -> Option<BodyAxis> {
    let first = rest.position_of(spine.first()?)?;
    let last = rest.position_of(spine.last()?)?;
    let direction = last - first;
    if direction.length_squared() <= f32::EPSILON {
        return None;
    }
    Some(if direction.y.abs() >= direction.z.abs() {
        BodyAxis::Upright
    } else {
        BodyAxis::Horizontal
    })
}

/// Places a template's rest pose onto a mesh: uniform scale, then translation.
///
/// Scale comes from **height alone**, not from the bounding box as a whole.
/// Width is set by how wide the arms or wings happen to be spread in the rest
/// pose, which says nothing about the animal — a T-posed human is far wider
/// than a human. Height is the one extent both the template and the mesh
/// measure the same way.
///
/// The result is aligned so the skeleton stands on the mesh's ground plane and
/// sits on its symmetry plane. Depth placement depends on `axis` — see
/// [`BodyAxis`]. Returns `None` if either input is empty or the template has no
/// height.
pub fn fit_uniform(rest: &RestPose, landmarks: &Landmarks, spine: &[String]) -> Option<Fitted> {
    let axis = body_axis(rest, spine)?;
    let (rest_min, rest_max) = rest.bounds()?;
    let rest_height = rest_max.y - rest_min.y;
    if rest_height <= f32::EPSILON {
        return None;
    }
    let scale = landmarks.extent().y / rest_height;

    // After scaling, put the skeleton's feet on the ground and its midline on
    // the symmetry plane. Depth is centred, having nothing better to go on
    // until per-chain fitting looks at the mesh.
    let scaled_centre_x = (rest_min.x + rest_max.x) * 0.5 * scale;

    // Both sides of the depth alignment must measure the same thing. The mesh
    // side is its midline; the template side must therefore be its *spine*, not
    // its bounding box — a box that includes arms reaching forward describes
    // the limbs, not the body. Aligning a box centre to a midline was wrong by
    // 0.027 on the base human, which was enough to push `human-female`'s
    // spine_01 out of her chest.
    let spine_z: Vec<f32> = spine
        .iter()
        .filter_map(|b| rest.position_of(b))
        .map(|p| p.z)
        .collect();
    let scaled_centre_z = match axis {
        BodyAxis::Upright if !spine_z.is_empty() => {
            spine_z.iter().sum::<f32>() / spine_z.len() as f32 * scale
        }
        _ => (rest_min.z + rest_max.z) * 0.5 * scale,
    };
    // Depth placement depends on what Z means for this creature. Upright: Z is
    // depth, so use the body's midline, which ignores hair and clothing that
    // drag the bounding box backwards. Horizontal: Z is length, and the midline
    // median would slide the skeleton along its own spine, so the box centre is
    // the right reference.
    let mesh_centre_z = match axis {
        BodyAxis::Upright => landmarks.medial_z,
        BodyAxis::Horizontal => (landmarks.min.z + landmarks.max.z) * 0.5,
    };
    let offset = Vec3::new(
        landmarks.symmetry_x - scaled_centre_x,
        landmarks.ground - rest_min.y * scale,
        mesh_centre_z - scaled_centre_z,
    );

    Some(Fitted {
        bones: rest.bones.clone(),
        positions: rest.positions.iter().map(|p| *p * scale + offset).collect(),
        scale,
        offset,
    })
}

impl Fitted {
    /// The fitted position of a named bone.
    pub fn position_of(&self, bone: &str) -> Option<Vec3> {
        let index = self.bones.iter().position(|b| b == bone)?;
        self.positions.get(index).copied()
    }
}

/// Places a template's skeleton onto a mesh: the whole pipeline, in one call.
///
/// # Why this exists
///
/// Fitting is four steps that must run in order and share state — a uniform
/// placement, a spine refinement, a limb swing and a per-joint pull. Every
/// caller so far has chained them by hand, and they did not agree: the report
/// example stops after `fit_uniform` and never voxelises, so the numbers it
/// printed were not the numbers the tests asserted. One entry point is one
/// order.
///
/// The spine comes from the **template**, never a hardcoded list of bone names:
/// a fox's spine bones are not a human's, and [`body_axis`] needs it to decide
/// what the Z axis even means for this creature.
///
/// Returns `None` when the mesh has no vertices, when the template's spine is
/// missing from the rest pose, or when the mesh cannot be voxelised.
///
/// # Cost
///
/// Dominated by voxelising the mesh at `resolution`. 128 is what the fitting
/// tests use across all nine creatures.
pub fn fit_template(
    template: &crate::template::Template,
    rest: &RestPose,
    mesh: &Mesh,
    resolution: u32,
) -> Option<Fitted> {
    let landmarks = Landmarks::of(mesh)?;
    let spine: Vec<String> = template
        .of_kind(crate::template::ChainKind::Spine)
        .flat_map(|chain| chain.bones.clone())
        .collect();

    let axis = body_axis(rest, &spine)?;
    let mut fitted = fit_uniform(rest, &landmarks, &spine)?;
    refine_spine(&mut fitted, mesh, &landmarks, &spine, axis);

    // Limb fitting and the spine's final containment correction both ask "is
    // this joint inside the mesh", which is a voxel query rather than a ray
    // cast per joint. One grid, shared.
    let grid = VoxelGrid::build(mesh, resolution)?;
    snap_spine_into_mesh(&mut fitted, &grid, &spine, axis);
    fit_limbs(&mut fitted, mesh, &grid, template);

    Some(fitted)
}

/// Where a named template bone's joint should sit on the mesh.
///
/// The input to the marker-placement pipeline — the Mixamo-style flow where a
/// person drops a handful of markers (chin, wrists, elbows, knees, groin) onto
/// their model and the rig solves to fit them. A marker naming a bone the
/// template does not carry is ignored, so one marker set can serve templates
/// that differ in which bones they have.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Marker {
    /// The template bone this marker pins.
    pub bone: String,
    /// Where that bone's joint should land, in mesh world space. An `[x, y, z]`
    /// array, not a `Vec3`, because this crosses IPC — the same reason
    /// [`Fitted`] positions do (`architecture.md` keeps glam out of the wire).
    pub position: [f32; 3],
}

/// Fits a template's skeleton to a mesh from user-placed markers.
///
/// # How it differs from [`fit_template`]
///
/// [`fit_template`] guesses joint positions from the mesh's shape. This is told
/// them: each marker is a joint a person placed. So the solve is not a search —
/// it is a placement that must honour the markers exactly and carry the bones
/// between and beyond them along smoothly.
///
/// # The solve
///
/// 1. A uniform **scale and translation** that best maps the marked bones' rest
///    positions onto their markers, least-squares. Rotation is not fitted — the
///    orient step has already faced the model forward, the same reason
///    [`fit_uniform`] scales uniformly and never rotates. Fewer than two markers
///    cannot fix a scale, so that returns `None`.
/// 2. Every bone is placed by that transform, then corrected by a per-bone
///    **delta**: a marked bone's delta lands it exactly on its marker; an
///    unmarked bone's delta is blended along its chain between the markers that
///    bracket it (and the chain's cross-chain parent, which anchors the end
///    nearest the body). A bone past the last marker on its chain rigid-follows
///    it — a finger carried by its hand, a foot by its knee.
///
/// Chains are walked so a chain's parent is always resolved first, which the
/// shipped rigs' parents-first order guarantees. Returns `None` when the markers
/// are degenerate (all on one point) or name fewer than two of the template's
/// bones.
pub fn fit_from_markers(
    template: &crate::template::Template,
    rest: &RestPose,
    parents: &[Option<usize>],
    markers: &[Marker],
    mesh: Option<&Mesh>,
) -> Option<Fitted> {
    let index_of: std::collections::HashMap<&str, usize> = rest
        .bones
        .iter()
        .enumerate()
        .map(|(i, b)| (b.as_str(), i))
        .collect();

    // Marked bones as (index, target). A marker for a bone this template does
    // not have is dropped rather than failing the whole solve.
    let marked: Vec<(usize, Vec3)> = markers
        .iter()
        .filter_map(|m| Some((*index_of.get(m.bone.as_str())?, Vec3::from(m.position))))
        .collect();
    if marked.len() < 2 {
        return None;
    }
    let marked_set: std::collections::HashSet<usize> = marked.iter().map(|&(i, _)| i).collect();

    // Uniform scale from the VERTICAL span of the marked joints, then translation
    // to line the centroids up. Height is the one axis a left/right mirror can't
    // flip and the reliable one for an upright figure (the same argument
    // `fit_uniform` makes) — a least-squares fit over all axes goes negative and
    // rejects the placement the moment a person marks the sides mirrored to the
    // template, which is a valid placement, not an error.
    let n = marked.len() as f32;
    let rest_mean = marked.iter().map(|&(i, _)| rest.positions[i]).sum::<Vec3>() / n;
    let target_mean = marked.iter().map(|&(_, t)| t).sum::<Vec3>() / n;
    let y_span = |lo_hi: (f32, f32)| lo_hi.1 - lo_hi.0;
    let rest_span = y_span(
        marked
            .iter()
            .map(|&(i, _)| rest.positions[i].y)
            .fold((f32::MAX, f32::MIN), |(lo, hi), y| (lo.min(y), hi.max(y))),
    );
    let target_span = y_span(
        marked
            .iter()
            .map(|&(_, t)| t.y)
            .fold((f32::MAX, f32::MIN), |(lo, hi), y| (lo.min(y), hi.max(y))),
    );
    if rest_span <= f32::EPSILON {
        return None;
    }
    let scale = target_span / rest_span;
    let offset = target_mean - rest_mean * scale;

    let transformed: Vec<Vec3> = rest.positions.iter().map(|p| *p * scale + offset).collect();

    // Per-bone correction. A marked bone's delta places it exactly; every other
    // bone blends between the markers along its chain.
    let marker_delta: std::collections::HashMap<usize, Vec3> = marked
        .iter()
        .map(|&(i, t)| (i, t - transformed[i]))
        .collect();
    let mut delta = vec![Vec3::ZERO; rest.bones.len()];

    let mut chains: Vec<&[String]> = template.chains.iter().map(|c| c.bones.as_slice()).collect();
    chains.sort_by_key(|bones| {
        bones
            .first()
            .and_then(|b| index_of.get(b.as_str()).copied())
            .unwrap_or(usize::MAX)
    });

    for bones in chains {
        let idxs: Vec<usize> = bones
            .iter()
            .filter_map(|b| index_of.get(b.as_str()).copied())
            .collect();
        if idxs.is_empty() {
            continue;
        }

        // Arc length along the chain, spaced by the template's own proportions.
        let mut arclen = vec![0.0f32; idxs.len()];
        for k in 1..idxs.len() {
            arclen[k] = arclen[k - 1] + transformed[idxs[k]].distance(transformed[idxs[k - 1]]);
        }

        // Control points to blend between: the cross-chain parent (already
        // resolved) as the anchor at the body end, then each marked bone. The
        // anchor sits one bone before the chain, at that bone's distance back.
        let parent = parents.get(idxs[0]).copied().flatten();
        let mut controls: Vec<(f32, Vec3)> = Vec::new();
        if let Some(p) = parent {
            let anchor_arclen = -transformed[idxs[0]].distance(transformed[p]);
            controls.push((anchor_arclen, delta[p]));
        }
        for (k, &i) in idxs.iter().enumerate() {
            if let Some(&d) = marker_delta.get(&i) {
                controls.push((arclen[k], d));
            }
        }

        for (k, &i) in idxs.iter().enumerate() {
            delta[i] = match marker_delta.get(&i) {
                Some(&d) => d,
                None => blend_deltas(&controls, arclen[k]),
            };
        }
    }

    let mut fitted = Fitted {
        bones: rest.bones.clone(),
        positions: transformed
            .iter()
            .zip(&delta)
            .map(|(p, d)| *p + *d)
            .collect(),
        scale,
        offset,
    };

    // Stage B: the mesh places what the markers didn't. Markers pin the knee; the
    // foot/ankle/toe below it are unmarked, so without the mesh they float or sink
    // (the reported feet/ankle failure). See docs/research/marker-skeleton-solving.md.
    if let Some(mesh) = mesh {
        ground_feet(&mut fitted, mesh, template, &marked_set);
    }
    Some(fitted)
}

/// Stands the unmarked feet on the mesh's ground plane.
///
/// The markers pin the knee; the foot, ankle and toe below it are unmarked and
/// otherwise just rigid-follow the knee, so a leg whose shin is a different length
/// than the template floats or sinks its foot. Here the mesh decides: the run of
/// bones below the deepest marked joint is scaled along itself, about that joint,
/// until the chain's lowest bone — its ground contact — sits on the mesh's lowest
/// surface. The marked joint never moves, and a leg with no marker is left alone.
fn ground_feet(
    fitted: &mut Fitted,
    mesh: &Mesh,
    template: &crate::template::Template,
    marked: &std::collections::HashSet<usize>,
) {
    let Some((mesh_min, _)) = mesh.bounds() else {
        return;
    };
    let floor = mesh_min.y;
    let index_of: std::collections::HashMap<&str, usize> = fitted
        .bones
        .iter()
        .enumerate()
        .map(|(i, b)| (b.as_str(), i))
        .collect();

    for chain in template.of_kind(crate::template::ChainKind::Limb) {
        if chain.role != Some(crate::template::LimbRole::Leg) {
            continue;
        }
        let idxs: Vec<usize> = chain
            .bones
            .iter()
            .filter_map(|b| index_of.get(b.as_str()).copied())
            .collect();
        // Anchor on the deepest marked joint of the leg (the knee); scale the run
        // below it. No marker on this leg means nothing to anchor to — leave it.
        let Some(anchor_at) = idxs.iter().rposition(|i| marked.contains(i)) else {
            continue;
        };
        let below = &idxs[anchor_at + 1..];
        if below.is_empty() {
            continue;
        }
        let anchor = fitted.positions[idxs[anchor_at]];
        // The ground contact is the lowest bone below the anchor.
        let toe = below
            .iter()
            .copied()
            .min_by(|&a, &b| fitted.positions[a].y.total_cmp(&fitted.positions[b].y))
            .expect("below is non-empty");
        let drop = anchor.y - fitted.positions[toe].y;
        if drop <= f32::EPSILON {
            continue; // the foot is not below the knee; nothing to stand on the floor
        }
        let scale = (anchor.y - floor) / drop;
        if !(scale.is_finite() && scale > 0.0) {
            continue;
        }
        for &i in below {
            fitted.positions[i] = anchor + (fitted.positions[i] - anchor) * scale;
        }
    }
}

/// Linearly blends the two control points bracketing `at`.
///
/// Clamps at both ends: before the first control it returns the first, after the
/// last it returns the last — which is how a chain tip past every marker comes to
/// rigid-follow the last one. Empty controls (a chain with no parent and no
/// marker) contribute no correction.
fn blend_deltas(controls: &[(f32, Vec3)], at: f32) -> Vec3 {
    let Some(&(first_x, first_d)) = controls.first() else {
        return Vec3::ZERO;
    };
    if at <= first_x {
        return first_d;
    }
    for pair in controls.windows(2) {
        let ((x0, d0), (x1, d1)) = (pair[0], pair[1]);
        if at <= x1 {
            let f = if x1 > x0 { (at - x0) / (x1 - x0) } else { 0.0 };
            return d0.lerp(d1, f);
        }
    }
    controls.last().map_or(Vec3::ZERO, |&(_, d)| d)
}

/// Moves each spine joint onto the mesh's midline **at its own height**.
///
/// [`fit_uniform`] gives the whole skeleton one depth, taken from the body's
/// overall midline. That is right on average and wrong in detail: a torso is
/// not a straight tube, and a chest sits further forward than a pelvis. On
/// `human-sintel` the single global depth left `spine_03` 0.031 outside her
/// chest — 1.7% of her height — with every other joint inside.
///
/// So each joint is re-placed against the slice of mesh at its own position
/// along the body axis: on the symmetry plane across, and on the median of that
/// slice in depth. A joint whose slice is empty keeps the placement it had,
/// since there is nothing to measure against.
///
/// **Upright bodies only.** The same idea applied to a horizontal body makes it
/// worse: a quadruped's backbone runs along the *top* of its torso, and the
/// median of a slice is dragged down by the legs hanging below it. Measured on
/// the fox, refining by slice median moved its whole spine from y 1.08 to 0.74
/// — a fall of 20% of its height — and pushed two joints out of a body they had
/// been inside. Until there is a statistic for "where a backbone sits
/// vertically" that is justified by measurement rather than plausible, a
/// horizontal body keeps its uniform placement, which puts every joint inside
/// on every horizontal creature tested.
///
/// Only the spine. Limbs leave the midline by definition and need their own
/// treatment — see [`ground_bone`].
pub fn refine_spine(
    fitted: &mut Fitted,
    mesh: &Mesh,
    landmarks: &Landmarks,
    spine: &[String],
    axis: BodyAxis,
) {
    let extent = landmarks.extent();
    // The slice is a band around the joint, and its width trades noise against
    // locality: too thin and a slice can be empty, too thick and it averages
    // away the very variation this exists to follow.
    //
    // Which way the band runs is the whole difference between the two axes. An
    // upright body is stacked along Y, so the slice is a horizontal band and
    // the joint is solved for depth. A horizontal body runs along Z, so the
    // slice is a cross-section and the joint is solved for height.
    let band = match axis {
        BodyAxis::Upright => extent.y,
        BodyAxis::Horizontal => extent.z,
    } * 0.04;
    if band <= f32::EPSILON {
        return;
    }
    let half_width = extent.x * 0.15;

    for bone in spine {
        let Some(index) = fitted.bones.iter().position(|b| b == bone) else {
            continue;
        };
        let at = fitted.positions[index];
        // Vertices near this joint along the body axis, and near the midline
        // across it. Off-midline vertices belong to limbs, not the body.
        let mut slice: Vec<Vec3> = mesh
            .positions
            .iter()
            .copied()
            .filter(|v| {
                let along = match axis {
                    BodyAxis::Upright => v.y - at.y,
                    BodyAxis::Horizontal => v.z - at.z,
                };
                along.abs() <= band && (v.x - landmarks.symmetry_x).abs() <= half_width
            })
            .collect();
        if slice.is_empty() {
            continue;
        }

        fitted.positions[index] = match axis {
            BodyAxis::Upright => {
                slice.sort_by(|a, b| a.z.total_cmp(&b.z));
                Vec3::new(landmarks.symmetry_x, at.y, slice[slice.len() / 2].z)
            }
            BodyAxis::Horizontal => {
                slice.sort_by(|a, b| a.y.total_cmp(&b.y));
                // Only a joint the body does not already contain is moved —
                // the same rule `fit_limb` states: something already inside is
                // already placed, and moving it can only take it out. A
                // quadruped's uniform fit is right; a long tapering tail's is
                // not, and this is what tells them apart.
                let (lo, hi) = (slice[0].y, slice[slice.len() - 1].y);
                let y = if at.y < lo || at.y > hi {
                    slice[slice.len() / 2].y
                } else {
                    at.y
                };
                Vec3::new(landmarks.symmetry_x, y, at.z)
            }
        };
    }
}

/// Nudges a spine joint the mesh does not contain onto the body, without moving
/// it along the body axis.
///
/// [`refine_spine`] places each joint on the mesh's midline, but a template
/// scaled uniformly onto a creature whose backbone tapers — a snake, a shark —
/// leaves a few tail joints just outside the surface. This is the containment
/// query [`refine_spine`] cannot make from a slice's bounding range: a point
/// inside a cross-section's y-extent can still be outside the body.
///
/// The correction keeps the joint's position **along the body** (its Z for a
/// horizontal creature) and searches that one cross-section for the nearest
/// interior voxel, moving the joint's X and Y onto it. Fixing Z is what makes
/// this safe: a joint is only ever pulled sideways onto the body it is level
/// with, never dragged back along the chain.
///
/// A joint whose cross-section holds no interior voxel is **left where it is**.
/// That is not a failure to place it — it is a joint past the end of the mesh's
/// tail, because the template's tail carries more bones than the shorter,
/// tapering mesh tail reaches. Cramming it inside would bunch the chain, which
/// is worse than a bone a little beyond a tapering tip. Measured: this takes the
/// snake from 6 spine joints outside to 5 and the shark from 5 to 3; the
/// remainder are past-the-tip and stay.
///
/// Upright creatures are untouched — [`refine_spine`] already puts every one of
/// their spine joints inside — so the axis is required and non-`Upright` only.
pub fn snap_spine_into_mesh(
    fitted: &mut Fitted,
    grid: &VoxelGrid,
    spine: &[String],
    axis: BodyAxis,
) {
    if axis != BodyAxis::Horizontal {
        return;
    }
    for bone in spine {
        let Some(index) = fitted.bones.iter().position(|b| b == bone) else {
            continue;
        };
        let at = fitted.positions[index];
        // Left where it is when already inside, or when its cross-section holds
        // no interior voxel — a joint past the end of the tail.
        if let Some(center) = nearest_interior_in_slab(grid, at) {
            fitted.positions[index] = Vec3::new(center.x, center.y, at.z);
        }
    }
}

/// The interior voxel nearest to `at` within `at`'s own Z cross-section, or
/// `None` when `at` is already inside or the cross-section is empty.
///
/// Fixing Z — searching only the one slab the joint is level with — is what
/// keeps [`snap_spine_into_mesh`] from ever dragging a joint along the body.
fn nearest_interior_in_slab(grid: &VoxelGrid, at: Vec3) -> Option<Vec3> {
    use m2m_core::voxel::VoxelState;
    let inside = |c: glam::IVec3| {
        matches!(
            grid.state(c),
            Some(VoxelState::Interior | VoxelState::Surface)
        )
    };

    let coord = grid.coord_of(at);
    if inside(coord) {
        return None;
    }
    let [nx, ny, _] = grid.dims();

    let mut best: Option<(f32, Vec3)> = None;
    for y in 0..ny as i32 {
        for x in 0..nx as i32 {
            let cell = glam::IVec3::new(x, y, coord.z);
            if !inside(cell) {
                continue;
            }
            let center = grid.center(cell);
            let planar = Vec3::new(center.x - at.x, center.y - at.y, 0.0).length();
            if best.is_none_or(|(d, _)| planar < d) {
                best = Some((planar, center));
            }
        }
    }
    best.map(|(_, center)| center)
}

/// Which bone of a limb chain touches the ground, measured from the template's
/// own rest pose.
///
/// **Measured, not derived from posture.** The first version of this picked a
/// bone by counting back from the end of the chain, on the reasoning that a
/// plantigrade sole contacts at the foot while an unguligrade hoof contacts at
/// the very tip. Checking the three rest poses says otherwise: the lowest bone
/// is the **last** bone in all of them — `ball_leaf_l` at y 0.012 for the human,
/// `Back_Leg_Tip_L` at 0.027 for the fox, `back_leg_leaf_l` at 0.011 for the
/// horse. The templates are authored standing on the ground, so the ground
/// contact is a thing to look up rather than infer.
///
/// What posture *does* describe is how high the ankle rides, which is a real
/// difference and a large one — see [`ankle_height`].
///
/// `None` for a chain with no bones, or whose bones are absent from the rest
/// pose.
pub fn ground_bone<'a>(chain: &'a crate::template::Chain, rest: &RestPose) -> Option<&'a str> {
    chain
        .bones
        .iter()
        .filter_map(|b| rest.position_of(b).map(|p| (b.as_str(), p.y)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(bone, _)| bone)
}

/// How high a limb's ankle rides, as a fraction of the rest pose's height.
///
/// This is what separates the three postures, and the separation is wide:
/// measured on the shipped templates, a plantigrade human's ankle sits at 6% of
/// its height, a digitigrade fox's at 21%, an unguligrade horse's at 32%. That
/// is the geometry a fitter has to preserve when it puts a leg on a new mesh —
/// grounding the toe is common to all three, but *where the ankle ends up* is
/// not.
///
/// The ankle is taken as the chain's highest bone that still sits in the lower
/// half of the limb's own span, which is what "the joint above the foot" means
/// without depending on how many bones a species puts in its foot.
///
/// `None` when the chain has no posture, or too few bones in the rest pose.
pub fn ankle_height(chain: &crate::template::Chain, rest: &RestPose) -> Option<f32> {
    chain.posture?;
    let heights: Vec<(usize, f32)> = chain
        .bones
        .iter()
        .enumerate()
        .filter_map(|(i, b)| rest.position_of(b).map(|p| (i, p.y)))
        .collect();
    if heights.len() < 3 {
        return None;
    }
    let (top, bottom) = (
        heights
            .iter()
            .map(|(_, y)| *y)
            .fold(f32::NEG_INFINITY, f32::max),
        heights
            .iter()
            .map(|(_, y)| *y)
            .fold(f32::INFINITY, f32::min),
    );
    let span = top - bottom;
    if span <= f32::EPSILON {
        return None;
    }
    let midpoint = bottom + span * 0.5;
    let ankle = heights
        .iter()
        .filter(|(_, y)| *y <= midpoint)
        .map(|(_, y)| *y)
        .fold(f32::NEG_INFINITY, f32::max);
    ankle.is_finite().then_some(ankle)
}

/// Swings a limb chain onto the limb the mesh actually has.
///
/// The body fit scales the whole skeleton uniformly, which keeps a limb's
/// length in proportion and its **direction** exactly as the template posed it.
/// That is the problem: `rig-human` is T-posed with its arms straight out, and
/// a mesh whose arms hang lower is a different pose, not a different size.
/// Measured before writing this, the body fit alone leaves `upperarm`,
/// `lowerarm` and `hand` outside the mesh on **every** human body tested, while
/// the clavicle — the one arm bone close to the torso — stays inside. The
/// quadrupeds are unaffected: fox 0 of 26 limb joints outside, horse 0 of 28,
/// because their rest pose already matches their meshes.
///
/// So the chain is rotated about its attachment to point at the far end of the
/// limb the mesh has, and scaled to reach it. The attachment itself does not
/// move: it is held by the body, and the body fit already placed it.
///
/// The far end is the mesh vertex furthest from the attachment **within a cone**
/// about the template's own direction. The cone matters: without it a left arm
/// would happily pick the right foot, which is further away.
///
/// Does nothing when the mesh offers no vertex in the cone, when the chain has
/// fewer than two bones, or when the template's own limb has no length.
pub fn fit_limb(
    fitted: &mut Fitted,
    mesh: &Mesh,
    grid: &VoxelGrid,
    chain: &crate::template::Chain,
) {
    let (Some(first), Some(last)) = (chain.bones.first(), chain.bones.last()) else {
        return;
    };

    // A limb already inside the mesh is already placed, and re-aiming it can
    // only move it out. Measured: the fox and horse rest poses match their own
    // meshes exactly — 0 of 26 and 0 of 28 limb joints outside — and swinging
    // them anyway put 6 and 5 joints outside a body they had been inside. The
    // human arms are the opposite case: the rig is T-posed and every mesh has
    // its arms lower, so `upperarm`, `lowerarm` and `hand` all start outside.
    let already_placed = chain.bones.iter().all(|bone| {
        fitted.position_of(bone).is_some_and(|at| {
            matches!(
                grid.state(grid.coord_of(at)),
                Some(VoxelState::Interior | VoxelState::Surface)
            )
        })
    });
    if already_placed {
        return;
    }
    let (Some(attach), Some(tip)) = (fitted.position_of(first), fitted.position_of(last)) else {
        return;
    };
    let reach = tip - attach;
    if reach.length_squared() <= f32::EPSILON {
        return;
    }
    let direction = reach.normalize();

    // Keep a lateral limb on its own side of the body. A leg's cone points
    // nearly straight down, so `along` (distance along the axis) barely depends
    // on X and the left/right foot tie is broken by vertex order — which aimed
    // both legs at the *same* foot. Rejecting vertices across the sagittal plane
    // from the limb's root fixes it, and self-disables for a limb rooted on the
    // centreline (where `attach_side` is ~0, so the product is never negative).
    let center_x = mesh
        .bounds()
        .map_or(attach.x, |(min, max)| 0.5 * (min.x + max.x));
    let attach_side = attach.x - center_x;

    // Half-angle of the cone. Wide enough to follow an arm from a T-pose down
    // to an A-pose, narrow enough that it cannot cross to the other side of the
    // body or pick up a limb it does not own.
    const COS_LIMIT: f32 = 0.85; // about 32 degrees
    let mut best = None;
    let mut best_reach = 0.0f32;
    for &vertex in &mesh.positions {
        let offset = vertex - attach;
        let distance = offset.length();
        if distance <= f32::EPSILON {
            continue;
        }
        let along = offset.dot(direction);
        if along / distance < COS_LIMIT {
            continue;
        }
        if (vertex.x - center_x) * attach_side < 0.0 {
            continue; // across the body from this limb's root — a different limb
        }
        // Ranked by how far along the limb's own axis a vertex lies, not by how
        // far away it is. Distance picks whatever is furthest anywhere in the
        // cone, which for a leg pointing down is a point diagonally across the
        // body.
        if along > best_reach {
            best_reach = along;
            best = Some(vertex);
        }
    }
    let Some(target) = best else {
        return;
    };

    let wanted = target - attach;
    if wanted.length_squared() <= f32::EPSILON {
        return;
    }
    let rotation = glam::Quat::from_rotation_arc(direction, wanted.normalize());
    let scale = wanted.length() / reach.length();

    for bone in &chain.bones {
        let Some(index) = fitted.bones.iter().position(|b| b == bone) else {
            continue;
        };
        let offset = fitted.positions[index] - attach;
        fitted.positions[index] = attach + rotation * (offset * scale);
    }
}

/// Swings every limb of a template onto the mesh.
///
/// Legs, arms, wings and fins alike: the operation is about following the
/// geometry that is there, and does not depend on posture. Posture governs the
/// proportions *within* a leg, which uniform scaling already preserves — the
/// measured leg tips sit 0.5% to 2.4% above the rig's own floor before and
/// after, because the body fit maps the rig's floor onto the mesh's ground.
pub fn fit_limbs(
    fitted: &mut Fitted,
    mesh: &Mesh,
    grid: &VoxelGrid,
    template: &crate::template::Template,
) {
    for chain in template.of_kind(crate::template::ChainKind::Limb) {
        fit_limb(fitted, mesh, grid, chain);
        refine_limb_joints(fitted, mesh, grid, chain);
    }
}

/// Pulls a limb's intermediate joints onto the middle of the limb the mesh has.
///
/// [`fit_limb`] rotates a chain rigidly, so it gets the limb's **direction and
/// reach** right and keeps the template's straight-line arrangement of joints
/// along the way. A real arm is not straight — it bends at the elbow, and a mesh
/// modelled in an A-pose bends differently from a T-posed template. That is what
/// the joints still outside the mesh are: `upperarm` and `lowerarm` on every
/// human body, while the clavicle at one end and the hand at the other are fine.
///
/// The legacy has no algorithm to borrow here. Its `RigModelVariations.ts`
/// carries a hand-authored `expandArms` angle per model, only `bunny` sets one
/// (-30 degrees), and it is used to pose the marketing page rather than to rig
/// anything. Notably `bunny` is also the worst body measured here, so the number
/// was real — it was just entered by a person.
///
/// A joint that is outside is moved to the centroid of the mesh near it,
/// gathered within a radius of the joint so the torso cannot pull an upper arm
/// inwards. A joint already inside is left where it is — a centroid is the
/// middle of the *nearby mesh*, which is not where a joint belongs when the
/// template already had it right.
///
/// **Endpoints are included**, which the first version of this did not do. The
/// reasoning for excluding them was that an attachment belongs to the body and a
/// tip was just aimed at the end of the limb, and it is wrong: on `human-jay`
/// and `human-bunny` the endpoints were the *only* joints still outside, 2 and 4
/// of them, and including them takes both bodies to zero. The guard already
/// protects the good cases — an attachment the body fit placed correctly and a
/// tip aimed at a surface vertex are both inside, so neither moves.
pub fn refine_limb_joints(
    fitted: &mut Fitted,
    mesh: &Mesh,
    grid: &VoxelGrid,
    chain: &crate::template::Chain,
) {
    if chain.bones.len() < 3 {
        return;
    }
    let (Some(first), Some(last)) = (chain.bones.first(), chain.bones.last()) else {
        return;
    };
    let (Some(attach), Some(tip)) = (fitted.position_of(first), fitted.position_of(last)) else {
        return;
    };
    let reach = (tip - attach).length();
    if reach <= f32::EPSILON {
        return;
    }
    // Wide enough to contain a limb's cross-section, tight enough that the
    // torso is not swept up along with the shoulder.
    let radius = reach * 0.25;

    for bone in &chain.bones {
        let Some(index) = fitted.bones.iter().position(|b| b == bone) else {
            continue;
        };
        let at = fitted.positions[index];
        // Only a joint that is actually outside gets moved. Measured: pulling
        // every intermediate joint to a local centroid helps the bodies that
        // need it (bunny 8 -> 4 joints outside, sophia 3 -> 1, bird 4 -> 0) and
        // hurts the ones that do not (fox 0 -> 5, horse 0 -> 6, human 5 -> 6),
        // because a centroid is the middle of the *mesh nearby*, which is not
        // where a joint belongs when the template already had it right.
        if matches!(
            grid.state(grid.coord_of(at)),
            Some(VoxelState::Interior | VoxelState::Surface)
        ) {
            continue;
        }
        let mut sum = Vec3::ZERO;
        let mut count = 0usize;
        for &vertex in &mesh.positions {
            if vertex.distance_squared(at) <= radius * radius {
                sum += vertex;
                count += 1;
            }
        }
        if count > 0 {
            fitted.positions[index] = sum / count as f32;
        }
    }
}
