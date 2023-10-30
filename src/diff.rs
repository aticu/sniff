//! Computing and displaying differences between snapshots.

use owo_colors::OwoColorize;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, ffi::OsStr, fmt, str::FromStr};

use crate::{
    database::Database,
    fs::{
        self, dir_entry::GenericDirEntry, dir_entry_type::DirEntryType, DirEntry, MetaDEntry,
        Metadata, OsStrExt as _,
    },
};

use self::{file::display_file, filters::FilterContext, metadata::display_metadata};

pub(crate) mod display_filters;
mod file;
pub(crate) mod filters;
pub(crate) mod metadata;
pub(crate) mod visualize;

/// The possible metrics for measuring the size of diffs.
#[derive(Clone, Copy)]
pub(crate) struct SizeMetric {
    /// The name of this size metric.
    name: Option<&'static str>,
    /// The function that calculates the size of a single entry in the tree.
    calculate: fn(&DiffTree) -> u64,
}

impl From<fn(&DiffTree) -> u64> for SizeMetric {
    fn from(calculate: fn(&DiffTree) -> u64) -> Self {
        Self {
            name: None,
            calculate,
        }
    }
}

impl From<(&'static str, fn(&DiffTree) -> u64)> for SizeMetric {
    fn from((name, calculate): (&'static str, fn(&DiffTree) -> u64)) -> Self {
        Self {
            name: Some(name),
            calculate,
        }
    }
}

impl fmt::Debug for SizeMetric {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("SizeMetric")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Default for SizeMetric {
    fn default() -> Self {
        SizeMetric::from_str("size-on-disk").unwrap()
    }
}

impl FromStr for SizeMetric {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (name, calculate): (&'static str, fn(&DiffTree) -> u64) = match s {
            "num-of-files" | "number-of-files" => ("number-of-files", |entry| match &entry.entry {
                fs::DirEntry::File(_) | fs::DirEntry::Symlink(_) | fs::DirEntry::Other(_) => 1,
                _ => 0,
            }),
            "num-of-changed-files" | "number-of-changed-files" => {
                ("number-of-changed-files", |entry| {
                    match (&entry.entry, &entry.context) {
                        (
                            fs::DirEntry::File(_)
                            | fs::DirEntry::Symlink(_)
                            | fs::DirEntry::Other(_),
                            DiffType::Changed { .. },
                        ) => 1,
                        _ => 0,
                    }
                })
            }
            "num-of-added-files" | "number-of-added-files" => ("number-of-added-files", |entry| {
                match (&entry.entry, &entry.context) {
                    (
                        fs::DirEntry::File(_) | fs::DirEntry::Symlink(_) | fs::DirEntry::Other(_),
                        DiffType::Added,
                    ) => 1,
                    _ => 0,
                }
            }),
            "num-of-removed-files" | "number-of-removed-files" => {
                ("number-of-removed-files", |entry| {
                    match (&entry.entry, &entry.context) {
                        (
                            fs::DirEntry::File(_)
                            | fs::DirEntry::Symlink(_)
                            | fs::DirEntry::Other(_),
                            DiffType::Removed,
                        ) => 1,
                        _ => 0,
                    }
                })
            }
            "num-of-changes" | "number-of-changes" => ("number-of-changes", |entry| {
                match (&entry.entry, &entry.context) {
                    (
                        fs::DirEntry::File(_) | fs::DirEntry::Symlink(_) | fs::DirEntry::Other(_),
                        DiffType::Changed { .. } | DiffType::Added | DiffType::Removed,
                    ) => 1,
                    _ => 0,
                }
            }),
            "size-of-change" => ("size-of-change", |entry| match &entry.context {
                DiffType::Changed { .. } | DiffType::Added | DiffType::Removed => {
                    entry.metadata.size
                }
                _ => 0,
            }),
            "size-on-disk" => ("size-on-disk", |entry| match &entry.context {
                DiffType::Changed { to } => std::cmp::max(entry.metadata.size, to.metadata.size),
                _ => entry.metadata.size,
            }),
            _ => return Err("unrecognized size metric"),
        };

        Ok(Self {
            name: Some(name),
            calculate,
        })
    }
}

/// Displays a hexdump difference of `former` or its difference to a hexdump of `latter`, if
/// `latter` is `Some(_)`.
fn display_hexdump(
    f: &mut fmt::Formatter,
    prefix: &str,
    former: &[u8],
    latter: Option<&[u8]>,
    show_offsets: bool,
    show_equal_lines: bool,
) -> fmt::Result {
    const LINE_LEN: usize = 16;

    fn display_line<C: owo_colors::Color>(
        f: &mut fmt::Formatter,
        line: &[u8],
        highlight: impl Fn(usize) -> bool,
    ) -> fmt::Result {
        for i in 0..LINE_LEN {
            if i == LINE_LEN / 2 {
                write!(f, " ")?;
            }

            if let Some(b) = line.get(i) {
                if highlight(i) {
                    write!(f, "{:02x} ", b.fg::<C>())?;
                } else {
                    write!(f, "{:02x} ", b)?;
                }
            } else {
                write!(f, "   ")?;
            }
        }
        write!(f, " |")?;
        for i in 0..LINE_LEN {
            if let Some(b) = line.get(i) {
                let c = if (0x20..0x80).contains(b) {
                    char::from(*b)
                } else {
                    '.'
                };

                if highlight(i) {
                    write!(f, "{}", c.fg::<C>())?;
                } else {
                    write!(f, "{}", c)?;
                }
            } else {
                write!(f, " ")?;
            }
        }

        write!(f, "|")
    }

    let mut first_line = true;
    let mut skipped_line = false;
    use std::cmp::{max, min};
    for line_start in (0..max(former.len(), latter.map(|l| l.len()).unwrap_or(0))).step_by(LINE_LEN)
    {
        let former_line = if former.len() >= line_start && !former.is_empty() {
            &former[line_start..min(line_start + LINE_LEN, former.len())]
        } else {
            &[]
        };
        if let Some(latter) = latter {
            let latter_line = if latter.len() >= line_start && !latter.is_empty() {
                &latter[line_start..min(line_start + LINE_LEN, latter.len())]
            } else {
                &[]
            };

            if former_line != latter_line || show_equal_lines {
                if !first_line {
                    writeln!(f)?;
                }
                first_line = false;

                if skipped_line {
                    writeln!(f, "{}*", prefix)?;
                    skipped_line = false;
                }

                write!(f, "{}", prefix)?;
                if show_offsets {
                    write!(f, "{:08x}  ", line_start)?;
                }
                display_line::<owo_colors::colors::Red>(f, former_line, |i| {
                    latter_line
                        .get(i)
                        .map(|b| *b != former_line[i])
                        .unwrap_or(true)
                })?;
                if former_line != latter_line {
                    writeln!(f)?;
                    write!(f, "{}", prefix)?;
                    if show_offsets {
                        write!(f, "          ")?;
                    }
                    display_line::<owo_colors::colors::Green>(f, latter_line, |i| {
                        former_line
                            .get(i)
                            .map(|b| *b != latter_line[i])
                            .unwrap_or(true)
                    })?;
                }
            } else {
                skipped_line = true;
            }
        } else {
            if !first_line {
                writeln!(f)?;
            }
            first_line = false;
            write!(f, "{}", prefix)?;
            if show_offsets {
                write!(f, "{:08x}  ", line_start)?;
            }
            display_line::<owo_colors::colors::Default>(f, former_line, |_| false)?;
        }
    }

    Ok(())
}

/// The types of differences that occur.
#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub(crate) enum DiffType {
    /// Nothing is changed (recursively), except for possibly metadata.
    Unchanged {
        /// The metadata was changed to the given metadata.
        metadata_changed_to: Option<Metadata>,
    },
    /// The entry is unchanged, but some of its children were changed.
    ChildrenChanged {
        /// The metadata was changed to the given metadata.
        metadata_changed_to: Option<Metadata>,
    },
    /// The entry was changed to the given other entry.
    Changed {
        /// The entry was changed to this entry.
        to: MetaDEntry,
    },
    /// The entry was added.
    Added,
    /// The entry was removed.
    Removed,
}

