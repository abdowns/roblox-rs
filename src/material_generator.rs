//! RBX::Graphics::MaterialGenerator for Workspace Parts at the low-quality
//! level the renderer draws: which technique a Part's material and
//! Reflectance select, the material texture coordinates the geometry
//! generators write, and the textures the techniques sample.
//!
//! Recovered from:
//! - createDefaultMaterial 010c0e00: Plastic and SmoothPlastic
//!   (`material & !0x10 == 0x100`) with Reflectance > 0.015 take the
//!   reflection variant; every other Part takes the plain one;
//! - createRenderMaterial 010bc630: the low-quality technique (quality 2) is
//!   DefaultPlasticFS (Plastic, SmoothPlastic, or
//!   Default<Name>ReflectionFS with reflection), DefaultNeonFS (Neon),
//!   LowQMaterialWangFS (Cobblestone) or LowQMaterialFS (the rest). The
//!   LowQMaterial techniques bind rbxasset://textures/<lowercase
//!   name>/diffuse.dds (".dds" is the static string 0224dde0, initializer
//!   010c8530) to slot 5 with
//!   sampler (Linear, Wrap), Cobblestone adds wangIndex.dds (MaterialGenerator
//!   +0x2510) to slot 8 with (Point, Wrap), and the constant
//!   LqmatFarTilingFactor is (f, f, 1, 1) with f from the immediates at
//!   010bdcd0..010bdcf2;
//! - getTiling 010c3c40, read with `material::follow_switch`;
//! - the per-Part UV offset in addBlock 010cd104, addSphere 010cb4a4 and
//!   the Wedge generator 010dc8c6: bits 3..8 and 9..14 of the geometry seed
//!   pick two entries of the 64-float table 017f0218 (Brick and WoodPlanks use
//!   the bit values themselves);
//! - the glow factor (FadeDistance_GlowFactor.w): SceneManager::renderBegin
//!   (011c2200) uses the Glow settings' intensity, 2.2 from the SceneManager
//!   constructor immediate at 011c084e, while the PostFX level is below 1
//!   (1.0 otherwise). The low-quality renderer has no post effects.
//!
//! The native geometry seed is `part + (part >> 3)` for the PartInstance's
//! heap address (addPartImpl 010d1850). An address cannot be reproduced, so
//! `part_seed` stands in for it with a fixed hash of the Part's index.
//!
//! The high-quality technique (`hq_technique`, `workspace_materials_quality`)
//! is the second one createRenderMaterial adds: "Default" + name +
//! ("Reflection") + "HQFS" with the vertex program "DefaultStatic" + "HQVS"
//! (SmoothPlastic, Neon) or "SurfaceHQVS" (the rest), Technique lod 1. Its
//! textures come from FUN_010bf300 and the slots shared by every technique
//! from FUN_010bc020. Material::getBestTechnique(lod) (011d8130) returns the
//! first technique whose lod is at least the requested one, and
//! RenderNode::updateRenderQueue (0114a1b0) requests lod 1 when the node's
//! squared camera distance (CullableSceneNode::updateIsCulledByFRM 010a9400,
//! point-to-AABB) is below FrameRateManager::getShadingSqDistance, else
//! lod 2, and lod 0 (the GBuffer techniques, not ported) when the GBuffer
//! setting is on. The shading distance and texture anisotropy per quality
//! level come from the FrameRateManager table (`quality_levels`).
use crate::{
    bundle::Bundle,
    dds::{Bc1Dds, Bc3Dds, luminance_alpha},
    geometry::MaterialUv,
    material::follow_switch,
    native_data::NativeImage,
    texture::PngTexture,
    workspace::Workspace,
};
use iced_x86::{Code, Decoder, DecoderOptions, Mnemonic, OpKind, Register};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
};

pub const PLASTIC: u32 = 0x100;
pub const SMOOTH_PLASTIC: u32 = 0x110;
pub const NEON: u32 = 0x120;
pub const WOOD_PLANKS: u32 = 0x210;
pub const BRICK: u32 = 0x350;
pub const COBBLESTONE: u32 = 0x370;

/// The 20 Part materials createRenderMaterial accepts (any other code returns
/// no material), with the name it builds shader and texture names from.
pub const RENDER_MATERIALS: [(u32, &str); 20] = [
    (0x100, "Plastic"),
    (0x110, "SmoothPlastic"),
    (0x120, "Neon"),
    (0x200, "Wood"),
    (0x210, "WoodPlanks"),
    (0x310, "Marble"),
    (0x320, "Slate"),
    (0x330, "Concrete"),
    (0x340, "Granite"),
    (0x350, "Brick"),
    (0x360, "Pebble"),
    (0x370, "Cobblestone"),
    (0x410, "Rust"),
    (0x420, "Diamondplate"),
    (0x430, "Aluminum"),
    (0x440, "Metal"),
    (0x500, "Grass"),
    (0x510, "Sand"),
    (0x520, "Fabric"),
    (0x600, "Ice"),
];

