//! XML model and place files (.rbxmx, .rbxlx, and the .rbxm files that are
//! XML) as the 0.255 build reads and writes them.
//!
//! Reading follows TextXmlParser (00bb92c0) and SerializerV2::loadXML:
//! - tags are read up to '>', attribute values between quotes;
//! - an element's text is a CDATA section (00bb8068 skips anything after it
//!   up to the next '<') or the characters up to the next '<' with the
//!   entities decoded (00bb7a56): `&lt; &gt; &amp; &quot; &apos;`, `&nbsp;`
//!   as a space, `&#N;` as the byte N (atoi, truncated to a byte), `&#x`
//!   is an error and any other `&name;` is kept as written;
//! - the root must be `roblox` with a `version` of at least 4.
//!
//! Writing follows the XmlElement tree the serializer builds and
//! TextXmlWriter (00bba086):
//! - an element is its tabs, `<tag`, ` name="value"` per attribute, `>`,
//!   its value, then `</tag>` straight after when it has no children, or
//!   each child on its own line one tab deeper and `</tag>` on its own line;
//! - a text value with a newline and no `]]>` is a CDATA section (00bb8640),
//!   any other is escaped byte by byte (00bb872c): `" & ' < >` as entities,
//!   newline, carriage return and 0x20..0x7e as they are, every other byte
//!   as `&#N;`;
//! - numbers are snprintf'd: int `%d`, token and Color3 `%u`, float `%.9g`,
//!   double `%.20g`, with `INF`, `-INF` and `NAN` (006dd3f2..006dd762);
//! - the number lists of NumberSequence, ColorSequence and NumberRange are
//!   ostream doubles (precision 6, `%g`), each followed by a space.
//!
//! Each Item gets the referent it was read with, or a new one. Natively a
//! new referent is "RBX" and a random GUID's 32 hex digits (0067202c); this
//! port derives new ones from a counter so saves are reproducible.
use crate::rbxm::{Model, ModelInstance, Property};
use crate::scene::CoordinateFrame;
use std::collections::{BTreeMap, HashMap, HashSet};

/// One parsed element.
#[derive(Debug, Default)]
struct Element {
    tag: Vec<u8>,
    attributes: Vec<(Vec<u8>, Vec<u8>)>,
    text: Vec<u8>,
    children: Vec<Element>,
}

impl Element {
    fn attribute(&self, name: &str) -> Option<&[u8]> {
        self.attributes
            .iter()
            .find(|(n, _)| n == name.as_bytes())
            .map(|(_, v)| v.as_slice())
    }
    fn child(&self, tag: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.tag == tag.as_bytes())
    }
    fn text_str(&self) -> &str {
        std::str::from_utf8(&self.text).unwrap_or("").trim()
    }
}

/// 00bb7a56.
fn decode_entities(raw: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let c = raw[i];
        i += 1;
        if c != b'&' {
            out.push(c);
            continue;
        }
        let start = i;
        while i < raw.len() && raw[i] != b';' {
            i += 1;
        }
        let name = &raw[start..i];
        i += 1;
        match name {
            b"lt" => out.push(b'<'),
            b"gt" => out.push(b'>'),
            b"amp" => out.push(b'&'),
            b"quot" => out.push(b'"'),
            b"apos" => out.push(b'\''),
            b"nbsp" => out.push(b' '),
            [b'#', rest @ ..] => {
                if rest.is_empty() {
                    return Err("bad XML. No character code following #".into());
                }
                if rest[0] == b'x' {
                    return Err("bad XML. Hex character codes are not supported".into());
                }
                out.push(atoi(rest) as u8);
            }
            other => {
                out.push(b'&');
                out.extend_from_slice(other);
                out.push(b';');
            }
        }
    }
    Ok(out)
}

