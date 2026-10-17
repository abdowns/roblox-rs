//! RBX::Workspace subset for the Studio new-document world: PartInstance
//! construction defaults, the Baseplate RobloxIDEDoc adds to a new place, and
//! the Play Solo character placement without a SpawnLocation.
//!
//! Recovered from:
//! - PartInstance ctor 009828ce / Part ctor 00708a70 (defaults),
//! - RobloxIDEDoc 001017a6 (Baseplate),
//! - Player::calculateNextSpawnLocation 00f6bdf6 and the safe-spot search
//!   00ae8e9a (spawn point),
//! - SpawnLocation.cpp 009f4706 with Model moveTo 009c1bc6, getLocation
//!   009703f0, GetPrimaryCFrame 0096e468 and SetPrimaryPartCFrame 0096e582
//!   (character placement).
use crate::rbxm::{Model, ModelInstance, Property};
use crate::{
    joints::{self, JointTree, TreePart},
    kernel_joint::SurfaceInput,
    rig::{compose, inverse},
    scene::{CoordinateFrame, Part},
};
use std::fmt::Write;
use std::path::Path;

/// RBX::SurfaceType values stored at Primitive +0xd2+NormalId.
pub const SURFACE_SMOOTH: u32 = 0;
pub const SURFACE_STUDS: u32 = 3;
pub const SURFACE_INLET: u32 = 4;
/// Part.Shape (PartType, PartInstance +0xb7).
pub const SHAPE_BALL: u32 = 0;
pub const SHAPE_BLOCK: u32 = 1;
pub const SHAPE_CYLINDER: u32 = 2;
/// A WedgePart. `shape` holds the Primitive part type (vtable +0xa4,
/// addPartImpl 010d1850), which matches Part.Shape for Ball, Block and
/// Cylinder and is 4 for a Wedge.
pub const SHAPE_WEDGE: u32 = 4;
/// NormalId order used by the surfaces array: Right, Top, Back, Left, Bottom, Front.
pub const FACE_NAMES: [&str; 6] = [
    "RightSurface",
    "TopSurface",
    "BackSurface",
    "LeftSurface",
    "BottomSurface",
    "FrontSurface",
];

const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// A block Part with the properties the core client reads.
#[derive(Clone, Debug, PartialEq)]
pub struct PartInstance {
    pub name: String,
    pub frame: CoordinateFrame,
    pub size: [f32; 3],
    pub brick_color: u32,
    /// RBX::PartMaterial (0x100 Plastic).
    pub material: u32,
    pub transparency: f32,
    pub reflectance: f32,
    /// Primitive +0xcb (vtable +0x110 setter, property descriptor 01d719d8).
    pub anchored: bool,
    /// PartInstance +0xb5 (vtable +0x12c setter, property descriptor 01d719a4).
    pub locked: bool,
    pub can_collide: bool,
    /// Part.Shape: `SHAPE_BALL`, `SHAPE_BLOCK` or `SHAPE_CYLINDER`, or
    /// `SHAPE_WEDGE` for a WedgePart. Set it with `set_shape`, which keeps a
    /// Ball's size uniform.
    pub shape: u32,
    /// Primitive +0xbc.
    pub friction: f32,
    pub elasticity: f32,
    pub surfaces: [u32; 6],
    /// Primitive +0xd8 by NormalId: each face's InputType, ParamA and
    /// ParamB (a motor hinge's axle reads its own).
    pub surface_inputs: [SurfaceInput; 6],
    /// FormFactorPart +0xb6, saved as `formFactorRaw` (0 Symmetric, 1 Brick,
    /// 2 Plate, 3 Custom). Only kept for saving: the size setter does not
    /// snap to it.
    pub form_factor: u32,
}

impl Default for PartInstance {
    /// Instance.new("Part"): size (4, 1.2, 2), BrickColor 194 (Medium stone
    /// grey), Plastic, friction 0.3, elasticity 0.5, unanchored, Top Studs
    /// (face 1 = 3) and Bottom Inlet (face 4 = 4), all other faces Smooth.
    fn default() -> Self {
        let mut surfaces = [SURFACE_SMOOTH; 6];
        surfaces[1] = SURFACE_STUDS;
        surfaces[4] = SURFACE_INLET;
        Self {
            name: "Part".into(),
            frame: CoordinateFrame {
                position: [0.0; 3],
                rotation: IDENTITY,
            },
            size: [4.0, 1.2, 2.0],
            brick_color: 194,
            material: 0x100,
            transparency: 0.0,
            reflectance: 0.0,
            anchored: false,
            locked: false,
            can_collide: true,
            shape: SHAPE_BLOCK,
            friction: 0.3,
            elasticity: 0.5,
            surfaces,
            surface_inputs: [SurfaceInput::default(); 6],
            // PartInstance ctor 009828ce: word +0xb6 = 0x101 (FormFactor
            // Brick, shape Block).
            form_factor: 1,
        }
    }
}

