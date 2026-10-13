//! RBX::World subset: one character assembly, the static contents, awake
//! contact impulses, and kernel substeps with a per-substep force callback
//! (the Humanoid state's kernel connector, vtable +0xc). The static contents
//! are either the walking fixture (floor plane, wall half-space) or the
//! anchored Parts of a Workspace (`World::in_workspace`).
use crate::{
    assembly::{AssemblyProperties, MassPart, MassProfile, Tensor},
    bundle::Bundle,
    collision::ContactBodyState,
    humanoid_state::Hit,
    kernel::{ContactParameters, ContactSpring, KernelBody, KernelProfile, cross},
    legacy_contact::{AwakeContact, BlockContactProfile, ImpulseInput, ImpulseProfile},
    physics,
    rig::{R6Rig, compose},
    scene::{CoordinateFrame, Scene},
    workspace::{Workspace, character_placement},
};
use std::collections::BTreeMap;

struct ContactState {
    spring: ContactSpring,
    impulse: AwakeContact,
}

/// An anchored, colliding block Part in the world.
#[derive(Clone, Debug)]
pub struct StaticBlock {
    pub name: String,
    pub frame: CoordinateFrame,
    pub size: [f32; 3],
}

/// G3D::CollisionDetection::collisionLocationForMovingPointFixedAABox
/// (0124e766, Woo's ray/box test). A ray starting inside the box misses.
/// The normal is -1/+1 on the hit axis (table 0177ed50) by direction sign.
fn moving_point_fixed_aabox(
    origin: [f32; 3],
    direction: [f32; 3],
    low: [f32; 3],
    high: [f32; 3],
) -> Option<([f32; 3], [f32; 3])> {
    let mut inside = true;
    let mut max_t = [-1.0f32; 3];
    let mut location = [0.0f32; 3];
    for i in 0..3 {
        let plane = if low[i] <= origin[i] {
            if high[i] < origin[i] {
                high[i]
            } else {
                continue;
            }
        } else {
            low[i]
        };
        location[i] = plane;
        inside = false;
        if direction[i] != 0.0 {
            max_t[i] = (plane - origin[i]) / direction[i];
        }
    }
    if inside {
        return None;
    }
    let first = usize::from(max_t[0] < max_t[1]);
    let which = if max_t[2] <= max_t[first] { first } else { 2 };
    let t = max_t[which];
    // Sign-bit test on the float bits, as G3D's IR(MaxT) & 0x80000000.
    if t.to_bits() & 0x8000_0000 != 0 {
        return None;
    }
    for i in 0..3 {
        if i != which {
            location[i] = direction[i] * t + origin[i];
            if location[i] < low[i] || (high[i] <= location[i] && location[i] != high[i]) {
                return None;
            }
        }
    }
    let mut normal = [0.0; 3];
    normal[which] = if 0.0 < direction[which] { -1.0 } else { 1.0 };
    Some((location, normal))
}

