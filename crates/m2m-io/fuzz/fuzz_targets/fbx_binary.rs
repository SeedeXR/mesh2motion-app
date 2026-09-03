#![no_main]
//! The binary FBX reader on arbitrary bytes.
//!
//! The contract is `memory/test.md` §4: a malformed file returns an error. It
//! never panics, never hangs, never exhausts memory.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The result is deliberately unused: the only assertion is that getting
    // here at all — rather than aborting — was possible. libFuzzer treats a
    // panic, a timeout, or an OOM as the failure.
    let _ = m2m_io::fbx::binary::parse(data);
});
