//! Renders proj's icon SVGs into the files the build embeds. See Cargo.toml.

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{Options, Tree};

/// icon-small.svg is drawn for these sizes, icon.svg for the rest.
const SMALL: [u32; 3] = [16, 20, 24];
const LARGE: [u32; 5] = [32, 40, 48, 64, 256];

fn load(path: &str) -> Tree {
    let svg = std::fs::read_to_string(path).unwrap_or_else(|err| panic!("{path}: {err}"));
    Tree::from_str(&svg, &Options::default()).unwrap_or_else(|err| panic!("{path}: {err}"))
}

fn render(tree: &Tree, size: u32) -> Pixmap {
    let mut pixmap = Pixmap::new(size, size).expect("non-zero size");
    let scale = size as f32 / tree.size().width();
    resvg::render(
        tree,
        Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
}

/// Straight (not premultiplied) RGBA, top row first.
fn rgba(pixmap: &Pixmap) -> Vec<u8> {
    pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect()
}

/// An ICO image entry: PNG for 256 px, an uncompressed 32-bit bitmap otherwise
/// (which every reader, NSIS included, accepts).
fn ico_image(pixmap: &Pixmap) -> Vec<u8> {
    let size = pixmap.width();
    if size >= 256 {
        return pixmap.encode_png().expect("PNG encoding");
    }
    let mask_row = size.div_ceil(32) * 4;
    let mut out = Vec::new();
    // BITMAPINFOHEADER; the height counts the colour rows and the AND mask rows.
    for value in [40, size, size * 2] {
        out.extend(value.to_le_bytes());
    }
    out.extend(1u16.to_le_bytes());
    out.extend(32u16.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend((size * size * 4 + mask_row * size).to_le_bytes());
    out.extend([0; 16]);
    // BGRA, bottom row first.
    let pixels = rgba(pixmap);
    for row in pixels.chunks_exact(size as usize * 4).rev() {
        for p in row.chunks_exact(4) {
            out.extend([p[2], p[1], p[0], p[3]]);
        }
    }
    // The alpha channel does the masking, so the AND mask is all zero.
    out.resize(out.len() + (mask_row * size) as usize, 0);
    out
}

fn ico(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let count = u16::try_from(images.len()).expect("few images");
    let mut out = Vec::new();
    for value in [0, 1, count] {
        out.extend(value.to_le_bytes());
    }
    let mut offset = 6 + 16 * u32::from(count);
    for (size, data) in images {
        // 0 means 256.
        let side = u8::try_from(*size).unwrap_or(0);
        out.extend([side, side, 0, 0]);
        out.extend(1u16.to_le_bytes());
        out.extend(32u16.to_le_bytes());
        let len = u32::try_from(data.len()).expect("image under 4 GB");
        out.extend(len.to_le_bytes());
        out.extend(offset.to_le_bytes());
        offset += len;
    }
    for (_, data) in images {
        out.extend(data);
    }
    out
}

fn main() {
    let small = load("assets/icon-small.svg");
    let large = load("assets/icon.svg");
    let images: Vec<(u32, Vec<u8>)> = SMALL
        .iter()
        .map(|&size| (size, &small))
        .chain(LARGE.iter().map(|&size| (size, &large)))
        .map(|(size, tree)| (size, ico_image(&render(tree, size))))
        .collect();
    std::fs::write("assets/proj.ico", ico(&images)).expect("writing assets/proj.ico");
    std::fs::write("assets/icon-32.rgba", rgba(&render(&large, 32)))
        .expect("writing assets/icon-32.rgba");
    println!("wrote assets/proj.ico and assets/icon-32.rgba");
}
