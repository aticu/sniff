//! Implements display filters to help simplify visualization of diffs.

use image::Rgb;

use crate::fs;

use super::{filters::FilterContext, visualize::ColorFilter};

/// The type of a dynamic display filter.
pub(crate) type DynDisplayFilter<'a> = Box<dyn Fn(FilterContext) -> ColorFilter + 'a>;

/// Highlight files matching the given filter with the given color.
pub(crate) fn highlight<F: Fn(FilterContext) -> bool>(
    color: Rgb<u8>,
    filter: F,
) -> impl Fn(FilterContext) -> ColorFilter {
    move |ctx| {
        if filter(ctx) {
            ColorFilter::Custom(color)
        } else {
            ColorFilter::Normal
        }
    }
}

/// Ignore files matching the given filter.
pub(crate) fn ignore<F: Fn(FilterContext) -> bool>(
    filter: F,
) -> impl Fn(FilterContext) -> ColorFilter {
    move |ctx| {
        if filter(ctx) {
            ColorFilter::HardIgnore
        } else {
            ColorFilter::Normal
        }
    }
}

/// Ignore files where only the metadata changed.
pub(crate) fn ignore_changed_metadata(ctx: FilterContext) -> ColorFilter {
    if ctx.entry.only_metadata_changed() {
        ColorFilter::Ignore
    } else {
        ColorFilter::Normal
    }
}

/// Highlight known files.
pub(crate) fn highlight_known(color: Rgb<u8>) -> impl Fn(FilterContext) -> ColorFilter {
    move |ctx| {
        if let Some(db) = ctx.database &&
            let fs::DirEntry::File(file) = &ctx.entry.entry &&
            db.file_is_known(file).unwrap_or(false) {
            ColorFilter::Custom(color)
        } else {
            ColorFilter::Normal
        }
    }
}

/// Applies all of the given display filters.
///
/// Later filters override earlier filters, but returning `ColorFilter::Normal` does not override
/// anything.
pub(crate) fn all_of(filters: Vec<DynDisplayFilter>) -> impl Fn(FilterContext) -> ColorFilter + '_ {
    move |ctx| {
        let mut result = ColorFilter::Normal;

        for filter in &filters {
            match filter(ctx) {
                ColorFilter::Normal => (),
                res @ (ColorFilter::Custom(_) | ColorFilter::Ignore) => result = res,
                ColorFilter::HardIgnore => return ColorFilter::HardIgnore,
            }
        }

        result
    }
}
