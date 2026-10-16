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
use crate::{
    joints::{self, JointTree, TreePart},
    kernel_joint::SurfaceInput,
    rig::{compose, inverse},
    scene::{CoordinateFrame, Part},
};
use std::fmt::Write;

/// RBX::SurfaceType values stored at Primitive +0xd2+NormalId.
pub const SURFACE_SMOOTH: u32 = 0;
pub const SURFACE_STUDS: u32 = 3;
pub const SURFACE_INLET: u32 = 4;
/// Part.Shape (PartType, PartInstance +0xb7).
pub const SHAPE_BALL: u32 = 0;
pub const SHAPE_BLOCK: u32 = 1;
pub const SHAPE_CYLINDER: u32 = 2;
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
    /// Part.Shape: `SHAPE_BALL`, `SHAPE_BLOCK` or `SHAPE_CYLINDER`. Set it
    /// with `set_shape`, which keeps a Ball's size uniform.
    pub shape: u32,
    /// Primitive +0xbc.
    pub friction: f32,
    pub elasticity: f32,
    pub surfaces: [u32; 6],
    /// Primitive +0xd8 by NormalId: each face's InputType, ParamA and
    /// ParamB (a motor hinge's axle reads its own).
    pub surface_inputs: [SurfaceInput; 6],
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
    /// Viewport demo place (not a native document): the new-document
    /// Baseplate plus explicit unanchored Parts showing the kernel features.
    /// - "Crate" (4 x 6 x 2, red) in the walking path, 8 studs ahead of the
    ///   spawn, for the character to push.
    /// - "Rest" and "Stack" (blue) placed just above the Baseplate and on
    ///   each other.
    /// - "Drop" (yellow) falling flat and "Tumble" (green) falling tilted,
    ///   both released from heights the falling character passes.
    pub fn loose_parts_demo() -> Self {
        let mut workspace = Self::studio_new_document();
        let part = |name: &str, color: u32, position: [f32; 3]| {
            let mut p = PartInstance {
                name: name.into(),
                brick_color: color,
                ..PartInstance::default()
            };
            p.frame.position = position;
            p
        };
        let mut crate_part = part("Crate", 21, [0.0, 3.05, -8.0]);
        crate_part.set_size([4.0, 6.0, 2.0]);
        let rest = part("Rest", 23, [-8.0, 0.65, -6.0]);
        let stack = part("Stack", 23, [-7.5, 1.9, -5.8]);
        let drop = part("Drop", 24, [7.0, 40.0, -6.0]);
        let mut tumble = part("Tumble", 28, [-6.0, 60.0, -14.0]);
        // 30 degrees about Z, then 20 degrees about X.
        let (sz, cz) = 30f32.to_radians().sin_cos();
        let (sx, cx) = 20f32.to_radians().sin_cos();
        let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
        let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
        tumble.frame.rotation = std::array::from_fn(|r| {
            std::array::from_fn(|c| (0..3).map(|k| rz[r][k] * rx[k][c]).sum())
        });
        workspace
            .parts
            .extend([crate_part, rest, stack, drop, tumble]);
        workspace.add_feature_showcase();
        workspace
    }
    /// The demo's showcase of the later kernel features, east of the spawn
    /// (+X), away from the walking path:
    /// - Balls (x 20..44, z -20..-34): "Roll" on an anchored 10 degree
    ///   "Ramp", "Bounce" dropped from height, "Pebble" dropped off-center on
    ///   an anchored "Dome", and "Kick" (diameter 4) on the ground for the
    ///   character to push;
    /// - a welded "L" ("Base" + "Mast", z 8) that falls as one assembly;
    /// - "Hung", glued under an anchored "Holder";
    /// - hinges on anchored blocks (z 16): "Swing" (free Hinge), "Wheel"
    ///   (motor hinge, constant 0.1) and a Motor arm on a loose "Tower".
    fn add_feature_showcase(&mut self) {
        const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let frame = |position: [f32; 3]| CoordinateFrame {
            position,
            rotation: IDENTITY,
        };
        let add = |workspace: &mut Workspace,
                   name: &str,
                   size: [f32; 3],
                   position: [f32; 3],
                   anchored: bool,
                   ball: bool,
                   color: u32| {
            let mut part = PartInstance {
                name: name.into(),
                anchored,
                brick_color: color,
                frame: frame(position),
                ..Default::default()
            };
            if ball {
                part.set_shape(SHAPE_BALL);
            }
            part.set_size(size);
            workspace.parts.push(part);
            workspace.parts.len() - 1
        };
        let joint = |workspace: &mut Workspace,
                     name: &str,
                     kind: u32,
                     parts: [usize; 2],
                     c0: CoordinateFrame,
                     c1: CoordinateFrame| {
            workspace.joints.push(JointInstance {
                name: name.into(),
                kind,
                part0: parts[0],
                part1: parts[1],
                c0,
                c1,
                face: [[0.0; 3]; 4],
                base_angle: 0.0,
                current_angle: 0.0,
                desired_angle: 0.0,
                max_velocity: 0.0,
            });
            workspace.joints.len() - 1
        };
        // Balls. The ramp's low end (-X) rests on the Baseplate.
        let (s, c) = 10f32.to_radians().sin_cos();
        let ramp_center = [34.0, 10.0 * s + 0.5 * c, -20.0];
        let ramp = add(
            self,
            "Ramp",
            [20.0, 1.0, 10.0],
            ramp_center,
            true,
            false,
            194,
        );
        self.parts[ramp].frame.rotation = [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]];
        let local = [8.0f32, 1.55, 0.0];
        let roll = [
            ramp_center[0] + c * local[0] - s * local[1],
            ramp_center[1] + s * local[0] + c * local[1],
            ramp_center[2],
        ];
        add(self, "Roll", [2.0; 3], roll, false, true, 21);
        add(
            self,
            "Bounce",
            [3.0; 3],
            [24.0, 30.0, -30.0],
            false,
            true,
            24,
        );
        add(self, "Dome", [6.0; 3], [40.0, 3.0, -32.0], true, true, 194);
        add(
            self,
            "Pebble",
            [1.0; 3],
            [40.5, 12.0, -32.0],
            false,
            true,
            23,
        );
        add(self, "Kick", [4.0; 3], [8.0, 2.0, -12.0], false, true, 28);
        // Weld: an L dropped from 10 studs.
        let base = add(
            self,
            "Base",
            [4.0, 1.2, 2.0],
            [20.0, 10.0, 8.0],
            false,
            false,
            21,
        );
        let mast = add(
            self,
            "Mast",
            [1.0, 4.0, 1.0],
            [21.5, 12.6, 8.0],
            false,
            false,
            21,
        );
        joint(
            self,
            "Weld",
            joints::WELD,
            [base, mast],
            frame([1.5, 2.6, 0.0]),
            frame([0.0; 3]),
        );
        // Glue: z of C0/C1 along -Y, the Holder's bottom face.
        let holder = add(
            self,
            "Holder",
            [4.0, 1.0, 2.0],
            [30.0, 10.0, 8.0],
            true,
            false,
            194,
        );
        let hung = add(
            self,
            "Hung",
            [4.0, 1.0, 2.0],
            [30.0, 9.0, 8.0],
            false,
            false,
            23,
        );
        let down = [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]];
        let glue = joint(
            self,
            "Glue",
            joints::GLUE,
            [holder, hung],
            CoordinateFrame {
                position: [0.0, -0.5, 0.0],
                rotation: down,
            },
            CoordinateFrame {
                position: [0.0, 0.5, 0.0],
                rotation: down,
            },
        );
        self.joints[glue].face = [
            [-2.0, -1.0, 0.0],
            [2.0, -1.0, 0.0],
            [2.0, 1.0, 0.0],
            [-2.0, 1.0, 0.0],
        ];
        // Hinges (C0 at the anchored block's center, z along +Z).
        let block = [1.0; 3];
        let pivot = add(self, "Pivot", block, [20.0, 12.0, 16.0], true, false, 194);
        let swing = add(
            self,
            "Swing",
            [6.0, 0.4, 1.0],
            [22.5, 12.0, 17.1],
            false,
            false,
            24,
        );
        joint(
            self,
            "Hinge",
            joints::ROTATE,
            [pivot, swing],
            frame([0.0; 3]),
            frame([-2.5, 0.0, -1.1]),
        );
        let axle = add(self, "Axle", block, [30.0, 12.0, 16.0], true, false, 194);
        self.parts[axle].surface_inputs[2] = SurfaceInput {
            input: crate::kernel_joint::INPUT_CONSTANT,
            param_a: 0.0,
            param_b: 0.1,
        };
        let wheel = add(
            self,
            "Wheel",
            [4.0, 4.0, 0.4],
            [30.0, 12.0, 16.8],
            false,
            false,
            28,
        );
        joint(
            self,
            "MotorHinge",
            joints::ROTATE_V,
            [axle, wheel],
            frame([0.0; 3]),
            frame([0.0, 0.0, -0.8]),
        );
        // Motor: an arm turning on top of a loose tower (z of C0/C1 along +Y).
        let up = [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]];
        let tower = add(
            self,
            "Tower",
            [2.0, 1.0, 2.0],
            [40.0, 0.5, 16.0],
            false,
            false,
            194,
        );
        let arm = add(
            self,
            "Arm",
            [4.0, 0.4, 0.4],
            [42.0, 1.2, 16.0],
            false,
            false,
            21,
        );
        let motor = joint(
            self,
            "Motor",
            joints::MOTOR,
            [tower, arm],
            CoordinateFrame {
                position: [0.0, 0.5, 0.0],
                rotation: up,
            },
            CoordinateFrame {
                position: [-2.0, -0.2, 0.0],
                rotation: up,
            },
        );
        self.joints[motor].desired_angle = std::f32::consts::FRAC_PI_2;
        self.joints[motor].max_velocity = 0.05;
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
