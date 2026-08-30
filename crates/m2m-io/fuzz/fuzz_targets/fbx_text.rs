#![no_main]
//! The ASCII FBX reader on arbitrary text.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The reader takes &str, so feed it only what decodes. Fuzzing invalid
    // UTF-8 here would test `from_utf8`, not the parser; `fbx_binary` covers
    // the byte-level entry point.
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = m2m_io::fbx::text::parse(text);
    }
});
