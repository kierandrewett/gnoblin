//! Points, directions, rays and planes: `data/js/gnoblin-core/math/vector.ts`, in the same arithmetic.

/// A point or a direction in three dimensions.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Vec3 {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) z: f64,
}

impl Vec3 {
    pub(crate) const fn new(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3 { x, y, z }
    }

    /// A vector from a list, with zero for a coordinate the list does not have.
    pub(crate) fn of(parts: &[f64]) -> Vec3 {
        let at = |index: usize| parts.get(index).copied().unwrap_or(0.0);
        Vec3::new(at(0), at(1), at(2))
    }

    pub(crate) fn array(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }

    pub(crate) fn add(self, other: Vec3) -> Vec3 {
        Vec3::new(self.x + other.x, self.y + other.y, self.z + other.z)
    }

    pub(crate) fn subtract(self, other: Vec3) -> Vec3 {
        Vec3::new(self.x - other.x, self.y - other.y, self.z - other.z)
    }

    pub(crate) fn scale(self, factor: f64) -> Vec3 {
        Vec3::new(self.x * factor, self.y * factor, self.z * factor)
    }

    pub(crate) fn dot(self, other: Vec3) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub(crate) fn cross(self, other: Vec3) -> Vec3 {
        Vec3::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    pub(crate) fn length(self) -> f64 {
        self.x.hypot(self.y).hypot(self.z)
    }

    /// The unit vector the same way, or zero for a zero vector.
    pub(crate) fn normalize(self) -> Vec3 {
        let length = self.length();
        if length == 0.0 {
            Vec3::default()
        } else {
            self.scale(1.0 / length)
        }
    }

    pub(crate) fn distance(self, other: Vec3) -> f64 {
        self.subtract(other).length()
    }
}

/// A half line from `origin` along `direction`, which need not be a unit vector.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Line {
    pub(crate) origin: Vec3,
    pub(crate) direction: Vec3,
}

impl Line {
    /// The point `distance` along the line, in lengths of `direction`.
    pub(crate) fn at(self, distance: f64) -> Vec3 {
        self.origin.add(self.direction.scale(distance))
    }
}

/// A plane through `point` with a normal vector.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Flat {
    pub(crate) point: Vec3,
    pub(crate) normal: Vec3,
}

impl Flat {
    /// Where the line crosses the plane, and how far along it: none when it is parallel to it or the plane is behind its start.
    pub(crate) fn intersect(self, line: Line) -> Option<(Vec3, f64)> {
        let denominator = self.normal.dot(line.direction);
        if denominator.abs() < 1e-12 {
            return None;
        }
        let t = self.normal.dot(self.point.subtract(line.origin)) / denominator;
        (t > 0.0).then(|| (line.at(t), t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_vector_normalises_to_zero() {
        assert_eq!(Vec3::default().normalize(), Vec3::default());
        assert!((Vec3::new(3.0, 0.0, 4.0).normalize().length() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_missing_coordinate_is_zero() {
        assert_eq!(Vec3::of(&[1.0]), Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(Vec3::of(&[1.0, 2.0, 3.0, 4.0]), Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn a_plane_behind_the_start_or_parallel_is_missed() {
        let plane = Flat {
            point: Vec3::new(0.0, 0.0, -2.0),
            normal: Vec3::new(0.0, 0.0, 1.0),
        };
        let line = |origin: [f64; 3], direction: [f64; 3]| Line {
            origin: Vec3::of(&origin),
            direction: Vec3::of(&direction),
        };
        assert_eq!(
            plane
                .intersect(line([0.0, 0.0, 0.0], [0.0, 0.0, -1.0]))
                .map(|hit| hit.0),
            Some(Vec3::new(0.0, 0.0, -2.0))
        );
        assert!(plane.intersect(line([0.0, 0.0, 0.0], [1.0, 0.0, 0.0])).is_none());
        assert!(plane.intersect(line([0.0, 0.0, -5.0], [0.0, 0.0, -1.0])).is_none());
    }
}
