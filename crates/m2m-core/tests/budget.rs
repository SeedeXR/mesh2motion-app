//! P1-11: the solver against the resource budgets in `memory/test.md` §6.
//!
//! The budget is stated for a 50k-vertex mesh. The largest shipping template is
//! 7399 vertices, so the meshes here are subdivided upward to reach it — that
//! measures the solver at the size the budget describes rather than
//! extrapolating from a mesh seven times smaller.
//!
//! Peak **heap** is measured with a tracking allocator rather than process RSS.
//! RSS never shrinks, so in a multi-test binary it reports the high-water mark
//! of everything that ran before, which is how an earlier version of the legacy
//! benchmark managed to report a 924-vertex spider using more memory than a
//! 7399-vertex human.

#[path = "fixture_support.rs"]
mod fixture_support;

use fixture_support::{load_mesh, load_rig};
use glam::Vec3;
use m2m_core::geodesic::GeodesicField;
use m2m_core::mesh::Mesh;
use m2m_core::skinning::{assign_weights, SkinningParams};
use m2m_core::voxel::{VoxelGrid, DEFAULT_RESOLUTION};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Tracks live and peak bytes handed out by the allocator.
struct Tracking;

/// Padded to a cache line each: every allocation from every rayon worker hits
/// both with an atomic read-modify-write, and adjacent statics would put them on
/// one line. That false sharing slows the allocator relative to the shipping
/// build, which lands directly in the timings this file asserts on.
#[repr(align(64))]
struct Counter(AtomicUsize);

static LIVE: Counter = Counter(AtomicUsize::new(0));
static PEAK: Counter = Counter(AtomicUsize::new(0));

unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let live = LIVE.0.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.0.fetch_max(live, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.0.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Forwarding to System::realloc matters for what is being measured, not
        // just for speed. The default implementation is alloc + copy + dealloc,
        // which forfeits any in-place grow: every Vec growth would then both
        // cost a memcpy the shipping build does not pay, and double the
        // transient peak — inflating the two numbers this file asserts on.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            let live = if new_size >= layout.size() {
                LIVE.0
                    .fetch_add(new_size - layout.size(), Ordering::Relaxed)
                    + (new_size - layout.size())
            } else {
                LIVE.0
                    .fetch_sub(layout.size() - new_size, Ordering::Relaxed)
                    - (layout.size() - new_size)
            };
            PEAK.0.fetch_max(live, Ordering::Relaxed);
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOC: Tracking = Tracking;

/// Serialises measurement.
///
/// The allocator is process-global and `cargo test` runs tests concurrently, so
/// two measurements in flight share one counter and corrupt each other. That is
/// not hypothetical: with both tests in this file running in parallel, the
/// 7399-vertex peak read 66.7 MB against 39.3 MB when measured alone.
static MEASURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Resets the peak counter and returns the bytes live at the start.
fn begin_measure() -> usize {
    let live = LIVE.0.load(Ordering::Relaxed);
    PEAK.0.store(live, Ordering::Relaxed);
    live
}

/// Peak bytes allocated since [`begin_measure`], above the starting baseline.
fn peak_since(baseline: usize) -> usize {
    PEAK.0.load(Ordering::Relaxed).saturating_sub(baseline)
}

/// Splits every triangle into four.
///
/// Vertex count goes to `V + 3T`, **not** `4V`: three fresh midpoints are pushed
/// per triangle with no welding, so nothing is shared between neighbours. On the
/// human fixture (7399 verts, 13757 tris) that is 7399 → 48670 → 213754, a
/// factor of ~6.6 rather than 4. Getting this wrong is what first made the
/// budget assertions land on a mesh four times larger than the budget describes.
///
/// Deliberately left unwelded: the duplicates make the mesh *harder* for the
/// solver, not easier.
fn subdivide(mesh: &Mesh) -> Mesh {
    let mut positions: Vec<f32> = mesh
        .positions
        .iter()
        .flat_map(|p| [p.x, p.y, p.z])
        .collect();
    let mut indices = Vec::with_capacity(mesh.indices.len() * 4);

    let push_midpoint = |a: Vec3, b: Vec3, positions: &mut Vec<f32>| -> u32 {
        let m = (a + b) * 0.5;
        positions.extend_from_slice(&[m.x, m.y, m.z]);
        (positions.len() / 3 - 1) as u32
    };

    for tri in mesh.indices.chunks_exact(3) {
        let (i0, i1, i2) = (tri[0], tri[1], tri[2]);
        let (p0, p1, p2) = (
            mesh.positions[i0 as usize],
            mesh.positions[i1 as usize],
            mesh.positions[i2 as usize],
        );
        let m01 = push_midpoint(p0, p1, &mut positions);
        let m12 = push_midpoint(p1, p2, &mut positions);
        let m20 = push_midpoint(p2, p0, &mut positions);
        indices.extend_from_slice(&[i0, m01, m20, m01, i1, m12, m20, m12, i2, m01, m12, m20]);
    }

    Mesh::from_flat(&positions, &indices).expect("subdivided mesh is valid")
}

