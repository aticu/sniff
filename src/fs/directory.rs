//! Reads and represents data of interest for directories.

use serde::{Deserialize, Serialize};

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    io, iter,
    ops::Deref,
    path::{self, Path, PathBuf},
};

use super::{dir_entry::GenericDirEntry, metadata::GenericMetadata};

mod walker;

/// Represents an entry in a directory with its metadata.
///
/// The `context` field represents arbitrary context that can be attached to metadata entries.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub(crate) struct MetaDirEntry<DirEntry, Metadata, Context> {
    /// The directory entry.
    pub(crate) entry: DirEntry,
    /// The metadata of the entry.
    pub(crate) metadata: Metadata,
    /// The metadata of the entry.
    pub(crate) context: Context,
}

impl<Metadata, DirEntry, Context> Deref for MetaDirEntry<DirEntry, Metadata, Context> {
    type Target = DirEntry;

    fn deref(&self) -> &Self::Target {
        &self.entry
    }
}

impl<DirEntry: GenericDirEntry<Metadata, Context>, Metadata: GenericMetadata, Context: Default>
    MetaDirEntry<DirEntry, Metadata, Context>
{
    /// Reads the directory entry at the given path.
    pub(crate) fn from_path(
        path: impl AsRef<Path>,
        trim_symlink_path: impl FnMut(PathBuf) -> PathBuf,
    ) -> io::Result<Self> {
        firestorm::profile_fn!(both_from_path);

        let path = path.as_ref();

        let metadata = Metadata::from_path(path)?;

        let entry = DirEntry::from_path(path, trim_symlink_path)?;

        Ok(Self {
            metadata,
            entry,
            context: Context::default(),
        })
    }

    /// Walks the directory tree up to the last path components, creating non-existent directories.
    ///
    /// Returns a reference to the directory referring to the second to last component and the name
    /// of the last path component.
    fn create_dir_recursively<'path, Iter: Iterator<Item = path::Component<'path>>>(
        &mut self,
        mut path: iter::Peekable<Iter>,
    ) -> (&mut Directory<DirEntry, Metadata, Context>, &'path OsStr) {
        let mut dir = self
            .directory_mut()
            .expect("tried to insert into a non directory");

        loop {
            let name = {
                let component = path
                    .next()
                    .expect("empty path given to `Directory::insert`");

                match component {
                    std::path::Component::Normal(name) => name,
                    _ => panic!("unsupported path given to `Directory::insert`"),
                }
            };

            if path.peek().is_none() {
                return (dir, name);
            }

            if !dir.entries.contains_key(name) {
                dir.entries.insert(
                    name.to_os_string(),
                    MetaDirEntry {
                        metadata: Metadata::meaningless(),
                        entry: DirEntry::empty_dir(),
                        context: Context::default(),
                    },
                );
            }

            match dir
                .entries
                .get_mut(name)
                .and_then(|entry| entry.directory_mut())
            {
                Some(next_dir) => {
                    dir = next_dir;
                }
                None => panic!("tried to insert into a non directory"),
            }
        }
    }

    /// Inserts an entry with the given entry information into the given path.
    pub(crate) fn insert(&mut self, path: impl AsRef<Path>, entry: Self) {
        firestorm::profile_fn!(insert);

        let path = path.as_ref();

        let mut components = path.components().skip(1).peekable();

        // Handle the root dir specially
        if components.peek().is_none() {
            if self
                .directory_mut()
                .expect("tried to insert into a non directory")
                .entries
                .is_empty()
            {
                // Ignore non-directories and metadata for the root path
                if let (
                    Some(Directory { entries }),
                    Some(Directory {
                        entries: self_entries,
                    }),
                ) = (entry.entry.into_dir(), self.directory_mut())
                {
                    *self_entries = entries;
                    self.metadata = entry.metadata;
                }
            }

            return;
        }

        let (dir, name) = self.create_dir_recursively(components);

        if dir.entries.contains_key(name) {
            entry.merge_into(dir.entries.get_mut(name).unwrap());
        } else {
            dir.entries.insert(name.to_os_string(), entry);
        }
    }
}

impl<DirEntry: GenericDirEntry<Metadata, Context>, Metadata: GenericMetadata, Context>
    MetaDirEntry<DirEntry, Metadata, Context>
{
    /// Merges this entry into the given `other` entry.
    fn merge_into(self, other: &mut Self) {
        other.metadata = self.metadata;
        other.context = self.context;
        self.entry.merge_into(&mut other.entry);
    }

    /// Returns a reference to the inner directory, if the entry is a directory.
    fn directory(&self) -> Option<&Directory<DirEntry, Metadata, Context>> {
        self.entry.dir()
    }

    /// Returns a mutable reference to the inner directory, if the entry is a directory.
    fn directory_mut(&mut self) -> Option<&mut Directory<DirEntry, Metadata, Context>> {
        self.entry.dir_mut()
    }

    /// Returns the entry at the specified path.
    pub(crate) fn get(
        &self,
        path: impl AsRef<Path>,
    ) -> anyhow::Result<&MetaDirEntry<DirEntry, Metadata, Context>> {
        let path = path.as_ref();

        let mut current = self;

        let mut iter = path.components();

        loop {
            current = match iter.next() {
                Some(path::Component::Normal(name)) => match current.directory() {
                    Some(dir) => {
                        // try to get the name directly first, as this is faster
                        if let Some(entry) = dir.entries.get(name) {
                            entry
                        } else {
                            // if the entry wasn't found, try a slower case insensitive search
                            dir.entries
                                .iter()
                                .find(|(entry_name, _)| entry_name.eq_ignore_ascii_case(name))
                                .map(|(_, entry)| entry)
                                .ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "could not find entry {name:?} in {}, found entries {:?}",
                                        path.display(),
                                        dir.entries.keys().collect::<Vec<_>>()
                                    )
                                })?
                        }
                    }
                    _ => anyhow::bail!("cannot index into a non directory"),
                },
                Some(path::Component::RootDir) => continue,
                Some(path::Component::CurDir | path::Component::ParentDir) => {
                    anyhow::bail!("relative paths (`.` and `..`) are not supported");
                }
                Some(path::Component::Prefix(prefix)) => {
                    anyhow::bail!("path prefixes are not supported: {prefix:?}");
                }
                None => break,
            };
        }

        Ok(current)
    }

    /// Walks the contained directory tree.
    pub(crate) fn walk(&self) -> walker::DirectoryWalker<DirEntry, Metadata, Context> {
        walker::DirectoryWalker::Root {
            name: OsStr::new("/"),
            entry: self,
        }
    }
}

/// Represents a directory.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub(crate) struct Directory<DirEntry, Metadata, Context> {
    /// The entries in the directory.
    pub(crate) entries: BTreeMap<OsString, MetaDirEntry<DirEntry, Metadata, Context>>,
}

impl<Metadata, DirEntry, Context> Default for Directory<DirEntry, Metadata, Context> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }
}
