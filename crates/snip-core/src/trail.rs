use crate::geometry::Point;
use std::collections::VecDeque;
#[derive(Clone, Copy)]
pub struct TrailPoint {
    pub point: Point,
    pub time: f64,
}
pub struct Trail {
    pub points: VecDeque<TrailPoint>,
    pub lifetime: f64,
    last_position: Option<Point>,
}
impl Trail {
    pub fn new(lifetime: f64) -> Self {
        Self {
            points: VecDeque::with_capacity(256),
            lifetime,
            last_position: None,
        }
    }
    pub fn update(&mut self, p: Point, time: f64) {
        while self
            .points
            .front()
            .is_some_and(|p| time - p.time > self.lifetime)
        {
            self.points.pop_front();
        }
        if self
            .last_position
            .is_none_or(|last| (last.x - p.x).abs() + (last.y - p.y).abs() > 0.5)
        {
            if self.points.len() == 256 {
                self.points.pop_front();
            }
            self.points.push_back(TrailPoint { point: p, time });
            self.last_position = Some(p);
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct TrailCurve {
    pub start: Point,
    pub control_a: Point,
    pub control_b: Point,
    pub end: Point,
    pub time: f64,
    pub start_time: f64,
}
#[derive(Clone, Copy, Debug)]
pub struct RibbonPoint {
    pub point: Point,
    pub normal: Point,
    pub time: f64,
}
impl Trail {
    /// A bounded, shared-edge ribbon. Every join reuses one position, tangent,
    /// timestamp and width, instead of separately colored constant-width strokes.
    pub fn ribbon(&self) -> Vec<RibbonPoint> {
        let mut out = Vec::with_capacity(self.points.len() * 8);
        for curve in self.curves() {
            let length = [
                (curve.start, curve.control_a),
                (curve.control_a, curve.control_b),
                (curve.control_b, curve.end),
            ]
            .iter()
            .map(|(a, b)| (b.x - a.x).hypot(b.y - a.y))
            .sum::<f32>();
            let steps = (length / 6.).ceil().clamp(1., 64.) as usize;
            for i in 0..=steps {
                if i == 0 && !out.is_empty() {
                    continue;
                }
                let t = i as f32 / steps as f32;
                let u = 1. - t;
                let [a, b, c, d] = [curve.start, curve.control_a, curve.control_b, curve.end];
                let point = Point::new(
                    u * u * u * a.x + 3. * u * u * t * b.x + 3. * u * t * t * c.x + t * t * t * d.x,
                    u * u * u * a.y + 3. * u * u * t * b.y + 3. * u * t * t * c.y + t * t * t * d.y,
                );
                let dx =
                    3. * u * u * (b.x - a.x) + 6. * u * t * (c.x - b.x) + 3. * t * t * (d.x - c.x);
                let dy =
                    3. * u * u * (b.y - a.y) + 6. * u * t * (c.y - b.y) + 3. * t * t * (d.y - c.y);
                let length = dx.hypot(dy);
                let normal = if length > 0.001 {
                    Point::new(-dy / length, dx / length)
                } else {
                    out.last()
                        .map_or(Point::new(0., 1.), |p: &RibbonPoint| p.normal)
                };
                out.push(RibbonPoint {
                    point,
                    normal,
                    time: curve.start_time + (curve.time - curve.start_time) * t as f64,
                });
            }
        }
        out
    }
    /// Catmull-Rom converted to cubic Beziers: neighboring curves share tangents.
    pub fn curves(&self) -> Vec<TrailCurve> {
        let mut curves = Vec::with_capacity(self.points.len().saturating_sub(1));
        for i in 0..self.points.len().saturating_sub(1) {
            let p0 = self.points[i.saturating_sub(1)].point;
            let p1 = self.points[i].point;
            let p2 = self.points[i + 1].point;
            let p3 = self.points[(i + 2).min(self.points.len() - 1)].point;
            curves.push(TrailCurve {
                start: p1,
                end: p2,
                time: self.points[i + 1].time,
                start_time: self.points[i].time,
                control_a: Point::new(p1.x + (p2.x - p0.x) / 6., p1.y + (p2.y - p0.y) / 6.),
                control_b: Point::new(p2.x - (p3.x - p1.x) / 6., p2.y - (p3.y - p1.y) / 6.),
            });
        }
        curves
    }
}
pub fn rainbow(h: f32) -> [u8; 3] {
    let h = h.rem_euclid(1.) * 6.;
    let x = 1. - (h % 2. - 1.).abs();
    let (r, g, b) = match h as u32 {
        0 => (1., x, 0.),
        1 => (x, 1., 0.),
        2 => (0., 1., x),
        3 => (0., x, 1.),
        4 => (x, 0., 1.),
        _ => (1., 0., x),
    };
    [(r * 255.) as u8, (g * 255.) as u8, (b * 255.) as u8]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expires_and_bounds() {
        let mut t = Trail::new(0.5);
        for i in 0..1000 {
            t.update(Point::new(i as f32, 0.), i as f64 / 1000.);
        }
        assert!(t.points.len() <= 256);
        t.update(Point::new(999., 0.), 2.);
        assert!(t.points.len() <= 1);
    }
    #[test]
    fn stationary_cursor_does_not_restart_trail() {
        let mut trail = Trail::new(0.5);
        trail.update(Point::new(20., 20.), 0.);
        trail.update(Point::new(20., 20.), 1.);
        assert!(trail.points.is_empty());
        trail.update(Point::new(20., 20.), 2.);
        assert!(trail.points.is_empty());
    }
}

#[cfg(test)]
mod curve_tests {
    use super::*;
    #[test]
    fn smooth_curves_share_endpoints_and_tangents() {
        let mut t = Trail::new(2.);
        for (i, p) in [
            Point::new(0., 0.),
            Point::new(20., 30.),
            Point::new(60., 10.),
            Point::new(80., 80.),
        ]
        .into_iter()
        .enumerate()
        {
            t.update(p, i as f64 * 0.1);
        }
        let c = t.curves();
        assert_eq!(c.len(), 3);
        for pair in c.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
            let left = pair[0].control_b;
            let right = pair[1].control_a;
            let join = pair[0].end;
            assert!(((join.x - left.x) - (right.x - join.x)).abs() < 0.0001);
            assert!(((join.y - left.y) - (right.y - join.y)).abs() < 0.0001);
        }
        let ribbon = t.ribbon();
        assert!(ribbon.len() > c.len());
        assert!(ribbon.len() <= 256 * 64 + 1);
        for pair in ribbon.windows(2) {
            assert!(pair[1].time >= pair[0].time);
            assert!((pair[1].normal.x.hypot(pair[1].normal.y) - 1.).abs() < 0.001);
        }
        assert_eq!(ribbon.first().unwrap().point, Point::new(0., 0.));
        assert_eq!(ribbon.last().unwrap().point, Point::new(80., 80.));
    }
}
