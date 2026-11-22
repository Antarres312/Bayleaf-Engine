//! The Bayleaf logo, decoded from `assets/brand/bayleaf-logo.png`.
//!
//! The source is a green leaf on a transparent background. It is turned into a single-colour
//! mark with an alpha channel, so it sits on any theme and can take the theme's brand colour.

use egui::Color32;

const LOGO_PNG: &[u8] = include_bytes!("../../../assets/brand/bayleaf-logo.png");

/// Darkest channel value of the leaf in a source without alpha; white maps to fully transparent.
const LEAF_MIN_CHANNEL: f32 = 127.0;

/// An RGBA8 image, rows top to bottom.
pub struct LogoImage {
    pub size: u32,
    pub rgba: Vec<u8>,
}

/// The logo as a `size` × `size` mark in `color`. `None` if the embedded file cannot be decoded.
pub fn logo(size: u32, color: Color32) -> Option<LogoImage> {
    let decoder = png::Decoder::new(std::io::Cursor::new(LOGO_PNG));
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    let channels = info.color_type.samples();
    let (width, height) = (info.width as usize, info.height as usize);
    let size = size.clamp(1, 1024) as usize;

    // Coverage of each source pixel: its alpha, or, in a source without alpha, 0 on white and 1
    // on the leaf.
    let coverage = |x: usize, y: usize| -> f32 {
        let start = (y * width + x) * channels;
        if channels == 4 || channels == 2 {
            return buffer.get(start + channels - 1).map_or(0.0, |a| f32::from(*a) / 255.0);
        }
        let pixel = buffer.get(start..start + channels.min(3)).unwrap_or(&[255, 255, 255]);
        let darkest = f32::from(pixel.iter().copied().min().unwrap_or(255));
        ((255.0 - darkest) / (255.0 - LEAF_MIN_CHANNEL)).clamp(0.0, 1.0)
    };

    // Box filter: average the source pixels that fall into each target pixel.
    let mut rgba = Vec::with_capacity(size * size * 4);
    for ty in 0..size {
        let (y0, y1) = (ty * height / size, ((ty + 1) * height / size).max(ty * height / size + 1));
        for tx in 0..size {
            let (x0, x1) = (tx * width / size, ((tx + 1) * width / size).max(tx * width / size + 1));
            let mut sum = 0.0;
            for y in y0..y1.min(height) {
                for x in x0..x1.min(width) {
                    sum += coverage(x, y);
                }
            }
            let count = ((y1.min(height) - y0) * (x1.min(width) - x0)).max(1) as f32;
            let alpha = (sum / count * 255.0).round() as u8;
            rgba.extend_from_slice(&[color.r(), color.g(), color.b(), alpha]);
        }
    }
    Some(LogoImage { size: size as u32, rgba })
}

/// Logo for the window icon and task bar.
pub fn window_icon() -> Option<LogoImage> {
    logo(256, crate::theme::DARK.brand)
}

#[cfg(test)]
mod tests {
    use super::logo;
    use egui::Color32;

    #[test]
    fn logo_has_transparent_corners_and_an_opaque_body() {
        let image = logo(64, Color32::WHITE).unwrap();
        let alpha = |x: usize, y: usize| image.rgba[(y * 64 + x) * 4 + 3];
        assert_eq!(alpha(0, 0), 0, "the background stays transparent");
        let opaque = image.rgba.chunks(4).filter(|p| p[3] > 200).count();
        assert!(opaque > 64 * 64 / 5, "the leaf must stay visible");
    }
}