impl StaticBlock {
    /// Primitive hitTest (00b631de) with Block geometry (vtable +0x70,
    /// 00adeffe): the unit ray in part space against the centered box, the
    /// hit point and normal back in world space.
    fn hit_test(&self, origin: [f32; 3], direction: [f32; 3]) -> Option<Hit> {
        let r = &self.frame.rotation;
        let t = self.frame.position;
        let p: [f32; 3] = std::array::from_fn(|i| origin[i] - t[i]);
        let local_origin: [f32; 3] =
            std::array::from_fn(|i| r[0][i] * p[0] + r[1][i] * p[1] + r[2][i] * p[2]);
        let local_direction: [f32; 3] = std::array::from_fn(|i| {
            r[0][i] * direction[0] + r[1][i] * direction[1] + r[2][i] * direction[2]
        });
        let half = self.size.map(|s| s * 0.5);
        let (hit, normal) =
            moving_point_fixed_aabox(local_origin, local_direction, half.map(|h| -h), half)?;
        Some((
            std::array::from_fn(|i| r[i][2] * hit[2] + r[i][1] * hit[1] + r[i][0] * hit[0] + t[i]),
            std::array::from_fn(|i| {
                r[i][2] * normal[2] + r[i][1] * normal[1] + r[i][0] * normal[0]
            }),
        ))
    }
    fn radius(&self) -> f32 {
        let h = self.size.map(|s| s * 0.5);
        (h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt()
    }
    fn corners(frame: &CoordinateFrame, size: [f32; 3]) -> [[f32; 3]; 8] {
        std::array::from_fn(|i| {
            frame.transform_point(std::array::from_fn(|a| {
                (if i & (1 << a) != 0 { 0.5 } else { -0.5 }) * size[a]
            }))
        })
    }
    /// Separating-axis overlap of two oriented boxes.
    fn overlaps(&self, frame: &CoordinateFrame, size: [f32; 3]) -> bool {
        let column =
            |f: &CoordinateFrame, c: usize| [f.rotation[0][c], f.rotation[1][c], f.rotation[2][c]];
        let mut axes = Vec::with_capacity(15);
        for c in 0..3 {
            axes.push(column(&self.frame, c));
            axes.push(column(frame, c));
        }
        for a in 0..3 {
            for b in 0..3 {
                axes.push(cross(column(&self.frame, a), column(frame, b)));
            }
        }
        let mine = Self::corners(&self.frame, self.size);
        let theirs = Self::corners(frame, size);
        let project = |points: &[[f32; 3]; 8], axis: [f32; 3]| {
            points
                .iter()
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| {
                    let d = p[0] * axis[0] + p[1] * axis[1] + p[2] * axis[2];
                    (lo.min(d), hi.max(d))
                })
        };
        axes.into_iter()
            .filter(|a| a[0] * a[0] + a[1] * a[1] + a[2] * a[2] > 1e-12)
            .all(|axis| {
                let (a0, a1) = project(&mine, axis);
                let (b0, b1) = project(&theirs, axis);
                a0 <= b1 && b0 <= a1
            })
    }
}

/// Static contents and the collision queries the Humanoid makes against
/// them (floor/ceiling rays, PopperCam, touch tests). The fixture planes are
/// disabled (infinite) in a Workspace world.
pub struct Statics {
    pub floor_y: f32,
    pub wall_x: f32,
    /// Anchored, colliding Workspace blocks (empty in the walking fixture).
    pub blocks: Vec<StaticBlock>,
    geometry: BlockContactProfile,
}

