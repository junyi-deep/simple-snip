use anyhow::{Result, ensure};
use image::GenericImageView;
use image::{RgbaImage, imageops};
/// Bounded, append-only stitcher; failures preserve the previous composite.
pub struct Stitcher {
    pub image: RgbaImage,
    pub frames: usize,
    pub max_bytes: usize,
    pub last_overlap: Option<u32>,
}
impl Stitcher {
    pub fn new(image: RgbaImage, max_bytes: usize) -> Result<Self> {
        ensure!(image.as_raw().len() <= max_bytes, "首帧超过长截图内存上限");
        Ok(Self {
            image,
            frames: 1,
            max_bytes,
            last_overlap: None,
        })
    }
    pub fn append(&mut self, next: &RgbaImage) -> Result<bool> {
        ensure!(
            next.width() == self.image.width(),
            "图像宽度改变，请重新选择区域"
        );
        if self.image.height() >= next.height()
            && imageops::crop_imm(
                &self.image,
                0,
                self.image.height() - next.height(),
                next.width(),
                next.height(),
            )
            .pixels()
            .all(|(x, y, p)| p == *next.get_pixel(x, y))
        {
            self.last_overlap = Some(next.height());
            return Ok(false);
        }
        let overlap = find_overlap(&self.image, next).ok_or_else(|| {
            anyhow::anyhow!("无法可靠匹配重叠区域，已保留当前结果；请减小滚动幅度后重试")
        })?;
        self.last_overlap = Some(overlap);
        if overlap >= next.height() {
            return Ok(false);
        }
        let height = self.image.height() + next.height() - overlap;
        // Includes old composite, incoming frame and new allocation at peak.
        ensure!(
            (height as usize * self.image.width() as usize * 4)
                + self.image.as_raw().len()
                + next.as_raw().len()
                <= self.max_bytes,
            "达到长截图内存上限，请导出已有结果"
        );
        let mut out = RgbaImage::new(next.width(), height);
        imageops::replace(&mut out, &self.image, 0, 0);
        let tail =
            imageops::crop_imm(next, 0, overlap, next.width(), next.height() - overlap).to_image();
        imageops::replace(&mut out, &tail, 0, self.image.height() as i64);
        self.image = out;
        self.frames += 1;
        Ok(true)
    }
}
/// Dense vertical search, sparse spatial samples; reject ambiguous blank bands.
pub fn find_overlap(previous: &RgbaImage, next: &RgbaImage) -> Option<u32> {
    if previous.width() != next.width() || next.width() < 4 {
        return None;
    }
    let max = previous.height().min(next.height());
    let min = (max / 8).max(8);
    if max < min {
        return None;
    }
    let mut best = (f64::MAX, 0);
    let mut second = f64::MAX;
    for overlap in min..=max {
        let mut error = 0u64;
        let mut n = 0u64;
        let mut low = 255u8;
        let mut high = 0u8;
        for y in (0..overlap).step_by((overlap / 24).max(1) as usize) {
            for x in (next.width() / 12..next.width() * 11 / 12)
                .step_by((next.width() / 48).max(1) as usize)
            {
                let a = previous.get_pixel(x, previous.height() - overlap + y);
                let b = next.get_pixel(x, y);
                for i in 0..3 {
                    error += a[i].abs_diff(b[i]) as u64;
                    low = low.min(b[i]);
                    high = high.max(b[i]);
                    n += 1;
                }
            }
        }
        if high - low < 20 {
            continue;
        }
        let score = error as f64 / n.max(1) as f64;
        if score < best.0 {
            second = best.0;
            best = (score, overlap);
        } else {
            second = second.min(score);
        }
    }
    if best.0 < 7. && (second > best.0 * 1.15 || best.0 < 0.5) {
        Some(best.1)
    } else {
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn joins_and_deduplicates() {
        let page = RgbaImage::from_fn(80, 500, |x, y| {
            image::Rgba([
                ((x * 13 + y * 17) % 251) as u8,
                ((y * 3 + x) % 241) as u8,
                (y % 255) as u8,
                255,
            ])
        });
        let a = imageops::crop_imm(&page, 0, 0, 80, 300).to_image();
        let b = imageops::crop_imm(&page, 0, 200, 80, 300).to_image();
        assert_eq!(find_overlap(&a, &b), Some(100));
        let mut s = Stitcher::new(a, 1_000_000).unwrap();
        assert!(s.append(&b).unwrap());
        assert_eq!(s.image, page);
        assert!(!s.append(&b).unwrap());
    }
    #[test]
    fn rejects_blank_and_preserves() {
        let a = RgbaImage::new(80, 100);
        let mut s = Stitcher::new(a.clone(), 1_000_000).unwrap();
        let b = RgbaImage::from_pixel(80, 100, image::Rgba([200, 200, 200, 255]));
        assert!(s.append(&b).is_err());
        assert_eq!(s.image, a);
    }
}
