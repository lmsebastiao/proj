//! Repository avatar first (where the Git host exposes one), then conventional
//! image files in the checkout. Never block the switcher's render on disk or network.

use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use gpui::{App, Asset, ImageSource, RenderImage, Window, div, img, prelude::*, px};

use crate::{git, platform};

use super::{app_icon::app_icon, theme::icons};

const MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_SIDE: u32 = 2048;
const SIZE: u32 = 96;

enum RemoteImage {}

impl Asset for RemoteImage {
    type Source = PathBuf;
    type Output = Option<Arc<RenderImage>>;

    #[allow(clippy::manual_async_fn)]
    fn load(root: PathBuf, _: &mut App) -> impl Future<Output = Self::Output> + Send + 'static {
        async move {
            let web = git::git_web_url(&root)?;
            decode(&repository_avatar(&web)?)
        }
    }
}

enum LocalImage {}

impl Asset for LocalImage {
    type Source = PathBuf;
    type Output = Option<Arc<RenderImage>>;

    #[allow(clippy::manual_async_fn)]
    fn load(root: PathBuf, _: &mut App) -> impl Future<Output = Self::Output> + Send + 'static {
        async move { local_image(&root) }
    }
}

fn local_image(root: &Path) -> Option<Arc<RenderImage>> {
    for path in local_candidates(root) {
        if std::fs::metadata(&path).is_ok_and(|meta| meta.len() <= MAX_BYTES)
            && let Ok(bytes) = std::fs::read(path)
            && let Some(image) = decode(&bytes)
        {
            return Some(image);
        }
    }
    None
}

// Fixed paths, not a recursive search: don't accidentally show a screenshot or badge.
fn local_candidates(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for dir in [".github", ".gitlab", "", "assets", "public"] {
        for name in ["logo", "icon"] {
            for ext in ["png", "webp", "jpg", "jpeg", "svg"] {
                paths.push(root.join(dir).join(format!("{name}.{ext}")));
            }
        }
    }
    for path in ["public/favicon.png", "public/favicon.ico", "favicon.ico"] {
        paths.push(root.join(path));
    }
    paths
}

// Only public hosted APIs with a repository-specific avatar field. GitHub's
// `owner.avatar_url` is the owner's image, not the repository's logo.
fn avatar_api(web: &str) -> Option<String> {
    let (host, path) = web.strip_prefix("https://")?.split_once('/')?;
    let path = path.trim_end_matches('/');
    match host {
        "gitlab.com" => Some(format!(
            "https://gitlab.com/api/v4/projects/{}",
            percent_encoding::utf8_percent_encode(path, percent_encoding::NON_ALPHANUMERIC)
        )),
        "codeberg.org" | "gitea.com" if path.split('/').count() == 2 => {
            Some(format!("https://{host}/api/v1/repos/{path}"))
        }
        _ => None,
    }
}

fn repository_avatar(web: &str) -> Option<Vec<u8>> {
    let url = avatar_api(web)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(1200)))
        .user_agent("proj (project icons)")
        .build()
        .into();
    let mut response = agent.get(&url).call().ok()?;
    let json: serde_json::Value = response.body_mut().read_json().ok()?;
    let avatar = json.get("avatar_url")?.as_str()?;
    // Never follow an avatar_url to a different host or an insecure URL.
    let (host, _) = avatar.strip_prefix("https://")?.split_once('/')?;
    let (api_host, _) = url.strip_prefix("https://")?.split_once('/')?;
    if host != api_host {
        return None;
    }
    let mut response = agent.get(avatar).call().ok()?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_vec()
        .ok()
}

