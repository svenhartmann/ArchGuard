//! Camera moves of the buttons and keys (`worldCameraTo` in
//! `viewer_world.js`): 320 ms cubic in-out, turning the shorter way.
//! Drags and the wheel move the camera at once.

use std::f64::consts::{PI, TAU};
use std::time::Duration;

use super::scene::Camera;

pub const DURATION: Duration = Duration::from_millis(320);

/// `worldEase`: cubic in-out on [0, 1].
pub fn ease(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// The turn from `from` to `to` the shorter way, in [-π, π).
pub fn shortest(from: f64, to: f64) -> f64 {
    (to - from + PI).rem_euclid(TAU) - PI
}

/// A move from one pose to another.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glide {
    pub from: Camera,
    pub to: Camera,
}

impl Glide {
    /// The pose at `t` (0 start, 1 end); the rotation lands in [0, 2π).
    pub fn at(&self, t: f64) -> Camera {
        let e = ease(t);
        let mix = |a: f64, b: f64| a + (b - a) * e;
        let turn = shortest(self.from.rotation, self.to.rotation);
        Camera {
            x: mix(self.from.x, self.to.x),
            y: mix(self.from.y, self.to.y),
            zoom: mix(self.from.zoom, self.to.zoom),
            rotation: (self.from.rotation + turn * e).rem_euclid(TAU),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curve_is_cubic_in_out() {
        assert_eq!((ease(0.0), ease(0.5), ease(1.0)), (0.0, 0.5, 1.0));
        assert!((ease(0.25) - 0.0625).abs() < 1e-12);
        assert!((ease(0.75) - 0.9375).abs() < 1e-12);
        assert_eq!((ease(-1.0), ease(2.0)), (0.0, 1.0));
    }

    #[test]
    fn turns_take_the_shorter_way() {
        let deg = f64::to_radians;
        assert!((shortest(deg(350.0), deg(10.0)) - deg(20.0)).abs() < 1e-12);
        assert!((shortest(deg(10.0), deg(350.0)) + deg(20.0)).abs() < 1e-12);
        assert!((shortest(0.0, deg(90.0)) - deg(90.0)).abs() < 1e-12);
    }

    /// From 345° to 15° through 0°, landing exactly on the target.
    #[test]
    fn a_glide_ends_on_its_target() {
        let deg = f64::to_radians;
        let from = Camera {
            rotation: deg(345.0),
            ..Camera::default()
        };
        let to = Camera {
            x: 40.0,
            zoom: 2.0,
            rotation: deg(15.0),
            ..Camera::default()
        };
        let g = Glide { from, to };
        assert_eq!(g.at(0.0), from);
        let end = g.at(1.0);
        assert!((end.rotation - to.rotation).abs() < 1e-12);
        assert_eq!((end.x, end.zoom), (40.0, 2.0));
        let mid = g.at(0.5);
        assert!(
            mid.rotation < 1e-9 || (TAU - mid.rotation) < 1e-9,
            "{mid:?}"
        );
        assert_eq!((mid.x, mid.zoom), (20.0, 1.5));
    }
}
