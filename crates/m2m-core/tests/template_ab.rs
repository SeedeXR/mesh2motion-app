//! P1-8: the geodesic solver against the legacy solver, on all nine shipping
//! templates.
//!
//! This is the gate for P1-9 — deleting `ArmWeightCorrector`,
//! `HeadWeightCorrector` and `ExtremityWeightCorrector`. Those exist to patch
//! the Euclidean nearest-bone failure mode; if the geodesic solver still needs
//! them, it is wrong and the solver gets fixed rather than the patches ported.

#[path = "fixture_support.rs"]
mod fixture_support;

use fixture_support::{load_mesh, load_rig};
use m2m_core::geodesic::GeodesicField;
use m2m_core::mesh::Mesh;
use m2m_core::skinning::{assign_weights, SkinWeights, SkinningParams};
use m2m_core::voxel::{VoxelGrid, DEFAULT_RESOLUTION};

/// Influences below this fraction of a vertex's weight do not move the surface.
///
/// The falloff leaves a fourth influence on nearly every vertex with a median
/// value around 0.001. Counting those flatters the comparison, so the headline
/// figures use this threshold and the raw count is reported alongside for
/// like-for-like comparison with the legacy baseline, which used 1e-6.
const MEANINGFUL: f32 = 0.01;

/// One template's legacy figures, from `bench/baselines/legacy-solver.json`
/// (captured in P0-10, before any solver work).
struct Legacy {
    name: &'static str,
    vertices: usize,
    /// Total bones, weightable or not. Asserted so a fixture cannot silently be
    /// paired with the wrong rig — only the vertex count was checked before.
    bones: usize,
    /// Exact, not rounded to whole percent. Rounding 60.637 to 61.0 moved the
    /// pass bar by up to half a point, so a geodesic result of 60.8% would have
    /// been recorded as an improvement.
    single_influence_pct: f32,
    mean_influences: f32,
}

const LEGACY: &[Legacy] = &[
    Legacy {
        name: "human",
        vertices: 7399,
        bones: 66,
        single_influence_pct: 86.782,
        mean_influences: 1.132,
    },
    Legacy {
        name: "fox",
        vertices: 1222,
        bones: 49,
        single_influence_pct: 49.100,
        mean_influences: 1.509,
    },
    Legacy {
        name: "bird",
        vertices: 1852,
        bones: 55,
        single_influence_pct: 60.637,
        mean_influences: 1.394,
    },
    Legacy {
        name: "horse",
        vertices: 2146,
        bones: 56,
        single_influence_pct: 55.172,
        mean_influences: 1.448,
    },
    Legacy {
        name: "fish",
        vertices: 3526,
        bones: 33,
        single_influence_pct: 67.357,
        mean_influences: 1.326,
    },
    Legacy {
        name: "dragon",
        vertices: 2561,
        bones: 99,
        single_influence_pct: 72.979,
        mean_influences: 1.270,
    },
    Legacy {
        name: "kaiju",
        vertices: 1571,
        bones: 58,
        single_influence_pct: 47.931,
        mean_influences: 1.521,
    },
    Legacy {
        name: "snake",
        vertices: 995,
        bones: 28,
        single_influence_pct: 18.995,
        mean_influences: 1.810,
    },
    Legacy {
        name: "spider",
        vertices: 924,
        bones: 56,
        single_influence_pct: 58.442,
        mean_influences: 1.416,
    },
];

/// `include_bytes!` needs a literal path, so the fixtures are listed explicitly.
fn fixture(name: &str) -> (Mesh, fixture_support::Rig) {
    macro_rules! pair {
        ($n:literal) => {
            (
                load_mesh(include_bytes!(concat!(
                    "fixtures/template-",
                    $n,
                    "-mesh.bin"
                ))),
                load_rig(include_bytes!(concat!(
                    "fixtures/template-",
                    $n,
                    "-rig.bin"
                ))),
            )
        };
    }
    match name {
        "human" => pair!("human"),
        "fox" => pair!("fox"),
        "bird" => pair!("bird"),
        "horse" => pair!("horse"),
        "fish" => pair!("fish"),
        "dragon" => pair!("dragon"),
        "kaiju" => pair!("kaiju"),
        "snake" => pair!("snake"),
        "spider" => pair!("spider"),
        other => panic!("unknown template {other}"),
    }
}

struct Outcome {
    weights: SkinWeights,
    single_pct: f32,
    mean: f32,
    mean_raw: f32,
    unreachable_bone_indices: Vec<usize>,
    elapsed_ms: f32,
}

