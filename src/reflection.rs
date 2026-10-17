//! The part of the 0.255 reflection database that the model writers need:
//! which properties hold a BrickColor, Content, ProtectedString or
//! BinaryString. Those are the types the two file formats lose. XML writes
//! BrickColor as `<int>`, and binary writes the three string types as type
//! 0x01. Native Studio gets the type back from each property's descriptor
//! (reflection Type at descriptor +0x28, XML tag at Type +0x18, used by
//! 0056260e). `rbxm::Model` has no descriptors, so the writers look the
//! property up here (`Model::with_native_types`).
//!
//! How the table was recovered (Ghidra, program RobloxStudio):
//! - the Type singletons are built on first use:
//!   - BrickColor 0223cc88 (getter 00ab9362, Type("BrickColor", tag "int"));
//!   - Content 0223ca08 (getter 00ab5d26);
//!   - ProtectedString 0222c7b0 (getter 006b922e);
//!   - BinaryString 0223ccd8 (getter 00ab9a3e).
//!
//!   The ContentId subtypes (AnimationId, TextureId, ...) have one-call
//!   getters that return the Content Type (0062d1e0 and others), so they
//!   are all Content;
//! - every property descriptor goes through the constructor 00562476
//!   (this, class descriptor, Type, name, category, ...). The 202
//!   functions that call it are the typed constructors. Each one's Type
//!   argument was traced to a Type getter or Type object. Name and class
//!   were traced up through the per-class wrappers to the static
//!   initializers, which pass the name as a literal;
//! - class names and bases come from the ClassDescriptor constructor
//!   0055ea3e (this, base, name), called by each class's descriptor
//!   singleton (347 classes).
//!
//! Every Type argument resolved. The 22 other Type getters never reach the
//! four types. The script, its output and the checks are in
//! RobloxStudio/analysis/reflection-types-20261016.*.
use crate::rbxm::Property;
use std::collections::HashMap;
use std::sync::OnceLock;

/// A property type that one of the file formats does not keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    BrickColor,
    Content,
    ProtectedString,
    BinaryString,
}

/// Class -> base class, for every class that has or inherits one of the
/// `PROPERTIES` (86 classes). From the ClassDescriptor constructor 0055ea3e
/// (this, base descriptor, name) at each class's descriptor singleton.
const BASES: [(&str, &str); 86] = [
    ("Animation", "Instance"),
    ("ArcHandles", "HandlesBase"),
    ("BackpackItem", "GuiItem"),
    ("BasePart", "PVInstance"),
    ("BaseScript", "LuaSourceContainer"),
    ("BinaryStringValue", "Instance"),
    ("BodyColors", "CharacterAppearance"),
    ("BoxHandleAdornment", "HandleAdornment"),
    ("BrickColorValue", "Instance"),
    ("Clothing", "CharacterAppearance"),
    ("ConeHandleAdornment", "HandleAdornment"),
    ("CoreScript", "BaseScript"),
    ("CornerWedgePart", "BasePart"),
    ("CylinderHandleAdornment", "HandleAdornment"),
    ("Decal", "FaceInstance"),
    ("FileMesh", "DataModelMesh"),
    ("Flag", "Tool"),
    ("FlagStand", "Part"),
    ("FloorWire", "GuiBase3d"),
    ("FormFactorPart", "BasePart"),
    ("Frame", "GuiObject"),
    ("GuiBase3d", "GuiBase"),
    ("GuiButton", "GuiObject"),
    ("GuiLabel", "GuiObject"),
    ("GuiObject", "GuiBase2d"),
    ("HandleAdornment", "PVAdornment"),
    ("Handles", "HandlesBase"),
    ("HandlesBase", "PartAdornment"),
    ("HopperBin", "BackpackItem"),
    ("ImageButton", "GuiButton"),
    ("ImageHandleAdornment", "HandleAdornment"),
    ("ImageLabel", "GuiLabel"),
    ("LineHandleAdornment", "HandleAdornment"),
    ("LocalScript", "Script"),
    ("LuaSourceContainer", "Instance"),
    ("MeshPart", "BasePart"),
    ("ModuleScript", "LuaSourceContainer"),
    ("Mouse", "Instance"),
    ("NegateOperation", "PartOperation"),
    ("PVAdornment", "GuiBase3d"),
    ("Pants", "Clothing"),
    ("ParallelRampPart", "BasePart"),
    ("Part", "FormFactorPart"),
    ("PartAdornment", "GuiBase3d"),
    ("PartOperation", "BasePart"),
    ("PartOperationAsset", "Instance"),
    ("ParticleEmitter", "Instance"),
    ("Platform", "Part"),
    ("Player", "Instance"),
    ("PlayerMouse", "Mouse"),
    ("PluginMouse", "Mouse"),
    ("PrismPart", "BasePart"),
    ("PyramidPart", "BasePart"),
    ("RightAngleRampPart", "BasePart"),
    ("Scale9Frame", "GuiObject"),
    ("Script", "BaseScript"),
    ("ScrollingFrame", "GuiObject"),
    ("Seat", "Part"),
    ("SelectionBox", "PVAdornment"),
    ("SelectionLasso", "GuiBase3d"),
    ("SelectionPartLasso", "SelectionLasso"),
    ("SelectionPointLasso", "SelectionLasso"),
    ("SelectionSphere", "PVAdornment"),
    ("Shirt", "Clothing"),
    ("ShirtGraphic", "CharacterAppearance"),
    ("SkateboardPlatform", "Part"),
    ("Skin", "CharacterAppearance"),
    ("Sky", "Instance"),
    ("Sound", "Instance"),
    ("SpawnLocation", "Part"),
    ("SpecialMesh", "FileMesh"),
    ("SphereHandleAdornment", "HandleAdornment"),
    ("SurfaceSelection", "PartAdornment"),
    ("Team", "Instance"),
    ("Terrain", "BasePart"),
    ("TerrainRegion", "Instance"),
    ("TextBox", "GuiObject"),
    ("TextButton", "GuiButton"),
    ("TextLabel", "GuiLabel"),
    ("Texture", "Decal"),
    ("TextureTrail", "GuiBase3d"),
    ("Tool", "BackpackItem"),
    ("TrussPart", "BasePart"),
    ("UnionOperation", "PartOperation"),
    ("VehicleSeat", "BasePart"),
    ("WedgePart", "FormFactorPart"),
];