/// The low-quality technique a Part draws with.
#[derive(Clone, Debug, PartialEq)]
pub enum Technique {
    /// DefaultPlasticFS: the stud texture modulates the vertex color.
    /// `reflection` selects Default<Name>ReflectionFS (createRenderMaterial
    /// appends "Reflection" to the material name, so SmoothPlastic is
    /// DefaultSmoothPlasticReflectionFS), which also blends in the
    /// environment map by the vertex reflectance. Without reflection both
    /// materials use DefaultPlasticFS. `smooth` only changes the name: the
    /// low-quality branch of plastic.hlsl and smoothplastic.hlsl is the same.
    Plastic { reflection: bool, smooth: bool },
    /// DefaultNeonFS: the vertex color times the glow factor, unlit.
    Neon,
    /// LowQMaterialFS / LowQMaterialWangFS with the material's diffuse map.
    LowQuality {
        material: u32,
        folder: String,
        far_tiling: f32,
        wang: bool,
    },
}

impl Technique {
    pub fn name(&self) -> String {
        match self {
            Technique::Plastic {
                reflection: false, ..
            } => "DefaultPlasticFS".into(),
            Technique::Plastic {
                reflection: true,
                smooth: false,
            } => "DefaultPlasticReflectionFS".into(),
            Technique::Plastic {
                reflection: true,
                smooth: true,
            } => "DefaultSmoothPlasticReflectionFS".into(),
            Technique::Neon => "DefaultNeonFS".into(),
            Technique::LowQuality { wang: true, .. } => "LowQMaterialWangFS".into(),
            Technique::LowQuality { .. } => "LowQMaterialFS".into(),
        }
    }
}

fn render_name(material: u32) -> Result<&'static str, String> {
    RENDER_MATERIALS
        .iter()
        .find(|(code, _)| *code == material)
        .map(|(_, name)| *name)
        .ok_or_else(|| format!("material {material:#x} has no render material"))
}

/// LqmatFarTilingFactor.xy (createRenderMaterial 010bdc9a..010bdcf2):
/// 0.25 for Slate, Concrete, Granite, Aluminum, Grass and Ice, 0.5 for Rust
/// and 1 for the other LowQMaterial materials.
pub fn far_tiling(material: u32) -> f32 {
    match material {
        0x320 | 0x330 | 0x340 | 0x430 | 0x500 | 0x600 => 0.25,
        0x410 => 0.5,
        _ => 1.0,
    }
}

/// createDefaultMaterial (010c0e00) then createRenderMaterial's low-quality
/// technique.
pub fn technique(material: u32, reflectance: f32) -> Result<Technique, String> {
    let name = render_name(material)?;
    let reflection = material & !0x10 == PLASTIC && reflectance > 0.015;
    Ok(match material {
        PLASTIC | SMOOTH_PLASTIC => Technique::Plastic {
            reflection,
            smooth: material == SMOOTH_PLASTIC,
        },
        NEON => Technique::Neon,
        _ => Technique::LowQuality {
            material,
            folder: name.to_lowercase(),
            far_tiling: far_tiling(material),
            wang: material == COBBLESTONE,
        },
    })
}

/// A stand-in for the native geometry seed `part + (part >> 3)`: a 16-byte
/// aligned pseudo address hashed from the Part's index in the Workspace.
pub fn part_seed(index: usize) -> u32 {
    let mut z = (index as u32).wrapping_add(1).wrapping_mul(0x9e37_79b9);
    z = (z ^ (z >> 16)).wrapping_mul(0x85eb_ca6b);
    z = (z ^ (z >> 13)).wrapping_mul(0xc2b2_ae35);
    z ^= z >> 16;
    let address = (z & 0x0fff_fff0) | 0x0800_0000;
    address.wrapping_add(address >> 3)
}

pub struct MaterialTables {
    tiling: BTreeMap<u32, f32>,
    offsets: [f32; 64],
    /// SceneManager Glow intensity (011c084e).
    pub glow: f32,
}

impl MaterialTables {
    pub fn load(image: &NativeImage) -> Result<Self, String> {
        let mut tiling = BTreeMap::new();
        for (code, _) in RENDER_MATERIALS {
            let t = follow_switch(
                image,
                0x010c3c40,
                0x010c3e65,
                Register::EAX,
                None,
                Register::XMM0,
                code,
            )?;
            if !t.is_finite() || t <= 0.0 {
                return Err(format!("invalid native tiling for {code:#x}"));
            }
            tiling.insert(code, t);
        }
        let bytes = image.region(0x017f0218, 256)?;
        let offsets: [f32; 64] = std::array::from_fn(|i| {
            f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
        });
        if offsets.iter().any(|v| !(0.0..=1.0).contains(v)) {
            return Err("invalid native material UV offset table".into());
        }
        // MOV dword ptr [EAX + 0x4], imm32.
        let code = image.region(0x011c084e, 7)?;
        if code[..3] != [0xc7, 0x40, 0x04] {
            return Err("unexpected SceneManager glow initializer".into());
        }
        let glow = f32::from_le_bytes(code[3..7].try_into().unwrap());
        if !glow.is_finite() || glow <= 0.0 {
            return Err("invalid native glow intensity".into());
        }
        Ok(Self {
            tiling,
            offsets,
            glow,
        })
    }