impl DiffType {
    /// Returns `true` if this entry was removed.
    pub(crate) fn is_unchanged(&self, consider_metadata: bool) -> bool {
        if consider_metadata {
            matches!(
                self,
                DiffType::Unchanged {
                    metadata_changed_to: None
                }
            )
        } else {
            matches!(self, DiffType::Unchanged { .. })
        }
    }
}

/// A tree representing differences between two file trees.
pub(crate) type DiffTree = MetaDEntry<DiffType>;

impl DiffTree {
    /// Compute the difference tree between two directory entries.
    pub(crate) fn compute(former: &MetaDEntry, latter: &MetaDEntry) -> Self {
        if former.entry == latter.entry {
            let mut entry = former.with_context(&mut || DiffType::Unchanged {
                metadata_changed_to: None,
            });
            if former.metadata != latter.metadata {
                entry.context = DiffType::Unchanged {
                    metadata_changed_to: Some(latter.metadata.clone()),
                }
            }

            entry
        } else {
            match (&former.entry, &latter.entry) {
                // We already checked that the directory is not entirely unchanged, thus it can only
                // have its children changed (a changed directory by itself doesn't make sense, since
                // its just a container for its children)
                (fs::DirEntry::Directory(former_dir), fs::DirEntry::Directory(latter_dir)) => {
                    let mut entries = BTreeMap::new();

                    for (name, entry) in &former_dir.entries {
                        if let Some(latter_entry) = latter_dir.entries.get(name) {
                            entries.insert(name.clone(), Self::compute(entry, latter_entry));
                        } else {
                            entries.insert(
                                name.clone(),
                                entry.with_context(&mut || DiffType::Removed),
                            );
                        }
                    }

                    for (name, entry) in &latter_dir.entries {
                        if !former_dir.entries.contains_key(name) {
                            entries
                                .insert(name.clone(), entry.with_context(&mut || DiffType::Added));
                        }
                    }

                    MetaDEntry {
                        entry: fs::DirEntry::Directory(fs::Directory { entries }),
                        metadata: former.metadata.clone(),
                        context: DiffType::ChildrenChanged {
                            metadata_changed_to: (former.metadata != latter.metadata)
                                .then(|| latter.metadata.clone()),
                        },
                    }
                }
                _ => former.with_context(&mut || DiffType::Changed { to: latter.clone() }),
            }
        }
    }