/// (class, property, type): every property descriptor (constructor
/// 00562476) whose Type is one of the four. The comment is the path from
/// the typed descriptor constructor through the per-class wrapper to the
/// static initializer that passes the name.
const PROPERTIES: [(&str, &str, ValueType); 69] = [
    ("Animation", "AnimationId", ValueType::Content), // 6e7c38>6e7aae>6e88b4
    ("BackpackItem", "TextureId", ValueType::Content), // 902d08>8fae82>8fdd76
    ("BasePart", "BrickColor", ValueType::BrickColor), // 8ae190>993c88>999b11
    ("BasePart", "brickColor", ValueType::BrickColor), // 8ae190>993c88>999b11
    ("BaseScript", "LinkedSource", ValueType::Content), // cb2838>5a9c94>5ab57d
    ("BinaryStringValue", "Value", ValueType::BinaryString), // cfd9d0>a588b2>a687a7
    ("BodyColors", "HeadColor", ValueType::BrickColor), // 8ae190>756f4a>759c6f
    ("BodyColors", "LeftArmColor", ValueType::BrickColor), // 8ae190>756f4a>759c6f
    ("BodyColors", "LeftLegColor", ValueType::BrickColor), // 8ae190>756f4a>759c6f
    ("BodyColors", "RightArmColor", ValueType::BrickColor), // 8ae190>756f4a>759c6f
    ("BodyColors", "RightLegColor", ValueType::BrickColor), // 8ae190>756f4a>759c6f
    ("BodyColors", "TorsoColor", ValueType::BrickColor), // 8ae190>756f4a>759c6f
    ("BrickColorValue", "Value", ValueType::BrickColor), // 8ae190>a5f0ce>a687a7
    ("Clothing", "Outfit1", ValueType::Content),      // 902d08>755d90>759c6f
    ("Clothing", "Outfit2", ValueType::Content),      // 902d08>755d90>759c6f
    ("Decal", "Texture", ValueType::Content),         // 902d08>800c8c>802341
    ("FileMesh", "MeshId", ValueType::Content),       // c4eaa8>862000>86285b
    ("FileMesh", "TextureId", ValueType::Content),    // 902d08>862194>86285b
    ("Flag", "TeamColor", ValueType::BrickColor),     // 8ae190>870f38>8720d1
    ("FlagStand", "TeamColor", ValueType::BrickColor), // 8ae190>87466e>876505
    ("FloorWire", "Texture", ValueType::Content),     // 902d08>ce6d0e>ce7dda
    ("GuiBase3d", "Color", ValueType::BrickColor),    // 8ae190>8886f4>888f40
    ("GuiObject", "BackgroundColor", ValueType::BrickColor), // 8ae190>8ae0c6>8bf6d7
    ("GuiObject", "BorderColor", ValueType::BrickColor), // 8ae190>8ae0c6>8bf6d7
    ("ImageButton", "Image", ValueType::Content),     // 902d08>902c3e>904309
    ("ImageHandleAdornment", "Image", ValueType::Content), // 902d08>82467c>827a14
    ("ImageLabel", "Image", ValueType::Content),      // 902d08>907240>908a1b
    (
        "LuaSourceContainer",
        "CachedRemoteSource",
        ValueType::ProtectedString,
    ), // cb25ec>d09af2>d0dd72
    ("MeshPart", "MeshID", ValueType::Content),       // c4eaa8>c4e9de>c50236
    ("MeshPart", "PhysicsData", ValueType::BinaryString), // cfd9d0>c4eec6>c50236
    ("MeshPart", "TextureID", ValueType::Content),    // 902d08>c4ecb4>c50236
    ("ModuleScript", "LinkedSource", ValueType::Content), // cb2838>cb276e>cb3aab
    ("ModuleScript", "Source", ValueType::ProtectedString), // cb25ec>cb2522>cb3aab
    ("Mouse", "Icon", ValueType::Content),            // 902d08>97b27a>97b91e
    ("Pants", "PantsTemplate", ValueType::Content),   // 902d08>756256>759c6f
    ("PartOperation", "AssetId", ValueType::Content), // cb2838>603b78>606e96
    ("PartOperation", "ChildData", ValueType::BinaryString), // cfd9d0>6032b4>606e96
    ("PartOperation", "MeshData", ValueType::BinaryString), // cfd9d0>6032b4>606e96
    ("PartOperation", "PhysicsData", ValueType::BinaryString), // cfd9d0>6032b4>606e96
    ("PartOperationAsset", "ChildData", ValueType::BinaryString), // cfd9d0>6dbdaa>6dca92
    ("PartOperationAsset", "MeshData", ValueType::BinaryString), // cfd9d0>6dbdaa>6dca92
    ("ParticleEmitter", "Texture", ValueType::Content), // 902d08>b7d2f2>b7f60e
    ("Player", "TeamColor", ValueType::BrickColor),   // 8ae190>f8dc32>fab1f2
    ("Script", "Source", ValueType::ProtectedString), // cb25ec>5a9b00>5ab57d
    ("ScrollingFrame", "BottomImage", ValueType::Content), // 902d08>936072>9375c7
    ("ScrollingFrame", "MidImage", ValueType::Content), // 902d08>936072>9375c7
    ("ScrollingFrame", "TopImage", ValueType::Content), // 902d08>936072>9375c7
    ("SelectionBox", "SurfaceColor", ValueType::BrickColor), // 8ae190>9d5220>9d619e
    ("SelectionSphere", "SurfaceColor", ValueType::BrickColor), // 8ae190>dfe372>dff2f0
    ("Shirt", "ShirtTemplate", ValueType::Content),   // 902d08>755f86>759c6f
    ("ShirtGraphic", "Graphic", ValueType::Content),  // 902d08>755b4a>759c6f
    ("Skin", "SkinColor", ValueType::BrickColor),     // 8ae190>756d02>759c6f
    ("Sky", "SkyboxBk", ValueType::Content),          // 902d08>9e8932>9e93ab
    ("Sky", "SkyboxDn", ValueType::Content),          // 902d08>9e8932>9e93ab
    ("Sky", "SkyboxFt", ValueType::Content),          // 902d08>9e8932>9e93ab
    ("Sky", "SkyboxLf", ValueType::Content),          // 902d08>9e8932>9e93ab
    ("Sky", "SkyboxRt", ValueType::Content),          // 902d08>9e8932>9e93ab
    ("Sky", "SkyboxUp", ValueType::Content),          // 902d08>9e8932>9e93ab
    ("Sound", "SoundId", ValueType::Content),         // dcb532>dcb468>dd4834
    ("SpawnLocation", "TeamColor", ValueType::BrickColor), // 8ae190>9f5640>9f715d
    ("Team", "TeamColor", ValueType::BrickColor),     // 8ae190>a145de>a157ff
    ("Terrain", "ClusterGridV3", ValueType::BinaryString), // cfd9d0>cfd906>d03cf5
    ("Terrain", "SmoothGrid", ValueType::BinaryString), // cfd9d0>cfd906>d03cf5
    ("TerrainRegion", "GridV3", ValueType::BinaryString), // cfd9d0>aa09c4>aa1864
    ("TerrainRegion", "SmoothGrid", ValueType::BinaryString), // cfd9d0>aa09c4>aa1864
    ("TextBox", "TextColor", ValueType::BrickColor),  // 8ae190>a1fe74>a220a3
    ("TextButton", "TextColor", ValueType::BrickColor), // 8ae190>a27072>a290e3
    ("TextLabel", "TextColor", ValueType::BrickColor), // 8ae190>a2e51c>a3051d
    ("TextureTrail", "Texture", ValueType::Content),  // 902d08>cbfaba>cc0b86
];

