//! A schema-less FlatBuffers reader.
//!
//! Supercell's art containers carry FlatBuffers metadata, but the generated
//! schema is not published: field *names* live in the compiled client, not in
//! the buffer. What the buffer does carry is enough structure to walk it —
//! every table points backwards to a vtable listing where each field sits.
//!
//! So fields are addressed by index and interpreted by the caller. That is
//! deliberately unglamorous: it means reverse-engineering a container is a
//! matter of walking real files and recording what each index turns out to
//! mean, rather than guessing byte offsets that happen to work on one sample.
//!
//! Hardcoding offsets is specifically what this module exists to prevent. Two
//! SCTX files in the same version put their width at different absolute
//! offsets, because a field the encoder omits is simply absent from the
//! vtable and everything after it shifts.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FbError {
    #[error("buffer too short: need {need} bytes at {at}, have {len}")]
    OutOfBounds { at: usize, need: usize, len: usize },
    #[error("vtable offset {0} is outside the buffer")]
    BadVtable(i64),
    #[error("vtable at {at} declares size {size}, which is malformed")]
    MalformedVtable { at: usize, size: u16 },
}

type Result<T> = std::result::Result<T, FbError>;

/// A FlatBuffers buffer with a known table location.
#[derive(Debug, Clone, Copy)]
pub struct Table<'a> {
    buf: &'a [u8],
    /// Absolute offset of the table within `buf`.
    pos: usize,
    /// Absolute offset of this table's vtable.
    vtable: usize,
    /// Number of fields the vtable describes.
    field_count: usize,
}

impl<'a> Table<'a> {
    /// Reads the table at `pos`, resolving its vtable.
    ///
    /// A table begins with a signed offset *backwards* to its vtable, which is
    /// why this is `i32` and subtracted rather than added.
    pub fn at(buf: &'a [u8], pos: usize) -> Result<Table<'a>> {
        let soffset = read_i32(buf, pos)?;
        let vtable = (pos as i64) - (soffset as i64);
        if vtable < 0 || vtable as usize + 4 > buf.len() {
            return Err(FbError::BadVtable(vtable));
        }
        let vtable = vtable as usize;
        let vt_size = read_u16(buf, vtable)?;
        if vt_size < 4 || vtable + vt_size as usize > buf.len() {
            return Err(FbError::MalformedVtable {
                at: vtable,
                size: vt_size,
            });
        }
        Ok(Table {
            buf,
            pos,
            vtable,
            // The first two u16 entries are the vtable size and the inline
            // table size; the rest are one slot per field.
            field_count: (vt_size as usize - 4) / 2,
        })
    }

    /// Number of fields this table's vtable describes.
    ///
    /// Fields the encoder omitted are still counted, but read back as absent.
    pub fn field_count(&self) -> usize {
        self.field_count
    }

    /// Absolute offset of a field's value, or `None` when the field is absent.
    ///
    /// A zero slot means the field was not written, which is how FlatBuffers
    /// encodes a default and why field positions are not stable across files.
    pub fn field_offset(&self, index: usize) -> Result<Option<usize>> {
        if index >= self.field_count {
            return Ok(None);
        }
        let slot = read_u16(self.buf, self.vtable + 4 + index * 2)?;
        if slot == 0 {
            return Ok(None);
        }
        Ok(Some(self.pos + slot as usize))
    }

    /// Which field indices are actually present.
    ///
    /// The starting point for identifying an unknown container: dump these,
    /// compare across several real files, and see which stay constant.
    pub fn present_fields(&self) -> Vec<usize> {
        (0..self.field_count)
            .filter(|i| matches!(self.field_offset(*i), Ok(Some(_))))
            .collect()
    }

    pub fn u8_at(&self, index: usize) -> Result<Option<u8>> {
        match self.field_offset(index)? {
            None => Ok(None),
            Some(o) => Ok(Some(read_u8(self.buf, o)?)),
        }
    }

    pub fn u16_at(&self, index: usize) -> Result<Option<u16>> {
        match self.field_offset(index)? {
            None => Ok(None),
            Some(o) => Ok(Some(read_u16(self.buf, o)?)),
        }
    }

    pub fn u32_at(&self, index: usize) -> Result<Option<u32>> {
        match self.field_offset(index)? {
            None => Ok(None),
            Some(o) => Ok(Some(read_u32(self.buf, o)?)),
        }
    }

    pub fn i32_at(&self, index: usize) -> Result<Option<i32>> {
        match self.field_offset(index)? {
            None => Ok(None),
            Some(o) => Ok(Some(read_i32(self.buf, o)?)),
        }
    }

    pub fn f32_at(&self, index: usize) -> Result<Option<f32>> {
        Ok(self.u32_at(index)?.map(f32::from_bits))
    }

    /// Reads a string field.
    ///
    /// The field slot holds a relative offset to a length-prefixed, UTF-8,
    /// NUL-terminated string.
    pub fn str_at(&self, index: usize) -> Result<Option<&'a str>> {
        let Some(slot) = self.field_offset(index)? else {
            return Ok(None);
        };
        let rel = read_u32(self.buf, slot)? as usize;
        let start = slot + rel;
        let len = read_u32(self.buf, start)? as usize;
        let body = start + 4;
        let end = body.checked_add(len).ok_or(FbError::OutOfBounds {
            at: body,
            need: len,
            len: self.buf.len(),
        })?;
        if end > self.buf.len() {
            return Err(FbError::OutOfBounds {
                at: body,
                need: len,
                len: self.buf.len(),
            });
        }
        Ok(std::str::from_utf8(&self.buf[body..end]).ok())
    }

