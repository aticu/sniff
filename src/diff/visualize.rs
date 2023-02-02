//! Implements visualization of differences.

use std::{ffi::OsStr, path::Path};

use anyhow::Context as _;
use image::{Rgb, RgbImage};

use crate::{
    database::Database,
    fs::{DirEntry, OsStrExt as _},
};

use super::filters::FilterContext;

/// The text color.
const TEXT: Rgb<u8> = Rgb([0, 0, 0]);

/// The border color.
const BORDER: Rgb<u8> = Rgb([0, 0, 0]);

/// The background color.
///
/// This should really not be seen unless there are bugs.
const BACKGROUND: Rgb<u8> = Rgb([255, 0, 255]);

/// The color of unchanged data.
const UNCHANGED: Rgb<u8> = Rgb([127, 127, 127]);

/// The color of changed data.
const CHANGED: Rgb<u8> = Rgb([255, 255, 0]);

/// The color of removed data.
const REMOVED: Rgb<u8> = Rgb([255, 0, 0]);

/// The color of added data.
const ADDED: Rgb<u8> = Rgb([0, 0, 255]);

/// A rectangle in an image.
#[derive(Debug, Default, Clone, Copy)]
struct Rect {
    /// The `x` position of the rectangle.
    x: u64,
    /// The `y` position of the rectangle.
    y: u64,
    /// The width of the rectangle.
    w: u64,
    /// The height of the rectangle.
    h: u64,
}

impl From<Rect> for streemap::Rect<u64> {
    fn from(rect: Rect) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: rect.h,
        }
    }
}

impl From<streemap::Rect<u64>> for Rect {
    fn from(rect: streemap::Rect<u64>) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: rect.h,
        }
    }
}

impl Rect {
    /// Draws the rectangle with the given color into the given image.
    fn draw(&self, image: &mut RgbImage, color: Rgb<u8>) -> anyhow::Result<()> {
        firestorm::profile_fn!(draw_rect);

        assert!(
            self.x + self.w <= image.width() as u64 && self.y + self.h <= image.height() as u64
        );

        for x in 0..self.w {
            for y in 0..self.h {
                image.put_pixel((self.x + x) as u32, (self.y + y) as u32, color)
            }
        }

        Ok(())
    }

    /// Draws the given text of the given color into the given image, if possible.
    fn draw_text(&self, image: &mut RgbImage, text: &str, color: Rgb<u8>) {
        firestorm::profile_fn!(draw_text);

        if self.h < 8 {
            return;
        }

        for (nchr, c) in text.as_bytes().iter().enumerate() {
            if nchr as u32 * 8 + 7 >= self.w as u32 {
                break;
            }

            if !c.is_ascii() {
                continue;
            }

            for (nrow, row) in font8x8::legacy::BASIC_LEGACY[*c as usize]
                .iter()
                .enumerate()
            {
                for bit in 0..8 {
                    match *row & 1 << bit {
                        0 => (),
                        _ => image.put_pixel(
                            (self.x + nchr as u64 * 8 + bit) as u32,
                            (self.y + nrow as u64) as u32,
                            color,
                        ),
                    }
                }
            }
        }
    }

    /// Returns the inner rectangle with a border of width `width`.
    fn with_border(&self, width: u64) -> Option<Self> {
        if width * 2 > self.w || width * 2 > self.h {
            None
        } else {
            Some(Self {
                x: self.x + width,
                y: self.y + width,
                w: self.w - 2 * width,
                h: self.h - 2 * width,
            })
        }
    }
}

/// A filter to apply for colors of items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorFilter {
    /// Display this item normally.
    Normal,
    /// Use a custom color color for this item.
    Custom(Rgb<u8>),
    /// Use the same color as for unchanged entries for this item.
    Ignore,
    /// Same as ignore, but it cannot be overwritten by later filters.
    HardIgnore,
}

/// The context required for visualization
pub(crate) struct VisualizationContext<'a, C: Fn(FilterContext<'a>) -> ColorFilter> {
    /// The color filter to apply on entries being visualized.
    pub(crate) color_filter: &'a C,
    /// The size metric to use for the size of entries.
    pub(crate) size_metric: super::SizeMetric,
    /// The database connection.
    pub(crate) database: Option<&'a Database>,
}

impl<'a, C: Fn(FilterContext<'a>) -> ColorFilter> Clone for VisualizationContext<'a, C> {
    fn clone(&self) -> Self {
        Self {
            color_filter: self.color_filter,
            size_metric: self.size_metric,
            database: self.database,
        }
    }
}

impl<'a, C: Fn(FilterContext<'a>) -> ColorFilter> Copy for VisualizationContext<'a, C> {}

/// An item that will be or is laid out in a tree map.
#[derive(Debug)]
struct TreeItem<'tree> {
    /// The subtree represented by this tree item.
    subtree: &'tree super::DiffTree,
    /// The name of this tree item.
    name: &'tree OsStr,
    /// The bounding box of the item.
    bounds: Rect,
    /// The precomputed size of the item.
    size: u64,
}

