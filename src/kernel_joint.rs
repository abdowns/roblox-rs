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
use crate::{g3d, scene::CoordinateFrame};

/// Joint::JointType of the Glue JointInstance (00b0e42a).
pub const GLUE: u32 = 9;

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
}
