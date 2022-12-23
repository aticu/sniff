//! Reads and represents data of interest for files.

use arrayvec::ArrayVec;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use std::{
    fmt, fs,
    io::{self, Read},
    mem,
    path::Path,
};

/// The number of bytes that will be stored in the `first_bytes` field.
const FIRST_BYTES_LEN: usize = 16;

/// Stores information about a file.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub(crate) struct File {
    /// The SHA2-256 hash of the files contents.
    pub(crate) sha2_256_hash: Sha256Hash,
    /// The MD5 hash of the files contents.
    pub(crate) md5_hash: Md5Hash,
    /// The first bytes of the file.
    pub(crate) first_bytes: ArrayVec<u8, FIRST_BYTES_LEN>,
    /// The flags stored about a file.
    pub(crate) flags: FileFlags,
    /// The entropy of the file.
    pub(crate) entropy: f32,
    /// The COFF-header of the file, if it exists.
    ///
    /// DOES NOT include the PE-header itself (it starts with the `Machine` field).
    /// See [the
    /// documentation](https://docs.microsoft.com/en-us/windows/win32/debug/pe-format) for more
    /// details.
    pub(crate) coff_header: Option<Vec<u8>>,
}

/// A common trait that all implementations of files should fulfill.
pub(crate) trait GenericFile: Serialize + DeserializeOwned + Clone + Sized + Send {
    /// Reads the file information from the specified path.
    fn from_path(path: impl AsRef<Path>) -> io::Result<Self>;
}

impl GenericFile for File {
    /// Reads the file information from the specified path.
    fn from_path(path: impl AsRef<Path>) -> io::Result<Self> {
        use sha2::Digest as _;

        let path = path.as_ref();

        let mut first_bytes = ArrayVec::new();
        let mut sha256hasher = sha2::Sha256::new();
        let mut md5hasher = md5::Md5::new();
        let mut byte_occurrences = [0u64; 256];

        let mut last_utf8_bytes = ArrayVec::<u8, 4>::new();
        let mut last_utf16le_bytes = ArrayVec::<u8, 4>::new();
        let mut last_utf16be_bytes = ArrayVec::<u8, 4>::new();
        let mut last_utf32_bytes = ArrayVec::<u8, 4>::new();

        let mut flags = FileFlags::UTF_ENCODING;

        let file = fs::File::open(path)?;
        let buf_reader = io::BufReader::new(&file);

        for byte in buf_reader.bytes() {
            let byte = byte?;

            sha256hasher.update([byte]);
            md5hasher.update([byte]);
            first_bytes.try_push(byte).ok();
            byte_occurrences[byte as usize] += 1;

            if flags.contains(FileFlags::UTF8) {
                last_utf8_bytes.push(byte);
                match std::str::from_utf8(&last_utf8_bytes) {
                    Ok(_) => last_utf8_bytes.clear(),
                    Err(err) if err.error_len().is_some() => flags.remove(FileFlags::UTF8),
                    Err(_) => (),
                }
            }

            fn valid_utf16(bytes: &[u8], convert: impl Fn([u8; 2]) -> u16) -> Option<bool> {
                if bytes.len() == 2 {
                    if let Some(Ok(_)) =
                        char::decode_utf16([convert(bytes[..].try_into().unwrap())]).next()
                    {
                        Some(true)
                    } else {
                        None
                    }
                } else if bytes.len() == 4 {
                    match char::decode_utf16([
                        convert(bytes[..2].try_into().unwrap()),
                        convert(bytes[2..].try_into().unwrap()),
                    ])
                    .next()
                    {
                        Some(Ok(_)) => Some(true),
                        _ => Some(false),
                    }
                } else {
                    None
                }
            }

            if flags.contains(FileFlags::UTF16BE) {
                last_utf16be_bytes.push(byte);
                match valid_utf16(&last_utf16be_bytes, u16::from_be_bytes) {
                    Some(true) => last_utf16be_bytes.clear(),
                    Some(false) => flags.remove(FileFlags::UTF16BE),
                    None => (),
                }
            }

            if flags.contains(FileFlags::UTF16LE) {
                last_utf16le_bytes.push(byte);
                match valid_utf16(&last_utf16le_bytes, u16::from_le_bytes) {
                    Some(true) => last_utf16le_bytes.clear(),
                    Some(false) => flags.remove(FileFlags::UTF16LE),
                    None => (),
                }
            }

            if flags.contains(FileFlags::UTF32BE) || flags.contains(FileFlags::UTF32LE) {
                last_utf32_bytes.push(byte);
                if last_utf32_bytes.len() == 4 {
                    let be = u32::from_be_bytes(last_utf32_bytes[..].try_into().unwrap());
                    if char::from_u32(be).is_none() {
                        flags.remove(FileFlags::UTF32BE)
                    }

                    let le = u32::from_le_bytes(last_utf32_bytes[..].try_into().unwrap());
                    if char::from_u32(le).is_none() {
                        flags.remove(FileFlags::UTF32LE)
                    }
                    last_utf32_bytes.clear();
                }
            }
        }

        if !last_utf16be_bytes.is_empty() {
            flags.remove(FileFlags::UTF16BE);
        }
        if !last_utf16le_bytes.is_empty() {
            flags.remove(FileFlags::UTF16LE);
        }
        if !last_utf32_bytes.is_empty() {
            flags.remove(FileFlags::UTF32BE);
            flags.remove(FileFlags::UTF32LE);
        }

        let total_bytes = byte_occurrences.iter().sum::<u64>() as f64;
        let mut entropy = -byte_occurrences
            .into_iter()
            .filter(|&num| num != 0)
            .map(|num| {
                let p = num as f64 / total_bytes;

                p * p.log2()
            })
            .sum::<f64>();

        if entropy.is_sign_negative() {
            entropy = -entropy;
        }

        let sha2_256_hash = Sha256Hash {
            bytes: sha256hasher.finalize().into(),
        };

        let md5_hash = Md5Hash {
            bytes: md5hasher.finalize().into(),
        };

        let coff_header = extract_coff_header(&file).ok().flatten();

        Ok(Self {
            sha2_256_hash,
            md5_hash,
            first_bytes,
            flags,
            entropy: entropy as f32,
            coff_header,
        })
    }
}

