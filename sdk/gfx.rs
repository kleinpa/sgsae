//! Drawing shapes that are defined somewhere other than where they land.
//!
//! The PPU is a bitmap and nothing else — no sprites, no tilemaps — so a game
//! draws every pixel itself. That is a liberty rather than a limitation: a
//! shape here is a list of points, and where it ends up on screen is a
//! [`Transform`] applied on the way out.
//!
//! ```no_run
//! # use console::gfx::{Shape, Transform};
//! # use console::fixed::{Angle, Fx, Vec2};
//! # fn draw(video: &mut console::video::Video) {
//! /// A ship, nose up, about the origin it turns around.
//! const SHIP: Shape = Shape::closed(&[Vec2::int(0, -6), Vec2::int(4, 5), Vec2::int(-4, 5)]);
//!
//! SHIP.draw(video, &Transform::at(Vec2::int(120, 80)).turned(Angle::from_degrees(30)), 1);
//! # }
//! ```
//!
//! Keeping the model still and moving the transform is what makes the same
//! geometry serve for the thing, its shadow, and its icon in the corner — and
//! it is the same arrangement a wireframe renderer wants, with a projection
//! standing in for the 2D transform.

use crate::fixed::{Angle, Fx, Vec2};
use crate::video::Video;

/// Where a shape goes: a rotation and a scale about its own origin, then a
/// translation.
///
/// Built by chaining, so a call site reads as the sentence it means:
/// `Transform::at(position).turned(heading).scaled(Fx::HALF)`.
#[derive(Debug, Clone, Copy)]
pub struct Transform {
    pub position: Vec2,
    pub rotation: Angle,
    pub scale: Fx,
}

impl Default for Transform {
    fn default() -> Transform {
        Transform::IDENTITY
    }
}

impl Transform {
    /// Leaves a shape exactly where its model puts it.
    pub const IDENTITY: Transform =
        Transform { position: Vec2::ZERO, rotation: Angle::ZERO, scale: Fx::ONE };

    /// Place a shape, unrotated and unscaled.
    pub const fn at(position: Vec2) -> Transform {
        Transform { position, rotation: Angle::ZERO, scale: Fx::ONE }
    }

    /// Place a shape at whole-pixel coordinates.
    pub const fn at_pixels(x: i32, y: i32) -> Transform {
        Transform::at(Vec2::int(x, y))
    }

    pub const fn turned(self, rotation: Angle) -> Transform {
        Transform { rotation, ..self }
    }

    pub const fn scaled(self, scale: Fx) -> Transform {
        Transform { scale, ..self }
    }

    /// Map a point from model space to the screen.
    pub fn apply(&self, point: Vec2) -> Vec2 {
        let scaled =
            if self.scale == Fx::ONE { point } else { point.scale(self.scale) };
        let turned = if self.rotation == Angle::ZERO { scaled } else { scaled.rotate(self.rotation) };
        turned + self.position
    }

    /// Map a point and round it to a pixel.
    pub fn pixel(&self, point: Vec2) -> (i32, i32) {
        self.apply(point).to_pixels()
    }
}

/// A run of points, drawn as connected line segments.
///
/// The points live in the shape's own space, so a model is written once — at
/// the origin, at a natural size — and placed by a [`Transform`]. Being a
/// borrow of a slice, a `Shape` is `const`-constructible and costs no RAM: the
/// geometry stays in ROM where a console of this era would have kept it.
#[derive(Debug, Clone, Copy)]
pub struct Shape<'a> {
    pub points: &'a [Vec2],
    /// Whether the last point joins back to the first.
    pub closed: bool,
}

impl<'a> Shape<'a> {
    /// An open path: a stroke from the first point to the last.
    pub const fn open(points: &'a [Vec2]) -> Shape<'a> {
        Shape { points, closed: false }
    }

    /// A closed outline.
    pub const fn closed(points: &'a [Vec2]) -> Shape<'a> {
        Shape { points, closed: true }
    }

    /// Draw the shape through `transform`.
    ///
    /// Points are transformed as they are consumed, one at a time, so nothing
    /// is buffered and a shape may be any length — there is no scratch array
    /// to size, which on a machine with 64 KiB of RAM is the difference
    /// between a primitive that always works and one with a limit to remember.
    pub fn draw(&self, video: &mut Video, transform: &Transform, colour: u8) {
        let Some(&first) = self.points.first() else {
            return;
        };
        let start = transform.pixel(first);
        let mut previous = start;
        for &point in &self.points[1..] {
            let next = transform.pixel(point);
            video.line(previous.0, previous.1, next.0, next.1, colour);
            previous = next;
        }
        if self.closed && self.points.len() > 2 {
            video.line(previous.0, previous.1, start.0, start.1, colour);
        }
    }
}

/// Draw a run of disconnected segments, each a pair of points, through
/// `transform`.
///
/// For geometry that is not one stroke — the struts of an airframe, a set of
/// tick marks — where a [`Shape`] would have to double back on itself.
pub fn segments(
    video: &mut Video,
    transform: &Transform,
    segments: &[(Vec2, Vec2)],
    colour: u8,
) {
    for &(from, to) in segments {
        let (x0, y0) = transform.pixel(from);
        let (x1, y1) = transform.pixel(to);
        video.line(x0, y0, x1, y1, colour);
    }
}

/// A dashed line, `on` pixels drawn and `off` pixels skipped.
///
/// Enough to distinguish a projected path or a horizon from a solid edge,
/// which is most of what a one-colour display has to work with.
pub fn dashed(
    video: &mut Video,
    from: (i32, i32),
    to: (i32, i32),
    on: i32,
    off: i32,
    colour: u8,
) {
    let period = (on + off).max(1);
    let span = Vec2::int(to.0 - from.0, to.1 - from.1);
    let length = span.length().round().max(1);
    let mut at = 0;
    while at < length {
        let end = (at + on).min(length);
        // Walking the parameter rather than the pixels keeps a dash the same
        // length whatever the line's slope.
        let point = |d: i32| {
            (from.0 + (to.0 - from.0) * d / length, from.1 + (to.1 - from.1) * d / length)
        };
        let (x0, y0) = point(at);
        let (x1, y1) = point(end);
        video.line(x0, y0, x1, y1, colour);
        at += period;
    }
}
