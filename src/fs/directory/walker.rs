//! Implements an iterator over all entries in a directory.

use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

use smallvec::SmallVec;

use super::{GenericDirEntry, MetaDirEntry};
use crate::fs;

/// An iterator that walks the entire directory structure.
pub(crate) enum DirectoryWalker<'root, DirEntry, Metadata, Context> {
    /// The walker is currently at a directory and hasn't returned the entry for that directory.
    Root {
        /// The name of the directory this walker is walking.
        name: &'root OsStr,
        /// The "root" directory entry at the current walker level.
        entry: &'root MetaDirEntry<DirEntry, Metadata, Context>,
    },
    /// The walker is currently iterating over the children of a directory.
    Children {
        /// The name of the directory this walker is walking.
        root_name: &'root OsStr,
        /// The current subdirectory walker.
        subwalker: Option<Box<DirectoryWalker<'root, DirEntry, Metadata, Context>>>,
        /// The iterator over the entries in the walker.
        entries: std::collections::btree_map::Iter<
            'root,
            OsString,
            MetaDirEntry<DirEntry, Metadata, Context>,
        >,
    },
    /// The walker is finished walking.
    Finished,
}

impl<'root, DirEntry: GenericDirEntry<Metadata, Context>, Metadata, Context> Iterator
    for DirectoryWalker<'root, DirEntry, Metadata, Context>
{
    type Item = WalkerEntry<'root, DirEntry, Metadata, Context>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            DirectoryWalker::Root { .. } => {
                // Extract the name and entry temporarily from `self`, note that `self` is set
                // again later and that this function must not return in between
                let (name, entry) = match std::mem::replace(self, DirectoryWalker::Finished) {
                    DirectoryWalker::Root { name, entry } => (name, entry),
                    _ => unreachable!(),
                };

                *self = match entry.dir() {
                    Some(dir) => DirectoryWalker::Children {
                        root_name: name,
                        subwalker: None,
                        entries: dir.entries.iter(),
                    },
                    _ => DirectoryWalker::Finished,
                };

                Some(WalkerEntry {
                    path: PathChain {
                        components: smallvec::smallvec![name],
                    },
                    entry,
                })
            }
            DirectoryWalker::Children {
                root_name,
                subwalker,
                entries,
            } => loop {
                if let Some(subwalker) = subwalker {
                    if let Some(WalkerEntry { mut path, entry }) = subwalker.next() {
                        path.components.push(root_name);
                        return Some(WalkerEntry { path, entry });
                    }
                }

                if let Some((name, entry)) = entries.next() {
                    *subwalker = Some(Box::new(DirectoryWalker::Root { name, entry }));
                } else {
                    *self = DirectoryWalker::Finished;

                    return None;
                }
            },
            DirectoryWalker::Finished => None,
        }
    }
}

/// Represents a whole path chain within the directory tree.
struct PathChain<'root> {
    /// The components of the path.
    ///
    /// Note that the components are stored in the opposite order.
    /// That means that the root component is the last element in the vector.
    components: SmallVec<[&'root OsStr; 8]>,
}

/// A single entry resulting from a `Walker` iterator.
pub(crate) struct WalkerEntry<'root, DirEntry, Metadata, Context> {
    /// The path to this entry in the tree.
    path: PathChain<'root>,
    /// The entry that is returned.
    pub(crate) entry: &'root MetaDirEntry<DirEntry, Metadata, Context>,
}

impl<'root, DirEntry, Metadata, Context> WalkerEntry<'root, DirEntry, Metadata, Context> {
    /// The file name of the entry.
    pub(crate) fn file_name(&self) -> Option<&OsStr> {
        if let Some(file_name) = self.path.components.first() && *file_name != OsStr::new("/") {
            Some(file_name)
        } else {
            None
        }
    }

    /// Returns a clone of the full path to this entry.
    pub(crate) fn clone_path(&self) -> PathBuf {
        self.path.components.iter().rev().collect()
    }

    /// Returns an iterator over the components in the path.
    pub(crate) fn path_components(&self) -> impl Iterator<Item = &OsStr> {
        self.path.components.iter().rev().cloned()
    }
}

impl<'root>
    WalkerEntry<
        'root,
        fs::DirEntry<
            fs::Metadata,
            fs::File,
            fs::Symlink,
            fs::dir_entry_type::DirEntryType,
            crate::diff::DiffType,
        >,
        fs::Metadata,
        crate::diff::DiffType,
    >
{
    /// Creates a filter context from this file system entry.
    pub(crate) fn filter(
        &self,
        database: Option<&crate::database::Database>,
        filter: impl Fn(crate::diff::filters::FilterContext) -> bool,
    ) -> Option<bool> {
        self.file_name().map(|name| {
            let ctx = crate::diff::filters::FilterContext {
                name,
                entry: self.entry,
                database,
            };

            filter(ctx)
        })
    }
}