struct Tables {
    bases: HashMap<&'static str, &'static str>,
    properties: HashMap<(&'static str, &'static str), ValueType>,
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| Tables {
        bases: BASES.into_iter().collect(),
        properties: PROPERTIES
            .into_iter()
            .map(|(c, p, t)| ((c, p), t))
            .collect(),
    })
}

/// The type of `class.property` when it is one of the four, looking
/// through the base classes as the native descriptor lookup does.
pub fn property_type(class: &str, property: &str) -> Option<ValueType> {
    let t = tables();
    let mut class = class;
    loop {
        if let Some(found) = t.properties.get(&(class, property)) {
            return Some(*found);
        }
        class = t.bases.get(class)?;
    }
}

/// `value` as the type reflection gives `class.property`: an Int on a
/// BrickColor property becomes BrickColor, a String on a Content or
/// ProtectedString property becomes that variant, and the string bytes on
/// a BinaryString property become BinaryString. None when nothing changes
/// (other properties, values that already have the type, and strings that
/// are not UTF-8 on a Content or ProtectedString property, which those
/// variants cannot hold).
pub fn native_value(class: &str, property: &str, value: &Property) -> Option<Property> {
    Some(match (property_type(class, property)?, value) {
        (ValueType::BrickColor, Property::Int(v)) => Property::BrickColor(*v as u32),
        (ValueType::Content, Property::String(s)) => Property::Content(s.clone()),
        (ValueType::ProtectedString, Property::String(s)) => Property::ProtectedString(s.clone()),
        (ValueType::BinaryString, Property::String(s)) => {
            Property::BinaryString(s.as_bytes().to_vec())
        }
        (ValueType::BinaryString, Property::Bytes(b)) => Property::BinaryString(b.clone()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inherited_properties_resolve() {
        assert_eq!(
            property_type("Part", "BrickColor"),
            Some(ValueType::BrickColor)
        );
        assert_eq!(
            property_type("SpecialMesh", "MeshId"),
            Some(ValueType::Content)
        );
        assert_eq!(
            property_type("LocalScript", "Source"),
            Some(ValueType::ProtectedString)
        );
        assert_eq!(
            property_type("LocalScript", "LinkedSource"),
            Some(ValueType::Content)
        );
        assert_eq!(
            property_type("MeshPart", "PhysicsData"),
            Some(ValueType::BinaryString)
        );
        assert_eq!(property_type("Tool", "TextureId"), Some(ValueType::Content));
        // Same name, other types: IntValue.Value stays Int, and a
        // PointLight's Color is a Color3.
        assert_eq!(property_type("IntValue", "Value"), None);
        assert_eq!(
            property_type("BrickColorValue", "Value"),
            Some(ValueType::BrickColor)
        );
        assert_eq!(property_type("PointLight", "Color"), None);
        assert_eq!(property_type("Part", "Name"), None);
        assert_eq!(property_type("NoSuchClass", "BrickColor"), None);
    }
    #[test]
    fn values_take_the_reflected_type() {
        assert_eq!(
            native_value("Part", "BrickColor", &Property::Int(194)),
            Some(Property::BrickColor(194))
        );
        assert_eq!(
            native_value("Part", "BrickColor", &Property::BrickColor(194)),
            None
        );
        assert_eq!(native_value("IntValue", "Value", &Property::Int(3)), None);
        assert_eq!(
            native_value("Script", "Source", &Property::String("print(1)".into())),
            Some(Property::ProtectedString("print(1)".into()))
        );
        assert_eq!(
            native_value(
                "Animation",
                "AnimationId",
                &Property::String("rbxassetid://1".into())
            ),
            Some(Property::Content("rbxassetid://1".into()))
        );
        assert_eq!(
            native_value("MeshPart", "PhysicsData", &Property::Bytes(vec![0xff, 0])),
            Some(Property::BinaryString(vec![0xff, 0]))
        );
        assert_eq!(
            native_value("Script", "Source", &Property::Bytes(vec![0xff])),
            None
        );
    }
}
