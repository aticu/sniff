#![feature(let_chains)]

use anyhow::Context as _;
use rayon::prelude::{IntoParallelRefIterator as _, ParallelIterator};
use std::{
    io::Write,
    path::{Path, PathBuf},
};
use structopt::StructOpt;

use crate::timestamp::Timestamp;

mod autoruns;
mod database;
mod diff;
mod fs;
mod snapshot;
mod timestamp;
mod updates;

/// creates and compares snapshots of directories
#[derive(Debug, StructOpt)]
#[allow(clippy::large_enum_variant)]
enum Config {
    /// creates a new snapshot
    CreateSnapshot {
        /// the path to the directory to snapshot
        path: PathBuf,
        /// the directory to place the snapshot in
        out_dir: PathBuf,
        /// a path to the database in which the snapshot should be added
        #[structopt(short = "D", long)]
        database: Option<PathBuf>,
        /// a comment to attach to the snapshot in the database
        #[structopt(short = "C", long)]
        comment: Option<String>,
    },
    /// lists the contents of `entry` in `snapshot`
    Ls {
        /// the snapshot in which to list files
        snapshot: PathBuf,
        /// a snapshot to compare the first one with
        #[structopt(short = "c", long)]
        compare: Option<PathBuf>,
        /// the entry within the snapshots to compare
        entry: Option<PathBuf>,
        /// only show entries with at least one timestamp before the given one
        #[structopt(short = "B", long)]
        before: Option<Timestamp>,
        /// only show entries with at least one timestamp after the given one
        #[structopt(short = "A", long)]
        after: Option<Timestamp>,
        /// only consider modification and creations timestamps for the before and after options
        #[structopt(short = "C", long)]
        only_changes: bool,
        /// whether to show unchanged entries
        #[structopt(short = "u", long)]
        show_unchanged: bool,
        /// whether to show known entries
        #[structopt(short = "k", long)]
        show_known: bool,
        /// the depth at which to start summarizing in the tree
        #[structopt(short = "d", long)]
        summary_depth: Option<u32>,
        /// display only the name of changed files
        #[structopt(short = "r", long)]
        raw: bool,
        /// whether to include metadata changes
        #[structopt(short = "i", long)]
        include_metadata: bool,
        /// whether to show hashes in the tree view
        #[structopt(short = "h", long)]
        show_hashes: bool,
        /// where to output the diff image
        #[structopt(short = "o", long)]
        output_image: Option<PathBuf>,
        /// the allow list of file extensions to consider for differences
        #[structopt(short = "e", long)]
        extensions: Option<String>,
        /// the deny list of file extensions to consider for differences
        ///
        /// the allow list takes precedence over this
        #[structopt(short = "E", long)]
        ignore_extensions: Option<String>,
        /// the size metric to use for visualizations and sorting
        #[structopt(short = "m", long, default_value = "size-on-disk")]
        size_metric: diff::SizeMetric,
        /// only include entries that have the given string as a substring
        #[structopt(short = "g", long)]
        grep: Option<String>,
        /// a path to the database to use during the analysis
        #[structopt(short = "D", long)]
        database: Option<PathBuf>,
    },
    /// updates all snapshots in the "source" directory to the newest version, storing them in "target"
    UpdateSnapshots {
        /// the source folder of the snapshots
        source: PathBuf,
        /// the folder where the updated snapshots are stored
        target: PathBuf,
    },
    /// inserts all snapshots in the given folder into the database
    InsertIntoDatabase {
        /// the file or folder of the snapshot(s) to insert
        file_or_folder: PathBuf,
        /// a path to the database to insert into
        #[structopt(short = "D", long)]
        database: PathBuf,
        /// a comment to attach to the snapshot in the database
        #[structopt(short = "C", long)]
        comment: String,
    },
    /// analyze the autoruns in the given snapshot
    AnalyzeAutoruns {
        /// the snapshot with the autoruns
        snapshot: PathBuf,
        /// an optional snapshot of the same image before the autoruns where recorded
        snapshot_without_autoruns: Option<PathBuf>,
        /// a path to the database to use during the analysis
        #[structopt(short = "D", long)]
        database: PathBuf,
        /// ignore entries where only the file hashes are unknown
        #[structopt(short = "i", long)]
        ignore_unknown_hashes: bool,
    },
}