impl PartInstance {
    /// PartInstance size setter (vtable +0xbc -> +0xd0 00984658): each axis is
    /// raised to the 0.2 minimum (0081b8c0); no FormFactor snapping here.
    /// A Ball has no three-dimensional size (0070915c), so +0xd4 00988cfc
    /// stores (x, x, x).
    pub fn set_size(&mut self, size: [f32; 3]) {
        let size: [f32; 3] = std::array::from_fn(|a| if size[a] <= 0.2 { 0.2 } else { size[a] });
        self.size = if self.shape == SHAPE_BALL {
            [size[0]; 3]
        } else {
            size
        };
    }
    /// Part.Shape setter 00708de2: a new shape, then a Ball's size is set
    /// again through the size setter (00988e50), which makes it (x, x, x).
    pub fn set_shape(&mut self, shape: u32) {
        if self.shape == shape {
            return;
        }
        self.shape = shape;
        if shape == SHAPE_BALL {
            self.set_size(self.size);
        }
    }
    /// World-space axis-aligned extents of the block.
    pub fn extents(&self) -> ([f32; 3], [f32; 3]) {
        let half = self.size.map(|s| s * 0.5);
        let r = &self.frame.rotation;
        let reach: [f32; 3] = std::array::from_fn(|i| {
            r[i][0].abs() * half[0] + r[i][1].abs() * half[1] + r[i][2].abs() * half[2]
        });
        (
            std::array::from_fn(|i| self.frame.position[i] - reach[i]),
            std::array::from_fn(|i| self.frame.position[i] + reach[i]),
        )
    }
    /// The scene-level Part view used by the existing render/mass code.
    pub fn as_part(&self) -> Part {
        Part {
            frame: self.frame.clone(),
            size: self.size,
            shape: self.shape,
            brick_color: self.brick_color,
            transparency: self.transparency,
            anchored: self.anchored,
            can_collide: self.can_collide,
        }
    }
}