/// C atoi: optional whitespace and sign, then the leading digits.
fn atoi(s: &[u8]) -> i32 {
    let mut i = 0;
    while i < s.len() && s[i].is_ascii_whitespace() {
        i += 1;
    }
    let negative = i < s.len() && s[i] == b'-';
    if i < s.len() && (s[i] == b'-' || s[i] == b'+') {
        i += 1;
    }
    let mut v: i32 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        v = v.wrapping_mul(10).wrapping_add(i32::from(s[i] - b'0'));
        i += 1;
    }
    if negative { v.wrapping_neg() } else { v }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn starts(&self, p: &[u8]) -> bool {
        self.s[self.i..].starts_with(p)
    }
    fn find(&self, p: &[u8]) -> Option<usize> {
        self.s[self.i..]
            .windows(p.len())
            .position(|w| w == p)
            .map(|k| self.i + k)
    }
    /// Skips to the next '<', and past declarations and comments.
    fn next_tag(&mut self) -> Result<(), String> {
        loop {
            while self.i < self.s.len() && self.s[self.i] != b'<' {
                self.i += 1;
            }
            if self.i >= self.s.len() {
                return Err("Expected '<' but got EOF in Xml stream".into());
            }
            let close = if self.starts(b"<?") {
                b"?>".as_slice()
            } else if self.starts(b"<!--") {
                b"-->".as_slice()
            } else {
                return Ok(());
            };
            let end = self.find(close).ok_or("unterminated XML declaration")?;
            self.i = end + close.len();
        }
    }
    fn element(&mut self) -> Result<Element, String> {
        self.next_tag()?;
        self.i += 1;
        let mut e = Element::default();
        let name_end = self.s[self.i..]
            .iter()
            .position(|c| c.is_ascii_whitespace() || *c == b'>' || *c == b'/')
            .ok_or("Expected '>' but got EOF in Xml stream")?;
        e.tag = self.s[self.i..self.i + name_end].to_vec();
        self.i += name_end;
        loop {
            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.starts(b"/>") {
                self.i += 2;
                return Ok(e);
            }
            if self.starts(b">") {
                self.i += 1;
                break;
            }
            let eq = self.find(b"=").ok_or("Unable to parse XML attributes. '=' not found")?;
            let name = self.s[self.i..eq].trim_ascii().to_vec();
            let open = self.s[eq..]
                .iter()
                .position(|c| *c == b'"')
                .map(|k| eq + k)
                .ok_or("Unable to parse XML attributes. '\"' not found")?;
            let close = self.s[open + 1..]
                .iter()
                .position(|c| *c == b'"')
                .map(|k| open + 1 + k)
                .ok_or("Unable to parse XML attributes. '\"' not found")?;
            e.attributes
                .push((name, decode_entities(&self.s[open + 1..close])?));
            self.i = close + 1;
        }
        if self.starts(b"<![CDATA[") {
            self.i += 9;
            let end = self.find(b"]]>").ok_or("unterminated CDATA")?;
            e.text = self.s[self.i..end].to_vec();
            self.i = end + 3;
        } else {
            let end = self.s[self.i..]
                .iter()
                .position(|c| *c == b'<')
                .map_or(self.s.len(), |k| self.i + k);
            e.text = decode_entities(&self.s[self.i..end])?;
            self.i = end;
        }
        loop {
            self.next_tag()?;
            if self.starts(b"</") {
                let end = self.find(b">").ok_or("Expected '>' but got EOF in Xml stream")?;
                self.i = end + 1;
                return Ok(e);
            }
            e.children.push(self.element()?);
        }
    }
}

fn float(text: &str) -> Option<f32> {
    match text {
        "INF" => Some(f32::INFINITY),
        "-INF" => Some(f32::NEG_INFINITY),
        "NAN" => Some(f32::NAN),
        t => t.parse().ok(),
    }
}

fn double(text: &str) -> Option<f64> {
    match text {
        "INF" => Some(f64::INFINITY),
        "-INF" => Some(f64::NEG_INFINITY),
        "NAN" => Some(f64::NAN),
        t => t.parse().ok(),
    }
}

fn int(text: &str) -> Option<i32> {
    text.parse::<i32>()
        .ok()
        .or_else(|| text.parse::<i64>().ok().map(|v| v as i32))
}

fn child_float(e: &Element, tag: &str) -> Option<f32> {
    float(e.child(tag)?.text_str())
}

fn child_int(e: &Element, tag: &str) -> Option<i32> {
    int(e.child(tag)?.text_str())
}

fn vector3(e: &Element) -> Option<[f32; 3]> {
    Some([
        child_float(e, "X")?,
        child_float(e, "Y")?,
        child_float(e, "Z")?,
    ])
}

fn vector2(e: &Element) -> Option<[f32; 2]> {
    Some([child_float(e, "X")?, child_float(e, "Y")?])
}

fn string(bytes: &[u8]) -> Property {
    match String::from_utf8(bytes.to_vec()) {
        Ok(s) => Property::String(s),
        Err(e) => Property::Bytes(e.into_bytes()),
    }
}

/// Floats separated by whitespace (the sequence and range forms).
fn floats(text: &str) -> Vec<f32> {
    text.split_ascii_whitespace().filter_map(float).collect()
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 00ac088c: standard base64 with '=' padding and a newline after every 18
/// complete groups.
fn base64_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 4 / 3 + 4);
    let mut groups = 0;
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(BASE64[usize::from(b0 >> 2)]);
        out.push(BASE64[usize::from((b0 & 3) << 4 | b1 >> 4)]);
        match chunk.len() {
            1 => out.extend_from_slice(b"=="),
            2 => {
                out.push(BASE64[usize::from((b1 & 0xf) << 2)]);
                out.push(b'=');
            }
            _ => {
                out.push(BASE64[usize::from((b1 & 0xf) << 2 | b2 >> 6)]);
                out.push(BASE64[usize::from(b2 & 0x3f)]);
                if groups == 17 {
                    out.push(b'\n');
                    groups = 0;
                } else {
                    groups += 1;
                }
            }
        }
    }
    out
}

