//! Kernel joints of the legacy solver: kernel Points, the break connectors
//! that pull them together, and GlueJoint's geometry.
//!
//! Recovered from:
//! - Kernel::newPoint 00a96432: a Point is a Body plus a local offset
//!   (00aa333c); 00a96270 hands back an existing Point with the same Body and
//!   bit-identical offset (reference counted) instead of adding a new one.
//! - Point::step 00aa3156 (the Body's PV: R x local + p, then the force
//!   cleared) and Point::forceToBody 00aa3244 (force and (p - cofm) x F into
//!   the root SimBody, when the Body has one).
//! - PointToPointBreakConnector::computeForce 00a91948: f = -k (p1 - p0),
//!   point 0 gets -f and point 1 +f; broken when |f|1 (summed in double)
//!   exceeds the threshold. Connector type 1 (00a91b1c).
//! - NormalBreakConnector::computeForce 00a91a12: the same spring, broken
//!   when the pull along point 0's Body axis (NormalId +0x30) exceeds the
//!   threshold. The flag is recomputed on every call.
//! - GlueJoint: MultiJoint with 4 breaking connectors (ctor 00b0ce2c);
//!   putInKernel 00b0d632 puts the face (F0..F3, +0xd0) through C0 into
//!   Part0 and through C1 into Part1, one Point pair per corner, k the
//!   smaller Part's joint stiffness (00b3ed8a -> 00b63194 -> 00a92640), the
//!   threshold 20 x Constants 00a92056 of the face size (00b0d5ea,
//!   006d86d4, 00b0e1da) and the normal C0's z axis (006b79ae -> 006b76c8).
//! - MultiJoint::removeFromKernel 00b3ee8e: per connector, removeConnector
//!   (a9611e -> a99ff2), then releasePoint (a964ec) for both Points; a Point
//!   whose use count reaches 0 leaves the Point list (IndexArray remove
//!   a95fa8: the last Point moves into its slot).
//! - RotateJoint::putInKernel 00b6b640 (legacy solver): two Point pairs at
//!   C0 x (0, 0, -1) / C1 x (0, 0, -1), then (0, 0, 1), each held by a
//!   PointToPointBreakConnector with the smaller joint stiffness and an
//!   infinite threshold (01cfca70). The Rotate family is not breakable
//!   (vtable slot 8, 00be5ee0, returns 0).
//! - DynamicRotateJoint::putInKernel 00b6bb04 adds a RotateConnector
//!   (ctor 00a9124a, computeForce 00a9178e, type 1 00a91b1c) whose gain is
//!   jointK x (0.1 x the smaller cross size 00b6b53c)^2. Its stepUi
//!   00b6bd86 reads the axle Part's surface input (00b6be18) and wakes both
//!   assemblies recursively when the value is not 0. RotatePJoint::stepWorld
//!   00b6bf20 sets a target (00a9159a), RotateVJoint::stepWorld 00b6c11e a
//!   velocity (00a916cc).
use crate::{g3d, scene::CoordinateFrame};
use std::f64::consts::{PI, TAU};

/// Joint::JointType of the hinge JointInstances: RotateJoint 6 (00b6ca2e),
/// RotatePJoint 7 (00b6ca62), RotateVJoint 8 (00b6ca6c).
pub const ROTATE: u32 = 6;
pub const ROTATE_P: u32 = 7;
pub const ROTATE_V: u32 = 8;
/// Joint::JointType of the Glue JointInstance (00b0e42a).
pub const GLUE: u32 = 9;

/// RBX::InputType values a motor surface acts on (00b6be18); the others
/// (NoInput 0 and the controller inputs) give 0.
pub const INPUT_CONSTANT: u32 = 12;
pub const INPUT_SIN: u32 = 13;

/// A Part face's motor input (Primitive +0xd8, 12 bytes per NormalId:
/// InputType, ParamA, ParamB).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceInput {
    pub input: u32,
    pub param_a: f32,
    pub param_b: f32,
}