/// Generates an image from a diff, storing the result at `path`.
pub(crate) fn generate_image(
    path: impl AsRef<Path>,
    name: &OsStr,
    diff: &super::DiffTree,
    ctx: VisualizationContext<impl Fn(FilterContext) -> ColorFilter>,
) -> anyhow::Result<()> {
    let path = path.as_ref();

    let mut img = RgbImage::new(1600, 1600);

    let bounds = Rect {
        x: 0,
        y: 0,
        w: img.width() as u64,
        h: img.height() as u64,
    };

    bounds
        .draw(&mut img, BACKGROUND)
        .context("Could not draw image background")?;

    draw_into(&mut img, bounds, (name, diff), true, ctx).context("Failed to render image")?;

    img.save(path)
        .with_context(|| format!("Could not save image to {}", path.display()))?;

    Ok(())
}

/// Draws the given diff into the given rectangle on the given image.
fn draw_into(
    img: &mut RgbImage,
    rect: Rect,
    (name, diff): (&OsStr, &super::DiffTree),
    is_outer: bool,
    ctx: VisualizationContext<impl Fn(FilterContext) -> ColorFilter>,
) -> anyhow::Result<()> {
    // there's nothing to draw, so quit early
    if rect.w == 0 || rect.h == 0 {
        return Ok(());
    }

    let mut items = if let DirEntry::Directory(dir) = &diff.entry {
        firestorm::profile_section!(build_items);

        Vec::from_iter(
            dir.entries
                .iter()
                .map(|(name, entry)| TreeItem {
                    subtree: entry,
                    name,
                    bounds: Rect::default(),
                    size: entry.size(ctx.size_metric),
                })
                // Items with a size of 0 do not contribute to the graph, so they can be removed
                .filter(|entry| entry.size != 0),
        )
    } else {
        let color = match (ctx.color_filter)(FilterContext {
            name,
            entry: diff,
            database: ctx.database,
        }) {
            ColorFilter::Normal => color_from_diff_type(&diff.context),
            ColorFilter::Custom(color) => color,
            ColorFilter::Ignore | ColorFilter::HardIgnore => UNCHANGED,
        };
        rect.draw(img, color)
            .with_context(|| format!("Could not draw rectangle for `{}`", name.display()))?;
        if is_outer {
            if let Some(name) = name.to_str() {
                if let Some(inner_bounds) = rect.with_border(2) {
                    inner_bounds.draw_text(img, name, TEXT);
                }
            }
        }

        return Ok(());
    };

    {
        firestorm::profile_section!(sort_items);

        items.sort_by(|i1, i2| {
            match i1.size.cmp(&i2.size) {
                std::cmp::Ordering::Equal => i1.name.cmp(i2.name),
                o => o,
            }
            .reverse()
        });
    }

    if !items.is_empty() {
        firestorm::profile_section!(layout_items);

        streemap::binary(
            rect.into(),
            &mut items,
            |i| i.size,
            |i, r| i.bounds = r.into(),
        );
    }

    for item in &items {
        let bounds = item.bounds;

        draw_into(img, bounds, (item.name, item.subtree), false, ctx)
            .with_context(|| format!("Failed recursive drawing of `{}`", item.name.display()))?;

        if is_outer && bounds.h > 0 && bounds.w > 0 {
            for i in 0..bounds.w {
                img.put_pixel((bounds.x + i) as u32, bounds.y as u32, BORDER);
                img.put_pixel(
                    (bounds.x + i) as u32,
                    (bounds.y + bounds.h - 1) as u32,
                    BORDER,
                );
            }

            for i in 0..bounds.h {
                img.put_pixel(bounds.x as u32, (bounds.y + i) as u32, BORDER);
                img.put_pixel(
                    (bounds.x + bounds.w - 1) as u32,
                    (bounds.y + i) as u32,
                    BORDER,
                );
            }
        }

        if is_outer {
            if let Some(name) = item.name.to_str() {
                if let Some(inner_bounds) = bounds.with_border(2) {
                    inner_bounds.draw_text(img, name, TEXT);
                }
            }
        }
    }

    if items.is_empty() {
        rect.draw(img, color_from_diff_type(&diff.context))
            .with_context(|| format!("Could not draw rectangle for empty `{}`", name.display()))?;
        if is_outer {
            if let Some(name) = name.to_str() {
                if let Some(inner_bounds) = rect.with_border(2) {
                    inner_bounds.draw_text(img, name, TEXT);
                }
            }
        }
    }

    Ok(())
}

/// Returns the appropriate color for the given type of difference.
fn color_from_diff_type(diff_type: &super::DiffType) -> Rgb<u8> {
    match diff_type {
        super::DiffType::Unchanged {
            metadata_changed_to: None,
        }
        | super::DiffType::ChildrenChanged {
            metadata_changed_to: None,
        } => UNCHANGED,
        super::DiffType::Unchanged {
            metadata_changed_to: Some(_),
        }
        | super::DiffType::ChildrenChanged {
            metadata_changed_to: Some(_),
        }
        | super::DiffType::Changed { .. } => CHANGED,
        super::DiffType::Added => ADDED,
        super::DiffType::Removed => REMOVED,
    }
}
