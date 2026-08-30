//! Skin clusters on a real rig: weights, bind matrices, and the remap onto
//! expanded geometry.

use glam::{DMat4, DVec4};
use m2m_core::skinning::MAX_INFLUENCES;
use m2m_io::fbx::binary::FbxProperty;
use m2m_io::fbx::geometry::{self, GeometricTransform};
use m2m_io::fbx::skin::{self, Skin};
use m2m_io::fbx::{binary, dom::Scene, text};
use std::collections::HashMap;

const MIXAMO: &[u8] =
    include_bytes!("../../../legacy/static/test-files/retarget testing/mixamo-original-rig.fbx");

fn scene() -> Scene {
    Scene::from_document(binary::parse(MIXAMO).expect("parses"))
}

#[test]
fn reads_both_skins_with_their_clusters() {
    let scene = scene();
    let (skins, _) = skin::parse_all(&scene);
    assert_eq!(skins.len(), 2);

    // Measured from the file: 65 clusters on Beta_Surface, 64 on Beta_Joints,
    // each skin bound to exactly one geometry.
    let by_geometry: HashMap<i64, &Skin> = skins.iter().map(|s| (s.geometry_id, s)).collect();
    let geometry_id = |name: &str| {
        scene
            .objects_of_kind("Geometry")
            .into_iter()
            .find(|g| g.name == name)
            .unwrap_or_else(|| panic!("{name}"))
            .id
    };

    for (name, clusters) in [("Beta_Surface", 65usize), ("Beta_Joints", 64)] {
        let s = by_geometry
            .get(&geometry_id(name))
            .unwrap_or_else(|| panic!("no skin for {name}"));
        assert_eq!(s.clusters.len(), clusters, "{name} clusters");
        assert_eq!(s.report, skin::SkinReport::default(), "{name} report");

        // Every cluster names a distinct bone. A duplicate would mean two
        // cluster indices collapsing onto one bone downstream.
        let bones = s.bone_ids();
        let unique: std::collections::BTreeSet<i64> = bones.iter().copied().collect();
        assert_eq!(unique.len(), bones.len(), "{name} has duplicate bones");

        // And every one of them is a Model that exists.
        for id in bones {
            let bone = scene.object(id).unwrap_or_else(|| panic!("bone {id}"));
            assert_eq!(bone.kind, "Model");
        }
    }
}

#[test]
fn cluster_weights_match_the_raw_arrays() {
    // Checked against the FBX node itself rather than against the parser's own
    // output, so a parser that invented or reordered values cannot agree.
    let scene = scene();
    let (skins, _) = skin::parse_all(&scene);

    let mut checked = 0usize;
    for s in &skins {
        for cluster in &s.clusters {
            let node = &scene.object(cluster.id).expect("cluster object").node;
            // 40 of the 129 clusters carry no Indexes node at all — every one
            // of them a finger bone (Pinky1, Thumb4, Index1, Index4 and so on).
            // A bone can be bound to the skin while influencing no vertices on
            // that mesh, and the legacy guards for it the same way
            // (`if ('Indexes' in boneNode)`). Treated as an empty influence
            // set, not an error.
            let raw_indices = node
                .child("Indexes")
                .and_then(|n| n.properties.first())
                .and_then(FbxProperty::as_i64_vec)
                .unwrap_or_default();
            let raw_weights = node
                .child("Weights")
                .and_then(|n| n.properties.first())
                .and_then(FbxProperty::as_f64_vec)
                .unwrap_or_default();

            assert_eq!(
                cluster.indices,
                raw_indices.iter().map(|&i| i as u32).collect::<Vec<_>>(),
                "cluster {} indices",
                cluster.id
            );
            assert_eq!(
                cluster.weights, raw_weights,
                "cluster {} weights",
                cluster.id
            );
            checked += cluster.indices.len();
        }
    }
    // Measured total across both skins: 15467 + 11278.
    assert_eq!(checked, 26_745, "index/weight pairs");

    // And the empty clusters are real structure worth pinning: a change that
    // started dropping them would shift every later bone index.
    let empty = skins
        .iter()
        .flat_map(|s| s.clusters.iter())
        .filter(|c| c.indices.is_empty())
        .count();
    assert_eq!(empty, 40, "clusters that influence no vertices");
}

