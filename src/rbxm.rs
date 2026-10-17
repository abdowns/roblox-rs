//! Roblox model and place files (.rbxm/.rbxl binary, .rbxmx XML) as a
//! generic instance tree.
//!
//! The binary format is SerializerBinary (the 0.255 build calls it
//! SerializerV2):
//! - the reader is 00e117bb (entry 00baf78a / 00bafe64);
//! - the writer is 00e10866, reached through 00e11780 and 00e1075f;
//! - the per-type value writer is 00e0cb3a;
//! - chunks go through 00e12a36 and are compressed with LZ4_compress
//!   00e07889 (00e07430 for inputs of at most 0x1000a bytes, 00e0703b for
//!   larger ones).
//!
//! `Model::to_binary` writes what the native writer writes for the same
//! tree. The XML format (TextXmlParser / TextXmlWriter) is in `rbxmx.rs`.
use crate::scene::CoordinateFrame;
use std::collections::BTreeMap;
use std::path::Path;

/// One property value. The binary type byte of each variant is in
/// `Property::type_id`; the 0.255 writer knows 0x01..0x19 except 0x11
/// (the quaternion CFrame, only written with stream flag 4).
#[derive(Clone, Debug, PartialEq)]
pub enum Property {
    /// 0x01: a string (valid UTF-8). The binary format has one string
    /// type, so Content and ProtectedString values read from a binary file
    /// are this variant too.
    String(String),
    Bool(bool),
    Int(i32),
    Float(f32),
    /// 0x12: an enum value (Token).
    Token(u32),
    CFrame(CoordinateFrame),
    /// 0x01 holding bytes that are not UTF-8 (BinaryString data such as
    /// MeshPart.PhysicsData).
    Bytes(Vec<u8>),
    Double(f64),
    /// 0x06: (scale, offset).
    UDim(f32, i32),
    /// 0x07: (scale x, scale y), (offset x, offset y).
    UDim2([f32; 2], [i32; 2]),
    /// 0x08: origin, direction.
    Ray([f32; 3], [f32; 3]),
    /// 0x09: NormalId bit mask.
    Faces(u8),
    /// 0x0a: X, Y, Z bit mask.
    Axes(u8),
    BrickColor(u32),
    Color3([f32; 3]),
    Vector2([f32; 2]),
    Vector3([f32; 3]),
    Vector2int16([i16; 2]),
    Vector3int16([i16; 3]),
    /// 0x13: an index into `Model::instances`, or None (a null reference).
    Ref(Option<usize>),
    /// 0x15: keypoints as stored (time, value, envelope).
    NumberSequence(Vec<[f32; 3]>),
    /// 0x16: keypoints as stored (time, r, g, b, envelope).
    ColorSequence(Vec<[f32; 5]>),
    /// 0x17: min, max.
    NumberRange([f32; 2]),
    /// 0x18: min x, min y, max x, max y.
    Rect2D([f32; 4]),
    /// 0x19: None for the material default, or the custom values as stored
    /// (density, friction, elasticity, friction weight, elasticity weight).
    PhysicalProperties(Option<[f32; 5]>),
    /// 0x01 in binary; the XML `Content` tag (an asset URL, empty for
    /// `<null>`).
    Content(String),
    /// 0x01 in binary; the XML `ProtectedString` tag (script Source).
    ProtectedString(String),
    /// 0x01 in binary; the XML `BinaryString` tag (base64 in XML).
    BinaryString(Vec<u8>),
}

