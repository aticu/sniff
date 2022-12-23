//! Display differences of metadata.

use std::{cmp::Ordering, collections::BTreeSet, ffi::OsString, fmt};

use owo_colors::OwoColorize as _;

use crate::{fs::metadata, timestamp::Timestamp};

/// The text to use for unknown values.
const UNKOWN_TEXT: &str = "<unknown>";

/// A helper for displaying an option more cleanly.
struct DisplayOption<T: fmt::Display>(Option<T>);

impl<T: fmt::Display> fmt::Display for DisplayOption<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Some(inner) => write!(f, "{}", inner),
            None => write!(f, "{}", UNKOWN_TEXT),
        }
    }
}

/// Write the separator between two parts of the diff, if necessary.
fn write_separator(f: &mut fmt::Formatter, detailed: bool, use_sep: &mut bool) -> fmt::Result {
    if *use_sep {
        if detailed {
            write!(f, ",\n    ")?;
        } else {
            write!(f, ", ")?;
        }
    }
    *use_sep = true;

    Ok(())
}

/// Display a possible difference between the `former` and the `latter` size.
fn display_size(
    f: &mut fmt::Formatter,
    former: u64,
    latter: Option<u64>,
    detailed: bool,
    use_sep: &mut bool,
) -> fmt::Result {
    write_separator(f, detailed, use_sep)?;

    let former_exact = if detailed && former >= 1024 {
        format!(" ({former} bytes)")
    } else {
        String::new()
    };

    if let Some(latter) = latter {
        let latter_exact = if detailed && latter >= 1024 && former != latter {
            format!(" ({latter} bytes)")
        } else {
            String::new()
        };

        if former == latter {
            write!(
                f,
                "size: {}B{}",
                size_format::SizeFormatterBinary::new(former),
                former_exact
            )?;
        } else {
            write!(
                f,
                "size: {}{} -> {}{}",
                format_args!("{}B", size_format::SizeFormatterBinary::new(former)).red(),
                former_exact,
                format_args!("{}B", size_format::SizeFormatterBinary::new(latter)).green(),
                latter_exact
            )?;
        }

        match former.cmp(&latter) {
            Ordering::Less => {
                write!(
                    f,
                    " ({}, {})",
                    format_args!(
                        "+{}B",
                        size_format::SizeFormatterBinary::new(latter - former)
                    )
                    .green(),
                    format_args!("+{:.2}%", latter as f64 / former as f64 * 100.0 - 100.0).green()
                )
            }
            Ordering::Equal => Ok(()),
            Ordering::Greater => {
                write!(
                    f,
                    " ({}, {})",
                    format_args!(
                        "-{}B",
                        size_format::SizeFormatterBinary::new(former - latter)
                    )
                    .red(),
                    format_args!("-{:.2}%", former as f64 / latter as f64 * 100.0 - 100.0).red()
                )
            }
        }
    } else {
        write!(
            f,
            "size: {}B{}",
            size_format::SizeFormatterBinary::new(former),
            former_exact
        )
    }
}

/// Display a possible difference between the `former` and the `latter` time stamp.
fn display_timestamp(
    f: &mut fmt::Formatter,
    former: Option<Timestamp>,
    latter: Option<Option<Timestamp>>,
    name: &str,
    detailed: bool,
    use_sep: &mut bool,
) -> fmt::Result {
    if let Some(latter) = latter {
        match (former, latter) {
            (Some(former), Some(latter)) => match former.cmp(&latter) {
                Ordering::Less => {
                    write_separator(f, detailed, use_sep)?;
                    if detailed {
                        write!(
                            f,
                            "{name}: {:?} -> {:?} ({})",
                            former.red(),
                            latter.green(),
                            format_args!("+{:#}", (latter - former)).green()
                        )
                    } else {
                        write!(f, "{name}: {}", (latter - former).green())
                    }
                }
                Ordering::Equal => {
                    if detailed {
                        write_separator(f, detailed, use_sep)?;
                        write!(f, "{name}: {:?}", former)
                    } else {
                        Ok(())
                    }
                }
                Ordering::Greater => {
                    write_separator(f, detailed, use_sep)?;
                    if detailed {
                        write!(
                            f,
                            "{name}: {:?} -> {:?} ({})",
                            former.red(),
                            latter.green(),
                            format_args!("-{:#}", (former - latter)).red()
                        )
                    } else {
                        write!(f, "{name}: {}", (former - latter).red())
                    }
                }
            },
            (Some(former), None) => {
                write_separator(f, detailed, use_sep)?;
                write!(f, "{name}: {:?} -> {}", former.red(), UNKOWN_TEXT.green())
            }
            (None, Some(latter)) => {
                write_separator(f, detailed, use_sep)?;
                write!(f, "{name}: {} -> {:?}", UNKOWN_TEXT.red(), latter.green())
            }
            (None, None) => Ok(()),
        }
    } else if detailed {
        if let Some(former) = former {
            write_separator(f, detailed, use_sep)?;
            write!(f, "{name}: {:?}", former)
        } else {
            Ok(())
        }
    } else {
        Ok(())
    }
}