#[test]
fn the_bind_matrix_returns_a_vertex_to_its_world_position() {
    // The check that the composition is the right way round. At bind time the
    // bone's matrix IS TransformLink, so
    //   TransformLink · inverse_bind · v  ==  Transform · v
    // for any v. If the two matrices were composed in the other order, or one
    // were dropped, this identity fails.
    let scene = scene();
    let (skins, _) = skin::parse_all(&scene);

    let probes = [
        DVec4::new(0.0, 0.0, 0.0, 1.0),
        DVec4::new(13.0, -47.0, 5.5, 1.0),
        DVec4::new(-100.0, 100.0, 100.0, 1.0),
    ];

    let mut checked = 0usize;
    for s in &skins {
        for cluster in &s.clusters {
            let composed = cluster.transform_link * cluster.inverse_bind();
            for v in probes {
                let got = composed * v;
                let want = cluster.transform * v;
                assert!(
                    (got - want).abs().max_element() < 1e-6,
                    "cluster {}: {got:?} != {want:?}",
                    cluster.id
                );
            }
            checked += 1;
        }
    }
    assert_eq!(checked, 129);
}

#[test]
fn bind_matrices_place_bones_at_human_scale() {
    // Physical sanity. TransformLink is the bone's world transform at bind
    // time, so its translation is where the bone sits — in the centimetres
    // Mixamo exports. A transposed or mis-strided matrix read lands nowhere
    // near a human.
    let scene = scene();
    let (skins, _) = skin::parse_all(&scene);

    let mut heights = Vec::new();
    for s in &skins {
        for cluster in &s.clusters {
            let t = cluster.transform_link.w_axis;
            assert!(t.x.is_finite() && t.y.is_finite() && t.z.is_finite());
            heights.push(t.y);
        }
    }
    let lo = heights.iter().cloned().fold(f64::MAX, f64::min);
    let hi = heights.iter().cloned().fold(f64::MIN, f64::max);
    assert!(
        (-5.0..30.0).contains(&lo) && (140.0..200.0).contains(&hi),
        "bone heights span {lo:.1}..{hi:.1} cm, which is not a standing human"
    );
}

#[test]
fn binding_produces_valid_weights_matching_the_source_influences() {
    let scene = scene();
    let (skins, _) = skin::parse_all(&scene);

    for s in &skins {
        let geometry = scene.object(s.geometry_id).expect("geometry");
        let mesh = geometry::parse(geometry, GeometricTransform::default()).expect("geometry");
        let (weights, report) = s.bind(&mesh).expect("binds");

        assert_eq!(weights.vertex_count(), mesh.vertex_count());
        assert_eq!(
            weights.first_unnormalised(1e-4),
            None,
            "{}: weights are not a valid normalised set",
            geometry.name
        );
        assert!(
            weights.fallback_vertices.is_empty(),
            "{}: every corner is weighted in this file",
            geometry.name
        );
        assert_eq!(
            report.vertices_over_influence_limit, 0,
            "{}: max 3 influences measured",
            geometry.name
        );

        // Rebuild the expected influences straight from the cluster arrays and
        // compare per corner. This is the assertion that fails if the remap
        // through vertex_source is wrong, if weights are misnormalised, or if
        // bone indices are off by one.
        let mut expected: HashMap<u32, Vec<(u16, f64)>> = HashMap::new();
        for (bone, cluster) in s.clusters.iter().enumerate() {
            for (&v, &w) in cluster.indices.iter().zip(cluster.weights.iter()) {
                if w > 0.0 {
                    expected.entry(v).or_default().push((bone as u16, w));
                }
            }
        }

        let mut compared = 0usize;
        for corner in 0..mesh.vertex_count() {
            let source = mesh.vertex_source[corner];
            let mut want: Vec<(u16, f64)> = expected.get(&source).cloned().unwrap_or_default();
            let total: f64 = want.iter().map(|(_, w)| w).sum();
            for e in want.iter_mut() {
                e.1 /= total;
            }
            want.sort_by_key(|(b, _)| *b);

            let mut got: Vec<(u16, f64)> = weights
                .influences(corner)
                .map(|(b, w)| (b, w as f64))
                .collect();
            got.sort_by_key(|(b, _)| *b);

            assert_eq!(got.len(), want.len(), "corner {corner} influence count");
            for ((gb, gw), (wb, ww)) in got.iter().zip(want.iter()) {
                assert_eq!(gb, wb, "corner {corner} bone");
                assert!(
                    (gw - ww).abs() < 1e-4,
                    "corner {corner} weight {gw} vs {ww}"
                );
            }
            compared += 1;
        }
        assert_eq!(compared, mesh.vertex_count());
    }
}

