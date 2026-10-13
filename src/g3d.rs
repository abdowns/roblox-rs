//! Exact ports of the G3D helpers the animation and joint code use, with the
//! binary's operation order and float/double conversions.

/// G3D::fuzzyEq(x, 0): eps 1e-5 (as float) scaled by |x|+1, in double.
pub fn fuzzy_zero(x: f32) -> bool {
    if x == 0.0 {
        return true;
    }
    let a = f64::from(x.abs());
    a <= (a + 1.0) * f64::from(1e-5f32)
}

/// G3D::Vector3::unitize(tolerance) 012590ea: normalizes when the length
/// exceeds the tolerance and returns the length, else leaves it and returns 0.
pub fn unitize(v: &mut [f32; 3], tolerance: f32) -> f32 {
    let length = (v[2] * v[2] + v[1] * v[1] + v[0] * v[0]).sqrt();
    if tolerance < length {
        let inverse = 1.0 / length;
        *v = [v[0] * inverse, v[1] * inverse, inverse * v[2]];
        length
    } else {
        0.0
    }
}

/// G3D::Matrix3::toAxisAngle 01255f14 with DFFlag::FixMatrixToAxisAngle at its
/// built-in default (true, initialized data byte 01d6ba58).
pub fn to_axis_angle(m: &[[f32; 3]; 3]) -> ([f32; 3], f32) {
    const PI_F: f32 = std::f32::consts::PI;
    let c = (m[0][0] + m[1][1] + m[2][2] + -1.0) * 0.5;
    let angle = if -1.0 < c {
        if 1.0 <= c {
            return ([1.0, 0.0, 0.0], 0.0);
        }
        f64::from(c).acos() as f32
    } else {
        PI_F
    };
    if angle <= 0.0001 {
        return ([1.0, 0.0, 0.0], angle);
    }
    if f64::from(angle) < f64::from_bits(0x400921c6e67e6d28) {
        let mut axis = [m[2][1] - m[1][2], m[0][2] - m[2][0], m[1][0] - m[0][1]];
        unitize(&mut axis, 0.0);
        return (axis, angle);
    }
    let (xx, yy, zz) = (m[0][0], m[1][1], m[2][2]);
    let root = |v: f32| ((f64::from(v) + 1.0).sqrt() * 0.5) as f32;
    let axis = if xx >= yy {
        if xx >= zz {
            let s = root((xx - yy) - zz);
            let inverse = 0.5 / s;
            [s, m[0][1] * inverse, inverse * m[0][2]]
        } else {
            let s = root((zz - xx) - yy);
            let inverse = 0.5 / s;
            [m[0][2] * inverse, inverse * m[1][2], s]
        }
    } else if yy < zz {
        let s = root((zz - xx) - yy);
        let inverse = 0.5 / s;
        [m[0][2] * inverse, inverse * m[1][2], s]
    } else {
        let s = root((yy - xx) - zz);
        let inverse = 0.5 / s;
        [m[0][1] * inverse, s, inverse * m[1][2]]
    };
    (axis, angle)
}

/// G3D::Matrix3::fromAxisAngleFast 01256164: (float)cos/sin of the double angle.
pub fn from_axis_angle_fast(axis: [f32; 3], angle: f32) -> [[f32; 3]; 3] {
    let c = f64::from(angle).cos() as f32;
    let s = f64::from(angle).sin() as f32;
    let t = 1.0 - c;
    let [x, y, z] = axis;
    let xy = (x * y) * t;
    let xz = (x * z) * t;
    let yz = (y * z) * t;
    let (xs, ys, zs) = (x * s, s * y, s * z);
    [
        [(x * x) * t + c, xy - zs, ys + xz],
        [zs + xy, (y * y) * t + c, yz - xs],
        [xz - ys, xs + yz, (z * z) * t + c],
    ]
}

/// Rotation from an axis*angle vector the way Motor6DJoint links build it
/// (00b36fde): unitize with 1e-6, then fromAxisAngleFast.
pub fn rotation_vector_matrix(rotation: [f32; 3]) -> [[f32; 3]; 3] {
    let mut axis = rotation;
    let angle = unitize(&mut axis, 1e-6);
    from_axis_angle_fast(axis, angle)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn axis_angle_round_trip() {
        let m = from_axis_angle_fast([0.0, 0.6, 0.8], 1.2);
        let (axis, angle) = to_axis_angle(&m);
        assert!((angle - 1.2).abs() < 1e-5);
        assert!((axis[1] - 0.6).abs() < 1e-5 && (axis[2] - 0.8).abs() < 1e-5);
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        assert_eq!(to_axis_angle(&identity), ([1.0, 0.0, 0.0], 0.0));
        let half = from_axis_angle_fast([0.0, 1.0, 0.0], std::f32::consts::PI);
        let (axis, angle) = to_axis_angle(&half);
        assert!((angle - std::f32::consts::PI).abs() < 1e-3 && (axis[1] - 1.0).abs() < 1e-3);
    }
    #[test]
    fn rotation_vector_z_matches_motor_angle() {
        let m = rotation_vector_matrix([0.0, 0.0, -0.5]);
        assert!((m[0][1] - 0.5f32.sin()).abs() < 1e-6);
        assert_eq!(rotation_vector_matrix([0.0; 3])[0][0], 1.0);
    }
}