/// Display a possible difference between the `former` and the `latter` attributes.
fn display_ntfs_attributes(
    f: &mut fmt::Formatter,
    former: Option<metadata::NtfsAttributes>,
    latter: Option<Option<metadata::NtfsAttributes>>,
    detailed: bool,
    use_sep: &mut bool,
) -> fmt::Result {
    if let Some(latter) = latter &&
        former != latter {
        write_separator(f, detailed, use_sep)?;

        write!(f, "attributes: ")?;

        match (former, latter) {
            (Some(former), Some(latter)) => {
                let removed = former.difference(latter);
                let added = latter.difference(former);
                let same = latter.intersection(former);

                let mut sep = false;

                if detailed && !same.is_empty() {
                    write!(f, "{:?}", same)?;
                    sep = true;
                }

                if !removed.is_empty() {
                    if sep {
                        write!(f, " | ")?;
                    }
                    write!(f, "{:?}", removed.red())?;
                    sep = true;
                }

                if !added.is_empty() {
                    if sep {
                        write!(f, " | ")?;
                    }
                    write!(f, "{:?}", added.green())?;
                }
            }
            (Some(former), None) => write!(f, "{:?} -> {}", former.red(), UNKOWN_TEXT.green())?,
            (None, Some(latter)) => write!(f, "{} -> {:?}", UNKOWN_TEXT.red(), latter.green())?,
            (None, None) => unreachable!("None == None"),
        }
    } else if detailed {
        if let Some(former) = former {
            write_separator(f, detailed, use_sep)?;
            write!(f, "attributes: {:?}", former)?;
        }
    }

    Ok(())
}

/// Display a possible difference between the `former` and the `latter` byte lists.
fn display_byte_list(
    f: &mut fmt::Formatter,
    former: &Option<Vec<u8>>,
    latter: Option<&Option<Vec<u8>>>,
    name: &str,
    detailed: bool,
    use_sep: &mut bool,
) -> fmt::Result {
    if let Some(latter) = latter &&
        former != latter {
        write_separator(f, detailed, use_sep)?;

        if detailed {
            match (former, latter) {
                (Some(_), Some(_)) => write!(f, "{}: ", name)?,
                (Some(_), None) => write!(f, "{}: ", name.red())?,
                (None, Some(_)) => write!(f, "{}: ", name.green())?,
                (None, None) => unreachable!(),
            }
            writeln!(f)?;
            super::display_hexdump(
                f,
                "        ",
                former.as_deref().unwrap_or(&[]),
                Some(latter.as_deref().unwrap_or(&[])),
                true,
                true,
            )?;
        } else {
            match (former, latter) {
                (Some(_), Some(_)) => {
                    write!(f, "{}", name.yellow())?;
                }
                (Some(_), None) => {
                    write!(f, "{}", name.red())?;
                }
                (None, Some(_)) => {
                    write!(f, "{}", name.green())?;
                }
                (None, None) => unreachable!(),
            }
        }
    } else if detailed {
        if let Some(former) = former {
            write_separator(f, detailed, use_sep)?;
            write!(f, "{name}: ")?;
            writeln!(f)?;
            super::display_hexdump(f, "        ", former, None, true, true)?;
        }
    }

    Ok(())
}

/// Display a possible difference between the `former` and the `latter` alternate data streams.
fn display_ads(
    f: &mut fmt::Formatter,
    former: &Option<crate::fs::metadata::AlternateDataStreams>,
    latter: Option<&Option<crate::fs::metadata::AlternateDataStreams>>,
    detailed: bool,
    use_sep: &mut bool,
) -> fmt::Result {
    if let Some(latter) = latter {
        if former != latter {
            write_separator(f, detailed, use_sep)?;

            let empty_map = std::collections::BTreeMap::new();
            let former_streams = former
                .as_ref()
                .map(|former| &former.streams)
                .unwrap_or(&empty_map);
            let latter_streams = latter
                .as_ref()
                .map(|latter| &latter.streams)
                .unwrap_or(&empty_map);

            write!(f, "ADS: (")?;
            let removed: BTreeSet<&OsString> = former_streams
                .keys()
                .filter(|&key| !latter_streams.contains_key(key))
                .collect();
            let added: BTreeSet<&OsString> = latter_streams
                .keys()
                .filter(|&key| !former_streams.contains_key(key))
                .collect();
            let changed: BTreeSet<&OsString> = former_streams
                .iter()
                .filter_map(|(key, value)| match latter_streams.get(&**key) {
                    Some(other_value) if value != other_value => Some(key),
                    _ => None,
                })
                .collect();

            if !removed.is_empty() {
                write!(f, " {:?}", removed.red())?;
            }

            if !added.is_empty() {
                write!(f, " {:?}", added.green())?;
            }

            if !changed.is_empty() {
                write!(f, " {:?}", changed.yellow())?;
            }

            write!(f, ")")?;
        }
    }

    Ok(())
}

