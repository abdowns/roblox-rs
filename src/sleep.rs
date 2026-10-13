//! Sim::AssemblyState and the assembly motion history that SleepStage
//! (v8world/SleepStage.cpp, step 00b707f4) uses to put still assemblies to
//! sleep.
//!
//! Recovered from:
//! - history ctor 00adafbc / refill 00adb022: 15 copies of the current
//!   sample, tick 0, countdown 15, spread 0;
//! - sample 00adaf4a: the assembly root primitive's position, then its
//!   rotation quaternion (G3D 006ba42a, not normalized) times the assembly
//!   radius (00ad853c -> 00ad9d72, cached at Assembly +0x74);
//! - step 00adb06a: every 8th call drops the countdown (not below 0), stores
//!   a sample in the ring and recomputes the spread (00adb118: the largest
//!   squared distance of a sample from the mean, 00adb2d8, whose quaternion
//!   sum starts from the identity); "still" is countdown 0 and spread below
//!   0.01 (017719a8);
//! - moving 00adb214: spread above 0.02 (017a3de0);
//! - countdown reset 00adb234 (Assembly 00ad8aa0, wakeAssembly 00b737c4,
//!   Humanoid::onStepped 00c0072c).
use crate::{bundle::Bundle, native_data::NativeImage, scene::CoordinateFrame};

/// Sim::AssemblyState values in the order of the native enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AssemblyState {
    Anchored = 0,
    RecursiveWakePending = 1,
    WakePending = 2,
    Awake = 3,
    SleepingChecking = 4,
    SleepingDeeply = 5,
}

impl AssemblyState {
    /// The SLEEPING_CHECKING / SLEEPING_DEEPLY pair ((state & ~1) == 4).
    pub fn sleeping(self) -> bool {
        matches!(self, Self::SleepingChecking | Self::SleepingDeeply)
    }
}

/// History length (00adafbc / 00adb022 push 0xf) and sample interval
/// (00adb06a compares the tick with 8).
const HISTORY: usize = 15;
const INTERVAL: u32 = 8;

/// The SleepStage thresholds read from the binary.
#[derive(Clone, Copy, Debug)]
pub struct SleepProfile {
    /// 017719a8: still when the spread is below this.
    pub still: f32,
    /// 017a3de0: moving when the spread is above this.
    pub moving: f32,
    /// 016f752c: MovingNoPhysicsBase wakes the floor above this speed.
    pub kinematic_wake_speed: f32,
}

impl SleepProfile {
    pub fn load(bundle: &Bundle) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let float = |at| -> Result<f32, String> {
            Ok(f32::from_le_bytes(image.region(at, 4)?.try_into().unwrap()))
        };
        let result = Self {
            still: float(0x017719a8)?,
            moving: float(0x017a3de0)?,
            kinematic_wake_speed: float(0x016f752c)?,
        };
        if [result.still, result.moving, result.kinematic_wake_speed]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || result.moving <= result.still
        {
            return Err("unexpected native SleepStage profile".into());
        }
        Ok(result)
    }
    #[cfg(test)]
    pub fn test_profile() -> Self {
        Self {
            still: 0.01,
            moving: 0.02,
            kinematic_wake_speed: 2.0,
        }
    }
}

/// G3D Quat(Matrix3) 006ba42a: x, y, z, w, without normalizing.
#[allow(clippy::neg_cmp_op_on_partial_ord)] // The native branches on `<=`.
pub fn rotation_quaternion(m: &[[f32; 3]; 3]) -> [f32; 4] {
    let [a, b, c] = [m[0][0], m[1][1], m[2][2]];
    let trace = a + b + c;
    if trace > 0.0 {
        let root = (trace + 1.0).sqrt();
        let scale = 0.5 / root;
        [
            (m[2][1] - m[1][2]) * scale,
            (m[0][2] - m[2][0]) * scale,
            (m[1][0] - m[0][1]) * scale,
            root * 0.5,
        ]
    } else if !(a <= b || a <= c) {
        let root = (((a + 1.0) - b) - c).sqrt();
        let scale = -0.5 / root;
        [
            root * -0.5,
            (m[0][1] + m[1][0]) * scale,
            (m[0][2] + m[2][0]) * scale,
            (m[2][1] - m[1][2]) * scale,
        ]
    } else if !(b <= c) {
        let root = (((b + 1.0) - a) - c).sqrt();
        let scale = -0.5 / root;
        [
            (m[0][1] + m[1][0]) * scale,
            root * -0.5,
            (m[1][2] + m[2][1]) * scale,
            (m[0][2] - m[2][0]) * scale,
        ]
    } else {
        let root = (((c + 1.0) - a) - b).sqrt();
        let scale = -0.5 / root;
        [
            (m[0][2] + m[2][0]) * scale,
            (m[1][2] + m[2][1]) * scale,
            root * -0.5,
            (m[1][0] - m[0][1]) * scale,
        ]
    }
}

