//! HumanoidState.cpp dispatch for the default R6 character: transition table,
//! floor rays and the Running/Landed/Jumping/Freefall/RunningNoPhysics math.
//! Constants and the table are read from the exact binary at runtime.
use crate::{bundle::Bundle, native_data::NativeImage, scene::CoordinateFrame};
use iced_x86::{Decoder, DecoderOptions, Mnemonic};

/// Native HumanoidStateType ids (factory 00c01ad8). Only these are simulated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateType {
    Jumping = 3,
    Freefall = 5,
    Landed = 7,
    Running = 8,
    RunningNoPhysics = 10,
}
impl StateType {
    fn from_native(id: i32) -> Result<Self, String> {
        Ok(match id {
            3 => Self::Jumping,
            5 => Self::Freefall,
            7 => Self::Landed,
            8 => Self::Running,
            10 => Self::RunningNoPhysics,
            _ => return Err(format!("Humanoid state {id} is not implemented")),
        })
    }
}

/// STATE_TABLE value meaning "no transition".
const NONE: i32 = 18;
pub const EVENTS: usize = 26;

#[derive(Clone, Debug)]
pub struct HumanoidStateProfile {
    /// 017af5d0: row 0 is the event header, row s+1 the state's targets.
    table: Vec<[i32; EVENTS]>,
    /// 017afd68, Torso-local Z multipliers of the first five floor rays.
    pub ray_offsets: [f32; 5],
    /// 017af5c0: leg-length factor without / with a previous floor.
    pub ray_length_factors: [f32; 2],
    pub ray_extent: f32,
    pub fast_fall_speed: f32,
    pub no_floor_seconds: f32,
    pub fall_down_tilt: f32,
    pub running_idle_seconds: f32,
    pub jump_seconds: f32,
    pub jump_direction_y: f32,
    pub jump_gain: f32,
    pub landed_seconds: f32,
    pub landed_balance: [f32; 2],
    pub freefall_balance_gain: f32,
    pub air_gain: f32,
    pub air_limit_squared: f32,
    pub air_limit: f32,
    pub freefall_yaw_gain: f32,
    pub freefall_torque_limit: f32,
    pub freefall_turn_threshold: f32,
    pub freefall_turn_near: f32,
    pub freefall_turn_far: f32,
    pub freefall_turn_sign: [f32; 2],
    pub freefall_decay: f32,
    pub freefall_decay_interval: f32,
    pub freefall_walk_threshold: f64,
    /// 017b0218: RunningNoPhysics acceleration limits with / without floor.
    pub kinematic_limits: [f32; 2],
}

fn immediate(
    image: &NativeImage,
    at: u32,
    length: usize,
    mnemonic: Mnemonic,
) -> Result<u32, String> {
    let i = Decoder::with_ip(
        32,
        image.region(at, length)?,
        u64::from(at),
        DecoderOptions::NONE,
    )
    .decode();
    if i.len() != length || i.mnemonic() != mnemonic {
        return Err(format!("unexpected Humanoid immediate at {at:08x}"));
    }
    Ok(i.immediate(if mnemonic == Mnemonic::Push { 0 } else { 1 }) as u32)
}