struct Measured {
    vertices: usize,
    voxelise_ms: f32,
    geodesic_ms: f32,
    weights_ms: f32,
    total_ms: f32,
    peak_mb: f32,
}

fn measure(mesh: &Mesh, rig: &fixture_support::Rig) -> Measured {
    // Held for the whole solve: the peak is meaningless if another test is
    // allocating against the same counter.
    let _guard = MEASURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let baseline = begin_measure();
    let t0 = std::time::Instant::now();

    let grid = VoxelGrid::build(mesh, DEFAULT_RESOLUTION).expect("grid");
    let t1 = std::time::Instant::now();

    let field = GeodesicField::compute(mesh, &grid, &rig.bones).expect("field");
    let t2 = std::time::Instant::now();

    let weights = assign_weights(
        &field,
        &mesh.positions,
        &rig.bones,
        &rig.weightable,
        SkinningParams::default(),
    );
    let t3 = std::time::Instant::now();

    // Read the peak before anything is dropped.
    let peak = peak_since(baseline);
    assert_eq!(weights.first_unnormalised(1e-4), None, "invalid weights");

    Measured {
        vertices: mesh.vertex_count(),
        voxelise_ms: (t1 - t0).as_secs_f32() * 1000.0,
        geodesic_ms: (t2 - t1).as_secs_f32() * 1000.0,
        weights_ms: (t3 - t2).as_secs_f32() * 1000.0,
        total_ms: (t3 - t0).as_secs_f32() * 1000.0,
        peak_mb: peak as f32 / (1024.0 * 1024.0),
    }
}

#[test]
fn solve_stays_within_the_budget_at_50k_vertices() {
    // memory/test.md §6: bind skin, 50k verts, fast path — 3 s, 1.5 GB peak.
    const TIME_BUDGET_S: f32 = 3.0;
    const MEMORY_BUDGET_MB: f32 = 1536.0;

    let (base, rig) = {
        // Fixture loading allocates too. Outside the lock those bytes are
        // fetch_max'd into whichever measurement is in flight on another thread.
        let _guard = MEASURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        (
            load_mesh(include_bytes!("fixtures/template-human-mesh.bin")),
            load_rig(include_bytes!("fixtures/template-human-rig.bin")),
        )
    };

    println!(
        "{:>8} {:>10} {:>10} {:>10} {:>10} {:>10}",
        "verts", "voxelise", "geodesic", "weights", "total ms", "peak MB"
    );

    // 7399 -> 48670 -> 213754. The middle one is what brackets the 50k the
    // budget names; a `>= 50_000` gate rejects it by 1330 vertices and lands the
    // assertions on a mesh 4.3x too large, which both over-claims on a pass and
    // risks a spurious failure.
    const TARGET_VERTS: usize = 50_000;
    let mut mesh = base;
    let mut measurements = Vec::new();
    for _ in 0..3 {
        let m = measure(&mesh, &rig);
        println!(
            "{:>8} {:>10.0} {:>10.0} {:>10.0} {:>10.0} {:>10.1}",
            m.vertices, m.voxelise_ms, m.geodesic_ms, m.weights_ms, m.total_ms, m.peak_mb
        );
        let past = m.vertices >= TARGET_VERTS;
        measurements.push(m);
        if past {
            break;
        }
        mesh = {
            let _guard = MEASURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            subdivide(&mesh)
        };
    }

    // The measurement closest to the budget's stated size, in either direction.
    let m = measurements
        .iter()
        .min_by_key(|m| m.vertices.abs_diff(TARGET_VERTS))
        .expect("at least one measurement");
    println!(
        "\nbudget applies to ~{TARGET_VERTS} verts; asserting on the {} row",
        m.vertices
    );

    // Debug builds are far slower; the release figure is the one the budget
    // describes. Asserting a different ceiling per profile rather than skipping
    // keeps the check alive under `cargo test --workspace`, which builds debug.
    let time_ceiling = if cfg!(debug_assertions) {
        TIME_BUDGET_S * 20.0
    } else {
        TIME_BUDGET_S
    };
    assert!(
        m.total_ms / 1000.0 < time_ceiling,
        "{} verts took {:.2} s, ceiling {:.1} s",
        m.vertices,
        m.total_ms / 1000.0,
        time_ceiling
    );

    // Peak heap is profile-independent, so this is the real budget either way.
    assert!(
        m.peak_mb < MEMORY_BUDGET_MB,
        "{} verts peaked at {:.0} MB, budget {:.0} MB",
        m.vertices,
        m.peak_mb,
        MEMORY_BUDGET_MB
    );
}

