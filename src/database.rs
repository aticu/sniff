//! Implements a database for storing information about files independent of the snapshots.

use anyhow::Context as _;
use owo_colors::OwoColorize as _;
use rusqlite as sql;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fmt,
    os::unix::prelude::{OsStrExt, OsStringExt as _},
    path::{Path, PathBuf},
};

use crate::{
    fs::{self, OsStrExt as _},
    snapshot::SnapshotLatest,
    timestamp::Timestamp,
};

/// The prepared statements necessary for snapshot insertion.
struct SnapshotInsertionStatements<'a> {
    /// The statement to insert an entry into the file table.
    insert_file: sql::Statement<'a>,
    /// The statement to get the `id` of an entry in the file table.
    get_file_id: sql::Statement<'a>,
    /// The statement to insert an entry into the path table.
    insert_path: sql::Statement<'a>,
    /// The statement to get the `id` of an entry in the path table.
    get_path_id: sql::Statement<'a>,
    /// The statement to insert an entry into the normalized path table.
    insert_normalized_path: sql::Statement<'a>,
    /// The statement to get the `id` of an entry in the normalized path table.
    get_normalized_path_id: sql::Statement<'a>,
    /// The statement to insert an entry into the records table.
    insert_entry: sql::Statement<'a>,
    /// The statement to insert an entry into the snapshot table.
    insert_snapshot: sql::Statement<'a>,
    /// The statement to get the `id` of an entry in the snapshot table.
    get_snapshot_id: sql::Statement<'a>,
    /// The statement to insert an autorun into the autoruns table.
    insert_autorun: sql::Statement<'a>,
}

/// The type of an `id` in SQL statements.
type SqlId = i64;

/// Access to a database containing information about multiple snapshots.
#[derive(Debug)]
pub(crate) struct Database {
    /// The connection to the underlying database.
    connection: sql::Connection,
    /// The id of the main snapshot.
    main_snapshot_id: Option<SqlId>,
    /// The id of the snapshot to compare to.
    comparison_snapshot_id: Option<SqlId>,
}

/// Returns the prepared statements necessary for snapshot insertion.
fn snapshot_insertion_stmts(
    connection: &sql::Connection,
) -> anyhow::Result<SnapshotInsertionStatements> {
    Ok(SnapshotInsertionStatements {
        insert_file: connection
            .prepare(
                "INSERT INTO Files (
                    sha256,
                    md5,
                    size,
                    first_bytes,
                    entropy,
                    coff_header,
                    valid_utf8,
                    valid_utf16be,
                    valid_utf16le,
                    valid_utf32be,
                    valid_utf32le
                ) VALUES (
                    :sha256,
                    :md5,
                    :size,
                    :first_bytes,
                    :entropy,
                    :coff_header,
                    :valid_utf8,
                    :valid_utf16be,
                    :valid_utf16le,
                    :valid_utf32be,
                    :valid_utf32le
                )",
            )
            .context("Failed to prepare file insertion statement")?,
        get_file_id: connection
            .prepare(
                "SELECT
                    id
                FROM
                    Files
                WHERE
                    sha256 = :sha256 AND
                    md5 = :md5 AND
                    size = :size AND
                    (1 OR -- this just exists to use the parameters
                        (
                            first_bytes = :first_bytes AND
                            entropy = :entropy AND
                            coff_header = :coff_header AND
                            valid_utf8 = :valid_utf8 AND
                            valid_utf16be = :valid_utf16be AND
                            valid_utf16le = :valid_utf16le AND
                            valid_utf32be = :valid_utf32be AND
                            valid_utf32le = :valid_utf32le
                        )
                    )",
            )
            .context("Failed to prepare file id statement")?,
        insert_path: connection
            .prepare(
                "INSERT INTO Paths (
                    path
                ) VALUES (
                    :path
                )",
            )
            .context("Failed to prepare path insertion statement")?,
        get_path_id: connection
            .prepare(
                "SELECT
                    id
                FROM
                    Paths
                WHERE
                    path = :path",
            )
            .context("Failed to prepare path id statement")?,
        insert_normalized_path: connection
            .prepare(
                "INSERT INTO NormalizedPaths (
                    path
                ) VALUES (
                    :path
                )",
            )
            .context("Failed to prepare normalized path insertion statement")?,
        get_normalized_path_id: connection
            .prepare(
                "SELECT
                    id
                FROM
                    NormalizedPaths
                WHERE
                    path = :path",
            )
            .context("Failed to prepare normalized path id statement")?,
        insert_entry: connection
            .prepare(
                "INSERT INTO Records (
                    snapshot_id,
                    path_id,
                    normalized_path_id,
                    file_id
                ) VALUES (
                    :snapshot_id,
                    :path_id,
                    :normalized_path_id,
                    :file_id
                )",
            )
            .context("Failed to prepare entry insertion statement")?,
        insert_snapshot: connection
            .prepare(
                "INSERT INTO Snapshots (
                    date,
                    version,
                    comment
                ) VALUES (
                    :date,
                    :version,
                    :comment
                )",
            )
            .context("Failed to prepare snapshot insertion statement")?,
        get_snapshot_id: connection
            .prepare(
                "SELECT
                    id
                FROM
                    Snapshots
                WHERE
                    date = :date AND
                    version IS :version AND
                    (1 OR -- this just exists to use the parameters
                        (
                            comment = :comment
                        )
                    )",
            )
            .context("Failed to prepare snapshot id statement")?,
        insert_autorun: connection
            .prepare(
                "INSERT INTO Autoruns (
                    snapshot_id,
                    path_id,
                    normalized_path_id,
                    file_id,
                    entry_name
                ) VALUES (
                    :snapshot_id,
                    :path_id,
                    :normalized_path_id,
                    :file_id,
                    :entry_name
                )",
            )
            .context("Failed to prepare autorun insertion statement")?,
    })
}