/// A JointInstance: Part0, Part1, C0, C1. A rigid joint (Weld, ManualWeld,
/// Snap) holds Part0.CFrame * C0 = Part1.CFrame * C1; a Glue pulls the
/// corners of its face in C0 toward the same corners in C1; a hinge
/// (Rotate, RotateP, RotateV) holds the two ends of C0's z axis on C1's.
#[derive(Clone, Debug, PartialEq)]
pub struct JointInstance {
    pub name: String,
    /// `joints::WELD`, `joints::MANUAL_WELD`, `joints::SNAP`, `joints::GLUE`,
    /// `joints::ROTATE`, `joints::ROTATE_P` or `joints::ROTATE_V`.
    pub kind: u32,
    /// Indexes into `Workspace::parts`.
    pub part0: usize,
    pub part1: usize,
    pub c0: CoordinateFrame,
    pub c1: CoordinateFrame,
    /// Glue F0..F3 (GlueJoint +0xd0, setters such as 009208cc), in C0's
    /// frame; zero for the other kinds (the GlueJoint ctor 00b0ce2c zeroes
    /// it).
    pub face: [[f32; 3]; 4],
    /// RotateP / RotateV BaseAngle (DynamicRotateJoint +0xd8, getter
    /// 0092b260); 0 for the others.
    pub base_angle: f32,
    /// Motor CurrentAngle, DesiredAngle and MaxVelocity (MotorJoint +0x9c,
    /// +0xb0, +0xac); 0 for the others.
    pub current_angle: f32,
    pub desired_angle: f32,
    pub max_velocity: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Workspace {
    pub parts: Vec<PartInstance>,
    pub joints: Vec<JointInstance>,
}

impl Workspace {
    /// RobloxIDEDoc new document (001017a6): one Part named "Baseplate",
    /// Size (512, 20, 512), Position (0, -10, 0), Anchored, Locked,
    /// BrickColor 199 (Dark stone grey); every other property keeps the Part
    /// defaults (Top Studs, Bottom Inlet).
    pub fn studio_new_document() -> Self {
        let mut baseplate = PartInstance {
            name: "Baseplate".into(),
            ..PartInstance::default()
        };
        baseplate.set_size([512.0, 20.0, 512.0]);
        baseplate.frame.position = [0.0, -10.0, 0.0];
        baseplate.anchored = true;
        baseplate.locked = true;
        baseplate.brick_color = 199;
        Self {
            parts: vec![baseplate],
            joints: Vec::new(),
        }
    }
    /// The World's joint spanning tree (`joints.rs`): every Part in
    /// Workspace order (World::insertPrimitive), then every joint in
    /// Workspace order (World::insertJoint). The native load interleaves
    /// them in DataModel order; the spanning tree does not depend on it,
    /// the Body children order can (not recovered).
    pub fn joint_tree(&self) -> Result<JointTree, String> {
        Ok(self.joint_tree_indexed()?.0)
    }
    /// `joint_tree` and each Workspace joint's index in the tree (ground
    /// joints share the tree's numbering).
    pub fn joint_tree_indexed(&self) -> Result<(JointTree, Vec<usize>), String> {
        let mut tree = JointTree::new();
        let mut ids = Vec::with_capacity(self.joints.len());
        for part in &self.parts {
            tree.add_part(TreePart {
                size: part.size,
                class: joints::name_class(&part.name),
                fixed: part.anchored,
            });
        }
        for joint in &self.joints {
            let ported = [
                joints::MOTOR,
                joints::MOTOR_6D,
                joints::GLUE,
                joints::ROTATE,
                joints::ROTATE_P,
                joints::ROTATE_V,
            ];
            if !joints::is_rigid(joint.kind) && !ported.contains(&joint.kind) {
                return Err(format!(
                    "joint {:?}: only rigid joints, the motors, Glue and the hinges are ported",
                    joint.name
                ));
            }
            ids.push(tree.add_joint(
                joint.kind,
                joint.part0,
                joint.part1,
                joint.c0.clone(),
                joint.c1.clone(),
            )?);
        }
        Ok((tree, ids))
    }
    /// Model extents (009708e0) over the workspace parts.
    pub fn extents(&self) -> Option<([f32; 3], [f32; 3])> {
        self.parts.iter().map(PartInstance::extents).reduce(|a, b| {
            (
                std::array::from_fn(|i| a.0[i].min(b.0[i])),
                std::array::from_fn(|i| a.1[i].max(b.1[i])),
            )
        })
    }
    /// Player::calculateNextSpawnLocation (00f6bdf6) when no SpawnLocation is
    /// enabled: (0, 100, 0), moved over the extents' x/z center at
    /// min.y + 100 only when the extents do not contain the origin; then the
    /// safe-spot search (00ae8e9a), which returns immediately when the
    /// workspace extents' max y is below the candidate.
    pub fn next_spawn_location(&self) -> Result<[f32; 3], String> {
        let mut spawn = [0.0, 100.0, 0.0];
        let (low, high) = self.extents().unwrap_or(([0.0; 3], [0.0; 3]));
        if 0.0 < low[0]
            || 0.0 < low[1]
            || 0.0 < low[2]
            || high[0] < 0.0
            || high[1] < 0.0
            || high[2] < 0.0
        {
            spawn = [
                (low[0] + high[0]) * 0.5 + spawn[0],
                low[1] + spawn[1],
                (low[2] + high[2]) * 0.5 + spawn[2],
            ];
        }
        if high[1] < spawn[1] {
            return Ok(spawn);
        }
        Err("spawn safe-spot search over workspace geometry is not ported".into())
    }
}

/// G3D::CoordinateFrame::lookAt rotation for a frame at the origin.
fn look_at(target: [f32; 3], up: [f32; 3]) -> [[f32; 3]; 3] {
    let unit = |v: [f32; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        v.map(|x| x / l)
    };
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let mut up = unit(up);
    let look = unit(target);
    if dot(look, up).abs() > 0.99 {
        up = [1.0, 0.0, 0.0];
        if dot(look, up).abs() > 0.99 {
            up = [0.0, 1.0, 0.0];
        }
    }
    let along = dot(look, up);
    up = unit(std::array::from_fn(|i| up[i] - look[i] * along));
    let z = look.map(|v| -v);
    let x = unit(cross(z, up).map(|v| -v));
    let y = cross(z, x);
    std::array::from_fn(|r| [x[r], y[r], z[r]])
}

/// SpawnLocation.cpp character placement (009f4706): move the model so its
/// location (the PrimaryPart CFrame, 009703f0) is at spawn + (0, 7, 0), then
/// SetPrimaryPartCFrame with a lookAt along the spawn's horizontal look
/// vector. Returns the rigid transform applied to every part of the model.
pub fn character_placement(spawn: &CoordinateFrame, primary: &CoordinateFrame) -> CoordinateFrame {
    let target = [
        spawn.position[0],
        spawn.position[1] + 7.0,
        spawn.position[2],
    ];
    // Model moveTo by the delta from the location (no obstruction search: the
    // mover only shifts a model that overlaps geometry).
    let delta: [f32; 3] = std::array::from_fn(|a| target[a] - primary.position[a]);
    let moved = CoordinateFrame {
        position: std::array::from_fn(|a| primary.position[a] + delta[a]),
        rotation: primary.rotation,
    };
    // lookVector = -column 2; fall back to column 1 if it is fuzzy-equal to
    // Vector3::unitY (005e5110); flatten to the horizontal plane and normalize.
    let column = |c: usize| {
        [
            spawn.rotation[0][c],
            spawn.rotation[1][c],
            spawn.rotation[2][c],
        ]
    };
    let mut look = column(2).map(|v| -v);
    let difference = look[2] * look[2] + (look[1] - 1.0) * (look[1] - 1.0) + look[0] * look[0];
    if crate::g3d::fuzzy_zero(difference) {
        look = column(1);
    }
    let inverse_length = 1.0 / (look[2] * look[2] + look[0] * look[0] + 0.0).sqrt();
    let look = [look[0] * inverse_length, 0.0, inverse_length * look[2]];
    let placed = CoordinateFrame {
        position: moved.position,
        rotation: look_at(look, [0.0, 1.0, 0.0]),
    };
    let translation = CoordinateFrame {
        position: delta,
        rotation: IDENTITY,
    };
    compose(&compose(&placed, &inverse(&moved)), &translation)
}

/// The viewport demo place (`viewport-baseplate`), in `places/`.
pub const DEMO_PLACE: &str = "viewport-baseplate.rbxm";

/// A place or model file shipped with the project, in `places/` (our own
/// files, not game data).
pub fn place_path(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("places")
        .join(name)
}

/// The JointInstance classes the port simulates, by Joint::JointType.
const JOINT_CLASSES: [(&str, u32); 9] = [
    ("Weld", joints::WELD),
    ("ManualWeld", joints::MANUAL_WELD),
    ("Snap", joints::SNAP),
    ("Glue", joints::GLUE),
    ("Rotate", joints::ROTATE),
    ("RotateP", joints::ROTATE_P),
    ("RotateV", joints::ROTATE_V),
    ("Motor", joints::MOTOR),
    ("Motor6D", joints::MOTOR_6D),
];
/// Surface property name stems by NormalId (`FACE_NAMES` without "Surface").
const FACE_STEMS: [&str; 6] = ["Right", "Top", "Back", "Left", "Bottom", "Front"];
/// JointInstance Surface0/Surface1 (0092aba0, functionality 0x0e) read the
/// kernel joint's faces only for a ManualWeld (+0x98/+0x9c) or joint type 10
/// (+0x10c/+0x110); every other joint, and a new ManualWeld (00921eec),
/// gives -1 (NormalId Unknown).
const NO_SURFACE: u32 = 0xffff_ffff;

impl Workspace {
    /// The instance tree a native save of these Parts and joints holds:
    /// one `Part` or `WedgePart` root per Part in Workspace order, then one
    /// root per joint in Workspace order. Each instance carries every
    /// property the 0.255 reflection marks XML-writable (functionality bit 8,
    /// see docs/RESEARCH.md): the 37 Part properties of BasePart 00999b11,
    /// the surfaces 00a11460 and FormFactorPart/Part 0070b4dc (`shape` only on
    /// Part), and for joints Name, Part0, Part1, C0, C1, Surface0 and
    /// Surface1 (0092aba0), with F0..F3 on Glue, BaseAngle on RotateP and
    /// RotateV, and DesiredAngle and MaxVelocity on Motor and Motor6D
    /// (CurrentAngle is not written: functionality 0x15).
    ///
    /// Joints are roots, not children of their Part0, so that loading the
    /// file gives back the same Part and joint order (the order is the
    /// creation order the solver's tie-breaks use).
    pub fn to_model(&self) -> Result<Model, String> {
        let mut model = Model::default();
        for part in &self.parts {
            let class = match part.shape {
                SHAPE_WEDGE => "WedgePart",
                SHAPE_BALL | SHAPE_BLOCK | SHAPE_CYLINDER => "Part",
                other => return Err(format!("Part {:?}: shape {other} has no class", part.name)),
            };
            let mut i = ModelInstance::new(class);
            i.set("Name", Property::String(part.name.clone()))
                .set("CFrame", Property::CFrame(part.frame.clone()))
                .set("size", Property::Vector3(part.size))
                .set("BrickColor", Property::BrickColor(part.brick_color))
                .set("Material", Property::Token(part.material))
                .set("Transparency", Property::Float(part.transparency))
                .set("Reflectance", Property::Float(part.reflectance))
                .set("Anchored", Property::Bool(part.anchored))
                .set("Locked", Property::Bool(part.locked))
                .set("CanCollide", Property::Bool(part.can_collide))
                .set("Friction", Property::Float(part.friction))
                .set("Elasticity", Property::Float(part.elasticity))
                .set("Velocity", Property::Vector3([0.0; 3]))
                .set("RotVelocity", Property::Vector3([0.0; 3]))
                .set(
                    "CustomPhysicalProperties",
                    Property::PhysicalProperties(None),
                )
                .set("formFactorRaw", Property::Token(part.form_factor));
            if class == "Part" {
                i.set("shape", Property::Token(part.shape));
            }
            for (face, stem) in FACE_STEMS.iter().enumerate() {
                let input = &part.surface_inputs[face];
                i.set(
                    &format!("{stem}Surface"),
                    Property::Token(part.surfaces[face]),
                )
                .set(&format!("{stem}SurfaceInput"), Property::Token(input.input))
                .set(&format!("{stem}ParamA"), Property::Float(input.param_a))
                .set(&format!("{stem}ParamB"), Property::Float(input.param_b));
            }
            model.add(i, None);
        }
        for joint in &self.joints {
            let class = JOINT_CLASSES
                .iter()
                .find(|(_, kind)| *kind == joint.kind)
                .map(|(class, _)| *class)
                .ok_or_else(|| {
                    format!("joint {:?}: kind {} has no class", joint.name, joint.kind)
                })?;
            for part in [joint.part0, joint.part1] {
                if part >= self.parts.len() {
                    return Err(format!("joint {:?}: no Part {part}", joint.name));
                }
            }
            let mut i = ModelInstance::new(class);
            i.set("Name", Property::String(joint.name.clone()))
                .set("Part0", Property::Ref(Some(joint.part0)))
                .set("Part1", Property::Ref(Some(joint.part1)))
                .set("C0", Property::CFrame(joint.c0.clone()))
                .set("C1", Property::CFrame(joint.c1.clone()))
                .set("Surface0", Property::Token(NO_SURFACE))
                .set("Surface1", Property::Token(NO_SURFACE));
            match joint.kind {
                joints::GLUE => {
                    for (k, corner) in joint.face.iter().enumerate() {
                        i.set(&format!("F{k}"), Property::Vector3(*corner));
                    }
                }
                joints::ROTATE_P | joints::ROTATE_V => {
                    i.set("BaseAngle", Property::Float(joint.base_angle));
                }
                joints::MOTOR | joints::MOTOR_6D => {
                    i.set("DesiredAngle", Property::Float(joint.desired_angle))
                        .set("MaxVelocity", Property::Float(joint.max_velocity));
                }
                _ => {}
            }
            model.add(i, None);
        }
        Ok(model)
    }
    /// Builds a Workspace from a model or place tree. Every Part and
    /// WedgePart becomes a Part (in pre-order) and every joint class above a
    /// joint. Properties are applied in name order as the native reader
    /// does (one PROP chunk at a time, alphabetically), so `shape` is set
    /// before `size` and a Ball ends up with (x, x, x). Properties the
    /// 0.255 classes do not have are skipped, as the native reader skips
    /// them.
    ///
    /// Errors, rather than silently dropping behavior the port does not
    /// simulate:
    /// - any other class;
    /// - a non-zero Velocity or RotVelocity, or custom physical properties;
    /// - a joint whose Part0 or Part1 is not one of the Parts;
    /// - a ManualWeld with surfaces set (the port has no surface joints).
    pub fn from_model(model: &Model) -> Result<Self, String> {
        let mut order = Vec::new();
        let mut stack: Vec<usize> = model.roots.iter().rev().copied().collect();
        while let Some(i) = stack.pop() {
            order.push(i);
            stack.extend(model.instances[i].children.iter().rev());
        }
        let mut workspace = Workspace::default();
        let mut part_index = std::collections::BTreeMap::new();
        for &index in &order {
            let instance = &model.instances[index];
            match instance.class.as_str() {
                "Part" | "WedgePart" => {
                    part_index.insert(index, workspace.parts.len());
                    workspace.parts.push(part_from(instance)?);
                }
                class if JOINT_CLASSES.iter().any(|(c, _)| *c == class) => {}
                other => {
                    return Err(format!(
                        "{other} {:?}: only Parts, WedgeParts and joints load",
                        instance.name()
                    ));
                }
            }
        }
        for &index in &order {
            let instance = &model.instances[index];
            let Some((_, kind)) = JOINT_CLASSES.iter().find(|(c, _)| *c == instance.class) else {
                continue;
            };
            workspace
                .joints
                .push(joint_from(instance, *kind, &part_index)?);
        }
        Ok(workspace)
    }
    /// The place `places/<name>.rbxm` (`load`).
    pub fn place(name: &str) -> Result<Self, String> {
        Self::load(&place_path(&format!("{name}.rbxm")))
    }
    /// The index of the first Part called `name`.
    pub fn part_index(&self, name: &str) -> Result<usize, String> {
        self.parts
            .iter()
            .position(|p| p.name == name)
            .ok_or_else(|| format!("no Part {name:?} in the workspace"))
    }
    /// Reads a .rbxm/.rbxl/.rbxmx file (`from_model`).
    pub fn load(path: &Path) -> Result<Self, String> {
        Self::from_model(&Model::read_file(path)?).map_err(|e| format!("{}: {e}", path.display()))
    }
    /// Writes the place: XML for a .rbxmx or .rbxlx path, binary for any
    /// other (`to_model`, `Model::write_file`).
    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.to_model()?.write_file(path)
    }
}

