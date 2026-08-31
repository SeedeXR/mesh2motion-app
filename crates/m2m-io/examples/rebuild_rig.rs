//! Reads an FBX rig through our own types and writes it back out.
//!
//! This is the acceptance test for O9 (`memory/project_context.md`): an
//! existing rig must survive import and export untouched — skeleton, bone
//! names, hierarchy, bind matrices and skin weights. Unlike
//! `encode_roundtrip`, which re-encodes the document the reader produced, this
//! goes all the way through the semantic layers and rebuilds from them, so
//! anything those layers drop is visible in the output.
//!
//!     cargo run -p m2m-io --release --example rebuild_rig -- <in.fbx> <out.fbx>

use m2m_io::fbx::binary::FbxProperty;
use m2m_io::fbx::{binary, build, dom::Scene, encode, model, skin};
use std::collections::HashMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        eprintln!("usage: rebuild_rig <in.fbx> <out.fbx>");
        std::process::exit(2);
    };

    let bytes = std::fs::read(&input)?;
    let scene = Scene::from_document(binary::parse(&bytes)?);
    let models = model::parse_all(&scene);
    let (skins, skins_without_geometry) = skin::parse_all(&scene);

    // Bones, parents before children. `build::Scene` requires that ordering so
    // a single forward pass can emit connections — and so a cycle cannot be
    // expressed at all.
    let mut order: Vec<i64> = Vec::new();
    let mut queue: Vec<i64> = models.roots.clone();
    while let Some(id) = queue.pop() {
        let Some(node) = models.get(id) else { continue };
        if node.is_bone() {
            order.push(id);
        }
        queue.extend(node.children.iter().rev().copied());
    }
    let bone_index: HashMap<i64, usize> =
        order.iter().enumerate().map(|(i, &id)| (id, i)).collect();

    let bones: Vec<build::Bone> = order
        .iter()
        .map(|&id| {
            let m = models.get(id).expect("in order");
            build::Bone {
                name: &m.name,
                parent: m.parent.and_then(|p| bone_index.get(&p).copied()),
                translation: m.transform.translation.to_array(),
                rotation: m.transform.rotation.to_array(),
                scale: m.transform.scale.to_array(),
                pre_rotation: m.transform.pre_rotation.to_array(),
            }
        })
        .collect();

    // Meshes, taken RAW rather than through `geometry::parse`.
    //
    // That layer triangulates and expands per corner, which is right for a
    // solver and wrong for a writer: rebuilding from the triangulated form
    // turns the reference rig's 11,120 and 14,222 polygons into 20,840 and
    // 28,272, splitting every quad. O9 says a model comes back as it went in,
    // and an artist notices a quad mesh returned as triangles.
    let mut mesh_positions: Vec<Vec<f32>> = Vec::new();
    let mut mesh_polygons: Vec<Vec<i32>> = Vec::new();
    let mut mesh_names: Vec<String> = Vec::new();
    let mut geometry_index: HashMap<i64, usize> = HashMap::new();

    for object in scene.objects_of_kind("Geometry") {
        let raw_vertices = object
            .node
            .child("Vertices")
            .and_then(|n| n.properties.first())
            .and_then(FbxProperty::as_f64_vec)
            .ok_or("geometry has no Vertices array")?;
        let raw_polygons = object
            .node
            .child("PolygonVertexIndex")
            .and_then(|n| n.properties.first())
            .and_then(FbxProperty::as_i64_vec)
            .ok_or("geometry has no PolygonVertexIndex array")?;
        geometry_index.insert(object.id, mesh_names.len());
        mesh_names.push(object.name.clone());
        mesh_positions.push(raw_vertices.iter().map(|&v| v as f32).collect());
        mesh_polygons.push(raw_polygons.iter().map(|&v| v as i32).collect());
    }

    let meshes: Vec<build::Mesh> = (0..mesh_names.len())
        .map(|i| build::Mesh {
            name: &mesh_names[i],
            positions: &mesh_positions[i],
            faces: build::Faces::Polygons(&mesh_polygons[i]),
        })
        .collect();

    // Skins. A cluster whose bone is not in the skeleton is dropped rather
    // than guessed at, and counted so the drop is never silent.
    let mut dropped_clusters = 0usize;
    let mut cluster_store: Vec<Vec<build::Cluster>> = Vec::new();
    let mut skin_meshes: Vec<usize> = Vec::new();
    for s in &skins {
        let Some(&mesh) = geometry_index.get(&s.geometry_id) else {
            continue;
        };
        let clusters: Vec<build::Cluster> = s
            .clusters
            .iter()
            .filter_map(|c| {
                let bone = bone_index.get(&c.bone_id).copied()?;
                Some(build::Cluster {
                    bone,
                    indices: &c.indices,
                    weights: &c.weights,
                    transform: c.transform.to_cols_array(),
                    transform_link: c.transform_link.to_cols_array(),
                })
            })
            .collect();
        dropped_clusters += s.clusters.len() - clusters.len();
        skin_meshes.push(mesh);
        cluster_store.push(clusters);
    }
    let built_skins: Vec<build::Skin> = skin_meshes
        .iter()
        .zip(&cluster_store)
        .map(|(&mesh, clusters)| build::Skin { mesh, clusters })
        .collect();

    let document = build::build(&build::Scene {
        meshes: &meshes,
        bones: &bones,
        skins: &built_skins,
    });
    let encoded = encode::encode(&document)?;
    std::fs::write(&output, &encoded)?;

    println!(
        "{} bones, {} meshes, {} skins, {} clusters ({} dropped, {} skins without geometry), {} bytes",
        bones.len(),
        meshes.len(),
        built_skins.len(),
        cluster_store.iter().map(Vec::len).sum::<usize>(),
        dropped_clusters,
        skins_without_geometry,
        encoded.len()
    );
    Ok(())
}
