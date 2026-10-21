//! Pure geometry types used by the overlay and capture pipeline.
//!
//! All coordinates are physical pixels. Rectangles are half-open: right and
//! bottom are exclusive, so width == right - left.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub const fn width(self) -> i32 {
        self.right - self.left
    }

    pub const fn height(self) -> i32 {
        self.bottom - self.top
    }

    pub const fn is_empty(self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    /// Build a normalized rectangle from two drag endpoints.
    pub fn from_points(a: Point, b: Point) -> Self {
        Self {
            left: a.x.min(b.x),
            top: a.y.min(b.y),
            right: a.x.max(b.x),
            bottom: a.y.max(b.y),
        }
    }

    /// Clamp this rectangle to `bounds`; may return an empty rectangle.
    pub fn clamp_to(self, bounds: Rect) -> Self {
        let left = self.left.clamp(bounds.left, bounds.right);
        let top = self.top.clamp(bounds.top, bounds.bottom);
        let right = self.right.clamp(bounds.left, bounds.right);
        let bottom = self.bottom.clamp(bounds.top, bounds.bottom);
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn contains(self, point: Point) -> bool {
        point.x >= self.left && point.x < self.right && point.y >= self.top && point.y < self.bottom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_points_normalizes_reverse_drag() {
        let rect = Rect::from_points(Point::new(100, 80), Point::new(20, 10));
        assert_eq!(rect, Rect::new(20, 10, 100, 80));
        assert_eq!(rect.width(), 80);
        assert_eq!(rect.height(), 70);
    }

    #[test]
    fn from_points_zero_size_is_empty() {
        let rect = Rect::from_points(Point::new(5, 5), Point::new(5, 5));
        assert!(rect.is_empty());
        assert_eq!(rect.width(), 0);
        assert_eq!(rect.height(), 0);
    }

    #[test]
    fn clamp_to_keeps_inside_bounds() {
        let rect = Rect::new(-10, -20, 200, 300);
        let clipped = rect.clamp_to(Rect::new(0, 0, 1920, 1080));
        assert_eq!(clipped, Rect::new(0, 0, 200, 300));
    }

    #[test]
    fn clamp_to_fully_outside_is_empty() {
        let rect = Rect::new(2000, 1200, 2100, 1300);
        let clipped = rect.clamp_to(Rect::new(0, 0, 1920, 1080));
        assert!(clipped.is_empty());
    }

    #[test]
    fn contains_uses_half_open_edges() {
        let rect = Rect::new(10, 10, 20, 20);
        assert!(rect.contains(Point::new(10, 10)));
        assert!(rect.contains(Point::new(19, 19)));
        assert!(!rect.contains(Point::new(20, 20)));
        assert!(!rect.contains(Point::new(9, 10)));
    }
}
