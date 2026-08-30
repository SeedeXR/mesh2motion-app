//! Bounds-checked sequential reader over an FBX byte buffer.
//!
//! Every read is checked. The TypeScript original leans on `DataView`, which
//! throws on an out-of-range access; in Rust the equivalent would be a panic,
//! and a malformed file must produce an error instead (`memory/test.md` §4).

use crate::fbx::FbxError;

/// Reads little-endian values sequentially, refusing to run off the end.
pub struct Cursor<'a> {
    data: &'a [u8],
    offset: usize,
}

/// Generates a `read_*` method for a fixed-size little-endian scalar.
macro_rules! read_scalar {
    ($name:ident, $ty:ty, $size:literal) => {
        #[doc = concat!("Reads a little-endian `", stringify!($ty), "`.")]
        pub fn $name(&mut self) -> Result<$ty, FbxError> {
            // copy_from_slice rather than try_into().expect(): the crate's rule
            // is no panicking constructs outside tests, and "obviously
            // unreachable" is exactly what the P2-8 fuzz targets exist to
            // disprove.
            let mut buf = [0u8; $size];
            buf.copy_from_slice(self.take($size)?);
            Ok(<$ty>::from_le_bytes(buf))
        }
    };
}

impl<'a> Cursor<'a> {
    /// Wraps a buffer, positioned at the start.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    /// Current byte offset.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Total buffer length.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Bytes remaining after the cursor.
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.offset)
    }

    /// Advances without reading.
    ///
    /// # Errors
    ///
    /// Fails if the skip would pass the end of the buffer.
    pub fn skip(&mut self, len: usize) -> Result<(), FbxError> {
        self.take(len).map(|_| ())
    }

    /// Moves the cursor to an absolute offset.
    ///
    /// # Errors
    ///
    /// Fails if the offset is past the end of the buffer.
    pub fn seek(&mut self, offset: usize) -> Result<(), FbxError> {
        if offset > self.data.len() {
            return Err(FbxError::Truncated {
                needed: offset,
                available: self.data.len(),
            });
        }
        self.offset = offset;
        Ok(())
    }

    /// Borrows the next `len` bytes and advances past them.
    ///
    /// # Errors
    ///
    /// Fails if fewer than `len` bytes remain.
    pub fn take(&mut self, len: usize) -> Result<&'a [u8], FbxError> {
        let end = self.offset.checked_add(len).ok_or(FbxError::Truncated {
            needed: usize::MAX,
            available: self.data.len(),
        })?;
        if end > self.data.len() {
            return Err(FbxError::Truncated {
                needed: end,
                available: self.data.len(),
            });
        }
        let slice = &self.data[self.offset..end];
        self.offset = end;
        Ok(slice)
    }

    read_scalar!(read_u8, u8, 1);
    read_scalar!(read_i16, i16, 2);
    read_scalar!(read_i32, i32, 4);
    read_scalar!(read_u32, u32, 4);
    read_scalar!(read_i64, i64, 8);
    read_scalar!(read_u64, u64, 8);
    read_scalar!(read_f32, f32, 4);
    read_scalar!(read_f64, f64, 8);

    /// Reads a boolean.
    ///
    /// Only the low bit is examined: exporters disagree on the encoding, using
    /// 1/0 or `'Y'` (0x59) / `'T'` (0x54), which agree in that bit.
    pub fn read_bool(&mut self) -> Result<bool, FbxError> {
        Ok(self.read_u8()? & 1 == 1)
    }

    /// Reads a fixed-length string, truncating at the first NUL.
    ///
    /// FBX stores names as length-prefixed bytes that may be NUL-padded, and
    /// embeds a `\0\x01` separator inside object names. Invalid UTF-8 is
    /// replaced rather than rejected — a mangled name should not fail a load
    /// when everything else about the file is sound.
    pub fn read_string(&mut self, len: usize) -> Result<String, FbxError> {
        let bytes = self.take(len)?;
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_scalars_little_endian() {
        let data = [0x01u8, 0x02, 0x00, 0x03, 0x00, 0x00, 0x00];
        let mut c = Cursor::new(&data);
        assert_eq!(c.read_u8().unwrap(), 1);
        assert_eq!(c.read_i16().unwrap(), 2);
        assert_eq!(c.read_i32().unwrap(), 3);
        assert_eq!(c.remaining(), 0);
    }

    #[test]
    fn refuses_to_read_past_the_end() {
        let data = [0u8; 3];
        let mut c = Cursor::new(&data);
        assert!(c.read_i32().is_err(), "4 bytes from a 3-byte buffer");
        assert!(c.read_u64().is_err());
        // A failed read must not move the cursor.
        assert_eq!(c.offset(), 0);
        assert!(c.read_u8().is_ok());
    }

    #[test]
    fn take_cannot_overflow_the_offset() {
        // A hostile length near usize::MAX must error, not wrap the addition.
        let data = [0u8; 8];
        let mut c = Cursor::new(&data);
        c.skip(4).unwrap();
        assert!(c.take(usize::MAX).is_err());
        assert!(c.take(usize::MAX - 2).is_err());
    }

    #[test]
    fn strings_truncate_at_nul() {
        let data = b"name\0padpad";
        let mut c = Cursor::new(data);
        assert_eq!(c.read_string(11).unwrap(), "name");
        assert_eq!(c.remaining(), 0, "the full field width is consumed");
    }

    #[test]
    fn invalid_utf8_is_replaced_not_rejected() {
        let data = [0xffu8, 0xfe, b'a'];
        let mut c = Cursor::new(&data);
        let s = c.read_string(3).expect("must not fail");
        assert!(s.ends_with('a'));
    }

    #[test]
    fn booleans_read_the_low_bit_only() {
        // Exporters write 1/0 or 'Y'(0x59)/'T'(0x54); both agree in bit 0.
        let data = [1u8, 0, 0x59, 0x54];
        let mut c = Cursor::new(&data);
        assert!(c.read_bool().unwrap());
        assert!(!c.read_bool().unwrap());
        assert!(c.read_bool().unwrap(), "'Y' is true");
        assert!(!c.read_bool().unwrap(), "'T' is false");
    }

    #[test]
    fn seek_is_bounds_checked() {
        let data = [0u8; 4];
        let mut c = Cursor::new(&data);
        assert!(c.seek(4).is_ok(), "seeking to the end is valid");
        assert!(c.seek(5).is_err());
    }
}