#[test]
fn a_skin_bound_to_the_wrong_mesh_is_rejected() {
    // Vertex counts do not identify a mesh. The earlier version of this test
    // passed only because the two reference meshes happen to differ in size --
    // it would have missed a skin binding to a same-sized stranger, which is
    // the case that produces a fully-weighted, completely wrong deformation.
    use m2m_io::fbx::geometry::MeshGeometry;
    let scene = scene();
    let (skins, _) = skin::parse_all(&scene);
    let skin = &skins[0];

    let mine = geometry::parse(
        scene.object(skin.geometry_id).expect("geometry"),
        GeometricTransform::default(),
    )
    .expect("geometry parses");

    // Identical in every respect except which object it came from. Nothing
    // about its shape can reveal the mismatch.
    let impostor = MeshGeometry {
        id: skin.geometry_id + 1,
        ..mine.clone()
    };

    assert!(skin.bind(&mine).is_ok(), "its own mesh binds");
    assert!(
        skin.bind(&impostor).is_err(),
        "a same-sized mesh from a different object must still be rejected"
    );

    // The other skin's mesh differs in size too, so the vertex-range check
    // stays exercised as well.
    let other = geometry::parse(
        scene.object(skins[1].geometry_id).expect("geometry"),
        GeometricTransform::default(),
    )
    .expect("geometry parses");
    assert!(skin.bind(&other).is_err(), "a different mesh is rejected");
}

#[test]
fn every_cluster_in_the_reference_rig_has_real_bind_matrices() {
    // This is the premise the module's bind-matrix argument rests on, not a
    // test of the fallback: it says the identity default is never what a good
    // file produces. `bind_matrices_that_are_unusable_are_counted_not_silent`
    // is what covers the fallback path itself.
    let scene = scene();
    let (skins, _) = skin::parse_all(&scene);
    // Every real cluster has both matrices, so none should be identity here;
    // that is what makes the fallback path a fallback rather than the norm.
    for s in &skins {
        for c in &s.clusters {
            assert_ne!(c.transform_link, DMat4::IDENTITY, "cluster {}", c.id);
            assert_ne!(c.transform, DMat4::IDENTITY, "cluster {}", c.id);
        }
    }
}

#[test]
fn influence_limit_is_enforced_when_a_vertex_is_over_weighted() {
    // Synthetic, because the reference rig maxes out at 3 influences and never
    // exercises the truncation path.
    use m2m_io::fbx::geometry::MeshGeometry;

    let mesh = MeshGeometry {
        id: 2,
        positions: vec![0.0; 3],
        indices: vec![0],
        normals: None,
        uvs: None,
        vertex_source: vec![0],
        source_vertex_count: 1,
        report: Default::default(),
    };

    // Six clusters all claiming vertex 0, with descending weights.
    let clusters: Vec<skin::Cluster> = (0..6)
        .map(|i| skin::Cluster {
            id: i as i64,
            bone_id: 100 + i as i64,
            indices: vec![0],
            weights: vec![1.0 - i as f64 * 0.1],
            transform_link: DMat4::IDENTITY,
            transform: DMat4::IDENTITY,
        })
        .collect();
    let s = Skin {
        id: 1,
        geometry_id: 2,
        clusters,
        report: Default::default(),
    };

    let (weights, report) = s.bind(&mesh).expect("binds");
    assert_eq!(report.vertices_over_influence_limit, 1);
    assert_eq!(weights.influences(0).count(), MAX_INFLUENCES);
    assert_eq!(
        weights.first_unnormalised(1e-5),
        None,
        "renormalised after truncation"
    );

    // The four kept must be the four strongest: bones 100..103.
    let mut kept: Vec<u16> = weights.influences(0).map(|(b, _)| b).collect();
    kept.sort();
    assert_eq!(kept, vec![0, 1, 2, 3], "the strongest four clusters");
}

