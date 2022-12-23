//! Create and manage snapshots of directories.

use anyhow::Context;
use serde::{Deserialize, Serialize};

use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
};

use crate::{
    autoruns::Autoruns,
    fs::{DEntry, MetaDirEntry, Metadata},
    timestamp::Timestamp,
    updates::Updates,
};

mod vdi_mount;

/// The magic string that's used to identify snapshot files.
const MAGIC_STR: &str = "snpsht";

/// The current version used for new snapshots.
///
/// ## Version history
///
/// ### Version 0
/// - Initial version
///
/// ### Version 1
/// - Added a lot more flexibility to version handling, to allow for easier updates in the future
/// - Added MD5 hashes, PE-data, valid UTF data and inodes
///
/// ### Version 2
/// - Made all metadata fields except for size optional
/// - Added fields for UNIX metadata (permissions, uid, gid, nlink)
const CURRENT_SNAPSHOT_VERSION: u8 = 2;

/// The header of a snapshot file with version information, to allow backwards compatible changes.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
struct SnapshotFileHeader {
    /// The magic string `MAGIC_STR`.
    magic: String,
    /// The version of the snapshot data structure that is used.
    version: u8,
}

impl Default for SnapshotFileHeader {
    fn default() -> Self {
        SnapshotFileHeader {
            magic: MAGIC_STR.to_string(),
            version: CURRENT_SNAPSHOT_VERSION,
        }
    }
}

impl SnapshotFileHeader {
    /// The specific bincode configuration used for serialization.
    fn bincode() -> impl bincode::Options {
        use bincode::Options as _;

        bincode::DefaultOptions::new()
            .with_varint_encoding()
            .allow_trailing_bytes()
    }

    /// Writes the header with the most recent version to a file.
    fn write_to_file(file: &mut impl io::Write) -> anyhow::Result<()> {
        Self::default().write_self_to_file(file)
    }

    /// Writes the header to a file.
    fn write_self_to_file(&self, file: &mut impl io::Write) -> anyhow::Result<()> {
        use bincode::Options as _;

        Self::bincode().serialize_into(file, &self)?;

        Ok(())
    }

    /// Reads the header from a file.
    fn read_from_file(file: &mut impl io::Read) -> anyhow::Result<Self> {
        use bincode::Options as _;

        let header: Self = Self::bincode().deserialize_from(file)?;

        if header.magic != MAGIC_STR {
            anyhow::bail!("unexpected magic header string: {}", header.magic);
        }

        Ok(header)
    }
}

/// Records information about the source of a snapshot.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub(crate) enum Source {
    /// The snapshot was created from the given directory.
    Directory(PathBuf),
    /// The snapshot was created from the given VDI image.
    VdiImage(PathBuf),
}

/// Represents a snapshot of a directory.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub(crate) struct Snapshot<DirEntry, Metadata, Autoruns, Updates> {
    /// The root directory of the snapshot.
    pub(crate) root: crate::fs::MetaDirEntry<DirEntry, Metadata, ()>,
    /// The source of the snapshot.
    pub(crate) source: Source,
    /// The creation time of the snapshot.
    pub(crate) timestamp: Timestamp,
    /// The version of the system in question.
    ///
    /// This is the output of the `ver` Windows command.
    pub(crate) version: Option<String>,
    /// Data about the autoruns on the system.
    pub(crate) autoruns: Option<Autoruns>,
    /// Data about the updates installed on the system.
    pub(crate) updates: Option<Updates>,
}

/// The latest version of the snapshot format.
pub(crate) type SnapshotLatest = Snapshot<DEntry, Metadata, Autoruns, Updates>;

impl SnapshotLatest {
    /// Creates a new snapshot of the specified location.
    pub(crate) fn create(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();

        if path.is_dir() {
            Ok(Self::create_from_dir(path))
        } else if path.is_file() {
            let mut file = io::BufReader::new(std::fs::File::open(path)?);

            use io::BufRead as _;

            let mut line = String::new();
            file.read_line(&mut line)?;

            if line == "<<< Oracle VM VirtualBox Disk Image >>>\n" {
                Self::create_from_vdi(path)
            } else {
                Err(anyhow::anyhow!("provided file is not a VDI image"))
            }
        } else {
            Err(anyhow::anyhow!(
                "snapshot cannot be created from something that isn't a directory or VDI image"
            ))
        }
    }

    /// Creates a new snapshot of the largest partition in the specified VDI image.
    pub(crate) fn create_from_vdi(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();

        let mount = vdi_mount::VDIMount::new(path)?;

        let mut snapshot = Self::create_from_dir(mount.path());

        snapshot.source = Source::VdiImage(path.to_path_buf());

        Ok(snapshot)
    }

