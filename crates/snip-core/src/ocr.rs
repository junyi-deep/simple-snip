use crate::geometry::{Point, Rect};
use anyhow::Result;
use image::RgbaImage;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}
#[derive(Clone, Debug)]
pub struct OcrWord {
    pub text: String,
    pub bounds: Rect,
    pub confidence: Option<f32>,
}
#[derive(Clone, Debug)]
pub struct OcrLine {
    pub text: String,
    pub bounds: Rect,
    pub words: Vec<OcrWord>,
    pub confidence: Option<f32>,
    pub language: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub struct OcrResult {
    pub lines: Vec<OcrLine>,
}
pub trait OcrEngine {
    fn recognize(&self, image: &RgbaImage, cancel: &Cancellation) -> Result<OcrResult>;
}
impl OcrResult {
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn hit(&self, p: Point) -> Option<(usize, usize)> {
        for (i, l) in self.lines.iter().enumerate() {
            for (j, w) in l.words.iter().enumerate() {
                if w.bounds.contains(p) {
                    return Some((i, j));
                }
            }
        }
        None
    }
    pub fn selected_text(&self, a: (usize, usize), b: (usize, usize)) -> String {
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        self.lines
            .iter()
            .enumerate()
            .filter(|(i, _)| *i >= a.0 && *i <= b.0)
            .map(|(i, l)| {
                let selected: Vec<_> = l
                    .words
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| (i, *j) >= a && (i, *j) <= b)
                    .map(|(_, w)| w)
                    .collect();
                let mut text = String::new();
                for (index, word) in selected.iter().enumerate() {
                    if index > 0 {
                        let previous = selected[index - 1];
                        let gap = word.bounds.x - previous.bounds.right();
                        if gap > word.bounds.height.min(previous.bounds.height) * 0.18 {
                            text.push(' ');
                        }
                    }
                    text.push_str(&word.text);
                }
                text
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    /// Simple geometric table extraction. No merged-cell or semantic claims.
    pub fn tsv(&self) -> String {
        let mut words: Vec<_> = self.lines.iter().flat_map(|l| l.words.iter()).collect();
        words.sort_by(|a, b| a.bounds.y.total_cmp(&b.bounds.y));
        let mut rows: Vec<Vec<&OcrWord>> = vec![];
        for word in words {
            if let Some(row) = rows.last_mut()
                && (row[0].bounds.y - word.bounds.y).abs()
                    < word.bounds.height.max(row[0].bounds.height) * 0.5
            {
                row.push(word);
                continue;
            }
            rows.push(vec![word]);
        }
        rows.into_iter()
            .map(|mut r| {
                r.sort_by(|a, b| a.bounds.x.total_cmp(&b.bounds.x));
                let mut text = String::new();
                for (index, word) in r.iter().enumerate() {
                    if index > 0 {
                        let previous = r[index - 1];
                        let gap = word.bounds.x - previous.bounds.right();
                        if gap > word.bounds.height * 1.2 {
                            text.push('\t');
                        } else if gap > word.bounds.height * 0.18 {
                            text.push(' ');
                        }
                    }
                    text.push_str(&word.text.replace(['\t', '\n'], " "));
                }
                text
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reverse_cross_line_selection() {
        let r = OcrResult {
            lines: (0..2)
                .map(|i| OcrLine {
                    text: format!("line {i}"),
                    bounds: Rect::default(),
                    confidence: None,
                    language: None,
                    words: vec![OcrWord {
                        text: i.to_string(),
                        bounds: Rect::new(0., i as f32 * 10., 8., 8.),
                        confidence: None,
                    }],
                })
                .collect(),
        };
        assert_eq!(r.selected_text((1, 0), (0, 0)), "0\n1");
        assert_eq!(r.hit(Point::new(2., 12.)), Some((1, 0)));
    }
}