#[test]
fn a_weight_too_small_to_survive_f32_is_treated_as_no_weight() {
    // The failure this guards is quiet: a weight that is positive as an f64
    // and zero as an f32 leaves the vertex holding an influence worth nothing.
    // Normalising by a zero total would then keep every weight at zero, and
    // the vertex would detach at animation time with nothing in the report to
    // say so. It must land in the fallback instead, and be counted.
    //
    // Two guards in `bind` each produce this outcome on their own -- narrowing
    // before the positivity test, and refusing to normalise by a zero total --
    // so removing either one alone leaves this test passing. It pins the
    // BEHAVIOUR, not one implementation of it; only removing both breaks it.
    // `a_weight_too_small_is_not_stored_as_an_influence` is what pins the
    // first guard specifically.
    use m2m_io::fbx::geometry::MeshGeometry;

    let mesh = MeshGeometry {
        id: 2,
        positions: vec![0.0; 6],
        indices: vec![0, 1],
        normals: None,
        uvs: None,
        vertex_source: vec![0, 1],
        source_vertex_count: 2,
        report: Default::default(),
    };

    // Vertex 0 gets a weight that underflows f32; vertex 1 a usable one.
    // 1e-300 is representable as an f64 and rounds to exactly 0.0 as an f32.
    let s = Skin {
        id: 1,
        geometry_id: 2,
        clusters: vec![skin::Cluster {
            id: 0,
            bone_id: 100,
            indices: vec![0, 1],
            weights: vec![1e-300, 0.5],
            transform_link: DMat4::IDENTITY,
            transform: DMat4::IDENTITY,
        }],
        report: Default::default(),
    };

    let (weights, _report) = s.bind(&mesh).expect("binds");

    assert_eq!(1e-300_f64 as f32, 0.0, "premise: the weight underflows");

    assert_eq!(
        weights.fallback_vertices,
        vec![0],
        "vertex 0 must be pinned by the fallback, not left at zero"
    );

    // The fallback pins it to bone 0 with FULL weight -- the point is that it
    // stays attached, so the weight must be 1.0 and not the 0.0 that silently
    // dropping it would leave behind.
    let v0: Vec<(u16, f32)> = weights.influences(0).collect();
    assert_eq!(v0, vec![(0, 1.0)], "vertex 0 fully pinned");

    // Vertex 1 is untouched by any of this and still normalises to 1.
    let v1: Vec<(u16, f32)> = weights.influences(1).collect();
    assert_eq!(v1, vec![(0, 1.0)], "vertex 1 keeps its real influence");
    assert_eq!(weights.first_unnormalised(1e-5), None);
}

#[test]
fn a_weight_too_small_is_not_stored_as_an_influence() {
    // Distinct from the all-underflow case: here the vertex has a real
    // influence too, so it is never unweighted and never reaches the fallback.
    // The underflowed weight must simply not be admitted -- storing it would
    // occupy an influence slot with a bone that contributes nothing.
    use m2m_io::fbx::geometry::MeshGeometry;

    let mesh = MeshGeometry {
        id: 2,
        positions: vec![0.0; 3],
        indices: vec![0],
        normals: None,
        uvs: None,
        vertex_source: vec![0],
        source_vertex_count: 1,
        report: Default::default(),
    };

    // Two clusters on vertex 0: one real, one that vanishes in f32.
    let cluster = |id: i64, weight: f64| skin::Cluster {
        id,
        bone_id: 100 + id,
        indices: vec![0],
        weights: vec![weight],
        transform_link: DMat4::IDENTITY,
        transform: DMat4::IDENTITY,
    };
    let s = Skin {
        id: 1,
        geometry_id: 2,
        clusters: vec![cluster(0, 0.5), cluster(1, 1e-300)],
        report: Default::default(),
    };

    let (weights, _report) = s.bind(&mesh).expect("binds");

    assert!(
        weights.fallback_vertices.is_empty(),
        "the vertex has a real weight, so it needs no fallback"
    );

    // Assert on the RAW slots, not on `influences()`: that iterator filters
    // out zero-weight entries, so it reports the same thing whether or not the
    // underflowed influence was stored. The difference is only visible here --
    // admitting it writes cluster 1's bone into slot 1.
    assert_eq!(
        &weights.indices[..MAX_INFLUENCES],
        &[0, 0, 0, 0],
        "the underflowed cluster must not occupy a slot"
    );
    assert_eq!(
        &weights.weights[..MAX_INFLUENCES],
        &[1.0, 0.0, 0.0, 0.0],
        "all the weight on the one real influence"
    );
}