    /// MaterialGenerator::getTiling (010c3c40).
    pub fn tiling(&self, material: u32) -> Result<f32, String> {
        self.tiling
            .get(&material)
            .copied()
            .ok_or_else(|| format!("material {material:#x} has no native tiling"))
    }

    /// The geometry generators' UV origin and scale for one Part. Brick and
    /// WoodPlanks start at the seed's bit values; the rest at
    /// (2 T[i] - 1) * (x + y + z) / 2, in the native float order.
    pub fn uv(&self, material: u32, size: [f32; 3], seed: u32) -> Result<MaterialUv, String> {
        let (a, b) = ((seed >> 3 & 0x3f) as usize, (seed >> 9 & 0x3f) as usize);
        let offset = if material == BRICK || material == WOOD_PLANKS {
            [a as f32, b as f32]
        } else {
            let s = ((size[0] + size[1]) + size[2]) * 0.5;
            let t = |i: usize| ((self.offsets[i] + self.offsets[i]) + -1.0) * s;
            [t(a), t(b)]
        };
        Ok(MaterialUv {
            tiling: self.tiling(material)?,
            offset,
        })
    }
}

/// The number of FrameRateManager quality levels (table entries).
pub const QUALITY_LEVELS: u32 = 22;
/// FrameRateManager's settings table (FrameRateManager ctor 0129dfd6 stores
/// it at +0x14c): 22 entries of 0x3c bytes, filled by the static
/// initializer FUN_0129f87c.
const QUALITY_TABLE: u32 = 0x0225_1548;
const QUALITY_STRIDE: u32 = 0x3c;
const QUALITY_INITIALIZER: u32 = 0x0129_f87c;

/// One FrameRateManager quality level: getShadingDistance (0129f59a, entry
/// +0x10) and getTextureAnisotropy (0129f5d8, entry +0x14). The level is
/// the table index (FrameRateManager +0x18).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QualityLevel {
    pub level: u32,
    pub shading_distance: f32,
    pub anisotropy: u32,
}

impl QualityLevel {
    /// getShadingSqDistance (0129f5b0): the distance squared in floats.
    pub fn shading_sq_distance(&self) -> f32 {
        self.shading_distance * self.shading_distance
    }
    /// RenderNode::updateRenderQueue's choice for a node at this squared
    /// distance: lod 1 (high quality) when below the shading distance
    /// squared, else lod 2.
    pub fn high_quality(&self, sq_distance: f32) -> bool {
        sq_distance < self.shading_sq_distance()
    }
    /// FadeDistance_GlowFactor.xy as RenderView::renderPrepareImpl (0119a22d)
    /// sets them: max(0.1, shading distance) and its inverse.
    pub fn fade_distance(&self) -> [f32; 2] {
        let d = if 0.1 <= self.shading_distance {
            self.shading_distance
        } else {
            0.1
        };
        [d, 1.0 / d]
    }
}

/// CullableSceneNode::updateIsCulledByFRM's squared distance from a point
/// to an axis-aligned box (0 inside), in its order: x, then y, then z.
pub fn sq_distance_to_box(point: [f32; 3], min: [f32; 3], max: [f32; 3]) -> f32 {
    let mut d = 0.0f32;
    for axis in 0..3 {
        let half = (max[axis] - min[axis]) * 0.5;
        let offset = point[axis] - (min[axis] + max[axis]) * 0.5;
        if offset < -half {
            d += (half + offset) * (half + offset);
        }
        if half < offset {
            d += (offset - half) * (offset - half);
        }
    }
    d
}

/// The FrameRateManager quality table, read by decoding its static
/// initializer (FUN_0129f87c). The function is straight-line code: it sets
/// a PIC base with `CALL next; POP reg`, loads immediates into registers
/// and stores them (or immediates) to [pic + disp]. CALLs clobber EAX, ECX
/// and EDX. Each entry's +0x10 (float) and +0x14 (int) must be stored by a
/// known value exactly once; no other store (unknown values, bytes, XMM)
/// may overlap them.
/// The 32-bit register a general register write lands in (AL, AH and AX
/// belong to EAX), or None for other registers.
fn full32(r: Register) -> Option<Register> {
    use Register::*;
    Some(match r {
        AL | AH | AX | EAX => EAX,
        CL | CH | CX | ECX => ECX,
        DL | DH | DX | EDX => EDX,
        BL | BH | BX | EBX => EBX,
        SI | ESI => ESI,
        DI | EDI => EDI,
        BP | EBP => EBP,
        SP | ESP => ESP,
        _ => return Option::None,
    })
}

