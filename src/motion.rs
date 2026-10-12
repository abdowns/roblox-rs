//! Part-space geometry to recovered physics frames. The driver is an explicitly
//! unconstrained diagnostic; it does not classify native assemblies or contacts.
use crate::{
    animation::{Pose, ProceduralAnimation},
    bundle::Bundle,
    geometry::{Draw, RenderScene},
    physics::{self, FreeFallBody, GravitySource, NativePhysics},
    rig::R6Rig,
    scene::CoordinateFrame,
    texture::{self, TexturedDraw},
};
use bevy::prelude::*;
use std::collections::BTreeMap;

#[derive(Component)]
pub(crate) struct LocalGeometry(Draw);
#[derive(Component)]
struct OriginalBody(FreeFallBody);
#[derive(Component)]
pub(crate) struct BodyName(String);
#[derive(Resource)]
struct OriginalRig(R6Rig);
#[derive(Component)]
pub(crate) struct LocalDecal {
    body: Entity,
    geometry: Draw,
}

#[derive(Resource)]
pub(crate) struct MotionControl {
    profile: NativePhysics,
    pub requested_steps: u32,
    pub steps: u32,
    pub error: Option<String>,
    pub animation_probe: bool,
    animation: ProceduralAnimation,
    animation_ticks: u32,
}

pub(crate) fn pose_draw(local: &Draw, frame: &CoordinateFrame) -> Draw {
    let mut draw = local.clone();
    for (target, source) in draw.vertices.iter_mut().zip(&local.vertices) {
        // Same arithmetic order as the recovered geometry generator, without
        // inverse-scale normals or accumulating transforms in previous vertices.
        let p = source.position;
        let n = source.normal;
        target.position = std::array::from_fn(|a| {
            let r = frame.rotation[a];
            frame.position[a] + p[1] * r[1] + p[0] * r[0] + p[2] * r[2]
        });
        target.normal = std::array::from_fn(|a| {
            let r = frame.rotation[a];
            r[2] * n[2] + r[0] * n[0] + n[1] * r[1]
        });
    }
    draw
}

pub(crate) fn install(app: &mut App, bundle: &Bundle) -> Result<(), String> {
    let local = RenderScene::character_local(bundle)?;
    let rig = R6Rig::character(bundle, &local.scene)?;
    let decals = texture::character_decals(bundle, &local)?;
    let mut draws: BTreeMap<_, _> = local
        .draws
        .into_iter()
        .map(|draw| (draw.name.clone(), draw))
        .collect();
    let entities: BTreeMap<_, _> = app
        .world_mut()
        .query::<(Entity, &Draw)>()
        .iter(app.world())
        .map(|(entity, draw)| (draw.name.clone(), entity))
        .collect();
    let mut bodies = BTreeMap::new();
    for (name, body) in physics::character_bodies(bundle)? {
        let entity = if let Some(entity) = entities.get(&name) {
            let draw = draws.remove(&name).ok_or("missing local Part geometry")?;
            app.world_mut()
                .entity_mut(*entity)
                .insert((
                    LocalGeometry(draw),
                    body.clone(),
                    OriginalBody(body),
                    BodyName(name.clone()),
                ))
                .id()
        } else {
            // Retain the transparent root as a physics body without drawing it.
            app.world_mut()
                .spawn((body.clone(), OriginalBody(body), BodyName(name.clone())))
                .id()
        };
        if bodies.insert(name, entity).is_some() {
            return Err("duplicate character Part name in motion fixture".into());
        }
    }
    if !draws.is_empty() {
        return Err("unbound local geometry".into());
    }
    let decal_entities: BTreeMap<_, _> = app
        .world_mut()
        .query::<(Entity, &TexturedDraw)>()
        .iter(app.world())
        .map(|(entity, decal)| (decal.draw.name.clone(), entity))
        .collect();
    for decal in decals.draws {
        let name = &decal.draw.name;
        let part = name.split_once('/').ok_or("missing decal parent")?.0;
        let body = *bodies.get(part).ok_or("missing decal physics body")?;
        let entity = *decal_entities.get(name).ok_or("missing rendered decal")?;
        app.world_mut().entity_mut(entity).insert(LocalDecal {
            body,
            geometry: decal.draw,
        });
    }
    app.insert_resource(MotionControl {
        profile: NativePhysics::load(bundle)?,
        requested_steps: 0,
        steps: 0,
        error: None,
        animation_probe: false,
        animation: ProceduralAnimation::load(bundle)?,
        animation_ticks: 0,
    });
    app.insert_resource(OriginalRig(rig.clone()));
    app.insert_resource(rig);
    Ok(())
}