/// Inserts a row and returns the `id` of the new row.
///
/// If the entry was already in the table, then the SQL statement `get_id` is executed to get
/// the `id` that way.
fn insert_and_get_id(
    connection: &sql::Connection,
    insert_stmt: &mut sql::Statement,
    get_id_stmt: &mut sql::Statement,
    params: impl sql::Params + Clone,
) -> anyhow::Result<SqlId> {
    match insert_stmt
        .execute(params.clone())
        .context("Failed the insertion")?
    {
        1 => Ok(connection.last_insert_rowid()),
        _ => get_id_stmt
            .query_row(params, |row| row.get("id"))
            .context("Failed the id retrieval"),
    }
}

/// Inserts the given file into the database, returning the `id` of the file.
fn insert_file(
    connection: &sql::Connection,
    stmts: &mut SnapshotInsertionStatements,
    file: &fs::File,
    size: u64,
) -> anyhow::Result<SqlId> {
    use fs::file::FileFlags;
    insert_and_get_id(
        connection,
        &mut stmts.insert_file,
        &mut stmts.get_file_id,
        sql::named_params! {
            ":sha256": file.sha2_256_hash.bytes,
            ":md5": file.md5_hash.bytes,
            ":size": size,
            ":first_bytes": &file.first_bytes[..],
            ":entropy": file.entropy,
            ":coff_header": file.coff_header,
            ":valid_utf8": file.flags.contains(FileFlags::UTF8),
            ":valid_utf16be": file.flags.contains(FileFlags::UTF16BE),
            ":valid_utf16le": file.flags.contains(FileFlags::UTF16LE),
            ":valid_utf32be": file.flags.contains(FileFlags::UTF32BE),
            ":valid_utf32le": file.flags.contains(FileFlags::UTF32LE),
        },
    )
}

/// Inserts the given path into the database, returning the `id` of the path entries.
///
/// There are two different SQL tables for paths:
///     1. One to store the paths in their original form as a binary blob. This is necessary,
///        because paths may not be valid strings.
///     2. One to store normalized paths. These are strings and normalize capitalization, but
///        they may not exist, because the path may not be a valid string.
///
/// The result of this function is a tuple: `(id, normalized_id)`.
fn insert_path(
    connection: &sql::Connection,
    stmts: &mut SnapshotInsertionStatements,
    path: impl AsRef<Path>,
) -> anyhow::Result<(SqlId, Option<SqlId>)> {
    use std::os::unix::ffi::OsStrExt as _;

    let path = path.as_ref();
    let bytes = path.as_os_str().as_bytes();

    let id = insert_and_get_id(
        connection,
        &mut stmts.insert_path,
        &mut stmts.get_path_id,
        sql::named_params! {
            ":path": bytes,
        },
    )
    .context("Failed to insert the original path")?;

    let normalized_id = if let Some(normalized_path) = path.as_os_str().normalize() {
        Some(
            insert_and_get_id(
                connection,
                &mut stmts.insert_normalized_path,
                &mut stmts.get_normalized_path_id,
                sql::named_params! {
                    ":path": normalized_path,
                },
            )
            .context("Failed to insert the normalized path")?,
        )
    } else {
        None
    };

    Ok((id, normalized_id))
}