pub fn quality_levels(image: &NativeImage) -> Result<Vec<QualityLevel>, String> {
    let mut registers: BTreeMap<Register, u32> = BTreeMap::new();
    let mut pic: Option<(Register, u32)> = None;
    let mut call_return = None;
    let mut stores: BTreeMap<u32, u32> = BTreeMap::new();
    let mut wide: Vec<(u32, u32)> = Vec::new();
    let mut ip = QUALITY_INITIALIZER;
    let mut finished = false;
    for _ in 0..8192 {
        let bytes = image.region(ip, 16)?;
        let i = Decoder::with_ip(32, bytes, u64::from(ip), DecoderOptions::NONE).decode();
        if i.is_invalid() {
            return Err(format!("invalid quality initializer instruction {ip:08x}"));
        }
        let next = i.next_ip() as u32;
        let address = |i: &iced_x86::Instruction| -> Option<u32> {
            let (base, at) = pic?;
            (i.memory_base() == base && i.memory_index() == Register::None)
                .then(|| at.wrapping_add(i.memory_displacement32()))
        };
        match i.mnemonic() {
            Mnemonic::Ret => {
                finished = true;
                break;
            }
            Mnemonic::Call => {
                if i.op0_kind() == OpKind::NearBranch32 && i.near_branch_target() as u32 == next {
                    call_return = Some(next);
                }
                for r in [Register::EAX, Register::ECX, Register::EDX] {
                    registers.remove(&r);
                }
            }
            // Reads only.
            Mnemonic::Push | Mnemonic::Cmp | Mnemonic::Test => {}
            Mnemonic::Pop if i.op0_kind() == OpKind::Register => {
                if let Some(at) = call_return.take() {
                    pic = Some((i.op0_register(), at));
                }
                registers.remove(&i.op0_register());
            }
            Mnemonic::Mov | Mnemonic::Lea | Mnemonic::Xor
                if i.op0_kind() == OpKind::Register && full32(i.op0_register()).is_some() =>
            {
                let r = full32(i.op0_register()).unwrap();
                let value = match (i.mnemonic(), i.op1_kind()) {
                    (Mnemonic::Mov, OpKind::Immediate32) => Some(i.immediate32()),
                    (Mnemonic::Mov, OpKind::Register) if i.op1_register() == r => None,
                    (Mnemonic::Mov, OpKind::Register) => registers.get(&i.op1_register()).copied(),
                    (Mnemonic::Lea, OpKind::Memory) => address(&i),
                    (Mnemonic::Xor, OpKind::Register) if i.op1_register() == i.op0_register() => {
                        Some(0)
                    }
                    _ => None,
                };
                // Partial-register writes (MOV AL, 1) leave the full
                // register unknown.
                match value {
                    Some(v) if i.op0_register() == r => {
                        registers.insert(r, v);
                    }
                    _ => {
                        registers.remove(&r);
                    }
                }
            }
            _ if i.op_count() > 0 && i.op0_kind() == OpKind::Memory => {
                if let Some(at) = address(&i) {
                    // Dword MOV stores keep their value; any other store
                    // (bytes, XMM) is recorded as up to 16 written bytes.
                    let value = match i.code() {
                        Code::Mov_rm32_imm32 => Some(Some(i.immediate32())),
                        Code::Mov_rm32_r32 => Some(registers.get(&i.op1_register()).copied()),
                        _ => None,
                    };
                    match value {
                        Some(Some(v)) => {
                            if stores.insert(at, v).is_some() {
                                return Err(format!("quality field {at:08x} stored twice"));
                            }
                        }
                        // Unknown values and other stores only matter if
                        // they touch a table field (checked below).
                        Some(None) => wide.push((at, 4)),
                        None => wide.push((at, 16)),
                    }
                }
            }
            _ if i.op_count() > 0 && i.op0_kind() == OpKind::Register => {
                if let Some(r) = full32(i.op0_register()) {
                    registers.remove(&r);
                }
            }
            _ => {}
        }
        ip = next;
    }
    if !finished {
        return Err("quality initializer did not return".into());
    }
    let mut levels = Vec::new();
    for level in 0..QUALITY_LEVELS {
        let entry = QUALITY_TABLE + level * QUALITY_STRIDE;
        let field = |offset: u32| -> Result<u32, String> {
            let at = entry + offset;
            if wide.iter().any(|&(w, n)| w < at + 4 && at < w + n) {
                return Err(format!("quality field {at:08x} written by a wide store"));
            }
            stores
                .get(&at)
                .copied()
                .ok_or_else(|| format!("quality field {at:08x} not stored"))
        };
        let shading_distance = f32::from_bits(field(0x10)?);
        let anisotropy = field(0x14)?;
        if !shading_distance.is_finite()
            || shading_distance < 0.0
            || !(1..=16).contains(&anisotropy)
        {
            return Err(format!("invalid quality level {level}"));
        }
        levels.push(QualityLevel {
            level,
            shading_distance,
            anisotropy,
        });
    }
    Ok(levels)
}

/// The native SamplerState {filter, address}: filter 0 point, 1 linear,
/// 2 anisotropic (the quality level's anisotropy); address 0 wrap, 1 clamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Sampler {
    pub filter: u32,
    pub address: u32,
}

/// A texture a high-quality technique binds: the Technique slot, the
/// rbxasset path and its file under PlatformContent/pc.
#[derive(Clone, Debug, PartialEq)]
pub struct HqTexture {
    pub slot: u32,
    pub asset: String,
    pub path: String,
    pub sampler: Sampler,
}

/// Which DefaultPS variant a high-quality technique compiles to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HqShading {
    /// plastic.hlsl with PIN_HQ: PIN_SURFACE, its own surfaceShader (studs
    /// and normal map from the plastic atlas, noise from normaldetail).
    PlasticSurface,
    /// smoothplastic.hlsl: PIN_PLASTIC with per-pixel specular.
    Plastic,
    /// neon.hlsl: the same output as DefaultNeonFS.
    Neon,
    /// material.hlsl through the per-material file.
    Material { wang: bool },
}