    /// Converts the given entry into a difference tree where everything is unchanged.
    pub(crate) fn unchanged(root: &MetaDEntry) -> Self {
        root.with_context(&mut || DiffType::Unchanged {
            metadata_changed_to: None,
        })
    }

    /// Computes the size of the difference using the given size metric.
    fn size(&self, size_metric: SizeMetric) -> u64 {
        firestorm::profile_fn!(size_computation);
        self.walk()
            .map(|entry| (size_metric.calculate)(entry.entry))
            .sum()
    }

    /// Returns `true` if only the metadata of the top-level entry is changed.
    pub(crate) fn only_metadata_changed(&self) -> bool {
        match &self.context {
            DiffType::Unchanged {
                metadata_changed_to,
            }
            | DiffType::ChildrenChanged {
                metadata_changed_to,
            } => metadata_changed_to.is_some(),
            DiffType::Changed { .. } | DiffType::Added | DiffType::Removed => false,
        }
    }

    /// Formats the difference tree into the given formatter.
    fn recursive_tree_display(
        &self,
        f: &mut fmt::Formatter,
        prefix: &str,
        depth: u32,
        name: &OsStr,
        ctx: DisplayContext<impl Fn(FilterContext) -> bool>,
    ) -> fmt::Result {
        firestorm::profile_section!(recursive_tree_display);

        let detailed = !self.is_dir() && depth == 0 || ctx.summary_level == Some(0);

        write!(f, "{}{}", prefix, if detailed { "" } else { " " })?;
        if !detailed && matches!(self.entry, fs::DirEntry::Directory(_)) {
            write!(f, "🗁 ")?;
        };

        match &self.context {
            DiffType::Unchanged { .. } => write!(f, "{}", name.display().bright_black())?,
            DiffType::ChildrenChanged { .. } => write!(f, "{}", name.display())?,
            DiffType::Changed { .. } => write!(f, "{}", name.display().yellow())?,
            DiffType::Added => write!(f, "{}", name.display().green())?,
            DiffType::Removed => write!(f, "{}", name.display().red())?,
        };
        if detailed
            && (ctx.summary_level != Some(0) || !matches!(self.entry, fs::DirEntry::Directory(_)))
        {
            writeln!(f)?;
        }

        let new_metadata = match &self.context {
            DiffType::Unchanged {
                metadata_changed_to: Some(new_meta),
            }
            | DiffType::ChildrenChanged {
                metadata_changed_to: Some(new_meta),
            }
            | DiffType::Changed {
                to:
                    crate::fs::MetaDirEntry {
                        metadata: new_meta, ..
                    },
            } => (new_meta != &self.metadata).then_some(new_meta),
            DiffType::Added
            | DiffType::Removed
            | DiffType::Unchanged {
                metadata_changed_to: None,
            }
            | DiffType::ChildrenChanged {
                metadata_changed_to: None,
            } => None,
        };

        let display_meta = |f| display_metadata(f, &self.metadata, new_metadata, detailed);

        match &self.entry {
            fs::DirEntry::File(file) => {
                match &self.context {
                    DiffType::Unchanged { .. }
                    | DiffType::ChildrenChanged { .. }
                    | DiffType::Added
                    | DiffType::Removed => {
                        if self.metadata.size == 0 && !detailed {
                            write!(f, " [empty file]")?;
                        } else {
                            display_file(
                                f,
                                file,
                                None,
                                &self.context,
                                ctx.database,
                                ctx.show_hashes,
                                detailed,
                            )?;
                        }
                    }
                    DiffType::Changed { to } => {
                        if let fs::DirEntry::File(latter_file) = &to.entry {
                            display_file(
                                f,
                                file,
                                Some(latter_file),
                                &self.context,
                                ctx.database,
                                ctx.show_hashes,
                                detailed,
                            )?;
                        } else {
                            display_type_change(
                                f,
                                self.entry.entry_type(),
                                to.entry_type(),
                                detailed,
                            )?;
                        }
                    }
                }
                display_meta(f)?;
                if !detailed {
                    writeln!(f)?;
                }
            }
            fs::DirEntry::Symlink(symlink) => {
                match &self.context {
                    DiffType::Unchanged { .. }
                    | DiffType::ChildrenChanged { .. }
                    | DiffType::Added
                    | DiffType::Removed => {
                        write!(
                            f,
                            "{}{} to {}{}",
                            if detailed { "" } else { " (" },
                            "symlink".blue(),
                            symlink.link_path.display(),
                            if detailed { "" } else { ")" }
                        )?;
                    }
                    DiffType::Changed { to } => {
                        if let fs::DirEntry::Symlink(latter_symlink) = &to.entry {
                            write!(
                                f,
                                "{}{} to {} -> {}{}",
                                if detailed { "" } else { " (" },
                                "symlink".blue(),
                                symlink.link_path.display().red(),
                                latter_symlink.link_path.display().green(),
                                if detailed { "" } else { ")" }
                            )?;
                        } else {
                            display_type_change(
                                f,
                                self.entry.entry_type(),
                                to.entry_type(),
                                detailed,
                            )?;
                        }
                    }
                }
                if detailed {
                    writeln!(f)?;
                }
                display_meta(f)?;
                if !detailed {
                    writeln!(f)?;
                }
            }
            fs::DirEntry::Directory(directory) => {
                display_dir_summary(f, self, ctx.filter, ctx.database)?;
                if detailed {
                    writeln!(f)?;
                }
                display_meta(f)?;
                if !detailed {
                    writeln!(f)?;
                }

                if let Some(level) = ctx.summary_level && depth == level {
                    return Ok(());
                }

                let stripped_prefix = if let Some(prefix) = prefix.strip_suffix(" └─") {
                    format!("{}   ", prefix)
                } else if let Some(prefix) = prefix.strip_suffix(" ├─") {
                    format!("{} │ ", prefix)
                } else {
                    prefix.to_string()
                };

                let mut entries =
                    Vec::from_iter(directory.entries.iter().filter(|(name, entry)| {
                        (ctx.filter)(FilterContext {
                            name,
                            entry,
                            database: ctx.database,
                        }) || entry
                            .walk()
                            .any(|child| child.filter(ctx.database, ctx.filter).unwrap_or(false))
                    }));

                entries.sort_by_cached_key(|(_, entry)| entry.size(ctx.size_metric));

                let mut iter = entries.into_iter().peekable();

                while let Some((name, entry)) = iter.next() {
                    let last = iter.peek().is_none();

                    let new_prefix = if last {
                        format!("{} └─", stripped_prefix)
                    } else {
                        format!("{} ├─", stripped_prefix)
                    };

                    entry.recursive_tree_display(f, &new_prefix, depth + 1, name, ctx)?;
                }
            }
            fs::DirEntry::Other(other) => {
                display_meta(f)?;
                write!(f, " ({})", other.blue())?;
                writeln!(f)?;
            }
        }

        Ok(())
    }