impl Property {
    /// The binary format's type byte (00e0cb3a).
    pub fn type_id(&self) -> u8 {
        match self {
            Self::String(_)
            | Self::Bytes(_)
            | Self::Content(_)
            | Self::ProtectedString(_)
            | Self::BinaryString(_) => 0x01,
            Self::Bool(_) => 0x02,
            Self::Int(_) => 0x03,
            Self::Float(_) => 0x04,
            Self::Double(_) => 0x05,
            Self::UDim(..) => 0x06,
            Self::UDim2(..) => 0x07,
            Self::Ray(..) => 0x08,
            Self::Faces(_) => 0x09,
            Self::Axes(_) => 0x0a,
            Self::BrickColor(_) => 0x0b,
            Self::Color3(_) => 0x0c,
            Self::Vector2(_) => 0x0d,
            Self::Vector3(_) => 0x0e,
            Self::Vector2int16(_) => 0x0f,
            Self::CFrame(_) => 0x10,
            Self::Token(_) => 0x12,
            Self::Ref(_) => 0x13,
            Self::Vector3int16(_) => 0x14,
            Self::NumberSequence(_) => 0x15,
            Self::ColorSequence(_) => 0x16,
            Self::NumberRange(_) => 0x17,
            Self::Rect2D(_) => 0x18,
            Self::PhysicalProperties(_) => 0x19,
        }
    }
    /// The raw bytes of any of the string variants.
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::String(s) | Self::Content(s) | Self::ProtectedString(s) => Some(s.as_bytes()),
            Self::Bytes(b) | Self::BinaryString(b) => Some(b),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ModelInstance {
    pub class: String,
    pub properties: BTreeMap<String, Property>,
    pub children: Vec<usize>,
    /// The XML `referent` the instance was read with. The native loader
    /// keeps it on the instance (00bb0cbe) and the XML writer reuses it
    /// (00bb5ea8), so a read and a write keep the referents.
    pub referent: Option<String>,
}

impl ModelInstance {
    pub fn new(class: &str) -> Self {
        Self {
            class: class.into(),
            ..Default::default()
        }
    }
    pub fn name(&self) -> &str {
        self.string("Name").unwrap_or("")
    }
    /// A String, Content or ProtectedString property.
    pub fn string(&self, name: &str) -> Option<&str> {
        match self.properties.get(name)? {
            Property::String(s) | Property::Content(s) | Property::ProtectedString(s) => Some(s),
            _ => None,
        }
    }
    pub fn float(&self, name: &str) -> Option<f32> {
        match self.properties.get(name)? {
            Property::Float(v) => Some(*v),
            _ => None,
        }
    }
    pub fn bool(&self, name: &str) -> Option<bool> {
        match self.properties.get(name)? {
            Property::Bool(v) => Some(*v),
            _ => None,
        }
    }
    pub fn token(&self, name: &str) -> Option<u32> {
        match self.properties.get(name)? {
            Property::Token(v) | Property::BrickColor(v) => Some(*v),
            Property::Int(v) => u32::try_from(*v).ok(),
            _ => None,
        }
    }
    pub fn cframe(&self, name: &str) -> Option<&CoordinateFrame> {
        match self.properties.get(name)? {
            Property::CFrame(v) => Some(v),
            _ => None,
        }
    }
    pub fn vector3(&self, name: &str) -> Option<[f32; 3]> {
        match self.properties.get(name)? {
            Property::Vector3(v) => Some(*v),
            _ => None,
        }
    }
    /// A Ref property: Some(None) for a null reference.
    pub fn reference(&self, name: &str) -> Option<Option<usize>> {
        match self.properties.get(name)? {
            Property::Ref(v) => Some(*v),
            _ => None,
        }
    }
    pub fn set(&mut self, name: &str, value: Property) -> &mut Self {
        self.properties.insert(name.into(), value);
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub instances: Vec<ModelInstance>,
    /// Top-level instances in file order.
    pub roots: Vec<usize>,
}

impl Model {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.starts_with(b"<roblox!") {
            parse_binary(bytes)
        } else if bytes.starts_with(b"<roblox") {
            crate::rbxmx::parse(bytes)
        } else {
            Err("not a Roblox model".into())
        }
    }
    pub fn read_file(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }
    /// Writes the XML format (`to_xml`) for a `.rbxmx` or `.rbxlx` path,
    /// as Studio's save dialog does, and the binary format (`to_binary`)
    /// for any other.
    pub fn write_file(&self, path: &Path) -> Result<(), String> {
        let xml = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("rbxmx") || e.eq_ignore_ascii_case("rbxlx"));
        let bytes = if xml { self.to_xml()? } else { self.to_binary()? };
        std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
    }
    /// The XML format as the native TextXmlWriter writes it (`rbxmx.rs`).
    pub fn to_xml(&self) -> Result<Vec<u8>, String> {
        crate::rbxmx::write(self)
    }
    /// Appends an instance under `parent` (a root when None) and returns
    /// its index.
    pub fn add(&mut self, instance: ModelInstance, parent: Option<usize>) -> usize {
        let index = self.instances.len();
        self.instances.push(instance);
        match parent {
            Some(p) => self.instances[p].children.push(index),
            None => self.roots.push(index),
        }
        index
    }
    /// A readable listing: one line per instance (indented by depth, with
    /// its class and Name), then one line per property in name order. Ref
    /// values print as the target's path of Names.
    pub fn dump(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        let mut parent = vec![None; self.instances.len()];
        for (i, instance) in self.instances.iter().enumerate() {
            for &c in &instance.children {
                parent[c] = Some(i);
            }
        }
        let path = |mut i: usize| {
            let mut names = vec![self.instances[i].name().to_string()];
            while let Some(p) = parent[i] {
                names.push(self.instances[p].name().to_string());
                i = p;
            }
            names.reverse();
            names.join(".")
        };
        let mut stack: Vec<(usize, usize)> = self.roots.iter().rev().map(|r| (*r, 0)).collect();
        while let Some((i, depth)) = stack.pop() {
            let instance = &self.instances[i];
            let pad = "  ".repeat(depth);
            writeln!(out, "{pad}{} {:?}", instance.class, instance.name()).unwrap();
            for (name, value) in &instance.properties {
                let text = match value {
                    Property::Ref(Some(target)) => format!("Ref({})", path(*target)),
                    other => format!("{other:?}"),
                };
                writeln!(out, "{pad}  .{name} = {text}").unwrap();
            }
            stack.extend(instance.children.iter().rev().map(|c| (*c, depth + 1)));
        }
        out
    }
    /// The instance index and its parent for every instance under the
    /// roots, in pre-order (the native writer's referent order).
    fn pre_order(&self) -> Result<Vec<(usize, Option<usize>)>, String> {
        let mut out = Vec::with_capacity(self.instances.len());
        let mut seen = vec![false; self.instances.len()];
        fn visit(
            model: &Model,
            index: usize,
            parent: Option<usize>,
            out: &mut Vec<(usize, Option<usize>)>,
            seen: &mut [bool],
        ) -> Result<(), String> {
            let flag = seen.get_mut(index).ok_or("child index out of range")?;
            if *flag {
                return Err(format!("instance {index} has two parents"));
            }
            *flag = true;
            out.push((index, parent));
            for &child in &model.instances[index].children {
                visit(model, child, Some(index), out, seen)?;
            }
            Ok(())
        }
        for &root in &self.roots {
            visit(self, root, None, &mut out, &mut seen)?;
        }
        Ok(out)
    }
    /// SerializerBinary output (writer 00e10866 with stream flags 0, the
    /// value of every Studio save path):
    /// - a 32-byte header: "<roblox!", 89 ff 0d 0a 1a 0a, version 0, the class
    ///   count, the instance count and 8 zero bytes;
    /// - an INST chunk per class, classes in Name order (Name::sortIndex is the
    ///   alphabetical rank, Name.cpp:116-121);
    /// - a PROP chunk per class and property, properties in Name order;
    /// - one PRNT chunk, then END.
    ///
    /// Referents are the pre-order positions from the roots (00e1293e); PRNT
    /// lists instances in post-order with -1 for the roots. Every chunk but
    /// END is LZ4-compressed; END holds "</roblox>" uncompressed (flag 2).
    /// A class's instances must all carry the same property names and types,
    /// as reflection gives them.
    pub fn to_binary(&self) -> Result<Vec<u8>, String> {
        let order = self.pre_order()?;
        let mut referent = vec![-1i32; self.instances.len()];
        for (id, (index, _)) in order.iter().enumerate() {
            referent[*index] = id as i32;
        }
        let mut classes: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (index, _) in &order {
            classes
                .entry(self.instances[*index].class.as_str())
                .or_default()
                .push(*index);
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"<roblox!\x89\xff\r\n\x1a\n\0\0");
        out.extend_from_slice(&(classes.len() as u32).to_le_bytes());
        out.extend_from_slice(&(order.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0; 8]);
        for (type_index, (class, members)) in classes.iter().enumerate() {
            let mut w = Writer::default();
            w.u32(type_index as u32);
            w.string(class.as_bytes());
            // Format 1 (Service instances, with a root flag each) is only
            // used for the services of a place.
            w.u8(0);
            w.u32(members.len() as u32);
            w.referents(&members.iter().map(|i| referent[*i]).collect::<Vec<_>>());
            chunk(&mut out, b"INST", &w.0, true);
        }
        for (type_index, (class, members)) in classes.iter().enumerate() {
            let first = &self.instances[members[0]];
            for member in &members[1..] {
                let other = &self.instances[*member];
                let same = other.properties.len() == first.properties.len()
                    && other
                        .properties
                        .iter()
                        .zip(&first.properties)
                        .all(|((a, x), (b, y))| a == b && x.type_id() == y.type_id());
                if !same {
                    return Err(format!(
                        "{class} instances {:?} and {:?} differ in properties",
                        first.name(),
                        other.name()
                    ));
                }
            }
            for name in first.properties.keys() {
                let values: Vec<&Property> = members
                    .iter()
                    .map(|i| &self.instances[*i].properties[name])
                    .collect();
                let mut w = Writer::default();
                w.u32(type_index as u32);
                w.string(name.as_bytes());
                w.values(&values, &referent)?;
                chunk(&mut out, b"PROP", &w.0, true);
            }
        }
        // Post-order: children before their parents (00e1293e appends to
        // its second list after the recursion).
        let mut post = Vec::with_capacity(order.len());
        fn visit(
            model: &Model,
            index: usize,
            parent: Option<usize>,
            post: &mut Vec<(usize, Option<usize>)>,
        ) {
            for &child in &model.instances[index].children {
                visit(model, child, Some(index), post);
            }
            post.push((index, parent));
        }
        for &root in &self.roots {
            visit(self, root, None, &mut post);
        }
        let mut w = Writer::default();
        w.u8(0);
        w.u32(post.len() as u32);
        w.referents(&post.iter().map(|(i, _)| referent[*i]).collect::<Vec<_>>());
        w.referents(
            &post
                .iter()
                .map(|(_, p)| p.map_or(-1, |p| referent[p]))
                .collect::<Vec<_>>(),
        );
        chunk(&mut out, b"PRNT", &w.0, true);
        chunk(&mut out, b"END\0", b"</roblox>", false);
        Ok(out)
    }
}