/// createRenderMaterial's high-quality technique for a Part.
#[derive(Clone, Debug, PartialEq)]
pub struct HqTechnique {
    pub material: u32,
    pub reflection: bool,
    pub fragment: String,
    pub vertex: String,
    pub shading: HqShading,
    pub textures: Vec<HqTexture>,
}

const STUDS: &str = "rbxasset://textures/studs.dds";
const ANISOTROPIC_WRAP: Sampler = Sampler {
    filter: 2,
    address: 0,
};

fn hq_texture(slot: u32, asset: String, sampler: Sampler) -> HqTexture {
    let path = format!(
        "Contents/Resources/PlatformContent/pc/{}",
        asset.trim_start_matches("rbxasset://")
    );
    HqTexture {
        slot,
        asset,
        path,
        sampler,
    }
}

/// createRenderMaterial's high-quality technique (010bd05b..010bd2d1):
/// slot 0 studs.dds (2, 0); then FUN_010bf300: Plastic binds
/// plastic/diffuse (5), plastic/normal (6) with (2, 0) and
/// plastic/normaldetail (8) with (1, 0); SmoothPlastic and Neon bind only
/// the TextureManager fallback at slot 5 (FUN_010bc540, never sampled by
/// their shaders, not listed); the rest bind <name>/diffuse (5), normal (6),
/// specular (7) with (2, 0) and normaldetail (8) with (2, 0), or the Wang
/// index with (0, 0) for Cobblestone. The environment map (slot 4,
/// linear clamp, FUN_010bc020) is shared with every technique.
pub fn hq_technique(material: u32, reflectance: f32) -> Result<HqTechnique, String> {
    let name = render_name(material)?;
    let reflection = material & !0x10 == PLASTIC && reflectance > 0.015;
    let surface = !matches!(material, SMOOTH_PLASTIC | NEON);
    let mut textures = vec![hq_texture(0, STUDS.into(), ANISOTROPIC_WRAP)];
    let shading = match material {
        PLASTIC => {
            for (slot, file, sampler) in [
                (5, "diffuse", ANISOTROPIC_WRAP),
                (6, "normal", ANISOTROPIC_WRAP),
                (
                    8,
                    "normaldetail",
                    Sampler {
                        filter: 1,
                        address: 0,
                    },
                ),
            ] {
                textures.push(hq_texture(
                    slot,
                    format!("rbxasset://textures/plastic/{file}.dds"),
                    sampler,
                ));
            }
            HqShading::PlasticSurface
        }
        SMOOTH_PLASTIC => HqShading::Plastic,
        NEON => HqShading::Neon,
        _ => {
            let folder = name.to_lowercase();
            for (slot, file) in [(5, "diffuse"), (6, "normal"), (7, "specular")] {
                textures.push(hq_texture(
                    slot,
                    format!("rbxasset://textures/{folder}/{file}.dds"),
                    ANISOTROPIC_WRAP,
                ));
            }
            if material == COBBLESTONE {
                textures.push(hq_texture(
                    8,
                    "rbxasset://textures/wangIndex.dds".into(),
                    Sampler {
                        filter: 0,
                        address: 0,
                    },
                ));
            } else {
                textures.push(hq_texture(
                    8,
                    format!("rbxasset://textures/{folder}/normaldetail.dds"),
                    ANISOTROPIC_WRAP,
                ));
            }
            HqShading::Material {
                wang: material == COBBLESTONE,
            }
        }
    };
    Ok(HqTechnique {
        material,
        reflection,
        fragment: format!(
            "Default{name}{}HQFS",
            if reflection { "Reflection" } else { "" }
        ),
        vertex: format!(
            "DefaultStatic{}",
            if surface { "SurfaceHQVS" } else { "HQVS" }
        ),
        shading,
        textures,
    })
}

/// One program in the shipped shaders/shaders.json: its source file and
/// preprocessor defines.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderEntry {
    pub name: String,
    pub source: String,
    pub defines: Vec<String>,
}

/// The `"key": "value"` string pairs of each `{...}` object in
/// shaders.json (the file is a flat array of such objects).
pub fn shader_entries(bundle: &Bundle) -> Result<Vec<ShaderEntry>, String> {
    let bytes = bundle.read("Contents/Resources/shaders/shaders.json", 4 * 1024 * 1024)?;
    let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for object in text.split('{').skip(1) {
        let body = object.split('}').next().unwrap_or("");
        let mut fields = BTreeMap::new();
        let quoted: Vec<&str> = body.split('"').collect();
        // key, value at quoted[1], quoted[3]; then quoted[5], quoted[7]...
        let mut k = 1;
        while k + 2 < quoted.len() {
            fields.insert(quoted[k], quoted[k + 2]);
            k += 4;
        }
        let field = |key: &str| fields.get(key).map(|v| v.to_string());
        if let (Some(name), Some(source)) = (field("name"), field("source")) {
            entries.push(ShaderEntry {
                name,
                source,
                defines: field("defines")
                    .unwrap_or_default()
                    .split_whitespace()
                    .map(String::from)
                    .collect(),
            });
        }
    }
    if entries.len() < 100 {
        return Err("shaders.json has too few programs".into());
    }
    Ok(entries)
}

