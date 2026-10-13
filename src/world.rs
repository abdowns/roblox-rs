//! RBX::World subset for the walking fixture: one character assembly, the
//! explicit static contents (floor plane, wall half-space), awake contact
//! impulses, and kernel substeps with a per-substep force callback (the
//! Humanoid state's kernel connector, vtable +0xc). The static contents are a
//! fixture, not native Workspace parts.
use crate::{
    assembly::{AssemblyProperties, MassPart, MassProfile, Tensor},
    bundle::Bundle,
    collision::ContactBodyState,
    humanoid_state::Hit,
    kernel::{ContactParameters, ContactSpring, KernelBody, KernelProfile, cross},
    legacy_contact::{AwakeContact, BlockContactProfile, ImpulseInput, ImpulseProfile},
    physics,
    rig::R6Rig,
    scene::{CoordinateFrame, Scene},
};
use std::collections::BTreeMap;

struct ContactState {
    spring: ContactSpring,
    impulse: AwakeContact,
}

/// Fixture static contents and the collision queries the Humanoid makes
/// against them (floor/ceiling rays, PopperCam, touch tests).
pub struct Statics {
    pub floor_y: f32,
    pub wall_x: f32,
    geometry: BlockContactProfile,
}

impl Statics {
    /// Floor plane (solid below) and wall half-space.
    pub fn raycast(&self, origin: [f32; 3], direction: [f32; 3], length: f32) -> Option<Hit> {
        let mut best: Option<(f32, Hit)> = None;
        if direction[1] < 0.0 && origin[1] > self.floor_y {
            let t = (origin[1] - self.floor_y) / -direction[1];
            if t <= length {
                best = Some((
                    t,
                    (
                        [
                            origin[0] + direction[0] * t,
                            self.floor_y,
                            origin[2] + direction[2] * t,
                        ],
                        [0.0, 1.0, 0.0],
                    ),
                ));
            }
        }
        if direction[0] > 0.0 && origin[0] < self.wall_x {
            let t = (self.wall_x - origin[0]) / direction[0];
            if t <= length && best.as_ref().is_none_or(|(b, _)| t < *b) {
                best = Some((
                    t,
                    (
                        [
                            self.wall_x,
                            origin[1] + direction[1] * t,
                            origin[2] + direction[2] * t,
                        ],
                        [-1.0, 0.0, 0.0],
                    ),
                ));
            }
        }
        best.map(|(_, hit)| hit)
    }
    pub fn wall(&self) -> CoordinateFrame {
        CoordinateFrame {
            position: [self.wall_x, self.floor_y, 0.0],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        }
    }
    /// Fixture stand-in for a touching kernel contact: the part generates
    /// a vertex/plane connector against the wall.
    pub fn touching(&self, frame: &CoordinateFrame, size: [f32; 3]) -> Result<bool, String> {
        Ok(!self
            .geometry
            .plane_vertices(frame, size, &self.wall(), 3)?
            .is_empty())
    }
}

pub struct World {
    pub body: KernelBody,
    pub statics: Statics,
    pub kernel: KernelProfile,
    pub steps: u32,
    pub contacts: usize,
    pub impulse_applications: u64,
    pub max_penetration: f32,
    /// Current assembly inertia in body space (animated Cofm refresh).
    pub inertia: Tensor,
    original: KernelBody,
    original_inertia: Tensor,
    impulses: ImpulseProfile,
    colliders: Vec<(String, [f32; 3])>,
    states: BTreeMap<(String, usize), ContactState>,
    mass_parts: BTreeMap<String, MassPart>,
}

