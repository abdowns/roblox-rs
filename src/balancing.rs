//! Balancing.cpp's legacy torque controller (00bd2008), including cached passes.
use crate::{
    assembly::{Tensor, transpose},
    bundle::Bundle,
    kernel::{KernelBody, cross},
    native_data::NativeImage,
    scene::CoordinateFrame,
};

#[derive(Clone)]
pub(crate) struct Balancing {
    gain: f32,
    damping: f32,
    threshold: f32,
    interval_divisor: f64,
    interval_offset: f64,
    remaining: i32,
    cached: [f32; 3],
}
// Native scalar SSE uses (x + y) + z, unlike some other matrix helpers.
fn transform(r: Tensor, v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|a| (r[a][0] * v[0] + r[a][1] * v[1]) + r[a][2] * v[2])
}
impl Balancing {
    pub(crate) fn load(bundle: &Bundle) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let float = |at| -> Result<f32, String> {
            Ok(f32::from_le_bytes(image.region(at, 4)?.try_into().unwrap()))
        };
        let double = |at| -> Result<f64, String> {
            Ok(f64::from_le_bytes(image.region(at, 8)?.try_into().unwrap()))
        };
        // 00bd1f42 copies 017ac940 to +ec/+f0. RunningBase's constructor
        // 00c09128 inherits these without replacing them.
        let result = Self {
            gain: float(0x017ac940)?,
            damping: float(0x017ac944)?,
            threshold: float(0x017ac958)?,
            interval_divisor: double(0x017ac950)?,
            interval_offset: double(0x016f10f0)?,
            remaining: -1,
            cached: [0.0; 3],
        };
        if !result.gain.is_finite()
            || result.gain <= 0.0
            || !result.damping.is_finite()
            || result.damping <= 0.0
            || !result.threshold.is_finite()
            || result.threshold <= 0.0
            || !result.interval_divisor.is_finite()
            || result.interval_divisor == 0.0
            || !result.interval_offset.is_finite()
        {
            return Err("invalid native balancing constants".into());
        }
        Ok(result)
    }
    /// A fresh controller as built by a state constructor with explicit
    /// gain/damping (RunningBase 00c09450, Freefall 00bd3ab4 +0xec).
    pub(crate) fn with_gains(&self, gain: f32, damping: f32) -> Self {
        Self {
            gain,
            damping,
            remaining: -1,
            cached: [0.0; 3],
            ..*self
        }
    }
    pub(crate) fn gains(&self) -> [f32; 2] {
        [self.gain, self.damping]
    }
    pub(crate) fn reset(&mut self) {
        self.remaining = -1;
        self.cached = [0.0; 3];
    }
    pub(crate) fn apply(
        &mut self,
        body: &mut KernelBody,
        torso: &CoordinateFrame,
        inertia: Tensor,
    ) -> Result<(), String> {
        if self.remaining > 0 {
            self.remaining -= 1;
            return body.apply_torque(self.cached);
        }
        // 005e5110 initializes worldUp=(0,1,0); column(1) is torso up.
        let up = std::array::from_fn(|a| torso.rotation[a][1]);
        let inverse = transpose(body.frame.rotation);
        let error = transform(inverse, cross([0.0, 1.0, 0.0], up));
        let velocity = transform(inverse, body.angular_velocity);
        let existing = transform(inverse, body.torque);
        let correction = transform(inertia, error).map(|v| v * -self.gain);
        let mut target = std::array::from_fn(|a| correction[a] + existing[a]);
        // 00bd24d3..2509: threshold tests correction minus existing torque,
        // only on local X/Z; below threshold the target excludes existing.
        for a in [0, 2] {
            if (correction[a] - existing[a]).abs() < inertia[a][a] * self.threshold {
                target[a] = correction[a];
            }
        }
        for a in 0..3 {
            target[a] -= (inertia[a][a] * velocity[a]) * self.damping;
        }
        let world = transform(body.frame.rotation, target);
        self.cached = std::array::from_fn(|a| world[a] - body.torque[a]);
        body.apply_torque(self.cached)?;
        let [x, y, z] = self.cached;
        let magnitude = ((x * x + y * y) + z * z).sqrt();
        // The divisor really is negative in this build. Preserve it, and the
        // truncating conversion, rather than inventing a fixed controller rate.
        self.remaining =
            (f64::from(magnitude) / self.interval_divisor + self.interval_offset) as i32;
        Ok(())
    }
}
