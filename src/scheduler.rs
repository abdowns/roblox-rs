//! Recovered default task/job ordering for Motor6D, Lua wait and world steps.
//! Native wall-clock jitter and TaskScheduler thread interleaving are not
//! reproduced: frames are ideal 1/physics_job_hz ticks, ordered explicitly.
use crate::{bundle::Bundle, native_data::NativeImage};
use iced_x86::{Decoder, DecoderOptions, Instruction, Mnemonic};
use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub struct SchedulerProfile {
    /// TaskScheduler cyclic executive, data default at 01cd5500.
    pub cyclic_executive: bool,
    /// PhysicsJob sleep rate 006bfeea: table 01749cf0 indexed by flag==0.
    pub physics_job_hz: f64,
    /// Constant dt handed to DataModel::physicsStep by PhysicsJob 006bff80.
    pub physics_job_dt: f32,
    /// World::step returns steps*00a91fa2; RunService receives the same.
    pub world_step_dt: f32,
    /// 00a91f38, multiplied by dt in 00ba9892.
    pub world_steps_per_second: i32,
    /// 00ba98ff: fixed count used only when the cyclic executive is off.
    pub legacy_world_steps: i32,
    /// MaxMissedWorldStepsRemembered, 01d3e604.
    pub max_missed_world_steps: i32,
    /// 00ba997d/017ab968 accumulator ceiling.
    pub accumulator_limit: f32,
    /// WaitingScriptsJob 005c6b96 sleep rate.
    pub waiting_scripts_hz: f64,
    /// HeartbeatTask constructor 006c05ca sleep rate.
    pub heartbeat_hz: f64,
    /// Lua settings constructor 005a132c +0x50, applied by 005b9b6a.
    pub minimum_wait: f64,
}

fn decode(image: &NativeImage, address: u32, length: usize) -> Result<Instruction, String> {
    let i = Decoder::with_ip(
        32,
        image.region(address, length)?,
        u64::from(address),
        DecoderOptions::NONE,
    )
    .decode();
    if i.len() != length {
        return Err(format!("unexpected scheduler instruction at {address:08x}"));
    }
    Ok(i)
}

impl SchedulerProfile {
    pub fn load(bundle: &Bundle) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let float = |at| -> Result<f32, String> {
            Ok(f32::from_le_bytes(image.region(at, 4)?.try_into().unwrap()))
        };
        let double = |at| -> Result<f64, String> {
            Ok(f64::from_le_bytes(image.region(at, 8)?.try_into().unwrap()))
        };
        let immediate = |at, length, mnemonic| -> Result<u32, String> {
            let i = decode(&image, at, length)?;
            if i.mnemonic() != mnemonic {
                return Err(format!("unexpected scheduler immediate at {at:08x}"));
            }
            Ok(i.immediate(if mnemonic == Mnemonic::Push { 0 } else { 1 }) as u32)
        };
        let job_double = |high, high_length, low, low_length, mnemonic| -> Result<f64, String> {
            Ok(f64::from_bits(
                u64::from(immediate(high, high_length, mnemonic)?) << 32
                    | u64::from(immediate(low, low_length, mnemonic)?),
            ))
        };
        let cyclic_executive = image.region(0x01cd5500, 1)?[0] != 0;
        // CyclicExecutiveThrottlingCancelWorldStepAccum (01d737fc) has no file
        // bytes: it is zero-filled, so 00ba9892 takes its unthrottled branch.
        if image.region(0x01d737fc, 1).is_ok() {
            return Err("unexpected file-backed throttling flag".into());
        }
        let result = Self {
            cyclic_executive,
            physics_job_hz: double(0x01749cf0 + 8 * u32::from(!cyclic_executive))?,
            physics_job_dt: float(0x0179fbf8)?,
            world_step_dt: float(0x0179fc00)?,
            world_steps_per_second: immediate(0x00a91f3b, 5, Mnemonic::Mov)? as i32,
            legacy_world_steps: immediate(0x00ba98ff, 7, Mnemonic::Mov)? as i32,
            max_missed_world_steps: i32::from_le_bytes(
                image.region(0x01d3e604, 4)?.try_into().unwrap(),
            ),
            accumulator_limit: f32::from_bits(immediate(0x00ba997d, 7, Mnemonic::Mov)?),
            waiting_scripts_hz: job_double(0x005c6b9c, 5, 0x005c6ba1, 2, Mnemonic::Push)?,
            heartbeat_hz: job_double(0x006c0717, 10, 0x006c0721, 10, Mnemonic::Mov)?,
            minimum_wait: double(0x0173dfa0)?,
        };
        if result.accumulator_limit != float(0x017ab968)?
            || !result.cyclic_executive
            || result.world_steps_per_second <= 0
            || result.max_missed_world_steps <= 0
            || !(result.physics_job_dt > 0.0 && result.world_step_dt > 0.0)
            || !(result.minimum_wait > 0.0 && result.minimum_wait.is_finite())
            || result.heartbeat_hz <= 0.0
            || result.physics_job_hz <= 0.0
            || result.physics_job_hz % result.heartbeat_hz != 0.0
            || result.waiting_scripts_hz != result.physics_job_hz
        {
            // Other combinations need a native multi-rate interleaving model.
            return Err(format!("unsupported native scheduler profile {result:?}"));
        }
        Ok(result)
    }
}