/// Inserts the given entry into the database.
fn insert_entry(
    connection: &sql::Connection,
    stmts: &mut SnapshotInsertionStatements,
    snapshot_id: SqlId,
    path: impl AsRef<Path>,
    entry: &fs::MetaDEntry,
) -> anyhow::Result<()> {
    if let fs::DirEntry::File(file) = &entry.entry {
        let file_id = insert_file(connection, stmts, file, entry.metadata.size)
            .context("Failed to insert the file into the files table")?;
        let (path_id, normalized_path_id) = insert_path(connection, stmts, path)
            .context("Failed to insert the path into the paths table")?;

        stmts
            .insert_entry
            .execute(sql::named_params! {
                ":snapshot_id": snapshot_id,
                ":path_id": path_id,
                ":normalized_path_id": normalized_path_id,
                ":file_id": file_id
            })
            .context("Failed to insert the entry into the records table")?;
    }

    Ok(())
}

/// Returns the ID of the given file, if it exists in the database.
fn get_file_id(connection: &sql::Connection, file: &fs::File) -> anyhow::Result<Option<SqlId>> {
    let mut stmt = connection
        .prepare_cached(
            "SELECT
                    id
                FROM
                    Files
                WHERE
                    (
                        md5 = :md5 AND
                        sha256 = :sha256
                    ) OR ( -- to allow for old snapshots, which don't have md5 values
                        md5 = x'00000000000000000000000000000000' AND
                        sha256 = :sha256
                    )",
        )
        .context("Failed to prepare statement for getting file ID")?;

    match stmt.query_row(
        sql::named_params! { ":md5": file.md5_hash.bytes, ":sha256": file.sha2_256_hash.bytes },
        |row: &sql::Row| row.get("id"),
    ) {
        Ok(result) => Ok(Some(result)),
        Err(sql::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(err),
    }
    .context("Failed to query file row from database")
}

/// Refers to a snapshot by some metadata about it.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SnapshotRef {
    /// The time stamp of the snapshot.
    pub(crate) timestamp: Timestamp,
    /// The version in the snapshot.
    pub(crate) version: Option<String>,
    /// The comment of the snapshot.
    pub(crate) comment: String,
}

impl fmt::Display for SnapshotRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {:?}", self.comment.blue(), self.timestamp)?;

        if let Some(version) = &self.version {
            write!(f, " ({version})")?;
        }

        Ok(())
    }
}

/// Data about the presence of a single file within all snapshots of the database, except the
/// current ones.
#[derive(Debug)]
pub(crate) struct FileOccurrences {
    /// All the paths where the file was seen within each snapshot.
    pub(crate) paths: BTreeMap<SnapshotRef, Vec<PathBuf>>,
}

impl Database {
    /// Opens the database at the specified `path`.
    pub(crate) fn open(path: impl AsRef<Path>) -> sql::Result<Self> {
        let this = Self {
            connection: sql::Connection::open(path)?,
            main_snapshot_id: None,
            comparison_snapshot_id: None,
        };

        this.setup_database()?;

        Ok(this)
    }