/// The CFG_* constants and flags a material shader source defines.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShaderConfig {
    pub source: String,
    pub values: BTreeMap<String, f32>,
    pub flags: BTreeSet<String>,
}

impl ShaderConfig {
    pub fn value(&self, key: &str) -> f32 {
        self.values.get(key).copied().unwrap_or(0.0)
    }
    pub fn flag(&self, key: &str) -> bool {
        self.flags.contains(key)
    }
}

/// `#define CFG_<NAME> [value]` lines of a shipped shader source
/// (shaders/source/<file>), the per-material constants material.hlsl and
/// plastic.hlsl read.
pub fn shader_config(bundle: &Bundle, source: &str) -> Result<ShaderConfig, String> {
    let bytes = bundle.read(
        &format!("Contents/Resources/shaders/source/{source}"),
        1024 * 1024,
    )?;
    let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
    let mut config = ShaderConfig {
        source: source.into(),
        ..Default::default()
    };
    for line in text.lines() {
        let mut words = line.split_whitespace();
        if words.next() != Some("#define") {
            continue;
        }
        let Some(key) = words.next().and_then(|k| k.strip_prefix("CFG_")) else {
            continue;
        };
        match words.next() {
            Some(value) => {
                let v: f32 = value
                    .parse()
                    .map_err(|_| format!("{source}: CFG_{key} is not a number"))?;
                config.values.insert(key.into(), v);
            }
            None => {
                config.flags.insert(key.into());
            }
        }
    }
    Ok(config)
}

fn load_dds(bundle: &Bundle, path: &str) -> Result<(PngTexture, String), String> {
    let bytes = bundle.read(path, 16 * 1024 * 1024)?;
    let texture = match bytes.get(84..88) {
        Some(b"DXT1") => Bc1Dds::parse(&bytes)?.decode(),
        Some(b"DXT5") => Bc3Dds::parse(&bytes)?.decode(),
        _ if path.ends_with("wangIndex.dds") => luminance_alpha(&bytes)?,
        _ => return Err(format!("{path}: unsupported DDS format")),
    };
    let first = &texture.mips[0];
    let format = match bytes.get(84..88) {
        Some(b"DXT1") => "DXT1",
        Some(b"DXT5") => "DXT5",
        _ => "L8A8",
    };
    let line = format!(
        "format={format} size={}x{} mips={} dds_sha256={:x}",
        first.width,
        first.height,
        texture.mips.len(),
        Sha256::digest(&bytes)
    );
    Ok((texture, line))
}

/// A Workspace's high-quality techniques at one quality level: per Part the
/// technique, per material the shader constants, the decoded textures by
/// path and the shipped shader entries they were checked against.
pub struct HqScene {
    pub level: QualityLevel,
    pub techniques: BTreeMap<String, HqTechnique>,
    pub configs: BTreeMap<u32, ShaderConfig>,
    pub textures: BTreeMap<String, PngTexture>,
    pub report: String,
}

/// The high-quality technique of every Workspace Part at `level` (an index
/// into `quality_levels`). Each technique's fragment and vertex programs
/// must exist in shaders.json with PIN_HQ (createRenderMaterial skips the
/// technique when ShaderManager::getProgram fails), and its textures load.
pub fn workspace_hq(
    bundle: &Bundle,
    image: &NativeImage,
    workspace: &Workspace,
    level: u32,
) -> Result<HqScene, String> {
    let levels = quality_levels(image)?;
    let level = *levels
        .get(level as usize)
        .ok_or_else(|| format!("quality level {level} is outside 0..{QUALITY_LEVELS}"))?;
    let entries = shader_entries(bundle)?;
    let entry = |name: &str| -> Result<&ShaderEntry, String> {
        entries
            .iter()
            .find(|e| e.name == name)
            .filter(|e| e.defines.iter().any(|d| d == "PIN_HQ"))
            .ok_or_else(|| format!("shaders.json has no PIN_HQ program {name}"))
    };
    let mut scene = HqScene {
        level,
        techniques: BTreeMap::new(),
        configs: BTreeMap::new(),
        textures: BTreeMap::new(),
        report: String::new(),
    };
    writeln!(
        scene.report,
        "quality_level level={} shading_distance={} anisotropy={} fade_distance={:?}",
        level.level,
        level.shading_distance,
        level.anisotropy,
        level.fade_distance()
    )
    .unwrap();
    for part in &workspace.parts {
        let t = hq_technique(part.material, part.reflectance)
            .map_err(|e| format!("Part {:?}: {e}", part.name))?;
        let fragment = entry(&t.fragment)?;
        entry(&t.vertex)?;
        if !scene.configs.contains_key(&t.material) {
            let config = shader_config(bundle, &fragment.source)?;
            let mut values = String::new();
            for (k, v) in &config.values {
                write!(values, " {k}={v}").unwrap();
            }
            writeln!(
                scene.report,
                "hq_config material={} source={}{values} flags={:?}",
                render_name(t.material)?,
                config.source,
                config.flags
            )
            .unwrap();
            scene.configs.insert(t.material, config);
        }
        for texture in &t.textures {
            if scene.textures.contains_key(&texture.path) {
                continue;
            }
            let (decoded, line) = load_dds(bundle, &texture.path)?;
            writeln!(
                scene.report,
                "hq_texture asset={} slot={} sampler={:?} {line}",
                texture.asset, texture.slot, texture.sampler
            )
            .unwrap();
            scene.textures.insert(texture.path.clone(), decoded);
        }
        writeln!(
            scene.report,
            "hq_part name={:?} material={} fragment={} vertex={} source={}",
            part.name,
            render_name(part.material)?,
            t.fragment,
            t.vertex,
            fragment.source
        )
        .unwrap();
        scene.techniques.insert(part.name.clone(), t);
    }
    Ok(scene)
}

