//! Bounded Bevy import diagnostic. Updates verify retention, not game simulation.
use crate::{
    bundle::{Bundle, TARGET_SHA256, TARGET_VERSION},
    mesh::MeshData,
    scene::{Instance, Part, Scene},
};
use bevy::prelude::*;
use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const FONTS: &str = "Contents/Resources/content/fonts";

#[derive(Resource)]
struct Expected {
    instances: Vec<(Entity, Option<Entity>, Instance)>,
    mesh_counts: (usize, usize, usize),
}

#[derive(Resource, Default)]
struct Checks {
    updates: u32,
    observations: Vec<String>,
    error: Option<String>,
}

#[derive(Component)]
struct ImportedMesh;

fn frame_matches(a: &Part, b: &Part) -> bool {
    a.frame
        .position
        .iter()
        .chain(a.frame.rotation.iter().flatten())
        .chain(&a.size)
        .zip(
            b.frame
                .position
                .iter()
                .chain(b.frame.rotation.iter().flatten())
                .chain(&b.size),
        )
        .all(|(left, right)| left.to_bits() == right.to_bits())
        && a.transparency.to_bits() == b.transparency.to_bits()
        && a.shape == b.shape
        && a.brick_color == b.brick_color
        && a.anchored == b.anchored
        && a.can_collide == b.can_collide
}

fn verify_import(
    expected: Res<Expected>,
    mut checks: ResMut<Checks>,
    instances: Query<(&Instance, Option<&Part>, Option<&ChildOf>)>,
    meshes: Query<&MeshData, With<ImportedMesh>>,
) {
    checks.updates += 1;
    if checks.error.is_some() {
        return;
    }
    let fail = (|| -> Result<(), String> {
        if instances.iter().count() != expected.instances.len() {
            return Err("instance count changed".into());
        }
        for (entity, parent, before) in &expected.instances {
            let (instance, part, link) = instances
                .get(*entity)
                .map_err(|_| "missing imported instance")?;
            if link.map(|link| link.parent()) != *parent
                || instance.referent != before.referent
                || instance.class != before.class
                || instance.name != before.name
                || instance.parent != before.parent
                || instance.serialized_properties != before.serialized_properties
            {
                return Err(format!(
                    "instance hierarchy/identity changed: {}",
                    before.referent
                ));
            }
            match (&before.part, part) {
                (Some(a), Some(b)) if frame_matches(a, b) => (),
                (None, None) => (),
                _ => return Err(format!("Part state changed: {}", before.referent)),
            }
        }
        let mut counts = (0, 0, 0);
        for mesh in &meshes {
            mesh.finite_bounds()?;
            counts.0 += 1;
            counts.1 += mesh.vertices.len();
            counts.2 += mesh.triangles.len();
        }
        if counts != expected.mesh_counts {
            return Err("mesh counts changed".into());
        }
        Ok(())
    })();
    if let Err(error) = fail {
        checks.error = Some(error);
    }
    if checks.updates == 1 || checks.updates.is_multiple_of(40) {
        let update = checks.updates;
        checks.observations.push(format!(
            "import_retained update={update} instances={} meshes={} vertices={} triangles={}",
            expected.instances.len(),
            expected.mesh_counts.0,
            expected.mesh_counts.1,
            expected.mesh_counts.2
        ));
    }
}

