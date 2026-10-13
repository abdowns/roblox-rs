//! Bundled 2016 PlayerScripts camera (characterCameraScript.rbxmx): RootCamera +
//! ClassicCamera (desktop default), PopperCam (DevCameraOcclusionMode Zoom) and
//! TransparencyController. Constants are parsed from the original Lua at runtime.
//! Lua numbers are doubles; Vector3/CFrame components are floats.
use crate::{bundle::Bundle, scene::CoordinateFrame};

#[derive(Clone, Debug, PartialEq)]
pub struct CameraProfile {
    pub head_offset: [f32; 3],
    pub spawn_zoom: f64,
    pub default_zoom: f64,
    pub first_person_zoom: f64,
    pub minimum_render_zoom: f64,
    pub look_drop: f32,
    pub pitch_limit: f64,
    pub mouse_sensitivity: [f64; 2],
    pub mouse_reference: [f64; 2],
    pub key_turn_degrees: f64,
    pub key_turn_max_delta: f64,
    pub wheel_zoom: f64,
    pub key_zoom: f64,
    pub pop_restore: f64,
    pub near_clip_offset: f64,
    pub transparency_rate: f64,
    /// Player CameraMinZoomDistance/MaxZoomDistance setter bounds 00b8a9f8/00b8a99a.
    pub zoom_limits: [f64; 2],
    /// Explicit: Camera.FieldOfView default not located; setter clamps 1..120.
    pub field_of_view_degrees: f64,
}

/// First numeric literal directly after any occurrence of `prefix`.
fn number_after(source: &str, prefix: &str) -> Result<f64, String> {
    for (at, _) in source.match_indices(prefix) {
        let tail = source[at + prefix.len()..].trim_start();
        let end = tail
            .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
            .unwrap_or(tail.len());
        if let Ok(value) = tail[..end].parse::<f64>() {
            return Ok(value);
        }
    }
    Err(format!("missing camera literal after {prefix:?}"))
}

