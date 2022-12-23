//! Representation of the different types of a directory entry.

use std::{fmt, io, path::Path};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// The types of directory entries that can occur.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
pub(crate) enum DirEntryType {
    /// The directory entry refers to a file.
    File,
    /// The directory entry refers to a symlink.
    Symlink,
    /// The directory entry refers to a directory.
    Directory,
    /// The directory entry refers to a block device.
    BlockDevice,
    /// The directory entry refers to a character device.
    CharacterDevice,
    /// The directory entry refers to a pipe.
    Pipe,
    /// The directory entry refers to a socket.
    Socket,
    /// The directory entry is of an unknown type.
    Unknown,
}

/// A common trait that all implementations of directory entry types.
pub(crate) trait GenericDirEntryType:
    Serialize + DeserializeOwned + Clone + Copy + Sized + Send
{
    /// Reads the entry type information from the specified path.
    fn from_path(path: impl AsRef<Path>) -> io::Result<Self>;

    /// Returns the directory entry type of a file.
    fn file() -> Self;

    /// Returns the directory entry type of a symlink.
    fn symlink() -> Self;

    /// Returns the directory entry type of a directory.
    fn directory() -> Self;
}

impl GenericDirEntryType for DirEntryType {
    fn from_path(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();

        let meta = Path::symlink_metadata(path)?;

        use std::os::unix::fs::FileTypeExt as _;

        let file_type = meta.file_type();

        Ok(if file_type.is_block_device() {
            DirEntryType::BlockDevice
        } else if file_type.is_char_device() {
            DirEntryType::CharacterDevice
        } else if file_type.is_fifo() {
            DirEntryType::Pipe
        } else if file_type.is_socket() {
            DirEntryType::Socket
        } else {
            DirEntryType::Unknown
        })
    }

    fn file() -> Self {
        DirEntryType::File
    }

    fn symlink() -> Self {
        DirEntryType::Symlink
    }

    fn directory() -> Self {
        DirEntryType::Directory
    }
}

impl fmt::Display for DirEntryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DirEntryType::File => write!(f, "file"),
            DirEntryType::Symlink => write!(f, "symlink"),
            DirEntryType::Directory => write!(f, "directory"),
            DirEntryType::BlockDevice => write!(f, "block device"),
            DirEntryType::CharacterDevice => write!(f, "character device"),
            DirEntryType::Pipe => write!(f, "pipe"),
            DirEntryType::Socket => write!(f, "socket"),
            DirEntryType::Unknown => write!(f, "unknown"),
        }
    }
}