    /// Displays the difference in a tree view.
    pub(crate) fn display_as_tree<'tree, F: Fn(FilterContext<'tree>) -> bool>(
        &'tree self,
        name: &'tree OsStr,
        filter: F,
        summary_level: Option<u32>,
        size_metric: SizeMetric,
        show_hashes: bool,
        database: Option<&'tree Database>,
    ) -> TreeDisplay<'tree, F> {
        TreeDisplay {
            entry: self,
            name,
            filter,
            summary_level,
            size_metric,
            show_hashes,
            database,
        }
    }
}

/// A helper struct implementing `Display` used for `DiffTree::display_as_tree` and
/// `DiffTree::display_summary`.
pub(crate) struct TreeDisplay<'tree, F> {
    /// The entry which is displayed.
    entry: &'tree DiffTree,
    /// The name of the entry.
    name: &'tree OsStr,
    /// The filter to apply when displaying files.
    filter: F,
    /// Whether to display a tree or a summary.
    summary_level: Option<u32>,
    /// The size metric to use when sorting the entries.
    size_metric: SizeMetric,
    /// Whether to show hashes.
    show_hashes: bool,
    /// The connection to the database.
    database: Option<&'tree Database>,
}

impl<'tree, F: Fn(FilterContext) -> bool> fmt::Display for TreeDisplay<'tree, F> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.entry.recursive_tree_display(
            f,
            "",
            0,
            self.name,
            DisplayContext {
                filter: &self.filter,
                summary_level: self.summary_level,
                size_metric: self.size_metric,
                show_hashes: self.show_hashes,
                database: self.database,
            },
        )
    }
}

