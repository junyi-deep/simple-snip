use crate::geometry::{Point, Rect};
use image::{RgbaImage, imageops};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tiny_skia::{Paint, PathBuilder, Pixmap, Stroke, Transform};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tool {
    Select,
    Rectangle,
    Ellipse,
    Line,
    Arrow,
    Pen,
    Highlight,
    Text,
    Number,
    Mosaic,
    Blur,
    Eraser,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArrowStyle {
    #[default]
    Uniform,
    HeadWide,
    TailWide,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    pub tool: Tool,
    pub points: Vec<Point>,
    pub color: [u8; 4],
    pub width: f32,
    pub text: String,
    #[serde(default)]
    pub arrow_style: ArrowStyle,
}
impl Annotation {
    pub fn number_radius(&self) -> f32 {
        let font = self.width * 6. + 12.;
        (font * 0.7).max(self.text.chars().count() as f32 * font * 0.32 + 4.)
    }
    pub fn arrow_outline(&self) -> Vec<Point> {
        if self.points.len() < 2 {
            return vec![];
        }
        let a = self.points[0];
        let b = *self.points.last().unwrap();
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let length = dx.hypot(dy);
        if length < 0.01 {
            return vec![];
        }
        let ux = dx / length;
        let uy = dy / length;
        let nx = -uy;
        let ny = ux;
        let head = (self.width * 4. + 10.).min(length * 0.45);
        let (tail, neck, tip) = match self.arrow_style {
            ArrowStyle::Uniform => (0.5, 0.5, 2.),
            ArrowStyle::HeadWide => (0.16, 1.1, 2.4),
            ArrowStyle::TailWide => (2., 0.25, 1.2),
        };
        let at = |along: f32, side: f32| {
            Point::new(
                a.x + ux * along + nx * side * self.width,
                a.y + uy * along + ny * side * self.width,
            )
        };
        vec![
            at(0., tail),
            at(length - head, neck),
            at(length - head, tip),
            b,
            at(length - head, -tip),
            at(length - head, -neck),
            at(0., -tail),
        ]
    }
    pub fn bounds(&self) -> Rect {
        if self.points.is_empty() {
            return Rect::new(0., 0., 0., 0.);
        }
        if self.tool == Tool::Number {
            let p = self.points[0];
            let r = self.number_radius();
            return Rect::new(p.x - r, p.y - r, r * 2., r * 2.);
        }

        let mut r = Rect::new(f32::MAX, f32::MAX, 0., 0.);
        let mut right = f32::MIN;
        let mut bottom = f32::MIN;
        let arrow = if self.tool == Tool::Arrow {
            self.arrow_outline()
        } else {
            vec![]
        };
        let points = if arrow.is_empty() {
            &self.points
        } else {
            &arrow
        };
        for p in points {
            r.x = r.x.min(p.x);
            r.y = r.y.min(p.y);
            right = right.max(p.x);
            bottom = bottom.max(p.y);
        }
        r.width = (right - r.x).max(1.);
        r.height = (bottom - r.y).max(1.);
        r
    }
    pub fn translate(&mut self, dx: f32, dy: f32) {
        for p in &mut self.points {
            p.x += dx;
            p.y += dy;
        }
    }
}
#[derive(Clone)]
pub struct Document {
    pub original: Arc<RgbaImage>,
    pub font: Option<Arc<fontdue::Font>>,
    pub objects: Vec<Annotation>,
    undo: Vec<Vec<Annotation>>,
    redo: Vec<Vec<Annotation>>,
}
impl Document {
    pub fn new(image: Arc<RgbaImage>) -> Self {
        Self {
            original: image,
            font: None,
            objects: vec![],
            undo: vec![],
            redo: vec![],
        }
    }
    pub fn checkpoint(&mut self) {
        if self.undo.len() >= 100 {
            self.undo.remove(0);
        }
        self.undo.push(self.objects.clone());
        self.redo.clear();
    }
    pub fn add(&mut self, a: Annotation) {
        self.checkpoint();
        self.objects.push(a);
    }
    pub fn undo(&mut self) {
        if let Some(v) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.objects, v));
        }
    }
    pub fn redo(&mut self) {
        if let Some(v) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.objects, v));
        }
    }
    pub fn remove_at(&mut self, p: Point) {
        if let Some(i) = self.hit(p) {
            self.checkpoint();
            self.objects.remove(i);
        }
    }
    pub fn remove_at_radius(&mut self, p: Point, radius: f32) {
        if let Some(i) = self.objects.iter().rposition(|a| {
            let r = a.bounds();
            let x = p.x.clamp(r.x, r.right());
            let y = p.y.clamp(r.y, r.bottom());
            (p.x - x).hypot(p.y - y) <= radius
        }) {
            self.checkpoint();
            self.objects.remove(i);
        }
    }
    pub fn hit(&self, p: Point) -> Option<usize> {
        self.objects.iter().rposition(|a| {
            let r = a.bounds();
            Rect::new(r.x - 8., r.y - 8., r.width + 16., r.height + 16.).contains(p)
        })
    }
    /// Composites once for export. Source pixels are never edited in place.
    pub fn flatten(&self) -> RgbaImage {
        let mut output = (*self.original).clone();
        for a in &self.objects {
            if a.points.is_empty() {
                continue;
            }
            if a.tool == Tool::Text {
                if let Some(font) = &self.font {
                    draw_text(&mut output, font, a);
                }
                continue;
            }
            if matches!(a.tool, Tool::Blur | Tool::Mosaic) {
                let (x, y, w, h) = a.bounds().pixel_bounds(output.width(), output.height());
                if w == 0 || h == 0 {
                    continue;
                }
                let region = imageops::crop_imm(&output, x, y, w, h).to_image();
                let filtered = if a.tool == Tool::Blur {
                    imageops::blur(&region, a.width.max(3.))
                } else {
                    let small = imageops::resize(
                        &region,
                        (w / (a.width * 3.).clamp(2., 96.) as u32).max(1),
                        (h / (a.width * 3.).clamp(2., 96.) as u32).max(1),
                        imageops::FilterType::Triangle,
                    );
                    imageops::resize(&small, w, h, imageops::FilterType::Nearest)
                };
                imageops::replace(&mut output, &filtered, x as i64, y as i64);
                continue;
            }
            let Some(mut layer) = Pixmap::new(output.width(), output.height()) else {
                continue;
            };
            let mut paint = Paint::default();
            let [r, g, b, alpha] = a.color;
            paint.set_color_rgba8(r, g, b, if a.tool == Tool::Highlight { 70 } else { alpha });
            paint.anti_alias = true;
            let stroke = Stroke {
                width: if a.tool == Tool::Highlight {
                    a.width * 5.
                } else {
                    a.width
                },
                line_cap: tiny_skia::LineCap::Round,
                line_join: tiny_skia::LineJoin::Round,
                ..Default::default()
            };
            let mut pb = PathBuilder::new();
            let rect = a.bounds();
            match a.tool {
                Tool::Number => {
                    let p = a.points[0];
                    pb.push_circle(p.x, p.y, a.number_radius());
                }
                Tool::Arrow => {
                    let points = a.arrow_outline();
                    if let Some(first) = points.first() {
                        pb.move_to(first.x, first.y);
                        for p in points.iter().skip(1) {
                            pb.line_to(p.x, p.y);
                        }
                        pb.close();
                    }
                }
                Tool::Rectangle => {
                    if let Some(r) =
                        tiny_skia::Rect::from_xywh(rect.x, rect.y, rect.width, rect.height)
                    {
                        pb.push_rect(r);
                    }
                }
                Tool::Ellipse => {
                    if let Some(r) =
                        tiny_skia::Rect::from_xywh(rect.x, rect.y, rect.width, rect.height)
                    {
                        pb.push_oval(r);
                    }
                }
                _ => {
                    pb.move_to(a.points[0].x, a.points[0].y);
                    for p in a.points.iter().skip(1) {
                        pb.line_to(p.x, p.y);
                    }
                }
            }
            if let Some(path) = pb.finish() {
                if matches!(a.tool, Tool::Arrow | Tool::Number) {
                    layer.fill_path(
                        &path,
                        &paint,
                        tiny_skia::FillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                } else {
                    layer.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
                }
            }
            // tiny-skia stores premultiplied RGBA; image expects straight RGBA.
            for (dst, src) in output.pixels_mut().zip(layer.pixels()) {
                let alpha = src.alpha() as f32 / 255.;
                if alpha == 0. {
                    continue;
                }
                for (i, value) in [src.red(), src.green(), src.blue()].iter().enumerate() {
                    dst[i] = (*value as f32 + dst[i] as f32 * (1. - alpha))
                        .round()
                        .min(255.) as u8;
                }
                dst[3] = (src.alpha() as f32 + dst[3] as f32 * (1. - alpha))
                    .round()
                    .min(255.) as u8;
            }
            if a.tool == Tool::Number
                && let Some(font) = &self.font
            {
                draw_text(&mut output, font, a);
            }
        }
        output
    }
}
fn draw_text(image: &mut RgbaImage, font: &fontdue::Font, a: &Annotation) {
    use fontdue::layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle};
    let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
    layout.reset(&LayoutSettings {
        x: if a.tool == Tool::Number {
            0.
        } else {
            a.points[0].x
        },
        y: if a.tool == Tool::Number {
            0.
        } else {
            a.points[0].y
        },
        ..Default::default()
    });
    layout.append(&[font], &TextStyle::new(&a.text, a.width * 6. + 12., 0));
    let mut offset = Point::new(0., 0.);
    if a.tool == Tool::Number && !layout.glyphs().is_empty() {
        let minx = layout.glyphs().iter().map(|g| g.x).fold(f32::MAX, f32::min);
        let miny = layout.glyphs().iter().map(|g| g.y).fold(f32::MAX, f32::min);
        let maxx = layout
            .glyphs()
            .iter()
            .map(|g| g.x + g.width as f32)
            .fold(f32::MIN, f32::max);
        let maxy = layout
            .glyphs()
            .iter()
            .map(|g| g.y + g.height as f32)
            .fold(f32::MIN, f32::max);
        offset = Point::new(
            a.points[0].x - (minx + maxx) / 2.,
            a.points[0].y - (miny + maxy) / 2.,
        );
    }
    let ink = if a.tool == Tool::Number {
        if (a.color[0] as f32 * 0.2126 + a.color[1] as f32 * 0.7152 + a.color[2] as f32 * 0.0722)
            > 165.
        {
            [20, 20, 20, 255]
        } else {
            [255, 255, 255, 255]
        }
    } else {
        a.color
    };
    for glyph in layout.glyphs() {
        let (metrics, bitmap) = font.rasterize_config(glyph.key);
        for y in 0..metrics.height {
            for x in 0..metrics.width {
                let px = (glyph.x + offset.x) as i32 + x as i32;
                let py = (glyph.y + offset.y) as i32 + y as i32;
                if px < 0 || py < 0 || px >= image.width() as i32 || py >= image.height() as i32 {
                    continue;
                }
                let alpha = bitmap[y * metrics.width + x] as f32 / 255. * ink[3] as f32 / 255.;
                let pixel = image.get_pixel_mut(px as u32, py as u32);
                for c in 0..3 {
                    pixel[c] = (pixel[c] as f32 * (1. - alpha) + ink[c] as f32 * alpha) as u8;
                }
                pixel[3] = 255;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn undo_redo_and_source_immutable() {
        let source = Arc::new(RgbaImage::from_pixel(40, 40, image::Rgba([255; 4])));
        let mut d = Document::new(source.clone());
        d.add(Annotation {
            tool: Tool::Rectangle,
            points: vec![Point::new(5., 5.), Point::new(30., 30.)],
            color: [255, 0, 0, 255],
            width: 3.,
            text: String::new(),
            arrow_style: ArrowStyle::default(),
        });
        assert_ne!(d.flatten(), *source);
        assert_eq!(source.get_pixel(5, 5).0, [255; 4]);
        d.undo();
        assert_eq!(d.flatten(), *source);
        d.redo();
        assert_eq!(d.objects.len(), 1);
    }
    #[test]
    fn pixel_filters_clip_to_image() {
        let mut d = Document::new(Arc::new(RgbaImage::new(20, 20)));
        d.add(Annotation {
            tool: Tool::Mosaic,
            points: vec![Point::new(-10., -10.), Point::new(30., 30.)],
            color: [0; 4],
            width: 4.,
            text: String::new(),
            arrow_style: ArrowStyle::default(),
        });
        assert_eq!(d.flatten().dimensions(), (20, 20));
    }
}

#[cfg(test)]
mod new_tool_tests {
    use super::*;
    fn arrow(style: ArrowStyle) -> Annotation {
        Annotation {
            tool: Tool::Arrow,
            points: vec![Point::new(30., 80.), Point::new(200., 80.)],
            color: [255, 0, 0, 255],
            width: 8.,
            text: String::new(),
            arrow_style: style,
        }
    }
    #[test]
    fn arrow_styles_have_distinct_taper_and_zero_length_is_safe() {
        let uniform = arrow(ArrowStyle::Uniform).arrow_outline();
        let head = arrow(ArrowStyle::HeadWide).arrow_outline();
        let tail = arrow(ArrowStyle::TailWide).arrow_outline();
        assert_eq!(uniform[0].y - uniform[6].y, uniform[1].y - uniform[5].y);
        assert!(head[0].y - head[6].y < head[1].y - head[5].y);
        assert!(tail[0].y - tail[6].y > tail[1].y - tail[5].y);
        let mut zero = arrow(ArrowStyle::HeadWide);
        zero.points[1] = zero.points[0];
        assert!(zero.arrow_outline().is_empty());
    }
    #[test]
    fn arrow_style_is_exported_and_undoable() {
        let mut doc = Document::new(Arc::new(RgbaImage::from_pixel(
            240,
            160,
            image::Rgba([255; 4]),
        )));
        doc.add(arrow(ArrowStyle::Uniform));
        let first = doc.flatten();
        doc.checkpoint();
        doc.objects[0].arrow_style = ArrowStyle::TailWide;
        assert_ne!(first, doc.flatten());
        doc.undo();
        assert_eq!(first, doc.flatten());
    }
    #[test]
    fn number_circle_is_hit_testable_and_exported_without_changing_source() {
        let source = Arc::new(RgbaImage::from_pixel(120, 120, image::Rgba([255; 4])));
        let mut doc = Document::new(source.clone());
        doc.add(Annotation {
            tool: Tool::Number,
            points: vec![Point::new(60., 60.)],
            width: 3.,
            color: [200, 0, 0, 255],
            text: "12".into(),
            arrow_style: ArrowStyle::default(),
        });
        assert_eq!(doc.hit(Point::new(50., 60.)), Some(0));
        assert_ne!(doc.flatten(), *source);
        doc.undo();
        assert_eq!(doc.flatten(), *source);
    }
}
