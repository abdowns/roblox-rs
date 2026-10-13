//! The bundled, no-tool procedural Lua branch, selected explicitly for research.
//! Walking explicitly selects this offline branch. It does not reproduce the
//! default keyframe clips referenced remotely by the shipped keyframe scripts.
use crate::{bundle::Bundle, native_data::NativeImage, rig::R6Rig};
use sha2::{Digest, Sha256};
use std::fmt::Write;

#[derive(Clone, Copy, Debug)]
pub enum Pose {
    Standing,
    Running,
    Jumping,
    FreeFall,
    Climbing,
    Seated,
}

pub struct ProceduralAnimation {
    pub source_sha256: String,
    pub wait_seconds: f64,
    jump_velocity: f32,
    raised_angle: f64,
    seated_velocity: f32,
    run_velocity: f32,
    run_threshold: f32,
    climb_velocity: f32,
    active_amplitude: f64,
    active_frequency: f64,
    idle_amplitude: f64,
    idle_frequency: f64,
}
/// Native Running event deadband, 00c02f36. Walking supplies nonnegative speed,
/// for which the signed/unsigned native branches produce the same result.
pub(crate) struct RunningSignal {
    pub(crate) speed: f32,
    cutoff: f32,
    low: f32,
    high: f32,
}
impl RunningSignal {
    pub(crate) fn load(bundle: &Bundle) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let float = |at| -> Result<f32, String> {
            Ok(f32::from_le_bytes(image.region(at, 4)?.try_into().unwrap()))
        };
        Ok(Self {
            speed: 0.0,
            cutoff: float(0x016eb8c4)?,
            low: float(0x01768c6c)?,
            high: float(0x017a5b8c)?,
        })
    }
    pub(crate) fn observe(&mut self, speed: f32) -> Result<bool, String> {
        if !speed.is_finite() || speed < 0.0 {
            return Err("invalid Running signal speed".into());
        }
        let value = if speed < self.cutoff { 0.0 } else { speed };
        if self.speed * self.low <= value && value <= self.speed * self.high {
            return Ok(false);
        }
        self.speed = value;
        Ok(true)
    }
}
fn literal(source: &str, prefix: &str) -> Result<f64, String> {
    let tail = source
        .split_once(prefix)
        .ok_or_else(|| format!("missing procedural contract {prefix:?}"))?
        .1;
    let number = tail
        .trim_start()
        .split(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
        .next()
        .unwrap_or("");
    let value = number
        .parse::<f64>()
        .map_err(|_| format!("invalid procedural literal {prefix:?}"))?;
    if !value.is_finite() {
        return Err("non-finite animation literal".into());
    }
    Ok(value)
}
impl ProceduralAnimation {
    pub fn load(bundle: &Bundle) -> Result<Self, String> {
        let bytes = bundle.read(
            "Contents/Resources/content/fonts/humanoidAnimateLocal.rbxm",
            1024 * 1024,
        )?;
        let doc =
            roxmltree::Document::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let source = doc
            .descendants()
            .find(|n| n.attribute("name") == Some("Source"))
            .and_then(|n| n.text())
            .ok_or("missing bundled animation source")?;
        // This is a bounded semantic port, not a general Lua interpreter. Parse
        // original literals in their original branches; fail on changed contracts.
        let run = source
            .split_once("if (pose == \"Running\") then")
            .ok_or("missing Running branch")?
            .1;
        let climb = run
            .split_once("elseif (pose == \"Climbing\") then")
            .ok_or("missing Climbing branch")?
            .1;
        let idle = climb.split_once("else").ok_or("missing Standing branch")?.1;
        let sit = source
            .split_once("function moveSit()")
            .ok_or("missing Seated branch")?
            .1;
        for contract in [
            "amplitude * math.sin(time*frequency)",
            "RightHip:SetDesiredAngle(-desiredAngle)",
            "LeftHip:SetDesiredAngle(-desiredAngle)",
            "RightShoulder:SetDesiredAngle(desiredAngle + climbFudge)",
            "LeftShoulder:SetDesiredAngle(desiredAngle - climbFudge)",
        ] {
            if !source.contains(contract) {
                return Err(format!("unsupported animation contract {contract}"));
            }
        }
        Ok(Self {
            source_sha256: format!("{:x}", Sha256::digest(&bytes)),
            wait_seconds: literal(source, "local _, time = wait(")?,
            jump_velocity: literal(source, "local jumpMaxLimbVelocity =")? as f32,
            raised_angle: literal(source, "RightShoulder:SetDesiredAngle(")?,
            seated_velocity: literal(sit, "RightShoulder.MaxVelocity =")? as f32,
            run_velocity: literal(
                run.split_once("else")
                    .ok_or("missing Running velocity branch")?
                    .1,
                "RightShoulder.MaxVelocity =",
            )? as f32,
            run_threshold: literal(run, "RightShoulder.CurrentAngle >")? as f32,
            climb_velocity: literal(climb, "RightShoulder.MaxVelocity =")? as f32,
            active_amplitude: literal(run, "amplitude =")?,
            active_frequency: literal(run, "frequency =")?,
            idle_amplitude: literal(idle, "amplitude =")?,
            idle_frequency: literal(idle, "frequency =")?,
        })
    }
    pub fn apply(&self, rig: &mut R6Rig, pose: Pose, time: f64) -> Result<(), String> {
        if !time.is_finite() {
            return Err("non-finite animation time".into());
        }
        if rig.joints.len() != 6 {
            return Err("procedural animation requires native R6 rig".into());
        }
        let targets;
        match pose {
            Pose::Jumping | Pose::FreeFall => {
                targets = [
                    self.raised_angle as f32,
                    -self.raised_angle as f32,
                    0.0,
                    0.0,
                ];
                for j in &mut rig.joints[1..3] {
                    j.max_velocity = self.jump_velocity;
                }
            }
            Pose::Seated => {
                let a = (self.raised_angle / 2.0) as f32;
                targets = [a, -a, a, -a];
                for j in &mut rig.joints[1..3] {
                    j.max_velocity = self.seated_velocity;
                }
            }
            _ => {
                let (amplitude, frequency, fudge) = match pose {
                    Pose::Running => {
                        for j in &mut rig.joints[1..3] {
                            j.max_velocity = if j.current_angle > self.run_threshold
                                || j.current_angle < -self.run_threshold
                            {
                                self.jump_velocity
                            } else {
                                self.run_velocity
                            };
                        }
                        (self.active_amplitude, self.active_frequency, 0.0)
                    }
                    Pose::Climbing => {
                        for j in &mut rig.joints[1..3] {
                            j.max_velocity = self.climb_velocity;
                        }
                        (
                            self.active_amplitude,
                            self.active_frequency,
                            self.raised_angle,
                        )
                    }
                    _ => (self.idle_amplitude, self.idle_frequency, 0.0),
                };
                let angle = amplitude * (time * frequency).sin();
                targets = [
                    (angle + fudge) as f32,
                    (angle - fudge) as f32,
                    -angle as f32,
                    -angle as f32,
                ];
            }
        }
        for (joint, target) in rig.joints[1..5].iter_mut().zip(targets) {
            joint.set_desired_angle(target)?;
        }
        Ok(())
    }
}

pub fn audit(bundle: &Bundle) -> Result<String, String> {
    let scene = crate::geometry::RenderScene::character(bundle)?.scene;
    let mut rig = R6Rig::character(bundle, &scene)?;
    let root = &scene
        .instances
        .iter()
        .find(|i| i.name == "HumanoidRootPart")
        .and_then(|i| i.part.as_ref())
        .ok_or("missing animation root")?
        .frame;
    let original = rig.pose(root)?;
    let profile = ProceduralAnimation::load(bundle)?;
    let mut report = format!(
        "animation_source profile=bundled_procedural_no_tools sha256={} script_wait={} native_script_selection=not_tested joint_callback_hz=explicit_probe_60\n",
        profile.source_sha256, profile.wait_seconds
    );
    let mut sample = 0u32;
    let mut max_residual = 0.0f32;
    for pose in [
        Pose::Standing,
        Pose::Running,
        Pose::Jumping,
        Pose::FreeFall,
        Pose::Climbing,
        Pose::Seated,
    ] {
        for _ in 0..60 {
            sample += 1;
            if sample.is_multiple_of(6) {
                profile.apply(&mut rig, pose, f64::from(sample) / 60.0)?;
            }
            for joint in &mut rig.joints {
                joint.step()?;
            }
            let frames = rig.pose(root)?;
            for j in &rig.joints {
                let a = crate::rig::compose(&frames[&j.part0], &j.c0).position;
                let b = crate::rig::compose(&frames[&j.part1], &j.c1).position;
                max_residual = max_residual.max(
                    a.into_iter()
                        .zip(b)
                        .map(|(a, b)| (a - b).abs())
                        .fold(0.0f32, f32::max),
                );
            }
        }
        writeln!(
            report,
            "animation_pose pose={pose:?} sample={sample} angles={:?} right_arm={:?}",
            rig.joints
                .iter()
                .map(|j| j.current_angle)
                .collect::<Vec<_>>(),
            rig.pose(root)?["Right Arm"].position
        )
        .unwrap();
    }
    if max_residual > 0.00001 {
        return Err("procedural animation detached a pivot".into());
    }
    rig.reset();
    if rig.pose(root)? != original {
        return Err("animation reset changed rest pose".into());
    }
    writeln!(report,"animation_pass poses=6 callbacks=360 max_pivot_error={max_residual} reset_identical=true tools=not_implemented keyframe_clips=not_loaded native_sine_parity=not_tested").unwrap();
    Ok(report)
}

#[cfg(test)]
mod signal_tests {
    use super::*;
    #[test]
    fn running_event_ignores_small_speed_noise_and_stops_below_cutoff() {
        let mut signal = RunningSignal {
            speed: 0.0,
            cutoff: 0.5,
            low: 0.95,
            high: 1.05,
        };
        assert!(!signal.observe(0.499).unwrap());
        assert!(signal.observe(16.0).unwrap());
        assert!(!signal.observe(15.5).unwrap());
        assert!(!signal.observe(16.5).unwrap());
        assert_eq!(signal.speed, 16.0);
        assert!(signal.observe(0.499).unwrap());
        assert_eq!(signal.speed, 0.0);
        assert!(!signal.observe(1e-20).unwrap());
        assert!(signal.observe(0.5).unwrap());
        assert!(signal.observe(f32::NAN).is_err());
    }
}