/// The context used for displaying a tree.
struct DisplayContext<'tree, F: Fn(FilterContext<'tree>) -> bool> {
    /// The filter to apply to the tree items.
    filter: &'tree F,
    /// The level at which should be summarized.
    summary_level: Option<u32>,
    /// The size metric to sort entries by.
    size_metric: SizeMetric,
    /// Whether to show hashes.
    show_hashes: bool,
    /// The connection to the database.
    database: Option<&'tree Database>,
}

impl<'tree, F: Fn(FilterContext<'tree>) -> bool> Clone for DisplayContext<'tree, F> {
    fn clone(&self) -> Self {
        Self {
            filter: self.filter,
            summary_level: self.summary_level,
            size_metric: self.size_metric,
            show_hashes: self.show_hashes,
            database: self.database,
        }
    }
}

impl<'tree, F: Fn(FilterContext<'tree>) -> bool> Copy for DisplayContext<'tree, F> {}

/// Display a summary of the changes in the given directory.
fn display_dir_summary(
    f: &mut fmt::Formatter,
    dir: &DiffTree,
    filter: &impl Fn(FilterContext) -> bool,
    database: Option<&Database>,
) -> fmt::Result {
    firestorm::profile_fn!(display_dir_summary);

    let mut added = 0;
    let mut removed = 0;
    let mut meta_only_changed = 0;
    let mut changed = 0;
    let mut unchanged = 0;

    let total_size = dir.walk().map(|entry| entry.entry.metadata.size).sum();

    for entry in dir
        .walk()
        .filter(|entry| entry.filter(database, filter).unwrap_or(false))
    {
        match &entry.entry.context {
            DiffType::Unchanged {
                metadata_changed_to,
            }
            | DiffType::ChildrenChanged {
                metadata_changed_to,
            } => {
                if metadata_changed_to.is_some() {
                    meta_only_changed += 1;
                } else {
                    unchanged += 1;
                }
            }
            DiffType::Changed { .. } => changed += 1,
            DiffType::Added => added += 1,
            DiffType::Removed => removed += 1,
        }
    }

    write!(f, " (")?;
    write!(f, "{}B", size_format::SizeFormatterBinary::new(total_size))?;
    if removed != 0 {
        write!(f, " {}", removed.red())?;
    }
    if added != 0 {
        write!(f, " {}", added.green())?;
    }
    if changed != 0 {
        write!(f, " {}", changed.yellow())?;
    }
    if unchanged != 0 {
        write!(f, " {}", unchanged.bright_black())?;
    }
    if meta_only_changed != 0 {
        write!(f, " {}{}", meta_only_changed.bright_black(), "M".yellow())?;
    }
    write!(f, ")")?;

    if let DirEntry::Directory(dir) = &dir.entry
        && dir.entries.is_empty() {
        write!(f, " (empty dir)")?;
    } else if removed == 0 && added == 0 && changed == 0 && unchanged == 0 && meta_only_changed == 0 {
        write!(f, " (differences filtered out)")?;
    }

    Ok(())
}

