//! Parses the text format of autoruns files and represents their content.

use std::{collections::BTreeMap, fmt, fs::File, io, path::Path};

use anyhow::Context as _;
use owo_colors::OwoColorize as _;
use serde::{Deserialize, Serialize};

use crate::{
    database::Database,
    fs::{self, dir_entry::GenericDirEntry},
    snapshot::SnapshotLatest,
    timestamp::Timestamp,
};

/// Reads a UTF-16-LE encoded file to a string.
fn read_utf16le_file(path: impl AsRef<Path>) -> io::Result<String> {
    let path = path.as_ref();

    let mut data = Vec::new();
    io::Read::read_to_end(&mut File::open(path)?, &mut data)?;

    char::decode_utf16(
        data.chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]])),
    )
    .map(|maybe_char| maybe_char.map_err(|err| io::Error::new(io::ErrorKind::Other, err)))
    .collect()
}

/// An entry in the autoruns file.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub(crate) struct AutorunsEntry {
    /// The name of the entry.
    pub(crate) entry: String,
    /// The description of the entry.
    pub(crate) description: String,
    /// The name of the signer, signing the entry.
    pub(crate) signer: String,
    /// The verification status of the signer.
    pub(crate) signer_verification: SignerVerification,
    /// The path to the file of the entry.
    pub(crate) image_path: Option<String>,
    /// The timestamp of the entry.
    pub(crate) timestamp: Option<Timestamp>,
    /// The autostart category of the entry.
    pub(crate) category: String,
    /// The location where the entry is marked as an autostart entry.
    pub(crate) location: String,
    /// The profile for which the entry was found.
    pub(crate) profile: String,
    /// The company associated with the entry.
    pub(crate) company: String,
    /// The version associated with the entry.
    pub(crate) version: String,
    /// The launch string of then entry.
    pub(crate) launch_string: String,
}

impl TryFrom<BTreeMap<String, String>> for AutorunsEntry {
    type Error = &'static str;

    fn try_from(mut map: BTreeMap<String, String>) -> Result<Self, Self::Error> {
        let entry = map.remove("Entry").ok_or("column `Entry` not found")?;
        let description = map
            .remove("Description")
            .ok_or("column `Description` not found")?;
        let signer = map.remove("Signer").ok_or("column `Signer` not found")?;
        let (signer_verification, signer) = if let Some(partial_signer) = signer.strip_prefix('(') {
            if let Some((verification, name)) = partial_signer.split_once(") ") {
                let verification = match verification {
                    "Verified" => SignerVerification::Verified,
                    "Not verified" => SignerVerification::NotVerified,
                    "" => SignerVerification::Unknown,
                    other => SignerVerification::Other(other.to_string()),
                };

                (verification, name.to_string())
            } else {
                (SignerVerification::Unknown, signer)
            }
        } else {
            (SignerVerification::Unknown, signer)
        };

        let image_path = map
            .remove("Image Path")
            .ok_or("column `Image Path` not found")?;
        let image_path = if image_path.starts_with("File not found: ") || image_path.is_empty() {
            None
        } else {
            Some(image_path)
        };
        let timestamp = map.remove("Time").and_then(|ts| {
            time::PrimitiveDateTime::parse(
                &ts,
                time::macros::format_description!("[day]/[month]/[year] [hour]:[minute]"),
            )
            .map(|ts| ts.assume_utc().into())
            .ok()
        });

        let category = map
            .remove("Category")
            .ok_or("column `Category` not found")?;
        let location = map
            .remove("Entry Location")
            .ok_or("column `Entry Location` not found")?;
        let profile = map.remove("Profile").ok_or("column `Profile` not found")?;
        let company = map.remove("Company").ok_or("column `Company` not found")?;
        let version = map.remove("Version").ok_or("column `Version` not found")?;
        let launch_string = map
            .remove("Launch String")
            .ok_or("column `Launch String` not found")?;

        Ok(Self {
            entry,
            description,
            signer,
            signer_verification,
            image_path,
            timestamp,
            category,
            location,
            profile,
            company,
            version,
            launch_string,
        })
    }
}

