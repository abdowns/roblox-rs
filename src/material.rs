//! RBX::MaterialProperties and the kernel's per-Contact ContactParams.
//!
//! Recovered from:
//! - PhysicalProperties(material) 00a4331a: density 00a4346a, elasticity
//!   00a43aac and friction 00a43766 switches, the friction-weight switch
//!   inlined at 00a43369, clamps and elasticity weight 1.0;
//! - Contact::computeContactParams 00ae26d0 -> 00a43fb4: custom properties
//!   (Primitive +0xc4) or the material defaults (+0xc8), weighted friction and
//!   elasticity (00a43e7e), the smaller Primitive stiffness (00b63194 cache
//!   of 00b6312e) and kNeg = kSpring * elasticity factor (00a91fda);
//! - Primitive stiffness 00a92640 / 00a92276 (v8kernel/Constants.cpp).
//!
//! The material switches are read from the binary: `follow_switch` walks the
//! compiled compare/branch tree for one material code and returns the float
//! that reaches the exit. It never executes native code.
use crate::{bundle::Bundle, kernel::ContactParameters, native_data::NativeImage};
use iced_x86::{Code, Decoder, DecoderOptions, Mnemonic, OpKind, Register};
use std::fmt::Write;

/// RBX::PartMaterial codes accepted by every switch of 00a4331a.
pub const PLASTIC: u32 = 0x100;

/// The 0x18-byte PhysicalProperties record (+0 customEnabled).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhysicalProperties {
    /// +0x04, clamped to [0.01, 100].
    pub density: f32,
    /// +0x08, clamped to [0, 1].
    pub elasticity: f32,
    /// +0x0c, clamped to [0, 2].
    pub friction: f32,
    /// +0x10.
    pub friction_weight: f32,
    /// +0x14.
    pub elasticity_weight: f32,
}

/// Contact +0x2c: copied into each new ContactConnector at +0xb0..+0xbc
/// (00ae5a80); the spring reads +0xb0/+0xb4/+0xb8 (00a93074).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContactParams {
    pub k_spring: f32,
    pub k_neg: f32,
    pub friction: f32,
    pub elasticity: f32,
}

impl ContactParams {
    /// The kernel's spring/impulse view of these parameters.
    pub fn parameters(&self) -> ContactParameters {
        ContactParameters {
            compression: self.k_spring,
            decompression: self.k_neg,
            friction: self.friction,
        }
    }
}

/// Geometry kinds that reach Primitive stiffness (Geometry vtable +0xc):
/// Ball = 1 selects the ball table, Block = 2 (vtable 01c866e8 00adf94a).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Ball,
    Block,
}

const DENSITY: (u32, u32, Register) = (0x00a4346a, 0x00a43756, Register::EAX);
const FRICTION: (u32, u32, Register) = (0x00a43766, 0x00a43a9d, Register::EAX);
const ELASTICITY: (u32, u32, Register) = (0x00a43aac, 0x00a43daf, Register::EAX);

pub struct MaterialProfile {
    image: NativeImage,
    density_max: f32,
    density_min: f32,
    one: f32,
    friction_max: f32,
    weight_threshold: f32,
    restitution_steps: [f32; 4],
    restitution_values: [f32; 4],
    restitution_table: [f32; 2],
    stiffness_scale: f32,
    stiffness_factor: f32,
}

fn float(image: &NativeImage, at: u32) -> Result<f32, String> {
    let x = f32::from_le_bytes(image.region(at, 4)?.try_into().unwrap());
    if !x.is_finite() {
        return Err(format!("invalid native material constant {at:08x}"));
    }
    Ok(x)
}