impl Statics {
    pub fn wall_enabled(&self) -> bool {
        self.wall_x.is_finite()
    }
    /// ContactManager::getSlowHit (00ae7d08) over the static blocks: bounding
    /// sphere test, block hit test, then the nearest hit with a positive
    /// distance along the ray strictly below min(length, 5000). The spatial
    /// grid walk of getFastHit (00ae8274) only orders candidates.
    fn block_hit(&self, origin: [f32; 3], direction: [f32; 3], length: f32) -> Option<(f32, Hit)> {
        let mut best = length.min(5000.0);
        let mut result = None;
        for block in &self.blocks {
            let c = block.frame.position;
            let along = (c[2] - origin[2]) * direction[2]
                + (c[1] - origin[1]) * direction[1]
                + (c[0] - origin[0]) * direction[0];
            let off: [f32; 3] = std::array::from_fn(|i| c[i] - (direction[i] * along + origin[i]));
            if (off[2] * off[2] + off[1] * off[1] + off[0] * off[0]).sqrt() > block.radius() {
                continue;
            }
            let Some((point, normal)) = block.hit_test(origin, direction) else {
                continue;
            };
            let distance = (point[2] - origin[2]) * direction[2]
                + (point[1] - origin[1]) * direction[1]
                + (point[0] - origin[0]) * direction[0];
            if 0.0 < distance && distance < best {
                best = distance;
                result = Some((distance, (point, normal)));
            }
        }
        result
    }
    /// Floor plane (solid below), wall half-space and the static blocks.
    pub fn raycast(&self, origin: [f32; 3], direction: [f32; 3], length: f32) -> Option<Hit> {
        let mut best: Option<(f32, Hit)> = self.block_hit(origin, direction, length);
        if direction[1] < 0.0 && origin[1] > self.floor_y {
            let t = (origin[1] - self.floor_y) / -direction[1];
            if t <= length && best.as_ref().is_none_or(|(b, _)| t < *b) {
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
    /// Stand-in for a touching kernel contact: the part generates a
    /// vertex/plane connector against the fixture wall, or overlaps a static
    /// block (until the BlockBlockContact narrowphase is ported).
    pub fn touching(&self, frame: &CoordinateFrame, size: [f32; 3]) -> Result<bool, String> {
        if self.blocks.iter().any(|b| b.overlaps(frame, size)) {
            return Ok(true);
        }
        Ok(self.wall_enabled()
            && !self
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
                blocks: Vec::new(),
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
    /// The character in a Workspace instead of the fixture: the fixture
    /// planes are disabled, the anchored colliding Parts become static
    /// blocks, and the character is placed the way Play Solo spawns it
    /// (`Workspace::next_spawn_location`, `character_placement`).
    pub fn in_workspace(
        bundle: &Bundle,
        scene: &Scene,
        rig: &R6Rig,
        workspace: &Workspace,
    ) -> Result<Self, String> {
        let mut world = Self::new(bundle, scene, rig)?;
        world.statics.floor_y = f32::NEG_INFINITY;
        world.statics.wall_x = f32::INFINITY;
        for part in &workspace.parts {
            if !part.anchored {
                return Err(format!(
                    "unanchored Part {:?} is not simulated yet",
                    part.name
                ));
            }
            if part.can_collide {
                world.statics.blocks.push(StaticBlock {
                    name: part.name.clone(),
                    frame: part.frame.clone(),
                    size: part.size,
                });
            }
        }
        let spawn = CoordinateFrame {
            position: workspace.next_spawn_location()?,
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        let head = scene
            .instances
            .iter()
            .find(|i| i.name == "Head")
            .and_then(|i| i.part.as_ref())
            .ok_or("missing character Head")?;
        let placement = character_placement(&spawn, &head.frame);
        let mut state = world.body.body_state();
        state.frame = compose(&placement, &state.frame);
        state.velocity = [0.0; 3];
        state.angular_velocity = [0.0; 3];
        world.body.set_body_state(&state)?;
        world.original = world.body.clone();
        Ok(world)
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
        for (name, size) in self
            .colliders
            .iter()
            .filter(|_| self.statics.wall_enabled())
        {
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
        // Block/block contacts with Workspace Parts are not ported yet; only
        // the fixture wall generates connectors.
        for (name, size) in self
            .colliders
            .iter()
            .filter(|_| self.statics.wall_enabled())
        {
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
            for (name, size) in self
                .colliders
                .iter()
                .filter(|_| self.statics.wall_enabled())
            {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn woo_ray_box_matches_g3d_rules() {
        let (low, high) = ([-1.0; 3], [1.0; 3]);
        let hit = moving_point_fixed_aabox([0.0, 5.0, 0.0], [0.0, -1.0, 0.0], low, high);
        assert_eq!(hit, Some(([0.0, 1.0, 0.0], [0.0, 1.0, 0.0])));
        // Inside the box: no hit. Pointing away: negative t, no hit.
        assert_eq!(
            moving_point_fixed_aabox([0.0; 3], [0.0, -1.0, 0.0], low, high),
            None
        );
        assert_eq!(
            moving_point_fixed_aabox([0.0, 5.0, 0.0], [0.0, 1.0, 0.0], low, high),
            None
        );
        // Missing to the side.
        assert_eq!(
            moving_point_fixed_aabox([3.0, 5.0, 0.0], [0.0, -1.0, 0.0], low, high),
            None
        );
        // Grazing the far edge counts (location == high).
        assert!(moving_point_fixed_aabox([1.0, 5.0, 0.0], [0.0, -1.0, 0.0], low, high).is_some());
        let side = moving_point_fixed_aabox([-5.0, 0.5, 0.0], [1.0, 0.0, 0.0], low, high);
        assert_eq!(side, Some(([-1.0, 0.5, 0.0], [-1.0, 0.0, 0.0])));
    }
    #[test]
    fn rotated_block_hit_is_in_world_space() {
        let block = StaticBlock {
            name: "Part".into(),
            frame: CoordinateFrame {
                position: [10.0, 0.0, 0.0],
                rotation: [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            },
            size: [4.0, 2.0, 2.0],
        };
        // Rotated 90 degrees about Z: the 4-stud axis points along world Y.
        let (point, normal) = block.hit_test([10.0, 10.0, 0.0], [0.0, -1.0, 0.0]).unwrap();
        assert_eq!(point, [10.0, 2.0, 0.0]);
        assert_eq!(normal, [0.0, 1.0, 0.0]);
        let other = CoordinateFrame {
            position: [10.0, 2.4, 0.0],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        };
        assert!(block.overlaps(&other, [1.0; 3]));
        let apart = CoordinateFrame {
            position: [10.0, 2.6, 0.0],
            ..other
        };
        assert!(!block.overlaps(&apart, [1.0; 3]));
    }
}