/// Display a summary of the changes in the given directory.
fn display_type_change(
    f: &mut fmt::Formatter,
    former: DirEntryType,
    latter: DirEntryType,
    detailed: bool,
) -> fmt::Result {
    if detailed {
        writeln!(f, "type changed: {} -> {}", former.red(), latter.green())
    } else {
        write!(f, "{} -> {}", former.red(), latter.green())
    }
}

use sniff_interop as interop;

/// Computes if a change occurred.
fn compute_change<T: Eq + Clone>(old: &T, new: &T) -> Option<interop::Change<T>> {
    if old == new {
        None
    } else {
        Some(interop::Change {
            from: old.clone(),
            to: new.clone(),
        })
    }
}

/// Computes if a change occurred.
fn compute_maybe_change<T: Eq + Clone>(old: &T, new: &T) -> interop::MaybeChange<T> {
    if let Some(change) = compute_change(old, new) {
        interop::MaybeChange::Change(change)
    } else {
        interop::MaybeChange::Same(old.clone())
    }
}

/// Computes the difference between two given entries.
pub(crate) fn compute_entry_diff<
    Context: Eq + Serialize + serde::de::DeserializeOwned + Clone + Sized + Send,
>(
    old: &fs::DEntry<Context>,
    new: &fs::DEntry<Context>,
) -> Option<interop::EntryDiff> {
    if old == new {
        return None;
    }

    match (old, new) {
        (DirEntry::File(old_file), DirEntry::File(new_file)) => {
            Some(interop::EntryDiff::FileChanged {
                hash_change: compute_change(
                    &interop::Hash(old_file.sha2_256_hash.bytes),
                    &interop::Hash(new_file.sha2_256_hash.bytes),
                )?,
            })
        }
        (
            DirEntry::Symlink(fs::Symlink {
                link_path: old_path,
            }),
            DirEntry::Symlink(fs::Symlink {
                link_path: new_path,
            }),
        ) => Some(interop::EntryDiff::SymlinkChanged {
            path_change: compute_change(
                &old_path.to_string_lossy().into_owned(),
                &new_path.to_string_lossy().into_owned(),
            )?,
        }),
        (old, new) => {
            if let Some(type_change) =
                compute_change(&old.entry_type().to_string(), &new.entry_type().to_string())
            {
                Some(interop::EntryDiff::TypeChange(type_change))
            } else {
                Some(interop::EntryDiff::OtherChange)
            }
        }
    }
}