/// Chunk writer 00e12a36: name, compressed size (0 when stored), size, 0,
/// then the payload.
fn chunk(out: &mut Vec<u8>, name: &[u8; 4], payload: &[u8], compress: bool) {
    out.extend_from_slice(name);
    if compress {
        let packed = lz4_compress(payload);
        out.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&packed);
    } else {
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(payload);
    }
}

#[derive(Default)]
struct Writer(Vec<u8>);
impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn string(&mut self, s: &[u8]) {
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s);
    }
    /// Byte-interleaved big-endian words.
    fn interleaved(&mut self, words: &[u32]) {
        for byte in 0..4 {
            for w in words {
                self.0.push(w.to_be_bytes()[byte]);
            }
        }
    }
    /// 00e103c9: zigzag-encoded, interleaved.
    fn ints(&mut self, v: &[i32]) {
        self.interleaved(
            &v.iter()
                .map(|x| ((x << 1) ^ (x >> 31)) as u32)
                .collect::<Vec<_>>(),
        );
    }
    /// 00e104d9: the sign bit moved to bit 0, interleaved.
    fn floats(&mut self, v: &[f32]) {
        self.interleaved(
            &v.iter()
                .map(|x| x.to_bits().rotate_left(1))
                .collect::<Vec<_>>(),
        );
    }
    /// 00e106bb: differences from the previous referent, as ints.
    fn referents(&mut self, v: &[i32]) {
        let mut last = 0i32;
        let deltas: Vec<i32> = v
            .iter()
            .map(|r| {
                let d = r.wrapping_sub(last);
                last = *r;
                d
            })
            .collect();
        self.ints(&deltas);
    }
    /// 00e0cb3a: the type byte, then every instance's value.
    fn values(&mut self, values: &[&Property], referent: &[i32]) -> Result<(), String> {
        macro_rules! each {
            ($variant:ident) => {
                values.iter().map(|v| match v {
                    Property::$variant(x) => x,
                    _ => unreachable!(),
                })
            };
        }
        let first = values[0];
        self.u8(first.type_id());
        match first {
            Property::String(_)
            | Property::Bytes(_)
            | Property::Content(_)
            | Property::ProtectedString(_)
            | Property::BinaryString(_) => {
                for v in values {
                    self.string(v.bytes().expect("type 0x01"));
                }
            }
            Property::Bool(_) => each!(Bool).for_each(|b| self.u8(u8::from(*b))),
            Property::Int(_) => self.ints(&each!(Int).copied().collect::<Vec<_>>()),
            Property::Float(_) => self.floats(&each!(Float).copied().collect::<Vec<_>>()),
            Property::Double(_) => {
                each!(Double).for_each(|d| self.0.extend_from_slice(&d.to_le_bytes()))
            }
            Property::UDim(..) => {
                let (s, o): (Vec<f32>, Vec<i32>) = values
                    .iter()
                    .map(|v| match v {
                        Property::UDim(s, o) => (*s, *o),
                        _ => unreachable!(),
                    })
                    .unzip();
                self.floats(&s);
                self.ints(&o);
            }
            Property::UDim2(..) => {
                let parts: Vec<([f32; 2], [i32; 2])> = values
                    .iter()
                    .map(|v| match v {
                        Property::UDim2(s, o) => (*s, *o),
                        _ => unreachable!(),
                    })
                    .collect();
                for a in 0..2 {
                    self.floats(&parts.iter().map(|p| p.0[a]).collect::<Vec<_>>());
                }
                for a in 0..2 {
                    self.ints(&parts.iter().map(|p| p.1[a]).collect::<Vec<_>>());
                }
            }
            Property::Ray(..) => {
                for v in values {
                    let Property::Ray(o, d) = v else {
                        unreachable!()
                    };
                    o.iter().chain(d).for_each(|x| self.f32(*x));
                }
            }
            Property::Faces(_) => each!(Faces).for_each(|b| self.u8(*b)),
            Property::Axes(_) => each!(Axes).for_each(|b| self.u8(*b)),
            Property::BrickColor(_) => {
                self.interleaved(&each!(BrickColor).copied().collect::<Vec<_>>())
            }
            Property::Token(_) => self.interleaved(&each!(Token).copied().collect::<Vec<_>>()),
            Property::Color3(_) => self.components(each!(Color3).map(|c| c.to_vec())),
            Property::Vector2(_) => self.components(each!(Vector2).map(|c| c.to_vec())),
            Property::Vector3(_) => self.components(each!(Vector3).map(|c| c.to_vec())),
            Property::Rect2D(_) => self.components(each!(Rect2D).map(|c| c.to_vec())),
            Property::Vector2int16(_) => each!(Vector2int16)
                .flatten()
                .for_each(|x| self.0.extend_from_slice(&x.to_le_bytes())),
            Property::Vector3int16(_) => each!(Vector3int16)
                .flatten()
                .for_each(|x| self.0.extend_from_slice(&x.to_le_bytes())),
            Property::CFrame(_) => {
                let frames: Vec<&CoordinateFrame> = each!(CFrame).collect();
                for f in &frames {
                    match rotation_id(&f.rotation) {
                        Some(id) => self.u8(id),
                        None => {
                            self.u8(0);
                            f.rotation.iter().flatten().for_each(|x| self.f32(*x));
                        }
                    }
                }
                for a in 0..3 {
                    self.floats(&frames.iter().map(|f| f.position[a]).collect::<Vec<_>>());
                }
            }
            Property::Ref(_) => {
                let ids: Vec<i32> = each!(Ref)
                    .map(|r| match r {
                        Some(i) => referent.get(*i).copied().filter(|r| *r >= 0).ok_or(()),
                        None => Ok(-1),
                    })
                    .collect::<Result<_, _>>()
                    .map_err(|_| "Ref to an instance outside the model".to_string())?;
                self.referents(&ids);
            }
            Property::NumberSequence(_) => {
                for keys in each!(NumberSequence) {
                    self.u32(keys.len() as u32);
                    keys.iter().flatten().for_each(|x| self.f32(*x));
                }
            }
            Property::ColorSequence(_) => {
                for keys in each!(ColorSequence) {
                    self.u32(keys.len() as u32);
                    keys.iter().flatten().for_each(|x| self.f32(*x));
                }
            }
            Property::NumberRange(_) => each!(NumberRange).flatten().for_each(|x| self.f32(*x)),
            Property::PhysicalProperties(_) => {
                for v in each!(PhysicalProperties) {
                    self.u8(u8::from(v.is_some()));
                    if let Some(v) = v {
                        v.iter().for_each(|x| self.f32(*x));
                    }
                }
            }
        }
        Ok(())
    }
    /// One interleaved float array per component.
    fn components(&mut self, values: impl Iterator<Item = Vec<f32>>) {
        let values: Vec<Vec<f32>> = values.collect();
        for a in 0..values[0].len() {
            self.floats(&values.iter().map(|v| v[a]).collect::<Vec<_>>());
        }
    }
}

