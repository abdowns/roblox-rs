//! Roblox model files (.rbxm/.rbxmx, binary and XML) as a generic instance
//! tree. Only the property types needed by the loaders are decoded; other
//! properties are skipped.
use crate::scene::CoordinateFrame;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Property {
    String(String),
    Bool(bool),
    Int(i32),
    Float(f32),
    Token(u32),
    CFrame(CoordinateFrame),
}

#[derive(Clone, Debug, Default)]
pub struct ModelInstance {
    pub class: String,
    pub properties: BTreeMap<String, Property>,
    pub children: Vec<usize>,
}

impl ModelInstance {
    pub fn name(&self) -> &str {
        match self.properties.get("Name") {
            Some(Property::String(s)) => s,
            _ => "",
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
            Property::Token(v) => Some(*v),
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
            parse_xml(std::str::from_utf8(bytes).map_err(|e| e.to_string())?)
        } else {
            Err("not a Roblox model".into())
        }
    }
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
    fn string(&mut self) -> Result<String, String> {
        let n = self.u32()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
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

/// Binary-format axis-aligned rotation id: id-1 = 6*right + up (NormalIds).
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
    if x.iter().zip(&y).any(|(a, b)| a * b != 0.0) {
        return Err("invalid CFrame rotation id".into());
    }
    let z = [
        x[1] * y[2] - x[2] * y[1],
        x[2] * y[0] - x[0] * y[2],
        x[0] * y[1] - x[1] * y[0],
    ];
    Ok(std::array::from_fn(|r| [x[r], y[r], z[r]]))
}

fn parse_binary(bytes: &[u8]) -> Result<Model, String> {
    let mut header = Reader { data: bytes, at: 0 };
    header.take(16)?;
    let _class_count = header.u32()?;
    let instance_count = header.u32()? as usize;
    header.take(8)?;
    let mut classes: BTreeMap<u32, (String, Vec<i32>)> = BTreeMap::new();
    let mut by_referent: BTreeMap<i32, usize> = BTreeMap::new();
    let mut instances = Vec::with_capacity(instance_count);
    let mut parents: Vec<(i32, i32)> = Vec::new();
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
                let _service = r.u8()?;
                let count = r.u32()? as usize;
                let referents = r.referents(count)?;
                for referent in &referents {
                    by_referent.insert(*referent, instances.len());
                    instances.push(ModelInstance {
                        class: class.clone(),
                        ..Default::default()
                    });
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
                            .map(|_| r.string().map(Property::String))
                            .collect::<Result<_, _>>()?,
                    ),
                    0x02 => Some(r.take(n)?.iter().map(|b| Property::Bool(*b != 0)).collect()),
                    0x03 => Some(r.ints(n)?.into_iter().map(Property::Int).collect()),
                    0x04 => Some(r.floats(n)?.into_iter().map(Property::Float).collect()),
                    0x12 => Some(r.interleaved(n)?.into_iter().map(Property::Token).collect()),
                    0x10 => {
                        let mut rotations = Vec::with_capacity(n);
                        for _ in 0..n {
                            let id = r.u8()?;
                            rotations.push(if id == 0 {
                                let f: Vec<f32> = r
                                    .take(36)?
                                    .chunks_exact(4)
                                    .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
                                    .collect();
                                std::array::from_fn(|row| {
                                    [f[row * 3], f[row * 3 + 1], f[row * 3 + 2]]
                                })
                            } else {
                                special_rotation(id)?
                            });
                        }
                        let x = r.floats(n)?;
                        let y = r.floats(n)?;
                        let z = r.floats(n)?;
                        Some(
                            rotations
                                .into_iter()
                                .enumerate()
                                .map(|(i, rotation)| {
                                    Property::CFrame(CoordinateFrame {
                                        position: [x[i], y[i], z[i]],
                                        rotation,
                                    })
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
                let _version = r.u8()?;
                let count = r.u32()? as usize;
                let children = r.referents(count)?;
                let parent = r.referents(count)?;
                parents.extend(children.into_iter().zip(parent));
            }
            b"END\0" => break,
            _ => {}
        }
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

fn parse_xml(text: &str) -> Result<Model, String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
    let mut model = Model::default();
    fn number<T: std::str::FromStr>(node: roxmltree::Node, tag: &str) -> Result<T, String> {
        node.children()
            .find(|n| n.has_tag_name(tag))
            .and_then(|n| n.text())
            .ok_or_else(|| format!("missing {tag}"))?
            .trim()
            .parse()
            .map_err(|_| format!("invalid {tag}"))
    }
    fn visit(node: roxmltree::Node, model: &mut Model) -> Result<usize, String> {
        let mut instance = ModelInstance {
            class: node.attribute("class").unwrap_or("").into(),
            ..Default::default()
        };
        if let Some(properties) = node.children().find(|n| n.has_tag_name("Properties")) {
            for p in properties.children().filter(|n| n.is_element()) {
                let Some(name) = p.attribute("name") else {
                    continue;
                };
                let text = p.text().unwrap_or("").trim();
                let value = match p.tag_name().name() {
                    "string" | "ProtectedString" => Property::String(p.text().unwrap_or("").into()),
                    "bool" => Property::Bool(text == "true"),
                    "int" => Property::Int(text.parse().map_err(|_| "invalid int")?),
                    "float" => Property::Float(text.parse().map_err(|_| "invalid float")?),
                    "token" => Property::Token(text.parse().map_err(|_| "invalid token")?),
                    "CoordinateFrame" => {
                        let r = |t| number::<f32>(p, t);
                        Property::CFrame(CoordinateFrame {
                            position: [r("X")?, r("Y")?, r("Z")?],
                            rotation: [
                                [r("R00")?, r("R01")?, r("R02")?],
                                [r("R10")?, r("R11")?, r("R12")?],
                                [r("R20")?, r("R21")?, r("R22")?],
                            ],
                        })
                    }
                    _ => continue,
                };
                instance.properties.insert(name.into(), value);
            }
        }
        let index = model.instances.len();
        model.instances.push(instance);
        for child in node.children().filter(|n| n.has_tag_name("Item")) {
            let c = visit(child, model)?;
            model.instances[index].children.push(c);
        }
        Ok(index)
    }
    for item in doc
        .root_element()
        .children()
        .filter(|n| n.has_tag_name("Item"))
    {
        let index = visit(item, &mut model)?;
        model.roots.push(index);
    }
    Ok(model)
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
    fn special_rotations_are_right_handed() {
        assert_eq!(
            special_rotation(0x02).unwrap(),
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        );
        assert_eq!(
            special_rotation(0x03).unwrap(),
            [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]]
        );
        assert!(special_rotation(0x01).is_err());
    }
}
