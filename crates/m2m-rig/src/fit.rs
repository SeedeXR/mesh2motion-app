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