/// Computes the difference between two given entries.
pub(crate) fn compute_meta_entry_diff<
    Context: Eq + Serialize + serde::de::DeserializeOwned + Clone + Sized + Send,
>(
    old: Option<&fs::MetaDEntry<Context>>,
    new: Option<&fs::MetaDEntry<Context>>,
) -> Option<interop::MetaEntryDiff<interop::Timestamp>> {
    let (old, new) = match (old, new) {
        (None, None) => return None,
        (None, Some(new)) => {
            return Some(interop::MetaEntryDiff::Added(
                metadata::compute_meta_info_from_single(&new.metadata),
            ))
        }
        (Some(old), None) => {
            return Some(interop::MetaEntryDiff::Deleted(
                metadata::compute_meta_info_from_single(&old.metadata),
            ))
        }
        (Some(old), Some(new)) => (old, new),
    };

    let entry_diff = compute_entry_diff(&old.entry, &new.entry);
    let meta_info = metadata::compute_meta_info_change(&old.metadata, &new.metadata);

    entry_diff.map(|entry_diff| interop::MetaEntryDiff::EntryChange(entry_diff, meta_info))
}

/// Computes a set of changes for the given diff tree.
pub(crate) fn compute_changeset(
    base_path: impl AsRef<std::path::Path>,
    diff: &MetaDEntry<DiffType>,
    filter: impl Fn(crate::diff::filters::FilterContext) -> bool,
    earliest_timestamp: crate::timestamp::Timestamp,
) -> interop::Changeset<interop::Timestamp> {
    let base_path = base_path.as_ref();
    let mut changes = std::collections::BTreeMap::new();

    for entry in diff
        .walk()
        .filter(|entry| entry.filter(None, &filter).unwrap_or(false))
    {
        let mut path = std::path::PathBuf::from(base_path);
        path.extend(
            entry
                .path_components()
                .skip_while(|component| *component == std::ffi::OsStr::new("/")),
        );

        let diff = match &entry.entry.context {
            DiffType::Added => compute_meta_entry_diff(None, Some(entry.entry)),
            DiffType::Removed => compute_meta_entry_diff(Some(entry.entry), None),
            DiffType::Changed { to } => {
                compute_meta_entry_diff(Some(&entry.entry.with_context(&mut || ())), Some(to))
            }
            DiffType::Unchanged {
                metadata_changed_to: Some(new_meta),
            }
            | DiffType::ChildrenChanged {
                metadata_changed_to: Some(new_meta),
            } => Some(interop::MetaEntryDiff::MetaOnlyChange(
                metadata::compute_meta_info_change(&entry.entry.metadata, new_meta),
            )),
            DiffType::Unchanged {
                metadata_changed_to: None,
            }
            | DiffType::ChildrenChanged {
                metadata_changed_to: None,
            } => None,
        };
        if let Some(diff) = diff {
            changes.insert(path.to_string_lossy().into_owned(), diff);
        }
    }

    interop::Changeset {
        earliest_timestamp: earliest_timestamp.into(),
        changes,
    }
}