impl HumanoidStateProfile {
    pub fn load(bundle: &Bundle) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let float = |at| -> Result<f32, String> {
            let v = f32::from_le_bytes(image.region(at, 4)?.try_into().unwrap());
            if !v.is_finite() {
                return Err(format!("non-finite Humanoid constant {at:08x}"));
            }
            Ok(v)
        };
        let floats = |at: u32, n: u32| -> Result<Vec<f32>, String> {
            (0..n).map(|i| float(at + 4 * i)).collect()
        };
        let imm = |at, len, m| -> Result<f32, String> {
            Ok(f32::from_bits(immediate(&image, at, len, m)?))
        };
        let mut table = Vec::new();
        for row in 0..18u32 {
            let bytes = image.region(0x017af5d0 + row * 0x6c, 0x6c)?;
            let words: Vec<i32> = bytes
                .chunks_exact(4)
                .map(|c| i32::from_le_bytes(c.try_into().unwrap()))
                .collect();
            // 00bfd1c0 asserts STATE_TABLE[s+1][0]==s and STATE_TABLE[0][e+1]==e.
            let expected = if row == 0 { NONE } else { row as i32 - 1 };
            if words[0] != expected || (row == 0 && (0..EVENTS).any(|e| words[e + 1] != e as i32)) {
                return Err("unexpected native Humanoid state table".into());
            }
            table.push(std::array::from_fn(|e| words[e + 1]));
        }
        let offsets = floats(0x017afd68, 5)?;
        let lengths = floats(0x017af5c0, 2)?;
        let sign = floats(0x01722200, 2)?;
        let limits = floats(0x017b0218, 2)?;
        let result = Self {
            table,
            ray_offsets: std::array::from_fn(|i| offsets[i]),
            ray_length_factors: [lengths[0], lengths[1]],
            ray_extent: float(0x016f753c)?,
            fast_fall_speed: float(0x01705dd0)?,
            no_floor_seconds: float(0x01708b7c)?,
            fall_down_tilt: float(0x01782284)?,
            running_idle_seconds: float(0x015d8978)?,
            jump_seconds: imm(0x00c05dda, 7, Mnemonic::Mov)?,
            jump_direction_y: imm(0x00c05cdf, 10, Mnemonic::Mov)?,
            jump_gain: float(0x017acbf8)?,
            landed_seconds: imm(0x00c08831, 7, Mnemonic::Mov)?,
            landed_balance: [
                imm(0x00c0880c, 5, Mnemonic::Push)?,
                imm(0x00c08807, 5, Mnemonic::Push)?,
            ],
            freefall_balance_gain: imm(0x00bd3bb5, 10, Mnemonic::Mov)?,
            air_gain: float(0x017af5a0)?,
            air_limit_squared: float(0x017acafc)?,
            air_limit: float(0x017acb00)?,
            freefall_yaw_gain: float(0x0179fc24)?,
            freefall_torque_limit: float(0x017acaf8)?,
            freefall_turn_threshold: float(0x017822bc)?,
            freefall_turn_near: float(0x018368f8)?,
            freefall_turn_far: float(0x01834cd4)?,
            freefall_turn_sign: [sign[0], sign[1]],
            freefall_decay: float(0x017abb4c)?,
            freefall_decay_interval: float(0x0179fbfc)?,
            freefall_walk_threshold: f64::from_le_bytes(
                image.region(0x017bb970, 8)?.try_into().unwrap(),
            ),
            kinematic_limits: [limits[0], limits[1]],
        };
        if result.ray_extent <= 0.0
            || result.jump_seconds <= 0.0
            || result.landed_seconds <= 0.0
            || result.jump_direction_y != 1.0
            || result.air_limit * result.air_limit != result.air_limit_squared
        {
            return Err(format!("unexpected Humanoid state constants {result:?}"));
        }
        Ok(result)
    }

    /// FUN_00bfd1c0.
    pub fn target(&self, state: StateType, event: usize) -> i32 {
        self.table[state as usize + 1][event]
    }

    /// FUN_00c0090e: first event, in order, whose target differs and whose
    /// condition holds. All 17 states are enabled by default (00bd7b06).
    pub fn transition(
        &self,
        state: StateType,
        mut condition: impl FnMut(usize) -> Result<bool, String>,
    ) -> Result<Option<StateType>, String> {
        for event in 0..EVENTS {
            let target = self.target(state, event);
            if target == NONE || target == state as i32 {
                continue;
            }
            if condition(event)? {
                return StateType::from_native(target).map(Some);
            }
        }
        Ok(None)
    }

    /// Which state change events (00c0132c 0xd/0xe/0xf) enable findFloor (00c039ba).
    pub fn searches_floor(&self, state: StateType) -> bool {
        [13, 15, 14].iter().any(|e| self.target(state, *e) != NONE)
    }

    /// FUN_00bd3e14's yaw-rate rule, legacy solver branch.
    pub fn freefall_yaw_rate(&self, delta: f32) -> f32 {
        let sign = self.freefall_turn_sign[usize::from(0.0 <= delta)];
        let magnitude = delta.abs();
        if magnitude <= self.freefall_turn_threshold {
            sign * self.freefall_turn_near * magnitude
        } else {
            sign * self.freefall_turn_far
        }
    }
}