/// Walk one compiled material switch: `CALL next; POP pic` sets the PIC
/// base, `CMP code, imm` and the conditional jumps select the branch, and
/// `MOVSS xmm, [pic+disp]` / `XORPS xmm, xmm` set the result. Any other
/// instruction (the assert path for unknown materials) is an error.
fn follow_switch(
    image: &NativeImage,
    start: u32,
    exit: u32,
    code_register: Register,
    mut pic: Option<(Register, u32)>,
    output: Register,
    code: u32,
) -> Result<f32, String> {
    let code = code as i32;
    let mut ip = start;
    let mut call_return = None;
    let mut compare: Option<i32> = None;
    let mut value: Option<f32> = None;
    for _ in 0..256 {
        if ip == exit {
            return value.ok_or_else(|| format!("material {code:#x} reached exit unset"));
        }
        let bytes = image.region(ip, 16)?;
        let i = Decoder::with_ip(32, bytes, u64::from(ip), DecoderOptions::NONE).decode();
        if i.is_invalid() {
            return Err(format!("invalid material switch instruction {ip:08x}"));
        }
        let next = i.next_ip() as u32;
        let target = || i.near_branch_target() as u32;
        let taken = |c: &dyn Fn(i32) -> bool| -> Result<bool, String> {
            Ok(c(compare.ok_or("material branch without compare")?))
        };
        ip = match i.mnemonic() {
            Mnemonic::Push | Mnemonic::Sub => next,
            Mnemonic::Mov
                if i.op0_kind() == OpKind::Register
                    && (i.op1_kind() == OpKind::Register || i.op1_kind() == OpKind::Memory)
                    && (i.op1_kind() == OpKind::Register || i.memory_base() == Register::EBP) =>
            {
                next
            }
            Mnemonic::Call if i.op0_kind() == OpKind::NearBranch32 && target() == next => {
                call_return = Some(next);
                next
            }
            Mnemonic::Pop if i.op0_kind() == OpKind::Register => {
                pic = Some((
                    i.op0_register(),
                    call_return.take().ok_or("material POP without PIC call")?,
                ));
                next
            }
            Mnemonic::Cmp
                if i.op0_kind() == OpKind::Register
                    && i.op0_register() == code_register
                    && matches!(i.op1_kind(), OpKind::Immediate32 | OpKind::Immediate8to32) =>
            {
                let imm = if i.op1_kind() == OpKind::Immediate32 {
                    i.immediate32() as i32
                } else {
                    i.immediate8to32()
                };
                compare = Some(match code.cmp(&imm) {
                    std::cmp::Ordering::Less => -1,
                    std::cmp::Ordering::Equal => 0,
                    std::cmp::Ordering::Greater => 1,
                });
                next
            }
            Mnemonic::Je if taken(&|c| c == 0)? => target(),
            Mnemonic::Jne if taken(&|c| c != 0)? => target(),
            Mnemonic::Jg if taken(&|c| c > 0)? => target(),
            Mnemonic::Jle if taken(&|c| c <= 0)? => target(),
            Mnemonic::Jl if taken(&|c| c < 0)? => target(),
            Mnemonic::Jge if taken(&|c| c >= 0)? => target(),
            Mnemonic::Je
            | Mnemonic::Jne
            | Mnemonic::Jg
            | Mnemonic::Jle
            | Mnemonic::Jl
            | Mnemonic::Jge => next,
            Mnemonic::Jmp if i.op0_kind() == OpKind::NearBranch32 => target(),
            Mnemonic::Movss
                if i.op0_kind() == OpKind::Register
                    && i.op0_register() == output
                    && i.op1_kind() == OpKind::Memory
                    && i.memory_index() == Register::None
                    && pic.is_some_and(|(r, _)| r == i.memory_base()) =>
            {
                let at = pic.unwrap().1.wrapping_add(i.memory_displacement32());
                value = Some(float(image, at)?);
                next
            }
            Mnemonic::Movss if i.op0_kind() == OpKind::Register && i.op0_register() != output => {
                next
            }
            Mnemonic::Xorps
                if i.code() == Code::Xorps_xmm_xmmm128
                    && i.op0_register() == output
                    && i.op1_kind() == OpKind::Register
                    && i.op1_register() == output =>
            {
                value = Some(0.0);
                next
            }
            _ => {
                return Err(format!(
                    "material {code:#x} is not a supported native material ({ip:08x} {:?})",
                    i.mnemonic()
                ));
            }
        };
    }
    Err(format!("material {code:#x} switch did not terminate"))
}

