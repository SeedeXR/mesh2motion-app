//! Models and the bone hierarchy.
//!
//! Ported from `parseModels`/`parseScene` in
//! `legacy/src/lib/io/fbx/FBXTreeParser.ts`. An FBX `Model` is any node in the
//! scene graph — a bone (`LimbNode`), a mesh, a null. This builds the tree from
//! the connection graph and composes every node's local and world matrix.
//!
//! Transform composition itself lives in [`crate::fbx::transform`], which is
//! checked against the legacy's output directly.
//!
//! # What the reference corpus does and does not exercise
//!
//! Measured over 8 Mixamo exports, 522 Models: 520 `LimbNode` and 2 `Mesh`;
//! `Lcl_Translation` on 520, `PreRotation` on 440, `Lcl_Rotation` on 274,
//! `InheritType` on 67 (values 1 and 2 only — the other 455 default to 0).
//! **No file carries `PostRotation`, any pivot or offset, or `RotationOrder`.**
//! Those paths exist because Maya and 3ds Max exports use them and dropping
//! them would silently misplace every node; their coverage is synthetic, in
//! `tests/fbx_transform.rs`.

use crate::fbx::dom::{Object, Scene};
use crate::fbx::transform::{
    generate_transform, EulerOrder, InheritType, ParentTransform, TransformData,
};
use glam::{DMat4, DVec3};
use std::collections::HashMap;

/// What building the hierarchy had to drop or repair.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelReport {
    /// Models naming more than one parent Model.
    ///
    /// The last connection wins, because three.js's `Object3D.add` detaches a
    /// child from its previous parent — so the legacy resolves it the same way
    /// by accident of the API. Matched deliberately, and counted, because the
    /// choice is arbitrary and a file relying on it is already malformed.
    pub multiple_parents: usize,
    /// Parent links cut because following them led back to the same Model.
    ///
    /// A cycle is unreachable from any root, so without this those Models get
    /// no transform at all — and a naive walk would not terminate.
    pub cycles_broken: usize,
    /// Models whose local transform could not be composed.
    ///
    /// Left as the identity. Happens when an ancestor collapsed to zero scale,
    /// making its world matrix uninvertible.
    pub transforms_defaulted: usize,
}

/// One node of the FBX scene graph.
#[derive(Debug, Clone)]
pub struct Model {
    /// The Model object's id.
    pub id: i64,
    /// Name with the `Model::` prefix stripped, e.g. `mixamorig:Hips`.
    pub name: String,
    /// `LimbNode` for a bone, `Mesh`, `Null`, `Root`, `LimbNode`…
    pub subclass: String,
    /// The raw components this node's matrix was composed from.
    pub transform: TransformData,
    /// Local matrix, relative to the parent.
    pub local: DMat4,
    /// World matrix.
    pub world: DMat4,
    /// Parent Model, if any.
    pub parent: Option<i64>,
    /// Child Models, in ascending id order.
    pub children: Vec<i64>,
}

impl Model {
    /// Whether this node is a bone.
    ///
    /// `LimbNode` **or** `Root`: the legacy builds a `Bone` for both
    /// (`FBXTreeParser.ts`, `case 'LimbNode': case 'Root':`). Every file in
    /// the reference corpus uses `LimbNode`, so nothing measured here can see
    /// the difference — but a 3ds Max Biped export names its skeleton root
    /// `Root`, and treating it as a non-bone would silently drop the root
    /// joint from anything filtering on this, leaving a skeleton short by one
    /// bone with no error anywhere.
    ///
    /// The legacy also treats any Model with a skin cluster as a bone
    /// regardless of subclass; that needs deformer knowledge and belongs to
    /// the rig layer, not here.
    pub fn is_bone(&self) -> bool {
        self.subclass == "LimbNode" || self.subclass == "Root"
    }
}

/// The scene's Models, with the hierarchy resolved.
#[derive(Debug, Clone)]
pub struct ModelTree {
    /// Every Model, in ascending id order.
    pub models: Vec<Model>,
    /// Models with no parent Model, in ascending id order.
    pub roots: Vec<i64>,
    /// What had to be dropped or repaired.
    pub report: ModelReport,
    index: HashMap<i64, usize>,
}

impl ModelTree {
    /// Looks a Model up by id.
    pub fn get(&self, id: i64) -> Option<&Model> {
        self.index.get(&id).map(|&i| &self.models[i])
    }

    /// Ids from `id` up to its root, `id` first. Empty if `id` is unknown.
    ///
    /// [`parse_all`] cuts every cycle, so on a tree it built this always ends
    /// at a root. The step limit is for the case that guarantee does not
    /// cover: [`Self::models`] is public, so a caller can reintroduce a loop,
    /// and an unbounded walk would hang rather than return a wrong answer.
    /// Hanging is the worse failure — it gives nothing to debug.
    pub fn ancestors(&self, id: i64) -> Vec<i64> {
        let mut out = Vec::new();
        let mut cur = Some(id);
        while let Some(c) = cur {
            let Some(model) = self.get(c) else { break };
            out.push(c);
            if out.len() > self.models.len() {
                break;
            }
            cur = model.parent;
        }
        out
    }
}

/// Reads a vec3 property, or its default when absent.
fn vec3(object: &Object, name: &str, default: DVec3) -> DVec3 {
    object
        .property(name)
        .and_then(|p| p.as_vec3())
        .map(DVec3::from)
        .unwrap_or(default)
}