/// The diffuse maps, Wang index and per-draw techniques of a Workspace.
pub struct MaterialScene {
    pub techniques: BTreeMap<String, Technique>,
    /// Vertex reflectance per draw: (int)(min(Reflectance, 1) * 0.8 * 255)
    /// / 255, 0 at or below 0 (addBlock 010cce01).
    pub reflectance: BTreeMap<String, f32>,
    pub textures: BTreeMap<u32, PngTexture>,
    pub wang: Option<PngTexture>,
    pub glow: f32,
    /// The high-quality techniques when a quality level was chosen; None
    /// draws every Part with its low-quality technique.
    pub hq: Option<HqScene>,
    pub report: String,
}

impl MaterialScene {
    /// A draw's technique: the Workspace's own, or plain Plastic for the
    /// character and fixtures.
    pub fn technique(&self, name: &str, material: u32) -> Result<Technique, String> {
        match self.techniques.get(name) {
            Some(t) => Ok(t.clone()),
            None => technique(material, 0.0),
        }
    }
}

/// The vertex reflectance byte addBlock packs into Extra.a.
pub fn vertex_reflectance(reflectance: f32) -> f32 {
    let clamped = if reflectance <= 1.0 { reflectance } else { 1.0 };
    if 0.0 < reflectance {
        ((clamped * 0.8 * 255.0) as i32) as f32 / 255.0
    } else {
        0.0
    }
}

fn load_diffuse(bundle: &Bundle, folder: &str) -> Result<(PngTexture, String), String> {
    let path = format!("Contents/Resources/PlatformContent/pc/textures/{folder}/diffuse.dds");
    let bytes = bundle.read(&path, 16 * 1024 * 1024)?;
    let texture = match bytes.get(84..88) {
        Some(b"DXT1") => Bc1Dds::parse(&bytes)?.decode(),
        Some(b"DXT5") => Bc3Dds::parse(&bytes)?.decode(),
        _ => return Err(format!("{path}: unsupported DDS format")),
    };
    let first = &texture.mips[0];
    let line = format!(
        "material_texture folder={folder} format={} size={}x{} mips={} dds_sha256={:x}",
        String::from_utf8_lossy(&bytes[84..88]),
        first.width,
        first.height,
        texture.mips.len(),
        Sha256::digest(&bytes)
    );
    Ok((texture, line))
}

/// Techniques and textures for every Workspace Part.
pub fn workspace_materials(
    bundle: &Bundle,
    workspace: &Workspace,
) -> Result<MaterialScene, String> {
    workspace_materials_quality(bundle, workspace, None)
}