impl MaterialProfile {
    pub fn load(bundle: &Bundle) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let f = |at| float(&image, at);
        let profile = Self {
            density_max: f(0x01705dd0)?,
            density_min: f(0x01705dcc)?,
            one: f(0x015d8978)?,
            friction_max: f(0x016f752c)?,
            weight_threshold: f(0x01834ecc)?,
            restitution_steps: [
                f(0x01700a30)?,
                f(0x0179fc14)?,
                f(0x01742688)?,
                f(0x0179fc1c)?,
            ],
            restitution_values: [f(0x0179fc0c)?, f(0x0179fc10)?, f(0x0179fc18)?, 0.0],
            restitution_table: [f(0x0179fc30)?, f(0x0179fc34)?],
            stiffness_scale: f(0x0179fc04)?,
            stiffness_factor: f(0x0179fc08)?,
            image,
        };
        if profile.density_min <= 0.0
            || profile.density_max <= profile.density_min
            || profile.one != 1.0
            || profile.friction_max <= 0.0
            || profile.weight_threshold <= 0.0
            || profile.stiffness_scale <= 0.0
            || profile.stiffness_factor <= 0.0
            || !profile.restitution_steps.is_sorted()
        {
            return Err("unexpected native material profile".into());
        }
        Ok(profile)
    }

    fn switch(
        &self,
        (start, exit, register): (u32, u32, Register),
        code: u32,
    ) -> Result<f32, String> {
        follow_switch(
            &self.image,
            start,
            exit,
            register,
            None,
            Register::XMM0,
            code,
        )
    }

    /// PhysicalProperties(material), 00a4331a, with its SSE clamps.
    pub fn defaults(&self, material: u32) -> Result<PhysicalProperties, String> {
        let density = self.switch(DENSITY, material)?;
        let friction = self.switch(FRICTION, material)?;
        let elasticity = self.switch(ELASTICITY, material)?;
        // The friction-weight tree is inlined; EDI holds the PIC base 00a43328.
        let friction_weight = follow_switch(
            &self.image,
            0x00a43369,
            0x00a433e9,
            Register::EBX,
            Some((Register::EDI, 0x00a43328)),
            Register::XMM1,
            material,
        )?;
        // MINSS keeps the first operand when it is smaller; CMPNLESS is true
        // for greater or unordered.
        let min = |limit: f32, v: f32| if limit < v { limit } else { v };
        Ok(PhysicalProperties {
            density: if density <= self.density_min {
                self.density_min
            } else {
                min(self.density_max, density)
            },
            elasticity: if elasticity <= 0.0 {
                0.0
            } else {
                min(self.one, elasticity)
            },
            friction: if friction <= 0.0 {
                0.0
            } else {
                min(self.friction_max, friction)
            },
            friction_weight,
            elasticity_weight: self.one,
        })
    }

    /// 00a43e7e: (w2*v2 + w1*v1) / (w1 + w2), zero unless 1e-6 < w1 + w2.
    fn weighted(&self, w1: f32, v1: f32, w2: f32, v2: f32) -> f32 {
        let denominator = w1 + w2;
        if self.weight_threshold >= denominator {
            return 0.0;
        }
        (w2 * v2 + w1 * v1) / denominator
    }

    /// 00a91fda: elasticity -> kNeg/kSpring.
    pub fn restitution_factor(&self, elasticity: f32) -> f32 {
        let [s0, s1, s2, s3] = self.restitution_steps;
        if s0 > elasticity {
            self.restitution_values[0]
        } else if s1 > elasticity {
            self.restitution_values[1]
        } else if s2 > elasticity {
            self.restitution_values[2]
        } else {
            self.restitution_table[usize::from(s3 > elasticity)]
        }
    }

    fn table(&self, at: u32) -> f32 {
        // Addresses are inside the verified constant region checked by load.
        float(&self.image, at).unwrap_or(f32::NAN)
    }

    /// 00a92276: stiffness units for a clipped, sorted size (Vector3int16).
    fn size_stiffness(&self, v: [i16; 3], ball: bool) -> f32 {
        let [x, y, z] = v.map(i32::from);
        let t = |at: u32, i: i32| self.table(at.wrapping_add((i as u32).wrapping_mul(4)));
        if ball {
            return if (x - 1) as u32 > 3 {
                (x.wrapping_mul(x).wrapping_mul(x)) as f32 * self.table(0x0179fc2c)
            } else {
                t(0x0179fc5c, x)
            };
        }
        match x {
            2 => match y {
                3 => match z {
                    3 => self.table(0x016f7544),
                    4 => self.table(0x01836338),
                    _ => z as f32 * t(0x0179fc38, i32::from(z < 15)),
                },
                2 if ((z - 2) as u32) < 3 => t(0x0179fca0, z),
                2 => z as f32 * t(0x0179fc40, i32::from(z < 15)),
                _ => y as f32 * self.table(0x0179fc28) * z as f32,
            },
            1 => match y {
                1 if ((z - 1) as u32) < 4 => t(0x0179fc6c, z),
                1 => z as f32 * self.table(0x016f753c),
                2 if ((z - 2) as u32) < 3 => t(0x0179fc94, z),
                2 => z as f32 * t(0x0179fc58, i32::from(z < 15)),
                3 => z as f32 * t(0x0179fc50, i32::from(z < 7)),
                4 => z as f32 * t(0x0179fc48, i32::from(z < 7)),
                _ => (y as f32 * self.table(0x016f7538) + self.table(0x0179fc28)) * z as f32,
            },
            _ => z.wrapping_mul(y.wrapping_mul(x)) as f32 * self.table(0x016ff160),
        }
    }

    /// Primitive stiffness, 00b6312e -> 00a92640(size, shape == Ball).
    pub fn stiffness(&self, size: [f32; 3], shape: Shape) -> Result<f32, String> {
        if size.iter().any(|v| !v.is_finite()) {
            return Err("invalid stiffness size".into());
        }
        // 006b213e: ascending sort with strict compares.
        let mut s = size;
        if s[2] < s[1] {
            s.swap(1, 2);
        }
        if s[1] < s[0] {
            s.swap(0, 1);
        }
        if s[2] < s[1] {
            s.swap(1, 2);
        }
        let clipped = s.map(|v| if v > self.one { v } else { self.one });
        // G3D::Vector3int16(Vector3) 01259528: (int16)(int)floor(v + 0.5).
        let rounded = clipped.map(|v| (f64::from(v) + 0.5).floor() as i32 as i16);
        let r = self.size_stiffness(rounded, shape == Shape::Ball);
        let r = if s[0] < self.one { r * s[0] } else { r };
        let k = r * self.stiffness_scale * self.stiffness_scale * self.stiffness_factor;
        if !k.is_finite() || k <= 0.0 {
            return Err(format!("invalid native stiffness for {size:?}"));
        }
        Ok(k)
    }

    /// Contact::computeContactParams (00a43fb4) for primitives A and B.
    pub fn contact_params(
        &self,
        a: &PhysicalProperties,
        a_stiffness: f32,
        b: &PhysicalProperties,
        b_stiffness: f32,
    ) -> ContactParams {
        let friction = self.weighted(a.friction_weight, a.friction, b.friction_weight, b.friction);
        let elasticity = self.weighted(
            a.elasticity_weight,
            a.elasticity,
            b.elasticity_weight,
            b.elasticity,
        );
        // MINSS B, A.
        let k_spring = if b_stiffness < a_stiffness {
            b_stiffness
        } else {
            a_stiffness
        };
        ContactParams {
            k_spring,
            k_neg: k_spring * self.restitution_factor(elasticity),
            friction,
            elasticity,
        }
    }
}

