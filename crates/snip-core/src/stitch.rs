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
            image.as_raw().len().saturating_mul(3) <= max_bytes,
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
            for (x, y, pixel) in next.enumerate_pixels() {
                let (gx, gy) = (nx + i64::from(x), ny + i64::from(y));
                let (ox, oy) = ((gx - left) as u32, (gy - top) as u32);
                // Keep accepted overlap, including its original header. Only
                // append newly exposed pixels or fill previously unvisited holes.
                if gx < 0
                    || gy < 0
                    || gx >= i64::from(self.image.width())
                    || gy >= i64::from(self.image.height())
                    || out.get_pixel(ox, oy)[3] == 0
                {
                    out.put_pixel(ox, oy, *pixel);
                }
            }
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
    let samples = edge_samples(previous);
    let mut candidates = motion_candidates(&samples, next);
    let factor = previous.width().max(previous.height()).div_ceil(384);
    if factor > 1 {
        let (w, h) = (previous.width() / factor, previous.height() / factor);
        if w >= 8 && h >= 8 {
            let a = imageops::resize(previous, w, h, imageops::FilterType::Triangle);
            let b = imageops::resize(next, w, h, imageops::FilterType::Triangle);
            // Low resolution scores are only search seeds. Always verify at
            // original resolution, including odd-pixel scroll displacements.
            for (_, x, y) in motion_candidates(&edge_samples(&a), &b)
                .into_iter()
                .take(16)
            {
                let horizontal = x != 0;
                let center = if horizontal { x } else { y } * factor as i32;
                for delta in center - factor as i32 * 2..=center + factor as i32 * 2 {
                    let (x, y) = if horizontal { (delta, 0) } else { (0, delta) };
                    if let Some(cost) = score(&samples, next, x, y) {
                        candidates.push((cost, x, y));
                    }
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
fn motion_candidates(samples: &[(i32, i32, [u8; 3])], next: &RgbaImage) -> Vec<(f64, i32, i32)> {
    let mut candidates = Vec::new();
    for horizontal in [false, true] {
        let length = if horizontal {
            next.width()
        } else {
            next.height()
        } as i32;
        let limit = length - (length / 5).max(8);
        let step = (length / 512).max(1);
        let mut coarse = Vec::new();
        for delta in (-limit..=limit).step_by(step as usize) {
            let (x, y) = if horizontal { (delta, 0) } else { (0, delta) };
            if let Some(s) = score(samples, next, x, y) {
                coarse.push((s, delta));
            }
        }
        coarse.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, center) in coarse.into_iter().take(8) {
            for delta in (center - step).max(-limit)..=(center + step).min(limit) {
                let (x, y) = if horizontal { (delta, 0) } else { (0, delta) };
                if let Some(s) = score(samples, next, x, y) {
                    candidates.push((s, x, y));
                }
            }
        }
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    candidates.dedup_by(|a, b| a.1 == b.1 && a.2 == b.2);
    candidates
}
// Select high contrast points once, rather than letting blank page margins
// dominate every candidate. Spatial tiles keep a fixed header from consuming
// the whole feature budget. At most 2 * 32 * 48 samples, with bounded tile scans.
fn edge_samples(image: &RgbaImage) -> Vec<(i32, i32, [u8; 3])> {
    let mut samples = Vec::new();
    let (w, h) = image.dimensions();
    for row in 0..48u32 {
        for col in 0..32u32 {
            let (left, right) = (col * w / 32, (col + 1) * w / 32);
            let (top, bottom) = (row * h / 48, (row + 1) * h / 48);
            if top < h / 8 || bottom > h - h / 16 || left < w / 32 || right > w - w / 32 {
                continue;
            }
            let mut best = (0u16, left, top);
            for y in (top..bottom.min(h - 1)).step_by(((bottom - top) / 12).max(1) as usize) {
                for x in (left..right.min(w - 1)).step_by(((right - left) / 12).max(1) as usize) {
                    let p = image.get_pixel(x, y);
                    let rx = image.get_pixel(x + 1, y);
                    let by = image.get_pixel(x, y + 1);
                    let contrast = (0..3)
                        .map(|i| u16::from(p[i].abs_diff(rx[i])) + u16::from(p[i].abs_diff(by[i])))
                        .sum::<u16>();
                    if contrast > best.0 {
                        best = (contrast, x, y);
                    }
                }
            }
            if best.0 >= 48 {
                let p = image.get_pixel(best.1, best.2);
                samples.push((best.1 as i32, best.2 as i32, [p[0], p[1], p[2]]));
                let (x, y) = if (0..3)
                    .map(|i| p[i].abs_diff(image.get_pixel(best.1 + 1, best.2)[i]) as u16)
                    .sum::<u16>()
                    >= (0..3)
                        .map(|i| p[i].abs_diff(image.get_pixel(best.1, best.2 + 1)[i]) as u16)
                        .sum::<u16>()
                {
                    (best.1 + 1, best.2)
                } else {
                    (best.1, best.2 + 1)
                };
                let q = image.get_pixel(x, y);
                samples.push((x as i32, y as i32, [q[0], q[1], q[2]]));
            }
        }
    }
    samples
}
fn score(samples: &[(i32, i32, [u8; 3])], next: &RgbaImage, dx: i32, dy: i32) -> Option<f64> {
    let mut errors = [0u32; 256];
    let mut count = 0;
    for &(x, y, a) in samples {
        let (x, y) = (x - dx, y - dy);
        if x < next.width() as i32 / 32
            || y < next.height() as i32 / 8
            || x >= next.width() as i32 - next.width() as i32 / 32
            || y >= next.height() as i32 - next.height() as i32 / 16
        {
            continue;
        }
        let b = next.get_pixel(x as u32, y as u32);
        let error = (0..3).map(|i| u16::from(a[i].abs_diff(b[i]))).sum::<u16>() / 3;
        errors[error as usize] += 1;
        count += 1;
    }
    if count < (samples.len() as u32 / 12).max(16) {
        return None;
    }
    // Ignore the worst 15%: sticky headers, scrollbars and a blinking caret
    // are not part of the scrolling document. Keep the ambiguity rejection.
    let retained = count * 85 / 100;
    let mut remaining = retained;
    let mut sum = 0u64;
    for (error, n) in errors.into_iter().enumerate() {
        let n = n.min(remaining);
        sum += error as u64 * u64::from(n);
        remaining -= n;
        if remaining == 0 {
            break;
        }
    }
    Some(sum as f64 / f64::from(retained))
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
        let mut s = Stitcher::new(a.clone(), 400_000).unwrap();
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

    #[test]
    fn sparse_text_scrolls_past_fixed_header_and_scrollbar() {
        let page = RgbaImage::from_fn(360, 900, |x, y| {
            let line = y / 29;
            let glyph = x / 9;
            let ink = x > 45
                && x < 100 + (line * 37 % 210)
                && y % 29 > 10
                && y % 29 < 21
                && x % 9 < 6
                && ((glyph.wrapping_mul(73856093) ^ line.wrapping_mul(19349663))
                    .rotate_left(y % 29)
                    >> (x % 9))
                    & 1
                    == 1;
            image::Rgba(if ink {
                [30, 30, 30, 255]
            } else {
                [255, 255, 255, 255]
            })
        });
        let viewport = |top, height| {
            let mut out = imageops::crop_imm(&page, 0, top, 360, height).to_image();
            for y in 0..26 {
                for x in 0..360 {
                    out.put_pixel(x, y, image::Rgba([80 + (x % 130) as u8, 100, 140, 255]));
                }
            }
            for y in 26..height {
                out.put_pixel(355, y, image::Rgba([100, 100, 100, 255]));
            }
            out
        };
        for height in [260, 620] {
            let mut stitch = Stitcher::new(viewport(0, height), 8_000_000).unwrap();
            assert!(stitch.append(&viewport(67, height)).unwrap());
            assert_eq!(stitch.last_motion, Some((0, 67)));
            assert_eq!(stitch.image.height(), height + 67);
            assert!(stitch.append(&viewport(132, height)).unwrap());
            assert_eq!(stitch.image.height(), height + 132);
            assert_eq!(*stitch.image.get_pixel(50, 70), *page.get_pixel(50, 70));
        }
    }
}