/// Binary rotation id (00e0cb3a): 0 unless every entry is exactly 0 or +-1
/// with one +-1 per row and column (006b10dc), else
/// 6 * NormalId(column 0) + NormalId(column 1) + 1 (006b11a7).
fn rotation_id(r: &[[f32; 3]; 3]) -> Option<u8> {
    let mut rows = [0; 3];
    let mut columns = [0; 3];
    for (i, row) in r.iter().enumerate() {
        for (j, v) in row.iter().enumerate() {
            if *v == 1.0 || *v == -1.0 {
                rows[i] += 1;
                columns[j] += 1;
            } else if *v != 0.0 {
                return None;
            }
        }
    }
    if rows != [1; 3] || columns != [1; 3] {
        return None;
    }
    let normal = |c: usize| -> u8 {
        let axis = (0..3).find(|i| r[*i][c] != 0.0).unwrap() as u8;
        if r[axis as usize][c] > 0.0 {
            axis
        } else {
            axis + 3
        }
    };
    Some(6 * normal(0) + normal(1) + 1)
}

/// LZ4_compress as built into 0.255 (00e07889): the 64K variant 00e07430
/// (13-bit hash of u16 offsets, an empty slot meaning offset 0) for inputs
/// of at most 0x1000a bytes, else 00e0703b (12-bit hash of positions, an
/// empty slot never matching). Both use the r9x-era greedy parse: skip
/// strength 6, MFLIMIT 12, LASTLITERALS 5, and a match must lie within
/// 0xffff bytes.
fn lz4_compress(src: &[u8]) -> Vec<u8> {
    let n = src.len();
    let mut out = Vec::with_capacity(n + n / 255 + 16);
    let mut anchor = 0usize;
    let read32 = |p: usize| u32::from_le_bytes(src[p..p + 4].try_into().unwrap());
    let read16 = |p: usize| u16::from_le_bytes(src[p..p + 2].try_into().unwrap());
    if n > 12 {
        let small = n <= 0x1000a;
        let shift = if small { 19 } else { 20 };
        let hash = |p: usize| (read32(p).wrapping_mul(2654435761) >> shift) as usize;
        let mut table: Vec<Option<usize>> = vec![small.then_some(0); 1 << (32 - shift)];
        let mflimit = n - 12;
        let reachable = |candidate: Option<usize>, ip: usize| {
            candidate.filter(|c| c + 0xffff >= ip && read32(*c) == read32(ip))
        };
        table[hash(0)] = Some(0);
        let mut ip = 1;
        let mut forward = 2;
        'main: while forward <= mflimit {
            // Find a match.
            let mut attempts = 68u32;
            let mut forward_hash = hash(ip);
            let mut reference = loop {
                let h = forward_hash;
                forward_hash = hash(forward);
                let candidate = table[h];
                table[h] = Some(ip);
                if let Some(r) = reachable(candidate, ip) {
                    break r;
                }
                let next = forward + (attempts >> 6) as usize;
                attempts += 1;
                ip = forward;
                forward = next;
                if forward > mflimit {
                    break 'main;
                }
            };
            // Catch up.
            while reference > 0 && ip > anchor && src[ip - 1] == src[reference - 1] {
                ip -= 1;
                reference -= 1;
            }
            // Literal run.
            let literals = ip - anchor;
            let mut token = out.len();
            out.push(0);
            if literals < 15 {
                out[token] = (literals as u8) << 4;
            } else {
                out[token] = 0xf0;
                let mut rest = literals - 15;
                while rest > 254 {
                    out.push(255);
                    rest -= 255;
                }
                out.push(rest as u8);
            }
            out.extend_from_slice(&src[anchor..ip]);
            loop {
                out.extend_from_slice(&((ip - reference) as u16).to_le_bytes());
                ip += 4;
                reference += 4;
                let start = ip;
                let mut counted = false;
                while ip < n - 8 {
                    let diff = read32(reference) ^ read32(ip);
                    if diff == 0 {
                        ip += 4;
                        reference += 4;
                    } else {
                        ip += (diff.trailing_zeros() >> 3) as usize;
                        counted = true;
                        break;
                    }
                }
                if !counted {
                    if ip < n - 6 && read16(reference) == read16(ip) {
                        ip += 2;
                        reference += 2;
                    }
                    if ip < n - 5 && src[reference] == src[ip] {
                        ip += 1;
                    }
                }
                let mut length = ip - start;
                if length < 15 {
                    out[token] += length as u8;
                } else {
                    out[token] += 15;
                    length -= 15;
                    while length > 509 {
                        out.extend_from_slice(&[255, 255]);
                        length -= 510;
                    }
                    if length > 254 {
                        out.push(255);
                        length -= 255;
                    }
                    out.push(length as u8);
                }
                anchor = ip;
                if ip > mflimit {
                    break 'main;
                }
                table[hash(ip - 2)] = Some(ip - 2);
                let h = hash(ip);
                let candidate = table[h];
                table[h] = Some(ip);
                match reachable(candidate, ip) {
                    Some(r) => {
                        reference = r;
                        token = out.len();
                        out.push(0);
                    }
                    None => break,
                }
            }
            ip += 1;
            forward = ip + 1;
        }
    }
    let last = n - anchor;
    if last < 15 {
        out.push((last as u8) << 4);
    } else {
        out.push(0xf0);
        let mut rest = last - 15;
        while rest > 254 {
            out.push(255);
            rest -= 255;
        }
        out.push(rest as u8);
    }
    out.extend_from_slice(&src[anchor..]);
    out
}

