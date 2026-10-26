use serde::{Deserialize, Serialize};

/// Image pixels, always top-left origin. Desktop and UI coordinates must be
/// converted through DisplayMap before they enter an image document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}
impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
#[derive(Clone, Copy, Debug)]
pub enum SelectionHandle {
    NorthWest,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
}
pub fn resize_selection(r: Rect, handle: SelectionHandle, p: Point, bounds: Rect) -> Rect {
    use SelectionHandle::*;
    let (mut left, mut top, mut right, mut bottom) = (r.x, r.y, r.right(), r.bottom());
    if matches!(handle, NorthWest | West | SouthWest) {
        left = p.x.clamp(bounds.x, (right - 1.).max(bounds.x));
    }
    if matches!(handle, NorthEast | East | SouthEast) {
        right = p.x.clamp((left + 1.).min(bounds.right()), bounds.right());
    }
    if matches!(handle, NorthWest | North | NorthEast) {
        top = p.y.clamp(bounds.y, (bottom - 1.).max(bounds.y));
    }
    if matches!(handle, SouthWest | South | SouthEast) {
        bottom = p.y.clamp((top + 1.).min(bounds.bottom()), bounds.bottom());
    }
    Rect::new(left, top, right - left, bottom - top)
}
#[cfg(test)]
mod selection_tests {
    use super::*;
    #[test]
    fn resize_keeps_opposite_edges_and_clamps_to_screen() {
        let bounds = Rect::new(0., 0., 400., 300.);
        let r = Rect::new(80., 60., 120., 100.);
        let enlarged = resize_selection(
            r,
            SelectionHandle::SouthEast,
            Point::new(450., 290.),
            bounds,
        );
        assert_eq!(enlarged, Rect::new(80., 60., 320., 230.));
        let collapsed = resize_selection(
            r,
            SelectionHandle::NorthWest,
            Point::new(250., 200.),
            bounds,
        );
        assert_eq!(collapsed, Rect::new(199., 159., 1., 1.));
        let wider = resize_selection(r, SelectionHandle::West, Point::new(-20., 120.), bounds);
        assert_eq!(wider, Rect::new(0., 60., 200., 100.));
    }
}
impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    pub fn from_points(a: Point, b: Point) -> Self {
        Self::new(
            a.x.min(b.x),
            a.y.min(b.y),
            (a.x - b.x).abs(),
            (a.y - b.y).abs(),
        )
    }
    pub fn right(self) -> f32 {
        self.x + self.width
    }
    pub fn bottom(self) -> f32 {
        self.y + self.height
    }
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.right() && p.y < self.bottom()
    }
    pub fn intersect(self, b: Self) -> Self {
        let x = self.x.max(b.x);
        let y = self.y.max(b.y);
        Self::new(
            x,
            y,
            (self.right().min(b.right()) - x).max(0.),
            (self.bottom().min(b.bottom()) - y).max(0.),
        )
    }
    pub fn pixel_bounds(self, width: u32, height: u32) -> (u32, u32, u32, u32) {
        let r = self.intersect(Self::new(0., 0., width as f32, height as f32));
        let x = r.x.floor().clamp(0., width as f32) as u32;
        let y = r.y.floor().clamp(0., height as f32) as u32;
        (
            x,
            y,
            (r.right().ceil() as u32).min(width).saturating_sub(x),
            (r.bottom().ceil() as u32).min(height).saturating_sub(y),
        )
    }
    pub fn translate(self, dx: f32, dy: f32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.width, self.height)
    }
}

