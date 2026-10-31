//! Downscaling (area average) and JPEG encoding, in plain Rust on one thread
//! so a frame never takes more than one core away from the game.

/// A captured frame as the GPU copy maps it: 4 bytes per pixel, rows
/// `row_pitch` bytes apart (may include padding).
#[derive(Debug, Clone, Copy)]
pub struct RawFrame<'a> {
    pub data: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub row_pitch: usize,
    /// Byte order B, G, R, A (else R, G, B, A).
    pub bgra: bool,
}

/// An RGB image, 3 bytes per pixel.
#[derive(Debug, Clone)]
pub struct Rgb {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl Rgb {
    /// Mean of all channels, 0-255. Near 0 means a black frame.
    pub fn mean(&self) -> f64 {
        if self.data.is_empty() {
            return 0.0;
        }
        // Every 7th byte is plenty to tell black from a game frame.
        let (sum, n) = self.data.iter().step_by(7).fold((0u64, 0u64), |(s, n), &b| (s + u64::from(b), n + 1));
        sum as f64 / n as f64
    }
}

/// Box ranges in the source for each destination index.
fn spans(src: u32, dst: u32) -> Vec<(usize, usize)> {
    (0..dst)
        .map(|i| {
            let a = (u64::from(i) * u64::from(src) / u64::from(dst)) as usize;
            let b = ((u64::from(i) + 1) * u64::from(src) / u64::from(dst)) as usize;
            (a, b.max(a + 1).min(src as usize))
        })
        .collect()
}

/// Scales to at most `max_width` wide (keeping the aspect ratio), averaging
/// each source box so small text (minimap, gold) stays readable.
pub fn downscale(f: &RawFrame<'_>, max_width: u32) -> Option<Rgb> {
    if f.width == 0 || f.height == 0 || f.row_pitch < f.width as usize * 4 {
        return None;
    }
    if f.data.len() < f.row_pitch * (f.height as usize - 1) + f.width as usize * 4 {
        return None;
    }
    let dw = f.width.min(max_width.max(1));
    let dh = ((u64::from(f.height) * u64::from(dw) + u64::from(f.width) / 2) / u64::from(f.width)).max(1) as u32;
    let (xs, ys) = (spans(f.width, dw), spans(f.height, dh));
    let (r, b) = if f.bgra { (2, 0) } else { (0, 2) };
    let mut out = Vec::with_capacity(dw as usize * dh as usize * 3);
    let mut acc = vec![[0u32; 3]; dw as usize];
    for &(y0, y1) in &ys {
        acc.iter_mut().for_each(|a| *a = [0; 3]);
        for y in y0..y1 {
            let row = &f.data[y * f.row_pitch..y * f.row_pitch + f.width as usize * 4];
            for (a, &(x0, x1)) in acc.iter_mut().zip(&xs) {
                for px in row[x0 * 4..x1 * 4].as_chunks::<4>().0 {
                    a[0] += u32::from(px[r]);
                    a[1] += u32::from(px[1]);
                    a[2] += u32::from(px[b]);
                }
            }
        }
        let rows = (y1 - y0) as u32;
        for (a, &(x0, x1)) in acc.iter().zip(&xs) {
            let n = rows * (x1 - x0) as u32;
            out.extend(a.iter().map(|&s| ((s + n / 2) / n) as u8));
        }
    }
    Some(Rgb { data: out, width: dw, height: dh })
}

pub const JPEG_QUALITY: u8 = 82;

pub fn jpeg(img: &Rgb, quality: u8) -> Result<Vec<u8>, String> {
    let (w, h) =
        (u16::try_from(img.width).map_err(|_| "too wide")?, u16::try_from(img.height).map_err(|_| "too tall")?);
    let mut out = Vec::with_capacity(img.data.len() / 8);
    jpeg_encoder::Encoder::new(&mut out, quality)
        .encode(&img.data, w, h, jpeg_encoder::ColorType::Rgb)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32, pitch: usize, px: [u8; 4]) -> Vec<u8> {
        let mut v = vec![0u8; pitch * h as usize];
        for y in 0..h as usize {
            for x in 0..w as usize {
                v[y * pitch + x * 4..y * pitch + x * 4 + 4].copy_from_slice(&px);
            }
        }
        v
    }

    #[test]
    fn downscales_with_padding_and_channel_order() {
        // 30x20 BGRA with 8 bytes of row padding, pure red (B=0, G=0, R=200).
        let data = frame(30, 20, 30 * 4 + 8, [0, 0, 200, 255]);
        let raw = RawFrame { data: &data, width: 30, height: 20, row_pitch: 30 * 4 + 8, bgra: true };
        let img = downscale(&raw, 20).expect("img");
        assert_eq!((img.width, img.height), (20, 13));
        assert_eq!(img.data.len(), 20 * 13 * 3);
        assert_eq!(&img.data[..3], &[200, 0, 0]);
        assert!(img.mean() > 50.0);
        // No upscaling.
        assert_eq!(downscale(&raw, 4000).map(|i| i.width), Some(30));
        // Truncated buffer is refused, not read out of bounds.
        let short = RawFrame { data: &data[..100], ..raw };
        assert!(downscale(&short, 20).is_none());
    }

    #[test]
    fn averages_boxes() {
        // 2x1 RGBA: black and white average to grey at width 1.
        let data = [0, 0, 0, 255, 255, 255, 255, 255];
        let raw = RawFrame { data: &data, width: 2, height: 1, row_pitch: 8, bgra: false };
        let img = downscale(&raw, 1).expect("img");
        assert_eq!(img.data, vec![128, 128, 128]);
    }

    #[test]
    fn encodes_jpeg() {
        let data = frame(64, 36, 64 * 4, [10, 120, 200, 255]);
        let raw = RawFrame { data: &data, width: 64, height: 36, row_pitch: 64 * 4, bgra: true };
        let bytes = jpeg(&downscale(&raw, 64).expect("img"), JPEG_QUALITY).expect("jpeg");
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "JPEG start marker");
        let black = Rgb { data: vec![0; 300], width: 10, height: 10 };
        assert!(black.mean() < 1.0);
    }
}