impl fmt::Display for AutorunsEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(path) = &self.image_path {
            write!(
                f,
                "{} at {path} by {} ({:?})",
                self.entry, self.signer, self.signer_verification
            )
        } else {
            write!(
                f,
                "{} by {} ({:?})",
                self.entry, self.signer, self.signer_verification
            )
        }
    }
}

/// The verification status of a signer for a file.
#[derive(PartialEq, Eq, Clone, Serialize, Deserialize)]
pub(crate) enum SignerVerification {
    /// The signer was verified.
    Verified,
    /// The signer was not verified.
    NotVerified,
    /// The verification status is unknown.
    Unknown,
    /// There was an unexpected verification status.
    Other(String),
}

impl fmt::Debug for SignerVerification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignerVerification::Verified => write!(f, "{}", "verified".green()),
            SignerVerification::NotVerified => write!(f, "{}", "not verified".red()),
            SignerVerification::Unknown => write!(f, "{}", "verification unknown".yellow()),
            SignerVerification::Other(text) => write!(f, "verification: {}", text.blue()),
        }
    }
}

/// Represents the data of an autoruns.txt file.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub(crate) struct Autoruns {
    /// The entries in the autoruns file.
    pub(crate) entries: Vec<AutorunsEntry>,
    /// The time the autoruns data was recorded.
    ///
    /// This corresponds to the last modification time of the file.
    pub(crate) recording_time: Timestamp,
}

impl Autoruns {
    /// Reads the autoruns information from the specified path.
    pub(crate) fn from_path(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();

        let content = read_utf16le_file(path)?;

        let mut entries: Vec<AutorunsEntry> = Vec::new();

        for raw_autorun in csv::Reader::from_reader(content.as_bytes()).deserialize() {
            let raw_autorun: BTreeMap<String, String> = raw_autorun?;
            if let Ok(autorun) = raw_autorun.try_into() {
                entries.push(autorun);
            }
        }

        let metadata = path.metadata()?;

        Ok(Self {
            entries,
            recording_time: metadata.modified()?.into(),
        })
    }
}

impl AutorunsEntry {
    /// Evaluates how unusual the autoruns entry is.
    pub(crate) fn evaluate(
        &self,
        db: &Database,
        snapshot: &SnapshotLatest,
        snapshot_without_autoruns: Option<&SnapshotLatest>,
    ) -> anyhow::Result<AutorunsEvaluation> {
        let Some(path) = &self.image_path else {
            return Ok(AutorunsEvaluation {
                entry: self,
                results: vec![AutorunsEvaluationResult::MissingImagePath],
            });
        };
        let path = fs::convert_windows_path(path);

        let mut results = Vec::new();

        fn get_file<'s>(
            snapshot: &'s SnapshotLatest,
            is_main: bool,
            path: &Path,
            results: &mut Vec<AutorunsEvaluationResult>,
        ) -> Option<&'s fs::File> {
            match snapshot.root.get(path) {
                Ok(fs::MetaDEntry {
                    entry: fs::DirEntry::File(file),
                    ..
                }) => Some(file),
                Ok(entry) => {
                    results.push(AutorunsEvaluationResult::EntryNotAFile {
                        is_main,
                        ty: entry.entry.entry_type(),
                    });
                    None
                }
                Err(_) => {
                    results.push(AutorunsEvaluationResult::MissingFile { is_main });
                    None
                }
            }
        }

        let file = get_file(snapshot, true, &path, &mut results);
        let file2 = snapshot_without_autoruns
            .map(|snapshot| get_file(snapshot, false, &path, &mut results));

        if let Some(file2) = file2 {
            if file != file2 {
                results.push(AutorunsEvaluationResult::FileChanged);
            }
        }

        if let Some(file) = file && !db
            .file_is_known(file)
            .context("failed checking if file2 exists")?
        {
            results.push(AutorunsEvaluationResult::HashUnknown { md5: file.md5_hash.clone() });
        }

        if !db.is_known_autorun_path(&path).with_context(|| {
            format!(
                "failed to query whether autoruns path is known for path {}",
                path.display()
            )
        })? {
            results.push(AutorunsEvaluationResult::UnknownPath);
        }

        Ok(AutorunsEvaluation {
            entry: self,
            results,
        })
    }
}

