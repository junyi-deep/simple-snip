use anyhow::{Result, ensure};
use image::{RgbaImage, imageops};

#[derive(Debug)]
pub enum StitchError {
    NoMatch,
    MemoryLimit,
}
impl std::fmt::Display for StitchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoMatch => "无法可靠匹配滚动内容，已保留结果；请减小滚动幅度、避开固定页头后重试",
            Self::MemoryLimit => "达到长截图内存上限，请导出已有结果",
        })
    }
}
impl std::error::Error for StitchError {}

/// Bounded canvas registration against the last viewport, including reversals.
/// A failed match or allocation check never changes the accepted canvas.
pub struct Stitcher {
    pub image: RgbaImage,
    pub frames: usize,
    pub max_bytes: usize,
    pub last_overlap: Option<u32>,
    pub last_motion: Option<(i32, i32)>,
    previous: RgbaImage,
    origin: (i64, i64),
}
impl Stitcher {
    pub fn new(image: RgbaImage, max_bytes: usize) -> Result<Self> {
        ensure!(image.width() > 0 && image.height() > 0, "截图区域为空");
        ensure!(
            image.as_raw().len().saturating_mul(2) <= max_bytes,
            "首帧超过长截图内存上限"
        );
        Ok(Self {
            previous: image.clone(),
            image,
            frames: 1,
            max_bytes,
            last_overlap: None,
            last_motion: None,
            origin: (0, 0),
        })
    }
    pub fn append(&mut self, next: &RgbaImage) -> Result<bool> {
        ensure!(
            next.dimensions() == self.previous.dimensions(),
            "采集区域尺寸改变，请重新选择区域"
        );
        if next == &self.previous {
            self.last_motion = Some((0, 0));
            return Ok(false);
        }
        let (dx, dy) = find_motion(&self.previous, next).ok_or(StitchError::NoMatch)?;
        let nx = self.origin.0 + dx as i64;
        let ny = self.origin.1 + dy as i64;
        let left = nx.min(0);
        let top = ny.min(0);
        let right = (nx + next.width() as i64).max(self.image.width() as i64);
        let bottom = (ny + next.height() as i64).max(self.image.height() as i64);
        let width = u32::try_from(right - left)?;
        let height = u32::try_from(bottom - top)?;
        // Old canvas, new canvas, previous viewport and incoming frame coexist.
        let bytes = (width as u64)
            .checked_mul(height as u64)
            .and_then(|v| v.checked_mul(4))
            .ok_or_else(|| anyhow::anyhow!("长截图尺寸超出上限"))?;
        let peak = bytes
            .saturating_add(self.image.as_raw().len() as u64)
            .saturating_add(self.previous.as_raw().len() as u64)
            .saturating_add(next.as_raw().len() as u64);
        if peak > self.max_bytes as u64 {
            return Err(StitchError::MemoryLimit.into());
        }
        let expanded = width != self.image.width() || height != self.image.height();
        if expanded {
            let mut out = RgbaImage::new(width, height);
            imageops::replace(&mut out, &self.image, -left, -top);
            imageops::replace(&mut out, next, nx - left, ny - top);
            self.image = out;
            self.frames += 1;
        }
        // Revisiting previously captured content changes the registration origin
        // even when it adds no pixels. This prevents duplicate strips on reversal.
        self.origin = (nx - left, ny - top);
        self.previous = next.clone();
        self.last_motion = Some((dx, dy));
        self.last_overlap = Some(if dx != 0 {
            next.width() - dx.unsigned_abs()
        } else {
            next.height() - dy.unsigned_abs()
        });
        Ok(expanded)
    }
}