/// Display a possible difference between the `former` and the `latter` property
/// implementing `fmt::Display`.
fn display_named_display<T: fmt::Display + Eq>(
    f: &mut fmt::Formatter,
    former: Option<T>,
    latter: Option<Option<T>>,
    name: &str,
    detailed: bool,
    use_sep: &mut bool,
) -> fmt::Result {
    if let Some(latter) = latter &&
        former != latter {
        write_separator(f, detailed, use_sep)?;
        if detailed {
            write!(
                f,
                "{name}: {} -> {}",
                DisplayOption(former).red(),
                DisplayOption(latter).green()
            )?;
        } else {
            write!(f, "{}", name.yellow())?;
        }
    } else if detailed {
        write_separator(f, detailed, use_sep)?;
        write!(f, "{name}: {}", DisplayOption(former))?;
    }

    Ok(())
}

/// Displays the metadata and possible difference between `former` and `latter` into `f`.
pub(super) fn display_metadata(
    f: &mut fmt::Formatter,
    former: &crate::fs::Metadata,
    latter: Option<&crate::fs::Metadata>,
    detailed: bool,
) -> fmt::Result {
    firestorm::profile_fn!(display_metadata);

    let mut use_sep = false;

    if detailed {
        writeln!(f, "metadata: {{")?;
        write!(f, "    ")?;
    } else {
        write!(f, " {{ ")?;
    }

    display_size(
        f,
        former.size,
        latter.map(|m| m.size),
        detailed,
        &mut use_sep,
    )?;
    if detailed {
        display_timestamp(
            f,
            former.created,
            latter.map(|m| m.created),
            if detailed { "created" } else { "C" },
            detailed,
            &mut use_sep,
        )?;
        display_timestamp(
            f,
            former.modified,
            latter.map(|m| m.modified),
            if detailed { "modified" } else { "M" },
            detailed,
            &mut use_sep,
        )?;
        display_timestamp(
            f,
            former.accessed,
            latter.map(|m| m.accessed),
            if detailed { "accessed" } else { "A" },
            detailed,
            &mut use_sep,
        )?;
        display_timestamp(
            f,
            former.mft_modified,
            latter.map(|m| m.mft_modified),
            if detailed { "MFT modified" } else { "MFTM" },
            detailed,
            &mut use_sep,
        )?;
    } else if let Some(latter) = latter {
        if former.created != latter.created
            || former.modified != latter.modified
            || former.accessed != latter.accessed
            || former.mft_modified != latter.mft_modified
        {
            write_separator(f, detailed, &mut use_sep)?;
            write!(f, "{}", "📅".yellow())?;
        }
    }
    display_ntfs_attributes(
        f,
        former.ntfs_attributes,
        latter.map(|m| m.ntfs_attributes),
        detailed,
        &mut use_sep,
    )?;
    display_named_display(
        f,
        former.unix_permissions.map(umask::Mode::from),
        latter.map(|m| m.unix_permissions.map(umask::Mode::from)),
        "permissions",
        detailed,
        &mut use_sep,
    )?;
    display_named_display(
        f,
        former.nlink,
        latter.map(|m| m.nlink),
        if detailed { "hard links" } else { "nlink" },
        detailed,
        &mut use_sep,
    )?;
    display_named_display(
        f,
        former.uid,
        latter.map(|m| m.uid),
        "UID",
        detailed,
        &mut use_sep,
    )?;
    display_named_display(
        f,
        former.gid,
        latter.map(|m| m.gid),
        "GID",
        detailed,
        &mut use_sep,
    )?;
    display_byte_list(
        f,
        &former.reparse_data,
        latter.map(|m| &m.reparse_data),
        "reparse data",
        detailed,
        &mut use_sep,
    )?;
    display_byte_list(
        f,
        &former.acl,
        latter.map(|m| &m.acl),
        "acl",
        detailed,
        &mut use_sep,
    )?;
    display_byte_list(
        f,
        &former.dos_name,
        latter.map(|m| &m.dos_name),
        "dos name",
        detailed,
        &mut use_sep,
    )?;
    display_byte_list(
        f,
        &former.object_id,
        latter.map(|m| &m.object_id),
        "object ID",
        detailed,
        &mut use_sep,
    )?;
    display_byte_list(
        f,
        &former.efs_info,
        latter.map(|m| &m.efs_info),
        "EFS info",
        detailed,
        &mut use_sep,
    )?;
    display_byte_list(
        f,
        &former.ea,
        latter.map(|m| &m.ea),
        "EA",
        detailed,
        &mut use_sep,
    )?;
    display_ads(
        f,
        &former.streams,
        latter.map(|m| &m.streams),
        detailed,
        &mut use_sep,
    )?;
    display_named_display(
        f,
        former.inode,
        latter.map(|m| m.inode),
        "inode",
        detailed,
        &mut use_sep,
    )?;

    if detailed {
        if use_sep {
            writeln!(f)?;
        }
        writeln!(f, "}}")?;
    } else {
        write!(f, " }}")?;
    }

    Ok(())
}