/// Every material code named by the density switch, in code order.
pub const MATERIALS: [(u32, &str); 36] = [
    (0x100, "Plastic"),
    (0x110, "SmoothPlastic"),
    (0x120, "Neon"),
    (0x200, "Wood"),
    (0x210, "WoodPlanks"),
    (0x310, "Marble"),
    (0x314, "Basalt"),
    (0x320, "Slate"),
    (0x324, "CrackedLava"),
    (0x330, "Concrete"),
    (0x334, "Limestone"),
    (0x340, "Granite"),
    (0x344, "Pavement"),
    (0x350, "Brick"),
    (0x360, "Pebble"),
    (0x370, "Cobblestone"),
    (0x380, "Rock"),
    (0x390, "Sandstone"),
    (0x410, "CorrodedMetal"),
    (0x420, "DiamondPlate"),
    (0x430, "Foil"),
    (0x440, "Metal"),
    (0x500, "Grass"),
    (0x504, "LeafyGrass"),
    (0x510, "Sand"),
    (0x520, "Fabric"),
    (0x530, "Snow"),
    (0x540, "Mud"),
    (0x550, "Ground"),
    (0x560, "Asphalt"),
    (0x570, "Salt"),
    (0x600, "Ice"),
    (0x610, "Glacier"),
    (0x700, "Glass"),
    (0x800, "Air"),
    (0xffff, "Water"),
];

