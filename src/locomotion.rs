//! RunningBase horizontal forces, with explicit assembly and floor inputs.
use crate::{bundle::Bundle, native_data::NativeImage};
use std::fmt::Write;

#[derive(Clone, Copy, Debug)]
pub enum Solver {
    Legacy,
    Pgs,
}
pub struct RunningProfile {
    legacy_gain: f32,
    pgs_gain: f32,
    ground_limit: f32,
    air_limit: f32,
}
pub struct RunningInput {
    pub velocity: [f32; 2],
    pub desired_velocity: [f32; 2],
    pub floor_velocity: [f32; 2],
    pub accumulated_force: [f32; 2],
    pub root_mass: f32,
    /// Native selection: min(root mass, movable-floor mass), or root mass.
    pub effective_mass: f32,
    pub floor_present: bool,
    /// Explicit result of the native floor material-friction query.
    pub floor_friction: f32,
    pub solver: Solver,
}
impl RunningProfile {
    pub fn load(bundle: &Bundle) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let float = |at| -> Result<f32, String> {
            let value = f32::from_le_bytes(image.region(at, 4)?.try_into().unwrap());
            if !value.is_finite() || value <= 0.0 {
                return Err("invalid RunningBase constant".into());
            }
            Ok(value)
        };
        Ok(Self {
            legacy_gain: float(0x017af5a0)?,
            pgs_gain: float(0x017af5a4)?,
            ground_limit: float(0x017acbf8)?,
            air_limit: float(0x017b05c4)?,
        })
    }
    pub fn horizontal_force(&self, input: &RunningInput) -> Result<[f32; 2], String> {
        if input
            .velocity
            .iter()
            .chain(&input.desired_velocity)
            .chain(&input.floor_velocity)
            .chain(&input.accumulated_force)
            .any(|v| !v.is_finite())
            || !input.root_mass.is_finite()
            || input.root_mass <= 0.0
            || !input.effective_mass.is_finite()
            || input.effective_mass <= 0.0
            || !input.floor_friction.is_finite()
            || input.floor_friction < 0.0
        {
            return Err("invalid explicit RunningBase inputs".into());
        }
        let gain = match input.solver {
            Solver::Legacy => self.legacy_gain,
            Solver::Pgs => self.pgs_gain,
        };
        let mut acceleration: [f32; 2] = std::array::from_fn(|a| {
            ((input.floor_velocity[a] + input.desired_velocity[a]) - input.velocity[a]) * gain
                - input.accumulated_force[a] * (1.0 / input.root_mass)
        });
        let limit = if input.floor_present {
            self.ground_limit
        } else {
            self.air_limit
        };
        let squared = acceleration[1] * acceleration[1] + acceleration[0] * acceleration[0] + 0.0;
        if !squared.is_finite() {
            return Err("RunningBase acceleration overflow".into());
        }
        if limit * limit < squared {
            let length = squared.sqrt();
            if length >= 1e-7 {
                if length >= 1.00001 || length <= 0.99999 {
                    acceleration = acceleration.map(|v| v * (1.0 / length));
                }
            } else {
                acceleration = [0.0; 2];
            }
            acceleration = acceleration.map(|v| v * limit);
        }
        let mut force = acceleration.map(|v| v * input.effective_mass);
        if input.floor_present {
            force = force.map(|v| v * input.floor_friction);
        }
        if force.iter().any(|v| !v.is_finite()) {
            return Err("RunningBase force overflow".into());
        }
        Ok(force)
    }
}
pub fn audit(bundle: &Bundle) -> Result<String, String> {
    let profile = RunningProfile::load(bundle)?;
    let mut input = RunningInput {
        velocity: [0.0; 2],
        desired_velocity: [16.0, 0.0],
        floor_velocity: [0.0; 2],
        accumulated_force: [0.0; 2],
        root_mass: 10.0,
        effective_mass: 10.0,
        floor_present: true,
        floor_friction: 1.0,
        solver: Solver::Legacy,
    };
    let mut report = format!(
        "running_profile legacy_gain={} pgs_gain={} ground_acceleration_limit={} air_acceleration_limit={} solver_selection=explicit mass_and_floor=fixture\n",
        profile.legacy_gain, profile.pgs_gain, profile.ground_limit, profile.air_limit
    );
    for (index, (name, expected)) in [
        ("ground_start", [5000.0, 0.0]),
        ("moving_floor_match", [0.0, 0.0]),
        ("air_start", [1430.0, 0.0]),
        ("ground_friction", [1250.0, 0.0]),
        ("pgs_small_error", [150.0, 0.0]),
        ("cancel_accumulated_force", [0.0, 0.0]),
    ]
    .into_iter()
    .enumerate()
    {
        match index {
            1 => {
                input.velocity = [18.0, 0.0];
                input.floor_velocity = [2.0, 0.0];
            }
            2 => {
                input.velocity = [0.0; 2];
                input.floor_velocity = [0.0; 2];
                input.floor_present = false;
            }
            3 => {
                input.floor_present = true;
                input.floor_friction = 0.25;
            }
            4 => {
                input.solver = Solver::Pgs;
                input.desired_velocity = [0.1, 0.0];
                input.floor_friction = 1.0;
            }
            5 => {
                input.accumulated_force = [150.0, 0.0];
            }
            _ => {}
        }
        let force = profile.horizontal_force(&input)?;
        if force
            .into_iter()
            .zip(expected)
            .any(|(a, b)| (a - b).abs() > 0.001)
        {
            return Err(format!("RunningBase fixture mismatch {name}: {force:?}"));
        }
        writeln!(report, "running_force case={name} force_xz={force:?}").unwrap();
    }
    writeln!(report,"running_force_pass cases=6 floor_queries=not_implemented vertical_spring=not_implemented assembly_integration=not_implemented native_differential=not_tested").unwrap();
    Ok(report)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn force_cap_is_radial_and_friction_applies_after_mass() {
        let profile = RunningProfile {
            legacy_gain: 1000.0,
            pgs_gain: 150.0,
            ground_limit: 500.0,
            air_limit: 143.0,
        };
        let mut input = RunningInput {
            velocity: [0.0; 2],
            desired_velocity: [3.0, 4.0],
            floor_velocity: [0.0; 2],
            accumulated_force: [0.0; 2],
            root_mass: 10.0,
            effective_mass: 2.0,
            floor_present: true,
            floor_friction: 0.5,
            solver: Solver::Legacy,
        };
        let f = profile.horizontal_force(&input).unwrap();
        assert!((f[0] - 300.0).abs() < 0.001 && (f[1] - 400.0).abs() < 0.001);
        input.root_mass = 0.0;
        assert!(profile.horizontal_force(&input).is_err());
    }
}