    /// Sets up the schema of the database.
    fn setup_database(&self) -> sql::Result<()> {
        // these PRAGMA values are designed to make the insertions as fast as possible
        self.connection.execute_batch(
            "PRAGMA foreign_keys=1;
            PRAGMA journal_mode = OFF;
            PRAGMA synchronous = 0;
            PRAGMA cache_size = 1000000;
            PRAGMA locking_mode = EXCLUSIVE;
            PRAGMA temp_store = MEMORY;
            -- these pragmas significantly increase the insertion speed

            CREATE TABLE IF NOT EXISTS Snapshots (
                id INTEGER PRIMARY KEY,
                date TEXT NOT NULL,
                version TEXT,
                comment TEXT NOT NULL,
                UNIQUE (date, version) ON CONFLICT IGNORE
            ) STRICT;

            CREATE TABLE IF NOT EXISTS Paths (
                id INTEGER PRIMARY KEY,
                path BLOB NOT NULL,
                UNIQUE (path) ON CONFLICT IGNORE
            ) STRICT;

            CREATE TABLE IF NOT EXISTS NormalizedPaths (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL,
                UNIQUE (path) ON CONFLICT IGNORE
            ) STRICT;

            CREATE TABLE IF NOT EXISTS Files (
                id INTEGER PRIMARY KEY,
                sha256 BLOB NOT NULL,
                md5 BLOB NOT NULL,
                size INTEGER NOT NULL,
                first_bytes BLOB NOT NULL,
                entropy REAL NOT NULL,
                coff_header BLOB,
                valid_utf8 INT NOT NULL,
                valid_utf16be INT NOT NULL,
                valid_utf16le INT NOT NULL,
                valid_utf32be INT NOT NULL,
                valid_utf32le INT NOT NULL,
                UNIQUE (sha256, md5, size, first_bytes) ON CONFLICT IGNORE
            ) STRICT;
            CREATE INDEX IF NOT EXISTS Sha256Idx on Files (sha256);
            CREATE INDEX IF NOT EXISTS Md5Idx on Files (md5);

            CREATE TABLE IF NOT EXISTS Records (
                id INTEGER PRIMARY KEY,
                snapshot_id INTEGER NOT NULL REFERENCES Snapshots(id),
                path_id INTEGER NOT NULL REFERENCES Paths(id),
                normalized_path_id INTEGER REFERENCES NormalizedPaths(id),
                file_id INTEGER NOT NULL REFERENCES Files(id),
                UNIQUE (snapshot_id, path_id, file_id) ON CONFLICT IGNORE
            ) STRICT;
            CREATE INDEX IF NOT EXISTS PathIdIdx on Records (normalized_path_id);
            CREATE INDEX IF NOT EXISTS FileIdIdx on Records (file_id);

            CREATE TABLE IF NOT EXISTS Autoruns (
                id INTEGER PRIMARY KEY,
                snapshot_id INTEGER NOT NULL REFERENCES Snapshots(id),
                path_id INTEGER NOT NULL REFERENCES Paths(id),
                normalized_path_id INTEGER REFERENCES NormalizedPaths(id),
                file_id INTEGER REFERENCES Files(id),
                entry_name TEXT NOT NULL,
                UNIQUE (snapshot_id, path_id) ON CONFLICT IGNORE
            ) STRICT;
            CREATE INDEX IF NOT EXISTS AutorunsPathIdIdx on Autoruns (normalized_path_id);",
        )?;

        Ok(())
    }

    /// Returns the ID of a snapshot already in the database.
    fn get_snapshot_id(&self, snapshot: &SnapshotLatest) -> anyhow::Result<Option<SqlId>> {
        firestorm::profile_method!(get_snapshot_id);

        let num_files = snapshot
            .root
            .walk()
            .filter(|entry| entry.entry.is_file())
            .count();
        let date = time::OffsetDateTime::from(snapshot.timestamp);

        let snapshot_id: SqlId = match self.connection.query_row(
            "SELECT
                id
            FROM
                Snapshots
            WHERE
                date = :date AND
                version IS :version",
            sql::named_params! { ":date": date, ":version": snapshot.version },
            |row| row.get("id"),
        ) {
            Ok(id) => id,
            Err(sql::Error::QueryReturnedNoRows) => return Ok(None),
            Err(err) => Err(err).context("Could not get snapshot id for loaded snapshot")?,
        };

        let result: usize = self.connection.query_row(
            "SELECT
                COUNT(1) AS result
            FROM
                Records
            WHERE
                snapshot_id = :snapshot_id",
            sql::named_params! {
                ":snapshot_id": snapshot_id,
            },
            |row| row.get("result"),
        )?;

        if result == num_files {
            self.connection
                .query_row(
                    "SELECT
                        id
                    FROM
                        Snapshots
                    WHERE
                         date = :date AND
                         version IS :version",
                    sql::named_params! { ":date": date, ":version": snapshot.version },
                    |row| row.get("id"),
                )
                .context("Failed to get id for already present snapshot")
        } else {
            Ok(None)
        }
    }