fn decode(bytes: &[u8]) -> Option<Arc<RenderImage>> {
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    let Ok(format) = image::guess_format(bytes) else {
        return decode_svg(bytes);
    };
    let reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    let (width, height) = reader.into_dimensions().ok()?;
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return None;
    }
    let bitmap = image::load_from_memory(bytes)
        .ok()?
        .thumbnail(SIZE, SIZE)
        .to_rgba8();
    let (width, height) = bitmap.dimensions();
    let mut pixels = bitmap.into_raw();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2); // gpui expects BGRA, not RGBA.
    }
    let bitmap = image::RgbaImage::from_raw(width, height, pixels)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(bitmap)])))
}

fn decode_svg(bytes: &[u8]) -> Option<Arc<RenderImage>> {
    let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).ok()?;
    let shape = tree.size();
    let scale = SIZE as f32 / shape.width().max(shape.height());
    let width = (shape.width() * scale).round() as u32;
    let height = (shape.height() * scale).round() as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let mut pixels = pixmap.take();
    for pixel in pixels.as_chunks_mut::<4>().0 {
        // tiny-skia gives premultiplied RGBA; GPUI expects straight BGRA.
        let alpha = f32::from(pixel[3]) / 255.;
        if alpha > 0. {
            for channel in &mut pixel[..3] {
                *channel = (f32::from(*channel) / alpha).round().min(255.) as u8;
            }
        }
        pixel.swap(0, 2);
    }
    let bitmap = image::RgbaImage::from_raw(width, height, pixels)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(bitmap)])))
}

/// The editor icon stays visible until an image is ready. Only successfully
/// decoded images are painted; workspaces and unmatched windows keep the editor icon.
pub(super) fn project_image(
    root: Option<PathBuf>,
    exe: PathBuf,
    size: f32,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement + use<> {
    let image = root.and_then(|root| {
        let remote = window.use_asset::<RemoteImage>(&root, cx).flatten();
        let local = window.use_asset::<LocalImage>(&root, cx).flatten();
        remote.or(local)
    });
    div().size(px(size)).child(match image {
        Some(image) => img(ImageSource::Render(image))
            .size_full()
            .object_fit(gpui::ObjectFit::Contain)
            .into_any_element(),
        None if platform::HAS_APP_ICONS => app_icon(Some(exe), size).into_any_element(),
        None => div().child(icons::PROGRAM).into_any_element(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_files_before_other_local_images() {
        let paths = local_candidates(Path::new("/repo"));
        assert_eq!(paths[0], Path::new("/repo/.github/logo.png"));
        assert!(
            paths.iter().position(|p| p.ends_with(".gitlab/logo.png"))
                < paths.iter().position(|p| p.ends_with("assets/logo.png"))
        );
        assert_eq!(paths.last().unwrap(), Path::new("/repo/favicon.ico"));
    }

    #[test]
    fn avatar_only_for_supported_repository_hosts() {
        assert_eq!(
            avatar_api("https://gitlab.com/team/app"),
            Some("https://gitlab.com/api/v4/projects/team%2Fapp".into())
        );
        assert_eq!(
            avatar_api("https://codeberg.org/team/app"),
            Some("https://codeberg.org/api/v1/repos/team/app".into())
        );
        assert_eq!(avatar_api("https://github.com/team/app"), None);
        assert_eq!(avatar_api("http://gitlab.com/team/app"), None);
    }

    #[test]
    fn skips_invalid_files_and_uses_the_next_local_image() {
        let root = std::env::temp_dir().join(format!(
            "proj-project-image-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(root.join("logo.png"), b"not an image").unwrap();
        let source = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(source)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        std::fs::write(root.join("assets/icon.png"), bytes.into_inner()).unwrap();
        assert!(local_image(&root).is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finds_this_repositories_svg_icon() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert!(local_image(&root).is_some());
    }

    #[test]
    fn images_are_decoded_and_invalid_images_are_skipped() {
        let source = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 255]));
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(source)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        assert!(decode(&bytes.into_inner()).is_some());
        assert!(decode(include_bytes!("../../../../assets/icon.svg")).is_some());
        assert!(decode(b"not an image").is_none());
    }
}
