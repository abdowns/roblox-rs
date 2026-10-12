//! Literal XML v4 Part import. No modern reflection defaults or script execution.
use bevy::prelude::Component;
use roxmltree::{Document, Node};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct CoordinateFrame {
    pub position: [f32; 3],
    /// The stored matrix has rows R00..R02, R10..R12, R20..R22.
    pub rotation: [[f32; 3]; 3],
}

impl CoordinateFrame {
    pub fn transform_point(&self, local: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|i| {
            self.position[i]
                + self.rotation[i][0] * local[0]
                + self.rotation[i][1] * local[1]
                + self.rotation[i][2] * local[2]
        })
    }
}

#[derive(Component, Clone, Debug)]
pub struct Part {
    pub frame: CoordinateFrame,
    pub size: [f32; 3],
    pub shape: u32,
    pub brick_color: u32,
    pub transparency: f32,
    pub anchored: bool,
    pub can_collide: bool,
}

#[derive(Component, Clone, Debug)]
pub struct Instance {
    pub referent: String,
    pub parent: Option<usize>,
    pub class: String,
    pub name: String,
    pub part: Option<Part>,
    /// Properties outside the current typed boundary are counted, not interpreted.
    pub serialized_properties: usize,
}

#[derive(Debug)]
pub struct Scene {
    pub instances: Vec<Instance>,
}

fn property<'a, 'b>(item: Node<'a, 'b>, name: &str, kind: &str) -> Result<Node<'a, 'b>, String> {
    let props = item
        .children()
        .find(|n| n.has_tag_name("Properties"))
        .ok_or("missing Properties")?;
    let mut found = props
        .children()
        .filter(|n| n.is_element() && n.attribute("name") == Some(name));
    let node = found
        .next()
        .ok_or_else(|| format!("missing {kind} {name}"))?;
    if found.next().is_some() || !node.has_tag_name(kind) {
        return Err(format!("invalid or duplicate {name}"));
    }
    Ok(node)
}

fn value<T: std::str::FromStr>(item: Node<'_, '_>, name: &str, kind: &str) -> Result<T, String> {
    property(item, name, kind)?
        .text()
        .unwrap_or("")
        .trim()
        .parse()
        .map_err(|_| format!("invalid {name}"))
}

fn components<const N: usize>(node: Node<'_, '_>, labels: [&str; N]) -> Result<[f32; N], String> {
    let mut values = [0.0f32; N];
    for (i, label) in labels.iter().enumerate() {
        let mut matches = node.children().filter(|n| n.has_tag_name(*label));
        values[i] = matches
            .next()
            .and_then(|n| n.text())
            .ok_or_else(|| format!("missing {label}"))?
            .trim()
            .parse()
            .map_err(|_| format!("invalid {label}"))?;
        if matches.next().is_some() || !values[i].is_finite() {
            return Err(format!("invalid {label}"));
        }
    }
    Ok(values)
}

impl Scene {
    pub fn parse(xml: &str) -> Result<Self, String> {
        let doc = Document::parse(xml).map_err(|e| e.to_string())?;
        let root = doc.root_element();
        if !root.has_tag_name("roblox") || root.attribute("version") != Some("4") {
            return Err("expected roblox XML version 4".into());
        }
        let mut instances = Vec::new();
        let mut nodes = std::collections::HashMap::new();
        let mut referents = std::collections::BTreeSet::new();
        for item in root.descendants().filter(|n| n.has_tag_name("Item")) {
            let referent = item
                .attribute("referent")
                .ok_or("missing referent")?
                .to_owned();
            if !referents.insert(referent.clone()) {
                return Err(format!("duplicate referent {referent}"));
            }
            let class = item.attribute("class").ok_or("missing class")?.to_owned();
            let name = property(item, "Name", "string")?
                .text()
                .unwrap_or("")
                .to_owned();
            let parent = item
                .parent()
                .filter(|n| n.has_tag_name("Item"))
                .map(|n| nodes.get(&n.id()).copied().ok_or("missing parent"))
                .transpose()?;
            let part = if class == "Part" {
                let f = components(
                    property(item, "CFrame", "CoordinateFrame")?,
                    [
                        "X", "Y", "Z", "R00", "R01", "R02", "R10", "R11", "R12", "R20", "R21",
                        "R22",
                    ],
                )?;
                let size = components(property(item, "size", "Vector3")?, ["X", "Y", "Z"])?;
                let transparency: f32 = value(item, "Transparency", "float")?;
                if size.iter().any(|n| *n <= 0.0)
                    || !transparency.is_finite()
                    || !(0.0..=1.0).contains(&transparency)
                {
                    return Err("invalid Part size or transparency".into());
                }
                Some(Part {
                    frame: CoordinateFrame {
                        position: f[..3].try_into().unwrap(),
                        rotation: [
                            f[3..6].try_into().unwrap(),
                            f[6..9].try_into().unwrap(),
                            f[9..12].try_into().unwrap(),
                        ],
                    },
                    size,
                    shape: value(item, "shape", "token")?,
                    brick_color: value(item, "BrickColor", "int")?,
                    transparency,
                    anchored: value(item, "Anchored", "bool")?,
                    can_collide: value(item, "CanCollide", "bool")?,
                })
            } else {
                None
            };
            nodes.insert(item.id(), instances.len());
            instances.push(Instance {
                referent,
                parent,
                class,
                name,
                part,
                serialized_properties: item
                    .children()
                    .find(|n| n.has_tag_name("Properties"))
                    .unwrap()
                    .children()
                    .filter(|n| n.is_element())
                    .count(),
            });
        }
        if instances.is_empty() {
            return Err("empty instance tree".into());
        }
        Ok(Self { instances })
    }

    pub fn class_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for item in &self.instances {
            *counts.entry(item.class.clone()).or_default() += 1;
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotated_frame_uses_rows_and_then_translation() {
        let frame = CoordinateFrame {
            position: [10.0, 20.0, 30.0],
            rotation: [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]],
        };
        assert_eq!(frame.transform_point([2.0, 3.0, 4.0]), [14.0, 23.0, 28.0]);
    }
    #[test]
    fn preserves_nested_instances_and_rejects_duplicates() {
        let xml = r#"<roblox version="4"><Item class="Model" referent="a"><Properties><string name="Name">Root</string></Properties><Item class="Model" referent="b"><Properties><string name="Name">Child</string></Properties></Item></Item></roblox>"#;
        let scene = Scene::parse(xml).unwrap();
        assert_eq!(scene.instances[1].parent, Some(0));
        assert!(Scene::parse(&xml.replace("referent=\"b\"", "referent=\"a\"")).is_err());
        assert!(Scene::parse(&xml.replace("version=\"4\"", "version=\"5\"")).is_err());
    }
}