/// Reads one Model's transform components.
fn transform_data(object: &Object) -> TransformData {
    TransformData {
        translation: vec3(object, "Lcl_Translation", DVec3::ZERO),
        pre_rotation: vec3(object, "PreRotation", DVec3::ZERO),
        rotation: vec3(object, "Lcl_Rotation", DVec3::ZERO),
        post_rotation: vec3(object, "PostRotation", DVec3::ZERO),
        scale: vec3(object, "Lcl_Scaling", DVec3::ONE),
        scaling_offset: vec3(object, "ScalingOffset", DVec3::ZERO),
        scaling_pivot: vec3(object, "ScalingPivot", DVec3::ZERO),
        rotation_offset: vec3(object, "RotationOffset", DVec3::ZERO),
        rotation_pivot: vec3(object, "RotationPivot", DVec3::ZERO),
        euler_order: object
            .property("RotationOrder")
            .and_then(|p| p.as_i64())
            .map(EulerOrder::from_fbx)
            .unwrap_or_default(),
        inherit_type: object
            .property("InheritType")
            .and_then(|p| p.as_i64())
            .map(InheritType::from_fbx)
            .unwrap_or_default(),
    }
}

/// Builds the Model hierarchy and composes every node's matrices.
pub fn parse_all(scene: &Scene) -> ModelTree {
    let objects = scene.objects_of_kind("Model");
    let index: HashMap<i64, usize> = objects.iter().enumerate().map(|(i, o)| (o.id, i)).collect();

    let mut report = ModelReport::default();
    let mut models: Vec<Model> = objects
        .iter()
        .map(|o| Model {
            id: o.id,
            name: o.name.clone(),
            subclass: o.subclass.clone(),
            transform: transform_data(o),
            local: DMat4::IDENTITY,
            world: DMat4::IDENTITY,
            parent: None,
            children: Vec::new(),
        })
        .collect();

    // Parent links. A Model may legally name several parents in the connection
    // graph; three.js keeps only the last, so this does too.
    for (i, o) in objects.iter().enumerate() {
        let parents = scene.parents_of(o.id, Some("Model"));
        if parents.len() > 1 {
            report.multiple_parents += 1;
        }
        models[i].parent = parents.last().copied();
    }

    break_cycles(&mut models, &index, &mut report);

    // Children in ascending id order: a child's position in this list becomes
    // a bone index downstream, so it cannot depend on connection order.
    for i in 0..models.len() {
        if let Some(parent) = models[i].parent {
            let id = models[i].id;
            if let Some(&p) = index.get(&parent) {
                models[p].children.push(id);
            }
        }
    }

    let roots: Vec<i64> = models
        .iter()
        .filter(|m| m.parent.is_none())
        .map(|m| m.id)
        .collect();

    compose(&mut models, &index, &roots, &mut report);

    ModelTree {
        models,
        roots,
        report,
        index,
    }
}

/// Cuts any parent link that closes a loop.
///
/// Each Model has at most one parent, so the graph is functional and a single
/// walk per node finds every cycle. Nodes already proven acyclic are not
/// rewalked, which keeps this linear rather than quadratic.
fn break_cycles(models: &mut [Model], index: &HashMap<i64, usize>, report: &mut ModelReport) {
    const UNVISITED: u8 = 0;
    const IN_PROGRESS: u8 = 1;
    const SETTLED: u8 = 2;

    let mut state = vec![UNVISITED; models.len()];
    let mut path = Vec::new();

    for start in 0..models.len() {
        if state[start] != UNVISITED {
            continue;
        }
        path.clear();
        let mut cursor = Some(start);
        while let Some(i) = cursor {
            if state[i] == IN_PROGRESS {
                // Reached a node already on this walk: cut its parent link,
                // which turns it into a root and breaks the loop.
                models[i].parent = None;
                report.cycles_broken += 1;
                break;
            }
            if state[i] == SETTLED {
                break;
            }
            state[i] = IN_PROGRESS;
            path.push(i);
            cursor = models[i].parent.and_then(|p| index.get(&p).copied());
        }
        for &i in &path {
            state[i] = SETTLED;
        }
    }
}

/// Composes local and world matrices, parents before children.
fn compose(
    models: &mut [Model],
    index: &HashMap<i64, usize>,
    roots: &[i64],
    report: &mut ModelReport,
) {
    // An explicit stack rather than recursion: depth is bounded only by the
    // file, and a deep chain in a hostile file must not blow the native stack.
    let mut stack: Vec<(usize, Option<ParentTransform>)> = roots
        .iter()
        .filter_map(|id| index.get(id).map(|&i| (i, None)))
        .collect();

    while let Some((i, parent)) = stack.pop() {
        let local = match generate_transform(&models[i].transform, parent) {
            Some(m) => m,
            None => {
                report.transforms_defaulted += 1;
                DMat4::IDENTITY
            }
        };
        let world = parent.map_or(local, |p| p.world * local);
        models[i].local = local;
        models[i].world = world;

        let inherited = ParentTransform { local, world };
        for child in models[i].children.clone() {
            if let Some(&c) = index.get(&child) {
                stack.push((c, Some(inherited)));
            }
        }
    }
}