/// What one ideal scheduler frame did, in the explicit order Heartbeat,
/// WaitingScripts, then PhysicsJob (gameStepped, World::uiStep, world steps).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameEvents {
    pub frame: u64,
    pub clock: f64,
    pub heartbeat: bool,
    /// `local _, time = wait(...)` second value: RunService+0x78.
    pub resumed_time: Option<f64>,
    /// Zero skips physicsStep entirely, including joint stepUi.
    pub world_steps: u32,
    /// DataModel::physicsStep passes (counter & 1) == 0 as World longStep.
    pub long_step: bool,
}

#[derive(Clone, Debug)]
pub struct NativeSchedule {
    profile: SchedulerProfile,
    pub frame: u64,
    accumulator: f32,
    pub world_steps_remaining: u32,
    /// RunService+0x78, advanced by heartbeat 006bc80a.
    pub heartbeat_elapsed: f64,
    /// RunService+0x68, advanced by gameStepped 006bc910.
    pub game_time: f64,
    last_heartbeat: f64,
    physics_counter: u32,
    /// Waiting thread: enqueue clock and wake clock (005d9106).
    wait: Option<(f64, f64)>,
    wait_seconds: f64,
    pub resumes: u32,
}

impl NativeSchedule {
    /// The script's loop starts by calling `wait(seconds)` at clock zero.
    pub fn new(profile: &SchedulerProfile, wait_seconds: f64) -> Result<Self, String> {
        let mut schedule = Self {
            profile: profile.clone(),
            frame: 0,
            accumulator: 0.0,
            world_steps_remaining: 0,
            heartbeat_elapsed: 0.0,
            game_time: 0.0,
            last_heartbeat: 0.0,
            physics_counter: 0,
            wait: None,
            wait_seconds,
            resumes: 0,
        };
        schedule.lua_wait(0.0)?;
        Ok(schedule)
    }

    /// Lua wait 005b0774: clamp through 005b9b6a, enqueue with wall-clock now.
    fn lua_wait(&mut self, now: f64) -> Result<(), String> {
        let mut seconds = self.wait_seconds;
        if self.profile.minimum_wait > seconds || (seconds as f32).is_nan() {
            seconds = self.profile.minimum_wait;
        }
        if !seconds.is_finite() {
            return Err("invalid Lua wait duration".into());
        }
        self.wait = Some((now, now + seconds));
        Ok(())
    }

    /// FUN_00ba9892, cyclic and unthrottled branch, in its float order.
    fn world_steps(&mut self) -> u32 {
        let sum = self.profile.world_steps_per_second as f32 * self.profile.physics_job_dt
            + self.accumulator;
        let floor = sum.floor();
        let max = self.profile.max_missed_world_steps as f32;
        let steps = if max < floor { max } else { floor }; // MINSS max, floor
        let remainder = sum - steps;
        // UCOMISS/CMOVBE: keep remainder unless it is above the ceiling.
        self.accumulator = if remainder > self.profile.accumulator_limit {
            self.profile.accumulator_limit
        } else {
            remainder
        };
        if 0.0 < steps { steps as i32 as u32 } else { 0 }
    }

    pub fn begin_frame(&mut self) -> Result<FrameEvents, String> {
        if self.world_steps_remaining != 0 {
            return Err("scheduler frame started before its world steps finished".into());
        }
        let clock = self.frame as f64 / self.profile.physics_job_hz;
        let period = (self.profile.physics_job_hz / self.profile.heartbeat_hz) as u64;
        let heartbeat = self.frame.is_multiple_of(period);
        if heartbeat {
            // HeartbeatTask 006c0ad8 passes (double)(float)stats dt.
            self.heartbeat_elapsed += f64::from((clock - self.last_heartbeat) as f32);
            self.last_heartbeat = clock;
        }
        let mut resumed_time = None;
        if let Some((_, wake)) = self.wait
            && clock >= wake
        {
            // 005d8c58 pushes now-enqueued and RunService+0x78.
            resumed_time = Some(self.heartbeat_elapsed);
            self.resumes += 1;
            self.lua_wait(clock)?;
        }
        let world_steps = self.world_steps();
        let long_step = self.physics_counter & 1 == 0;
        if world_steps > 0 {
            // gameStepped 006bc910 receives (double)((float)steps*(float)dt).
            self.game_time += f64::from(world_steps as f32 * self.profile.world_step_dt);
            self.physics_counter = self.physics_counter.wrapping_add(1);
        }
        self.world_steps_remaining = world_steps;
        let events = FrameEvents {
            frame: self.frame,
            clock,
            heartbeat,
            resumed_time,
            world_steps,
            long_step,
        };
        self.frame += 1;
        Ok(events)
    }

