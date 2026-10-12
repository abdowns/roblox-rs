//! Legacy BlockBlock contained-face geometry and awake connector impulses.
use crate::{
    assembly::{Tensor, multiply, rotate, transpose},
    bundle::Bundle,
    collision::{ContactBodyState, contact_velocity},
    kernel::{ContactParameters, ContactSpring, KernelBody, KernelProfile},
    native_data::NativeImage,
    scene::CoordinateFrame,
};
use std::fmt::Write;
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[2] * b[2] + a[1] * b[1] + a[0] * b[0]
}
fn mul_vector(m: Tensor, v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|a| m[a][2] * v[2] + (m[a][1] * v[1] + m[a][0] * v[0]))
}
fn inverse(m: Tensor) -> Result<Tensor, String> {
    // G3D::Matrix3::inverse 01255ab2, strict determinant tolerance.
    let [[a, b, c], [d, e, f], [g, h, i]] = m;
    let cof = [
        [e * i - f * h, c * h - b * i, b * f - c * e],
        [f * g - d * i, a * i - c * g, c * d - a * f],
        [d * h - e * g, b * g - a * h, a * e - b * d],
    ];
    let determinant = cof[2][0] * c + cof[1][0] * b + cof[0][0] * a;
    if !determinant.is_finite() || determinant.abs() <= 1e-20 {
        return Err("singular native contact mass matrix".into());
    }
    Ok(cof.map(|r| r.map(|v| v * (1.0 / determinant))))
}
fn skew([x, y, z]: [f32; 3]) -> Tensor {
    [[0.0, -z, y], [z, 0.0, -x], [-y, x, 0.0]]
}
#[derive(Clone, Debug)]
pub struct ContactGeometry {
    pub normal: [f32; 3],
    pub distance: f32,
    pub point: [f32; 3],
}
pub struct BlockContactProfile {
    faces: [[usize; 4]; 6],
    overlap_goal: f32,
}
fn vertices(size: [f32; 3]) -> [[f32; 3]; 8] {
    // 00ae07f2: X outer, Y middle, Z inner; positive signs first.
    std::array::from_fn(|i| {
        std::array::from_fn(|a| size[a] * 0.5 * if i & (4 >> a) == 0 { 1.0 } else { -1.0 })
    })
}
fn transform(f: &CoordinateFrame, p: [f32; 3]) -> [f32; 3] {
    let r = rotate(f.rotation, p);
    std::array::from_fn(|a| r[a] + f.position[a])
}
impl BlockContactProfile {
    /// Selected incident face against an explicit infinite plane fixture. Native
    /// face selection chooses the largest local normal component (006b2077).
    pub fn plane_vertices(
        &self,
        a: &CoordinateFrame,
        size: [f32; 3],
        b: &CoordinateFrame,
        face: usize,
    ) -> Result<Vec<(usize, ContactGeometry)>, String> {
        if size.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            return Err("invalid plane-contact box size".into());
        }
        let normal = self.vertex_plane(a, [0.0; 3], b, [0.0; 3], face)?.normal;
        let direction = rotate(transpose(a.rotation), normal);
        let axis = if direction[0].abs() <= direction[1].abs()
            && direction[2].abs() < direction[1].abs()
        {
            1
        } else if direction[0].abs() > direction[1].abs() && direction[2].abs() < direction[0].abs()
        {
            0
        } else {
            2
        };
        let incident = axis + if direction[axis] > 0.0 { 0 } else { 3 };
        let vertices = vertices(size);
        let mut result = Vec::new();
        for corner in (0..4).rev() {
            let index = self.faces[incident][corner];
            let g = self.vertex_plane(a, vertices[index], b, [0.0; 3], face)?;
            if g.distance < -self.overlap_goal {
                result.push((index, g));
            }
        }
        Ok(result)
    }
    pub fn load(bundle: &Bundle) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let data = image.region(0x017a3e68, 96)?;
        let faces = std::array::from_fn(|f| {
            std::array::from_fn(|i| {
                let at = (f * 4 + i) * 4;
                u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize
            })
        });
        let overlap_goal = -f32::from_le_bytes(image.region(0x017efdb0, 4)?.try_into().unwrap());
        if faces.iter().flatten().any(|v| *v >= 8)
            || !overlap_goal.is_finite()
            || overlap_goal <= 0.0
        {
            return Err("invalid native contact profile".into());
        }
        Ok(Self {
            faces,
            overlap_goal,
        })
    }
    /// GeoPair vertex-plane case 00aa1d78. Point is the original world vertex.
    pub fn vertex_plane(
        &self,
        a: &CoordinateFrame,
        vertex: [f32; 3],
        b: &CoordinateFrame,
        plane: [f32; 3],
        face: usize,
    ) -> Result<ContactGeometry, String> {
        if face >= 6
            || vertex
                .iter()
                .chain(&plane)
                .chain(&a.position)
                .chain(a.rotation.iter().flatten())
                .chain(&b.position)
                .chain(b.rotation.iter().flatten())
                .any(|v| !v.is_finite())
        {
            return Err("invalid vertex-plane geometry".into());
        }
        let axis = face % 3;
        let sign = if face < 3 { 1.0 } else { -1.0 };
        let normal = std::array::from_fn(|a| -b.rotation[a][axis] * sign);
        let point = transform(a, vertex);
        let plane = transform(b, plane);
        Ok(ContactGeometry {
            normal,
            distance: dot(std::array::from_fn(|a| plane[a] - point[a]), normal),
            point,
        })
    }
    /// Same rotation and full rectangle containment only. Bool indicates native
    /// vertex/plane body-role swap. Partial clipping needs edge-pair connectors.
    pub fn contained_faces(
        &self,
        a: &CoordinateFrame,
        size_a: [f32; 3],
        b: &CoordinateFrame,
        size_b: [f32; 3],
    ) -> Result<Vec<(bool, ContactGeometry)>, String> {
        if size_a
            .iter()
            .chain(&size_b)
            .any(|v| !v.is_finite() || *v <= 0.0)
            || a.rotation != b.rotation
        {
            return Err("contact boundary requires shared rotation and positive sizes".into());
        }
        let delta = rotate(
            transpose(a.rotation),
            std::array::from_fn(|i| b.position[i] - a.position[i]),
        );
        let mut depth = f32::INFINITY;
        let mut face = 0;
        for (axis, d) in delta.iter().enumerate() {
            let overlap = (size_a[axis] + size_b[axis]) * 0.5 - d.abs();
            if !overlap.is_finite() {
                return Err("invalid contact overlap".into());
            }
            if overlap <= 0.0 {
                return Ok(Vec::new());
            }
            if overlap < depth {
                depth = overlap;
                face = axis + if *d > 0.0 { 0 } else { 3 };
            }
        }
        let other = if face < 3 { face + 3 } else { face - 3 };
        let va = vertices(size_a);
        let vb = vertices(size_b);
        let axis = face % 3;
        let inside = |p: [f32; 3], size: [f32; 3]| {
            (0..3)
                .filter(|i| *i != axis)
                .all(|i| p[i] >= -size[i] * 0.5 && p[i] <= size[i] * 0.5)
        };
        let bi: Vec<_> = self.faces[other]
            .iter()
            .map(|i| inside(std::array::from_fn(|a| vb[*i][a] + delta[a]), size_a))
            .collect();
        let ai: Vec<_> = self.faces[face]
            .iter()
            .map(|i| inside(std::array::from_fn(|a| va[*i][a] - delta[a]), size_b))
            .collect();
        let mut result = Vec::new();
        // 00ae533e: incident corners 3..0, early return if all four contained;
        // otherwise reference corners 0..3. No extracted face table in source.
        if bi.iter().all(|v| *v) {
            for i in (0..4).rev() {
                let g = self.vertex_plane(
                    b,
                    vb[self.faces[other][i]],
                    a,
                    va[self.faces[face][0]],
                    face,
                )?;
                if g.distance < -self.overlap_goal {
                    result.push((true, g));
                }
            }
        } else if ai.iter().all(|v| *v) && bi.iter().all(|v| !*v) {
            for i in 0..4 {
                let g = self.vertex_plane(
                    a,
                    va[self.faces[face][i]],
                    b,
                    vb[self.faces[other][0]],
                    other,
                )?;
                if g.distance < -self.overlap_goal {
                    result.push((false, g));
                }
            }
        } else {
            return Err("partial contact overlap requires unported edge connectors".into());
        }
        Ok(result)
    }
}
pub struct ImpulseInput {
    pub geometry: ContactGeometry,
    pub parameters: ContactParameters,
    pub reference_impulse_a: [f32; 3],
    pub reference_impulse_b: [f32; 3],
    pub friction_enabled: bool,
}
pub struct ImpulseProfile {
    penetration_goal: f32,
    penetration_gain: f32,
    friction_gain: f32,
}
impl ImpulseProfile {
    pub fn load(bundle: &Bundle) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let float = |at| -> Result<f32, String> {
            Ok(f32::from_le_bytes(image.region(at, 4)?.try_into().unwrap()))
        };
        Ok(Self {
            penetration_goal: float(0x017424d0)?,
            penetration_gain: float(0x01713e40)?,
            friction_gain: float(0x01752a24)?,
        })
    }
}
#[derive(Default)]
pub struct AwakeContact {
    pub iterations: u32,
    pub rebound_velocity: f32,
    pub error: f32,
    cached: Option<(Tensor, Tensor, f32, f32)>,
}
impl AwakeContact {
    /// Explicit caller invalidation; native interval scheduling remains pending.
    pub fn invalidate(&mut self) {
        self.cached = None;
        self.error = 0.0;
    }
    pub fn solve(
        &mut self,
        input: &ImpulseInput,
        a: &mut KernelBody,
        mut b: Option<&mut KernelBody>,
        static_b: &ContactBodyState,
        kernel: &KernelProfile,
        profile: &ImpulseProfile,
    ) -> Result<bool, String> {
        let g = &input.geometry;
        let p = input.parameters;
        if !g.distance.is_finite()
            || g.normal
                .iter()
                .chain(&g.point)
                .chain(&input.reference_impulse_a)
                .chain(&input.reference_impulse_b)
                .any(|v| !v.is_finite())
            || (dot(g.normal, g.normal) - 1.0).abs() > 0.00001
            || p.compression <= 0.0
            || !p.compression.is_finite()
            || !p.decompression.is_finite()
            || p.decompression < 0.0
            || !p.friction.is_finite()
            || p.friction < 0.0
        {
            return Err("invalid awake contact inputs".into());
        }
        if g.distance >= 0.0 {
            self.iterations = 0;
            self.rebound_velocity = 0.0;
            self.cached = None;
            return Ok(false);
        }
        let bs = b.as_ref().map(|b| b.contact_state());
        let v = contact_velocity(
            &a.contact_state(),
            bs.as_ref().unwrap_or(static_b),
            g.point,
            g.normal,
        )?;
        if v.normal_speed >= 0.0 {
            return Ok(false);
        }
        let inv_mass = a.inverse_mass() + b.as_ref().map_or(0.0, |b| b.inverse_mass());
        let correction = dot(input.reference_impulse_a, g.normal) * a.inverse_mass()
            - b.as_ref().map_or(0.0, |b| {
                dot(input.reference_impulse_b, g.normal) * b.inverse_mass()
            });
        if self.cached.is_none() {
            self.iterations = self
                .iterations
                .checked_add(1)
                .ok_or("contact iteration overflow")?;
            let s = skew(std::array::from_fn(|i| g.point[i] - a.center[i]));
            let d = multiply(multiply(s, a.inverse_inertia_world()), s);
            let mut m = d.map(|r| r.map(|v| -v));
            if let Some(b) = b.as_ref() {
                let s = skew(std::array::from_fn(|i| g.point[i] - b.center[i]));
                let d = multiply(multiply(s, b.inverse_inertia_world()), s);
                m = std::array::from_fn(|r| std::array::from_fn(|c| m[r][c] - d[r][c]));
            }
            for (i, row) in m.iter_mut().enumerate() {
                row[i] += inv_mass;
            }
            let inv = inverse(m)?;
            self.rebound_velocity = 0.0;
            if v.normal_speed <= -1.0
                && self.iterations < 5
                && correction <= -v.normal_speed
                && correction >= 0.0
            {
                self.rebound_velocity =
                    (v.normal_speed + correction) * (-p.decompression / p.compression);
            }
            self.rebound_velocity = self.rebound_velocity.max(
                ((profile.penetration_goal - g.distance).max(0.0) / kernel.physics.free_fall_dt)
                    * profile.penetration_gain,
            );
            self.cached = Some((m, inv, inv_mass, v.normal_speed.abs().max(1.0)));
        }
        let (m, inv, inv_mass, clip) = self.cached.unwrap();
        let mut delta = self.rebound_velocity - v.normal_speed;
        self.error += (delta / clip).abs();
        let mut impulse = mul_vector(
            inv,
            std::array::from_fn(|i| g.normal[i] * delta - v.tangent_component[i]),
        );
        if input.friction_enabled {
            let ni = dot(impulse, g.normal);
            let tangent = std::array::from_fn(|i| impulse[i] - g.normal[i] * ni);
            let bound = (correction.abs() / inv_mass).max(ni.abs()) * p.friction;
            let squared = dot(tangent, tangent);
            if bound * bound < squared {
                let length_inverse = 1.0 / squared.sqrt();
                let coefficient = p.friction * profile.friction_gain;
                let direction = std::array::from_fn(|i| {
                    tangent[i] * length_inverse * coefficient + g.normal[i]
                });
                let md = mul_vector(m, direction);
                // Native ensures a MINIMUM impulse magnitude in this branch.
                delta = if delta <= 0.0 {
                    delta.min(-correction.abs())
                } else {
                    delta.max(correction.abs())
                };
                impulse = direction.map(|v| v * (delta / dot(g.normal, md)));
            }
        }
        a.contact_impulse(impulse.map(|v| -v), g.point, kernel)?;
        if let Some(b) = b.as_mut() {
            b.contact_impulse(impulse, g.point, kernel)?;
        }
        Ok(true)
    }
}
pub fn audit(bundle: &Bundle) -> Result<String, String> {
    let kernel = KernelProfile::load(bundle)?;
    let profile = ImpulseProfile::load(bundle)?;
    let generator = BlockContactProfile::load(bundle)?;
    let frame = CoordinateFrame {
        position: [0.0, 0.98, 0.0],
        rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };
    let floor = CoordinateFrame {
        position: [0.0, -0.5, 0.0],
        rotation: frame.rotation,
    };
    let contacts = generator.contained_faces(&frame, [2.0; 3], &floor, [20.0, 1.0, 20.0])?;
    if contacts.len() != 4
        || contacts.iter().any(|(swap, g)| {
            *swap || g.normal != [0.0, -1.0, 0.0] || (g.distance + 0.02).abs() > 0.00001
        })
    {
        return Err("native face fixture mismatch".into());
    }
    let part = crate::assembly::MassPart {
        name: "synthetic contact body".into(),
        frame: frame.clone(),
        mass: 2.0,
        inertia: crate::assembly::MassProfile::load(bundle)?.block_inertia([2.0; 3], 2.0)?,
    };
    let assembly = crate::assembly::AssemblyProperties::combine(&part, &[])?;
    let original = crate::physics::FreeFallBody::new(frame, [0.0, -10.0, 0.0], [0.0; 3])?;
    let mut body = KernelBody::new(&assembly, &original, &kernel)?;
    let fixed = ContactBodyState {
        center: floor.position,
        velocity: [0.0; 3],
        angular_velocity: [0.0; 3],
    };
    let parameters = ContactParameters {
        compression: 7500.0,
        decompression: 0.0,
        friction: 0.3,
    };
    for (name, rebound, expected) in [
        ("inelastic_center", 0.0, 0.0),
        ("restitution_center", 3750.0, 5.0),
    ] {
        let mut central = KernelBody::new(&assembly, &original, &kernel)?;
        let input = ImpulseInput {
            geometry: ContactGeometry {
                normal: [0.0, -1.0, 0.0],
                distance: -0.015,
                point: central.center,
            },
            parameters: ContactParameters {
                decompression: rebound,
                ..parameters
            },
            reference_impulse_a: [0.0; 3],
            reference_impulse_b: [0.0; 3],
            friction_enabled: false,
        };
        let mut solver = AwakeContact::default();
        if !solver.solve(&input, &mut central, None, &fixed, &kernel, &profile)?
            || (central.velocity[1] - expected).abs() > 0.00001
            || central.angular_velocity.iter().any(|v| v.abs() > 0.00001)
        {
            return Err(format!("central impulse analytic mismatch {name}"));
        }
    }
    let mut a = KernelBody::new(&assembly, &original, &kernel)?;
    let mut b = a.clone();
    b.velocity = [0.0; 3];
    let input = ImpulseInput {
        geometry: ContactGeometry {
            normal: [0.0, -1.0, 0.0],
            distance: -0.015,
            point: a.center,
        },
        parameters,
        reference_impulse_a: [0.0; 3],
        reference_impulse_b: [0.0; 3],
        friction_enabled: false,
    };
    AwakeContact::default().solve(&input, &mut a, Some(&mut b), &fixed, &kernel, &profile)?;
    if (a.velocity[1] + 5.0).abs() > 0.00001 || (b.velocity[1] + 5.0).abs() > 0.00001 {
        return Err("two-body impulse momentum conservation mismatch".into());
    }
    let mut solvers: Vec<_> = contacts.iter().map(|_| AwakeContact::default()).collect();
    let mut report = String::new();
    let mut solved = 0;
    for iteration in 0..40 {
        for (solver, (_, g)) in solvers.iter_mut().zip(&contacts) {
            let input = ImpulseInput {
                geometry: g.clone(),
                parameters,
                reference_impulse_a: [0.0; 3],
                reference_impulse_b: [0.0; 3],
                friction_enabled: true,
            };
            solved +=
                usize::from(solver.solve(&input, &mut body, None, &fixed, &kernel, &profile)?);
        }
        if iteration == 0 || iteration == 39 {
            writeln!(
                report,
                "contact_impulse iteration={iteration} velocity={:?} angular_velocity={:?}",
                body.velocity, body.angular_velocity
            )
            .unwrap();
        }
    }
    let worst = contacts
        .iter()
        .map(|(_, g)| {
            contact_velocity(&body.contact_state(), &fixed, g.point, g.normal)
                .map(|v| v.normal_speed)
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .fold(0.0, f32::min);
    if solved == 0 || worst < -0.001 {
        return Err(format!(
            "contact convergence failed solved={solved} normal={worst}"
        ));
    }
    let mut spring = ContactSpring::default();
    let g = &contacts[0].1;
    let _ = spring.force(
        &kernel,
        parameters,
        g.distance,
        g.normal,
        g.point,
        &body.contact_state(),
        &fixed,
    )?;
    let f = spring.force(
        &kernel,
        parameters,
        g.distance - 0.01,
        g.normal,
        g.point,
        &body.contact_state(),
        &fixed,
    )?;
    body.apply_force(f.map(|v| -v), g.point)?;
    body.step(&kernel)?;
    writeln!(report,"legacy_contact_pass generated=4 impulse_applications={solved} worst_approach_speed={worst} analytic_center_cases=2 momentum_conserved=true spring_integrated=true iterations=explicit_fixture_40 sleep=not_implemented partial_clipping=not_implemented scheduler=not_implemented native_differential=not_tested").unwrap();
    Ok(report)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inverse_keeps_nondiagonal_effective_mass() {
        let m = [[2.0, 1.0, 0.0], [1.0, 3.0, 1.0], [0.0, 1.0, 2.0]];
        let p = multiply(m, inverse(m).unwrap());
        for (r, row) in p.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                assert!((v - if r == c { 1.0 } else { 0.0 }).abs() < 1e-6);
            }
        }
        assert!(inverse([[0.0; 3]; 3]).is_err());
    }
    #[test]
    fn vertices_and_plane_point_are_not_midpoints() {
        let p = BlockContactProfile {
            faces: [[0; 4]; 6],
            overlap_goal: 0.01,
        };
        let a = CoordinateFrame {
            position: [0.0, 0.98, 0.0],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        let mut b = a.clone();
        b.position = [0.0; 3];
        assert_eq!(vertices([2.0; 3])[6], [-1.0, -1.0, 1.0]);
        let g = p
            .vertex_plane(&a, [1.0, -1.0, 1.0], &b, [0.0; 3], 1)
            .unwrap();
        assert!((g.point[1] + 0.02).abs() < 1e-7);
        assert_eq!(g.normal, [0.0, -1.0, 0.0]);
        assert_eq!(g.distance, g.point[1]);
    }
    #[test]
    fn rotation_changes_normal_and_signed_distance() {
        let p = BlockContactProfile {
            faces: [[0; 4]; 6],
            overlap_goal: 0.01,
        };
        let b = CoordinateFrame {
            position: [0.0; 3],
            rotation: [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        };
        let mut a = b.clone();
        a.position = [0.02, 0.0, 0.0];
        let g = p.vertex_plane(&a, [0.0; 3], &b, [0.0; 3], 1).unwrap();
        assert_eq!(g.normal, [1.0, 0.0, 0.0]);
        assert_eq!(g.distance, -0.02);
    }
}