fn solve(mesh: &Mesh, rig: &fixture_support::Rig) -> Outcome {
    let t = std::time::Instant::now();
    let grid = VoxelGrid::build(mesh, DEFAULT_RESOLUTION).expect("grid");
    let field = GeodesicField::compute(mesh, &grid, &rig.bones).expect("field");
    let weights = assign_weights(
        &field,
        &mesh.positions,
        &rig.bones,
        &rig.weightable,
        SkinningParams::default(),
    );
    let elapsed_ms = t.elapsed().as_secs_f32() * 1000.0;

    let n = weights.vertex_count();
    let mut single = 0usize;
    let mut total = 0usize;
    let mut total_raw = 0usize;
    for v in 0..n {
        let meaningful = weights
            .influences(v)
            .filter(|(_, w)| *w > MEANINGFUL)
            .count();
        if meaningful == 1 {
            single += 1;
        }
        total += meaningful;
        total_raw += weights.influences(v).count();
    }

    Outcome {
        single_pct: 100.0 * single as f32 / n as f32,
        mean: total as f32 / n as f32,
        mean_raw: total_raw as f32 / n as f32,
        // Only bones that are supposed to carry weight. Leaf bones legitimately
        // sit outside the mesh — they exist to orient their parent — so counting
        // them reports failures that are not failures. Doing so once made bird,
        // horse, dragon and spider all look broken when none of them were.
        unreachable_bone_indices: field
            .unreachable_bones()
            .into_iter()
            .filter(|&b| rig.weightable[b])
            .collect(),
        elapsed_ms,
        weights,
    }
}

#[test]
fn geodesic_beats_legacy_on_every_template() {
    println!(
        "{:<8} {:>6} {:>6} {:>18} {:>18} {:>8} {:>7}",
        "template", "verts", "bones", "single-influence %", "mean influences", "raw mean", "ms"
    );
    println!(
        "{:<8} {:>6} {:>6} {:>18} {:>18} {:>8} {:>7}",
        "", "", "weightable", "legacy → geodesic", "legacy → geodesic", "geodesic", ""
    );

    let mut regressions = Vec::new();

    for legacy in LEGACY {
        let (mesh, rig) = fixture(legacy.name);
        let weightable = rig.weightable.iter().filter(|&&w| w).count();

        // The fixture must be the same geometry the baseline measured, or the
        // comparison is meaningless.
        assert_eq!(
            rig.bones.len(),
            legacy.bones,
            "{}: rig fixture has {} bones, baseline measured {}",
            legacy.name,
            rig.bones.len(),
            legacy.bones
        );
        assert_eq!(
            mesh.vertex_count(),
            legacy.vertices,
            "{}: fixture has {} verts, baseline measured {}",
            legacy.name,
            mesh.vertex_count(),
            legacy.vertices
        );

        let got = solve(&mesh, &rig);

        println!(
            "{:<8} {:>6} {:>6} {:>8.1} → {:<7.1} {:>8.2} → {:<7.2} {:>8.2} {:>7.0}",
            legacy.name,
            mesh.vertex_count(),
            weightable,
            legacy.single_influence_pct,
            got.single_pct,
            legacy.mean_influences,
            got.mean,
            got.mean_raw,
            got.elapsed_ms
        );
        // The baseline counted any influence above 1e-6, so the raw figure is
        // the like-for-like one; the 1% figure above is the honest one. Both are
        // shown because they answer different questions.
        assert!(
            got.mean_raw > legacy.mean_influences,
            "{}: raw mean {:.2} did not beat legacy {:.2} at the same threshold",
            legacy.name,
            got.mean_raw,
            legacy.mean_influences
        );
        assert!(
            got.weights.fallback_vertices.is_empty(),
            "{}: {} vertices needed the Euclidean fallback",
            legacy.name,
            got.weights.fallback_vertices.len()
        );
        // Known asset issue, not a solver failure: in rig-shark.glb the
        // back_fin_2_l/r bones (indices 13 and 17, a mirrored pair at
        // x = +/-0.451, z = -1.83) sit outside model-shark.glb. Several
        // fin *tip* bones are outside too, but those are leaf bones and the
        // weightable filter already excludes them.
        //
        // Pinned by INDEX rather than by count: a bare count would still pass if
        // these two became reachable while two other bones died. Note the
        // property is resolution-coupled — at resolution 192 the first sample of
        // bone 13 lands on a surface voxel — so this holds at DEFAULT_RESOLUTION.
        let expected: &[usize] = if legacy.name == "fish" {
            &[13, 17]
        } else {
            &[]
        };
        assert_eq!(
            got.unreachable_bone_indices, expected,
            "{}: unexpected set of weightable bones reached no vertex",
            legacy.name
        );

        // Invariants must hold on every template, not just the human.
        assert_eq!(
            got.weights.first_unnormalised(1e-4),
            None,
            "{}: produced invalid weights",
            legacy.name
        );
        // Only populated influences: an unused slot holds index 0, which is the
        // root bone, so scanning the raw index buffer reports a false positive.
        for v in 0..got.weights.vertex_count() {
            for (bone, weight) in got.weights.influences(v) {
                assert!(
                    rig.weightable[bone as usize],
                    "{}: vertex {v} has weight {weight} on non-weightable bone {bone}",
                    legacy.name
                );
            }
        }

        // The comparison itself. Blending strictly more than the legacy solver
        // is the whole point; a template that does not is a regression to
        // investigate, not to wave through.
        if got.mean <= legacy.mean_influences {
            regressions.push(format!(
                "{}: mean influences {:.2} did not beat legacy {:.2}",
                legacy.name, got.mean, legacy.mean_influences
            ));
        }
        if got.single_pct >= legacy.single_influence_pct {
            regressions.push(format!(
                "{}: single-influence {:.1}% did not beat legacy {:.1}%",
                legacy.name, got.single_pct, legacy.single_influence_pct
            ));
        }
    }

    assert!(
        regressions.is_empty(),
        "A/B regressions:\n  {}",
        regressions.join("\n  ")
    );
}
