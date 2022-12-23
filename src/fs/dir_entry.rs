//! Representation of a single directory entry.

use std::{
    io,
    path::{Path, PathBuf},
};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::{
    dir_entry_type::GenericDirEntryType, file::GenericFile, metadata::GenericMetadata,
    symlink::GenericSymlink, Directory,
};

/// Represents an entry in a directory without its metadata.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub(crate) enum DirEntry<Metadata, File, Symlink, DirEntryType, Context> {
    /// The directory entry is a file.
    File(File),
    /// The directory entry is a symlink.
    Symlink(Symlink),
    /// The directory entry is a directory.
    Directory(Directory<Self, Metadata, Context>),
    /// The directory entry is something else.
    Other(DirEntryType),
}

/// A common trait that all implementations of directory entries should fulfill.
pub(crate) trait GenericDirEntry<Metadata, Context>:
    Serialize + DeserializeOwned + Clone + Sized + Send
{
    /// Represents all possible types of directory entries.
    type DirEntryType;

    /// Merges this entry into the given `other` entry.
    fn merge_into(self, other: &mut Self);

    /// Reads the directory entry at the given path.
    fn from_path(
        path: impl AsRef<Path>,
        trim_symlink_path: impl FnMut(PathBuf) -> PathBuf,
    ) -> io::Result<Self>;

    /// Returns a reference to the inner directory, if the entry is a directory.
    fn dir(&self) -> Option<&Directory<Self, Metadata, Context>>;

    /// Returns a mutable reference to the inner directory, if the entry is a directory.
    fn dir_mut(&mut self) -> Option<&mut Directory<Self, Metadata, Context>>;

    /// Returns the inner directory, if the entry is a directory.
    fn into_dir(self) -> Option<Directory<Self, Metadata, Context>>;

    /// Returns a directory entry representing an empty directory.
    fn empty_dir() -> Self;

    /// Returns the type of the directory entry.
    fn entry_type(&self) -> Self::DirEntryType;
}

impl<
        Metadata: GenericMetadata,
        File: GenericFile,
        Symlink: GenericSymlink,
        DirEntryType: GenericDirEntryType,
        Context: Serialize + DeserializeOwned + Clone + Sized + Send,
    > GenericDirEntry<Metadata, Context>
    for DirEntry<Metadata, File, Symlink, DirEntryType, Context>
{
    type DirEntryType = DirEntryType;

    fn merge_into(self, other: &mut Self) {
        match (self, other) {
            (DirEntry::Directory(self_dir), DirEntry::Directory(other_dir)) => {
                for (name, entry) in self_dir.entries {
                    other_dir.entries.insert(name, entry);
                }
            }
            (this, other) => {
                *other = this;
            }
        }
    }

    fn from_path(
        path: impl AsRef<Path>,
        trim_symlink_path: impl FnMut(PathBuf) -> PathBuf,
    ) -> io::Result<Self> {
        let path = path.as_ref();

        let os_meta = Path::symlink_metadata(path)?;

        if os_meta.is_symlink() {
            let mut symlink = Symlink::from_path(path)?;
            symlink.update_path(trim_symlink_path);

            Ok(DirEntry::Symlink(symlink))
        } else if os_meta.is_file() {
            let file = File::from_path(path)?;

            Ok(DirEntry::File(file))
        } else if os_meta.is_dir() {
            let dir = Directory::default();

            Ok(DirEntry::Directory(dir))
        } else {
            let ty = DirEntryType::from_path(path)?;

            Ok(DirEntry::Other(ty))
        }
    }

    fn dir(&self) -> Option<&Directory<Self, Metadata, Context>> {
        match self {
            DirEntry::Directory(dir) => Some(dir),
            _ => None,
        }
    }

    fn dir_mut(&mut self) -> Option<&mut Directory<Self, Metadata, Context>> {
        match self {
            DirEntry::Directory(dir) => Some(dir),
            _ => None,
        }
    }

    fn into_dir(self) -> Option<Directory<Self, Metadata, Context>> {
        match self {
            DirEntry::Directory(dir) => Some(dir),
            _ => None,
        }
    }

    fn empty_dir() -> Self {
        DirEntry::Directory(Directory::default())
    }

    fn entry_type(&self) -> DirEntryType {
        match self {
            DirEntry::File(_) => DirEntryType::file(),
            DirEntry::Symlink(_) => DirEntryType::symlink(),
            DirEntry::Directory(_) => DirEntryType::directory(),
            DirEntry::Other(ty) => *ty,
        }
    }
}