fn expect<T>(
    instance: &ModelInstance,
    name: &str,
    get: impl Fn(&Property) -> Option<T>,
) -> Result<Option<T>, String> {
    match instance.properties.get(name) {
        None => Ok(None),
        Some(value) => get(value).map(Some).ok_or_else(|| {
            format!(
                "{} {:?}: {name} has type {:#04x}",
                instance.class,
                instance.name(),
                value.type_id()
            )
        }),
    }
}
fn as_f32(p: &Property) -> Option<f32> {
    match p {
        Property::Float(v) => Some(*v),
        _ => None,
    }
}
fn as_bool(p: &Property) -> Option<bool> {
    match p {
        Property::Bool(v) => Some(*v),
        _ => None,
    }
}
/// An enum (or, from XML, an int).
fn as_token(p: &Property) -> Option<u32> {
    match p {
        Property::Token(v) => Some(*v),
        Property::Int(v) => u32::try_from(*v).ok(),
        _ => None,
    }
}
fn as_vector3(p: &Property) -> Option<[f32; 3]> {
    match p {
        Property::Vector3(v) => Some(*v),
        _ => None,
    }
}
fn as_cframe(p: &Property) -> Option<CoordinateFrame> {
    match p {
        Property::CFrame(v) => Some(v.clone()),
        _ => None,
    }
}