#[test]
fn influences_whose_total_overflows_fall_back_instead_of_zeroing() {
    // The other way a vertex loses all its weight. Each of these weights is
    // finite in f32, so they pass admission, but their SUM saturates to
    // infinity -- and dividing by infinity would set every weight to exactly
    // zero, detaching the vertex with a report that claims it was fine.
    use m2m_io::fbx::geometry::MeshGeometry;

    let mesh = MeshGeometry {
        id: 2,
        positions: vec![0.0; 3],
        indices: vec![0],
        normals: None,
        uvs: None,
        vertex_source: vec![0],
        source_vertex_count: 1,
        report: Default::default(),
    };

    let huge = 3.0e38_f64; // just under f32::MAX; two of them overflow
    assert!((huge as f32).is_finite(), "premise: each weight is finite");
    assert!(
        (huge as f32 + huge as f32).is_infinite(),
        "premise: the sum is not"
    );

    let cluster = |id: i64| skin::Cluster {
        id,
        bone_id: 100 + id,
        indices: vec![0],
        weights: vec![huge],
        transform_link: DMat4::IDENTITY,
        transform: DMat4::IDENTITY,
    };
    let s = Skin {
        id: 1,
        geometry_id: 2,
        clusters: vec![cluster(0), cluster(1)],
        report: Default::default(),
    };

    let (weights, _report) = s.bind(&mesh).expect("binds");

    assert_eq!(weights.fallback_vertices, vec![0], "pinned by the fallback");
    assert_eq!(
        &weights.weights[..MAX_INFLUENCES],
        &[1.0, 0.0, 0.0, 0.0],
        "fully pinned to bone 0, not divided down to nothing"
    );
    assert_eq!(weights.first_unnormalised(1e-5), None);
}

#[test]
fn a_cluster_with_no_bone_and_one_with_ragged_arrays_are_both_reported() {
    // Both counters describe influences the parser DROPS, so a file that
    // silently loses deformation is exactly what they exist to make visible.
    // The reference rig is clean and drives neither, so this builds the two
    // malformed shapes directly.
    let doc = text::parse(concat!(
        "FBXVersion: 7400\n",
        "Objects:  {\n",
        "\tGeometry: 9, \"Geometry::Body\", \"Mesh\" {\n",
        "\t\tVertices: *9 {\n",
        "\t\t\ta: 0,0,0,1,0,0,0,1,0\n",
        "\t\t}\n",
        "\t}\n",
        "\tModel: 7, \"Model::Hips\", \"LimbNode\" {\n",
        "\t}\n",
        "\tDeformer: 20, \"Deformer::skin\", \"Skin\" {\n",
        "\t}\n",
        // Three indices, two weights: the trailing index has no weight.
        "\tDeformer: 30, \"SubDeformer::ragged\", \"Cluster\" {\n",
        "\t\tIndexes: *3 {\n",
        "\t\t\ta: 0,1,2\n",
        "\t\t}\n",
        "\t\tWeights: *2 {\n",
        "\t\t\ta: 0.5,0.5\n",
        "\t\t}\n",
        "\t}\n",
        // A cluster with weights but no Model connected: nothing to drive.
        "\tDeformer: 31, \"SubDeformer::orphan\", \"Cluster\" {\n",
        "\t\tIndexes: *1 {\n",
        "\t\t\ta: 0\n",
        "\t\t}\n",
        "\t\tWeights: *1 {\n",
        "\t\t\ta: 1.0\n",
        "\t\t}\n",
        "\t}\n",
        "}\n",
        "Connections:  {\n",
        "\tC: \"OO\",20,9\n",  // skin deforms the geometry
        "\tC: \"OO\",30,20\n", // ragged cluster belongs to the skin
        "\tC: \"OO\",31,20\n", // orphan cluster too
        "\tC: \"OO\",7,30\n",  // only the ragged cluster has a bone
        "}\n",
    ))
    .expect("ascii parses");

    let (skins, skipped) = skin::parse_all(&Scene::from_document(doc));
    assert_eq!(skipped, 0, "both skins have geometry");
    assert_eq!(skins.len(), 1, "one skin");
    let skin = &skins[0];

    assert_eq!(skin.report.clusters_without_bone, 1, "the orphan cluster");
    assert_eq!(skin.report.mismatched_arrays, 1, "the ragged cluster");

    // The orphan is dropped entirely, so only the ragged one survives -- and
    // it keeps the two pairs that were complete, not the three indices.
    assert_eq!(skin.clusters.len(), 1, "the orphan is not kept");
    assert_eq!(skin.clusters[0].indices, vec![0, 1], "truncated to pairs");
    assert_eq!(skin.clusters[0].weights, vec![0.5, 0.5]);
    assert_eq!(skin.bone_ids(), vec![7], "the one cluster that had a bone");
}