/// One mapping per display: never multiply virtual-desktop origins by DPI.
#[derive(Clone, Copy, Debug)]
pub struct DisplayMap {
    pub desktop: Rect,
    pub pixel_width: u32,
    pub pixel_height: u32,
}
impl DisplayMap {
    pub fn to_image(self, p: Point) -> Point {
        Point::new(
            (p.x - self.desktop.x) * self.pixel_width as f32 / self.desktop.width,
            (p.y - self.desktop.y) * self.pixel_height as f32 / self.desktop.height,
        )
    }
    pub fn to_desktop(self, p: Point) -> Point {
        Point::new(
            self.desktop.x + p.x * self.desktop.width / self.pixel_width as f32,
            self.desktop.y + p.y * self.desktop.height / self.pixel_height as f32,
        )
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ImageTransform {
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub quarter_turns: u8,
    pub flip_x: bool,
    pub flip_y: bool,
}
impl ImageTransform {
    pub fn forward(self, mut p: Point) -> Point {
        if self.flip_x {
            p.x = self.width - p.x;
        }
        if self.flip_y {
            p.y = self.height - p.y;
        }
        p = match self.quarter_turns % 4 {
            1 => Point::new(self.height - p.y, p.x),
            2 => Point::new(self.width - p.x, self.height - p.y),
            3 => Point::new(p.y, self.width - p.x),
            _ => p,
        };
        Point::new(p.x * self.scale, p.y * self.scale)
    }
    pub fn inverse(self, p: Point) -> Point {
        let p = Point::new(p.x / self.scale, p.y / self.scale);
        let mut p = match self.quarter_turns % 4 {
            1 => Point::new(p.y, self.height - p.x),
            2 => Point::new(self.width - p.x, self.height - p.y),
            3 => Point::new(self.width - p.y, p.x),
            _ => p,
        };
        if self.flip_x {
            p.x = self.width - p.x;
        }
        if self.flip_y {
            p.y = self.height - p.y;
        }
        p
    }
}
pub fn snap_at(p: Point, windows: &[Rect], display: Rect) -> Rect {
    windows
        .iter()
        .find(|r| r.contains(p))
        .copied()
        .unwrap_or(display)
        .intersect(display)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reverse_drag_and_crop() {
        let r = Rect::from_points(Point::new(40., 30.), Point::new(-4., 2.));
        assert_eq!(r.pixel_bounds(32, 20), (0, 2, 32, 18));
    }
    #[test]
    fn mixed_dpi_negative_origin() {
        let m = DisplayMap {
            desktop: Rect::new(-1920., -200., 1920., 1080.),
            pixel_width: 3840,
            pixel_height: 2160,
        };
        let p = Point::new(-1800., 100.);
        assert_eq!(m.to_desktop(m.to_image(p)), p);
        assert_eq!(m.to_image(p), Point::new(240., 600.));
    }
    #[test]
    fn rotation_roundtrip() {
        for r in 0..4 {
            for f in [false, true] {
                let t = ImageTransform {
                    width: 800.,
                    height: 600.,
                    scale: 0.63,
                    quarter_turns: r,
                    flip_x: f,
                    flip_y: !f,
                };
                let p = Point::new(100., 210.);
                let q = t.inverse(t.forward(p));
                assert!((q.x - p.x).abs() < 0.001 && (q.y - p.y).abs() < 0.001);
            }
        }
    }
    #[test]
    fn topmost_window_wins() {
        let a = Rect::new(10., 10., 50., 40.);
        assert_eq!(
            snap_at(
                Point::new(20., 20.),
                &[a, Rect::new(0., 0., 100., 100.)],
                Rect::new(0., 0., 200., 200.)
            ),
            a
        );
    }
}

/// Toolbar follows the image's bottom-right corner, using its lower inside edge
/// when the display has no room below. All values share one logical space.
pub fn place_toolbar(image: Rect, screen: Rect, width: f32, height: f32) -> Rect {
    let width = width.min((screen.width - 16.).max(1.));
    let height = height.min((screen.height - 12.).max(1.));
    let x = (image.right() - width).clamp(
        screen.x + 8.,
        (screen.right() - width - 8.).max(screen.x + 8.),
    );
    let below = image.bottom() + 6.;
    let y = if below + height <= screen.bottom() - 6. {
        below
    } else {
        (image.bottom() - height - 6.).clamp(
            screen.y + 6.,
            (screen.bottom() - height - 6.).max(screen.y + 6.),
        )
    };
    Rect::new(x, y, width, height)
}
#[cfg(test)]
mod toolbar_tests {
    use super::*;
    #[test]
    fn follows_lower_right_and_falls_inside() {
        let screen = Rect::new(0., 0., 1440., 900.);
        assert_eq!(
            place_toolbar(Rect::new(200., 100., 800., 400.), screen, 520., 40.),
            Rect::new(480., 506., 520., 40.)
        );
        assert_eq!(
            place_toolbar(Rect::new(200., 500., 800., 400.), screen, 520., 72.),
            Rect::new(480., 822., 520., 72.)
        );
    }
    #[test]
    fn small_image_and_negative_monitor_origin() {
        let screen = Rect::new(-1440., -100., 1440., 900.);
        let result = place_toolbar(Rect::new(-1400., 400., 100., 100.), screen, 520., 72.);
        assert_eq!(result.x, -1432.);
        assert_eq!(result.y, 506.);
        assert_eq!(result.intersect(screen), result);
    }
}