impl World {
    pub fn new(bundle: &Bundle, scene: &Scene, rig: &R6Rig) -> Result<Self, String> {
        let kernel = KernelProfile::load(bundle)?;
        let parts = MassProfile::load(bundle)?.character_parts(bundle, scene)?;
        let assembly = AssemblyProperties::from_rig(&parts, rig)?;
        let root = physics::character_bodies(bundle)?
            .into_iter()
            .find(|(n, _)| n == "HumanoidRootPart")
            .ok_or("missing walking root")?
            .1;
        let body = KernelBody::new(&assembly, &root, &kernel)?;
        let bytes = bundle.read(
            "Contents/Resources/content/fonts/character3.rbxm",
            16 * 1024 * 1024,
        )?;
        let xml = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
        let doc = roxmltree::Document::parse(xml).map_err(|e| e.to_string())?;
        let mut colliders = Vec::new();
        let mut floor_y = f32::INFINITY;
        for instance in &scene.instances {
            let Some(part) = &instance.part else { continue };
            if instance.name.ends_with(" Leg") {
                floor_y = floor_y.min(part.frame.position[1] - part.size[1] * 0.5);
            }
            let item = doc
                .descendants()
                .find(|n| {
                    n.has_tag_name("Item") && n.attribute("referent") == Some(&instance.referent)
                })
                .ok_or("missing collider properties")?;
            let properties = item
                .children()
                .find(|n| n.has_tag_name("Properties"))
                .ok_or("missing collider properties")?;
            match properties
                .children()
                .find(|n| n.attribute("name") == Some("CanCollide"))
                .and_then(|n| n.text())
            {
                Some("true") if part.shape == 1 => {
                    colliders.push((instance.name.clone(), part.size))
                }
                Some("false") => {}
                _ => return Err("unsupported original walking collider".into()),
            }
        }
        if colliders.len() != 2 || !floor_y.is_finite() {
            return Err("unexpected walking fixture geometry".into());
        }
        Ok(Self {
            original: body.clone(),
            body,
            statics: Statics {
                floor_y,
                wall_x: root.frame.position[0] + 4.0,
                geometry: BlockContactProfile::load(bundle)?,
            },
            kernel,
            steps: 0,
            contacts: 0,
            impulse_applications: 0,
            max_penetration: 0.0,
            inertia: assembly.inertia_in_body,
            original_inertia: assembly.inertia_in_body,
            impulses: ImpulseProfile::load(bundle)?,
            colliders,
            states: BTreeMap::new(),
            mass_parts: parts,
        })
    }
    pub fn reset(&mut self) {
        self.body = self.original.clone();
        self.steps = 0;
        self.contacts = 0;
        self.impulse_applications = 0;
        self.max_penetration = 0.0;
        self.states.clear();
        self.inertia = self.original_inertia;
    }
    pub fn is_original(&self) -> bool {
        self.body.frame == self.original.frame && self.inertia == self.original_inertia
    }
    pub fn inertia_changed(&self) -> bool {
        self.inertia != self.original_inertia
    }
    pub fn point_velocity(&self, point: [f32; 3]) -> [f32; 3] {
        let arm = std::array::from_fn(|a| point[a] - self.body.center[a]);
        let spin = cross(self.body.angular_velocity, arm);
        std::array::from_fn(|a| self.body.velocity[a] + spin[a])
    }
    /// The assembly returned to the kernel; contact accumulators restart.
    pub fn clear_contacts(&mut self) {
        self.states.clear();
    }
    /// Motor angles changed: recompute the assembly mass properties (Cofm).
    pub fn refresh_assembly(&mut self, rig: &R6Rig) -> Result<(), String> {
        let frames = rig.pose(&self.body.frame)?;
        let mut parts = self.mass_parts.clone();
        for (name, part) in &mut parts {
            part.frame = frames
                .get(name)
                .ok_or("missing animated mass part")?
                .clone();
        }
        let assembly = AssemblyProperties::from_rig(&parts, rig)?;
        self.body.refresh_assembly(&assembly, &self.kernel)?;
        self.inertia = assembly.inertia_in_body;
        Ok(())
    }
    /// The assembly is placed kinematically (outside the kernel); only touch
    /// detection runs.
    pub fn step_kinematic(&mut self, rig: &R6Rig) -> Result<(), String> {
        let wall = self.statics.wall();
        let frames = rig.pose(&self.body.frame)?;
        for (name, size) in &self.colliders {
            for (_, g) in self.statics.geometry.plane_vertices(
                frames.get(name).ok_or("missing wall collider frame")?,
                *size,
                &wall,
                3,
            )? {
                self.max_penetration = self.max_penetration.max(-g.distance);
            }
        }
        self.states.clear();
        self.contacts = 0;
        self.steps = self.steps.checked_add(1).ok_or("walking tick overflow")?;
        Ok(())
    }
    /// doWorldStep: awake contact impulses, then the kernel substeps with the
    /// force callback (torso frame, current inertia) before each integration.
    pub fn step(
        &mut self,
        rig: &R6Rig,
        parameters: ContactParameters,
        mut force: impl FnMut(&mut KernelBody, &Statics, &CoordinateFrame, Tensor) -> Result<(), String>,
    ) -> Result<(), String> {
        let wall = self.statics.wall();
        let fixed = ContactBodyState {
            center: wall.position,
            velocity: [0.0; 3],
            angular_velocity: [0.0; 3],
        };
        let frames = rig.pose(&self.body.frame)?;
        let mut active = Vec::new();
        for (name, size) in &self.colliders {
            for (vertex, g) in self.statics.geometry.plane_vertices(
                frames.get(name).ok_or("missing wall collider frame")?,
                *size,
                &wall,
                3,
            )? {
                let key = (name.clone(), vertex);
                self.states
                    .entry(key.clone())
                    .or_insert_with(|| ContactState {
                        spring: ContactSpring::default(),
                        impulse: AwakeContact::default(),
                    })
                    .impulse
                    .invalidate();
                active.push((key, g));
            }
        }
        self.states
            .retain(|key, _| active.iter().any(|(k, _)| k == key));
        self.contacts = active.len();
        // Explicit bounded awake fixture scheduling; full native adaptive/sleep
        // scheduling and +0xf0 reference-impulse accumulation remain unported.
        for _ in 0..60 {
            let mut applied = false;
            for (key, g) in &active {
                let input = ImpulseInput {
                    geometry: g.clone(),
                    parameters,
                    reference_impulse_a: [0.0; 3],
                    reference_impulse_b: [0.0; 3],
                    friction_enabled: true,
                };
                if self
                    .states
                    .get_mut(key)
                    .ok_or("missing walking contact")?
                    .impulse
                    .solve(
                        &input,
                        &mut self.body,
                        None,
                        &fixed,
                        &self.kernel,
                        &self.impulses,
                    )?
                {
                    self.impulse_applications += 1;
                    applied = true;
                }
            }
            if !applied {
                break;
            }
        }
        for _ in 0..self.kernel.physics.kernel_substeps {
            let frames = rig.pose(&self.body.frame)?;
            let torso = frames.get("Torso").ok_or("missing state torso")?.clone();
            force(&mut self.body, &self.statics, &torso, self.inertia)?;
            for (name, size) in &self.colliders {
                for (vertex, g) in self.statics.geometry.plane_vertices(
                    frames.get(name).ok_or("missing spring collider")?,
                    *size,
                    &wall,
                    3,
                )? {
                    self.max_penetration = self.max_penetration.max(-g.distance);
                    if let Some(state) = self.states.get_mut(&(name.clone(), vertex)) {
                        let force = state.spring.force(
                            &self.kernel,
                            parameters,
                            g.distance,
                            g.normal,
                            g.point,
                            &self.body.contact_state(),
                            &fixed,
                        )?;
                        self.body.apply_force(force.map(|v| -v), g.point)?;
                    }
                }
            }
            self.body.step(&self.kernel)?;
        }
        self.steps = self.steps.checked_add(1).ok_or("walking tick overflow")?;
        Ok(())
    }
}