#[test]
fn cluster_order_is_deterministic_and_keyed_on_cluster_id() {
    // A cluster's POSITION is its bone index downstream, so this ordering is
    // part of the output, not an implementation detail: a rig whose bones are
    // numbered differently between runs is a different rig. The connection
    // lookup behind `children_of` has no inherent order, which is why `parse`
    // sorts at all.
    let (skins, _) = skin::parse_all(&scene());

    for skin in &skins {
        let ids: Vec<i64> = skin.clusters.iter().map(|c| c.id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "clusters must be in cluster-id order");

        // Keyed on cluster id specifically. If bone id gave the same order the
        // assertion above would not distinguish the two, so check it does not.
        let mut by_bone: Vec<(i64, i64)> =
            skin.clusters.iter().map(|c| (c.bone_id, c.id)).collect();
        by_bone.sort_unstable();
        let bone_order: Vec<i64> = by_bone.into_iter().map(|(_, id)| id).collect();
        assert_ne!(
            bone_order, ids,
            "bone-id order coincides with cluster-id order, so this test cannot \
             tell the two apart -- it needs a different rig to stay meaningful"
        );
    }

    // And it is stable across parses, not merely sorted within one.
    let (again, _) = skin::parse_all(&scene());
    assert_eq!(
        again.iter().map(|s| s.bone_ids()).collect::<Vec<_>>(),
        skins.iter().map(|s| s.bone_ids()).collect::<Vec<_>>(),
        "two parses of the same bytes must number bones identically"
    );
}

#[test]
fn a_negative_vertex_index_drops_its_weight_with_it() {
    // The quiet one. A vertex index that cannot be a u32 has to be discarded
    // as a PAIR: dropping the index alone leaves its weight in place, and
    // every later weight slides one position onto a vertex it was never
    // painted for. Nothing about the result looks wrong -- the arrays are the
    // same length as each other, the weights still sum to 1 -- but the mesh
    // deforms from the wrong bone.
    let doc = text::parse(concat!(
        "FBXVersion: 7400\n",
        "Objects:  {\n",
        "\tGeometry: 9, \"Geometry::Body\", \"Mesh\" {\n",
        "\t\tVertices: *9 {\n",
        "\t\t\ta: 0,0,0,1,0,0,0,1,0\n",
        "\t\t}\n",
        "\t}\n",
        "\tModel: 7, \"Model::Hips\", \"LimbNode\" {\n",
        "\t}\n",
        "\tDeformer: 20, \"Deformer::skin\", \"Skin\" {\n",
        "\t}\n",
        // Index 1 is negative. Its weight (0.25) must go with it, leaving
        // vertex 0 -> 0.5 and vertex 2 -> 0.75 correctly paired.
        "\tDeformer: 30, \"SubDeformer::c\", \"Cluster\" {\n",
        "\t\tIndexes: *3 {\n",
        "\t\t\ta: 0,-1,2\n",
        "\t\t}\n",
        "\t\tWeights: *3 {\n",
        "\t\t\ta: 0.5,0.25,0.75\n",
        "\t\t}\n",
        "\t}\n",
        "}\n",
        "Connections:  {\n",
        "\tC: \"OO\",20,9\n",
        "\tC: \"OO\",30,20\n",
        "\tC: \"OO\",7,30\n",
        "}\n",
    ))
    .expect("ascii parses");

    let (skins, _skipped) = skin::parse_all(&Scene::from_document(doc));
    let cluster = &skins[0].clusters[0];

    assert_eq!(skins[0].report.unusable_indices, 1, "the negative index");
    // Not counted as a ragged array: the two arrays were the same length.
    assert_eq!(skins[0].report.mismatched_arrays, 0);

    // The surviving pairs keep their ORIGINAL pairing. Filtering the index
    // alone would give weights [0.5, 0.25] here -- same length, wrong values.
    assert_eq!(cluster.indices, vec![0, 2]);
    assert_eq!(cluster.weights, vec![0.5, 0.75], "0.25 went with its index");
}