fn part_from(instance: &ModelInstance) -> Result<PartInstance, String> {
    let mut part = PartInstance::default();
    if instance.class == "WedgePart" {
        part.name = "Wedge".into();
        part.set_shape(SHAPE_WEDGE);
    }
    let unsupported = |what: &str| format!("Part {:?}: {what} is not ported", instance.name());
    // BTreeMap order is the native PROP order (Name sortIndex).
    for (name, value) in &instance.properties {
        let v = Some(value);
        let f32_of = |p| expect(instance, name, as_f32).map(|o| o.unwrap_or(p));
        match name.as_str() {
            "Name" => match value {
                Property::String(s) => part.name = s.clone(),
                _ => return Err(format!("Part Name has type {:#04x}", value.type_id())),
            },
            "CFrame" => part.frame = expect(instance, name, as_cframe)?.unwrap(),
            "size" => part.set_size(expect(instance, name, as_vector3)?.unwrap()),
            "shape" if instance.class == "Part" => {
                part.set_shape(expect(instance, name, as_token)?.unwrap())
            }
            "formFactorRaw" => part.form_factor = expect(instance, name, as_token)?.unwrap(),
            "BrickColor" => {
                part.brick_color = match value {
                    Property::BrickColor(c) => *c,
                    other => as_token(other)
                        .ok_or_else(|| format!("BrickColor has type {:#04x}", other.type_id()))?,
                }
            }
            "Material" => part.material = expect(instance, name, as_token)?.unwrap(),
            "Transparency" => part.transparency = f32_of(0.0)?,
            "Reflectance" => part.reflectance = f32_of(0.0)?,
            "Friction" => part.friction = f32_of(0.0)?,
            "Elasticity" => part.elasticity = f32_of(0.0)?,
            "Anchored" => part.anchored = expect(instance, name, as_bool)?.unwrap(),
            "Locked" => part.locked = expect(instance, name, as_bool)?.unwrap(),
            "CanCollide" => part.can_collide = expect(instance, name, as_bool)?.unwrap(),
            "Velocity" | "RotVelocity" => {
                if expect(instance, name, as_vector3)?.unwrap() != [0.0; 3] {
                    return Err(unsupported(&format!("a starting {name}")));
                }
            }
            "CustomPhysicalProperties" => {
                if !matches!(v, Some(Property::PhysicalProperties(None))) {
                    return Err(unsupported("CustomPhysicalProperties"));
                }
            }
            _ => {
                let face = FACE_STEMS
                    .iter()
                    .position(|stem| name.starts_with(stem) && name.len() > stem.len());
                let Some(face) = face else { continue };
                match &name[FACE_STEMS[face].len()..] {
                    "Surface" => part.surfaces[face] = expect(instance, name, as_token)?.unwrap(),
                    "SurfaceInput" => {
                        part.surface_inputs[face].input = expect(instance, name, as_token)?.unwrap()
                    }
                    "ParamA" => part.surface_inputs[face].param_a = f32_of(0.0)?,
                    "ParamB" => part.surface_inputs[face].param_b = f32_of(0.0)?,
                    _ => {}
                }
            }
        }
    }
    Ok(part)
}