impl CameraProfile {
    pub fn load(bundle: &Bundle) -> Result<Self, String> {
        let bytes = bundle.read(
            "Contents/Resources/content/fonts/characterCameraScript.rbxmx",
            4 * 1024 * 1024,
        )?;
        let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
        let doc = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
        let module = |name: &str| -> Result<String, String> {
            doc.descendants()
                .filter(|n| n.has_tag_name("Item"))
                .find(|item| {
                    item.children()
                        .find(|n| n.has_tag_name("Properties"))
                        .is_some_and(|p| {
                            p.children().any(|n| {
                                n.attribute("name") == Some("Name") && n.text() == Some(name)
                            })
                        })
                })
                .and_then(|item| {
                    item.children()
                        .find(|n| n.has_tag_name("Properties"))?
                        .children()
                        .find(|n| n.attribute("name") == Some("Source"))?
                        .text()
                        .map(str::to_owned)
                })
                .ok_or_else(|| format!("missing camera module {name}"))
        };
        let root = module("RootCamera")?;
        let classic = module("ClassicCamera")?;
        let popper = module("PopperCam")?;
        let transparency = module("TransparencyController")?;
        for contract in [
            "local HEAD_OFFSET = Vector3.new(0, 1.5, 0)",
            "local MIN_Y = math.rad(-80)",
            "local MAX_Y = math.rad(80)",
            "local MOUSE_SENSITIVITY = Vector2.new(math.pi*4, math.pi*1.9)",
            "local xTheta = (translationVector.x / 1920)",
            "local yTheta = (translationVector.y / 1200)",
            "this:ZoomCamera(12.5)",
            "Vector3.new(0,0.23,0)",
            "isFirstPerson = self:GetCameraZoom() < 2",
            "this:ZoomCameraBy(clamp(-1, 1, -input.Position.Z) * 1.4)",
            "this:ZoomCameraBy(-5)",
            "local accel = direction * math.max(1, (p / 3.3) + 0.5)",
            "zoom = rk4Integrator(zoom, zoomScale, 1)",
            "clamp(player.CameraMinZoomDistance, player.CameraMaxZoomDistance, 10)",
        ] {
            if !root.contains(contract) {
                return Err(format!("unsupported RootCamera contract {contract}"));
            }
        }
        for contract in [
            "if zoom < 0.5 then",
            "local delta = math.min(0.1, now - lastUpdate)",
            "angle = angle + (self.TurningLeft and -120 or 0)",
            "camera.CoordinateFrame = CFrame.new(camera.Focus.p - (zoom * newLookVector), camera.Focus.p)",
        ] {
            if !classic.contains(contract) {
                return Err(format!("unsupported ClassicCamera contract {contract}"));
            }
        }
        if !transparency.contains("transparency = (7 - distance) / 5") {
            return Err("unsupported TransparencyController contract".into());
        }
        Ok(Self {
            head_offset: [
                0.0,
                number_after(&root, "local HEAD_OFFSET = Vector3.new(0,")? as f32,
                0.0,
            ],
            spawn_zoom: number_after(
                root.split_once("local function OnCharacterAdded(newCharacter)")
                    .ok_or("missing OnCharacterAdded")?
                    .1,
                "this:ZoomCamera(",
            )?,
            default_zoom: 10.0,
            first_person_zoom: number_after(&root, "isFirstPerson = self:GetCameraZoom() <")?,
            minimum_render_zoom: number_after(&classic, "if zoom <")?,
            look_drop: number_after(&root, "lookVector - Vector3.new(0,")? as f32,
            pitch_limit: number_after(&root, "local MAX_Y = math.rad(")?.to_radians(),
            mouse_sensitivity: [std::f64::consts::PI * 4.0, std::f64::consts::PI * 1.9],
            mouse_reference: [1920.0, 1200.0],
            key_turn_degrees: number_after(&classic, "self.TurningRight and")?,
            key_turn_max_delta: number_after(&classic, "local delta = math.min(")?,
            wheel_zoom: number_after(&root, "-input.Position.Z) *")?,
            key_zoom: number_after(&root, "this:ZoomCameraBy(-")?,
            pop_restore: number_after(&popper, "local POP_RESTORE_RATE =")?,
            near_clip_offset: number_after(&popper, "local NEAR_CLIP_PLANE_OFFSET =")?,
            transparency_rate: number_after(&transparency, "local MAX_TWEEN_RATE =")?,
            zoom_limits: [0.5, 400.0],
            field_of_view_degrees: 70.0,
        })
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}
fn unit(v: [f32; 3]) -> [f32; 3] {
    let m = length(v);
    v.map(|x| x / m)
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn mul(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
}
/// CFrame.lookVector: -Z column.
pub fn look_vector(frame: &CoordinateFrame) -> [f32; 3] {
    [
        -frame.rotation[0][2],
        -frame.rotation[1][2],
        -frame.rotation[2][2],
    ]
}
/// CFrame.new(eye, target): G3D CoordinateFrame::lookAt with world up.
pub fn look_at(eye: [f32; 3], target: [f32; 3]) -> CoordinateFrame {
    let look = unit(sub(target, eye));
    let mut up = [0.0f32, 1.0, 0.0];
    if dot(look, up).abs() > 0.99 {
        up = [1.0, 0.0, 0.0];
        if dot(look, up).abs() > 0.99 {
            up = [0.0, 1.0, 0.0];
        }
    }
    let d = dot(look, up);
    up = unit(std::array::from_fn(|i| up[i] - look[i] * d));
    let z = look.map(|v| -v);
    let x = unit(cross(up, z));
    let y = cross(z, x);
    CoordinateFrame {
        position: eye,
        rotation: std::array::from_fn(|r| [x[r], y[r], z[r]]),
    }
}
fn angles_y(a: f64) -> [[f32; 3]; 3] {
    let (s, c) = (a.sin() as f32, a.cos() as f32);
    [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]]
}
fn angles_x(a: f64) -> [[f32; 3]; 3] {
    let (s, c) = (a.sin() as f32, a.cos() as f32);
    [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]
}
/// RootCamera:RotateVector.
pub fn rotate_vector(start: [f32; 3], rotate: [f64; 2]) -> [f32; 3] {
    let start_frame = look_at([0.0; 3], start);
    let r = mul(
        mul(angles_y(-rotate[0]), start_frame.rotation),
        angles_x(-rotate[1]),
    );
    [-r[0][2], -r[1][2], -r[2][2]]
}
/// findAngleBetweenXZVectors(vec2, vec1).
fn xz_angle(vec2: [f32; 3], vec1: [f32; 3]) -> f64 {
    f64::from(vec1[0] * vec2[2] - vec1[2] * vec2[0])
        .atan2(f64::from(vec1[0] * vec2[0] + vec1[2] * vec2[2]))
}
/// RootCamera rk4Integrator (legacy UserBetterInertialScrolling off).
fn rk4(position: f64, velocity: f64, t: f64) -> f64 {
    let direction = if velocity < 0.0 { -1.0 } else { 1.0 };
    let a = |p: f64| direction * (p / 3.3 + 0.5).max(1.0);
    let (p1, v1) = (position, velocity);
    let a1 = a(p1);
    let p2 = p1 + v1 * (t / 2.0);
    let v2 = v1 + a1 * (t / 2.0);
    let a2 = a(p2);
    let p3 = p1 + v2 * (t / 2.0);
    let v3 = v1 + a2 * (t / 2.0);
    let a3 = a(p3);
    let v4 = v1 + a3 * t;
    position + (v1 + 2.0 * v2 + 2.0 * v3 + v4) * (t / 6.0)
}

/// Fixture world query used by PopperCam: first hit on the segment, or None.
pub type CameraRaycast<'a> = &'a dyn Fn([f32; 3], [f32; 3]) -> Option<[f32; 3]>;

#[derive(Clone, Debug)]
pub struct ClassicCamera {
    pub profile: CameraProfile,
    pub frame: CoordinateFrame,
    pub focus: [f32; 3],
    rotate_input: [f64; 2],
    current_zoom: Option<f64>,
    pub first_person: bool,
    pub turning_left: bool,
    pub turning_right: bool,
    pub panning: bool,
    last_update: f64,
    last_user_pan: f64,
    last_pop: f64,
    last_zoom_level: f64,
    pub transparency: Option<f64>,
    transparency_update: f64,
    pending_spawn_look: bool,
}

impl ClassicCamera {
    /// `initial` is workspace.CurrentCamera before the script runs.
    pub fn new(profile: CameraProfile, initial: CoordinateFrame, now: f64) -> Self {
        let mut camera = Self {
            focus: initial.position,
            frame: initial,
            profile,
            rotate_input: [0.0; 2],
            current_zoom: None,
            first_person: false,
            turning_left: false,
            turning_right: false,
            panning: false,
            last_update: now,
            last_user_pan: now,
            last_pop: 0.0,
            last_zoom_level: 0.0,
            transparency: None,
            transparency_update: now,
            pending_spawn_look: false,
        };
        // OnCharacterAdded: ZoomCamera(12.5), then one wait() before the look.
        camera.zoom_camera(camera.profile.spawn_zoom);
        camera.pending_spawn_look = true;
        camera
    }
    pub fn zoom(&mut self) -> f64 {
        *self.current_zoom.get_or_insert(
            self.profile
                .default_zoom
                .clamp(self.profile.zoom_limits[0], self.profile.zoom_limits[1]),
        )
    }
    pub fn zoom_camera(&mut self, desired: f64) -> f64 {
        let [low, high] = self.profile.zoom_limits;
        self.current_zoom = Some(if low <= high {
            desired.max(low).min(high)
        } else {
            desired
        });
        self.first_person = self.zoom() < self.profile.first_person_zoom;
        self.zoom()
    }
    fn actual_zoom(&self) -> f64 {
        f64::from(length(sub(self.frame.position, self.focus)))
    }
    pub fn zoom_by(&mut self, scale: f64) -> f64 {
        let zoom = rk4(self.actual_zoom(), scale, 1.0);
        self.zoom_camera(zoom)
    }
    pub fn mouse_wheel(&mut self, notches: f64) {
        self.zoom_by((-notches).clamp(-1.0, 1.0) * self.profile.wheel_zoom);
    }
    pub fn zoom_key(&mut self, zoom_in: bool) {
        let step = self.profile.key_zoom;
        self.zoom_by(if zoom_in { -step } else { step });
    }
    /// Right-mouse drag: MouseTranslationToAngle * MOUSE_SENSITIVITY.
    pub fn mouse_pan(&mut self, delta: [f64; 2]) {
        for (a, d) in delta.iter().enumerate() {
            self.rotate_input[a] +=
                d / self.profile.mouse_reference[a] * self.profile.mouse_sensitivity[a];
        }
    }
    /// RootCamera:RotateCamera with the ±80° pitch clamp.
    fn rotate_camera(&self, start: [f32; 3], rotate: [f64; 2]) -> [f32; 3] {
        let vertical = f64::from(start[1]).asin();
        let y = rotate[1].clamp(
            -self.profile.pitch_limit + vertical,
            self.profile.pitch_limit + vertical,
        );
        rotate_vector(start, [rotate[0], y])
    }
    /// RenderStep "cameraRenderUpdate": ClassicCamera:Update, PopperCam:Update,
    /// TransparencyController:Update. `torso` is humanoid.Torso's render CFrame.
    pub fn update(
        &mut self,
        now: f64,
        torso: &CoordinateFrame,
        viewport: [f64; 2],
        raycast: CameraRaycast,
    ) {
        if self.pending_spawn_look {
            // setLookBehindCharacter.
            let look = torso.rotation.map(|r| r[2]).map(|v| -v);
            let desired = unit([look[0], look[1] - self.profile.look_drop, look[2]]);
            let current = look_vector(&self.frame);
            let mut horizontal = xz_angle(desired, current);
            let mut vertical = f64::from(current[1]).asin() - f64::from(desired[1]).asin();
            if !horizontal.is_finite() {
                horizontal = 0.0;
            }
            if !vertical.is_finite() {
                vertical = 0.0;
            }
            self.rotate_input = [horizontal, vertical];
            self.pending_spawn_look = false;
        }
        let elapsed = now - self.last_update;
        let delta = self.profile.key_turn_max_delta.min(elapsed);
        let mut panning = self.panning;
        let mut angle = 0.0;
        if self.turning_left {
            angle -= self.profile.key_turn_degrees;
        }
        if self.turning_right {
            angle += self.profile.key_turn_degrees;
        }
        if angle != 0.0 {
            self.rotate_input[0] += (angle * delta).to_radians();
            panning = true;
        }
        if panning {
            self.last_user_pan = now;
        }
        // GetSubjectPosition: Torso CFrame.p + vectorToWorldSpace(HEAD_OFFSET).
        let offset = self.profile.head_offset;
        let subject: [f32; 3] = std::array::from_fn(|r| {
            torso.position[r]
                + (torso.rotation[r][0] * offset[0]
                    + torso.rotation[r][1] * offset[1]
                    + torso.rotation[r][2] * offset[2])
        });
        let zoom = self.zoom().max(self.profile.minimum_render_zoom);
        let look = self.rotate_camera(look_vector(&self.frame), self.rotate_input);
        self.rotate_input = [0.0; 2];
        self.focus = subject;
        let eye = std::array::from_fn(|i| subject[i] - zoom as f32 * look[i]);
        self.frame = look_at(eye, subject);
        self.pop(viewport, raycast);
        self.update_transparency(now);
        self.last_update = now;
    }
    /// PopperCam:Update with the five near-plane screen points.
    fn pop(&mut self, viewport: [f64; 2], raycast: CameraRaycast) {
        let focus = self.focus;
        let frame = self.frame.clone();
        let look = look_vector(&frame);
        let near = self.profile.near_clip_offset;
        let front: [f32; 3] = std::array::from_fn(|i| frame.position[i] + look[i] * near as f32);
        let plane =
            viewport[1] / (2.0 * (self.profile.field_of_view_degrees.to_radians() / 2.0).tan());
        let mut largest = 0.0f64;
        for scale in [[0.5, 0.5], [0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
            let point = [viewport[0] * scale[0], viewport[1] * scale[1]];
            let local = [
                (point[0] - viewport[0] / 2.0) as f32,
                (viewport[1] / 2.0 - point[1]) as f32,
                -plane as f32,
            ];
            let world = unit(std::array::from_fn(|r| {
                frame.rotation[r][0] * local[0]
                    + frame.rotation[r][1] * local[1]
                    + frame.rotation[r][2] * local[2]
            }));
            let theta = f64::from(dot(world, look)).min(1.0).acos();
            let push = near / (std::f64::consts::FRAC_PI_2 - theta).sin();
            let clip: [f32; 3] =
                std::array::from_fn(|i| frame.position[i] + world[i] * push as f32);
            let start: [f32; 3] = std::array::from_fn(|i| focus[i] + (clip[i] - front[i]));
            let hit = raycast(start, clip).unwrap_or(clip);
            largest = largest.max(f64::from(length(sub(hit, clip))));
        }
        let zoom_level = f64::from(length(sub(frame.position, focus)));
        if (zoom_level - self.last_zoom_level).abs() > 0.001 {
            self.last_pop = 0.0;
        }
        let pop = largest.max(self.last_pop);
        if pop > 0.0 {
            self.frame.position = std::array::from_fn(|i| frame.position[i] + look[i] * pop as f32);
            self.last_pop = (pop - self.profile.pop_restore).max(0.0);
        }
        self.last_zoom_level = zoom_level;
    }
    fn update_transparency(&mut self, now: f64) {
        let distance = f64::from(length(sub(self.focus, self.frame.position)));
        let mut transparency = (7.0 - distance) / 5.0;
        if transparency < 0.5 {
            transparency = 0.0;
        }
        if let Some(last) = self.transparency {
            let mut change = transparency - last;
            if transparency < 1.0 && last < 0.95 {
                let max = self.profile.transparency_rate * (now - self.transparency_update);
                change = change.clamp(-max, max);
            }
            transparency = last + change;
        }
        self.transparency = Some(((transparency * 100.0 + 0.5).floor() / 100.0).clamp(0.0, 1.0));
        self.transparency_update = now;
    }
}

/// Headless check of the ported camera driving the walking fixture at 60 Hz.
pub fn audit(bundle: &Bundle) -> Result<String, String> {
    use std::fmt::Write;
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let scene =
        crate::scene::Scene::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let rig = crate::rig::R6Rig::character(bundle, &scene)?;
    let mut fixture = crate::walking::WalkingFixture::new(bundle, &scene, &rig)?;
    let profile = CameraProfile::load(bundle)?;
    let mut report = format!("camera_profile {profile:?}\n");
    // Explicit pre-script workspace camera (same as viewport-walking).
    let initial = look_at([-8.0, 22.0, 35.0], [0.0, 17.5, 22.5]);
    let mut camera = ClassicCamera::new(profile.clone(), initial, 0.0);
    let viewport = [1920.0, 1440.0];
    let mut frame = 0u32;
    let mut run = |camera: &mut ClassicCamera,
                   fixture: &mut crate::walking::WalkingFixture,
                   input: [f32; 3],
                   frames: u32|
     -> Result<(), String> {
        for _ in 0..frames {
            frame += 1;
            let torso = fixture.torso_frame(&rig)?;
            camera.update(f64::from(frame) / 60.0, &torso, viewport, &|a, b| {
                fixture.camera_raycast(a, b)
            });
            fixture.set_camera(Some(camera.frame.clone()));
            for _ in 0..4 {
                fixture.step(&rig, input)?;
            }
        }
        Ok(())
    };
    let torso = fixture.torso_frame(&rig)?;
    run(&mut camera, &mut fixture, [0.0; 3], 1)?;
    let distance = length(sub(camera.frame.position, camera.focus));
    let focus_error = length(sub(
        camera.focus,
        [
            torso.position[0],
            torso.position[1] + 1.5,
            torso.position[2],
        ],
    ));
    let look = look_vector(&camera.frame);
    let expected = unit([0.0, -0.23, 1.0]);
    writeln!(report, "camera_spawn eye={:?} focus={:?} distance={distance} look={look:?} expected_look={expected:?}", camera.frame.position, camera.focus).unwrap();
    if (distance - 12.5).abs() > 1e-4 || focus_error > 1e-5 || length(sub(look, expected)) > 1e-4 {
        return Err(format!(
            "camera spawn framing failed focus_error={focus_error}"
        ));
    }
    // W is camera-forward: the character walks along the camera's ground look.
    let start = fixture.world.body.frame.position;
    run(&mut camera, &mut fixture, [0.0, 0.0, -1.0], 60)?;
    let moved = sub(fixture.world.body.frame.position, start);
    let distance = length(sub(camera.frame.position, camera.focus));
    writeln!(
        report,
        "camera_relative_walk moved={moved:?} camera_distance={distance}"
    )
    .unwrap();
    if moved[2] < 14.0 || moved[0].abs() > 0.05 || (distance - 12.5).abs() > 1e-3 {
        return Err(format!("camera-relative walking failed: {report}"));
    }
    // Quarter of the 1920 reference width is a half turn (4π sensitivity).
    let before = look_vector(&camera.frame);
    camera.mouse_pan([480.0, 0.0]);
    run(&mut camera, &mut fixture, [0.0; 3], 1)?;
    let after = look_vector(&camera.frame);
    let turned = f64::from(before[0] * after[0] + before[2] * after[2])
        / f64::from(
            (before[0].powi(2) + before[2].powi(2)).sqrt()
                * (after[0].powi(2) + after[2].powi(2)).sqrt(),
        );
    camera.mouse_pan([0.0, 100000.0]);
    run(&mut camera, &mut fixture, [0.0; 3], 1)?;
    let pitch = f64::from(look_vector(&camera.frame)[1]).asin().to_degrees();
    writeln!(
        report,
        "camera_pan half_turn_cos={turned} clamped_pitch_degrees={pitch}"
    )
    .unwrap();
    if turned > -0.9999 || (pitch + 80.0).abs() > 0.01 {
        return Err(format!("camera pan or pitch clamp failed: {report}"));
    }
    // Pan back up from -80° to -13° (positive y input pitches down).
    let back =
        -(80.0f64 - 13.0).to_radians() * profile.mouse_reference[1] / profile.mouse_sensitivity[1];
    camera.mouse_pan([0.0, back]);
    run(&mut camera, &mut fixture, [0.0; 3], 1)?;
    let actual = f64::from(length(sub(camera.frame.position, camera.focus)));
    camera.mouse_wheel(1.0);
    let zoom_after = camera.zoom();
    writeln!(
        report,
        "camera_wheel actual_before={actual} zoom_after={zoom_after} pitch_degrees={}",
        f64::from(look_vector(&camera.frame)[1]).asin().to_degrees()
    )
    .unwrap();
    if (zoom_after - rk4(actual, -1.4, 1.0)).abs() > 1e-6 || (actual - 12.5).abs() > 1e-3 {
        return Err(format!("camera wheel zoom failed: {report}"));
    }
    // TransparencyController fades the character once the camera is near.
    let mut transparency = Vec::new();
    camera.zoom_camera(3.5);
    for _ in 0..20 {
        run(&mut camera, &mut fixture, [0.0; 3], 1)?;
        transparency.push(camera.transparency.unwrap_or(0.0));
    }
    writeln!(
        report,
        "camera_transparency samples={transparency:?} first_person={}",
        camera.first_person
    )
    .unwrap();
    if transparency.last().copied().unwrap_or(0.0) < 0.5
        || transparency
            .windows(2)
            .any(|w| w[1] - w[0] > 2.8 / 60.0 + 0.011)
    {
        return Err(format!("camera transparency fade failed: {report}"));
    }
    camera.zoom_camera(12.5);
    run(&mut camera, &mut fixture, [0.0; 3], 30)?;
    // PopperCam: walk to the wall, then look back so the wall is behind the eye.
    camera.zoom_camera(12.5);
    camera.mouse_pan([-480.0, 0.0]);
    run(&mut camera, &mut fixture, [0.0; 3], 1)?;
    let left_of_view = [-1.0, 0.0, 0.0];
    run(&mut camera, &mut fixture, left_of_view, 120)?;
    let wall = fixture.world.statics.wall_x;
    camera.mouse_pan([480.0 * 0.5, 0.0]);
    run(&mut camera, &mut fixture, [0.0; 3], 2)?;
    let eye = camera.frame.position;
    let look = look_vector(&camera.frame);
    // PopperCam moves the near-clip plane (0.5 ahead of the eye) onto the hit.
    let near: [f32; 3] =
        std::array::from_fn(|i| eye[i] + look[i] * profile.near_clip_offset as f32);
    let unpopped = camera.focus[0] - camera.zoom() as f32 * look[0];
    writeln!(report, "camera_popper eye={eye:?} near_plane={near:?} unpopped_eye_x={unpopped} wall_x={wall} character_x={}", fixture.world.body.frame.position[0]).unwrap();
    if near[0] > wall + 1e-3
        || unpopped < wall + 1.0
        || fixture.world.body.frame.position[0] < wall - 1.0
    {
        return Err(format!(
            "PopperCam did not pull the camera in front of the wall: {report}"
        ));
    }
    writeln!(report, "camera_pass classic=true popper=true transparency_fade=true camera_relative_move=true fov_degrees={} fov_default=explicit_not_located first_person_rotation=not_implemented native_parity=not_tested", profile.field_of_view_degrees).unwrap();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotate_vector_and_look_at_round_trip() {
        let f = look_at([0.0, 0.0, 0.0], [0.0, 0.0, -1.0]);
        assert_eq!(look_vector(&f), [0.0, 0.0, -1.0]);
        let yawed = rotate_vector([0.0, 0.0, -1.0], [std::f64::consts::FRAC_PI_2, 0.0]);
        // CFrame.Angles(0, -x, 0): positive x turns -Z toward +X.
        assert!((yawed[0] - 1.0).abs() < 1e-6, "{yawed:?}");
        let pitched = rotate_vector([0.0, 0.0, -1.0], [0.0, 0.3]);
        assert!((pitched[1] + 0.3f32.sin()).abs() < 1e-6, "{pitched:?}");
        assert!(
            (xz_angle([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]) + std::f64::consts::FRAC_PI_2).abs() < 1e-9
        );
    }
    #[test]
    fn rk4_zoom_matches_lua_integrator() {
        // From 12.5, one wheel notch in (scale -1.4): acceleration -max(1, p/3.3+0.5).
        let z = rk4(12.5, -1.4, 1.0);
        assert!((z - 9.0806).abs() < 0.001, "{z}");
        // Zero velocity accelerates outward (direction +1), as in the Lua.
        assert!(rk4(10.0, 0.0, 1.0) > 10.0);
    }
}
