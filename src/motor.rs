//! MotorJoint (the Motor JointInstance, Joint type 4): a Body link that turns
//! the child Part about the parent-side joint frame's z axis.
//!
//! Recovered from:
//! - MotorJoint vtable 01c87398 (typeinfo 01c87420): stepUi 00b381f6 (slot
//!   17), the link builder 00b38196 (slot 18), the type 00b3858a.
//! - stepUi: the angle moves toward DesiredAngle (+0xb0) by at most
//!   |MaxVelocity| (+0xac) per call, then the animation offset (+0xa0),
//!   motor weight (+0xa4) and countdown (+0xa8) decay as in Motor6D, and
//!   00b3831c stores angle x weight + offset as CurrentAngle (+0x9c). A new
//!   value re-poses the link (00b38100: RevoluteLink +0xcc, then 00a9d2c0
//!   marks the Body tree dirty) and wakes both Parts' assemblies
//!   recursively (World::wakeAssembly 00ba7fb0).
//! - The link (00b38196 -> 00a9d39a): the parent side's joint frame and
//!   the child side's, with its inverse. When Part0's Body is the parent
//!   (00b3807c) they are C0 and C1; otherwise C1 and C0, and the same angle
//!   then turns the other way.
//! - RevoluteLink::computeJointCoord 00a9d5b0: parent x Rz(angle) x child^-1,
//!   with Rz from (float)sin and cos of the angle in double (006b130e).
use crate::{g3d, scene::CoordinateFrame};

/// A MotorJoint's stepped state.
#[derive(Clone, Debug, PartialEq)]
pub struct MotorJoint {
    pub name: String,
    /// +0x9c CurrentAngle.
    pub current_angle: f32,
    /// +0xb0 DesiredAngle.
    pub desired_angle: f32,
    /// +0xac MaxVelocity: the largest change per stepUi.
    pub max_velocity: f32,
    /// +0xa0: the animated angle (IAnimatableJoint).
    animated: f32,
    /// +0xa4: the weight of the motor's own angle.
    weight: f32,
    /// +0xa8: stepUi calls before the animation starts to fade.
    countdown: i32,
}

impl MotorJoint {
    /// A Motor with no animation applied.
    pub fn new(name: &str, current_angle: f32, desired_angle: f32, max_velocity: f32) -> Self {
        Self {
            name: name.into(),
            current_angle,
            desired_angle,
            max_velocity,
            animated: 0.0,
            weight: 1.0,
            countdown: 0,
        }
    }
    /// stepUi 00b381f6. Returns the new CurrentAngle when it changed
    /// (00b3831c; a NaN counts as a change).
    pub fn step_ui(&mut self) -> Option<f32> {
        let current = self.current_angle;
        let limit = self.max_velocity.abs();
        let mut angle = self.desired_angle;
        if limit <= (angle - current).abs() {
            angle = if angle - current <= 0.0 {
                current - limit
            } else {
                limit + current
            };
        }
        if self.countdown < 1 {
            if self.weight < 1.0 || 0.0 < self.animated {
                let w = self.weight + 0.03125;
                self.weight = if w < 1.0 { w } else { 1.0 };
                let a = self.animated * 0.96875 + -0.01;
                self.animated = if 0.0 < a { a } else { 0.0 };
            }
        } else {
            self.countdown -= 1;
        }
        let value = angle * self.weight + self.animated;
        if self.current_angle != value || value.is_nan() || self.current_angle.is_nan() {
            self.current_angle = value;
            Some(value)
        } else {
            None
        }
    }
}

/// RevoluteLink::computeJointCoord 00a9d5b0: the child Body's offset in its
/// parent, `parent` x Rz(`angle`) x `child`^-1, where `parent` and `child`
/// are the joint frames on each side (00a9d39a keeps the inverse,
/// `g3d::frame_inverse`).
pub fn revolute_offset(
    parent: &CoordinateFrame,
    child: &CoordinateFrame,
    angle: f32,
) -> CoordinateFrame {
    let s = f64::from(angle).sin() as f32;
    let c = f64::from(angle).cos() as f32;
    // 006b130e: identity with columns (c, s, 0) and (-s, c, 0).
    let rz = [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]];
    let turned = CoordinateFrame {
        position: parent.position,
        rotation: g3d::matrix_multiply(&parent.rotation, &rz),
    };
    g3d::frame_multiply(&turned, &g3d::frame_inverse(child))
}

#[cfg(test)]
mod tests {
    use super::*;
    const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    #[test]
    fn steps_toward_the_desired_angle() {
        let mut m = MotorJoint::new("Motor", 0.0, 1.0, 0.25);
        assert_eq!(m.step_ui(), Some(0.25));
        assert_eq!(m.step_ui(), Some(0.5));
        m.desired_angle = 0.6;
        assert_eq!(m.step_ui(), Some(0.6));
        assert_eq!(m.step_ui(), None);
        // A negative MaxVelocity limits the same way.
        m.max_velocity = -0.5;
        m.desired_angle = -1.0;
        assert_eq!(m.step_ui(), Some(0.6 - 0.5));
    }
    #[test]
    fn revolute_offset_turns_about_the_parent_z_axis() {
        let parent = CoordinateFrame {
            position: [0.0, 1.0, 0.0],
            rotation: IDENTITY,
        };
        let child = CoordinateFrame {
            position: [-2.0, 0.0, 0.0],
            rotation: IDENTITY,
        };
        let at_rest = revolute_offset(&parent, &child, 0.0);
        assert_eq!(at_rest.position, [2.0, 1.0, 0.0]);
        // A quarter turn swings the child's center from +X to +Y of the
        // joint.
        let turned = revolute_offset(&parent, &child, std::f32::consts::FRAC_PI_2);
        assert!((turned.position[0]).abs() < 1e-6, "{:?}", turned.position);
        assert!((turned.position[1] - 3.0).abs() < 1e-6);
        assert!((turned.rotation[1][0] - 1.0).abs() < 1e-6);
    }
}
