//! Visual regression across every creature, through the real independent reader.
//!
//! The whole rig pipeline — fit, bind, export — is run for each of the nine
//! templates, and the exported `.glb` is imported by **Blender** (the only
//! reader in the project that does not share our design) and checked against a
//! committed baseline. A change that silently alters what a rig looks like to a
//! real DCC tool fails here.
//!
//! **Local only.** CI has no Blender, so this is `#[ignore]`d and run by hand
//! (`cargo test -p mesh2motion --release -- --ignored`). It is the automated
//! form of the Blender check every session has done by hand — see P4-1 and
//! `memory/test.md` §9.

use mesh2motion_lib::rig;

/// A creature's expected Blender report after the full pipeline.
///
/// Baselines were recorded by running the pipeline once (session 056) and are
/// the numbers Blender reports for the export. They are exact on purpose: a
/// regression that moves any of them is a regression to see, not to absorb.
struct Baseline {
    creature: &'static str,
    /// Bones Blender counts in the exported armature — equals the template's
    /// joint count.
    bones: u32,
    /// Vertices after lossless welding of the imported mesh.
    vertices: u32,
}

const BASELINES: &[Baseline] = &[
    Baseline {
        creature: "human",
        bones: 66,
        vertices: 7399,
    },
    Baseline {
        creature: "fox",
        bones: 49,
        vertices: 1222,
    },
    Baseline {
        creature: "horse",
        bones: 56,
        vertices: 2146,
    },
    Baseline {
        creature: "bird",
        bones: 55,
        vertices: 1852,
    },
    Baseline {
        creature: "spider",
        bones: 56,
        vertices: 924,
    },
    Baseline {
        creature: "snake",
        bones: 28,
        vertices: 995,
    },
    Baseline {
        creature: "shark",
        bones: 33,
        vertices: 3526,
    },
    Baseline {
        creature: "kaiju",
        bones: 58,
        vertices: 1571,
    },
    Baseline {
        creature: "dragon",
        bones: 99,
        vertices: 2561,
    },
];

#[test]
#[ignore = "needs a local Blender install"]
fn every_creature_rigs_and_reads_back_the_same_in_blender() {
    let blender = match m2m_bridge::blender_path() {
        Ok(path) => path,
        Err(_) => return, // no Blender on this box; the ignore covers CI
    };

    let mut failures = Vec::new();
    for base in BASELINES {
        let path = format!("../legacy/static/models/model-{}.glb", base.creature);
        let bytes = std::fs::read(&path).expect("a creature model");

        // The whole pipeline the app runs.
        let skeleton = rig::fit(base.creature, &bytes).expect("fits");
        let bound = rig::bind(&bytes, &skeleton, 2.0).expect("binds");
        let glb = rig::export_glb(&bytes, &skeleton, 2.0, None).expect("exports");
        let report = m2m_bridge::inspect(&glb, "glb", &blender).expect("Blender reads it");

        // Invariants that must hold for EVERY creature regardless of the exact
        // numbers: the rig imports, deforms nothing it should not, and every
        // vertex keeps a full unit of weight.
        let mut problems = Vec::new();
        if !report.imported {
            problems.push(format!("did not import: {:?}", report.error));
        }
        if report.meshes != Some(1) {
            problems.push(format!("meshes = {:?}, expected 1", report.meshes));
        }
        if bound.fallback_vertices != 0 {
            problems.push(format!(
                "{} fallback vertices — a disconnected island",
                bound.fallback_vertices
            ));
        }
        // weight_total equals the vertex count exactly when every vertex sums to
        // one — the skinning invariant, seen from the outside.
        if report.weight_total != Some(f64::from(base.vertices)) {
            problems.push(format!(
                "weight_total = {:?}, expected {} (every vertex should sum to 1)",
                report.weight_total, base.vertices
            ));
        }

        // The committed baseline: a change here is a real change in what the
        // rig looks like to Blender.
        if report.bones != Some(base.bones) {
            problems.push(format!(
                "bones = {:?}, baseline {}",
                report.bones, base.bones
            ));
        }
        if report.mesh_vertices != vec![base.vertices] {
            problems.push(format!(
                "mesh_vertices = {:?}, baseline [{}]",
                report.mesh_vertices, base.vertices
            ));
        }

        if !problems.is_empty() {
            failures.push(format!("{}: {}", base.creature, problems.join("; ")));
        }
    }

    assert!(
        failures.is_empty(),
        "visual regression:\n  {}",
        failures.join("\n  ")
    );
}
