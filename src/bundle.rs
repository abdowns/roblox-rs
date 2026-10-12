use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

pub const TARGET_VERSION: &str = "0.255.0.82159";
pub const TARGET_SHA256: &str = "f8954ab868b1a9e0f2a2cfdfdc6788c5989ea543dc4f500fe215e07a911dce15";

/// A verified installation. File access is confined to this canonical root.
#[derive(Debug)]
pub struct Bundle {
    root: PathBuf,
}

impl Bundle {
    pub fn open(root: &Path) -> Result<Self, String> {
        let bundle = Self {
            root: root.canonicalize().map_err(|e| e.to_string())?,
        };
        let plist = bundle.read("Contents/Info.plist", 1024 * 1024)?;
        let xml = std::str::from_utf8(&plist).map_err(|e| e.to_string())?;
        // Apple's standard plist declaration is permitted; roxmltree does no network I/O.
        let doc = roxmltree::Document::parse_with_options(
            xml,
            roxmltree::ParsingOptions {
                allow_dtd: true,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
        let version = doc
            .descendants()
            .find(|n| n.has_tag_name("key") && n.text() == Some("CFBundleShortVersionString"))
            .and_then(|n| n.next_sibling_element())
            .filter(|n| n.has_tag_name("string"))
            .and_then(|n| n.text())
            .ok_or("missing CFBundleShortVersionString")?;
        if version != TARGET_VERSION {
            return Err(format!(
                "wrong Studio version: expected {TARGET_VERSION}, found {version}"
            ));
        }
        let path = bundle.path("Contents/MacOS/RobloxStudio")?;
        let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
        let mut header = [0; 28];
        file.read_exact(&mut header).map_err(|e| e.to_string())?;
        verify_mach_header(&header)?;
        let mut hash = Sha256::new();
        hash.update(header);
        let mut block = [0; 64 * 1024];
        loop {
            let length = file.read(&mut block).map_err(|e| e.to_string())?;
            if length == 0 {
                break;
            }
            hash.update(&block[..length]);
        }
        let actual = format!("{:x}", hash.finalize());
        if actual != TARGET_SHA256 {
            return Err(format!("executable fingerprint mismatch: {actual}"));
        }
        Ok(bundle)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self, relative: &str) -> Result<PathBuf, String> {
        let relative = Path::new(relative);
        if relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err("bundle path must be relative with no traversal".into());
        }
        let path = self
            .root
            .join(relative)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !path.starts_with(&self.root) {
            return Err("bundle path escapes installation".into());
        }
        Ok(path)
    }

    pub fn read(&self, relative: &str, limit: u64) -> Result<Vec<u8>, String> {
        let path = self.path(relative)?;
        let file = fs::File::open(path).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        file.take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > limit {
            return Err(format!("{relative} exceeds read limit"));
        }
        Ok(bytes)
    }

    pub fn files(&self, relative: &str, extension: &str) -> Result<Vec<String>, String> {
        let mut names = Vec::new();
        for entry in fs::read_dir(self.path(relative)?).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.path().extension().and_then(|s| s.to_str()) == Some(extension) {
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| "non-UTF8 asset name")?;
                // Verify symlinks too before returning an asset name.
                self.path(&format!("{relative}/{name}"))?;
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }
}

pub fn verify_mach_header(header: &[u8]) -> Result<(), String> {
    if header.len() < 28 {
        return Err("truncated Mach-O header".into());
    }
    let word = |offset| u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap());
    if (word(0), word(4), word(8), word(12)) != (0xfeedface, 7, 3, 2) {
        return Err("expected little-endian i386 Mach-O executable".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn header_rejects_64_bit_and_non_executable_images() {
        let mut data = Vec::new();
        for word in [0xfeedfaceu32, 7, 3, 2, 0, 0, 0] {
            data.extend(word.to_le_bytes());
        }
        verify_mach_header(&data).unwrap();
        data[4..8].copy_from_slice(&0x01000007u32.to_le_bytes());
        assert!(verify_mach_header(&data).is_err());
        assert!(verify_mach_header(&data[..20]).is_err());
    }
    #[test]
    fn paths_reject_absolute_and_parent_paths() {
        let bundle = Bundle {
            root: PathBuf::from("/synthetic/root"),
        };
        assert!(bundle.path("../Cargo.toml").is_err());
        assert!(bundle.path("/outside").is_err());
    }
}
