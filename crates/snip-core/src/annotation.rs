use crate::geometry::{Point, Rect};
use image::{RgbaImage, imageops};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tiny_skia::{Paint, PathBuilder, Pixmap, Stroke, Transform};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Tool {
    Select,
    Rectangle,
    RoundedRectangle,
    Diamond,
    Triangle,
    Ellipse,
    Line,
    Polyline,
    Curve,
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
    Uniform,
    HeadWide,
    TailWide,
    #[default]
    Swallow,
    TriangleFilled,
    TriangleHollow,
    CircleFilled,
    CircleHollow,
    DiamondFilled,
    DiamondHollow,
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
impl Tool {
    pub fn is_shape(self) -> bool {
        matches!(
            self,
            Self::Rectangle
                | Self::RoundedRectangle
                | Self::Ellipse
                | Self::Diamond
                | Self::Triangle
        )
    }
    pub fn is_line(self) -> bool {
        matches!(self, Self::Line | Self::Polyline | Self::Curve)
    }
}
impl Annotation {
    pub fn path_points(&self) -> Vec<Point> {
        let r = self.bounds();
        match self.tool {
            Tool::Diamond => vec![
                Point::new(r.x + r.width / 2., r.y),
                Point::new(r.right(), r.y + r.height / 2.),
                Point::new(r.x + r.width / 2., r.bottom()),
                Point::new(r.x, r.y + r.height / 2.),
                Point::new(r.x + r.width / 2., r.y),
            ],
            Tool::Triangle => vec![
                Point::new(r.x + r.width / 2., r.y),
                Point::new(r.right(), r.bottom()),
                Point::new(r.x, r.bottom()),
                Point::new(r.x + r.width / 2., r.y),
            ],
            Tool::RoundedRectangle => {
                let radius = r.width.min(r.height) * 0.2;
                let mut points = vec![];
                for (x, y, angle) in [
                    (r.right() - radius, r.y + radius, -90.),
                    (r.right() - radius, r.bottom() - radius, 0.),
                    (r.x + radius, r.bottom() - radius, 90.),
                    (r.x + radius, r.y + radius, 180.),
                ] {
                    for i in 0..=8 {
                        let t = (angle + i as f32 * 90. / 8.).to_radians();
                        points.push(Point::new(x + radius * t.cos(), y + radius * t.sin()));
                    }
                }
                if let Some(p) = points.first().copied() {
                    points.push(p);
                }
                points
            }
            Tool::Curve if self.points.len() > 2 => {
                let mut points = vec![];
                for i in 0..self.points.len() - 1 {
                    let a = self.points[i.saturating_sub(1)];
                    let b = self.points[i];
                    let c = self.points[i + 1];
                    let d = self.points[(i + 2).min(self.points.len() - 1)];
                    for k in 0..8 {
                        let t = k as f32 / 8.;
                        let f = |a: f32, b: f32, c: f32, d: f32| {
                            0.5 * ((2. * b)
                                + (-a + c) * t
                                + (2. * a - 5. * b + 4. * c - d) * t * t
                                + (-a + 3. * b - 3. * c + d) * t * t * t)
                        };
                        points.push(Point::new(f(a.x, b.x, c.x, d.x), f(a.y, b.y, c.y, d.y)));
                    }
                }
                points.push(*self.points.last().unwrap());
                points
            }
            _ => self.points.clone(),
        }
    }
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
            _ => (0.5, 0.5, 2.),
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
    /// Shared shaft/head geometry for preview, export and hit bounds.
    pub fn arrow_parts(&self) -> (Vec<Point>, Vec<Point>, bool) {
        if matches!(
            self.arrow_style,
            ArrowStyle::Uniform | ArrowStyle::HeadWide | ArrowStyle::TailWide
        ) {
            return (self.arrow_outline(), vec![], true);
        }
        if self.points.len() < 2 {
            return (vec![], vec![], true);
        }
        let a = self.points[0];
        let b = *self.points.last().unwrap();
        let length = (b.x - a.x).hypot(b.y - a.y);
        if length < 0.01 {
            return (vec![], vec![], true);
        }
        let ux = (b.x - a.x) / length;
        let uy = (b.y - a.y) / length;
        let r = (self.width * 2. + 5.).min(length * 0.3);
        let at = |x: f32, y: f32| Point::new(a.x + ux * x - uy * y, a.y + uy * x + ux * y);
        let circle = matches!(
            self.arrow_style,
            ArrowStyle::CircleFilled | ArrowStyle::CircleHollow
        );
        let diamond = matches!(
            self.arrow_style,
            ArrowStyle::DiamondFilled | ArrowStyle::DiamondHollow
        );
        let swallow = self.arrow_style == ArrowStyle::Swallow;
        let mut head = if circle {
            (0..32)
                .map(|i| {
                    let t = i as f32 * std::f32::consts::TAU / 32.;
                    at(length - r + r * t.cos(), r * t.sin())
                })
                .collect()
        } else if diamond {
            vec![
                at(length, 0.),
                at(length - r, r),
                at(length - 2. * r, 0.),
                at(length - r, -r),
            ]
        } else if swallow {
            vec![
                at(length, 0.),
                at(length - 2. * r, r),
                at(length - r * 1.3, 0.),
                at(length - 2. * r, -r),
            ]
        } else {
            vec![
                at(length, 0.),
                at(length - 2. * r, r),
                at(length - 2. * r, -r),
            ]
        };
        head.push(head[0]);
        let end = length - if swallow { r * 1.3 } else { r * 2. };
        let half = self.width * 0.5;
        let shaft = vec![
            at(0., half),
            at(end, half),
            at(end, -half),
            at(0., -half),
            at(0., half),
        ];
        let filled = !matches!(
            self.arrow_style,
            ArrowStyle::TriangleHollow | ArrowStyle::CircleHollow | ArrowStyle::DiamondHollow
        );
        (shaft, head, filled)
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
            let (mut shaft, head, _) = self.arrow_parts();
            shaft.extend(head);
            shaft
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
    /// Rebase source pixels and every history snapshot after resizing a screen
    /// selection. Marks retain their physical desktop position; export clips.
    pub fn reframe(&mut self, image: Arc<RgbaImage>, dx: f32, dy: f32) {
        self.original = image;
        for object in &mut self.objects {
            object.translate(dx, dy);
        }
        for snapshot in self.undo.iter_mut().chain(self.redo.iter_mut()) {
            for object in snapshot {
                object.translate(dx, dy);
            }
        }
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
                    let (shaft, head, filled) = a.arrow_parts();
                    for (points, fill) in [(shaft, true), (head, filled)] {
                        let mut path = PathBuilder::new();
                        if let Some(first) = points.first() {
                            path.move_to(first.x, first.y);
                            for p in points.iter().skip(1) {
                                path.line_to(p.x, p.y);
                            }
                            path.close();
                            if let Some(path) = path.finish() {
                                if fill {
                                    layer.fill_path(
                                        &path,
                                        &paint,
                                        tiny_skia::FillRule::Winding,
                                        Transform::identity(),
                                        None,
                                    );
                                } else {
                                    layer.stroke_path(
                                        &path,
                                        &paint,
                                        &stroke,
                                        Transform::identity(),
                                        None,
                                    );
                                }
                            }
                        }
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
                    let points = a.path_points();
                    pb.move_to(points[0].x, points[0].y);
                    for p in points.iter().skip(1) {
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
#[cfg(test)]
mod reframe_tests {
    use super::*;
    #[test]
    fn new_shapes_and_curves_export_visible_paths_without_mutating_source() {
        for tool in [
            Tool::RoundedRectangle,
            Tool::Diamond,
            Tool::Triangle,
            Tool::Polyline,
            Tool::Curve,
        ] {
            let mut doc = Document::new(Arc::new(RgbaImage::new(100, 100)));
            let points = vec![
                Point::new(10., 20.),
                Point::new(70., 30.),
                Point::new(50., 80.),
            ];
            let a = Annotation {
                tool,
                points: points.clone(),
                color: [220, 20, 30, 255],
                width: 3.,
                text: String::new(),
                arrow_style: ArrowStyle::default(),
            };
            let path = a.path_points();
            assert!(path.iter().all(|p| p.x.is_finite() && p.y.is_finite()));
            if tool.is_shape() {
                assert_eq!(path.first(), path.last());
            }
            if tool == Tool::Curve {
                assert!(path.len() > points.len());
                assert_eq!(path.first(), points.first());
                assert_eq!(path.last(), points.last());
            }
            doc.add(a);
            assert!(doc.flatten().pixels().any(|p| p[3] != 0), "{tool:?}");
            assert!(doc.original.pixels().all(|p| p[3] == 0));
            doc.undo();
            assert!(doc.objects.is_empty());
        }
    }

    #[test]
    fn all_arrow_heads_export_and_hollow_heads_preserve_background() {
        for style in [
            ArrowStyle::Swallow,
            ArrowStyle::TriangleFilled,
            ArrowStyle::TriangleHollow,
            ArrowStyle::CircleFilled,
            ArrowStyle::CircleHollow,
            ArrowStyle::DiamondFilled,
            ArrowStyle::DiamondHollow,
        ] {
            let original = Arc::new(RgbaImage::from_pixel(160, 80, image::Rgba([255; 4])));
            let mut doc = Document::new(original.clone());
            let a = Annotation {
                tool: Tool::Arrow,
                points: vec![Point::new(10., 40.), Point::new(140., 40.)],
                color: [240, 0, 0, 255],
                width: 3.,
                text: String::new(),
                arrow_style: style,
            };
            let (shaft, head, filled) = a.arrow_parts();
            assert!(!shaft.is_empty() && !head.is_empty());
            doc.add(a);
            let output = doc.flatten();
            assert!(output.pixels().any(|p| p[1] < 200));
            if !filled {
                assert_eq!(output.get_pixel(129, 40).0, [255; 4], "{style:?}");
            }
            assert!(original.pixels().all(|p| p.0 == [255; 4]));
        }
    }

    #[test]
    fn crop_rebase_preserves_desktop_marks_and_undo_coordinates() {
        let mut doc = Document::new(Arc::new(RgbaImage::new(100, 100)));
        doc.add(Annotation {
            tool: Tool::Rectangle,
            points: vec![Point::new(20., 30.), Point::new(40., 50.)],
            color: [20, 20, 200, 255],
            width: 2.,
            text: String::new(),
            arrow_style: ArrowStyle::default(),
        });
        doc.checkpoint();
        doc.objects[0].translate(10., 0.);
        doc.reframe(Arc::new(RgbaImage::new(150, 120)), -15., -5.);
        assert_eq!(doc.objects[0].points[0], Point::new(15., 25.));
        assert_eq!(doc.flatten().dimensions(), (150, 120));
        doc.undo();
        assert_eq!(doc.objects[0].points[0], Point::new(5., 25.));
        doc.redo();
        assert_eq!(doc.objects[0].points[0], Point::new(15., 25.));
        assert!(doc.original.pixels().all(|p| p.0 == [0, 0, 0, 0]));
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