/// LZ4 block format.
fn lz4(source: &[u8], size: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(size);
    let mut i = 0;
    let byte = |i: usize| source.get(i).copied().ok_or("truncated LZ4 block");
    while i < source.len() {
        let token = byte(i)?;
        i += 1;
        let mut literal = usize::from(token >> 4);
        if literal == 15 {
            loop {
                let b = byte(i)?;
                i += 1;
                literal += usize::from(b);
                if b != 255 {
                    break;
                }
            }
        }
        out.extend_from_slice(source.get(i..i + literal).ok_or("truncated LZ4 literal")?);
        i += literal;
        if i >= source.len() {
            break;
        }
        let offset = usize::from(byte(i)?) | (usize::from(byte(i + 1)?) << 8);
        i += 2;
        let mut length = usize::from(token & 15);
        if length == 15 {
            loop {
                let b = byte(i)?;
                i += 1;
                length += usize::from(b);
                if b != 255 {
                    break;
                }
            }
        }
        length += 4;
        if offset == 0 || offset > out.len() {
            return Err("invalid LZ4 offset".into());
        }
        for _ in 0..length {
            out.push(out[out.len() - offset]);
        }
        if out.len() > size {
            return Err("LZ4 block overflow".into());
        }
    }
    if out.len() != size {
        return Err("LZ4 size mismatch".into());
    }
    Ok(out)
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self
            .data
            .get(self.at..self.at + n)
            .ok_or("truncated binary model")?;
        self.at += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn f32s<const N: usize>(&mut self) -> Result<[f32; N], String> {
        let mut v = [0.0; N];
        for x in &mut v {
            *x = self.f32()?;
        }
        Ok(v)
    }
    fn i16(&mut self) -> Result<i16, String> {
        Ok(i16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn bytes(&mut self) -> Result<Vec<u8>, String> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    fn string(&mut self) -> Result<String, String> {
        Ok(String::from_utf8_lossy(&self.bytes()?).into_owned())
    }
    /// Byte-interleaved big-endian 32-bit words.
    fn interleaved(&mut self, n: usize) -> Result<Vec<u32>, String> {
        let b = self.take(n * 4)?;
        Ok((0..n)
            .map(|i| u32::from_be_bytes([b[i], b[n + i], b[2 * n + i], b[3 * n + i]]))
            .collect())
    }
    fn ints(&mut self, n: usize) -> Result<Vec<i32>, String> {
        Ok(self
            .interleaved(n)?
            .into_iter()
            .map(|v| ((v >> 1) as i32) ^ -((v & 1) as i32))
            .collect())
    }
    fn floats(&mut self, n: usize) -> Result<Vec<f32>, String> {
        Ok(self
            .interleaved(n)?
            .into_iter()
            .map(|v| f32::from_bits(v.rotate_right(1)))
            .collect())
    }
    /// `count` interleaved float arrays, transposed to one entry per value.
    fn components<const N: usize>(&mut self, n: usize) -> Result<Vec<[f32; N]>, String> {
        let arrays: Vec<Vec<f32>> = (0..N).map(|_| self.floats(n)).collect::<Result<_, _>>()?;
        Ok((0..n)
            .map(|i| std::array::from_fn(|a| arrays[a][i]))
            .collect())
    }
    fn referents(&mut self, n: usize) -> Result<Vec<i32>, String> {
        let mut acc = 0i32;
        Ok(self
            .ints(n)?
            .into_iter()
            .map(|d| {
                acc = acc.wrapping_add(d);
                acc
            })
            .collect())
    }
}

/// Binary-format axis-aligned rotation id (006b11ff): id-1 = 6*right + up
/// (NormalIds), and column 2 is right x up with the native products, so the
/// signs of zero match. Like the native reader it does not check that the two
/// axes are perpendicular.
fn special_rotation(id: u8) -> Result<[[f32; 3]; 3], String> {
    let direction = |n: u8| -> Result<[f32; 3], String> {
        Ok(match n {
            0 => [1.0, 0.0, 0.0],
            1 => [0.0, 1.0, 0.0],
            2 => [0.0, 0.0, 1.0],
            3 => [-1.0, 0.0, 0.0],
            4 => [0.0, -1.0, 0.0],
            5 => [0.0, 0.0, -1.0],
            _ => return Err("invalid CFrame rotation id".into()),
        })
    };
    let index = id.checked_sub(1).ok_or("invalid CFrame rotation id")?;
    let x = direction(index / 6)?;
    let y = direction(index % 6)?;
    let z = [
        x[1] * y[2] - x[2] * y[1],
        x[2] * y[0] - x[0] * y[2],
        x[0] * y[1] - x[1] * y[0],
    ];
    Ok(std::array::from_fn(|r| [x[r], y[r], z[r]]))
}

/// Reader 00e117bb: header check ("<roblox!", the 6 signature bytes and
/// version 0), then chunks until END; unknown chunks are skipped.
fn parse_binary(bytes: &[u8]) -> Result<Model, String> {
    let mut header = Reader { data: bytes, at: 0 };
    if header.take(14)? != b"<roblox!\x89\xff\r\n\x1a\n" {
        return Err("The file header is corrupted".into());
    }
    let version = u16::from_le_bytes(header.take(2)?.try_into().unwrap());
    if version != 0 {
        return Err(format!("Unrecognized version {version}"));
    }
    let _class_count = header.u32()?;
    let instance_count = header.u32()? as usize;
    header.take(8)?;
    let mut classes: BTreeMap<u32, (String, Vec<i32>)> = BTreeMap::new();
    let mut by_referent: BTreeMap<i32, usize> = BTreeMap::new();
    let mut instances = Vec::with_capacity(instance_count);
    let mut parents: Vec<(i32, i32)> = Vec::new();
    let mut refs: Vec<(usize, String, i32)> = Vec::new();
    loop {
        let name = header.take(4)?.to_vec();
        let compressed = header.u32()? as usize;
        let size = header.u32()? as usize;
        header.take(4)?;
        let payload = if compressed == 0 {
            header.take(size)?.to_vec()
        } else {
            lz4(header.take(compressed)?, size)?
        };
        let mut r = Reader {
            data: &payload,
            at: 0,
        };
        match &name[..] {
            b"INST" => {
                let id = r.u32()?;
                let class = r.string()?;
                let format = r.u8()?;
                if format > 1 {
                    return Err(format!("Unrecognized object format {format}"));
                }
                let count = r.u32()? as usize;
                let referents = r.referents(count)?;
                for referent in &referents {
                    by_referent.insert(*referent, instances.len());
                    instances.push(ModelInstance::new(&class));
                }
                classes.insert(id, (class, referents));
            }
            b"PROP" => {
                let id = r.u32()?;
                let property = r.string()?;
                let kind = r.u8()?;
                let referents = &classes.get(&id).ok_or("PROP before INST")?.1;
                let n = referents.len();
                let values: Option<Vec<Property>> = match kind {
                    0x01 => Some(
                        (0..n)
                            .map(|_| {
                                r.bytes().map(|b| match String::from_utf8(b) {
                                    Ok(s) => Property::String(s),
                                    Err(e) => Property::Bytes(e.into_bytes()),
                                })
                            })
                            .collect::<Result<_, _>>()?,
                    ),
                    0x02 => Some(r.take(n)?.iter().map(|b| Property::Bool(*b != 0)).collect()),
                    0x03 => Some(r.ints(n)?.into_iter().map(Property::Int).collect()),
                    0x04 => Some(r.floats(n)?.into_iter().map(Property::Float).collect()),
                    0x05 => Some(
                        (0..n)
                            .map(|_| {
                                r.take(8).map(|b| {
                                    Property::Double(f64::from_le_bytes(b.try_into().unwrap()))
                                })
                            })
                            .collect::<Result<_, _>>()?,
                    ),
                    0x06 => {
                        let s = r.floats(n)?;
                        let o = r.ints(n)?;
                        Some((0..n).map(|i| Property::UDim(s[i], o[i])).collect())
                    }
                    0x07 => {
                        let s = r.components::<2>(n)?;
                        let ox = r.ints(n)?;
                        let oy = r.ints(n)?;
                        Some(
                            (0..n)
                                .map(|i| Property::UDim2(s[i], [ox[i], oy[i]]))
                                .collect(),
                        )
                    }
                    0x08 => Some(
                        (0..n)
                            .map(|_| Ok(Property::Ray(r.f32s()?, r.f32s()?)))
                            .collect::<Result<_, String>>()?,
                    ),
                    0x09 => Some(r.take(n)?.iter().map(|b| Property::Faces(*b)).collect()),
                    0x0a => Some(r.take(n)?.iter().map(|b| Property::Axes(*b)).collect()),
                    0x0b => Some(
                        r.interleaved(n)?
                            .into_iter()
                            .map(Property::BrickColor)
                            .collect(),
                    ),
                    0x0c => Some(
                        r.components::<3>(n)?
                            .into_iter()
                            .map(Property::Color3)
                            .collect(),
                    ),
                    0x0d => Some(
                        r.components::<2>(n)?
                            .into_iter()
                            .map(Property::Vector2)
                            .collect(),
                    ),
                    0x0e => Some(
                        r.components::<3>(n)?
                            .into_iter()
                            .map(Property::Vector3)
                            .collect(),
                    ),
                    0x0f => Some(
                        (0..n)
                            .map(|_| Ok(Property::Vector2int16([r.i16()?, r.i16()?])))
                            .collect::<Result<_, String>>()?,
                    ),
                    0x12 => Some(r.interleaved(n)?.into_iter().map(Property::Token).collect()),
                    0x13 => {
                        for (referent, target) in referents.iter().zip(r.referents(n)?) {
                            refs.push((by_referent[referent], property.clone(), target));
                        }
                        None
                    }
                    0x14 => Some(
                        (0..n)
                            .map(|_| Ok(Property::Vector3int16([r.i16()?, r.i16()?, r.i16()?])))
                            .collect::<Result<_, String>>()?,
                    ),
                    0x15 => Some(
                        (0..n)
                            .map(|_| {
                                let k = r.u32()? as usize;
                                Ok(Property::NumberSequence(
                                    (0..k).map(|_| r.f32s()).collect::<Result<_, _>>()?,
                                ))
                            })
                            .collect::<Result<_, String>>()?,
                    ),
                    0x16 => Some(
                        (0..n)
                            .map(|_| {
                                let k = r.u32()? as usize;
                                Ok(Property::ColorSequence(
                                    (0..k).map(|_| r.f32s()).collect::<Result<_, _>>()?,
                                ))
                            })
                            .collect::<Result<_, String>>()?,
                    ),
                    0x17 => Some(
                        (0..n)
                            .map(|_| Ok(Property::NumberRange(r.f32s()?)))
                            .collect::<Result<_, String>>()?,
                    ),
                    0x18 => Some(
                        r.components::<4>(n)?
                            .into_iter()
                            .map(Property::Rect2D)
                            .collect(),
                    ),
                    0x19 => Some(
                        (0..n)
                            .map(|_| {
                                Ok(Property::PhysicalProperties(if r.u8()? != 0 {
                                    Some(r.f32s()?)
                                } else {
                                    None
                                }))
                            })
                            .collect::<Result<_, String>>()?,
                    ),
                    0x10 => {
                        let mut rotations = Vec::with_capacity(n);
                        for _ in 0..n {
                            let id = r.u8()?;
                            rotations.push(if id == 0 {
                                let f: [f32; 9] = r.f32s()?;
                                std::array::from_fn(|row| {
                                    [f[row * 3], f[row * 3 + 1], f[row * 3 + 2]]
                                })
                            } else {
                                special_rotation(id)?
                            });
                        }
                        let p = r.components::<3>(n)?;
                        Some(
                            rotations
                                .into_iter()
                                .zip(p)
                                .map(|(rotation, position)| {
                                    Property::CFrame(CoordinateFrame { position, rotation })
                                })
                                .collect(),
                        )
                    }
                    _ => None,
                };
                if let Some(values) = values {
                    for (referent, value) in referents.iter().zip(values) {
                        let index = by_referent[referent];
                        instances[index].properties.insert(property.clone(), value);
                    }
                }
            }
            b"PRNT" => {
                let version = r.u8()?;
                if version != 0 {
                    return Err(format!("Unrecognized parent link format {version}"));
                }
                let count = r.u32()? as usize;
                let children = r.referents(count)?;
                let parent = r.referents(count)?;
                parents.extend(children.into_iter().zip(parent));
            }
            b"END\0" => break,
            _ => {}
        }
    }
    for (index, property, target) in refs {
        let value = by_referent.get(&target).copied();
        if value.is_none() && target != -1 {
            return Err(format!("{property} refers to unknown referent {target}"));
        }
        instances[index]
            .properties
            .insert(property, Property::Ref(value));
    }
    let mut roots = Vec::new();
    for (child, parent) in parents {
        let child = *by_referent.get(&child).ok_or("unknown PRNT child")?;
        match by_referent.get(&parent) {
            Some(parent) => instances[*parent].children.push(child),
            None if parent == -1 => roots.push(child),
            None => return Err("unknown PRNT parent".into()),
        }
    }
    Ok(Model { instances, roots })
}

/// The binary models shipped in the install that the native SerializerBinary
/// wrote (the other `.rbxm` files there are XML).
pub const NATIVE_BINARY_MODELS: [&str; 2] = [
    "Contents/Resources/content/fonts/characterR15_2.rbxm",
    "Contents/Resources/content/fonts/humanoidAnimateR15_2.rbxm",
];

/// Parses each native binary model and writes it again with `to_binary`;
/// the bytes must be identical.
pub fn audit(bundle: &crate::bundle::Bundle) -> Result<String, String> {
    use std::fmt::Write;
    let mut report = String::new();
    for path in NATIVE_BINARY_MODELS {
        let bytes = bundle.read(path, 16 * 1024 * 1024)?;
        let model = Model::parse(&bytes)?;
        let written = model.to_binary()?;
        let mut classes: BTreeMap<&str, usize> = BTreeMap::new();
        for i in &model.instances {
            *classes.entry(i.class.as_str()).or_default() += 1;
        }
        let first_difference = written.iter().zip(&bytes).position(|(a, b)| a != b);
        writeln!(
            report,
            "rbxm_rewrite file={path} bytes={} written={} identical={} first_difference={first_difference:?} instances={} classes={classes:?}",
            bytes.len(),
            written.len(),
            written == bytes,
            model.instances.len()
        )
        .unwrap();
        if written != bytes {
            return Err(format!("{report}rewrite of {path} differs"));
        }
    }
    let mut identical = 0;
    for (line, same) in crate::rbxmx::rewrite_report(bundle)? {
        writeln!(report, "{line}").unwrap();
        identical += usize::from(same);
    }
    writeln!(
        report,
        "rbxm_pass files={} xml_files={} xml_identical={identical}",
        NATIVE_BINARY_MODELS.len(),
        crate::rbxmx::NATIVE_XML_MODELS.len()
    )
    .unwrap();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lz4_literal_and_match() {
        // "abcabcabc": 3 literals then a match of 6 at offset 3.
        let block = [0x32, b'a', b'b', b'c', 3, 0];
        assert_eq!(lz4(&block, 9).unwrap(), b"abcabcabc");
    }
    #[test]
    fn lz4_compress_round_trips() {
        let mut data = Vec::new();
        for i in 0..5000u32 {
            data.extend_from_slice(&(i % 97).to_le_bytes());
            data.push((i * 7 % 13) as u8);
        }
        for len in [0, 1, 12, 13, 64, 300, data.len()] {
            let packed = lz4_compress(&data[..len]);
            assert_eq!(lz4(&packed, len).unwrap(), &data[..len], "length {len}");
        }
        // The large-input variant.
        let big: Vec<u8> = (0..70000u32)
            .map(|i| (i.wrapping_mul(i) % 251) as u8)
            .collect();
        assert_eq!(lz4(&lz4_compress(&big), big.len()).unwrap(), big);
    }
    #[test]
    fn special_rotations_are_right_handed() {
        assert_eq!(
            special_rotation(0x02).unwrap(),
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        );
        assert_eq!(
            special_rotation(0x03).unwrap(),
            [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]]
        );
        // Right, Right: degenerate, as the native reader builds it.
        assert_eq!(
            special_rotation(0x01).unwrap(),
            [[1.0, 1.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]]
        );
        assert!(special_rotation(0).is_err());
        assert!(special_rotation(37).is_err());
    }
    #[test]
    fn rotation_ids_invert_special_rotations() {
        for id in 1..=36u8 {
            let r = special_rotation(id).unwrap();
            let perpendicular = (id - 1) / 6 % 3 != (id - 1) % 6 % 3;
            assert_eq!(rotation_id(&r), perpendicular.then_some(id));
        }
        let tilted = [[0.8, -0.6, 0.0], [0.6, 0.8, 0.0], [0.0, 0.0, 1.0]];
        assert_eq!(rotation_id(&tilted), None);
    }
    #[test]
    fn every_type_round_trips() {
        let mut model = Model::default();
        let frame = |p: [f32; 3]| CoordinateFrame {
            position: p,
            rotation: [[0.8, -0.6, 0.0], [0.6, 0.8, 0.0], [0.0, 0.0, 1.0]],
        };
        let values = |k: f32| -> Vec<(&str, Property)> {
            vec![
                ("A", Property::String(format!("s{k}"))),
                ("B", Property::Bytes(vec![0xff, k as u8])),
                ("C", Property::Bool(k > 1.0)),
                ("D", Property::Int(-(k as i32) * 1000)),
                ("E", Property::Float(-k * 0.5)),
                ("F", Property::Double(k as f64 * 1e10)),
                ("G", Property::UDim(k, -3)),
                ("H", Property::UDim2([k, 0.25], [7, -k as i32])),
                ("I", Property::Ray([k, 1.0, 2.0], [0.0, -k, 0.5])),
                ("J", Property::Faces(0b100101)),
                ("K", Property::Axes(0b101)),
                ("L", Property::BrickColor(194 + k as u32)),
                ("M", Property::Color3([k / 4.0, 0.5, 1.0])),
                ("N", Property::Vector2([k, -k])),
                ("O", Property::Vector3([k, -0.0, 1e30])),
                ("P", Property::Vector2int16([-k as i16, 300])),
                ("Q", Property::CFrame(frame([k, 2.0, 3.0]))),
                ("R", Property::Token(k as u32)),
                ("S", Property::Ref(if k > 1.0 { Some(0) } else { None })),
                ("T", Property::Vector3int16([1, -2, k as i16])),
                (
                    "U",
                    Property::NumberSequence(vec![[0.0, k, 0.0], [1.0, 2.0, 0.1]]),
                ),
                ("V", Property::ColorSequence(vec![[0.0, k, 0.5, 0.25, 0.0]])),
                ("W", Property::NumberRange([k, k + 1.0])),
                ("X", Property::Rect2D([k, 1.0, 2.0, 3.0])),
                (
                    "Y",
                    Property::PhysicalProperties((k > 1.0).then_some([0.7, 0.3, 0.5, 1.0, 1.0])),
                ),
            ]
        };
        let mut make = |k: f32, parent| {
            let mut i = ModelInstance::new("Thing");
            for (name, value) in values(k) {
                i.set(name, value);
            }
            model.add(i, parent)
        };
        let root = make(1.0, None);
        make(2.0, Some(root));
        let bytes = model.to_binary().unwrap();
        let back = Model::parse(&bytes).unwrap();
        assert_eq!(back.roots, vec![0]);
        assert_eq!(back.instances[0].children, vec![1]);
        for (a, b) in model.instances.iter().zip(&back.instances) {
            assert_eq!(a.properties, b.properties);
        }
        assert_eq!(back.to_binary().unwrap(), bytes);
    }
    /// The two binary models shipped in the install were written by the
    /// native SerializerBinary; writing their parsed trees again must give
    /// the same bytes.
    #[test]
    fn native_binary_models_rewrite_byte_identical() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("RobloxStudio/Contents/Resources/content/fonts");
        for name in ["characterR15_2.rbxm", "humanoidAnimateR15_2.rbxm"] {
            let Ok(bytes) = std::fs::read(root.join(name)) else {
                eprintln!("skipped: {name} not installed");
                continue;
            };
            let model = Model::parse(&bytes).unwrap();
            assert_eq!(model.to_binary().unwrap(), bytes, "{name}");
        }
    }
}
