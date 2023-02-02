//! Display differences of files.

use std::fmt;

use owo_colors::OwoColorize as _;

use crate::{
    database::Database,
    fs::{file::FileFlags, File},
};

/// The width to use for the descriptions in detailed displays.
const DETAILED_WIDTH: usize = 20;

/// Displays a hash and possibly its change into the formatter.
fn display_hash<H: fmt::Debug + Eq>(
    f: &mut fmt::Formatter,
    hash: H,
    latter_hash: Option<H>,
    name: &str,
    detailed: bool,
) -> fmt::Result {
    if detailed {
        write!(f, "{:DETAILED_WIDTH$}", format!("{name}:"))?;
    } else {
        write!(f, " ")?;
    }

    if let Some(latter_hash) = latter_hash {
        if latter_hash != hash {
            write!(f, "{:?} -> {:?}", hash.red(), latter_hash.green())?;
        } else {
            write!(f, "{:?}", hash)?;
        }
    } else {
        write!(f, "{:?}", hash)?;
    }

    if detailed {
        writeln!(f)
    } else {
        Ok(())
    }
}

/// Displays entropy and possibly its change into the formatter.
fn display_entropy(
    f: &mut fmt::Formatter,
    entropy: f32,
    latter_entropy: Option<f32>,
    detailed: bool,
) -> fmt::Result {
    use owo_colors::colored::Color::{self, *};
    const ENTROPY_TABLE: &[char] = &[' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    const COLOR_TABLE: &[Color] = &[
        BrightBlack,
        BrightGreen,
        Green,
        BrightWhite,
        White,
        BrightYellow,
        Yellow,
        BrightRed,
        Red,
    ];

    if detailed {
        write!(f, "{:DETAILED_WIDTH$}", "entropy:")?;
        if let Some(latter_entropy) = latter_entropy {
            if (latter_entropy * 100.0).round() != (entropy * 100.0).round() {
                writeln!(f, "{:.2} -> {:.2}", entropy.red(), latter_entropy.green())
            } else {
                writeln!(f, "{:.2}", entropy)
            }
        } else {
            writeln!(f, "{:.2}", entropy)
        }
    } else {
        write!(f, " ")?;

        let entropy = entropy.round().clamp(0.0, 8.0) as usize;
        if let Some(latter_entropy) = latter_entropy {
            let latter_entropy = latter_entropy.round().clamp(0.0, 8.0) as usize;
            if latter_entropy != entropy {
                write!(
                    f,
                    "{} -> {}",
                    ENTROPY_TABLE[entropy]
                        .color(COLOR_TABLE[entropy])
                        .on_bright_black(),
                    ENTROPY_TABLE[latter_entropy]
                        .color(COLOR_TABLE[latter_entropy])
                        .on_bright_black()
                )
            } else {
                write!(
                    f,
                    "{}",
                    ENTROPY_TABLE[entropy]
                        .color(COLOR_TABLE[entropy])
                        .on_bright_black()
                )
            }
        } else {
            write!(
                f,
                "{}",
                ENTROPY_TABLE[entropy]
                    .color(COLOR_TABLE[entropy])
                    .on_bright_black()
            )
        }
    }
}

/// Displays the first bytes and possibly their change into the formatter.
fn display_first_bytes(
    f: &mut fmt::Formatter,
    first_bytes: &[u8],
    latter_first_bytes: Option<&[u8]>,
) -> fmt::Result {
    write!(f, "{:DETAILED_WIDTH$}", "first bytes:")?;
    if Some(first_bytes) != latter_first_bytes && latter_first_bytes.is_some() {
        // Add a newline so the diff looks aligned
        writeln!(f)?;
        super::display_hexdump(f, "    ", first_bytes, latter_first_bytes, false, true)?;
    } else if !first_bytes.is_empty() {
        super::display_hexdump(f, "", first_bytes, latter_first_bytes, false, true)?;
    } else {
        write!(f, "<empty>")?;
    }
    writeln!(f)
}

