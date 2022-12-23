//! Models the filesystem in snapshots.
//!
//! # Design of this module
//!
//! The types in the module are meant to be serialized, thus backwards compatibility needs to be
//! considered in the design.
//!
//! The first version of this module (still available under `fsv0`) used fixed non-generic types
//! for everything.
//! The problem with that approach is that it results in a lot of code duplication.
//! For example if the information that is stored about a file is changed, then the `File` type is
//! changed.
//! However `DirEntry` and `Directory` types would also be changed, since their inner type is
//! changed.
//! So even for a small change in `File` a lot of types would have to be copied and changed, since
//! the old code must be kept around to still be able to read old serialized version.
//!
//! To get around the issue of code duplication, this module is designed with generic types in
//! mind.
//! For example the type of a regular directory entry with metadata is
//! `MetaDirEntry<DirEntry<Metadata, File, Symlink, DirEntryType>, Metadata>`.
//! This way, if the definition of `File` changes, no other code is affected, since the new type
//! could simply be `MetaDirEntry<DirEntry<Metadata, FileV2, Symlink, DirEntryType>, Metadata>`.
//!
//! This design implies that `DirEntry` must be generic over its members and that the `Directory`
//! type is generic over the entries.
//! Also everything must be generic over the metadata.
//! To facilitate this, each type of item gets its own trait (prefixed by "Generic" for example
//! `GenericFile`).
//! This allows users of the types to still interact with the types, even though they are generic.
//! Note that these traits can be changed more freely than the types, since they do not affect the
//! serialization.
//!
//! The only two container types that know directly about each other in this module are
//! `MetaDirEntry` and `Directory`.
//! This is done because it significantly reduces the implementation complexity and these
//! containers are so basic, that they are unlikely to be changed.

use std::{
    ffi::OsStr,
    fmt,
    path::{Path, PathBuf},
};

pub(crate) use dir_entry::DirEntry;
pub(crate) use directory::{Directory, MetaDirEntry};
pub(crate) use file::File;
pub(crate) use metadata::Metadata;
pub(crate) use symlink::Symlink;

pub(crate) mod dir_entry;
pub(crate) mod dir_entry_type;
pub(crate) mod directory;
pub(crate) mod file;
pub(crate) mod metadata;
pub(crate) mod symlink;

/// A shorthand for the latest version of the directory entry type.
pub(crate) type MetaDEntry<Context = ()> = MetaDirEntry<DEntry<Context>, Metadata, Context>;

/// A shorthand for the latest version of the directory entry type.
pub(crate) type DEntry<Context = ()> =
    DirEntry<Metadata, File, Symlink, dir_entry_type::DirEntryType, Context>;

/// The type of a directory entry with metadata of version 1.
pub(crate) type MetaDEntryV1<Context = ()> =
    MetaDirEntry<DEntryV1<Context>, metadata::MetadataV1, Context>;

/// The directory entry of version 1.
pub(crate) type DEntryV1<Context = ()> =
    DirEntry<metadata::MetadataV1, File, Symlink, dir_entry_type::DirEntryType, Context>;

/// The directory type of version 1.
pub(crate) type DirectoryV1<Context = ()> =
    Directory<DEntryV1<Context>, metadata::MetadataV1, Context>;

impl<Context> From<MetaDEntryV1<Context>> for MetaDEntry<Context> {
    fn from(entry: MetaDEntryV1<Context>) -> Self {
        Self {
            entry: entry.entry.into(),
            metadata: entry.metadata.into(),
            context: entry.context,
        }
    }
}

impl<Context> From<DEntryV1<Context>> for DEntry<Context> {
    fn from(entry: DEntryV1<Context>) -> Self {
        match entry {
            DirEntry::File(file) => DirEntry::File(file),
            DirEntry::Symlink(symlink) => DirEntry::Symlink(symlink),
            DirEntry::Directory(directory) => DirEntry::Directory(directory.into()),
            DirEntry::Other(ty) => DirEntry::Other(ty),
        }
    }
}

