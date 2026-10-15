//! Exact ports of the G3D helpers the animation and joint code use, with the
//! binary's operation order and float/double conversions.
use crate::scene::CoordinateFrame;

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

/// G3D::Matrix3::operator* 0125581a: (x + y) + z per element.
pub fn matrix_multiply(a: &[[f32; 3]; 3], b: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    std::array::from_fn(|r| {
        std::array::from_fn(|c| (a[r][0] * b[0][c] + a[r][1] * b[1][c]) + a[r][2] * b[2][c])
    })
}

/// CoordinateFrame multiply 00972080: the rotation product, and
/// ((x + y) + z) + translation for the point.
pub fn frame_multiply(a: &CoordinateFrame, b: &CoordinateFrame) -> CoordinateFrame {
    let p = b.position;
    CoordinateFrame {
        position: std::array::from_fn(|r| {
            ((a.rotation[r][0] * p[0] + a.rotation[r][1] * p[1]) + a.rotation[r][2] * p[2])
                + a.position[r]
        }),
        rotation: matrix_multiply(&a.rotation, &b.rotation),
    }
}

/// The rigid inverse built inline in 00b69dd6: the transposed rotation, and
/// the negated transpose times the translation, ((x + y) + z).
pub fn frame_inverse(a: &CoordinateFrame) -> CoordinateFrame {
    let rotation: [[f32; 3]; 3] =
        std::array::from_fn(|r| std::array::from_fn(|c| a.rotation[c][r]));
    let p = a.position;
    CoordinateFrame {
        position: std::array::from_fn(|r| {
            (-rotation[r][0] * p[0] + -rotation[r][1] * p[1]) + -rotation[r][2] * p[2]
        }),
        rotation,
    }
}

/// G3D::fuzzyEq as Matrix3::isOrthonormal 01255004 compiles it: equal floats
/// pass, otherwise |a - b| <= eps(|a| + 1) in double, with eps 1e-5 as float
/// (01834cf8) and the plain eps when |a| + 1 is infinite.
fn fuzzy_eq(a: f32, b: f32) -> bool {
    if a == b {
        return true;
    }
    let epsilon = f64::from(1e-5f32);
    let base = f64::from(a.abs()) + 1.0;
    let tolerance = if base == f64::INFINITY {
        epsilon
    } else {
        base * epsilon
    };
    (f64::from(a) - f64::from(b)).abs() <= tolerance
}

/// G3D::Matrix3::isOrthonormal 01255004: the column dot products are fuzzy
/// zero and the column lengths squared fuzzy one.
pub fn is_orthonormal(m: &[[f32; 3]; 3]) -> bool {
    let column = |c: usize| [m[0][c], m[1][c], m[2][c]];
    let dot = |a: [f32; 3], b: [f32; 3]| (a[0] * b[0] + a[1] * b[1]) + a[2] * b[2];
    let (x, y, z) = (column(0), column(1), column(2));
    fuzzy_eq(dot(x, y), 0.0)
        && fuzzy_eq(dot(y, z), 0.0)
        && fuzzy_eq(dot(x, z), 0.0)
        && fuzzy_eq(dot(x, x), 1.0)
        && fuzzy_eq(dot(y, y), 1.0)
        && fuzzy_eq(dot(z, z), 1.0)
}

/// G3D::Matrix3::orthonormalize 01255ca6: Gram-Schmidt on the columns, each
/// scale (float)(1 / sqrt((double)length²)).
pub fn orthonormalize(m: &mut [[f32; 3]; 3]) {
    let scale = |a: f32, b: f32, c: f32| (1.0 / f64::from((a * a + b * b) + c * c).sqrt()) as f32;
    let s = scale(m[0][0], m[1][0], m[2][0]);
    for row in m.iter_mut() {
        row[0] *= s;
    }
    let dot0 = (m[0][0] * m[0][1] + m[1][0] * m[1][1]) + m[2][0] * m[2][1];
    for row in m.iter_mut() {
        row[1] -= row[0] * dot0;
    }
    let s = scale(m[0][1], m[1][1], m[2][1]);
    for row in m.iter_mut() {
        row[1] *= s;
    }
    let dot1 = (m[0][1] * m[0][2] + m[1][1] * m[1][2]) + m[2][1] * m[2][2];
    let dot0 = (m[0][2] * m[0][0] + m[1][2] * m[1][0]) + m[2][2] * m[2][0];
    for row in m.iter_mut() {
        row[2] -= row[1] * dot1 + row[0] * dot0;
    }
    let s = scale(m[0][2], m[1][2], m[2][2]);
    for row in m.iter_mut() {
        row[2] *= s;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn orthonormal_checks() {
        let mut m = from_axis_angle_fast([0.0, 0.6, 0.8], 1.2);
        assert!(is_orthonormal(&m));
        m[0][0] += 0.01;
        assert!(!is_orthonormal(&m));
        orthonormalize(&mut m);
        assert!(is_orthonormal(&m));
        let frame = CoordinateFrame {
            position: [1.0, 2.0, 3.0],
            rotation: from_axis_angle_fast([1.0, 0.0, 0.0], 0.5),
        };
        let back = frame_multiply(&frame, &frame_inverse(&frame));
        assert!(back.position.iter().all(|v| v.abs() < 1e-6));
    }
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