/// Run the same file import used by future runtime adapters, retaining original
/// data in components. No window, network requests, scripts or physics are run.
pub fn audit(bundle: &Bundle, updates: u32) -> Result<String, String> {
    if updates == 0 || updates > 10_000 {
        return Err("diagnostic updates must be 1..=10000".into());
    }
    let start = Instant::now();
    let mut report = format!(
        "bundle_verified version={TARGET_VERSION} architecture=i386 sha256={TARGET_SHA256}\n"
    );
    let mut meshes = BTreeMap::new();
    for name in bundle.files(FONTS, "mesh")? {
        let data = bundle.read(&format!("{FONTS}/{name}"), 64 * 1024 * 1024)?;
        let mesh = MeshData::parse(&data).map_err(|e| format!("{name}: {e}"))?;
        let (low, high) = mesh.finite_bounds().map_err(|e| format!("{name}: {e}"))?;
        writeln!(
            report,
            "mesh_loaded name={name:?} vertices={} triangles={} min={low:?} max={high:?}",
            mesh.vertices.len(),
            mesh.triangles.len()
        )
        .unwrap();
        meshes.insert(name, mesh);
    }
    if meshes.is_empty() {
        return Err("no mesh files found".into());
    }
    let mesh_counts = (
        meshes.len(),
        meshes.values().map(|m| m.vertices.len()).sum(),
        meshes.values().map(|m| m.triangles.len()).sum(),
    );
    writeln!(
        report,
        "mesh_totals meshes={} vertices={} triangles={}",
        mesh_counts.0, mesh_counts.1, mesh_counts.2
    )
    .unwrap();

    let data = bundle.read(&format!("{FONTS}/character3.rbxm"), 16 * 1024 * 1024)?;
    let scene = Scene::parse(std::str::from_utf8(&data).map_err(|e| e.to_string())?)?;
    for (class, count) in scene.class_counts() {
        writeln!(report, "scene_class class={class:?} count={count}").unwrap();
    }
    let parts = scene
        .instances
        .iter()
        .filter_map(|i| i.part.as_ref())
        .count();
    let visible = scene
        .instances
        .iter()
        .filter_map(|i| i.part.as_ref())
        .filter(|p| p.transparency < 1.0)
        .count();
    writeln!(
        report,
        "scene_loaded instances={} parts={parts} visible_parts={visible}",
        scene.instances.len()
    )
    .unwrap();
    for item in &scene.instances {
        if let Some(part) = &item.part {
            writeln!(report, "part_loaded referent={:?} name={:?} parent={:?} position={:?} rotation={:?} size={:?} shape={} brick_color={} transparency={} anchored={} can_collide={}",
                item.referent, item.name, item.parent, part.frame.position, part.frame.rotation,
                part.size, part.shape, part.brick_color, part.transparency, part.anchored, part.can_collide).unwrap();
        }
    }

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    let mut entities = Vec::new();
    let mut expected = Vec::new();
    for instance in scene.instances {
        let parent = instance.parent.map(|index| entities[index]);
        let mut entity = app.world_mut().spawn(instance.clone());
        if let Some(part) = &instance.part {
            entity.insert(part.clone());
        }
        if let Some(parent) = parent {
            entity.insert(ChildOf(parent));
        }
        let id = entity.id();
        entities.push(id);
        expected.push((id, parent, instance));
    }
    for (name, mesh) in meshes {
        app.world_mut().spawn((ImportedMesh, Name::new(name), mesh));
    }
    app.insert_resource(Expected {
        instances: expected,
        mesh_counts,
    });
    app.init_resource::<Checks>();
    app.add_systems(Update, verify_import);
    for _ in 0..updates {
        app.update();
    }
    let checks = app.world().resource::<Checks>();
    if let Some(error) = &checks.error {
        return Err(error.clone());
    }
    if checks.updates != updates {
        return Err("Bevy update count mismatch".into());
    }
    for line in &checks.observations {
        writeln!(report, "{line}").unwrap();
    }
    writeln!(
        report,
        "static_import_pass updates={updates} elapsed_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.0
    )
    .unwrap();
    writeln!(
        report,
        "native_behavior_parity=not_tested visual_parity=not_tested"
    )
    .unwrap();
    Ok(report)
}

/// Create a new evidence log inside the ignored analysis directory. Never
/// overwrite an existing log, follow an output symlink, or write installed data.
pub fn save_log(bundle: &Bundle, report: &str) -> Result<PathBuf, String> {
    let directory = bundle.root().join("analysis");
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => (),
        Ok(_) => return Err("analysis output must be a real directory".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&directory).map_err(|e| e.to_string())?
        }
        Err(error) => return Err(error.to_string()),
    }
    let directory = directory.canonicalize().map_err(|e| e.to_string())?;
    if !directory.starts_with(bundle.root()) {
        return Err("analysis output escapes bundle".into());
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let path = directory.join(format!("runtime-audit-{stamp}.log"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    file.write_all(report.as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(path)
}