fn base64_decode(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for &c in text {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => continue,
        };
        acc = acc << 6 | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// 00ab7e9e: the custom values with the reader's defaults and clamps.
fn physical_properties(e: &Element) -> Property {
    if e.child("CustomPhysics").map(Element::text_str) != Some("true") {
        return Property::PhysicalProperties(None);
    }
    let get = |tag, default| child_float(e, tag).unwrap_or(default);
    let density = get("Density", 1.0);
    let friction = get("Friction", 0.0);
    let elasticity = get("Elasticity", 0.0);
    let friction_weight = get("FrictionWeight", 1.0);
    let elasticity_weight = get("ElasticityWeight", 1.0);
    let clamp = |v: f32, max: f32| {
        if 0.0 < v {
            if max <= v { max } else { v }
        } else {
            0.0
        }
    };
    let density = if 0.01 < density {
        if 100.0 <= density { 100.0 } else { density }
    } else {
        0.01
    };
    let elasticity_weight = if 0.0 < elasticity_weight {
        if elasticity_weight <= 100.0 {
            elasticity_weight
        } else {
            100.0
        }
    } else {
        0.0
    };
    Property::PhysicalProperties(Some([
        density,
        clamp(friction, 2.0),
        clamp(elasticity, 1.0),
        clamp(friction_weight, 100.0),
        elasticity_weight,
    ]))
}

/// One property element's value, or None for a tag this port does not
/// read (and for a value the native reader would leave unset).
fn property(e: &Element) -> Option<Property> {
    let text = e.text_str();
    Some(match e.tag.as_slice() {
        b"bool" => Property::Bool(text == "true"),
        b"int" => Property::Int(int(text)?),
        b"float" => Property::Float(float(text)?),
        b"double" => Property::Double(double(text)?),
        b"token" => Property::Token(text.parse().ok()?),
        b"string" => string(&e.text),
        b"ProtectedString" => match string(&e.text) {
            Property::String(s) => Property::ProtectedString(s),
            bytes => bytes,
        },
        b"BinaryString" => Property::BinaryString(base64_decode(&e.text)),
        b"Content" => {
            let url = e.child("url").map_or(Vec::new(), |u| u.text.clone());
            match String::from_utf8(url) {
                Ok(s) => Property::Content(s),
                Err(e) => Property::Bytes(e.into_bytes()),
            }
        }
        b"CoordinateFrame" => {
            let r = |t| child_float(e, t);
            Property::CFrame(CoordinateFrame {
                position: [r("X")?, r("Y")?, r("Z")?],
                rotation: [
                    [r("R00")?, r("R01")?, r("R02")?],
                    [r("R10")?, r("R11")?, r("R12")?],
                    [r("R20")?, r("R21")?, r("R22")?],
                ],
            })
        }
        b"Vector3" => Property::Vector3(vector3(e)?),
        b"Vector2" => Property::Vector2(vector2(e)?),
        b"Vector3int16" => Property::Vector3int16([
            child_int(e, "X")? as i16,
            child_int(e, "Y")? as i16,
            child_int(e, "Z")? as i16,
        ]),
        b"Vector2int16" => {
            Property::Vector2int16([child_int(e, "X")? as i16, child_int(e, "Y")? as i16])
        }
        // 00ab8ef2: a packed 0xAARRGGBB, or R, G and B elements.
        b"Color3" => match text.parse::<u32>() {
            Ok(v) => Property::Color3(
                [v >> 16, v >> 8, v].map(|c| f32::from((c & 0xff) as u8) / 255.0),
            ),
            Err(_) => Property::Color3([
                child_float(e, "R")?,
                child_float(e, "G")?,
                child_float(e, "B")?,
            ]),
        },
        b"UDim2" => Property::UDim2(
            [child_float(e, "XS")?, child_float(e, "YS")?],
            [child_int(e, "XO")?, child_int(e, "YO")?],
        ),
        b"UDim" => Property::UDim(child_float(e, "S")?, child_int(e, "O")?),
        b"Ray" => Property::Ray(vector3(e.child("origin")?)?, vector3(e.child("direction")?)?),
        b"Faces" => Property::Faces(child_int(e, "faces")? as u8),
        b"Axes" => Property::Axes(child_int(e, "axes")? as u8),
        b"NumberSequence" => Property::NumberSequence(
            floats(text)
                .chunks_exact(3)
                .map(|k| [k[0], k[1], k[2]])
                .collect(),
        ),
        b"ColorSequence" => Property::ColorSequence(
            floats(text)
                .chunks_exact(5)
                .map(|k| [k[0], k[1], k[2], k[3], k[4]])
                .collect(),
        ),
        b"NumberRange" => {
            let v = floats(text);
            Property::NumberRange([*v.first()?, *v.get(1)?])
        }
        b"Rect2D" => {
            let min = vector2(e.child("min")?)?;
            let max = vector2(e.child("max")?)?;
            Property::Rect2D([min[0], min[1], max[0], max[1]])
        }
        b"PhysicalProperties" => physical_properties(e),
        // Resolved once every Item is read.
        b"Ref" => Property::Ref(None),
        _ => return None,
    })
}

/// Parses an XML model or place.
pub fn parse(bytes: &[u8]) -> Result<Model, String> {
    let mut parser = Parser { s: bytes, i: 0 };
    let root = parser.element()?;
    if root.tag != b"roblox" {
        return Err("SerializerV2::loadXML ill-formed XML. No Roblox tag".into());
    }
    let version = root
        .attribute("version")
        .ok_or("SerializerV2::loadXML no version number")?;
    if atoi(version) < 4 {
        return Err("SerializerV2::loadXML schemaVersionLoading<4".into());
    }
    let mut model = Model::default();
    let mut refs: Vec<(usize, String, Vec<u8>)> = Vec::new();
    fn item(
        e: &Element,
        model: &mut Model,
        refs: &mut Vec<(usize, String, Vec<u8>)>,
        parent: Option<usize>,
    ) {
        let mut instance = ModelInstance::new(
            &String::from_utf8_lossy(e.attribute("class").unwrap_or_default()),
        );
        instance.referent = e
            .attribute("referent")
            .map(|r| String::from_utf8_lossy(r).into_owned());
        let index = model.add(instance, parent);
        for properties in e.children.iter().filter(|c| c.tag == b"Properties") {
            for p in &properties.children {
                let Some(name) = p.attribute("name") else {
                    continue;
                };
                let name = String::from_utf8_lossy(name).into_owned();
                let Some(value) = property(p) else {
                    continue;
                };
                if p.tag == b"Ref" {
                    refs.push((index, name.clone(), p.text.trim_ascii().to_vec()));
                }
                model.instances[index].properties.insert(name, value);
            }
        }
        for child in e.children.iter().filter(|c| c.tag == b"Item") {
            item(child, model, refs, Some(index));
        }
    }
    for e in root.children.iter().filter(|c| c.tag == b"Item") {
        item(e, &mut model, &mut refs, None);
    }
    let mut by_referent: HashMap<&str, usize> = HashMap::new();
    for (i, instance) in model.instances.iter().enumerate() {
        if let Some(r) = &instance.referent {
            by_referent.entry(r.as_str()).or_insert(i);
        }
    }
    let resolved: Vec<_> = refs
        .into_iter()
        .map(|(index, name, target)| {
            let target = std::str::from_utf8(&target)
                .ok()
                .and_then(|t| by_referent.get(t).copied());
            (index, name, target)
        })
        .collect();
    for (index, name, target) in resolved {
        model.instances[index]
            .properties
            .insert(name, Property::Ref(target));
    }
    Ok(model)
}

/// An element to write: its value (00bb5ea8's value types reduced to what
/// the text writer does with them) and children.
struct Node {
    tag: &'static str,
    attributes: Vec<(&'static str, Vec<u8>)>,
    value: Value,
    children: Vec<Node>,
}

enum Value {
    None,
    Text(Vec<u8>),
    /// Value type 3: `<url>` or `<null></null>`.
    Content(Vec<u8>),
}

impl Node {
    fn new(tag: &'static str) -> Self {
        Self {
            tag,
            attributes: Vec::new(),
            value: Value::None,
            children: Vec::new(),
        }
    }
    fn text(tag: &'static str, text: impl Into<Vec<u8>>) -> Self {
        let mut n = Self::new(tag);
        n.value = Value::Text(text.into());
        n
    }
}

/// C printf `%.<precision>g`.
fn c_g(v: f64, precision: usize) -> String {
    if v == 0.0 {
        return if v.is_sign_negative() { "-0" } else { "0" }.into();
    }
    fn strip(s: &str) -> &str {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.')
        } else {
            s
        }
    }
    let p = precision.max(1);
    let e = format!("{:.*e}", p - 1, v);
    let (mantissa, exponent) = e.split_once('e').expect("exponent");
    let x: i32 = exponent.parse().expect("exponent");
    if x < -4 || x >= p as i32 {
        let sign = if x < 0 { '-' } else { '+' };
        format!("{}e{sign}{:02}", strip(mantissa), x.abs())
    } else {
        strip(&format!("{:.*}", (p as i32 - 1 - x) as usize, v)).into()
    }
}

/// 006dd8f4.
fn format_float(v: f32) -> String {
    if v.is_nan() {
        "NAN".into()
    } else if v == f32::INFINITY {
        "INF".into()
    } else if v == f32::NEG_INFINITY {
        "-INF".into()
    } else {
        c_g(f64::from(v), 9)
    }
}

/// 006dd762.
fn format_double(v: f64) -> String {
    if v.is_nan() {
        "NAN".into()
    } else if v == f64::INFINITY {
        "INF".into()
    } else if v == f64::NEG_INFINITY {
        "-INF".into()
    } else {
        c_g(v, 20)
    }
}

/// ostream << double with the default flags (libstdc++: `%.6g`).
fn format_stream(v: f32) -> String {
    let v = f64::from(v);
    if v.is_nan() {
        "nan".into()
    } else if v.is_infinite() {
        if v > 0.0 { "inf" } else { "-inf" }.into()
    } else {
        c_g(v, 6)
    }
}

fn float_node(tag: &'static str, v: f32) -> Node {
    Node::text(tag, format_float(v))
}

fn int_node(tag: &'static str, v: i32) -> Node {
    Node::text(tag, v.to_string())
}

fn vector_nodes(v: &[f32]) -> Vec<Node> {
    ["X", "Y", "Z"]
        .iter()
        .zip(v)
        .map(|(t, v)| float_node(t, *v))
        .collect()
}

/// G3D::Color3uint8(const Color3&) (0124ff24): floorf(c * 256) truncated,
/// at most 255, the low byte otherwise (no lower clamp).
fn color_byte(c: f32) -> u32 {
    let v = (c * 256.0).floor();
    // cvttss2si gives 0x80000000 for NaN and out-of-range values.
    let i = if v.is_nan() || !(-2147483648.0..2147483648.0).contains(&v) {
        i32::MIN
    } else {
        v as i32
    };
    if i > 255 { 255 } else { (i as u32) & 0xff }
}

/// The element of one property (0056260e and the descriptors' writers).
fn property_node(name: &str, value: &Property, referent: &dyn Fn(usize) -> String) -> Node {
    let (tag, mut node) = match value {
        Property::String(s) => ("string", Node::text("", s.as_bytes())),
        Property::Bytes(b) => ("string", Node::text("", b.clone())),
        Property::ProtectedString(s) => ("ProtectedString", Node::text("", s.as_bytes())),
        Property::Content(s) => {
            let mut n = Node::new("");
            n.value = Value::Content(s.as_bytes().to_vec());
            ("Content", n)
        }
        Property::BinaryString(b) => ("BinaryString", Node::text("", base64_encode(b))),
        Property::Bool(b) => ("bool", Node::text("", if *b { "true" } else { "false" })),
        Property::Int(v) => ("int", Node::text("", v.to_string())),
        Property::BrickColor(v) => ("int", Node::text("", (*v as i32).to_string())),
        Property::Float(v) => ("float", Node::text("", format_float(*v))),
        Property::Double(v) => ("double", Node::text("", format_double(*v))),
        Property::Token(v) => ("token", Node::text("", v.to_string())),
        Property::CFrame(c) => {
            // 0098b48a: X, Y, Z, then R00..R22.
            let mut n = Node::new("");
            n.children = vector_nodes(&c.position);
            for (row, r) in c.rotation.iter().enumerate() {
                for (col, v) in r.iter().enumerate() {
                    const TAGS: [[&str; 3]; 3] = [
                        ["R00", "R01", "R02"],
                        ["R10", "R11", "R12"],
                        ["R20", "R21", "R22"],
                    ];
                    n.children.push(float_node(TAGS[row][col], *v));
                }
            }
            ("CoordinateFrame", n)
        }
        Property::Vector3(v) => {
            let mut n = Node::new("");
            n.children = vector_nodes(v);
            ("Vector3", n)
        }
        Property::Vector2(v) => {
            let mut n = Node::new("");
            n.children = vector_nodes(v);
            ("Vector2", n)
        }
        // 00ab7cc6 (Vector3int16) and 00ab7a00 (Vector2int16), sign
        // extended.
        Property::Vector3int16(v) => {
            let mut n = Node::new("");
            n.children = vec![
                int_node("X", i32::from(v[0])),
                int_node("Y", i32::from(v[1])),
                int_node("Z", i32::from(v[2])),
            ];
            ("Vector3int16", n)
        }
        Property::Vector2int16(v) => {
            let mut n = Node::new("");
            n.children = vec![int_node("X", i32::from(v[0])), int_node("Y", i32::from(v[1]))];
            ("Vector2int16", n)
        }
        // 00ab900e: 0xFF000000 | r << 16 | g << 8 | b, as %u.
        Property::Color3(c) => {
            let packed =
                0xff00_0000 | color_byte(c[0]) << 16 | color_byte(c[1]) << 8 | color_byte(c[2]);
            ("Color3", Node::text("", packed.to_string()))
        }
        // 00ab88f2 (the reader is 00ab87d6): XS, XO, YS, YO.
        Property::UDim2(scale, offset) => {
            let mut n = Node::new("");
            n.children = vec![
                float_node("XS", scale[0]),
                int_node("XO", offset[0]),
                float_node("YS", scale[1]),
                int_node("YO", offset[1]),
            ];
            ("UDim2", n)
        }
        // No 0.255 class has a UDim property and no writer uses the S and O
        // atoms; this layout follows them and is not native output.
        Property::UDim(scale, offset) => {
            let mut n = Node::new("");
            n.children = vec![float_node("S", *scale), int_node("O", *offset)];
            ("UDim", n)
        }
        // 00ab92d8: origin and direction, each X, Y, Z.
        Property::Ray(origin, direction) => {
            let mut o = Node::new("origin");
            o.children = vector_nodes(origin);
            let mut d = Node::new("direction");
            d.children = vector_nodes(direction);
            let mut n = Node::new("");
            n.children = vec![o, d];
            ("Ray", n)
        }
        // 00ab8b74.
        Property::Faces(v) => {
            let mut n = Node::new("");
            n.children = vec![int_node("faces", i32::from(*v))];
            ("Faces", n)
        }
        Property::Axes(v) => {
            let mut n = Node::new("");
            n.children = vec![int_node("axes", i32::from(*v))];
            ("Axes", n)
        }
        // 00c5ce95 and 007e79a5.
        Property::NumberSequence(keys) => {
            let text: String = keys.iter().flatten().map(|v| format_stream(*v) + " ").collect();
            ("NumberSequence", Node::text("", text))
        }
        Property::ColorSequence(keys) => {
            let text: String = keys.iter().flatten().map(|v| format_stream(*v) + " ").collect();
            ("ColorSequence", Node::text("", text))
        }
        // 0087248a.
        Property::NumberRange(r) => {
            let text: String = r.iter().map(|v| format_stream(*v) + " ").collect();
            ("NumberRange", Node::text("", text))
        }
        // 00ab84fe: min and max, each X, Y.
        Property::Rect2D(r) => {
            let mut min = Node::new("min");
            min.children = vector_nodes(&r[0..2]);
            let mut max = Node::new("max");
            max.children = vector_nodes(&r[2..4]);
            let mut n = Node::new("");
            n.children = vec![min, max];
            ("Rect2D", n)
        }
        // 00ab813c.
        Property::PhysicalProperties(custom) => {
            let mut n = Node::new("");
            n.children.push(Node::text(
                "CustomPhysics",
                if custom.is_some() { "true" } else { "false" },
            ));
            if let Some([density, friction, elasticity, friction_weight, elasticity_weight]) =
                custom
            {
                n.children.extend([
                    float_node("Density", *density),
                    float_node("Friction", *friction),
                    float_node("Elasticity", *elasticity),
                    float_node("FrictionWeight", *friction_weight),
                    float_node("ElasticityWeight", *elasticity_weight),
                ]);
            }
            ("PhysicalProperties", n)
        }
        Property::Ref(target) => (
            "Ref",
            Node::text("", target.map_or_else(|| "null".into(), referent)),
        ),
    };
    node.tag = tag;
    node.attributes.push(("name", name.as_bytes().to_vec()));
    node
}

/// 00bb872c.
fn escape(out: &mut Vec<u8>, text: &[u8]) {
    for &c in text {
        match c {
            b'"' => out.extend_from_slice(b"&quot;"),
            b'&' => out.extend_from_slice(b"&amp;"),
            b'\'' => out.extend_from_slice(b"&apos;"),
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            b'\n' | b'\r' | 0x20..=0x7e => out.push(c),
            _ => out.extend_from_slice(format!("&#{c};").as_bytes()),
        }
    }
}

/// 00bba086 with 00bb8884 (open), 00bba128 (value) and 00bb8a60 (close).
fn write_node(out: &mut Vec<u8>, node: &Node, depth: usize) {
    out.extend(std::iter::repeat_n(b'\t', depth));
    out.push(b'<');
    out.extend_from_slice(node.tag.as_bytes());
    for (name, value) in &node.attributes {
        out.push(b' ');
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(b"=\"");
        escape(out, value);
        out.push(b'"');
    }
    out.push(b'>');
    match &node.value {
        Value::None => {}
        Value::Text(t) => {
            let cdata = t.contains(&b'\n') && !t.windows(3).any(|w| w == b"]]>");
            if cdata {
                // The section is written with strlen: it ends at a NUL.
                let end = t.iter().position(|c| *c == 0).unwrap_or(t.len());
                out.extend_from_slice(b"<![CDATA[");
                out.extend_from_slice(&t[..end]);
                out.extend_from_slice(b"]]>");
            } else {
                escape(out, t);
            }
        }
        Value::Content(url) if url.is_empty() => out.extend_from_slice(b"<null></null>"),
        Value::Content(url) => {
            out.extend_from_slice(b"<url>");
            escape(out, url);
            out.extend_from_slice(b"</url>");
        }
    }
    if !node.children.is_empty() {
        for child in &node.children {
            out.push(b'\n');
            write_node(out, child, depth + 1);
        }
        out.push(b'\n');
        out.extend(std::iter::repeat_n(b'\t', depth));
    }
    out.extend_from_slice(b"</");
    out.extend_from_slice(node.tag.as_bytes());
    out.push(b'>');
}

/// The referent of every instance: the one it was read with when that is
/// unique, otherwise a new "RBX" + 32 hex digits.
fn referents(model: &Model) -> Vec<String> {
    let mut used = HashSet::new();
    let mut out = vec![String::new(); model.instances.len()];
    for (i, instance) in model.instances.iter().enumerate() {
        if let Some(r) = &instance.referent
            && used.insert(r.clone())
        {
            out[i] = r.clone();
        }
    }
    let mut state: u64 = 0x5241_4e44_5245_4652;
    let mut next = || {
        // splitmix64
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    for r in out.iter_mut().filter(|r| r.is_empty()) {
        loop {
            let id = format!("RBX{:016X}{:016X}", next(), next());
            if used.insert(id.clone()) {
                *r = id;
                break;
            }
        }
    }
    out
}

/// Writes the model as the native XML serializer does (00bb070a builds
/// the root, Instance::writeXml 00ac2362 each Item).
pub fn write(model: &Model) -> Result<Vec<u8>, String> {
    let ids = referents(model);
    let referent = |i: usize| ids[i].clone();
    let mut seen = vec![false; model.instances.len()];
    fn item(
        model: &Model,
        index: usize,
        ids: &[String],
        referent: &dyn Fn(usize) -> String,
        seen: &mut [bool],
    ) -> Result<Node, String> {
        if std::mem::replace(&mut seen[index], true) {
            return Err(format!("instance {index} appears twice in the tree"));
        }
        let instance = &model.instances[index];
        let mut node = Node::new("Item");
        node.attributes = vec![
            ("class", instance.class.as_bytes().to_vec()),
            ("referent", ids[index].as_bytes().to_vec()),
        ];
        let mut properties = Node::new("Properties");
        for (name, value) in &instance.properties {
            if let Property::Ref(Some(t)) = value
                && *t >= model.instances.len()
            {
                return Err(format!("{}.{name}: no instance {t}", instance.class));
            }
            properties.children.push(property_node(name, value, referent));
        }
        node.children.push(properties);
        for &child in &instance.children {
            node.children.push(item(model, child, ids, referent, seen)?);
        }
        Ok(node)
    }
    let mut root = Node::new("roblox");
    root.attributes = vec![
        ("xmlns:xmime", b"http://www.w3.org/2005/05/xmlmime".to_vec()),
        ("xmlns:xsi", b"http://www.w3.org/2001/XMLSchema-instance".to_vec()),
        (
            "xsi:noNamespaceSchemaLocation",
            b"http://www.roblox.com/roblox.xsd".to_vec(),
        ),
        ("version", b"4".to_vec()),
    ];
    root.children.push(Node::text("External", "null"));
    root.children.push(Node::text("External", "nil"));
    for &r in &model.roots {
        root.children
            .push(item(model, r, &ids, &referent, &mut seen)?);
    }
    let mut out = Vec::new();
    write_node(&mut out, &root, 0);
    Ok(out)
}

/// The XML models in the install: every `.rbxm` that is not binary and
/// every `.rbxmx`.
pub const NATIVE_XML_MODELS: [&str; 19] = [
    "Contents/Resources/BuiltInPlugins/PhysicsAnalyzer.rbxmx",
    "Contents/Resources/BuiltInPlugins/TerrainTools.rbxmx",
    "Contents/Resources/BuiltInPlugins/TransformDragger.rbxmx",
    "Contents/Resources/BuiltInPlugins/VideoDocumentation.rbxmx",
    "Contents/Resources/content/fonts/Rocket.rbxm",
    "Contents/Resources/content/fonts/SlingshotPellet.rbxm",
    "Contents/Resources/content/fonts/character.rbxm",
    "Contents/Resources/content/fonts/character3.rbxm",
    "Contents/Resources/content/fonts/characterCameraScript.rbxmx",
    "Contents/Resources/content/fonts/characterControlScript.rbxmx",
    "Contents/Resources/content/fonts/characterR15.rbxm",
    "Contents/Resources/content/fonts/humanoidAnimate.rbxm",
    "Contents/Resources/content/fonts/humanoidAnimateLocal.rbxm",
    "Contents/Resources/content/fonts/humanoidAnimateLocalKeyframe.rbxm",
    "Contents/Resources/content/fonts/humanoidAnimateLocalKeyframe2.rbxm",
    "Contents/Resources/content/fonts/humanoidAnimateR15.rbxm",
    "Contents/Resources/content/fonts/humanoidExtra.rbxm",
    "Contents/Resources/content/fonts/humanoidSoundNewLocal.rbxmx",
    "Contents/Resources/content/fonts/humanoidStatic.rbxm",
];

/// The native XML models that `write` reproduces byte for byte (the files
/// the 0.255-era macOS writer produced). The others were written by other
/// writers:
/// - TerrainTools.rbxmx has a newline after `</roblox>`;
/// - Rocket.rbxm and characterR15.rbxm print exponents with three digits
///   (`e-007`, MSVC's printf);
/// - the rest indent with spaces, have Items without a referent, or escape
///   multi-line strings instead of using CDATA.
pub const NATIVE_XML_IDENTICAL: [&str; 7] = [
    "Contents/Resources/BuiltInPlugins/PhysicsAnalyzer.rbxmx",
    "Contents/Resources/BuiltInPlugins/TransformDragger.rbxmx",
    "Contents/Resources/BuiltInPlugins/VideoDocumentation.rbxmx",
    "Contents/Resources/content/fonts/character3.rbxm",
    "Contents/Resources/content/fonts/humanoidAnimateLocalKeyframe2.rbxm",
    "Contents/Resources/content/fonts/humanoidAnimateR15.rbxm",
    "Contents/Resources/content/fonts/humanoidSoundNewLocal.rbxmx",
];

/// For each native XML model: parse, write again and compare. Returns one
/// line per file (with the per-tag counts of properties the reader skipped)
/// and whether the rewrite is identical. Fails when a file in
/// `NATIVE_XML_IDENTICAL` is not.
pub fn rewrite_report(bundle: &crate::bundle::Bundle) -> Result<Vec<(String, bool)>, String> {
    let mut lines = Vec::new();
    for path in NATIVE_XML_MODELS {
        let bytes = bundle.read(path, 16 * 1024 * 1024)?;
        let model = parse(&bytes).map_err(|e| format!("{path}: {e}"))?;
        let written = write(&model)?;
        let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
        count_skipped(&bytes, &mut skipped)?;
        let first_difference = written.iter().zip(&bytes).position(|(a, b)| a != b).or(
            (written.len() != bytes.len()).then_some(written.len().min(bytes.len())),
        );
        let context = first_difference.map(|d| {
            let lo = d.saturating_sub(20);
            (
                String::from_utf8_lossy(&bytes[lo..(d + 30).min(bytes.len())]).into_owned(),
                String::from_utf8_lossy(&written[lo..(d + 30).min(written.len())]).into_owned(),
            )
        });
        if written != bytes && NATIVE_XML_IDENTICAL.contains(&path) {
            return Err(format!(
                "rewrite of {path} differs at {first_difference:?}: {context:?}"
            ));
        }
        lines.push((
            format!(
                "rbxmx_rewrite file={path} bytes={} written={} identical={} instances={} skipped={skipped:?} first_difference={first_difference:?} context={context:?}",
                bytes.len(),
                written.len(),
                written == bytes,
                model.instances.len(),
            ),
            written == bytes,
        ));
    }
    Ok(lines)
}

fn count_skipped(bytes: &[u8], skipped: &mut BTreeMap<String, usize>) -> Result<(), String> {
    let mut parser = Parser { s: bytes, i: 0 };
    let root = parser.element()?;
    fn visit(e: &Element, skipped: &mut BTreeMap<String, usize>) {
        if e.tag == b"Properties" {
            for p in &e.children {
                if property(p).is_none() {
                    *skipped
                        .entry(String::from_utf8_lossy(&p.tag).into_owned())
                        .or_default() += 1;
                }
            }
        }
        for c in &e.children {
            visit(c, skipped);
        }
    }
    visit(&root, skipped);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printf_g_matches_c() {
        let cases: [(f64, usize, &str); 12] = [
            (0.300000011920929, 9, "0.300000012"),
            (1.0, 9, "1"),
            (-0.5, 9, "-0.5"),
            (100.0, 9, "100"),
            (1.350_000_02, 9, "1.35000002"),
            (1e-5, 9, "9.99999975e-06"),
            (123456789.0, 9, "123456792"),
            (1234567890.0, 9, "1.23456794e+09"),
            (0.0001, 6, "0.0001"),
            (1e20, 20, "1e+20"),
            (0.1, 20, "0.10000000000000000555"),
            (-0.0, 9, "-0"),
        ];
        for (v, p, want) in cases {
            let v = if p == 9 { f64::from(v as f32) } else { v };
            assert_eq!(c_g(v, p), want, "{v} %.{p}g");
        }
    }

    #[test]
    fn escapes_and_entities_invert() {
        let raw = b"a<b>&\"'\t\xc3\xa9\x7f\r\n";
        let mut out = Vec::new();
        escape(&mut out, raw);
        assert_eq!(out, b"a&lt;b&gt;&amp;&quot;&apos;&#9;&#195;&#169;&#127;\r\n".to_vec());
        assert_eq!(decode_entities(&out).unwrap(), raw.to_vec());
        assert_eq!(decode_entities(b"&foo;&nbsp;").unwrap(), b"&foo; ".to_vec());
    }

    #[test]
    fn base64_wraps_after_18_groups() {
        let data: Vec<u8> = (0..60).collect();
        let text = base64_encode(&data);
        assert_eq!(text.iter().position(|c| *c == b'\n'), Some(72));
        assert_eq!(base64_decode(&text), data);
        assert_eq!(base64_encode(b"ab"), b"YWI=".to_vec());
    }

    #[test]
    fn color3_bytes_round_trip() {
        for k in 0..=255u32 {
            assert_eq!(color_byte(k as f32 / 255.0), k);
        }
        assert_eq!(color_byte(2.0), 255);
    }

    #[test]
    fn every_type_round_trips() {
        let mut model = Model::default();
        let mut a = ModelInstance::new("Folder");
        a.set("A", Property::String("line\nbreak".into()))
            .set("B", Property::Bytes(vec![0xff, 1]))
            .set("C", Property::Content("rbxasset://x".into()))
            .set("D", Property::Content(String::new()))
            .set("E", Property::ProtectedString("print(1)".into()))
            .set("F", Property::BinaryString((0..100).collect()))
            .set("G", Property::Bool(true))
            .set("H", Property::Int(-7))
            .set("I", Property::Float(0.1))
            .set("J", Property::Double(0.1))
            .set("K", Property::Token(3))
            .set(
                "L",
                Property::CFrame(CoordinateFrame {
                    position: [1.0, 2.0, 3.0],
                    rotation: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
                }),
            )
            .set("M", Property::Vector3([1.5, -2.0, 1e-7]))
            .set("N", Property::Vector2([0.25, 4.0]))
            .set("O", Property::Vector3int16([-3, 4, 5]))
            .set("P", Property::Vector2int16([-1, 2]))
            .set("Q", Property::Color3([1.0, 0.0, 128.0 / 255.0]))
            .set("R", Property::UDim2([0.5, 1.0], [-10, 20]))
            .set("S", Property::UDim(0.5, 3))
            .set("T", Property::Ray([1.0, 2.0, 3.0], [0.0, -1.0, 0.0]))
            .set("U", Property::Faces(0b101101))
            .set("V", Property::Axes(0b101))
            .set("W", Property::NumberSequence(vec![[0.0, 1.0, 0.0], [1.0, 0.5, 0.25]]))
            .set(
                "X",
                Property::ColorSequence(vec![[0.0, 1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 1.0, 0.0]]),
            )
            .set("Y", Property::NumberRange([1.0, 2.5]))
            .set("Z", Property::Rect2D([0.0, 1.0, 2.0, 3.0]))
            .set("a", Property::PhysicalProperties(None))
            .set(
                "b",
                Property::PhysicalProperties(Some([0.7, 0.3, 0.5, 1.0, 1.0])),
            )
            .set("c", Property::BrickColor(194));
        let root = model.add(a, None);
        let mut b = ModelInstance::new("ObjectValue");
        b.set("Value", Property::Ref(Some(root)))
            .set("Other", Property::Ref(None));
        model.add(b, Some(root));
        let xml = write(&model).unwrap();
        let back = parse(&xml).unwrap();
        assert_eq!(back.instances.len(), 2);
        let mut want = model.instances[0].properties.clone();
        // XML has no BrickColor type: the native tag is int.
        want.insert("c".into(), Property::Int(194));
        assert_eq!(back.instances[0].properties, want);
        assert_eq!(back.instances[1].properties, model.instances[1].properties);
        assert_eq!(back.instances[0].children, vec![1]);
        // The referents read back are written back.
        assert_eq!(write(&back).unwrap(), xml);
    }
}