/// The main function that executes when the program is launched.
fn main() {
    fn run_and_handle_errors() {
        firestorm::profile_fn!(main);

        let config = Config::from_args();

        match run(config) {
            Ok(()) => (),
            Err(err) => eprintln!("{err:?}"),
        }
    }

    if firestorm::enabled() {
        firestorm::bench("./target", run_and_handle_errors).unwrap();
    } else {
        run_and_handle_errors()
    }
}

/// Runs the program.
fn run(config: Config) -> anyhow::Result<()> {
    match config {
        Config::CreateSnapshot {
            path,
            out_dir,
            database,
            comment,
        } => {
            let time = std::time::Instant::now();

            eprintln!("Creating snapshot of {}", path.display());
            let snapshot = snapshot::Snapshot::create(path).context("Could not create snapshot")?;

            let file_name = format!("{:?}.snp", Timestamp::now())
                .replace(' ', "_")
                .replace(':', "_");
            let out_file = if out_dir.extension() == Some(std::ffi::OsStr::new("snp")) {
                out_dir
            } else {
                out_dir.join(file_name)
            };

            snapshot
                .to_file(&out_file)
                .with_context(|| format!("Could not write snapshot file {}", out_file.display()))?;

            if let Some(database) = database {
                let mut db =
                    database::Database::open(database).context("Could not open database")?;

                db.insert_snapshot(
                    &snapshot,
                    &comment.ok_or_else(|| {
                        anyhow::anyhow!(
                            "When specifying the database to insert, also specify a comment"
                        )
                    })?,
                )
                .context("Could not insert snapshot into database")?;
            };

            eprintln!("Snapshot created in {:.2?}", time.elapsed());
        }
        Config::Ls {
            snapshot: former,
            compare: latter,
            entry,
            before,
            after,
            only_changes,
            show_unchanged,
            show_known,
            summary_depth,
            raw,
            include_metadata,
            show_hashes,
            output_image,
            extensions,
            ignore_extensions,
            size_metric,
            grep,
            database,
        } => {
            let (former, latter) = std::thread::scope(|s| {
                firestorm::profile_section!(load_snapshots);

                let former = s.spawn(|| {
                    snapshot::Snapshot::from_file(&former).with_context(|| {
                        format!("Could not read snapshot from file {}", former.display())
                    })
                });

                let latter = s.spawn(|| {
                    latter
                        .map(|latter| {
                            snapshot::Snapshot::from_file(&latter).with_context(|| {
                                format!("Could not read snapshot from file {}", latter.display())
                            })
                        })
                        .transpose()
                });

                (former.join().unwrap(), latter.join().unwrap())
            });
            let former = former?;
            let latter = latter?;

            let database = if let Some(database) = database {
                let mut db =
                    database::Database::open(database).context("Could not open database")?;

                db.main_snapshot(&former)?;
                if let Some(latter) = &latter {
                    db.comparison_snapshot(latter)?;
                }

                Some(db)
            } else {
                None
            };

            let full_diff = {
                firestorm::profile_section!(diff_computation);

                if let Some(ref latter) = latter {
                    diff::DiffTree::compute(&former.root, &latter.root)
                } else {
                    diff::DiffTree::unchanged(&former.root)
                }
            };

            let path = entry.as_deref().unwrap_or_else(|| Path::new("/"));
            let diff = full_diff
                .get(path)
                .with_context(|| format!("Could find path {} in diff", path.display()))?;

            // Note that filter execution is short circuiting, so filters that are fast to execute
            // should come first and slow filters should come later.
            // Otherwise the order does not matter.
            let mut filters: Vec<diff::filters::DynFilter> = Vec::new();

            if !show_unchanged && latter.is_some() {
                filters.push(Box::new(diff::filters::changes_only(include_metadata)));
            }

            if before.is_some() || after.is_some() {
                filters.push(Box::new(diff::filters::timestamps(
                    before,
                    after,
                    only_changes,
                )));
            }

            if let Some(extensions) = &extensions {
                filters.push(Box::new(diff::filters::extensions_only(
                    extensions.split(','),
                )));
            }

            if let Some(ignore_extensions) = &ignore_extensions {
                filters.push(Box::new(diff::filters::extensions_none_of(
                    ignore_extensions.split(','),
                )));
            }

            if let Some(grep) = &grep {
                let grep = grep.to_lowercase();
                filters.push(Box::new(move |ctx| {
                    if let Some(name) = ctx.name.to_str() {
                        name.to_lowercase().contains(&grep)
                    } else {
                        false
                    }
                }));
            }

            if !show_known {
                filters.push(Box::new(diff::filters::unknown_only));
            }

            let filter = diff::filters::all_of(filters);

            if raw {
                for entry in diff
                    .walk()
                    .filter(|entry| entry.filter(database.as_ref(), &filter).unwrap_or(false))
                {
                    let mut path = PathBuf::from(path);
                    path.extend(
                        entry
                            .path_components()
                            .skip_while(|component| *component == std::ffi::OsStr::new("/")),
                    );

                    println!("{}", path.display());
                }
            } else {
                print!(
                    "{}",
                    diff.display_as_tree(
                        path.as_os_str(),
                        &filter,
                        summary_depth,
                        size_metric,
                        show_hashes,
                        database.as_ref(),
                    )
                );
            }

            if let Some(output_image) = output_image {
                firestorm::profile_section!(image_generation);

                let mut display_filters: Vec<diff::display_filters::DynDisplayFilter> = Vec::new();

                if let Some(extensions) = &extensions {
                    use fs::OsStrExt as _;
                    use image::Rgb;

                    const EXT_COLORS: &[Rgb<u8>] = &[
                        Rgb([0, 255, 255]),
                        Rgb([0, 255, 0]),
                        Rgb([255, 0, 255]),
                        Rgb([255, 128, 0]),
                    ];

                    for (i, extension) in extensions.split(',').enumerate() {
                        display_filters.push(Box::new(diff::display_filters::highlight(
                            *EXT_COLORS
                                .get(i)
                                .unwrap_or(&EXT_COLORS[EXT_COLORS.len() - 1]),
                            move |ctx| {
                                ctx.name.has_extension(extension)
                                    && ctx.entry.context.is_unchanged(include_metadata)
                            },
                        )));
                    }

                    display_filters.push(Box::new(diff::display_filters::ignore(|ctx| {
                        extensions
                            .split(',')
                            .all(|ext| !ctx.name.has_extension(ext))
                    })));
                }

                if let Some(ignore_extensions) = &ignore_extensions {
                    use fs::OsStrExt as _;

                    for extension in ignore_extensions.split(',') {
                        display_filters.push(Box::new(diff::display_filters::ignore(move |ctx| {
                            ctx.name.has_extension(extension)
                        })));
                    }
                }

                if !include_metadata {
                    display_filters.push(Box::new(diff::display_filters::ignore_changed_metadata));
                }

                if latter.is_none() || show_known {
                    display_filters.push(Box::new(diff::display_filters::highlight_known(
                        image::Rgb([255, 255, 255]),
                    )));
                }

                let display_filter = diff::display_filters::all_of(display_filters);

                diff::visualize::generate_image(
                    output_image,
                    path.as_os_str(),
                    diff,
                    diff::visualize::VisualizationContext {
                        color_filter: &display_filter,
                        size_metric,
                        database: database.as_ref(),
                    },
                )
                .context("Could not generate diff image")?;
            }

            {
                firestorm::profile_section!(snapshot_dropping);

                drop(former);
                drop(latter);
            }
        }
        Config::UpdateSnapshots { source, target } => {
            let dir_iter = std::fs::read_dir(&source)
                .with_context(|| format!("Failed to read directory {}", source.display()))?;

            for entry in dir_iter {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(err) => {
                        eprintln!("error getting directory entry: {}", err);
                        continue;
                    }
                };

                let mut out_path = target.clone();
                out_path.push(entry.file_name());

                eprint!("{} -> {}...", entry.path().display(), out_path.display());
                std::io::stderr().flush().ok();

                let snapshot = match snapshot::Snapshot::from_file(entry.path()) {
                    Ok(snapshot) => snapshot,
                    Err(err) => {
                        eprintln!(
                            "Could not read snapshot from file {}: {}",
                            entry.path().display(),
                            err
                        );
                        continue;
                    }
                };

                match snapshot.to_file(&out_path) {
                    Ok(()) => (),
                    Err(err) => {
                        eprintln!(
                            "Could not write snapshot to file {}: {}",
                            out_path.display(),
                            err
                        );
                        continue;
                    }
                }

                eprintln!("DONE");
            }
        }
        Config::InsertIntoDatabase {
            file_or_folder,
            database,
            comment,
        } => {
            let mut db = database::Database::open(&database).context("Could not open database")?;

            if file_or_folder.is_file() {
                let snapshot =
                    snapshot::Snapshot::from_file(&file_or_folder).with_context(|| {
                        format!(
                            "Could not read snapshot from file {}",
                            file_or_folder.display()
                        )
                    })?;

                match db.insert_snapshot(&snapshot, &comment) {
                    Ok(_) => (),
                    Err(err) => {
                        eprintln!("WARNING: could not insert snapshot into database: {err:?}");
                    }
                }
            } else {
                let dir_iter = std::fs::read_dir(&file_or_folder).with_context(|| {
                    format!("Failed to read directory {}", file_or_folder.display())
                })?;

                let mut paths = Vec::new();

                for entry in dir_iter {
                    let entry = match entry {
                        Ok(entry) => entry,
                        Err(err) => {
                            eprintln!("error getting directory entry: {}", err);
                            continue;
                        }
                    };

                    paths.push(entry.path());
                }

                let (send, recv) = crossbeam_channel::bounded(4);
                let semaphore = std_semaphore::Semaphore::new(4);

                std::thread::scope(|scope| {
                    scope.spawn(|| {
                        paths.par_iter().for_each(|path| {
                            let _guard = semaphore.access();

                            send.send(snapshot::Snapshot::from_file(path).with_context(|| {
                                format!("Could not read snapshot from file {}", path.display())
                            }))
                            .unwrap();
                        });
                    });

                    let mut num_recv = 0;
                    for snapshot in recv.iter().take(paths.len()) {
                        num_recv += 1;
                        match snapshot {
                            Ok(snapshot) => {
                                match db.insert_snapshot(&snapshot, &comment) {
                                    Ok(_) => (),
                                    Err(err) => {
                                        eprintln!("could not insert snapshot into database: {err}");
                                    }
                                };
                                drop(snapshot);
                            }
                            Err(err) => {
                                eprintln!("{err}");
                            }
                        }
                        eprintln!("Inserted {}/{} snapshots", num_recv, paths.len());
                    }
                });
            }
        }
        Config::AnalyzeAutoruns {
            snapshot,
            snapshot_without_autoruns,
            database,
            ignore_unknown_hashes,
        } => {
            let mut db = database::Database::open(&database).context("Could not open database")?;
            let snapshot = snapshot::Snapshot::from_file(&snapshot).with_context(|| {
                format!("Could not read snapshot from file {}", snapshot.display())
            })?;
            let snapshot_without_autoruns = match snapshot_without_autoruns {
                Some(path) => Some(snapshot::Snapshot::from_file(&path).with_context(|| {
                    format!(
                        "Could not read snapshot without autoruns from file {}",
                        path.display()
                    )
                })?),
                None => None,
            };
            let Some(autoruns) = &snapshot.autoruns else {
                anyhow::bail!("Snapshot does not contain autoruns information");
            };

            db.main_snapshot(&snapshot)
                .context("Could not communicate with database")?;
            if let Some(snapshot_without_autoruns) = &snapshot_without_autoruns {
                db.comparison_snapshot(snapshot_without_autoruns)
                    .context("Could not communicate with database")?;
            }

            for entry in &autoruns.entries {
                if entry.entry.is_empty() && entry.image_path.is_none() {
                    continue;
                }

                let result = entry.evaluate(&db, &snapshot, snapshot_without_autoruns.as_ref())?;
                if !result.should_be_printed() {
                    continue;
                }
                if ignore_unknown_hashes && result.unknown_hash_only() {
                    continue;
                }
                println!("{}", result);
            }
        }
    }

    Ok(())
}