impl Default for SurfaceInput {
    /// 0098fffc, the entry a Part without surface inputs reads: NoInput,
    /// ParamA -0.5, ParamB 0.5.
    fn default() -> Self {
        Self {
            input: 0,
            param_a: -0.5,
            param_b: 0.5,
        }
    }
}

impl SurfaceInput {
    /// 00b6be18: the motor value at `time` (Workspace.DistributedGameTime):
    /// Constant gives ParamB, Sin ParamA x sin((float)time x ParamB).
    pub fn value(&self, time: f64) -> f32 {
        match self.input {
            INPUT_CONSTANT => self.param_b,
            INPUT_SIN => self.param_a * f64::from(time as f32 * self.param_b).sin() as f32,
            _ => 0.0,
        }
    }
}

/// math.h 006af7f9's range reduction: into [-pi, pi) by whole turns, in
/// double.
fn reduce(d: f64) -> f64 {
    // d < -pi || pi <= d (NaN stays NaN either way).
    if !(-PI..PI).contains(&d) {
        d - ((d + PI) / TAU).floor() * TAU
    } else {
        d
    }
}

/// 006af7f9: a - b brought into [-pi, pi).
pub fn wrap_angle(a: f32, b: f32) -> f32 {
    reduce(f64::from(a - b)) as f32
}

/// 006af8f5: the angle halfway from b toward a, the short way round.
pub fn halfway(a: f32, b: f32) -> f32 {
    reduce(f64::from(wrap_angle(a, b) * 0.5 + b)) as f32
}

/// RotateJoint::putInKernel 00b6b640: the two hinge Points of one side,
/// frame x (0, 0, -1) then frame x (0, 0, 1) (0177ed54 and 0177ed50 times
/// Vector3::unitZ 005e50b0).
pub fn hinge_points(frame: &CoordinateFrame) -> [[f32; 3]; 2] {
    [-1.0f32, 1.0].map(|s| {
        g3d::frame_multiply(
            frame,
            &CoordinateFrame {
                position: [0.0 * s, 0.0 * s, 1.0 * s],
                rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            },
        )
        .position
    })
}

/// 00b6b53c for one Part: the larger of its two sizes across the face
/// `normal` (a NormalId), the first one on a tie.
pub fn cross_size(size: [f32; 3], normal: u32) -> f32 {
    let a = ((normal + 1) % 3) as usize;
    let b = ((normal + 2) % 3) as usize;
    if size[b] <= size[a] { size[a] } else { size[b] }
}

/// NormalId::fromVector 006b76c8 for an exact unit axis: +X 0, +Y 1, +Z 2,
/// -X 3, -Y 4, -Z 5, and 6 for anything else (an assert in debug builds).
pub fn normal_id(v: [f32; 3]) -> u32 {
    if v[0] == 1.0 {
        0
    } else if v[1] == 1.0 {
        1
    } else if v[2] == 1.0 {
        2
    } else if v[0] == -1.0 {
        3
    } else if v[1] == -1.0 {
        4
    } else if v[2] == -1.0 {
        5
    } else {
        6
    }
}

/// 017a1f90: the rounding offsets iRound adds (-0.5 below zero, 0.5 above).
fn round(x: f32) -> i32 {
    (if 0.0 <= x { 0.5 } else { -0.5 } + x) as i32
}

/// Constants 00a92056 (v8kernel/Constants.cpp): the joint strength of a
/// grid1 x grid2 stud face. Both sides round to at least 1 stud; the shorter
/// one scales a per-length factor of the longer one (table 0179fc80 below 7
/// studs, longer/7 x 4.681 from 7).
pub fn joint_strength(grid1: f32, grid2: f32) -> f32 {
    // 0179fc80, bit for bit.
    const TABLE: [u32; 7] = [
        0, 0x3f8c8b44, 0x40089375, 0x401b53f8, 0x404c3958, 0x409245a2, 0x4095cac1,
    ];
    let a = round(grid1).max(1);
    let b = round(grid2).max(1);
    let (short, long) = if a < b { (a, b) } else { (b, a) };
    let factor = if long < 7 {
        f32::from_bits(TABLE[long as usize])
    } else {
        (long as f32 / 7.0) * 4.681
    };
    (short as f32 * factor * 0.5) * 7500.0
}