#[test]
fn resolution_is_the_dominant_cost_not_mesh_density() {
    // The shape of the cost, which matters more than any single number for the
    // artist-facing resolution control in P3.
    //
    // Solve time barely moves with vertex count — 7399, 48670 and 213754
    // vertices all land between 320 and 491 ms — because the geodesic Dijkstra
    // runs over the VOXEL grid, whose size depends only on resolution. Only the
    // weight-assignment pass scales with vertices, and it is the cheapest of
    // the three (3, 18, 78 ms respectively).
    //
    // Resolution, by contrast, is cubic. Measured geodesic time:
    //
    //   res  64      81972 voxels      7 ms
    //   res 128     506730 voxels     39 ms
    //   res 192    1547238 voxels    131 ms
    //   res 256    3495312 voxels    295 ms
    //   res 384   11339739 voxels   1004 ms
    //
    // Doubling resolution costs roughly 8x. DEFAULT_RESOLUTION at ~300 ms
    // leaves an order of magnitude of headroom under the 3 s budget; 512 would
    // be the practical ceiling.
    let (mesh, rig) = {
        let _guard = MEASURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        (
            load_mesh(include_bytes!("fixtures/template-human-mesh.bin")),
            load_rig(include_bytes!("fixtures/template-human-rig.bin")),
        )
    };

    // The **fastest** of several runs, not one run. This asserts a *ratio* of
    // wall-clock timings, and load can only ever add time, so the minimum is the
    // estimator that survives a busy machine. Taking a single sample made this
    // fail once during a full-workspace run — 122 ms at 64 against 641 ms at
    // 256, a ratio of 5.3 where 8 is required — while passing three times in a
    // row in isolation. A gate that reddens under load is as corrosive as one
    // that cannot redden at all.
    let time_at = |res: u32| -> f32 {
        let _guard = MEASURE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        (0..3)
            .map(|_| {
                let t = std::time::Instant::now();
                let grid = VoxelGrid::build(&mesh, res).expect("grid");
                let _ = GeodesicField::compute(&mesh, &grid, &rig.bones).expect("field");
                t.elapsed().as_secs_f32() * 1000.0
            })
            .fold(f32::INFINITY, f32::min)
    };

    // Discarded: the first call pays rayon pool spin-up and first-touch page
    // faults, and it would otherwise inflate the denominator of the ratio below
    // — on only ~7 ms of real work.
    let _warmup = time_at(64);

    let coarse = time_at(64);
    let default_res = time_at(DEFAULT_RESOLUTION);

    // A 4x resolution increase is a 64x voxel increase. Anything close to
    // linear in resolution would mean the field is not being solved over the
    // whole grid.
    assert!(
        default_res > coarse * 8.0,
        "resolution scaling looks wrong: {coarse:.0} ms at 64 vs {default_res:.0} ms at {DEFAULT_RESOLUTION}"
    );

    // And the headroom that makes DEFAULT_RESOLUTION the right default.
    let ceiling_ms = if cfg!(debug_assertions) {
        20_000.0
    } else {
        1_000.0
    };
    assert!(
        default_res < ceiling_ms,
        "solve at DEFAULT_RESOLUTION took {default_res:.0} ms, ceiling {ceiling_ms:.0} ms"
    );
}
