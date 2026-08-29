//! Shared loading for the binary fixtures produced by
//! `legacy/bench/dump-fixtures.ts`.
//!
//! `m2m-core` does no I/O by design (`memory/architecture.md` §2) and `m2m-io`
//! does not exist until P2, so real geometry reaches the tests as flat binary
//! rather than through a GLB loader.

#![allow(dead_code)]

use glam::Vec3;
use m2m_core::geodesic::BoneSegment;
use m2m_core::mesh::Mesh;

/// `[u32 vertexCount][u32 indexCount][f32 positions...][u32 indices...]`, LE.
pub fn load_mesh(bytes: &[u8]) -> Mesh {
    assert!(bytes.len() >= 8, "mesh fixture truncated");
    let vertex_count = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    let index_count = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;

    // Checked arithmetic: a corrupt header could otherwise wrap on a 32-bit
    // target and turn the size assertion into a slice-range panic.
    let pos_bytes = vertex_count
        .checked_mul(12)
        .and_then(|n| n.checked_add(8))
        .expect("mesh fixture header overflows");
    let idx_bytes = index_count
        .checked_mul(4)
        .and_then(|n| n.checked_add(pos_bytes))
        .expect("mesh fixture header overflows");
    assert_eq!(
        bytes.len(),
        idx_bytes,
        "mesh fixture size does not match its header"
    );

    let positions: Vec<f32> = bytes[8..pos_bytes]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    let indices: Vec<u32> = bytes[pos_bytes..idx_bytes]
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect();

    Mesh::from_flat(&positions, &indices).expect("fixture is a valid mesh")
}

/// A rig: bone segments plus which of them may receive weight.
pub struct Rig {
    pub bones: Vec<BoneSegment>,
    /// False for the root and for leaf bones, matching the legacy solver's
    /// exclusions. See the format note in `dump-fixtures.ts`.
    pub weightable: Vec<bool>,
}

/// `[u32 count][f32 head.xyz, tail.xyz]xN[u8 flags]xN`, LE. Flags: 1 root, 2 leaf.
pub fn load_rig(bytes: &[u8]) -> Rig {
    assert!(bytes.len() >= 4, "rig fixture truncated");
    let count = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;

    let seg_end = count
        .checked_mul(24)
        .and_then(|n| n.checked_add(4))
        .expect("rig fixture header overflows");
    let total = seg_end
        .checked_add(count)
        .expect("rig fixture header overflows");
    assert_eq!(
        bytes.len(),
        total,
        "rig fixture size does not match its header"
    );

    let f: Vec<f32> = bytes[4..seg_end]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    let bones = f
        .chunks_exact(6)
        .map(|b| BoneSegment {
            head: Vec3::new(b[0], b[1], b[2]),
            tail: Vec3::new(b[3], b[4], b[5]),
        })
        .collect();
    // Mask the two defined bits explicitly. Treating any unknown bit as
    // non-weightable would mean a future third flag silently disables every
    // bone, and the failure would surface as "at least one bone must be
    // allowed" from deep inside the solver rather than as a format mismatch.
    const ROOT: u8 = 1;
    const LEAF: u8 = 2;
    let weightable = bytes[seg_end..]
        .iter()
        .map(|&flag| {
            assert!(
                flag & !(ROOT | LEAF) == 0,
                "rig fixture has unknown flag bits {flag:#04b}; regenerate it"
            );
            flag & (ROOT | LEAF) == 0
        })
        .collect();

    Rig { bones, weightable }
}