/// HumanoidState.cpp:1430 ray result: hit point and surface normal.
pub type Hit = ([f32; 3], [f32; 3]);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FloorQuery {
    /// +0x60/+0x6c/+0x78: first primary hit.
    pub floor: Option<Hit>,
    /// +0x84: mean of every hit, used for the support height.
    pub average: Option<[f32; 3]>,
}

/// FUN_00c003f4/00c02606. `torso_size` and `leg_height` are Part sizes; rays
/// point along -worldUp and ignore the character's own assembly.
pub fn find_floor(
    profile: &HumanoidStateProfile,
    torso: &CoordinateFrame,
    torso_size: [f32; 3],
    leg_height: Option<f32>,
    had_floor: bool,
    torso_velocity_y: f32,
    mut raycast: impl FnMut([f32; 3], [f32; 3], f32) -> Option<Hit>,
) -> FloorQuery {
    let mut factor = profile.ray_length_factors[usize::from(had_floor)];
    let speed = torso_velocity_y.abs();
    if profile.fast_fall_speed < speed {
        factor += speed / profile.fast_fall_speed;
    }
    let length = leg_height.map_or(1.0, |leg| leg * factor + 1.0);
    let x_extent = torso_size[0];
    let y = -(torso_size[1] * profile.ray_extent);
    let z_extent = torso_size[2] * profile.ray_extent;
    let down = [-0.0, -1.0, -0.0];
    let mut result = FloorQuery::default();
    let (mut sum, mut count) = ([0.0f32; 3], 0u32);
    let mut cast = |local: [f32; 3], primary: bool, result: &mut FloorQuery| {
        let r = torso.rotation;
        let origin: [f32; 3] = std::array::from_fn(|a| {
            r[a][2] * local[2] + r[a][1] * local[1] + r[a][0] * local[0] + torso.position[a]
        });
        if let Some(hit) = raycast(origin, down, length) {
            sum = std::array::from_fn(|a| hit.0[a] + sum[a]);
            count += 1;
            if primary && result.floor.is_none() {
                result.floor = Some(hit);
            }
        }
    };
    for (i, offset) in profile.ray_offsets.iter().enumerate() {
        cast([0.0, y, offset * z_extent], i < 3, &mut result);
    }
    if result.floor.is_none() {
        for x in [-1.0f32, 1.0] {
            for z in [-1.0f32, 1.0] {
                cast(
                    [x * x_extent * profile.ray_extent, y, z * z_extent],
                    true,
                    &mut result,
                );
            }
        }
        if result.floor.is_none() {
            return result;
        }
    }
    let inverse = 1.0 / count as f32;
    result.average = Some(sum.map(|v| v * inverse));
    result
}

/// FUN_00bff30e/00c03048 with HipHeight disabled (flag 01d73b90 is BSS).
pub fn support_height(average_y: f32, torso_height: f32, leg_height: Option<f32>) -> f32 {
    let mut offset = torso_height * 0.5 + 0.0;
    if let Some(leg) = leg_height {
        offset += leg;
    }
    average_y + offset
}