/// Displays the COFF header and possibly its change into the formatter.
fn display_coff_header(
    f: &mut fmt::Formatter,
    coff_header: Option<&[u8]>,
    latter_coff_header: Option<Option<&[u8]>>,
    detailed: bool,
) -> fmt::Result {
    if let Some(latter_coff_header) = latter_coff_header {
        if coff_header != latter_coff_header {
            if detailed {
                match (coff_header, latter_coff_header) {
                    (Some(_), Some(_)) => write!(f, "COFF-header: ")?,
                    (Some(_), None) => write!(f, "{}: ", "COFF-header: ".red())?,
                    (None, Some(_)) => write!(f, "{}: ", "COFF-header: ".green())?,
                    (None, None) => unreachable!(),
                }
                writeln!(f)?;
                super::display_hexdump(
                    f,
                    "        ",
                    coff_header.unwrap_or(&[]),
                    Some(latter_coff_header.unwrap_or(&[])),
                    true,
                    true,
                )?;
                writeln!(f)?;
            } else {
                match (coff_header, latter_coff_header) {
                    (Some(_), Some(_)) => {
                        write!(f, ", {}", "PE".yellow())?;
                    }
                    (Some(_), None) => {
                        write!(f, ", {}", "PE".red())?;
                    }
                    (None, Some(_)) => {
                        write!(f, ", {}", "PE".green())?;
                    }
                    (None, None) => unreachable!(),
                }
            }
        } else if detailed {
            if let Some(coff_header) = coff_header {
                write!(f, "COFF-header: ")?;
                writeln!(f)?;
                super::display_hexdump(f, "        ", coff_header, None, true, true)?;
                writeln!(f)?;
            }
        } else if coff_header.is_some() {
            write!(f, ", {}", "PE".blue())?;
        }
    } else if detailed {
        if let Some(coff_header) = coff_header {
            write!(f, "COFF-header: ")?;
            writeln!(f)?;
            super::display_hexdump(f, "        ", coff_header, None, true, true)?;
            writeln!(f)?;
        }
    } else if coff_header.is_some() {
        write!(f, ", {}", "PE".blue())?;
    }

    Ok(())
}

/// Displays the file flags and possibly their change into the formatter.
fn display_file_flags(
    f: &mut fmt::Formatter,
    flags: FileFlags,
    latter_flags: Option<FileFlags>,
    detailed: bool,
) -> fmt::Result {
    if detailed {
        fn display_flag(
            f: &mut fmt::Formatter,
            name: &str,
            flags: FileFlags,
            latter_flags: Option<FileFlags>,
            flag: FileFlags,
        ) -> fmt::Result {
            let yes = "yes";
            let no = "no";

            write!(f, "{:DETAILED_WIDTH$}", format!("{name}:"))?;

            match (
                flags.contains(flag),
                latter_flags.map(|inner| inner.contains(flag)),
            ) {
                (true, None) => writeln!(f, "{}", yes),
                (false, None) => writeln!(f, "{}", no),
                (true, Some(true)) => writeln!(f, "{}", yes),
                (true, Some(false)) => writeln!(f, "{} -> {}", yes.red(), no.green()),
                (false, Some(true)) => writeln!(f, "{} -> {}", no.red(), yes.green()),
                (false, Some(false)) => writeln!(f, "{}", no),
            }
        }

        display_flag(f, "valid UTF-8", flags, latter_flags, FileFlags::UTF8)?;
        display_flag(f, "valid UTF-16BE", flags, latter_flags, FileFlags::UTF16BE)?;
        display_flag(f, "valid UTF-16LE", flags, latter_flags, FileFlags::UTF16LE)?;
        display_flag(f, "valid UTF-32BE", flags, latter_flags, FileFlags::UTF32BE)?;
        display_flag(f, "valid UTF-32LE", flags, latter_flags, FileFlags::UTF32LE)?;

        Ok(())
    } else {
        match (
            flags.intersects(FileFlags::UTF_ENCODING),
            latter_flags.map(|inner| inner.intersects(FileFlags::UTF_ENCODING)),
        ) {
            (true, None) | (true, Some(true)) => write!(f, ", {}", "text".blue()),
            (true, Some(false)) => write!(f, ", {}", "text".red()),
            (false, Some(true)) => write!(f, ", {}", "text".green()),
            (false, None) | (false, Some(false)) => Ok(()),
        }
    }
}