fn joint_from(
    instance: &ModelInstance,
    kind: u32,
    part_index: &std::collections::BTreeMap<usize, usize>,
) -> Result<JointInstance, String> {
    let identity = CoordinateFrame {
        position: [0.0; 3],
        rotation: IDENTITY,
    };
    let part = |name: &str| -> Result<usize, String> {
        match instance.reference(name) {
            Some(Some(i)) => part_index.get(&i).copied().ok_or_else(|| {
                format!(
                    "{} {:?}: {name} is not a Part",
                    instance.class,
                    instance.name()
                )
            }),
            _ => Err(format!(
                "{} {:?}: {name} is not set (a joint without both Parts is not ported)",
                instance.class,
                instance.name()
            )),
        }
    };
    let float = |name: &str| expect(instance, name, as_f32).map(|v| v.unwrap_or(0.0));
    let mut joint = JointInstance {
        name: match instance.properties.get("Name") {
            Some(Property::String(s)) => s.clone(),
            _ => instance.class.clone(),
        },
        kind,
        part0: part("Part0")?,
        part1: part("Part1")?,
        c0: expect(instance, "C0", as_cframe)?.unwrap_or(identity.clone()),
        c1: expect(instance, "C1", as_cframe)?.unwrap_or(identity),
        face: [[0.0; 3]; 4],
        base_angle: 0.0,
        current_angle: 0.0,
        desired_angle: 0.0,
        max_velocity: 0.0,
    };
    if kind == joints::MANUAL_WELD {
        for name in ["Surface0", "Surface1"] {
            if expect(instance, name, as_token)?.is_some_and(|s| s != NO_SURFACE) {
                return Err(format!(
                    "ManualWeld {:?}: {name} is set (surface joints are not ported)",
                    joint.name
                ));
            }
        }
    }
    match kind {
        joints::GLUE => {
            for k in 0..4 {
                joint.face[k] = expect(instance, &format!("F{k}"), as_vector3)?.unwrap_or([0.0; 3]);
            }
        }
        joints::ROTATE_P | joints::ROTATE_V => joint.base_angle = float("BaseAngle")?,
        joints::MOTOR | joints::MOTOR_6D => {
            joint.desired_angle = float("DesiredAngle")?;
            joint.max_velocity = float("MaxVelocity")?;
            // CurrentAngle is read if present (functionality 0x15 reads XML)
            // but never written.
            joint.current_angle = float("CurrentAngle")?;
        }
        _ => {}
    }
    Ok(joint)
}