/// 00b0e1da: the face's two side lengths, |F1 - F0| and |F3 - F0|.
pub fn face_size(face: &[[f32; 3]; 4]) -> [f32; 2] {
    let side = |p: [f32; 3]| {
        let d: [f32; 3] = std::array::from_fn(|a| p[a] - face[0][a]);
        (d[2] * d[2] + d[1] * d[1] + d[0] * d[0]).sqrt()
    };
    [side(face[1]), side(face[3])]
}

/// GlueJoint's break threshold (00b0d5ea): 20 x the face's strength.
pub fn glue_threshold(face: &[[f32; 3]; 4]) -> f32 {
    let [a, b] = face_size(face);
    joint_strength(a, b) * 20.0
}

/// 0066ea7a: the face's points through a CFrame.
pub fn face_through(face: &[[f32; 3]; 4], frame: &CoordinateFrame) -> [[f32; 3]; 4] {
    face.map(|p| {
        g3d::frame_multiply(
            frame,
            &CoordinateFrame {
                position: p,
                rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            },
        )
        .position
    })
}

/// The Body a kernel Point rides on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum JointBody {
    /// An unanchored Part (`LooseWorld::prims`).
    Loose(usize),
    /// An anchored or grounded Part (`LooseWorld` fixed Bodies).
    Fixed(usize),
}

/// A kernel Point: a place on a Part's Body.
#[derive(Clone, Debug, PartialEq)]
pub struct KernelPoint {
    pub body: JointBody,
    /// Point +0x10: the offset in the Body.
    pub local: [f32; 3],
    /// Point +0x1c: the world position at the last Point::step.
    pub world: [f32; 3],
    /// Point +0x28: the force gathered this substep.
    pub force: [f32; 3],
    /// Point +0x8: connectors using this Point.
    pub uses: u32,
}

impl KernelPoint {
    /// Point::step 00aa3156 with the Body's current PV.
    pub fn step(&mut self, body: &CoordinateFrame) {
        let r = &body.rotation;
        let l = self.local;
        self.world = std::array::from_fn(|a| {
            ((r[a][0] * l[0] + r[a][1] * l[1]) + r[a][2] * l[2]) + body.position[a]
        });
        self.force = [0.0; 3];
    }
}

/// What a break connector tests (+0x30 for the normal kind).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BreakTest {
    /// PointToPointBreakConnector: the force's |f|1.
    PointToPoint,
    /// NormalBreakConnector: the pull along point 0's Body axis.
    Normal(u32),
}

/// A PointToPointBreakConnector or NormalBreakConnector.
#[derive(Clone, Debug, PartialEq)]
pub struct BreakConnector {
    /// +0x1c, +0x20: indices into the kernel's Points.
    pub points: [usize; 2],
    /// +0x24.
    pub k: f32,
    /// +0x28.
    pub threshold: f32,
    pub test: BreakTest,
    /// +0x2c, set by the last computeForce.
    pub broken: bool,
}