pub fn audit(bundle: &Bundle) -> Result<String, String> {
    let profile = MaterialProfile::load(bundle)?;
    let mut report = String::new();
    let mut supported = 0;
    for (code, name) in MATERIALS {
        match profile.defaults(code) {
            Ok(p) => {
                supported += 1;
                writeln!(
                    report,
                    "material {name} code={code:#x} density={} elasticity={} friction={} friction_weight={} elasticity_weight={}",
                    p.density, p.elasticity, p.friction, p.friction_weight, p.elasticity_weight
                )
                .unwrap();
            }
            Err(e) => writeln!(report, "material {name} code={code:#x} unsupported: {e}").unwrap(),
        }
    }
    let plastic = profile.defaults(PLASTIC)?;
    if plastic.density != 0.7
        || plastic.friction != 0.3
        || plastic.elasticity != 0.5
        || plastic.friction_weight != 1.0
        || plastic.elasticity_weight != 1.0
    {
        return Err(format!("unexpected native Plastic {plastic:?}"));
    }
    for (name, size) in [
        ("Head", [2.0, 1.0, 1.0]),
        ("Torso", [2.0, 2.0, 1.0]),
        ("Part", [4.0, 1.2, 2.0]),
        ("Baseplate", [512.0, 20.0, 512.0]),
        ("Brick1", [1.0, 1.0, 1.0]),
        ("Plate", [4.0, 0.4, 2.0]),
    ] {
        writeln!(
            report,
            "stiffness {name} size={size:?} k={}",
            profile.stiffness(size, Shape::Block)?
        )
        .unwrap();
    }
    let torso = profile.stiffness([2.0, 2.0, 1.0], Shape::Block)?;
    let baseplate = profile.stiffness([512.0, 20.0, 512.0], Shape::Block)?;
    let params = profile.contact_params(&plastic, torso, &plastic, baseplate);
    if params.k_spring != torso || params.friction != 0.3 || params.elasticity != 0.5 {
        return Err(format!("unexpected Plastic contact params {params:?}"));
    }
    for e in [0.0, 0.05, 0.25, 0.26, 0.5, 0.51, 0.75, 0.76, 1.0] {
        writeln!(
            report,
            "restitution elasticity={e} factor={}",
            profile.restitution_factor(e)
        )
        .unwrap();
    }
    writeln!(
        report,
        "contact_params torso_baseplate k_spring={} k_neg={} friction={} elasticity={}",
        params.k_spring, params.k_neg, params.friction, params.elasticity
    )
    .unwrap();
    writeln!(
        report,
        "material_pass supported={supported}/{} native_differential=not_tested",
        MATERIALS.len()
    )
    .unwrap();
    Ok(report)
}