/// An evaluation of an autoruns entry.
pub(crate) struct AutorunsEvaluation<'entry> {
    /// The entry that was evaluated.
    entry: &'entry AutorunsEntry,
    /// The result of the evaluation.
    results: Vec<AutorunsEvaluationResult>,
}

impl AutorunsEvaluation<'_> {
    /// Whether the evaluation result is only that the file hash is unknown.
    pub(crate) fn unknown_hash_only(&self) -> bool {
        matches!(
            &self.results[..],
            [AutorunsEvaluationResult::HashUnknown { .. }]
        )
    }

    /// Whether the evaluation result is interesting enough to print.
    pub(crate) fn should_be_printed(&self) -> bool {
        if self.entry.signer_verification != SignerVerification::Verified {
            return true;
        }

        // If the file is known but simply the path changed, then this entry is very likely not
        // malicious.
        if matches!(&self.results[..], [AutorunsEvaluationResult::UnknownPath]) {
            return false;
        }

        !self.results.is_empty()
    }
}

/// The result of an evaluation of an autoruns entry.
#[derive(Debug)]
enum AutorunsEvaluationResult {
    /// The entry does not have an image path and could thus not be further evaluated.
    MissingImagePath,
    /// The entry has an image path, but there is no entry in the file system at the given
    /// location.
    MissingFile {
        /// Whether the snapshot is the main one where the autoruns where recorded.
        is_main: bool,
    },
    /// The entry at the given image path was not a file.
    EntryNotAFile {
        /// Whether the snapshot is the main one where the autoruns where recorded.
        is_main: bool,
        /// The type that was encountered instead.
        ty: fs::dir_entry_type::DirEntryType,
    },
    /// The file was changed between the snapshot with autoruns and the one without.
    FileChanged,
    /// The file hash of the autorun entry was not known in the database.
    HashUnknown {
        /// The MD5 hash of the file that was unknown.
        md5: fs::file::Md5Hash,
    },
    /// The path was never seen in any autoruns entry previously.
    UnknownPath,
}

impl fmt::Display for AutorunsEvaluation<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let warn = |f: &mut fmt::Formatter<'_>, text: &str| -> fmt::Result {
            writeln!(f, "  {}: {text}", "WARNING".on_yellow())
        };

        let suspicious = |f: &mut fmt::Formatter<'_>, text: &str| -> fmt::Result {
            writeln!(f, "  {}: {text}", "SUSPICIOUS".on_red())
        };

        writeln!(f, "{:?}", self.entry.entry)?;
        writeln!(f, "  Description: {}", self.entry.description)?;
        writeln!(
            f,
            "  Signer: {} ({:?})",
            self.entry.signer, self.entry.signer_verification
        )?;
        if let Some(image_path) = &self.entry.image_path {
            writeln!(f, "  Path: {}", image_path)?;
        }
        writeln!(f, "  Launch string: {:?}", self.entry.launch_string)?;

        for result in &self.results {
            match result {
                AutorunsEvaluationResult::MissingImagePath => {
                    warn(f, "the entry was missing an image path")?;
                }
                AutorunsEvaluationResult::MissingFile { is_main } => {
                    warn(
                        f,
                        &format!(
                            "there was no file at the path in the {}snapshot",
                            if *is_main { "" } else { "original " }
                        ),
                    )?;
                }
                AutorunsEvaluationResult::EntryNotAFile { is_main, ty } => {
                    warn(
                        f,
                        &format!(
                            "the entry in the {}snapshot was not a file, but a {}",
                            if *is_main { "" } else { "original " },
                            ty
                        ),
                    )?;
                }
                AutorunsEvaluationResult::FileChanged => {
                    suspicious(f, "the file was changed between the snapshots")?;
                }
                AutorunsEvaluationResult::HashUnknown { md5 } => {
                    warn(f, &format!("unknown file hash: {:?}", md5))?;
                }
                AutorunsEvaluationResult::UnknownPath => {
                    warn(f, "unknown autoruns path")?;
                }
            }
        }

        Ok(())
    }
}