/// FUN_006b1ff4: R' = AxisAngle(worldUp, angle) * R.
pub fn yaw_frame(rotation: [[f32; 3]; 3], angle: f32) -> [[f32; 3]; 3] {
    let (s, c) = angle.sin_cos();
    let axis = [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]];
    std::array::from_fn(|r| {
        std::array::from_fn(|k| (0..3).map(|j| axis[r][j] * rotation[j][k]).sum())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> HumanoidStateProfile {
        let mut table = vec![[NONE; EVENTS]; 18];
        table[0] = std::array::from_fn(|e| e as i32);
        table[9][2] = 3; // Running jump
        table[9][14] = 5; // Running no-floor
        table[9][22] = 10;
        table[6][15] = 7;
        HumanoidStateProfile {
            table,
            ray_offsets: [0.0, 1.0, -1.0, 2.0, -2.0],
            ray_length_factors: [1.1, 1.5],
            ray_extent: 0.4,
            fast_fall_speed: 100.0,
            no_floor_seconds: 0.125,
            fall_down_tilt: 0.7,
            running_idle_seconds: 1.0,
            jump_seconds: 0.5,
            jump_direction_y: 1.0,
            jump_gain: 500.0,
            landed_seconds: 0.05,
            landed_balance: [7500.0, 50.0],
            freefall_balance_gain: 5000.0,
            air_gain: 1250.0,
            air_limit_squared: 20449.0,
            air_limit: 143.0,
            freefall_yaw_gain: 7500.0,
            freefall_torque_limit: 120000.0,
            freefall_turn_threshold: 0.2,
            freefall_turn_near: 1.5,
            freefall_turn_far: 6.0,
            freefall_turn_sign: [-1.0, 1.0],
            freefall_decay: -0.05,
            freefall_decay_interval: 0.033333335,
            freefall_walk_threshold: 0.1,
            kinematic_limits: [500.0, 143.0],
        }
    }
    fn flat(floor: f32) -> impl FnMut([f32; 3], [f32; 3], f32) -> Option<Hit> {
        move |o, d, len| {
            (d[1] < 0.0 && o[1] > floor && o[1] - floor <= len)
                .then_some(([o[0], floor, o[2]], [0.0, 1.0, 0.0]))
        }
    }
    #[test]
    fn floor_rays_use_previous_floor_hysteresis_and_leg_length() {
        let p = profile();
        let torso = CoordinateFrame {
            position: [0.0, 18.0, 0.0],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        let q = find_floor(
            &p,
            &torso,
            [2.0, 2.0, 1.0],
            Some(2.0),
            false,
            0.0,
            flat(15.0),
        );
        assert_eq!(q.floor.unwrap().0, [0.0, 15.0, 0.0]);
        assert_eq!(q.average.unwrap()[1], 15.0);
        assert_eq!(support_height(15.0, 2.0, Some(2.0)), 18.0);
        // Origin 17.2; new floor rays reach 3.2, retained floor rays 4.0.
        let high = CoordinateFrame {
            position: [0.0, 18.5, 0.0],
            ..torso.clone()
        };
        assert!(
            find_floor(
                &p,
                &high,
                [2.0, 2.0, 1.0],
                Some(2.0),
                false,
                0.0,
                flat(14.4)
            )
            .floor
            .is_none()
        );
        assert!(
            find_floor(&p, &high, [2.0, 2.0, 1.0], Some(2.0), true, 0.0, flat(14.4))
                .floor
                .is_some()
        );
        // Fast falling extends the ray by |vy|/100.
        assert!(
            find_floor(
                &p,
                &high,
                [2.0, 2.0, 1.0],
                Some(2.0),
                false,
                -200.0,
                flat(12.0)
            )
            .floor
            .is_some()
        );
    }
    #[test]
    fn transitions_follow_event_order_and_freefall_turn_rule() {
        let p = profile();
        let next = p
            .transition(StateType::Running, |e| Ok(e == 14 || e == 22))
            .unwrap();
        assert_eq!(next, Some(StateType::Freefall));
        assert_eq!(
            p.transition(StateType::Running, |_| Ok(false)).unwrap(),
            None
        );
        assert!(p.searches_floor(StateType::Freefall));
        assert_eq!(p.freefall_yaw_rate(0.1), 1.5 * 0.1);
        assert_eq!(p.freefall_yaw_rate(-1.0), -6.0);
        let r = yaw_frame(
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            std::f32::consts::FRAC_PI_2,
        );
        assert!((r[0][2] - 1.0).abs() < 1e-6 && (r[2][0] + 1.0).abs() < 1e-6);
    }
}