impl BreakConnector {
    /// computeForce (00a91948 / 00a91a12). `axes` is point 0's Body
    /// rotation (Body +0xb4), read only by the normal test.
    pub fn compute_force(&mut self, points: &mut [KernelPoint], axes: &[[f32; 3]; 3]) {
        let [i0, i1] = self.points;
        let (p0, p1) = (points[i0].world, points[i1].world);
        let k = -self.k;
        let f: [f32; 3] = std::array::from_fn(|a| (p1[a] - p0[a]) * k);
        let a = &mut points[i0];
        a.force = std::array::from_fn(|i| a.force[i] - f[i]);
        let b = &mut points[i1];
        b.force = std::array::from_fn(|i| f[i] + b.force[i]);
        let measure = match self.test {
            BreakTest::PointToPoint => {
                let sum = (f64::from(f[0].abs()) + f64::from(f[1].abs())) + f64::from(f[2].abs());
                sum as f32
            }
            BreakTest::Normal(id) => {
                let axis = (id % 3) as usize;
                let sign = (((id / 3) as i32 * 2).wrapping_neg() | 1) as f32;
                let n = [
                    axes[0][axis] * sign,
                    axes[1][axis] * sign,
                    sign * axes[2][axis],
                ];
                -((n[0] * f[0] + n[1] * f[1]) + n[2] * f[2])
            }
        };
        self.broken = measure > self.threshold;
    }
}

/// A RotateConnector: the torque spring of a RotatePJoint or RotateVJoint
/// about C0's z axis.
#[derive(Clone, Debug, PartialEq)]
pub struct RotateConnector {
    /// +0x20, +0x24: Part0's and Part1's Bodies.
    pub bodies: [JointBody; 2],
    /// +0x28, +0x58: the joint's C0 and C1.
    coords: [CoordinateFrame; 2],
    /// +0x1c: the joint's BaseAngle.
    pub base: f32,
    /// +0x88.
    pub k: f32,
    /// +0x8c: the angle at the last computeForce, less the base.
    pub current: f32,
    /// +0x90.
    pub desired: f32,
    /// +0x94: added to the desired angle at every computeForce.
    pub increment: f32,
    /// +0x98: move the desired angle halfway to the current one, once.
    pub recenter: bool,
}

impl RotateConnector {
    /// ctor 00a9124a: k = (jointK x size) x size, and the desired angle is
    /// the angle now less the base (00a91368). `rotations` are the Bodies'
    /// PV rotations.
    pub fn new(
        bodies: [JointBody; 2],
        coords: [CoordinateFrame; 2],
        base: f32,
        joint_k: f32,
        size: f32,
        rotations: [&[[f32; 3]; 3]; 2],
    ) -> Self {
        let mut c = Self {
            bodies,
            coords,
            base,
            k: joint_k * size * size,
            current: 0.0,
            desired: 0.0,
            increment: 0.0,
            recenter: false,
        };
        c.desired = c.angle(rotations).0 - base;
        c
    }
    /// 00a913ec: A = R0 x C0, B = R1 x C1, M = A^T x B; the angle is
    /// atan2(M[1][0], M[0][0]) in double, about A's z axis (returned too).
    pub fn angle(&self, rotations: [&[[f32; 3]; 3]; 2]) -> (f32, [f32; 3]) {
        let a = g3d::matrix_multiply(rotations[0], &self.coords[0].rotation);
        let b = g3d::matrix_multiply(rotations[1], &self.coords[1].rotation);
        let t: [[f32; 3]; 3] = std::array::from_fn(|r| std::array::from_fn(|c| a[c][r]));
        let m = g3d::matrix_multiply(&t, &b);
        let angle = f64::from(m[1][0]).atan2(f64::from(m[0][0])) as f32;
        (angle, [a[0][2], a[1][2], a[2][2]])
    }
    /// computeForce 00a9178e: the current angle (00a914d6), the desired one
    /// moved on (00a91724), and a torque of k x wrap(desired - current)
    /// about the axis. Returns the torque on Body 1; Body 0 takes its
    /// negation (both through accumulateTorque 00a6c134).
    pub fn compute_torque(&mut self, rotations: [&[[f32; 3]; 3]; 2]) -> [f32; 3] {
        let (angle, axis) = self.angle(rotations);
        self.current = angle - self.base;
        let desired = if self.recenter {
            self.recenter = false;
            halfway(self.current, self.desired)
        } else {
            self.desired
        };
        self.desired = desired + self.increment;
        let s = wrap_angle(self.desired, self.current) * self.k;
        [axis[0] * s, axis[1] * s, s * axis[2]]
    }
    /// 00a9159a (RotatePJoint::stepWorld): reach `value`, brought into
    /// [-pi, pi), over the next `substeps` computeForce calls.
    pub fn set_target(&mut self, value: f32, substeps: u32) {
        let target = reduce(f64::from(value)) as f32;
        self.increment = wrap_angle(target, self.desired) / substeps as f32;
    }
    /// 00a916cc (RotateVJoint::stepWorld): turn (30 x value) x the kernel
    /// dt per computeForce; at 0 the desired angle is recentred once.
    pub fn set_velocity(&mut self, value: f32, kernel_dt: f32) {
        self.increment = 30.0 * value * kernel_dt;
        if value == 0.0 {
            self.recenter = true;
        }
    }
}