    /// Inserts a snapshot and all its files into the database.
    pub(crate) fn insert_snapshot(
        &mut self,
        snapshot: &SnapshotLatest,
        comment: &str,
    ) -> anyhow::Result<SqlId> {
        firestorm::profile_method!(insert_snapshot);

        let date = time::OffsetDateTime::from(snapshot.timestamp);

        if let Some(id) = self
            .get_snapshot_id(snapshot)
            .context("Failed to check if snapshot is already present in database")?
        {
            return Ok(id);
        }

        let transaction = self
            .connection
            .transaction()
            .context("Failed to initiate snapshot insertion transaction")?;

        let mut stmts = snapshot_insertion_stmts(&transaction)
            .context("Failed to prepare insertions statements")?;

        let snapshot_id = insert_and_get_id(
            &transaction,
            &mut stmts.insert_snapshot,
            &mut stmts.get_snapshot_id,
            sql::named_params! {
                ":date": date,
                ":version": snapshot.version,
                ":comment": comment
            },
        )
        .context("Failed to insert the snapshot into the snapshots table")?;

        for entry in snapshot.root.walk() {
            insert_entry(
                &transaction,
                &mut stmts,
                snapshot_id,
                &entry.clone_path(),
                entry.entry,
            )
            .with_context(|| {
                format!(
                    "Failed to insert entry at `{}`",
                    entry.clone_path().display()
                )
            })?;
        }

        if let Some(autoruns) = &snapshot.autoruns {
            for entry in &autoruns.entries {
                if let Some(path) = &entry.image_path {
                    let path = fs::convert_windows_path(path);
                    let (path_id, normalized_path_id) =
                        insert_path(&transaction, &mut stmts, &path)?;

                    let file = snapshot.root.get(&path);
                    let file_id = file
                        .ok()
                        .and_then(|file| {
                            if let fs::DirEntry::File(file) = &file.entry {
                                get_file_id(&transaction, file).transpose()
                            } else {
                                None
                            }
                        })
                        .transpose()
                        .context("Failed to get ID for file")?;

                    stmts
                        .insert_autorun
                        .execute(sql::named_params! {
                            ":snapshot_id": snapshot_id,
                            ":path_id": path_id,
                            ":normalized_path_id": normalized_path_id,
                            ":file_id": file_id,
                            ":entry_name": entry.entry
                        })
                        .with_context(|| {
                            format!("Failed to insert the autoruns entry for {}", entry.entry)
                        })?;
                }
            }
        }

        drop(stmts);
        transaction.commit()?;

        Ok(snapshot_id)
    }

    /// Marks the given snapshot as the main snapshot in the analysis.
    pub(crate) fn main_snapshot(&mut self, snapshot: &SnapshotLatest) -> anyhow::Result<()> {
        self.main_snapshot_id = self.get_snapshot_id(snapshot)?;

        Ok(())
    }

    /// Marks the given snapshot as the comparison snapshot in the analysis.
    pub(crate) fn comparison_snapshot(&mut self, snapshot: &SnapshotLatest) -> anyhow::Result<()> {
        self.comparison_snapshot_id = self.get_snapshot_id(snapshot)?;

        Ok(())
    }

    /// Checks if the file is known as a file to the database.
    pub(crate) fn file_is_known(&self, file: &fs::File) -> anyhow::Result<bool> {
        let Some(file_id) =
            get_file_id(&self.connection, file)
            .with_context(|| format!("Failed getting the file ID for {:?}", file.sha2_256_hash))? else {
                return Ok(false);
            };

        let mut stmt = self
            .connection
            .prepare_cached(
                "SELECT
                    1
                FROM
                    Records,
                    Snapshots
                WHERE
                    file_id = :file_id AND
                    snapshot_id = Snapshots.id AND
                    snapshot_id IS NOT :main_id AND
                    snapshot_id IS NOT :comparison_id",
            )
            .context("Failed to prepare statement for file existence checking")?;

        stmt.exists(sql::named_params! {
            ":file_id": file_id,
            ":main_id": self.main_snapshot_id,
            ":comparison_id": self.comparison_snapshot_id,
        })
        .context("Failed to check for existence of file")
    }

