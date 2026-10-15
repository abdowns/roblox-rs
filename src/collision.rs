//! Native Bullet manifold refresh and Roblox contact-point velocity boundaries.
//! Contact generation, warm starting and impulse solving remain separate work.
use crate::{bundle::Bundle, native_data::NativeImage, scene::CoordinateFrame};
use std::fmt::Write;

pub struct ContactProfile {
    orthogonal_factor: f32,
    activation_factor: f32,
}
#[derive(Clone, Debug)]
pub struct ContactPoint {
    pub local_a: [f32; 3],
    pub local_b: [f32; 3],
    pub world_a: [f32; 3],
    pub world_b: [f32; 3],
    pub normal_world_b: [f32; 3],
    pub distance: f32,
    pub lifetime: u32,
}
pub struct Manifold {
    pub points: Vec<ContactPoint>,
    pub breaking_threshold: f32,
}
fn transform(frame: &CoordinateFrame, p: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|a| {
        let r = frame.rotation[a];
        r[2] * p[2] + r[1] * p[1] + r[0] * p[0] + frame.position[a]
    })
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[2] * b[2] + a[1] * b[1] + a[0] * b[0]
}
impl ContactProfile {
    pub fn load(bundle: &Bundle) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let data = image.region(0x01ca8d98, 8)?;
        let value = |at| f32::from_le_bytes(data[at..at + 4].try_into().unwrap());
        let result = Self {
            orthogonal_factor: value(0),
            activation_factor: value(4),
        };
        if [result.orthogonal_factor, result.activation_factor]
            .into_iter()
            .any(|v| !v.is_finite() || v < 0.0)
        {
            return Err("invalid native contact factors".into());
        }
        Ok(result)
    }
    /// Normals stay in world space; this routine does not rotate them with B.
    pub fn refresh(
        &self,
        m: &mut Manifold,
        a: &CoordinateFrame,
        b: &CoordinateFrame,
    ) -> Result<usize, String> {
        if m.points.len() > 4
            || !m.breaking_threshold.is_finite()
            || m.breaking_threshold < 0.0
            || a.position
                .iter()
                .chain(a.rotation.iter().flatten())
                .chain(&b.position)
                .chain(b.rotation.iter().flatten())
                .any(|v| !v.is_finite())
        {
            return Err("unsupported native manifold inputs".into());
        }
        for p in &m.points {
            if p.local_a
                .iter()
                .chain(&p.local_b)
                .chain(&p.normal_world_b)
                .any(|v| !v.is_finite())
                || !p.distance.is_finite()
            {
                return Err("non-finite contact point".into());
            }
        }
        let previous: Vec<_> = m.points.iter().map(|p| p.distance).collect();
        // All points update before the native reverse-order removal loop.
        for p in &mut m.points {
            p.world_a = transform(a, p.local_a);
            p.world_b = transform(b, p.local_b);
            p.distance = dot(
                std::array::from_fn(|axis| p.world_a[axis] - p.world_b[axis]),
                p.normal_world_b,
            );
            p.lifetime = p.lifetime.wrapping_add(1);
            if !p.distance.is_finite() {
                return Err("contact refresh overflow".into());
            }
        }
        let mut removed = 0;
        for index in (0..m.points.len()).rev() {
            let p = &m.points[index];
            let factor = if previous[index] > 0.0 && p.distance < 0.0 {
                self.activation_factor
            } else {
                self.orthogonal_factor
            };
            let delta = std::array::from_fn(|axis| {
                p.world_b[axis] - (p.world_a[axis] - p.distance * p.normal_world_b[axis])
            });
            let threshold = m.breaking_threshold;
            if threshold < p.distance || factor * factor * threshold * threshold < dot(delta, delta)
            {
                m.points.swap_remove(index);
                removed += 1;
            }
        }
        Ok(removed)
    }
}
pub struct ContactBodyState {
    pub center: [f32; 3],
    pub velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
}
pub struct ContactVelocity {
    pub relative: [f32; 3],
    pub normal_speed: f32,
    pub normal_component: [f32; 3],
    pub tangent_component: [f32; 3],
}
/// The velocity of a point moving with the body: (w x r) + v, as a929f6
/// and Body::updatePV 00a8eabe compile it.
pub(crate) fn point_velocity(body: &ContactBodyState, point: [f32; 3]) -> [f32; 3] {
    let r: [f32; 3] = std::array::from_fn(|a| point[a] - body.center[a]);
    let w = body.angular_velocity;
    [
        (w[1] * r[2] - r[1] * w[2]) + body.velocity[0],
        (w[2] * r[0] - r[2] * w[0]) + body.velocity[1],
        (r[1] * w[0] - r[0] * w[1]) + body.velocity[2],
    ]
}
/// FUN_00a929f6: velocity(B)-velocity(A), projected onto the supplied unit normal.
pub fn contact_velocity(
    a: &ContactBodyState,
    b: &ContactBodyState,
    point: [f32; 3],
    normal: [f32; 3],
) -> Result<ContactVelocity, String> {
    if a.center
        .iter()
        .chain(&a.velocity)
        .chain(&a.angular_velocity)
        .chain(&b.center)
        .chain(&b.velocity)
        .chain(&b.angular_velocity)
        .chain(&point)
        .chain(&normal)
        .any(|v| !v.is_finite())
    {
        return Err("invalid contact body state".into());
    }
    let va = point_velocity(a, point);
    let vb = point_velocity(b, point);
    let relative = std::array::from_fn(|axis| vb[axis] - va[axis]);
    let normal_speed = dot(relative, normal);
    let normal_component = normal.map(|v| v * normal_speed);
    let tangent_component = std::array::from_fn(|axis| relative[axis] - normal_component[axis]);
    if !normal_speed.is_finite() || tangent_component.iter().any(|v| !v.is_finite()) {
        return Err("contact velocity overflow".into());
    }
    Ok(ContactVelocity {
        relative,
        normal_speed,
        normal_component,
        tangent_component,
    })
}
fn identity() -> CoordinateFrame {
    CoordinateFrame {
        position: [0.0; 3],
        rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    }
}
fn fixture() -> Manifold {
    Manifold {
        breaking_threshold: 0.02,
        points: vec![ContactPoint {
            local_a: [0.0; 3],
            local_b: [0.0; 3],
            world_a: [0.0; 3],
            world_b: [0.0; 3],
            normal_world_b: [0.0, 1.0, 0.0],
            distance: 0.0,
            lifetime: 0,
        }],
    }
}
pub fn audit(bundle: &Bundle) -> Result<String, String> {
    let profile = ContactProfile::load(bundle)?;
    let b = identity();
    let mut report = format!(
        "contact_profile orthogonal_factor={} activation_factor={} breaking_threshold=explicit_fixture_0.02\n",
        profile.orthogonal_factor, profile.activation_factor
    );
    for (name, position, expected) in [
        ("penetrating", [0.0, -0.01, 0.0], 0),
        ("normal_boundary", [0.0, 0.02, 0.0], 0),
        ("normal_separation", [0.0, 0.021, 0.0], 1),
        ("lateral_boundary", [0.02, 0.0, 0.0], 0),
        ("lateral_drift", [0.021, 0.0, 0.0], 1),
    ] {
        let mut a = identity();
        a.position = position;
        let mut m = fixture();
        let removed = profile.refresh(&mut m, &a, &b)?;
        if removed != expected || m.points.iter().any(|p| p.lifetime != 1) {
            return Err(format!("native contact refresh mismatch {name}"));
        }
        writeln!(
            report,
            "contact_refresh case={name} removed={removed} retained={} distance={:?}",
            m.points.len(),
            m.points.first().map(|p| p.distance)
        )
        .unwrap();
    }
    let a = ContactBodyState {
        center: [0.0; 3],
        velocity: [0.0, -3.0, 0.0],
        angular_velocity: [0.0, 0.0, 2.0],
    };
    let b = ContactBodyState {
        center: [0.0; 3],
        velocity: [1.0, 0.0, 0.0],
        angular_velocity: [0.0; 3],
    };
    let v = contact_velocity(&a, &b, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0])?;
    if v.relative != [1.0, 1.0, 0.0]
        || v.normal_speed != 1.0
        || v.tangent_component != [1.0, 0.0, 0.0]
    {
        return Err("native angular contact velocity mismatch".into());
    }
    writeln!(
        report,
        "contact_velocity relative={:?} normal_speed={} tangent={:?}",
        v.relative, v.normal_speed, v.tangent_component
    )
    .unwrap();
    writeln!(report,"contact_pass refresh_cases=5 angular_velocity=true broadphase=not_implemented narrowphase=not_implemented impulse_solver=not_implemented native_differential=not_tested").unwrap();
    Ok(report)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refresh_transforms_local_points_but_keeps_cached_world_normal() {
        let profile = ContactProfile {
            orthogonal_factor: 1.0,
            activation_factor: 1.0,
        };
        let mut a = identity();
        a.rotation = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        a.position = [0.0, -1.01, 0.0];
        let mut m = fixture();
        m.points[0].local_a = [1.0, 0.0, 0.0];
        assert_eq!(profile.refresh(&mut m, &a, &identity()).unwrap(), 0);
        assert_eq!(m.points[0].world_a[0], 0.0);
        assert!((m.points[0].distance + 0.01).abs() < 0.000001);
        assert_eq!(m.points[0].normal_world_b, [0.0, 1.0, 0.0]);
    }
    #[test]
    fn contact_swap_removal_preserves_survivor_age_and_local_points() {
        let profile = ContactProfile {
            orthogonal_factor: 1.0,
            activation_factor: 1.0,
        };
        let i = identity();
        let mut m = fixture();
        m.points[0].local_a[1] = 0.03;
        let mut survivor = m.points[0].clone();
        survivor.local_a = [0.01, -0.01, 0.0];
        survivor.lifetime = 7;
        m.points.push(survivor);
        assert_eq!(profile.refresh(&mut m, &i, &i).unwrap(), 1);
        assert_eq!(m.points.len(), 1);
        assert_eq!(m.points[0].lifetime, 8);
        assert_eq!(m.points[0].local_a, [0.01, -0.01, 0.0]);
    }
    #[test]
    fn activation_factor_applies_only_on_positive_to_negative_transition() {
        let profile = ContactProfile {
            orthogonal_factor: 1.0,
            activation_factor: 2.0,
        };
        let mut a = identity();
        a.position = [0.03, -0.01, 0.0];
        let b = identity();
        let mut m = fixture();
        m.points[0].distance = 0.01;
        assert_eq!(profile.refresh(&mut m, &a, &b).unwrap(), 0);
        a.position[1] = 0.0;
        assert_eq!(profile.refresh(&mut m, &a, &b).unwrap(), 1);
    }
}
