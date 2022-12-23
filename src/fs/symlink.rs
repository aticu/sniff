//! Reads and represents data of interest for symlinks.

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use std::{
    io,
    path::{Path, PathBuf},
};

/// Stores information about a symlink.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub(crate) struct Symlink {
    /// The path contained in the link.
    pub(crate) link_path: PathBuf,
}

/// A common trait that all implementations of symlinks should fulfill.
pub(crate) trait GenericSymlink:
    Serialize + DeserializeOwned + Clone + Sized + Send
{
    /// Reads information about the symlink at the specified path.
    fn from_path(path: impl AsRef<Path>) -> io::Result<Self>;

    /// Updates the path using the given update function.
    fn update_path(&mut self, update_path: impl FnMut(PathBuf) -> PathBuf);
}

impl GenericSymlink for Symlink {
    fn from_path(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();

        let link_path = std::fs::read_link(path)?;

        Ok(Self { link_path })
    }

    fn update_path(&mut self, mut update_path: impl FnMut(PathBuf) -> PathBuf) {
        self.link_path = update_path(std::mem::take(&mut self.link_path))
    }
}