/// One skin over a three-vertex mesh, with the cluster's body supplied by the
/// caller. Geometry is 9, the bone Model is 7, the skin 20, the cluster 30.
fn synthetic(cluster_body: &str, extra_connections: &str) -> Scene {
    let doc = text::parse(&format!(
        concat!(
            "FBXVersion: 7400\n",
            "Objects:  {{\n",
            "\tGeometry: 9, \"Geometry::Body\", \"Mesh\" {{\n",
            "\t\tVertices: *9 {{\n",
            "\t\t\ta: 0,0,0,1,0,0,0,1,0\n",
            "\t\t}}\n",
            "\t\tPolygonVertexIndex: *3 {{\n",
            "\t\t\ta: 0,1,-3\n",
            "\t\t}}\n",
            "\t}}\n",
            "\tModel: 7, \"Model::Hips\", \"LimbNode\" {{\n",
            "\t}}\n",
            "\tDeformer: 20, \"Deformer::skin\", \"Skin\" {{\n",
            "\t}}\n",
            "\tDeformer: 30, \"SubDeformer::c\", \"Cluster\" {{\n",
            "{}",
            "\t}}\n",
            "}}\n",
            "Connections:  {{\n",
            "\tC: \"OO\",20,9\n",
            "\tC: \"OO\",30,20\n",
            "{}",
            "}}\n",
        ),
        cluster_body, extra_connections
    ))
    .expect("ascii parses");
    Scene::from_document(doc)
}

/// The weights body used wherever the test is about something else.
const GOOD_WEIGHTS: &str =
    "\t\tIndexes: *1 {\n\t\t\ta: 0\n\t\t}\n\t\tWeights: *1 {\n\t\t\ta: 1.0\n\t\t}\n";
const BONE: &str = "\tC: \"OO\",7,30\n";

#[test]
fn bind_matrices_that_are_unusable_are_counted_not_silently_identity() {
    // The module's whole bind-matrix argument is that `Transform` is never
    // identity on a real file -- worst deviation 179.9 on the reference rig.
    // So defaulting to identity misplaces the mesh by that much, and a file
    // truncated inside a matrix block must not produce it in silence.
    let cases = [
        // Twelve floats: an exporter writing a 4x3, or a truncated block.
        (
            "\t\tTransformLink: *12 {\n\t\t\ta: 1,0,0,0,0,1,0,0,0,0,1,0\n\t\t}\n",
            "wrong length",
        ),
        // All zeros. `DMat4::inverse` does NOT panic on this -- glam is built
        // without `glam-assert` -- it returns all-NaN, which would spread into
        // every vertex the bone touches with nothing to show it happened.
        (
            "\t\tTransformLink: *16 {\n\t\t\ta: 0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0\n\t\t}\n",
            "singular",
        ),
    ];

    for (matrix_body, why) in cases {
        let scene = synthetic(&format!("{GOOD_WEIGHTS}{matrix_body}"), BONE);
        let (skins, _) = skin::parse_all(&scene);
        let cluster = &skins[0].clusters[0];

        // Both TransformLink (bad) and Transform (absent) default here.
        assert_eq!(skins[0].report.matrices_defaulted, 2, "{why}: counted");
        assert_eq!(cluster.transform_link, DMat4::IDENTITY, "{why}");

        // And the inverse bind is usable rather than NaN.
        let inverse = cluster.inverse_bind();
        assert!(
            inverse.is_finite(),
            "{why}: inverse_bind must not be NaN, got {inverse:?}"
        );
    }
}

#[test]
fn a_singular_matrix_really_does_invert_to_nan_in_this_glam_build() {
    // The premise behind rejecting singular TransformLink at parse time. If a
    // future glam release started panicking or returning something defined,
    // this fails and the guard above can be reconsidered.
    assert!(!DMat4::ZERO.inverse().is_finite());
}