/// Native-numbers report for the Studio default world.
pub fn audit() -> Result<String, String> {
    let mut report = String::new();
    let part = PartInstance::default();
    writeln!(report, "part_defaults name={:?} size={:?} brick_color={} material={:#x} friction={} elasticity={} anchored={} locked={} can_collide={} surfaces={:?} source=PartInstance_ctor_009828ce", part.name, part.size, part.brick_color, part.material, part.friction, part.elasticity, part.anchored, part.locked, part.can_collide, part.surfaces).unwrap();
    let workspace = Workspace::studio_new_document();
    for p in &workspace.parts {
        let (low, high) = p.extents();
        writeln!(report, "workspace_part name={:?} position={:?} size={:?} brick_color={} anchored={} locked={} surfaces={:?} extents_low={low:?} extents_high={high:?} source=RobloxIDEDoc_001017a6", p.name, p.frame.position, p.size, p.brick_color, p.anchored, p.locked, p.surfaces).unwrap();
    }
    let spawn = workspace.next_spawn_location()?;
    writeln!(report, "spawn_location position={spawn:?} spawn_location_instances=0 safe_spot=extents_below_candidate").unwrap();
    if spawn != [0.0, 100.0, 0.0] || workspace.parts[0].extents().1[1] != 0.0 {
        return Err("unexpected Studio default world spawn".into());
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn assert_same(a: &Workspace, b: &Workspace) {
        assert_eq!(a.parts.len(), b.parts.len());
        for (x, y) in a.parts.iter().zip(&b.parts) {
            assert_eq!(x, y);
        }
        assert_eq!(a.joints, b.joints);
    }
    #[test]
    fn the_demo_place_round_trips() {
        let path = place_path(DEMO_PLACE);
        let bytes = std::fs::read(&path).unwrap();
        let demo = Workspace::load(&path).unwrap();
        assert_eq!(demo.parts.len(), 27);
        assert_eq!(demo.joints.len(), 5);
        // Saved again, the file is the same.
        assert_eq!(demo.to_model().unwrap().to_binary().unwrap(), bytes);
        let back = Workspace::from_model(&Model::parse(&bytes).unwrap()).unwrap();
        assert_same(&demo, &back);
    }
    #[test]
    fn the_demo_place_round_trips_through_xml() {
        let demo = Workspace::place("viewport-baseplate").unwrap();
        let xml = demo.to_model().unwrap().to_xml().unwrap();
        let back = Workspace::from_model(&Model::parse(&xml).unwrap()).unwrap();
        assert_same(&demo, &back);
        // And back to binary, the file is the same.
        let bytes = std::fs::read(place_path(DEMO_PLACE)).unwrap();
        assert_eq!(back.to_model().unwrap().to_binary().unwrap(), bytes);
    }
    #[test]
    fn a_part_has_the_native_property_set() {
        let model = Workspace::studio_new_document().to_model().unwrap();
        let names: Vec<&str> = model.instances[0]
            .properties
            .keys()
            .map(|k| k.as_str())
            .collect();
        // characterR15_2.rbxm (native-written) lists these for Part.
        assert_eq!(
            names.join(" "),
            "Anchored BackParamA BackParamB BackSurface BackSurfaceInput BottomParamA \
             BottomParamB BottomSurface BottomSurfaceInput BrickColor CFrame CanCollide \
             CustomPhysicalProperties Elasticity Friction FrontParamA FrontParamB \
             FrontSurface FrontSurfaceInput LeftParamA LeftParamB LeftSurface \
             LeftSurfaceInput Locked Material Name Reflectance RightParamA RightParamB \
             RightSurface RightSurfaceInput RotVelocity TopParamA TopParamB TopSurface \
             TopSurfaceInput Transparency Velocity formFactorRaw shape size"
        );
    }
    #[test]
    fn part_properties_match_a_native_saved_part() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("RobloxStudio/Contents/Resources/content/fonts/characterR15_2.rbxm");
        let Ok(native) = Model::read_file(&path) else {
            eprintln!("skipped: characterR15_2.rbxm not installed");
            return;
        };
        let native = native.instances.iter().find(|i| i.class == "Part").unwrap();
        let ours = Workspace::studio_new_document().to_model().unwrap();
        let types = |i: &ModelInstance| -> Vec<(String, u8)> {
            i.properties
                .iter()
                .map(|(k, v)| (k.clone(), v.type_id()))
                .collect()
        };
        assert_eq!(types(&ours.instances[0]), types(native));
    }
    #[test]
    fn baseplate_matches_native_new_document() {
        let w = Workspace::studio_new_document();
        assert_eq!(w.parts.len(), 1);
        let b = &w.parts[0];
        assert_eq!(b.extents(), ([-256.0, -20.0, -256.0], [256.0, 0.0, 256.0]));
        assert_eq!(b.surfaces, [0, 3, 0, 0, 4, 0]);
        assert_eq!(w.next_spawn_location().unwrap(), [0.0, 100.0, 0.0]);
    }
    #[test]
    fn a_ball_keeps_a_uniform_size() {
        let mut part = PartInstance::default();
        part.set_shape(SHAPE_BALL);
        assert_eq!(part.size, [4.0; 3]);
        part.set_size([3.0, 1.0, 2.0]);
        assert_eq!(part.size, [3.0; 3]);
        part.set_size([0.1, 5.0, 5.0]);
        assert_eq!(part.size, [0.2; 3]);
        // Back to a block: the size stays as it is.
        part.set_shape(SHAPE_BLOCK);
        part.set_size([3.0, 1.0, 2.0]);
        assert_eq!(part.size, [3.0, 1.0, 2.0]);
    }
    #[test]
    fn spawn_moves_over_extents_that_miss_the_origin() {
        let mut w = Workspace::studio_new_document();
        w.parts[0].frame.position = [1000.0, -10.0, 0.0];
        assert_eq!(w.next_spawn_location().unwrap(), [1000.0, 80.0, 0.0]);
    }
    #[test]
    fn placement_turns_the_character_to_face_minus_z() {
        let spawn = CoordinateFrame {
            position: [0.0, 100.0, 0.0],
            rotation: IDENTITY,
        };
        let head = CoordinateFrame {
            position: [0.0, 19.5, 22.5],
            rotation: [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]],
        };
        let t = character_placement(&spawn, &head);
        let placed = compose(&t, &head);
        assert_eq!(placed.position, [0.0, 107.0, 0.0]);
        assert_eq!(placed.rotation, IDENTITY);
        let arm = compose(
            &t,
            &CoordinateFrame {
                position: [1.5, 18.0, 22.5],
                rotation: head.rotation,
            },
        );
        assert_eq!(arm.position, [-1.5, 105.5, 0.0]);
    }
}