/// Image-space displacement of the next viewport. Supports cardinal scrolling;
/// diagonal movement, ambiguous periodic patterns and flat fields are rejected.
pub fn find_motion(previous: &RgbaImage, next: &RgbaImage) -> Option<(i32, i32)> {
    if previous.dimensions() != next.dimensions() || next.width().min(next.height()) < 8 {
        return None;
    }
    if previous == next {
        return Some((0, 0));
    }
    if score(previous, next, 0, 0).is_some_and(|s| s < 1.) {
        return Some((0, 0));
    }
    let mut candidates = Vec::new();
    for horizontal in [false, true] {
        let length = if horizontal {
            next.width()
        } else {
            next.height()
        } as i32;
        let limit = length - (length / 5).max(8);
        let step = (length / 256).max(1);
        let mut coarse = Vec::new();
        for delta in (-limit..=limit).step_by(step as usize) {
            let (x, y) = if horizontal { (delta, 0) } else { (0, delta) };
            if let Some(s) = score(previous, next, x, y) {
                coarse.push((s, delta));
            }
        }
        coarse.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, center) in coarse.into_iter().take(8) {
            for delta in (center - step).max(-limit)..=(center + step).min(limit) {
                let (x, y) = if horizontal { (delta, 0) } else { (0, delta) };
                if let Some(s) = score(previous, next, x, y) {
                    candidates.push((s, x, y));
                }
            }
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    let &(best, x, y) = candidates.first()?;
    let second = candidates
        .iter()
        .find(|(_, dx, dy)| (dx - x).abs() + (dy - y).abs() > 2)
        .map_or(f64::INFINITY, |v| v.0);
    if best < 7. && second > best + 0.8 {
        Some((x, y))
    } else {
        None
    }
}
fn score(previous: &RgbaImage, next: &RgbaImage, dx: i32, dy: i32) -> Option<f64> {
    let (w, h) = (next.width() as i32, next.height() as i32);
    let (x0, x1) = ((-dx).max(0), (w - dx).min(w));
    let (y0, y1) = ((-dy).max(0), (h - dy).min(h));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut error = 0u64;
    let (mut low, mut high) = (255u16, 0u16);
    for gy in 0..16 {
        let y = y0 + (y1 - y0) * (gy * 2 + 1) / 32;
        for gx in 0..24 {
            let x = x0 + (x1 - x0) * (gx * 2 + 1) / 48;
            let a = previous.get_pixel((x + dx) as u32, (y + dy) as u32);
            let b = next.get_pixel(x as u32, y as u32);
            for i in 0..3 {
                error += a[i].abs_diff(b[i]) as u64;
            }
            let luma = (b[0] as u16 * 3 + b[1] as u16 * 6 + b[2] as u16) / 10;
            low = low.min(luma);
            high = high.max(luma);
        }
    }
    if high - low < 16 {
        None
    } else {
        Some(error as f64 / (16. * 24. * 3.))
    }
}
/// Compatibility helper for vertical append callers.
pub fn find_overlap(previous: &RgbaImage, next: &RgbaImage) -> Option<u32> {
    if previous.width() != next.width() || previous.height() < next.height() {
        return None;
    }
    let tail = imageops::crop_imm(
        previous,
        0,
        previous.height() - next.height(),
        next.width(),
        next.height(),
    )
    .to_image();
    let (dx, dy) = find_motion(&tail, next)?;
    (dx == 0 && dy >= 0).then_some(next.height() - dy as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page() -> RgbaImage {
        RgbaImage::from_fn(420, 500, |x, y| {
            let z = x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663);
            image::Rgba([
                (z % 251) as u8,
                ((z >> 8) % 241) as u8,
                ((z >> 16) % 239) as u8,
                255,
            ])
        })
    }
    #[test]
    fn joins_and_deduplicates() {
        let page = page();
        let a = imageops::crop_imm(&page, 0, 0, 80, 300).to_image();
        let b = imageops::crop_imm(&page, 0, 200, 80, 300).to_image();
        assert_eq!(find_overlap(&a, &b), Some(100));
        let mut s = Stitcher::new(a, 1_000_000).unwrap();
        assert!(s.append(&b).unwrap());
        assert_eq!(s.image, imageops::crop_imm(&page, 0, 0, 80, 500).to_image());
        assert!(!s.append(&b).unwrap());
    }
    #[test]
    fn horizontal_reverse_and_axis_changes_register_the_last_viewport() {
        let page = page();
        let crop = |x, y| imageops::crop_imm(&page, x, y, 200, 200).to_image();
        let mut s = Stitcher::new(crop(100, 100), 4_000_000).unwrap();
        assert!(s.append(&crop(20, 100)).unwrap());
        assert_eq!(s.last_motion, Some((-80, 0)));
        assert!(!s.append(&crop(100, 100)).unwrap());
        assert!(s.append(&crop(170, 100)).unwrap());
        assert_eq!(
            s.image,
            imageops::crop_imm(&page, 20, 100, 350, 200).to_image()
        );
        assert!(s.append(&crop(170, 20)).unwrap());
        assert_eq!(s.last_motion, Some((0, -80)));
        assert_eq!(s.image.dimensions(), (350, 280));
        assert_eq!(*s.image.get_pixel(0, 0), image::Rgba([0, 0, 0, 0]));
        assert_eq!(*s.image.get_pixel(150, 0), *page.get_pixel(170, 20));
    }
    #[test]
    fn rejects_blank_diagonal_and_budget_overflow_without_losing_result() {
        let page = page();
        let a = imageops::crop_imm(&page, 0, 0, 160, 180).to_image();
        let mut s = Stitcher::new(a.clone(), 240_000).unwrap();
        assert!(
            s.append(&imageops::crop_imm(&page, 60, 50, 160, 180).to_image())
                .is_err()
        );
        assert!(
            s.append(&imageops::crop_imm(&page, 0, 100, 160, 180).to_image())
                .is_err()
        );
        assert_eq!(s.image, a);
        assert!(
            s.append(&RgbaImage::from_pixel(
                160,
                180,
                image::Rgba([200, 200, 200, 255])
            ))
            .is_err()
        );
        assert_eq!(s.image, a);
    }
    #[test]
    fn rejects_periodic_content_instead_of_guessing_scroll_distance() {
        let a = RgbaImage::from_fn(100, 160, |x, y| {
            image::Rgba([((x % 11) * 20) as u8, ((y % 16) * 15) as u8, 0, 255])
        });
        let b = RgbaImage::from_fn(100, 160, |x, y| *a.get_pixel(x, (y + 8) % 160));
        assert!(find_motion(&a, &b).is_none());
    }
}