#[test]
fn an_unreadable_index_array_is_not_mistaken_for_an_empty_one() {
    // 40 clusters on the reference rig legitimately have no `Indexes` node --
    // bones that influence nothing. A cluster whose `Indexes` node is present
    // but undecodable looks identical after parsing (an empty vec), so without
    // the distinction that bone's entire influence set vanishes unrecorded.
    let scene = synthetic(
        "\t\tIndexes: *2 {\n\t\t\ta: 0.5,1.5\n\t\t}\n\t\tWeights: *2 {\n\t\t\ta: 0.5,0.5\n\t\t}\n",
        BONE,
    );
    let (skins, _) = skin::parse_all(&scene);

    assert_eq!(
        skins[0].report.undecodable_arrays, 1,
        "the fractional array"
    );
    assert!(skins[0].clusters[0].indices.is_empty());

    // A cluster with NO Indexes node is the legitimate case and must stay
    // uncounted, or the signal is worthless on any real rig.
    let absent = synthetic("\t\tWeights: *1 {\n\t\t\ta: 1.0\n\t\t}\n", BONE);
    let (absent, _) = skin::parse_all(&absent);
    assert_eq!(
        absent[0].report.undecodable_arrays, 0,
        "absent is not corrupt"
    );
}

#[test]
fn a_skin_with_no_usable_clusters_fails_instead_of_pinning_to_a_bone_that_is_not_there() {
    // Every cluster dropped (no bone connected) leaves `bone_ids()` empty while
    // the fallback would still write bone index 0 into every corner -- an index
    // into an empty bone list, reached from an Ok.
    let scene = synthetic(GOOD_WEIGHTS, ""); // no bone connection
    let (skins, _) = skin::parse_all(&scene);
    assert_eq!(skins[0].report.clusters_without_bone, 1);
    assert!(skins[0].bone_ids().is_empty(), "no bones survive");

    let mesh = geometry::parse(
        scene.object(skins[0].geometry_id).expect("geometry"),
        GeometricTransform::default(),
    )
    .expect("geometry parses");

    assert!(
        skins[0].bind(&mesh).is_err(),
        "binding with no bones must fail, not pin every vertex to bone 0"
    );
}

#[test]
fn a_skin_whose_geometry_link_is_missing_is_reported_not_just_dropped() {
    // Without the count, a truncated Connections section is indistinguishable
    // from a mesh that was never skinned -- and this drop is larger than any
    // counted inside SkinReport, because the report goes with the skin.
    let doc = text::parse(concat!(
        "FBXVersion: 7400\n",
        "Objects:  {\n",
        "\tDeformer: 20, \"Deformer::skin\", \"Skin\" {\n",
        "\t}\n",
        "}\n",
        "Connections:  {\n",
        "}\n",
    ))
    .expect("ascii parses");

    let (skins, skipped) = skin::parse_all(&Scene::from_document(doc));
    assert!(skins.is_empty());
    assert_eq!(skipped, 1, "the skin with no geometry must be counted");
}

#[test]
fn a_bind_matrix_holding_infinity_is_rejected_even_though_it_is_not_singular() {
    // The determinant check cannot catch this one: a matrix with a non-finite
    // entry has a non-finite determinant, and `NaN == 0.0` is false, so it
    // would sail through as "invertible". Only the finiteness check stops it,
    // and `Transform` is never inverted at all -- finiteness is its only guard.
    let inf = "1e400";
    let body =
        format!("\t\tTransform: *16 {{\n\t\t\ta: {inf},0,0,0,0,1,0,0,0,0,1,0,0,0,0,1\n\t\t}}\n");
    let scene = synthetic(&format!("{GOOD_WEIGHTS}{body}"), BONE);
    let (skins, _) = skin::parse_all(&scene);
    let cluster = &skins[0].clusters[0];

    // Premise: the file really does carry a non-finite value, and it is not
    // the singularity case -- so if this guard goes, nothing else catches it.
    assert!(inf.parse::<f64>().expect("parses").is_infinite());

    assert_eq!(skins[0].report.matrices_defaulted, 2, "both defaulted");
    assert_eq!(
        cluster.transform,
        DMat4::IDENTITY,
        "not the infinite matrix"
    );
    assert!(cluster.inverse_bind().is_finite());
}
