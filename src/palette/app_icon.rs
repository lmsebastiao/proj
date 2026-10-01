//! Programs' own icons, on the editor and window rows, and by the name of a
//! project's own editor.

use std::{path::PathBuf, sync::Arc};

use gpui::{App, Asset, ImageCacheError, ImageSource, RenderImage, div, img, prelude::*, px};

use crate::platform;

/// On screen, in pixels: on the editor and window rows, and before a
/// project's own editor's name.
pub(super) const ROW_SIZE: f32 = 24.;
pub(super) const SMALL_SIZE: f32 = 14.;
/// Extracted at twice the row size, for high-DPI displays.
const EXTRACT_SIZE: u32 = 48;

/// Loads a program's icon off the main thread; gpui keeps it once loaded.
enum AppIcon {}

impl Asset for AppIcon {
    type Source = PathBuf;
    type Output = Option<Arc<RenderImage>>;

    // Not an `async fn`: that would hold on to `App`, which can't leave its thread.
    #[allow(clippy::manual_async_fn)]
    fn load(path: PathBuf, _: &mut App) -> impl Future<Output = Self::Output> + Send + 'static {
        async move {
            let (width, height, bgra) = platform::app_icon(&path, EXTRACT_SIZE)?;
            // Named RGBA, but gpui wants BGRA, which is what Windows gives.
            let buffer = image::RgbaImage::from_raw(width, height, bgra)?;
            Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
        }
    }
}

/// The program's icon, `size` pixels square, or empty space (while it loads,
/// or if there's none) so the titles still line up.
pub(super) fn app_icon(program: Option<PathBuf>, size: f32) -> impl IntoElement {
    div()
        .flex_none()
        .size(px(size))
        .children(program.map(|path| {
            img(ImageSource::Custom(Arc::new(move |window, cx| {
                window
                    .use_asset::<AppIcon>(&path, cx)
                    .map(|icon| icon.ok_or_else(|| ImageCacheError::Asset("no icon".into())))
            })))
            .size_full()
        }))
}
