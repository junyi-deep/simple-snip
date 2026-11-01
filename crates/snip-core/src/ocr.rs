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
    /// Native engines usually return whole words. Expose character-sized units
    /// so selection can start/end inside a word; preserve the engine's text.
    pub fn selection_units(mut self) -> Self {
        for line in &mut self.lines {
            line.words = line
                .words
                .iter()
                .flat_map(|word| {
                    let count = word.text.chars().count().max(1) as f32;
                    word.text
                        .chars()
                        .enumerate()
                        .map(move |(index, ch)| OcrWord {
                            text: ch.to_string(),
                            bounds: Rect::new(
                                word.bounds.x + index as f32 * word.bounds.width / count,
                                word.bounds.y,
                                word.bounds.width / count,
                                word.bounds.height,
                            ),
                            confidence: word.confidence,
                        })
                })
                .collect();
        }
        self
    }
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
    /// Continue a text drag through whitespace and beyond the first/last glyph.
    pub fn nearest_hit(&self, p: Point) -> Option<(usize, usize)> {
        let distance = |r: Rect| {
            let dy = (r.y - p.y).max(0.).max(p.y - r.bottom());
            let dx = (r.x - p.x).max(0.).max(p.x - r.right());
            (dy, dx)
        };
        let (line, row) = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| !l.words.is_empty())
            .min_by(|(_, a), (_, b)| {
                let a = distance(a.bounds);
                let b = distance(b.bounds);
                a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1))
            })?;
        let word = row
            .words
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| distance(a.bounds).1.total_cmp(&distance(b.bounds).1))?
            .0;
        Some((line, word))
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
    #[test]
    fn character_selection_crosses_gaps_and_clamps_outside_lines() {
        let result = OcrResult {
            lines: vec![
                OcrLine {
                    text: "Hello world".into(),
                    bounds: Rect::new(10., 20., 120., 16.),
                    words: vec![
                        OcrWord {
                            text: "Hello".into(),
                            bounds: Rect::new(10., 20., 50., 16.),
                            confidence: None,
                        },
                        OcrWord {
                            text: "world".into(),
                            bounds: Rect::new(80., 20., 50., 16.),
                            confidence: None,
                        },
                    ],
                    confidence: None,
                    language: None,
                },
                OcrLine {
                    text: "中文".into(),
                    bounds: Rect::new(10., 60., 32., 16.),
                    words: vec![OcrWord {
                        text: "中文".into(),
                        bounds: Rect::new(10., 60., 32., 16.),
                        confidence: None,
                    }],
                    confidence: None,
                    language: None,
                },
            ],
        }
        .selection_units();
        assert_eq!(result.lines[0].words.len(), 10);
        assert_eq!(result.selected_text((0, 1), (0, 3)), "ell");
        assert_eq!(result.selected_text((0, 0), (0, 9)), "Hello world");
        assert_eq!(result.nearest_hit(Point::new(68., 27.)), Some((0, 4)));
        assert_eq!(result.nearest_hit(Point::new(300., 64.)), Some((1, 1)));
        assert_eq!(result.nearest_hit(Point::new(-100., -100.)), Some((0, 0)));
        assert_eq!(result.selected_text((1, 1), (0, 3)), "lo world\n中文");
    }
}