    /// Creates a new snapshot of the specified directory.
    pub(crate) fn create_from_dir(root_path: impl AsRef<Path>) -> Self {
        let root_path = root_path.as_ref();

        let version = if let Ok(mut file) = File::open(root_path.join("sniff/version")) {
            let mut version = String::new();
            if io::Read::read_to_string(&mut file, &mut version).is_ok() {
                Some(version.trim().to_string())
            } else {
                None
            }
        } else {
            None
        };

        let autoruns = Autoruns::from_path(root_path.join("sniff/autoruns.csv")).ok();

        let updates = Updates::from_path(root_path.join("sniff/updates.csv")).ok();

        let mut paths = Vec::new();

        for entry in walkdir::WalkDir::new(root_path) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    eprintln!("could not read path information: {}", err);
                    continue;
                }
            };

            // Ignore the `sniff` root directory, since it isn't really part of the normal
            // system.
            if entry
                .path()
                .strip_prefix(root_path)
                .map(|path| path.starts_with("sniff"))
                != Ok(true)
            {
                paths.push(entry.path().to_path_buf());
            }
        }

        // Due to a bug in glibc the $MFT file may not be listed, but can still be accessed (source:
        // man 8 ntfs-3g)
        let mft_path = root_path.join("$MFT");
        if !paths.contains(&mft_path) {
            paths.push(mft_path);
        }

        use crate::fs::{dir_entry::GenericDirEntry as _, metadata::GenericMetadata as _};
        let metadata = if let Ok(metadata) = Metadata::from_path(root_path) {
            metadata
        } else {
            Metadata::meaningless()
        };
        let mut insertion_count = 0;
        let mut root = MetaDirEntry {
            metadata,
            entry: DEntry::empty_dir(),
            context: (),
        };

        crossbeam_utils::thread::scope(|s| {
            let (sender, receiver) =
                crossbeam_channel::bounded::<(PathBuf, MetaDirEntry<DEntry, Metadata, ()>)>(100);

            s.spawn(|_| {
                for (path, entry) in receiver {
                    root.insert(path, entry);
                    insertion_count += 1;
                }
            });

            use rayon::prelude::*;
            paths.par_iter().for_each(|path| {
                let entry = match MetaDirEntry::from_path(path, |symlink_path| {
                    if let Ok(path) = symlink_path.strip_prefix(root_path) {
                        Path::new("/").join(path)
                    } else {
                        symlink_path
                    }
                }) {
                    Ok(entry) => entry,
                    Err(err) => {
                        eprintln!(
                            "could not read directory entry information for {}: {}",
                            path.display(),
                            err
                        );
                        return;
                    }
                };

                let trimmed_path = Path::new("/").join(path.strip_prefix(root_path).unwrap());
                sender.send((trimmed_path, entry)).unwrap();
            });
        })
        .unwrap();

        Self {
            root,
            source: Source::Directory(root_path.to_path_buf()),
            timestamp: Timestamp::now(),
            version,
            autoruns,
            updates,
        }
    }

    /// The specific bincode configuration used for serialization.
    fn bincode() -> impl bincode::Options {
        use bincode::Options as _;

        bincode::DefaultOptions::new()
            .with_varint_encoding()
            .reject_trailing_bytes()
    }

    /// Writes the snapshot to the specified path.
    pub(crate) fn to_file(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let path = path.as_ref();

        let mut out_file = std::fs::File::create(path)
            .with_context(|| format!("failed creating file at {}", path.display()))?;

        SnapshotFileHeader::write_to_file(&mut out_file).context("failed writing file header")?;

        let out_file_compressed =
            flate2::write::GzEncoder::new(out_file, flate2::Compression::best());

        use bincode::Options as _;
        Self::bincode()
            .serialize_into(std::io::BufWriter::new(out_file_compressed), &self)
            .context("failed serialization into compressed file")?;

        Ok(())
    }

    /// Reads the snapshot from the specified path.
    pub(crate) fn from_file(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        firestorm::profile_method!(from_file);

        let mut in_file = std::fs::File::open(path)?;

        let header = SnapshotFileHeader::read_from_file(&mut in_file)?;

        match header.version {
            0 => Err(anyhow::anyhow!(
                "version 0 snapshots are no longer supported"
            )),
            1 => {
                let v1: Snapshot<
                    crate::fs::DEntryV1<()>,
                    crate::fs::metadata::MetadataV1,
                    Autoruns,
                    Updates,
                > = bincode::Options::deserialize_from(
                    Self::bincode(),
                    std::io::BufReader::new(flate2::read::GzDecoder::new(in_file)),
                )?;

                Ok(Self {
                    root: v1.root.into(),
                    source: v1.source,
                    timestamp: v1.timestamp,
                    version: v1.version,
                    autoruns: v1.autoruns,
                    updates: v1.updates,
                })
            }
            2 => {
                let v2: SnapshotLatest = bincode::Options::deserialize_from(
                    Self::bincode(),
                    std::io::BufReader::new(flate2::read::GzDecoder::new(in_file)),
                )?;

                Ok(v2)
            }
            _ => Err(anyhow::anyhow!("unknown version: {}", header.version)),
        }
    }
}
