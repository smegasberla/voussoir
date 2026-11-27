//! Patch application for the updater.
//!
//! A patch is a small script over the *old* binary: copy a run of bytes from it,
//! or insert new bytes. Applying one needs the old file, which the updater has
//! (it is the running binary), and produces the new file, which is then checked
//! against the manifest's digest and signature exactly as a full download would
//! be. The patch can therefore be hostile without being dangerous: anything it
//! gets wrong fails the signature.
//!
//! Format, little-endian:
//!
//! ```text
//! "VSD1"
//! repeated:
//!   0x01 offset:u32 length:u32   copy `length` bytes from old at `offset`
//!   0x02 length:u32 bytes        insert the following `length` bytes
//! ```
//!
//! There is no end marker: the patch ends with the file. The generator lives in
//! the CLI (`cli/src/delta.ts`), which is where a release is assembled.
//!
//! A patch whose operations deflate to fewer bytes is written as `"VSC1"`
//! followed by raw deflate (no gzip or zlib container, since the payload is the
//! whole file). A patch with little to compress stays `VSD1`, and both are
//! accepted here: a feed signed before compression existed keeps working, and a
//! reader that predates it keeps working against an uncompressed patch. The
//! inflation is bounded by the same output cap as the operations, so a crafted
//! stream cannot expand without limit.

use crate::Error;

const MAGIC: &[u8; 4] = b"VSD1";
/// The same operations, deflated. The generator writes it when that is smaller.
const COMPRESSED_MAGIC: &[u8; 4] = b"VSC1";
const COPY: u8 = 1;
const INSERT: u8 = 2;

/// The largest reconstructed binary accepted, matching the update cap.
pub const MAX_OUTPUT: usize = 256 * 1024 * 1024;

/// Reconstruct `old` plus `patch` into the new binary.
pub fn apply(old: &[u8], patch: &[u8]) -> Result<Vec<u8>, Error> {
    apply_capped(old, patch, MAX_OUTPUT)
}

/// [`apply`] with the output cap as a parameter, for tests.
pub fn apply_capped(old: &[u8], patch: &[u8], cap: usize) -> Result<Vec<u8>, Error> {
    // A patch is mostly inserted machine code, so the generator deflates the
    // operations when that is smaller and marks the file `VSC1`. A reader that
    // predates this only knows `VSD1`, which still applies unchanged, so both
    // shapes have to be understood here.
    let inflated;
    let patch = if patch.starts_with(COMPRESSED_MAGIC) {
        inflated = miniz_oxide::inflate::decompress_to_vec_with_limit(
            &patch[COMPRESSED_MAGIC.len()..],
            cap.saturating_add(MAGIC.len()),
        )
        .map_err(|error| {
            Error::Handler(format!(
                "update patch could not be decompressed: {:?}",
                error.status
            ))
        })?;
        &inflated[..]
    } else {
        patch
    };

    if !patch.starts_with(MAGIC) {
        return Err(Error::Handler(
            "update patch is not a Voussoir patch (missing VSD1 header)".to_owned(),
        ));
    }
    let mut cursor = &patch[MAGIC.len()..];
    let mut out = Vec::new();

    while !cursor.is_empty() {
        let tag = cursor[0];
        cursor = &cursor[1..];
        match tag {
            COPY => {
                let offset = read_u32(&mut cursor)? as usize;
                let length = read_u32(&mut cursor)? as usize;
                let end = offset.checked_add(length).ok_or_else(|| {
                    Error::Handler("update patch copy range overflows".to_owned())
                })?;
                if end > old.len() {
                    return Err(Error::Handler(format!(
                        "update patch copies {end} bytes but the old binary is {}",
                        old.len()
                    )));
                }
                if out.len() + length > cap {
                    return Err(Error::Handler(format!(
                        "update patch produces more than {cap} bytes"
                    )));
                }
                out.extend_from_slice(&old[offset..end]);
            }
            INSERT => {
                let length = read_u32(&mut cursor)? as usize;
                if length > cursor.len() {
                    return Err(Error::Handler(format!(
                        "update patch inserts {length} bytes but only {} remain",
                        cursor.len()
                    )));
                }
                if out.len() + length > cap {
                    return Err(Error::Handler(format!(
                        "update patch produces more than {cap} bytes"
                    )));
                }
                out.extend_from_slice(&cursor[..length]);
                cursor = &cursor[length..];
            }
            other => {
                return Err(Error::Handler(format!(
                    "update patch has an unknown opcode {other}"
                )))
            }
        }
    }

    Ok(out)
}

fn read_u32(cursor: &mut &[u8]) -> Result<u32, Error> {
    if cursor.len() < 4 {
        return Err(Error::Handler(
            "update patch ends in the middle of a length".to_owned(),
        ));
    }
    let (head, rest) = cursor.split_at(4);
    *cursor = rest;
    Ok(u32::from_le_bytes([head[0], head[1], head[2], head[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch(ops: &[(u8, &[u8])]) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        for (tag, bytes) in ops {
            out.push(*tag);
            out.extend_from_slice(bytes);
        }
        out
    }

    #[test]
    fn copies_and_inserts_rebuild_a_binary() {
        let old = b"the-old-binary-tail";
        let p = patch(&[
            (COPY, &[0, 0, 0, 0, 3, 0, 0, 0]),           // "the"
            (INSERT, &[3, 0, 0, 0, b'n', b'e', b'w']),     // "new"
            (COPY, &[3, 0, 0, 0, 12, 0, 0, 0]),            // "-old-binary-"
        ]);
        assert_eq!(apply(old, &p).unwrap(), b"thenew-old-binary-");
    }

    #[test]
    fn an_empty_op_list_copies_nothing_and_succeeds() {
        assert_eq!(apply(b"old", MAGIC).unwrap(), b"");
    }

    #[test]
    fn a_copy_past_the_old_binary_is_refused() {
        let p = patch(&[(COPY, &[0, 0, 0, 0, 99, 0, 0, 0])]);
        let error = apply(b"old", &p).unwrap_err().to_string();
        assert!(error.contains("old binary is 3"), "{error}");
    }

    #[test]
    fn malformed_patches_say_what_is_wrong() {
        let error = apply(b"old", b"NOPE").unwrap_err().to_string();
        assert!(error.contains("missing VSD1"), "{error}");

        let truncated = patch(&[(COPY, &[0, 0])]);
        assert!(apply(b"old", &truncated).unwrap_err().to_string().contains("length"));

        let unknown = patch(&[(7, &[])]);
        assert!(apply(b"old", &unknown).unwrap_err().to_string().contains("unknown opcode"));

        let short_insert = patch(&[(INSERT, &[5, 0, 0, 0, b'a'])]);
        assert!(apply(b"old", &short_insert).unwrap_err().to_string().contains("only 1 remain"));
    }

    #[test]
    fn the_output_cap_applies_to_both_opcodes() {
        let p = patch(&[
            (COPY, &[0, 0, 0, 0, 3, 0, 0, 0]),
            (INSERT, &[2, 0, 0, 0, b'a', b'b']),
        ]);
        assert!(apply_capped(b"old", &p, 4).is_err());
        assert!(apply_capped(b"old", &p, 5).is_ok());
    }
}
