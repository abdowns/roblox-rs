//! The recovered RenderCamera perspective contract, independent of engine defaults.
use crate::scene::CoordinateFrame;

pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Result<[[f32; 4]; 4], String> {
    if ![fov_y, aspect, near, far].iter().all(|v| v.is_finite())
        || !(0.0..std::f32::consts::PI).contains(&fov_y)
        || fov_y == 0.0
        || aspect <= 0.0
        || near <= 0.0
        || far <= near
    {
        return Err("invalid camera projection".into());
    }
    let height = 1.0 / (fov_y * 0.5).tan();
    let z = -far / (far - near);
    Ok([
        [height / aspect, 0.0, 0.0, 0.0],
        [0.0, height, 0.0, 0.0],
        [0.0, 0.0, z, near * z],
        [0.0, 0.0, -1.0, 0.0],
    ])
}

pub fn view(frame: &CoordinateFrame) -> [[f32; 4]; 4] {
    let mut result = [[0.0; 4]; 4];
    for (row, output) in result[..3].iter_mut().enumerate() {
        for (column, value) in output[..3].iter_mut().enumerate() {
            *value = frame.rotation[column][row];
        }
        output[3] = -(output[0] * frame.position[0]
            + output[1] * frame.position[1]
            + output[2] * frame.position[2]);
    }
    result[3][3] = 1.0;
    result
}

pub fn multiply(a: [[f32; 4]; 4], b: [[f32; 4]; 4]) -> [[f32; 4]; 4] {
    std::array::from_fn(|row| {
        std::array::from_fn(|column| (0..4).map(|k| a[row][k] * b[k][column]).sum())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovered_projection_maps_near_and_far_to_zero_and_one() {
        let p = perspective(70.0f32.to_radians(), 16.0 / 9.0, 0.5, 5000.0).unwrap();
        for (z, expected) in [(-0.5, 0.0), (-5000.0, 1.0)] {
            let depth = (p[2][2] * z + p[2][3]) / -z;
            assert!((depth - expected).abs() < 0.000001);
        }
        assert!(perspective(0.0, 1.0, 0.5, 5000.0).is_err());
    }
    #[test]
    fn view_uses_inverse_rigid_frame() {
        let frame = CoordinateFrame {
            position: [5.0, 7.0, 11.0],
            rotation: [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]],
        };
        let p = frame.transform_point([2.0, 3.0, 4.0]);
        let v = view(&frame);
        assert_eq!(
            std::array::from_fn::<_, 3, _>(|row| v[row][0] * p[0]
                + v[row][1] * p[1]
                + v[row][2] * p[2]
                + v[row][3]),
            [2.0, 3.0, 4.0]
        );
    }
}
