//! Programs' own icons, on the editor and window rows.

use std::{path::PathBuf, sync::Arc};

use gpui::{App, Asset, ImageCacheError, ImageSource, RenderImage, div, img, prelude::*, px};

use crate::platform;

/// On screen, in pixels.
const SIZE: f32 = 24.;
/// Extracted at twice that, for high-DPI displays.
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

/// The program's icon, or empty space (while it loads, or if there's none) so
/// the titles still line up.
pub(super) fn app_icon(program: Option<PathBuf>) -> impl IntoElement {
    div()
        .flex_none()
        .size(px(SIZE))
        .children(program.map(|path| {
            img(ImageSource::Custom(Arc::new(move |window, cx| {
                window
                    .use_asset::<AppIcon>(&path, cx)
                    .map(|icon| icon.ok_or_else(|| ImageCacheError::Asset("no icon".into())))
            })))
            .size_full()
        }))
}