/// 00ad9d72 for one primitive: |abs(position - root) + size / 2|, summed
/// z + (y + x).
pub fn primitive_radius(position: [f32; 3], root: [f32; 3], size: [f32; 3]) -> f32 {
    let d: [f32; 3] = std::array::from_fn(|a| (position[a] - root[a]).abs() + size[a] * 0.5);
    (d[2] * d[2] + (d[1] * d[1] + d[0] * d[0])).sqrt()
}

/// 00adaf4a: root primitive position, then quaternion times radius.
pub fn sample(frame: &CoordinateFrame, radius: f32) -> [f32; 7] {
    let q = rotation_quaternion(&frame.rotation);
    let p = frame.position;
    [
        p[0],
        p[1],
        p[2],
        radius * q[0],
        radius * q[1],
        radius * q[2],
        radius * q[3],
    ]
}

/// The motion history at Assembly +0x3c.
#[derive(Clone, Debug)]
pub struct MotionHistory {
    samples: Vec<[f32; 7]>,
    next: usize,
    tick: u32,
    countdown: u32,
    spread: f32,
}

impl MotionHistory {
    /// 00adafbc.
    pub fn new(current: [f32; 7]) -> Self {
        Self {
            samples: vec![current; HISTORY],
            next: 0,
            tick: 0,
            countdown: HISTORY as u32,
            spread: 0.0,
        }
    }
    /// 00adb234.
    pub fn reset_countdown(&mut self) {
        self.countdown = HISTORY as u32;
    }
    pub fn spread(&self) -> f32 {
        self.spread
    }
    /// 00adb214.
    pub fn moving(&self, profile: &SleepProfile) -> bool {
        profile.moving < self.spread
    }
    /// 00adb06a: returns whether the assembly is still.
    pub fn step(&mut self, profile: &SleepProfile, current: impl FnOnce() -> [f32; 7]) -> bool {
        self.tick += 1;
        if self.tick == INTERVAL {
            self.tick = 0;
            self.countdown = self.countdown.saturating_sub(1);
            self.samples[self.next] = current();
            self.next = (self.next + 1) % self.samples.len();
            self.spread = self.compute_spread();
        }
        self.countdown == 0 && self.spread < profile.still
    }
    /// 00adb2d8 then 00adb118.
    fn compute_spread(&self) -> f32 {
        let mut sum = [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0f32];
        for s in &self.samples {
            for a in 0..7 {
                sum[a] += s[a];
            }
        }
        let inverse = 1.0 / self.samples.len() as f32;
        let m = sum.map(|v| inverse * v);
        let mut largest = 0.0f32;
        for s in &self.samples {
            let d: [f32; 7] = std::array::from_fn(|a| (s[a] - m[a]) * (s[a] - m[a]));
            let distance = (d[6] + ((d[4] + d[3]) + d[5])) + (d[2] + (d[0] + d[1]));
            // CMOVA: NaN keeps the previous largest.
            if distance > largest {
                largest = distance;
            }
        }
        largest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(y: f32) -> CoordinateFrame {
        CoordinateFrame {
            position: [0.0, y, 0.0],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        }
    }
    #[test]
    fn still_after_fifteen_samples() {
        let profile = SleepProfile::test_profile();
        let radius = primitive_radius([0.0; 3], [0.0; 3], [6.0, 0.3, 6.0]);
        let mut history = MotionHistory::new(sample(&frame(0.15), radius));
        let mut still_at = None;
        for step in 1..=200 {
            if history.step(&profile, || sample(&frame(0.15), radius)) {
                still_at = Some(step);
                break;
            }
        }
        // The countdown reaches 0 on the 15th sample (step 120). The
        // identity start of the quaternion sum leaves (1/15)^2 of spread.
        assert_eq!(still_at, Some(120));
        assert!((history.spread() - 1.0 / 225.0).abs() < 1e-5);
        assert!(!history.moving(&profile));
    }
    #[test]
    fn moving_samples_keep_it_awake() {
        let profile = SleepProfile::test_profile();
        let mut history = MotionHistory::new(sample(&frame(0.0), 1.0));
        for step in 1..=400 {
            let y = step as f32 * 0.01;
            assert!(!history.step(&profile, || sample(&frame(y), 1.0)));
        }
        assert!(history.moving(&profile));
    }
}