impl Eq for File {}

bitflags::bitflags! {
    /// The flags of information stored about a file.
    #[rustfmt::skip]
    #[derive(Deserialize, Serialize)]
    #[repr(transparent)]
    pub(crate) struct FileFlags: u16 {
        /// Whether the file contains valid UTF-8.
        const UTF8                = 0x0001;
        /// Whether the file contains valid UTF-16BE.
        const UTF16BE             = 0x0002;
        /// Whether the file contains valid UTF-16LE.
        const UTF16LE             = 0x0004;
        /// Whether the file contains valid UTF-32BE.
        const UTF32BE             = 0x0008;
        /// Whether the file contains valid UTF-32LE.
        const UTF32LE             = 0x0010;
        /// The union of the UTF encoding flags.
        const UTF_ENCODING        = 0x001f;
    }
}

/// Represents a SHA2-256 hash.
#[derive(PartialEq, Eq, Clone, Serialize, Deserialize)]
pub(crate) struct Sha256Hash {
    /// The bytes of the hash.
    pub(crate) bytes: [u8; 32],
}

impl fmt::Debug for Sha256Hash {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for byte in self.bytes {
            write!(f, "{byte:02x}")?;
        }

        Ok(())
    }
}

/// Represents an MD5 hash.
#[derive(PartialEq, Eq, Clone, Serialize, Deserialize)]
pub(crate) struct Md5Hash {
    /// The bytes of the hash.
    pub(crate) bytes: [u8; 16],
}

impl fmt::Debug for Md5Hash {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for byte in self.bytes {
            write!(f, "{byte:02x}")?;
        }

        Ok(())
    }
}

/// Extracts the PE header from the given file.
fn extract_coff_header(file: &fs::File) -> io::Result<Option<Vec<u8>>> {
    use std::os::unix::prelude::FileExt as _;

    /// The offset into a MZ-header where the offset of the PE-header is listed.
    const MZ_PE_OFFSET: u64 = 0x3c;

    /// The length of the standard COFF-header.
    const COFF_HEADER_LEN: usize = 20;

    /// The maximum length of a COFF optional header that will still be recorded.
    ///
    /// Note that longer headers are theoretically possible, but won't be considered here.
    const COFF_OPTIONAL_HEADER_MAX_LEN: u16 = 256;

    /// The magic bytes of a PE-file.
    const PE_MAGIC: &[u8] = b"PE\x00\x00";

    /// The magic bytes of an MZ-file.
    const MZ_MAGIC: &[u8] = b"MZ";

    let mut u16_buf = [0; mem::size_of::<u16>()];
    let mut u32_buf = [0; mem::size_of::<u32>()];

    assert_eq!(u16_buf.len(), MZ_MAGIC.len());
    file.read_exact_at(&mut u16_buf, 0)?;
    if u16_buf != MZ_MAGIC {
        return Ok(None);
    }

    file.read_exact_at(&mut u32_buf, MZ_PE_OFFSET)?;
    let pe_offset = u32::from_le_bytes(u32_buf);

    assert_eq!(u32_buf.len(), PE_MAGIC.len());
    file.read_exact_at(&mut u32_buf, pe_offset.into())?;
    if u32_buf != PE_MAGIC {
        return Ok(None);
    }

    let header_start = u64::from(pe_offset) + PE_MAGIC.len() as u64;
    file.read_exact_at(&mut u16_buf, header_start + 16)?;
    let optional_header_len = u16::from_le_bytes(u16_buf).clamp(0, COFF_OPTIONAL_HEADER_MAX_LEN);
    let mut total_headers = vec![0; COFF_HEADER_LEN + usize::from(optional_header_len)];
    file.read_exact_at(&mut total_headers, header_start)?;
    Ok(Some(total_headers))
}