impl<Context> From<DirectoryV1<Context>> for Directory<DEntry<Context>, Metadata, Context> {
    fn from(dir: DirectoryV1<Context>) -> Self {
        let mut entries = std::collections::BTreeMap::new();

        for (name, entry) in dir.entries {
            entries.insert(name, entry.into());
        }

        Self { entries }
    }
}

impl<Context> MetaDEntry<Context> {
    /// Clones this entry, annotating each node with the context given to it by `ctx`.
    pub(crate) fn with_context<NewContext>(
        &self,
        ctx: &mut impl FnMut(&Self) -> NewContext,
    ) -> MetaDEntry<NewContext> {
        let entry = match &self.entry {
            DirEntry::File(file) => DirEntry::File(file.clone()),
            DirEntry::Symlink(symlink) => DirEntry::Symlink(symlink.clone()),
            DirEntry::Directory(Directory { entries }) => DirEntry::Directory(Directory {
                entries: entries
                    .iter()
                    .map(|(name, entry)| (name.clone(), entry.with_context(ctx)))
                    .collect(),
            }),
            DirEntry::Other(other) => DirEntry::Other(*other),
        };

        MetaDEntry {
            entry,
            metadata: self.metadata.clone(),
            context: ctx(self),
        }
    }

    /// Returns `true` if this directory entry is a file.
    pub(crate) fn is_file(&self) -> bool {
        matches!(self.entry, DirEntry::File(_))
    }

    /// Returns `true` if this directory entry is a directory.
    pub(crate) fn is_dir(&self) -> bool {
        matches!(self.entry, DirEntry::Directory(_))
    }
}

/// Converts a windows path to a workable path for internal uses.
pub(crate) fn convert_windows_path(path: &str) -> PathBuf {
    let path = path
        .strip_prefix("C:")
        .unwrap_or_else(|| path.strip_prefix("c:").unwrap_or(path));

    PathBuf::from(path.replace('\\', "/"))
}

/// An extension trait implemented to make dealing with `OsStr` easier.
pub(crate) trait OsStrExt {
    /// Returns true if the `OsStr` has the given extension.
    fn has_extension(&self, extension: impl AsRef<str>) -> bool;

    /// Returns a type implementing `Display` to display an `OsStr` without quotes.
    fn display(&self) -> DisplayOsStr;

    /// Returns a case-normalized string of the given `OsStr`.
    fn normalize(&self) -> Option<String>;
}

impl OsStrExt for OsStr {
    fn has_extension(&self, extension: impl AsRef<str>) -> bool {
        Path::new(self).extension() == Some(OsStr::new(extension.as_ref()))
    }

    fn display(&self) -> DisplayOsStr {
        DisplayOsStr(self)
    }

    fn normalize(&self) -> Option<String> {
        self.to_str()
            .map(|string| string.chars().map(|c| c.casefold()).collect::<String>())
    }
}

/// The bytes of an NFTS `$UpCase` file, used for case folding in normalized paths.
const UPCASE: &[u8] = include_bytes!("../ntfs_upcase");

/// An extension trait for `char` case folding.
trait CharExt {
    /// Normalize the case of `self`.
    fn casefold(self) -> Self;
}

impl CharExt for char {
    fn casefold(self) -> Self {
        let idx = u32::from(self) as usize * 2;
        if UPCASE.len() > idx + 1 {
            let folded = u16::from_le_bytes([UPCASE[idx], UPCASE[idx + 1]]);

            char::from_u32(folded as u32).unwrap_or(self)
        } else {
            self
        }
    }
}

/// A wrapper to use the `display` method of `Path` for the given `&OsStr`.
pub(crate) struct DisplayOsStr<'s>(&'s OsStr);

impl fmt::Display for DisplayOsStr<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", std::path::Path::new(self.0).display())
    }
}
