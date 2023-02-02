//! Implements filters to help simplify handling of diffs.

use std::ffi::OsStr;

use crate::{
    database::Database,
    fs::{self, Metadata, OsStrExt as _},
    timestamp::Timestamp,
};

use super::{DiffTree, DiffType};

/// The context for applying a filter.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FilterContext<'a> {
    /// The name of the filtered entry.
    pub(crate) name: &'a OsStr,
    /// The filtered entry.
    pub(crate) entry: &'a DiffTree,
    /// The database connection.
    pub(crate) database: Option<&'a Database>,
}

/// The type of a dynamic filter.
pub(crate) type DynFilter<'a> = Box<dyn Fn(FilterContext) -> bool + 'a>;

/// Allows only entries with at least one timestamp before `before` and after `after`.
///
/// Optionally only the timestamps relevant for file content changes are compared.
pub(crate) fn timestamps(
    before: Option<Timestamp>,
    after: Option<Timestamp>,
    only_changes: bool,
) -> impl Fn(FilterContext) -> bool {
    assert!(before.is_some() || after.is_some());

    let ts_matches = move |ts| {
        if let Some(ts) = ts {
            match (before, after) {
                (Some(b), Some(a)) => a <= ts && ts <= b,
                (None, Some(a)) => a <= ts,
                (Some(b), None) => ts <= b,
                (None, None) => unreachable!(),
            }
        } else {
            false
        }
    };

    // I think this is more readable than the "minimal" version clippy suggests
    #[allow(clippy::nonminimal_bool)]
    let meta_matches = move |metadata: &Metadata| {
        (!only_changes && ts_matches(metadata.accessed))
            || (!only_changes && ts_matches(metadata.mft_modified))
            || ts_matches(metadata.modified)
            || ts_matches(metadata.created)
    };

    move |ctx| {
        meta_matches(&ctx.entry.metadata)
            || match &ctx.entry.context {
                DiffType::Unchanged {
                    metadata_changed_to: Some(metadata),
                }
                | DiffType::ChildrenChanged {
                    metadata_changed_to: Some(metadata),
                } => meta_matches(metadata),
                _ => false,
            }
    }
}

/// Allows only entries that are changed, added or removed and their parents.
pub(crate) fn changes_only(include_metadata: bool) -> impl Fn(FilterContext) -> bool {
    move |ctx| match &ctx.entry.context {
        DiffType::Unchanged {
            metadata_changed_to,
        }
        | DiffType::ChildrenChanged {
            metadata_changed_to,
        } => include_metadata && metadata_changed_to.is_some(),
        DiffType::Changed { .. } | DiffType::Added | DiffType::Removed => true,
    }
}

/// Allows only entries of the specified extensions.
pub(crate) fn extensions_only<'a, S: AsRef<str> + 'a, E: Clone + IntoIterator<Item = S> + 'a>(
    extensions: E,
) -> impl Fn(FilterContext) -> bool + 'a {
    move |ctx| {
        extensions
            .clone()
            .into_iter()
            .any(|ext| ctx.name.has_extension(ext))
    }
}

/// Allows only entries that don't have the specified extensions.
pub(crate) fn extensions_none_of<'a, S: AsRef<str> + 'a, E: Clone + IntoIterator<Item = S> + 'a>(
    extensions: E,
) -> impl Fn(FilterContext) -> bool + 'a {
    move |ctx| {
        extensions
            .clone()
            .into_iter()
            .all(|ext| !ctx.name.has_extension(ext))
    }
}

/// Allows only entries that don't have the specified extensions.
pub(crate) fn unknown_only(ctx: FilterContext) -> bool {
    // We only filter known files if nothing is changed
    if !matches!(ctx.entry.context, crate::diff::DiffType::Unchanged { .. }) {
        return true;
    }

    let db = if let Some(db) = ctx.database {
        db
    } else {
        // Consider the entry unknown if there is no database to check
        return true;
    };

    let file = if let fs::DirEntry::File(file) = &ctx.entry.entry {
        file
    } else {
        // Consider the entry unknown if it's not a file
        return true;
    };

    match db.file_is_known(file) {
        // The file is known, ignore it
        Ok(true) => false,
        // The file is unknown, so let's include it
        Ok(false) => true,
        // If the database couldn't find a result, let's just include the file to be safe
        Err(_) => true,
    }
}

/// Allows only entries that match all the filters.
pub(crate) fn all_of(filters: Vec<DynFilter>) -> impl Fn(FilterContext) -> bool + '_ {
    move |ctx| {
        firestorm::profile_fn!(filters_all_of);

        filters.iter().all(|filter| filter(ctx))
    }
}
