//! Recovered Humanoid command math. Locomotion forces/state transitions are separate.
use crate::{bundle::Bundle, scene::CoordinateFrame};
use bevy::prelude::*;
use std::fmt::Write;

#[derive(Component, Clone, Debug)]
pub struct HumanoidProperties {
    pub walk_speed: f32,
    pub jump_power: f32,
    pub hip_height: f32,
    pub max_slope_angle: f32,
    pub health: f32,
    pub max_health: f32,
}

impl HumanoidProperties {
    pub fn character(bundle: &Bundle) -> Result<Self, String> {
        let bytes = bundle.read(
            "Contents/Resources/content/fonts/character3.rbxm",
            16 * 1024 * 1024,
        )?;
        Self::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)
    }

    pub fn parse(xml: &str) -> Result<Self, String> {
        let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
        let mut humanoids = doc
            .descendants()
            .filter(|n| n.has_tag_name("Item") && n.attribute("class") == Some("Humanoid"));
        let item = humanoids.next().ok_or("missing Humanoid")?;
        if humanoids.next().is_some() {
            return Err("character probe requires one Humanoid".into());
        }
        let properties = item
            .children()
            .find(|n| n.has_tag_name("Properties"))
            .ok_or("missing Humanoid properties")?;
        let float = |name| -> Result<f32, String> {
            let mut matches = properties
                .children()
                .filter(|n| n.attribute("name") == Some(name));
            let node = matches.next().ok_or_else(|| format!("missing {name}"))?;
            if !node.has_tag_name("float") || matches.next().is_some() {
                return Err(format!("invalid or duplicate {name}"));
            }
            let value: f32 = node
                .text()
                .unwrap_or("")
                .trim()
                .parse()
                .map_err(|_| format!("invalid {name}"))?;
            if !value.is_finite() {
                return Err(format!("non-finite {name}"));
            }
            Ok(value)
        };
        Ok(Self {
            walk_speed: float("WalkSpeed")?,
            jump_power: float("JumpPower")?,
            hip_height: float("HipHeight")?,
            max_slope_angle: float("MaxSlopeAngle")?,
            health: float("Health_XML")?,
            max_health: float("MaxHealth")?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct MoveContext {
    pub attached_to_data_model: bool,
    pub relative_to_camera: bool,
    pub camera: Option<CoordinateFrame>,
    /// Explicit probe branch. Native state dispatch is not implemented here.
    pub swimming: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveOutcome {
    Detached,
    CameraUnavailable,
    Applied { direction_changed: bool },
}

#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct HumanoidMotion {
    pub move_direction: [f32; 3],
    pub raw_unit_direction: [f32; 3],
    pub speed_fraction: f32,
    pub walk_to_active: bool,
}

fn length_squared(v: [f32; 3]) -> Result<f32, String> {
    let squared = v[2] * v[2] + v[1] * v[1] + v[0] * v[0];
    if !v.iter().all(|x| x.is_finite()) || !squared.is_finite() {
        return Err("movement probe requires finite, bounded vectors".into());
    }
    Ok(squared)
}

fn unitize(v: &mut [f32; 3]) -> Result<f32, String> {
    let length = length_squared(*v)?.sqrt();
    if length > 1e-6 {
        let inverse = 1.0 / length;
        *v = v.map(|x| x * inverse);
        Ok(length)
    } else {
        Ok(0.0)
    }
}

fn camera_direction(
    frame: &CoordinateFrame,
    unit: [f32; 3],
    swimming: bool,
) -> Result<[f32; 3], String> {
    if frame
        .position
        .iter()
        .chain(frame.rotation.iter().flatten())
        .any(|x| !x.is_finite())
    {
        return Err("camera probe requires a finite frame".into());
    }
    let rotation = if swimming {
        frame.rotation
    } else {
        // Native copies the frame, lookAt(translation-column2 with Y retained,
        // world-up). Preserve the subtraction rather than ignoring translation.
        let mut forward = [
            (frame.position[0] - frame.rotation[0][2]) - frame.position[0],
            0.0,
            (frame.position[2] - frame.rotation[2][2]) - frame.position[2],
        ];
        if unitize(&mut forward)? == 0.0 {
            return Err("degenerate ground camera basis is outside the probe boundary".into());
        }
        let mut right = [-forward[2], 0.0, forward[0]];
        unitize(&mut right)?;
        let up = [0.0, right[2] * forward[0] - forward[2] * right[0], 0.0];
        std::array::from_fn(|row| [right[row], up[row], -forward[row]])
    };
    let mut result = std::array::from_fn(|i| {
        rotation[i][2] * unit[2] + rotation[i][1] * unit[1] + rotation[i][0] * unit[0]
    });
    if !swimming {
        result[1] = 0.0;
        if unitize(&mut result)? == 0.0 {
            result = [0.0; 3];
        }
    }
    length_squared(result)?;
    Ok(result)
}

impl HumanoidMotion {
    /// Native MoveDirectionInternal setter math for finite inputs. Returns the
    /// property-change decision; reflection/replication events are not emitted.
    pub fn set_internal_direction(&mut self, mut direction: [f32; 3]) -> Result<bool, String> {
        let squared = length_squared(direction)?;
        // Native fuzzyEq(lengthSquared,0) uses the float 1e-5 promoted to double.
        if squared == 0.0
            || f64::from(squared.abs()) <= (f64::from(squared.abs()) + 1.0) * f64::from(1e-5f32)
        {
            direction = [0.0; 3];
        } else if squared.sqrt() > 1.0 {
            let inverse = 1.0 / squared.sqrt();
            direction = direction.map(|x| x * inverse);
        }
        if direction == self.move_direction {
            return Ok(false);
        }
        self.move_direction = direction;
        if direction != [0.0; 3] {
            self.walk_to_active = false;
        }
        Ok(true)
    }

    /// Player:Move forwards to this Humanoid:Move path. Camera absence occurs
    /// after raw direction/fraction updates in the native function.
    pub fn move_input(
        &mut self,
        mut input: [f32; 3],
        context: &MoveContext,
    ) -> Result<MoveOutcome, String> {
        if !context.attached_to_data_model {
            return Ok(MoveOutcome::Detached);
        }
        let squared = length_squared(input)?;
        if input == [0.0; 3] {
            let changed = self.set_internal_direction([0.0; 3])?;
            self.raw_unit_direction = [0.0; 3];
            // Native zero branch leaves the previous speed fraction intact.
            return Ok(MoveOutcome::Applied {
                direction_changed: changed,
            });
        }
        if squared.sqrt() > 1.0 {
            unitize(&mut input)?;
        }
        let length = length_squared(input)?.sqrt();
        if length == 0.0 {
            return Err("underflowed nonzero Move input is outside the probe boundary".into());
        }
        self.speed_fraction = length.min(1.0);
        self.raw_unit_direction = input.map(|x| x * (1.0 / length));
        let direction = if context.relative_to_camera {
            let Some(camera) = &context.camera else {
                return Ok(MoveOutcome::CameraUnavailable);
            };
            camera_direction(camera, self.raw_unit_direction, context.swimming)?
        } else {
            self.raw_unit_direction
        };
        Ok(MoveOutcome::Applied {
            direction_changed: self.set_internal_direction(direction)?,
        })
    }
}

#[derive(Resource)]
struct Probe {
    commands: Vec<([f32; 3], MoveContext)>,
    next: usize,
    report: String,
    error: Option<String>,
}

fn command_step(mut probe: ResMut<Probe>, mut motion: Query<&mut HumanoidMotion>) {
    let Some((input, context)) = probe.commands.get(probe.next).cloned() else {
        return;
    };
    let mut motion = motion.single_mut().unwrap();
    let outcome = motion.move_input(input, &context);
    let step = probe.next;
    writeln!(probe.report, "player_command step={step} input={input:?} outcome={outcome:?} direction={:?} raw={:?} speed_fraction={} walk_to_active={}", motion.move_direction, motion.raw_unit_direction, motion.speed_fraction, motion.walk_to_active).unwrap();
    if let Err(error) = outcome {
        probe.error = Some(error);
    }
    probe.next += 1;
}

pub fn audit(bundle: &Bundle) -> Result<String, String> {
    let properties = HumanoidProperties::character(bundle)?;
    let world = MoveContext {
        attached_to_data_model: true,
        relative_to_camera: false,
        camera: None,
        swimming: false,
    };
    let camera = CoordinateFrame {
        position: [0.0; 3],
        rotation: [[0.0, 0.0, 1.0], [0.6, 0.8, 0.0], [-0.8, 0.6, 0.0]],
    };
    let ground = MoveContext {
        relative_to_camera: true,
        camera: Some(camera),
        ..world.clone()
    };
    let swim = MoveContext {
        swimming: true,
        ..ground.clone()
    };
    let missing = MoveContext {
        relative_to_camera: true,
        ..world.clone()
    };
    let detached = MoveContext {
        attached_to_data_model: false,
        ..world.clone()
    };
    let commands = vec![
        ([0.25, 0.0, 0.0], world.clone()),
        ([1.0, 0.0, 1.0], world.clone()),
        ([0.0; 3], world),
        ([0.0, 0.0, -1.0], ground),
        ([0.0, 0.5, 0.0], swim),
        ([0.0, 0.0, -0.5], missing),
        ([1.0, 0.0, 0.0], detached),
    ];
    let count = commands.len();
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    let entity = app
        .world_mut()
        .spawn((
            properties.clone(),
            HumanoidMotion {
                walk_to_active: true,
                ..Default::default()
            },
        ))
        .id();
    app.insert_resource(Probe { commands, next: 0, report: format!("humanoid_properties walk_speed={} jump_power={} hip_height={} max_slope_angle={} health={} max_health={}\n",properties.walk_speed, properties.jump_power, properties.hip_height, properties.max_slope_angle, properties.health, properties.max_health), error: None });
    app.add_systems(Update, command_step);
    for _ in 0..count {
        app.update();
    }
    let probe = app.world().resource::<Probe>();
    if let Some(error) = &probe.error {
        return Err(error.clone());
    }
    let state = app
        .world()
        .get::<HumanoidMotion>(entity)
        .ok_or("missing Humanoid")?;
    if probe.next != count
        || state.move_direction != [0.0, 0.8, 0.6]
        || state.raw_unit_direction != [0.0, 0.0, -1.0]
        || state.speed_fraction != 0.5
        || state.walk_to_active
    {
        return Err("player command probe state mismatch".into());
    }
    Ok(format!(
        "{}player_command_pass updates={count} locomotion=not_implemented native_behavior_parity=not_tested\n",
        probe.report
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn world() -> MoveContext {
        MoveContext {
            attached_to_data_model: true,
            relative_to_camera: false,
            camera: None,
            swimming: false,
        }
    }
    #[test]
    fn move_preserves_analog_fraction_while_direction_is_unit_and_zero_keeps_fraction() {
        let mut motion = HumanoidMotion {
            walk_to_active: true,
            ..Default::default()
        };
        motion.move_input([0.25, 0.0, 0.0], &world()).unwrap();
        assert_eq!(motion.move_direction, [1.0, 0.0, 0.0]);
        assert_eq!(motion.speed_fraction, 0.25);
        assert!(!motion.walk_to_active);
        motion.move_input([0.0; 3], &world()).unwrap();
        assert_eq!(motion.move_direction, [0.0; 3]);
        assert_eq!(motion.raw_unit_direction, [0.0; 3]);
        assert_eq!(motion.speed_fraction, 0.25);
    }
    #[test]
    fn internal_direction_dead_zone_is_distinct_from_move_command() {
        let mut motion = HumanoidMotion::default();
        assert!(!motion.set_internal_direction([0.003, 0.0, 0.0]).unwrap());
        motion.set_internal_direction([0.004, 0.0, 0.0]).unwrap();
        assert_eq!(motion.move_direction, [0.004, 0.0, 0.0]);
        motion.move_input([0.003, 0.0, 0.0], &world()).unwrap();
        assert_eq!(motion.move_direction, [1.0, 0.0, 0.0]);
        motion.set_internal_direction([3.0, 0.0, 4.0]).unwrap();
        assert_eq!(motion.move_direction, [0.6, 0.0, 0.8]);
    }
    #[test]
    fn detached_is_noop_but_missing_camera_updates_raw_fraction_first() {
        let mut motion = HumanoidMotion::default();
        let before = motion.clone();
        assert_eq!(
            motion
                .move_input(
                    [1.0, 0.0, 0.0],
                    &MoveContext {
                        attached_to_data_model: false,
                        ..world()
                    }
                )
                .unwrap(),
            MoveOutcome::Detached
        );
        assert_eq!(motion, before);
        assert_eq!(
            motion
                .move_input(
                    [0.5, 0.0, 0.0],
                    &MoveContext {
                        relative_to_camera: true,
                        ..world()
                    }
                )
                .unwrap(),
            MoveOutcome::CameraUnavailable
        );
        assert_eq!(motion.move_direction, [0.0; 3]);
        assert_eq!(motion.raw_unit_direction, [1.0, 0.0, 0.0]);
        assert_eq!(motion.speed_fraction, 0.5);
    }
    #[test]
    fn camera_ground_projection_and_swimming_take_different_paths() {
        let frame = CoordinateFrame {
            position: [10.0, 20.0, 30.0],
            rotation: [[0.0, 0.0, 1.0], [0.6, 0.8, 0.0], [-0.8, 0.6, 0.0]],
        };
        assert_eq!(
            camera_direction(&frame, [0.0, 0.0, -1.0], false).unwrap(),
            [-1.0, 0.0, 0.0]
        );
        assert_eq!(
            camera_direction(&frame, [0.0, 1.0, 0.0], true).unwrap(),
            [0.0, 0.8, 0.6]
        );
        assert_eq!(
            camera_direction(&frame, [0.0, 1.0, 0.0], false).unwrap(),
            [0.0; 3]
        );
    }
    #[test]
    fn malformed_properties_and_nonfinite_inputs_fail_without_guessing_defaults() {
        assert!(HumanoidProperties::parse("<roblox/>").is_err());
        assert!(
            HumanoidMotion::default()
                .move_input([f32::NAN, 0.0, 0.0], &world())
                .is_err()
        );
    }
}
