//! Lighting service defaults, Roblox's G3D::LightingParameters::setTime
//! (012933c0) and RenderView::presetLighting (01197280) / SceneManager::setFog
//! (011c5ab0). Defaults and keyframe colors are recovered by tracing the
//! original constructors' stores; no table is embedded here.
use crate::{bundle::Bundle, native_data::NativeImage};
use iced_x86::{Decoder, DecoderOptions, Instruction, Mnemonic, OpKind, Register};
use std::collections::{BTreeMap, HashMap};

const COLOR3_WHITE: u64 = 0x0124f8d8;
const COLOR3_BLACK: u64 = 0x0124f666;
const LIGHTING_CTOR: u32 = 0x0095d672;
const PARAMETERS_CTOR: u32 = 0x0129324e;
const SET_TIME: u32 = 0x012933c0;
/// G3D::pi() as compiled into setTime (not std's f64 PI).
#[allow(clippy::approx_constant)]
const G3D_PI: f64 = 3.1415926535898;

fn is_xmm(r: Register) -> bool {
    (Register::XMM0 as u32..=Register::XMM31 as u32).contains(&(r as u32))
}
fn full32(r: Register) -> Register {
    match r {
        Register::AL | Register::AH | Register::AX => Register::EAX,
        Register::CL | Register::CH | Register::CX => Register::ECX,
        Register::DL | Register::DH | Register::DX => Register::EDX,
        Register::BL | Register::BH | Register::BX => Register::EBX,
        r => r,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Key {
    Absolute(u32),
    Relative(Register, i64),
}

/// Straight-line store tracer for constructor/static-init blocks: follows the
/// PIC base, 32-bit immediates, SSE constant loads, Color3::white()/black()
/// results, scalar multiplies and stack spills. Unknown values are dropped.
fn trace(
    image: &NativeImage,
    start: u32,
    length: usize,
) -> Result<(BTreeMap<Key, u32>, Vec<u32>), String> {
    let code = image.region(start, length)?;
    let mut decoder = Decoder::with_ip(32, code, u64::from(start), DecoderOptions::NONE);
    let mut pic: Option<(Register, u32)> = None;
    let mut pending_pic: Option<u32> = None;
    let mut gpr: HashMap<Register, u32> = HashMap::new();
    let mut constant_pointer: HashMap<Register, f32> = HashMap::new();
    let mut xmm: HashMap<Register, [Option<u32>; 4]> = HashMap::new();
    let mut stack: HashMap<i64, u32> = HashMap::new();
    let mut stores = BTreeMap::new();
    let mut lea_targets = Vec::new();
    let mut instruction = Instruction::default();
    let read = |address: u32, n: usize| -> Vec<Option<u32>> {
        (0..n)
            .map(|i| {
                image
                    .region(address + 4 * i as u32, 4)
                    .ok()
                    .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            })
            .collect()
    };
    while decoder.can_decode() {
        decoder.decode_out(&mut instruction);
        let i = &instruction;
        if i.mnemonic() == Mnemonic::Ret {
            break;
        }
        let memory_key = |i: &Instruction| -> Option<Key> {
            if i.memory_index() != Register::None {
                return None;
            }
            let disp = i.memory_displacement32();
            match pic {
                Some((reg, base)) if i.memory_base() == reg => {
                    Some(Key::Absolute(base.wrapping_add(disp)))
                }
                _ if i.memory_base() == Register::None => Some(Key::Absolute(disp)),
                _ => Some(Key::Relative(i.memory_base(), disp as i32 as i64)),
            }
        };
        let load_scalar = |i: &Instruction, op: u32| -> Option<u32> {
            if i.op_kind(op) == OpKind::Register {
                return xmm.get(&i.op_register(op)).and_then(|v| v[0]);
            }
            if let Some(value) = constant_pointer.get(&i.memory_base()) {
                return Some(value.to_bits());
            }
            match memory_key(i)? {
                Key::Absolute(a) => read(a, 1)[0],
                Key::Relative(Register::EBP, d) => stack.get(&d).copied(),
                _ => None,
            }
        };
        match i.mnemonic() {
            Mnemonic::Call => {
                let target = i.near_branch_target();
                if target == i.next_ip() {
                    pending_pic = Some(i.next_ip() as u32);
                    continue;
                }
                constant_pointer.remove(&Register::EAX);
                gpr.remove(&Register::EAX);
                if target == COLOR3_WHITE {
                    constant_pointer.insert(Register::EAX, 1.0);
                } else if target == COLOR3_BLACK {
                    constant_pointer.insert(Register::EAX, 0.0);
                }
            }
            Mnemonic::Pop => {
                if let Some(base) = pending_pic.take() {
                    pic = Some((i.op0_register(), base));
                }
            }
            Mnemonic::Lea => {
                if let Some(Key::Absolute(a)) = memory_key(i) {
                    lea_targets.push(a);
                }
                gpr.remove(&i.op0_register());
                constant_pointer.remove(&i.op0_register());
            }
            Mnemonic::Xor
                if i.op0_kind() == OpKind::Register
                    && i.op1_kind() == OpKind::Register
                    && i.op0_register() == i.op1_register() =>
            {
                if is_xmm(i.op0_register()) {
                    xmm.insert(i.op0_register(), [Some(0); 4]);
                } else {
                    gpr.insert(i.op0_register(), 0);
                }
            }
            Mnemonic::Xorps if i.op0_register() == i.op1_register() => {
                xmm.insert(i.op0_register(), [Some(0); 4]);
            }
            Mnemonic::Mov => match (i.op0_kind(), i.op1_kind()) {
                (OpKind::Register, OpKind::Immediate32) => {
                    gpr.insert(i.op0_register(), i.immediate32());
                }
                (OpKind::Register, OpKind::Register) => {
                    match gpr.get(&i.op1_register()).copied() {
                        Some(v) => gpr.insert(i.op0_register(), v),
                        None => gpr.remove(&i.op0_register()),
                    };
                }
                (OpKind::Memory, OpKind::Immediate32 | OpKind::Immediate8) => {
                    if let Some(key) = memory_key(i) {
                        stores.insert(key, i.immediate(1) as u32);
                    }
                }
                (OpKind::Memory, OpKind::Register) => {
                    if let (Some(key), Some(v)) = (memory_key(i), gpr.get(&i.op1_register())) {
                        stores.insert(key, *v);
                    }
                }
                (OpKind::Register, OpKind::Memory) => {
                    // Loads through Color3::white()/black() pointers.
                    let value = constant_pointer.get(&i.memory_base()).map(|v| v.to_bits());
                    constant_pointer.remove(&i.op0_register());
                    match value {
                        Some(v) => gpr.insert(i.op0_register(), v),
                        None => gpr.remove(&i.op0_register()),
                    };
                }
                (OpKind::Register, _) => {
                    gpr.remove(&i.op0_register());
                    constant_pointer.remove(&i.op0_register());
                }
                _ => {}
            },
            Mnemonic::Movss => {
                if i.op0_kind() == OpKind::Register {
                    let value = load_scalar(i, 1);
                    xmm.insert(i.op0_register(), [value, Some(0), Some(0), Some(0)]);
                } else if let (Some(key), Some(v)) =
                    (memory_key(i), xmm.get(&i.op1_register()).and_then(|v| v[0]))
                {
                    match key {
                        Key::Relative(Register::EBP, d) => {
                            stack.insert(d, v);
                        }
                        key => {
                            stores.insert(key, v);
                        }
                    }
                }
            }
            Mnemonic::Mulss => {
                let a = xmm.get(&i.op0_register()).and_then(|v| v[0]);
                let b = load_scalar(i, 1);
                let product = a
                    .zip(b)
                    .map(|(a, b)| (f32::from_bits(a) * f32::from_bits(b)).to_bits());
                xmm.insert(i.op0_register(), [product, None, None, None]);
            }
            Mnemonic::Movaps | Mnemonic::Movups | Mnemonic::Movapd | Mnemonic::Movupd => {
                if i.op0_kind() == OpKind::Register {
                    let lanes = match (i.op1_kind(), memory_key(i)) {
                        (OpKind::Register, _) => {
                            xmm.get(&i.op1_register()).copied().unwrap_or([None; 4])
                        }
                        (_, Some(Key::Absolute(a))) => {
                            let v = read(a, 4);
                            [v[0], v[1], v[2], v[3]]
                        }
                        _ => [None; 4],
                    };
                    xmm.insert(i.op0_register(), lanes);
                } else if let Some(key) = memory_key(i) {
                    let lanes = xmm.get(&i.op1_register()).copied().unwrap_or([None; 4]);
                    for (n, lane) in lanes.iter().enumerate() {
                        if let Some(v) = lane {
                            let k = match key {
                                Key::Absolute(a) => Key::Absolute(a + 4 * n as u32),
                                Key::Relative(r, d) => Key::Relative(r, d + 4 * n as i64),
                            };
                            stores.insert(k, *v);
                        }
                    }
                }
            }
            _ => {
                if i.op_count() > 0 && i.op0_kind() == OpKind::Register {
                    let r = i.op0_register();
                    if is_xmm(r) {
                        xmm.remove(&r);
                    } else {
                        gpr.remove(&full32(r));
                        constant_pointer.remove(&full32(r));
                    }
                }
            }
        }
    }
    Ok((stores, lea_targets))
}

fn f(bits: u32) -> f32 {
    f32::from_bits(bits)
}

#[derive(Clone, Debug, PartialEq)]
pub struct LightingProperties {
    pub clear_color: [f32; 4],
    pub shadow_color: [f32; 3],
    pub fog_color: [f32; 3],
    pub fog_start: f32,
    pub fog_end: f32,
    pub time_of_day: f64,
    pub color_shift_top: [f32; 3],
    pub color_shift_bottom: [f32; 3],
    pub brightness: f32,
    pub ambient: [f32; 3],
    pub outdoor_ambient: [f32; 3],
    pub global_shadows: bool,
    pub outlines: bool,
    pub geographic_latitude: f32,
}

#[derive(Clone, Debug)]
struct Spline {
    times: Vec<f64>,
    colors: Vec<[f32; 3]>,
}
impl Spline {
    /// FUN_0129420b.
    fn eval(&self, t: f64) -> [f32; 3] {
        let n = self.times.len();
        if n == 1 || t < self.times[0] {
            return self.colors[0];
        }
        for i in 1..n {
            if t < self.times[i] {
                let s = (self.times[i] - t) / (self.times[i] - self.times[i - 1]);
                let (a, b) = ((1.0 - s) as f32, s as f32);
                let (p, q) = (self.colors[i - 1], self.colors[i]);
                return [
                    p[0] * b + q[0] * a,
                    p[1] * b + q[1] * a,
                    b * p[2] + a * q[2],
                ];
            }
        }
        self.colors[n - 1]
    }
}

#[derive(Clone, Debug)]
pub struct LightingProfile {
    pub properties: LightingProperties,
    splines: [Spline; 5],
    sun_tilt: f64,
    moon_tilt: f64,
}

/// G3D::LightingParameters fields used by the renderer.
#[derive(Clone, Debug, PartialEq)]
pub struct SkyParameters {
    pub emissive: [f32; 3],
    pub sky_ambient: [f32; 3],
    pub diffuse_ambient: [f32; 3],
    pub light_color: [f32; 3],
    pub ambient: [f32; 3],
    pub light_direction: [f32; 3],
    pub moon_is_source: bool,
    pub true_sun: [f32; 3],
    pub sun: [f32; 3],
    pub true_moon: [f32; 3],
    pub moon: [f32; 3],
    pub moon_phase: f64,
    pub sky_horizon: [f32; 3],
}

/// SceneManager globals written by presetLighting and setFog.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderLighting {
    pub ambient: [f32; 3],
    pub lamp0: [f32; 3],
    pub lamp0_direction: [f32; 3],
    pub lamp1: [f32; 3],
    pub fog_color: [f32; 3],
    pub fog_params: [f32; 4],
}

impl LightingProfile {
    pub fn load(bundle: &Bundle) -> Result<Self, String> {
        let image = NativeImage::load(bundle)?;
        let (lighting, leas) = trace(&image, LIGHTING_CTOR, 0x260)?;
        // `this` is the register used for the +0x17c FogEnd immediate.
        let this = lighting
            .keys()
            .find_map(|k| match k {
                Key::Relative(r, 0x17c) => Some(*r),
                _ => None,
            })
            .ok_or("Lighting constructor FogEnd store not found")?;
        let get = |offset: i64| -> Result<f32, String> {
            lighting
                .get(&Key::Relative(this, offset))
                .map(|b| f(*b))
                .ok_or_else(|| format!("Lighting default +{offset:#x} not recovered"))
        };
        let color = |offset: i64| -> Result<[f32; 3], String> {
            Ok([get(offset)?, get(offset + 4)?, get(offset + 8)?])
        };
        let byte = |offset: i64| -> bool {
            lighting
                .get(&Key::Relative(this, offset))
                .is_some_and(|b| b & 0xff != 0)
        };
        let time = leas
            .iter()
            .find_map(|a| {
                let s = image.region(*a, 8).ok()?;
                let text = std::str::from_utf8(s).ok()?;
                let mut parts = text.split(':').map(|p| p.parse::<u32>().ok());
                let (h, m, sec) = (parts.next()??, parts.next()??, parts.next()??);
                Some(f64::from(h * 3600 + m * 60 + sec))
            })
            .ok_or("Lighting TimeOfDay literal not found")?;
        let (parameters, _) = trace(&image, PARAMETERS_CTOR, 0x200)?;
        let latitude = parameters
            .iter()
            .find_map(|(k, v)| matches!(k, Key::Relative(_, 0xf4)).then_some(f(*v)))
            .ok_or("LightingParameters latitude default not found")?;
        let properties = LightingProperties {
            clear_color: [get(0x150)?, get(0x154)?, get(0x158)?, get(0x15c)?],
            shadow_color: color(0x160)?,
            fog_color: color(0x16c)?,
            fog_start: get(0x178)?,
            fog_end: get(0x17c)?,
            time_of_day: time,
            color_shift_top: color(0x188)?,
            color_shift_bottom: color(0x194)?,
            brightness: get(0x1a0)?,
            ambient: color(0x1a4)?,
            outdoor_ambient: color(0x1b0)?,
            global_shadows: byte(0x1bc),
            outlines: byte(0x1bd),
            geographic_latitude: latitude,
        };
        let (statics, _) = trace(&image, SET_TIME, 0x1400)?;
        let table = |times_at: u32, colors_at: u32, keys: usize| -> Result<Spline, String> {
            let times = (0..keys)
                .map(|k| {
                    image
                        .region(times_at + 8 * k as u32, 8)
                        .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let colors = (0..keys)
                .map(|k| {
                    std::array::from_fn(|c| {
                        statics
                            .get(&Key::Absolute(colors_at + 12 * k as u32 + 4 * c as u32))
                            .map_or(0.0, |b| f(*b))
                    })
                })
                .collect();
            Ok(Spline { times, colors })
        };
        let double = |at: u32| -> Result<f64, String> {
            // Static initializer 012942fe writes these doubles as two halves.
            let low = statics_or(&image, 0x012942fe, at)?;
            let high = statics_or(&image, 0x012942fe, at + 4)?;
            Ok(f64::from_bits(u64::from(high) << 32 | u64::from(low)))
        };
        let result = Self {
            properties,
            splines: [
                table(0x018360a0, 0x02251240, 10)?,
                table(0x018360f0, 0x022512c0, 10)?,
                table(0x01836140, 0x02251340, 10)?,
                table(0x01836190, 0x022513c0, 10)?,
                table(0x018361e0, 0x02251440, 12)?,
            ],
            sun_tilt: double(0x02251228)?,
            moon_tilt: double(0x02251230)?,
        };
        if result
            .splines
            .iter()
            .any(|s| s.colors.iter().flatten().any(|v| !(0.0..=1.0).contains(v)))
            || !(0.0..1.0).contains(&result.sun_tilt)
            || !(0.0..1.0).contains(&result.moon_tilt)
        {
            return Err("unexpected recovered LightingParameters tables".into());
        }
        Ok(result)
    }

    /// G3D::LightingParameters::setTime with physicallyCorrect = true.
    pub fn sky(&self, time: f64) -> SkyParameters {
        let local = time - (time / 86400.0).floor() * 86400.0;
        let angle = ((local * 6.2831854820251465) / 86400.0) as f32;
        let a = f64::from(angle);
        let sun = [a.sin() as f32, -(a.cos() as f32), 0.0];
        let b = f64::from(angle + std::f32::consts::PI);
        let moon_simple = [b.sin() as f32, -(b.cos() as f32), 0.0];
        let year = (time / 31558152.959999997).floor();
        let moon_phase = (time / 2551392.0).floor() + 0.75;
        let season = (((((time - year * time) / 86400.0) as f32) as f64 - 182.6282) * G3D_PI
            / 182.6282)
            .cos();
        let latitude =
            f64::from((self.properties.geographic_latitude * std::f32::consts::PI) / 180.0);
        let unit_z = [0.0f32, 0.0, 1.0];
        let axis = {
            let c = [
                unit_z[1] * sun[2] - unit_z[2] * sun[1],
                unit_z[2] * sun[0] - sun[2] * unit_z[0],
                sun[1] * unit_z[0] - unit_z[1] * sun[0],
            ];
            let inv = 1.0 / (c[2] * c[2] + c[1] * c[1] + c[0] * c[0]).sqrt();
            c.map(|v| v * inv)
        };
        let true_sun = rotate(axis, (-(season * self.sun_tilt) - latitude) as f32, sun);
        let m = ((moon_phase * G3D_PI + moon_phase * G3D_PI) as f32 + angle) as f64;
        let (ms, mc) = (m.sin() as f32, m.cos() as f32);
        let moon_vector = [ms, -mc, 0.0];
        let moon_axis = [
            mc * unit_z[2] + unit_z[1] * 0.0,
            unit_z[2] * ms - unit_z[0] * 0.0,
            -mc * unit_z[0] - ms * unit_z[1],
        ];
        let moon_angle =
            ((self.moon_tilt - self.sun_tilt) * (moon_phase * 4.0).sin() - latitude) as f32;
        let true_moon = rotate(moon_axis, moon_angle, moon_vector);
        let moon_is_source = true_sun[1] <= -0.3;
        SkyParameters {
            emissive: [1.0; 3],
            light_color: self.splines[0].eval(local),
            ambient: self.splines[1].eval(local),
            diffuse_ambient: self.splines[2].eval(local),
            sky_ambient: self.splines[3].eval(local),
            sky_horizon: self.splines[4].eval(local),
            light_direction: if moon_is_source { true_moon } else { true_sun },
            moon_is_source,
            true_sun,
            sun,
            true_moon,
            moon: moon_simple,
            moon_phase,
        }
    }

    /// presetLighting(lighting, black, 1.0) and setFog without quality culling.
    pub fn render_lighting(&self, sky: &SkyParameters) -> RenderLighting {
        let p = &self.properties;
        let brightness = if 0.05 < p.brightness {
            p.brightness.min(5.0)
        } else {
            0.05
        };
        let avg = |c: [f32; 3]| (c[0] + c[1] + c[2]) * 0.3333;
        let (top, bottom) = (avg(p.color_shift_top), avg(p.color_shift_bottom));
        let d = sky.light_direction;
        let inv = 1.0 / (d[2] * d[2] + d[1] * d[1] + d[0] * d[0]).sqrt();
        let l: [f32; 3] = std::array::from_fn(|i| sky.light_color[i] * 0.9 * brightness);
        let length = (l[2] * l[2] + l[1] * l[1] + l[0] * l[0]).sqrt();
        let l1 = l.map(|v| v * 0.4);
        let length1 = (l1[2] * l1[2] + l1[1] * l1[1] + l1[0] * l1[0]).sqrt();
        let lamp0 =
            std::array::from_fn(|i| ((p.color_shift_top[i] - top) * length + l[i]).min(1.0));
        let lamp1 = std::array::from_fn(|i| {
            ((p.color_shift_bottom[i] - bottom) * length1 + l1[i]).min(1.0)
        });
        let end = p.fog_end;
        let start = if 0.0 < p.fog_start {
            if end - 0.1 <= p.fog_start {
                end - 0.1
            } else {
                p.fog_start
            }
        } else {
            0.0
        };
        RenderLighting {
            ambient: p.ambient,
            lamp0,
            lamp0_direction: d.map(|v| -(v * inv)),
            lamp1,
            fog_color: p.fog_color,
            fog_params: [
                0.0,
                start,
                end,
                if start != end {
                    1.0 / (end - start)
                } else {
                    0.0
                },
            ],
        }
    }
}

fn statics_or(image: &NativeImage, function: u32, address: u32) -> Result<u32, String> {
    let (stores, _) = trace(image, function, 0x80)?;
    stores
        .get(&Key::Absolute(address))
        .copied()
        .ok_or_else(|| format!("static {address:08x} not recovered"))
}

/// Matrix3::fromAxisAngleFast(axis, angle) * v.
fn rotate(axis: [f32; 3], angle: f32, v: [f32; 3]) -> [f32; 3] {
    let (s, c) = (angle.sin(), angle.cos());
    let t = 1.0 - c;
    let [x, y, z] = axis;
    let m = [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    ];
    std::array::from_fn(|r| m[r][2] * v[2] + m[r][1] * v[1] + m[r][0] * v[0])
}

pub fn audit(bundle: &Bundle) -> Result<String, String> {
    use std::fmt::Write;
    let profile = LightingProfile::load(bundle)?;
    let mut report = format!("lighting_properties {:?}\n", profile.properties);
    for (name, spline) in [
        "light_color",
        "ambient",
        "diffuse_ambient",
        "sky_ambient",
        "sky_horizon",
    ]
    .iter()
    .zip(&profile.splines)
    {
        writeln!(
            report,
            "lighting_spline {name} times={:?} colors={:?}",
            spline.times, spline.colors
        )
        .unwrap();
    }
    writeln!(
        report,
        "lighting_tilts sun={} moon={}",
        profile.sun_tilt, profile.moon_tilt
    )
    .unwrap();
    let sky = profile.sky(profile.properties.time_of_day);
    let render = profile.render_lighting(&sky);
    writeln!(
        report,
        "lighting_sky time={} {sky:?}",
        profile.properties.time_of_day
    )
    .unwrap();
    writeln!(report, "lighting_render {render:?}").unwrap();
    for hour in [0.0, 6.0, 7.0, 12.0, 18.0, 21.0] {
        let s = profile.sky(hour * 3600.0);
        let r = profile.render_lighting(&s);
        writeln!(
            report,
            "lighting_hour hour={hour} sun={:?} moon_source={} lamp0={:?} lamp1={:?}",
            s.true_sun, s.moon_is_source, r.lamp0, r.lamp1
        )
        .unwrap();
    }
    let up = sky.true_sun[1];
    if profile.properties.time_of_day != 50400.0
        || render.lamp0.iter().any(|v| !(0.0..=1.0).contains(v))
        || up <= 0.0
        || sky.moon_is_source
    {
        return Err(format!("native lighting audit failed: {report}"));
    }
    writeln!(report, "lighting_pass time_of_day=14:00:00 sun_up={up} lamp0={:?} ambient={:?} fog_end={} light_grid=not_implemented quality_fog_cull=not_modeled native_pixels=not_tested", render.lamp0, render.ambient, render.fog_params[2]).unwrap();
    Ok(report)
}