/// Kernel::newPoint 00a96432 / 00a96270: the index of a Point for a Body and
/// offset, reusing an identical one.
pub fn new_point(points: &mut Vec<KernelPoint>, body: JointBody, local: [f32; 3]) -> usize {
    if let Some(i) = points
        .iter()
        .position(|p| p.body == body && p.local == local)
    {
        points[i].uses += 1;
        return i;
    }
    points.push(KernelPoint {
        body,
        local,
        world: [0.0; 3],
        force: [0.0; 3],
        uses: 1,
    });
    points.len() - 1
}

/// Kernel::releasePoint 00a964ec: one use less; an unused Point leaves the
/// list (a95fa8). Returns the (old, new) index of the Point that moved into
/// the freed slot, for the connectors that refer to it.
pub fn release_point(points: &mut Vec<KernelPoint>, index: usize) -> Option<(usize, usize)> {
    let point = &mut points[index];
    point.uses -= 1;
    if point.uses > 0 {
        return None;
    }
    let last = points.len() - 1;
    points.swap_remove(index);
    (last != index).then_some((last, index))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strength_table_and_rounding() {
        // 1 x 1 face: 1.098 x 0.5 x 7500.
        assert_eq!(joint_strength(0.0, 0.0), 1.098 * 0.5 * 7500.0);
        assert_eq!(joint_strength(2.0, 4.0), (2.0 * 3.191 * 0.5) * 7500.0);
        assert_eq!(joint_strength(4.4, 1.6), joint_strength(2.0, 4.0));
        assert_eq!(
            joint_strength(14.0, 1.0),
            (1.0 * ((14.0 / 7.0) * 4.681) * 0.5) * 7500.0
        );
    }
    #[test]
    fn normal_ids() {
        assert_eq!(normal_id([0.0, 0.0, 1.0]), 2);
        assert_eq!(normal_id([0.0, -1.0, 0.0]), 4);
        assert_eq!(normal_id([0.6, 0.8, 0.0]), 6);
    }
    #[test]
    fn spring_pulls_points_together_and_breaks() {
        let mut points = Vec::new();
        let a = new_point(&mut points, JointBody::Fixed(0), [0.0; 3]);
        let b = new_point(&mut points, JointBody::Loose(0), [0.0; 3]);
        assert_eq!(new_point(&mut points, JointBody::Fixed(0), [0.0; 3]), a);
        assert_eq!(points[a].uses, 2);
        points[a].world = [0.0, 0.0, 0.0];
        points[b].world = [0.0, -0.5, 0.0];
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let mut c = BreakConnector {
            points: [a, b],
            k: 100.0,
            threshold: 60.0,
            test: BreakTest::Normal(4),
            broken: false,
        };
        c.compute_force(&mut points, &identity);
        // Point 1 is pulled up toward point 0, point 0 down.
        assert_eq!(points[b].force, [0.0, 50.0, 0.0]);
        assert_eq!(points[a].force[1], -50.0);
        // Pull along -Y of body 0 is 50, under the threshold.
        assert!(!c.broken);
        c.threshold = 40.0;
        c.compute_force(&mut points, &identity);
        assert!(c.broken);
        // Releasing: the shared Point stays until its last use.
        assert_eq!(release_point(&mut points, a), None);
        assert_eq!(points.len(), 2);
        assert_eq!(release_point(&mut points, a), Some((1, 0)));
        assert_eq!(points[0].body, JointBody::Loose(0));
    }
    #[test]
    fn angles_wrap_into_minus_pi_to_pi() {
        assert_eq!(wrap_angle(0.5, 0.25), 0.25);
        // 3 - (-3) = 6 turns into 6 - 2 pi.
        assert_eq!(wrap_angle(3.0, -3.0), (6.0f64 - TAU) as f32);
        // pi itself is out of range; -pi is in.
        assert_eq!(
            wrap_angle(std::f32::consts::PI, 0.0),
            (f64::from(std::f32::consts::PI) - TAU) as f32
        );
        // Halfway from 3 toward -3 goes through pi, not through 0.
        let h = halfway(-3.0, 3.0);
        assert!((h.abs() - std::f32::consts::PI).abs() < 1e-6, "{h}");
        assert_eq!(halfway(1.0, 0.0), 0.5);
    }
    #[test]
    fn hinge_points_and_cross_sizes() {
        let frame = CoordinateFrame {
            position: [1.0, 2.0, 3.0],
            rotation: [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]],
        };
        // C0's z axis is world +Y here.
        assert_eq!(hinge_points(&frame), [[1.0, 1.0, 3.0], [1.0, 3.0, 3.0]]);
        // Across +Y (1): the larger of x and z.
        assert_eq!(cross_size([4.0, 1.0, 2.0], 1), 4.0);
        assert_eq!(cross_size([4.0, 1.0, 2.0], 0), 2.0);
        assert_eq!(cross_size([4.0, 1.0, 2.0], 5), 4.0);
    }
    #[test]
    fn surface_inputs() {
        let none = SurfaceInput::default();
        assert_eq!(none.value(3.0), 0.0);
        let constant = SurfaceInput {
            input: INPUT_CONSTANT,
            ..none
        };
        assert_eq!(constant.value(3.0), 0.5);
        let sin = SurfaceInput {
            input: INPUT_SIN,
            param_a: 2.0,
            param_b: 1.0,
        };
        assert_eq!(sin.value(0.5), 2.0 * (0.5f64.sin() as f32));
    }
    #[test]
    fn rotate_connector_pulls_back_to_the_desired_angle() {
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let c = CoordinateFrame {
            position: [0.0; 3],
            rotation: identity,
        };
        let bodies = [JointBody::Fixed(0), JointBody::Loose(0)];
        let mut r = RotateConnector::new(
            bodies,
            [c.clone(), c],
            0.0,
            100.0,
            2.0,
            [&identity, &identity],
        );
        assert_eq!(r.k, 400.0);
        assert_eq!(r.desired, 0.0);
        // Body 1 turned +0.1 about z: the torque on it is about -z.
        let turned = g3d::from_axis_angle_fast([0.0, 0.0, 1.0], 0.1);
        let t = r.compute_torque([&identity, &turned]);
        assert!((r.current - 0.1).abs() < 1e-6);
        assert_eq!(t[0], 0.0);
        assert!((t[2] + 40.0).abs() < 1e-3, "{t:?}");
        // RotateP: a quarter turn spread over 19 substeps.
        r.set_target(std::f32::consts::FRAC_PI_2, 19);
        assert_eq!(r.increment, std::f32::consts::FRAC_PI_2 / 19.0);
        // RotateV at 0 recentres once on the next computeForce.
        r.set_velocity(0.0, 0.5);
        assert_eq!(r.increment, 0.0);
        assert!(r.recenter);
        r.compute_torque([&identity, &turned]);
        assert!(!r.recenter);
        assert!((r.desired - 0.05).abs() < 1e-6, "{}", r.desired);
        r.set_velocity(2.0, 0.5);
        assert_eq!(r.increment, 30.0);
    }
}