pub(crate) fn advance(
    mut bodies: Query<(&BodyName, &mut FreeFallBody)>,
    mut control: ResMut<MotionControl>,
    mut rig: ResMut<R6Rig>,
) {
    if control.error.is_some() {
        return;
    }
    let steps = std::mem::take(&mut control.requested_steps);
    if steps > 240 {
        control.error = Some("motion probe step request exceeds bound".into());
        return;
    }
    for _ in 0..steps {
        if control.animation_probe {
            let time = f64::from(control.steps + 1) / 60.0;
            let ticks = (time / control.animation.wait_seconds + 1e-9).floor() as u32;
            if ticks > control.animation_ticks {
                if let Err(error) = control.animation.apply(&mut rig, Pose::Running, time) {
                    control.error = Some(error);
                    return;
                }
                control.animation_ticks = ticks;
            }
            for joint in &mut rig.joints {
                if let Err(error) = joint.step() {
                    control.error = Some(error);
                    return;
                }
            }
        } else {
            for (name, mut body) in &mut bodies {
                if name.0 == "HumanoidRootPart"
                    && let Err(error) = body.step(&control.profile, GravitySource::Legacy)
                {
                    control.error = Some(error);
                    return;
                }
            }
        }
        control.steps += 1;
    }
    if control.steps == 0 {
        // Keep the serialized rest-frame bits until a root step is requested.
        // Native preset composition has equivalent but differently signed zeros.
        return;
    }
    // Integrate one unconstrained root, then pose the recovered native links.
    // This does not substitute kinematic posing for an unrecovered contact solver.
    let Some(root) = bodies
        .iter()
        .find(|(name, _)| name.0 == "HumanoidRootPart")
        .map(|(_, body)| body.frame.clone())
    else {
        control.error = Some("missing rig motion root".into());
        return;
    };
    match rig.pose(&root) {
        Ok(frames) => {
            for (name, mut body) in &mut bodies {
                if let Some(frame) = frames.get(&name.0) {
                    body.frame = frame.clone();
                }
            }
        }
        Err(error) => control.error = Some(error),
    }
}

pub(crate) fn sync_bodies(mut draws: Query<(&LocalGeometry, &FreeFallBody, &mut Draw)>) {
    for (local, body, mut draw) in &mut draws {
        *draw = pose_draw(&local.0, &body.frame);
    }
}

pub(crate) fn sync_decals(
    bodies: Query<&FreeFallBody>,
    mut decals: Query<(&LocalDecal, &mut TexturedDraw)>,
    mut control: ResMut<MotionControl>,
) {
    for (local, mut decal) in &mut decals {
        match bodies.get(local.body) {
            Ok(body) => decal.draw = pose_draw(&local.geometry, &body.frame),
            Err(_) => control.error = Some("rendered decal lost its parent body".into()),
        }
    }
}

pub(crate) fn reset(world: &mut World) {
    let rig = world.resource::<OriginalRig>().0.clone();
    *world.resource_mut::<R6Rig>() = rig;
    for (original, mut body) in world
        .query::<(&OriginalBody, &mut FreeFallBody)>()
        .iter_mut(world)
    {
        *body = original.0.clone();
    }
    let mut control = world.resource_mut::<MotionControl>();
    control.steps = 0;
    control.requested_steps = 0;
    control.error = None;
    control.animation_probe = false;
    control.animation_ticks = 0;
}

pub(crate) fn timestep(world: &World) -> f64 {
    let control = world.resource::<MotionControl>();
    if control.animation_probe {
        1.0 / 60.0
    } else {
        f64::from(control.profile.free_fall_dt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::RenderVertex;

    #[test]
    fn pose_rotates_normals_translates_positions_and_preserves_material_coordinates() {
        let local = Draw {
            name: "synthetic".into(),
            vertices: vec![RenderVertex {
                position: [2.0, 3.0, 4.0],
                normal: [1.0, 0.0, 0.0],
                uv: [0.25, 0.75],
                color: [0.5; 4],
                specular: [0.25, 9.0],
            }],
            indices: vec![0, 0, 0],
            material: 256,
            geometry: "synthetic".into(),
        };
        let frame = CoordinateFrame {
            position: [10.0, 20.0, 30.0],
            rotation: [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]],
        };
        let draw = pose_draw(&local, &frame);
        assert_eq!(draw.vertices[0].position, [14.0, 23.0, 28.0]);
        assert_eq!(draw.vertices[0].normal, [0.0, 0.0, -1.0]);
        assert_eq!(draw.vertices[0].uv, [0.25, 0.75]);
        assert_eq!(draw.vertices[0].color, [0.5; 4]);
        assert_eq!(draw.vertices[0].specular, [0.25, 9.0]);
        assert_eq!(draw.indices, local.indices);
        assert_eq!(
            pose_draw(&local, &frame).vertices[0].position,
            draw.vertices[0].position
        );
        assert_eq!(local.vertices[0].position, [2.0, 3.0, 4.0]);
    }
}