    /// Checks if the file is known as a file to the database.
    pub(crate) fn file_occurrences(&self, file: &fs::File) -> anyhow::Result<FileOccurrences> {
        let Some(file_id) =
            get_file_id(&self.connection, file)
            .with_context(|| format!("Failed getting the file ID for {:?}", file.sha2_256_hash))? else {
                return Ok(FileOccurrences { paths: Default::default() });
            };

        let mut stmt = self
            .connection
            .prepare_cached(
                "SELECT
                    date,
                    version,
                    comment,
                    path
                FROM
                    Records,
                    Snapshots,
                    Paths
                WHERE
                    file_id = :file_id AND
                    snapshot_id = Snapshots.id AND
                    path_id = Paths.id AND
                    snapshot_id IS NOT :main_id AND
                    snapshot_id IS NOT :comparison_id",
            )
            .context("Failed to prepare statement for file existence checking")?;

        let mut paths: BTreeMap<SnapshotRef, Vec<PathBuf>> = BTreeMap::new();

        for row in stmt
            .query_map(
                sql::named_params! {
                    ":file_id": file_id,
                    ":main_id": self.main_snapshot_id,
                    ":comparison_id": self.comparison_snapshot_id,
                },
                |row| {
                    let date: time::OffsetDateTime = row.get("date")?;
                    let timestamp = Timestamp::from(date);
                    let version = row.get("version")?;
                    let comment = row.get("comment")?;
                    let path = row.get("path")?;

                    Ok((
                        SnapshotRef {
                            timestamp,
                            version,
                            comment,
                        },
                        path,
                    ))
                },
            )
            .context("Failed to query for occurrence of file")?
        {
            let (snapshot, path) = row.context("Failed to get row in database")?;

            paths
                .entry(snapshot)
                .or_default()
                .push(PathBuf::from(OsString::from_vec(path)));
        }

        Ok(FileOccurrences { paths })
    }

    /// Find all paths of files within the given snapshot.
    fn find_file_paths_in_snapshot(
        &self,
        file: &fs::File,
        id: SqlId,
    ) -> anyhow::Result<Vec<PathBuf>> {
        let Some(file_id) =
            get_file_id(&self.connection, file)
            .with_context(|| format!("Failed getting the file ID for {:?}", file.sha2_256_hash))? else {
                return Ok(Vec::new());
            };

        let mut stmt = self
            .connection
            .prepare_cached(
                "SELECT
                    path
                FROM
                    Records,
                    Paths
                WHERE
                    file_id = :file_id AND
                    path_id = Paths.id AND
                    snapshot_id = :id",
            )
            .context("Failed to prepare statement for file existence checking")?;

        // This seems to be a false positive clippy lint, this binding here is necessary
        #[allow(clippy::let_and_return)]
        let result = stmt
            .query_and_then(
                sql::named_params! { ":file_id": file_id, ":id": id },
                |row: &sql::Row| -> anyhow::Result<Vec<u8>> {
                    row.get("path").context("Failed getting path from row")
                },
            )?
            .map(|maybe_vec| maybe_vec.map(|vec| PathBuf::from(OsString::from_vec(vec))))
            .collect();

        result
    }

    /// Find potential rename targets for `file` from the main snapshot to the comparison snapshot.
    pub(crate) fn find_potential_rename_targets(
        &self,
        file: &fs::File,
    ) -> anyhow::Result<Vec<PathBuf>> {
        let comparison_id = self.comparison_snapshot_id.ok_or_else(|| {
            anyhow::anyhow!("Cannot get renames without specifying target snapshot")
        })?;

        self.find_file_paths_in_snapshot(file, comparison_id)
    }

    /// Find potential rename sources for `file` from the main snapshot to the comparison snapshot.
    pub(crate) fn find_potential_rename_sources(
        &self,
        file: &fs::File,
    ) -> anyhow::Result<Vec<PathBuf>> {
        let main_id = self.main_snapshot_id.ok_or_else(|| {
            anyhow::anyhow!("Cannot get renames without specifying target snapshot")
        })?;

        self.find_file_paths_in_snapshot(file, main_id)
    }

    /// Whether the path is a known autoruns path.
    pub(crate) fn is_known_autorun_path(&self, path: impl AsRef<Path>) -> anyhow::Result<bool> {
        let path = path.as_ref();

        let mut stmt = self
            .connection
            .prepare_cached(
                "SELECT
                    1
                FROM
                    Autoruns,
                    Paths
                WHERE
                    path_id = Paths.id AND
                    path = :path",
            )
            .context("Failed to prepare statement for path existence checking")?;

        if stmt
            .exists(sql::named_params! {
                ":path": path.as_os_str().as_bytes(),
            })
            .context("could not check if path exists in database")?
        {
            Ok(true)
        } else {
            let normalized_path = path.as_os_str().normalize();
            let mut stmt = self
                .connection
                .prepare_cached(
                    "SELECT
                        1
                    FROM
                        Autoruns,
                        NormalizedPaths
                    WHERE
                        normalized_path_id = NormalizedPaths.id AND
                        path IS :path",
                )
                .context("Failed to prepare statement for normalized path existence checking")?;

            stmt.exists(sql::named_params! {
                ":path": normalized_path,
            })
            .context("could not check if normalized path exists in database")
        }
    }
}