/// `workspace_materials` plus, with a quality level, the high-quality
/// techniques the renderer uses for Parts within that level's shading
/// distance.
pub fn workspace_materials_quality(
    bundle: &Bundle,
    workspace: &Workspace,
    quality: Option<u32>,
) -> Result<MaterialScene, String> {
    let image = NativeImage::load(bundle)?;
    let tables = MaterialTables::load(&image)?;
    let mut scene = MaterialScene {
        techniques: BTreeMap::new(),
        reflectance: BTreeMap::new(),
        textures: BTreeMap::new(),
        wang: None,
        glow: tables.glow,
        hq: None,
        report: String::new(),
    };
    for part in &workspace.parts {
        let t = technique(part.material, part.reflectance)
            .map_err(|e| format!("Part {:?}: {e}", part.name))?;
        if let Technique::LowQuality {
            material,
            folder,
            wang,
            ..
        } = &t
        {
            if !scene.textures.contains_key(material) {
                let (texture, line) = load_diffuse(bundle, folder)?;
                writeln!(scene.report, "{line}").unwrap();
                scene.textures.insert(*material, texture);
            }
            if *wang && scene.wang.is_none() {
                let bytes = bundle.read(
                    "Contents/Resources/PlatformContent/pc/textures/wangIndex.dds",
                    1024 * 1024,
                )?;
                let index = luminance_alpha(&bytes)?;
                writeln!(
                    scene.report,
                    "material_wang_index size={}x{} dds_sha256={:x}",
                    index.mips[0].width,
                    index.mips[0].height,
                    Sha256::digest(&bytes)
                )
                .unwrap();
                scene.wang = Some(index);
            }
        }
        writeln!(
            scene.report,
            "material_part name={:?} material={} technique={} reflectance={}",
            part.name,
            render_name(part.material)?,
            t.name(),
            vertex_reflectance(part.reflectance)
        )
        .unwrap();
        scene
            .reflectance
            .insert(part.name.clone(), vertex_reflectance(part.reflectance));
        scene.techniques.insert(part.name.clone(), t);
    }
    writeln!(
        scene.report,
        "workspace_material_pass parts={} textures={} wang={} glow={}",
        workspace.parts.len(),
        scene.textures.len(),
        scene.wang.is_some(),
        scene.glow
    )
    .unwrap();
    if let Some(level) = quality {
        let hq = workspace_hq(bundle, &image, workspace, level)?;
        scene.report.push_str(&hq.report);
        scene.hq = Some(hq);
    }
    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn techniques_follow_create_default_material() {
        assert_eq!(
            technique(PLASTIC, 0.015).unwrap(),
            Technique::Plastic {
                reflection: false,
                smooth: false
            }
        );
        assert_eq!(
            technique(SMOOTH_PLASTIC, 0.5).unwrap().name(),
            "DefaultSmoothPlasticReflectionFS"
        );
        assert_eq!(
            technique(PLASTIC, 0.5).unwrap().name(),
            "DefaultPlasticReflectionFS"
        );
        assert_eq!(
            technique(SMOOTH_PLASTIC, 0.0).unwrap().name(),
            "DefaultPlasticFS"
        );
        // Neon and the textured materials never reflect at low quality.
        assert_eq!(technique(NEON, 1.0).unwrap(), Technique::Neon);
        assert_eq!(
            technique(COBBLESTONE, 1.0).unwrap(),
            Technique::LowQuality {
                material: COBBLESTONE,
                folder: "cobblestone".into(),
                far_tiling: 1.0,
                wang: true
            }
        );
        assert_eq!(
            technique(0x410, 0.0).unwrap().name(),
            "LowQMaterialFS".to_string()
        );
        // Terrain-only materials have no render material.
        assert!(technique(0x530, 0.0).is_err());
    }
    #[test]
    fn vertex_reflectance_quantizes_like_add_block() {
        assert_eq!(vertex_reflectance(0.0), 0.0);
        assert_eq!(vertex_reflectance(-1.0), 0.0);
        assert_eq!(vertex_reflectance(1.0), 204.0 / 255.0);
        assert_eq!(vertex_reflectance(5.0), 204.0 / 255.0);
        assert_eq!(vertex_reflectance(0.5), 102.0 / 255.0);
    }
    #[test]
    fn hq_techniques_follow_create_render_material() {
        let plastic = hq_technique(PLASTIC, 0.0).unwrap();
        assert_eq!(plastic.fragment, "DefaultPlasticHQFS");
        assert_eq!(plastic.vertex, "DefaultStaticSurfaceHQVS");
        assert_eq!(plastic.shading, HqShading::PlasticSurface);
        assert_eq!(
            plastic.textures.iter().map(|t| t.slot).collect::<Vec<_>>(),
            [0, 5, 6, 8]
        );
        assert_eq!(plastic.textures[3].sampler.filter, 1);
        let smooth = hq_technique(SMOOTH_PLASTIC, 0.5).unwrap();
        assert_eq!(smooth.fragment, "DefaultSmoothPlasticReflectionHQFS");
        assert_eq!(smooth.vertex, "DefaultStaticHQVS");
        assert_eq!(hq_technique(NEON, 1.0).unwrap().fragment, "DefaultNeonHQFS");
        let cobblestone = hq_technique(COBBLESTONE, 1.0).unwrap();
        assert_eq!(cobblestone.fragment, "DefaultCobblestoneHQFS");
        assert_eq!(
            cobblestone.textures[4].asset,
            "rbxasset://textures/wangIndex.dds"
        );
        assert_eq!(cobblestone.textures[4].sampler.filter, 0);
        let planks = hq_technique(WOOD_PLANKS, 0.0).unwrap();
        assert_eq!(
            planks.textures[3].path,
            "Contents/Resources/PlatformContent/pc/textures/woodplanks/specular.dds"
        );
        assert!(hq_technique(0x530, 0.0).is_err());
    }
    #[test]
    fn shading_distance_selects_like_render_node() {
        let level = QualityLevel {
            level: 5,
            shading_distance: 40.0,
            anisotropy: 1,
        };
        assert!(level.high_quality(1599.0));
        assert!(!level.high_quality(1600.0));
        let off = QualityLevel {
            shading_distance: 0.0,
            ..level
        };
        assert!(!off.high_quality(0.0));
        assert_eq!(off.fade_distance(), [0.1, 10.0]);
        assert_eq!(
            sq_distance_to_box([0.0, 0.0, 10.0], [-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]),
            81.0
        );
        assert_eq!(
            sq_distance_to_box([0.5, 0.0, 0.0], [-1.0; 3], [1.0; 3]),
            0.0
        );
    }
    #[test]
    fn seeds_are_stable_and_spread() {
        assert_eq!(part_seed(3), part_seed(3));
        let bits: std::collections::BTreeSet<u32> =
            (0..64).map(|i| part_seed(i) >> 3 & 0x3f).collect();
        assert!(bits.len() > 30);
    }
}