/// Display the occurrences of the between the `former` and the `latter` file in the given database.
pub(super) fn display_occurrences(
    f: &mut fmt::Formatter,
    former: &File,
    latter: Option<&File>,
    database: &Database,
    detailed: bool,
) -> fmt::Result {
    let same = latter.map(|latter| former == latter).unwrap_or(true);
    let former = database
        .file_occurrences(former)
        .ok()
        .filter(|occ| !occ.paths.is_empty());
    let latter = latter
        .and_then(|latter| database.file_occurrences(latter).ok())
        .filter(|occ| !occ.paths.is_empty());

    if detailed {
        let display_snapshot = |f: &mut fmt::Formatter, snapshot, paths| {
            writeln!(f, "    {} [", snapshot)?;
            let mut first = true;
            for path in paths {
                if !first {
                    writeln!(f, ",")?;
                }
                first = false;

                write!(f, "        {:?}", path)?;
            }
            writeln!(f)?;
            writeln!(f, "    ]")?;

            Ok(())
        };

        if let Some(former) = former {
            if same {
                writeln!(f, "File found here:")?;
            } else {
                writeln!(f, "Previous file found here:")?;
            }

            for (snapshot, paths) in former.paths.into_iter() {
                display_snapshot(f, snapshot, paths)?;
            }
        }

        if let Some(latter) = latter && !same {
            writeln!(f, "New file found here:")?;

            for (snapshot, paths) in latter.paths.into_iter() {
                display_snapshot(f, snapshot, paths)?;
            }
        }
    } else {
        let date_range = |occurrences: crate::database::FileOccurrences| {
            let first = occurrences
                .paths
                .iter()
                .next()
                .expect("File did not occur in snapshot, if there isn't at least one path");
            let last = occurrences
                .paths
                .iter()
                .next_back()
                .expect("File did not occur in snapshot, if there isn't at least one path");
            let first_comment = &occurrences
                .paths
                .iter()
                .next()
                .expect("File did not occur in snapshot, if there isn't at least one path")
                .0
                .comment;
            let comment = if occurrences
                .paths
                .iter()
                .all(|occ| &occ.0.comment == first_comment)
            {
                Some(first_comment.clone())
            } else {
                None
            };

            (
                crate::timestamp::DateRange {
                    from: first.0.timestamp,
                    to: last.0.timestamp,
                },
                comment,
            )
        };

        if same {
            if let Some(former) = former {
                let (range, comment) = date_range(former);

                write!(
                    f,
                    " (seen {}{})",
                    range,
                    if let Some(comment) = comment {
                        format!(" on {comment}")
                    } else {
                        String::new()
                    }
                )?;
            }
        } else if former.is_some() || latter.is_some() {
            let former = if let Some(former) = former {
                let (range, comment) = date_range(former);

                format!(
                    "{}{}",
                    range,
                    if let Some(comment) = comment {
                        format!(" on {comment}")
                    } else {
                        String::new()
                    }
                )
            } else {
                "never".to_string()
            };

            let latter = if let Some(latter) = latter {
                let (range, comment) = date_range(latter);

                format!(
                    "{}{}",
                    range,
                    if let Some(comment) = comment {
                        format!(" on {comment}")
                    } else {
                        String::new()
                    }
                )
            } else {
                "never".to_string()
            };

            write!(f, " (seen {} -> {})", former.red(), latter.green())?;
        }
    }

    Ok(())
}

/// Displays possible renames of the file.
pub(super) fn display_renames(
    f: &mut fmt::Formatter,
    file: &File,
    context: &super::DiffType,
    database: &Database,
    detailed: bool,
) -> fmt::Result {
    firestorm::profile_fn!(display_renames);

    if let Some((mut paths, own_paths, direction)) = match context {
        super::DiffType::Added => Some((
            database
                .find_potential_rename_sources(file)
                .unwrap_or_default(),
            database
                .find_potential_rename_targets(file)
                .unwrap_or_default(),
            "from",
        )),
        super::DiffType::Removed => Some((
            database
                .find_potential_rename_targets(file)
                .unwrap_or_default(),
            database
                .find_potential_rename_sources(file)
                .unwrap_or_default(),
            "to",
        )),
        _ => None,
    } {
        // Remove all paths that are also present in the target snapshot, since those very likely
        // just stayed where they are
        paths.retain(|path| !own_paths.contains(path));

        if detailed {
            match paths.len() {
                0 => (),
                1 => writeln!(f, "Potentially renamed {direction} {:?}", paths[0])?,
                _ => {
                    writeln!(f, "Potentially renamed {direction} [")?;
                    let mut first = true;
                    for path in paths {
                        if !first {
                            writeln!(f, ",")?;
                        }
                        first = false;

                        write!(f, "    {:?}", path)?;
                    }
                    writeln!(f)?;
                    writeln!(f, "]")?;
                }
            }
        } else if paths.len() == 1 {
            write!(f, " (renamed {direction} {:?}?)", paths[0])?;
        }
    }

    Ok(())
}

/// Display a possible difference between the `former` and the `latter` file.
pub(super) fn display_file(
    f: &mut fmt::Formatter,
    former: &File,
    latter: Option<&File>,
    context: &super::DiffType,
    database: Option<&Database>,
    show_hashes: bool,
    detailed: bool,
) -> fmt::Result {
    firestorm::profile_fn!(display_file);

    display_entropy(f, former.entropy, latter.map(|f| f.entropy), detailed)?;

    if detailed {
        display_hash(
            f,
            &former.sha2_256_hash,
            latter.map(|f| &f.sha2_256_hash),
            "SHA2-256",
            detailed,
        )?;
    }
    if detailed || show_hashes {
        display_hash(
            f,
            &former.md5_hash,
            latter.map(|f| &f.md5_hash),
            "MD5",
            detailed,
        )?;
    }

    if detailed {
        display_first_bytes(
            f,
            &former.first_bytes[..],
            latter.map(|f| &f.first_bytes[..]),
        )?;
    }

    display_coff_header(
        f,
        former.coff_header.as_deref(),
        latter.map(|f| f.coff_header.as_deref()),
        detailed,
    )?;
    display_file_flags(f, former.flags, latter.map(|f| f.flags), detailed)?;

    if let Some(database) = database {
        display_occurrences(f, former, latter, database, detailed)?;

        if latter.is_none() {
            display_renames(f, former, context, database, detailed)?;
        }
    }

    Ok(())
}
