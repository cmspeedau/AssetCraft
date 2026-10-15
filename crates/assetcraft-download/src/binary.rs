//! A cursor over a byte slice, shared by the binary artifact parsers.
//!
//! Fab hands back two binary formats: the manifest that indexes a download and
//! the chunk files it points at. Both are little-endian with length-prefixed
//! strings, so the reading primitives live here and the layout lives in
//! [`crate::manifest`] and [`crate::chunk`].

use std::io::Read;

use assetcraft_core::{Error, Result};

/// An error for bytes that do not follow the documented layout.
pub(crate) fn corrupt(detail: impl std::fmt::Display) -> Error {
    Error::Artifact(detail.to_string())
}

/// Decompress a zlib-compressed body.
pub(crate) fn inflate(data: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = flate2::read::ZlibDecoder::new(data);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|err| corrupt(format!("body is not valid zlib: {err}")))?;
    Ok(out)
}

/// A cursor over a manifest's or chunk's bytes.
pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub(crate) fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| corrupt("length overflow"))?;
        if end > self.data.len() {
            return Err(corrupt(format!(
                "unexpected end of data: needed {len} bytes, {} left",
                self.remaining()
            )));
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    pub(crate) fn skip(&mut self, len: usize) -> Result<()> {
        self.take(len).map(|_| ())
    }

    pub(crate) fn seek(&mut self, pos: usize) -> Result<()> {
        if pos > self.data.len() {
            return Err(corrupt(format!(
                "cannot seek to {pos} in {} bytes",
                self.data.len()
            )));
        }
        self.pos = pos;
        Ok(())
    }

    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub(crate) fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub(crate) fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub(crate) fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.array()?))
    }

    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let slice = self.take(N)?;
        Ok(slice
            .try_into()
            .expect("take returned exactly the requested number of bytes"))
    }

    /// A GUID, four little-endian 32-bit words.
    pub(crate) fn guid(&mut self) -> Result<[u32; 4]> {
        let mut guid = [0u32; 4];
        for word in &mut guid {
            *word = self.u32()?;
        }
        Ok(guid)
    }

    /// A length-prefixed string. A positive length is ASCII/UTF-8 with a NUL
    /// terminator; a negative length is UTF-16 with a two-byte terminator.
    pub(crate) fn fstring(&mut self) -> Result<String> {
        let length = self.i32()?;
        if length == 0 {
            return Ok(String::new());
        }
        if length > 0 {
            let bytes = self.take(length as usize)?;
            // The final byte is the NUL terminator.
            let text = &bytes[..bytes.len() - 1];
            return Ok(String::from_utf8_lossy(text).into_owned());
        }
        // The count includes the two-byte terminator.
        let units = (length as i64).unsigned_abs() as usize;
        let bytes = self.take(
            units
                .checked_mul(2)
                .ok_or_else(|| corrupt("string too long"))?,
        )?;
        let mut code_units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        code_units.pop();
        Ok(String::from_utf16_lossy(&code_units))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian_values_and_a_guid() {
        let mut bytes = Vec::new();
        bytes.push(0x7F);
        bytes.extend_from_slice(&0x0102_0304u32.to_le_bytes());
        bytes.extend_from_slice(&0x1122_3344_5566_7788u64.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&4u32.to_le_bytes());

        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.u8().unwrap(), 0x7F);
        assert_eq!(reader.u32().unwrap(), 0x0102_0304);
        assert_eq!(reader.u64().unwrap(), 0x1122_3344_5566_7788);
        assert_eq!(reader.guid().unwrap(), [1, 2, 3, 4]);
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn reading_past_the_end_is_an_error() {
        let mut reader = Reader::new(&[1, 2, 3]);
        assert!(reader.u32().is_err());
    }

    #[test]
    fn fstring_reads_utf16_when_the_length_is_negative() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(-5i32).to_le_bytes());
        for unit in [0x0052u16, 0x006Fu16, 0x0063u16, 0x006Bu16] {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);
        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.fstring().unwrap(), "Rock");
    }
}