    /// Follows a field that points at a nested table.
    pub fn table_at(&self, index: usize) -> Result<Option<Table<'a>>> {
        let Some(slot) = self.field_offset(index)? else {
            return Ok(None);
        };
        let rel = read_u32(self.buf, slot)? as usize;
        Ok(Some(Table::at(self.buf, slot + rel)?))
    }

    /// Length of a vector field.
    pub fn vector_len(&self, index: usize) -> Result<Option<usize>> {
        let Some(slot) = self.field_offset(index)? else {
            return Ok(None);
        };
        let rel = read_u32(self.buf, slot)? as usize;
        Ok(Some(read_u32(self.buf, slot + rel)? as usize))
    }
}

/// Reads the root table of a standard FlatBuffers buffer.
///
/// `base` is where the FlatBuffers region begins, which is not necessarily
/// zero: Supercell prefixes its own header ahead of it.
pub fn root(buf: &[u8], base: usize) -> Result<Table<'_>> {
    let rel = read_u32(buf, base)? as usize;
    Table::at(buf, base + rel)
}

fn need(buf: &[u8], at: usize, n: usize) -> Result<()> {
    if at.checked_add(n).is_none_or(|end| end > buf.len()) {
        return Err(FbError::OutOfBounds {
            at,
            need: n,
            len: buf.len(),
        });
    }
    Ok(())
}

fn read_u8(buf: &[u8], at: usize) -> Result<u8> {
    need(buf, at, 1)?;
    Ok(buf[at])
}

fn read_u16(buf: &[u8], at: usize) -> Result<u16> {
    need(buf, at, 2)?;
    Ok(u16::from_le_bytes([buf[at], buf[at + 1]]))
}

fn read_u32(buf: &[u8], at: usize) -> Result<u32> {
    need(buf, at, 4)?;
    Ok(u32::from_le_bytes([
        buf[at],
        buf[at + 1],
        buf[at + 2],
        buf[at + 3],
    ]))
}

fn read_i32(buf: &[u8], at: usize) -> Result<i32> {
    Ok(read_u32(buf, at)? as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-built FlatBuffers buffer: one table, two present fields
    /// (u32 = 0x11223344 at index 0, u16 = 0x5566 at index 1) and one absent
    /// field at index 2.
    fn sample() -> Vec<u8> {
        let mut b = vec![0u8; 4]; // root offset, filled in below
        // vtable at 4: size=10, table size=12, slots 8, 12, 0
        let vt: [u16; 5] = [10, 12, 8, 12, 0];
        for v in vt {
            b.extend_from_slice(&v.to_le_bytes());
        }
        // table at 14: soffset back to vtable = 14 - 4 = 10
        let table_pos = b.len();
        b.extend_from_slice(&((table_pos as i32) - 4).to_le_bytes());
        b.extend_from_slice(&[0, 0, 0, 0]); // padding to slot 8
        b.extend_from_slice(&0x11223344u32.to_le_bytes()); // slot 8
        b.extend_from_slice(&0x5566u16.to_le_bytes()); // slot 12
        let root_rel = table_pos as u32;
        b[0..4].copy_from_slice(&root_rel.to_le_bytes());
        b
    }

    #[test]
    fn reads_present_fields_by_index() {
        let buf = sample();
        let t = root(&buf, 0).expect("root parses");
        assert_eq!(t.field_count(), 3);
        assert_eq!(t.u32_at(0).unwrap(), Some(0x11223344));
        assert_eq!(t.u16_at(1).unwrap(), Some(0x5566));
    }

    #[test]
    fn absent_field_reads_as_none_not_zero() {
        // The distinction that makes hardcoded offsets wrong: an omitted
        // field is absent, not a zero value at a predictable place.
        let buf = sample();
        let t = root(&buf, 0).expect("root parses");
        assert_eq!(t.u32_at(2).unwrap(), None);
        assert_eq!(t.field_offset(2).unwrap(), None);
        assert_eq!(t.present_fields(), vec![0, 1]);
    }

    #[test]
    fn index_past_the_vtable_is_absent_rather_than_an_error() {
        let buf = sample();
        let t = root(&buf, 0).expect("root parses");
        assert_eq!(t.u32_at(99).unwrap(), None);
    }

    #[test]
    fn truncated_buffer_errors_rather_than_panicking() {
        let buf = sample();
        for cut in 1..buf.len() {
            // Must never panic on malformed input, whatever the truncation.
            let _ = root(&buf[..cut], 0).map(|t| {
                let _ = t.u32_at(0);
                let _ = t.str_at(0);
                let _ = t.table_at(0);
            });
        }
    }

    #[test]
    fn rejects_a_vtable_pointing_outside_the_buffer() {
        let mut buf = sample();
        let root_rel = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
        // Point the table's vtable offset far past the buffer.
        buf[root_rel..root_rel + 4].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(matches!(root(&buf, 0), Err(FbError::BadVtable(_))));
    }
}