    pub fn finish_world_step(&mut self) -> Result<(), String> {
        self.world_steps_remaining = self
            .world_steps_remaining
            .checked_sub(1)
            .ok_or("world step outside a scheduler frame")?;
        Ok(())
    }
}

pub fn audit(bundle: &Bundle) -> Result<String, String> {
    let profile = SchedulerProfile::load(bundle)?;
    let mut report = format!("scheduler_profile {profile:?}\n");
    let animation = crate::animation::ProceduralAnimation::load(bundle)?;
    let mut schedule = NativeSchedule::new(&profile, animation.wait_seconds)?;
    let mut steps = 0u64;
    let mut heartbeats = 0u32;
    let mut physics_jobs = 0u32;
    let mut resumed = Vec::new();
    for _ in 0..(profile.physics_job_hz as u64 * 2) {
        let events = schedule.begin_frame()?;
        if events.world_steps != 4 {
            return Err(format!("unexpected world step count {events:?}"));
        }
        heartbeats += u32::from(events.heartbeat);
        physics_jobs += u32::from(events.world_steps > 0);
        if let Some(time) = events.resumed_time {
            resumed.push((events.frame, time));
        }
        for _ in 0..events.world_steps {
            schedule.finish_world_step()?;
            steps += 1;
        }
    }
    for (frame, time) in &resumed {
        writeln!(
            report,
            "scheduler_wait_resume frame={frame} heartbeat_time={time}"
        )
        .unwrap();
    }
    let gaps: Vec<_> = resumed.windows(2).map(|w| w[1].0 - w[0].0).collect();
    if gaps.iter().any(|g| !(6..=7).contains(g))
        || heartbeats != 60
        || steps != 480
        || physics_jobs != 120
    {
        return Err(format!("scheduler cadence failed gaps={gaps:?}"));
    }
    writeln!(report,"scheduler_pass seconds=2 physics_jobs={physics_jobs} world_steps={steps} joint_step_ui={physics_jobs} heartbeats={heartbeats} script_resumes={} resume_gap_frames={gaps:?} game_time={} heartbeat_time={} wall_clock=ideal_frames job_order=heartbeat_waiting_physics_explicit",resumed.len(),schedule.game_time,schedule.heartbeat_elapsed).unwrap();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> SchedulerProfile {
        SchedulerProfile {
            cyclic_executive: true,
            physics_job_hz: 60.0,
            physics_job_dt: f32::from_bits(0x3c888889),
            world_step_dt: 1.0 / 240.0,
            world_steps_per_second: 240,
            legacy_world_steps: 8,
            max_missed_world_steps: 12,
            accumulator_limit: f32::from_bits(0x3fffffac),
            waiting_scripts_hz: 60.0,
            heartbeat_hz: 30.0,
            minimum_wait: 0.03,
        }
    }
    #[test]
    fn native_float_accumulator_gives_four_steps_and_caps_missed_steps() {
        let mut schedule = NativeSchedule::new(&profile(), 0.1).unwrap();
        for _ in 0..100_000 {
            assert_eq!(schedule.world_steps(), 4);
        }
        assert_eq!(schedule.accumulator, 0.0);
        let mut slow = profile();
        slow.physics_job_dt = 1.0;
        let mut schedule = NativeSchedule::new(&slow, 0.1).unwrap();
        assert_eq!(schedule.world_steps(), 12);
        assert_eq!(schedule.accumulator, slow.accumulator_limit);
    }
    #[test]
    fn wait_resumes_on_wall_clock_and_returns_heartbeat_time() {
        let mut schedule = NativeSchedule::new(&profile(), 0.1).unwrap();
        let mut resumed = Vec::new();
        for _ in 0..30 {
            let events = schedule.begin_frame().unwrap();
            if let Some(time) = events.resumed_time {
                resumed.push((events.frame, time));
            }
            for _ in 0..events.world_steps {
                schedule.finish_world_step().unwrap();
            }
        }
        // Heartbeats at frames 0,2,4,6 accumulate float-rounded 1/30 steps.
        let step = f64::from((2.0f64 / 60.0) as f32);
        assert_eq!(resumed[0].0, 6);
        assert!((resumed[0].1 - step * 3.0).abs() < 1e-12);
        assert_eq!(resumed[1].0, 12);
        // 0.2 + 0.1 exceeds 18/60 in doubles, so the ideal clock waits a frame.
        assert_eq!(resumed[2].0, 19);
        let mut short = NativeSchedule::new(&profile(), 0.0).unwrap();
        assert_eq!(short.wait, Some((0.0, 0.03)));
        let first = short.begin_frame().unwrap();
        assert!(first.resumed_time.is_none() && first.long_step);
        assert!(short.begin_frame().is_err());
        for _ in 0..first.world_steps {
            short.finish_world_step().unwrap();
        }
        assert!(short.finish_world_step().is_err());
        let second = short.begin_frame().unwrap();
        assert!(second.resumed_time.is_none() && !second.long_step);
        for _ in 0..second.world_steps {
            short.finish_world_step().unwrap();
        }
        assert_eq!(short.begin_frame().unwrap().resumed_time, Some(step));
    }
}
