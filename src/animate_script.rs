//! The R6 "Animate" LocalScript the 0.255 loader selects
//! (content/fonts/humanoidAnimateLocalKeyframe2.rbxm), ported as a bounded
//! state machine over the native Animator. Its animation table, transition
//! times and speed scales are parsed from the shipped Lua source; the clips
//! are the remote KeyframeSequences it names, read from the offline asset
//! cache RobloxStudio/assets/<id> (tools/fetch_animations.py). A clip that is
//! not cached loads as an empty sequence (no poses), as a failed asset load
//! would natively.
use crate::{
    animator::{Animator, KeyframeSequence, TrackEvent, TrackId},
    bundle::Bundle,
    humanoid::HumanoidEvent,
    rbxm::{Model, Property},
    rig::R6Rig,
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt::Write};

pub const SCRIPT_PATH: &str = "Contents/Resources/content/fonts/humanoidAnimateLocalKeyframe2.rbxm";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptPose {
    Standing,
    Running,
    Jumping,
    FreeFall,
    Climbing,
    Seated,
    Dead,
    GettingUp,
    FallingDown,
    PlatformStanding,
}

#[derive(Clone, Debug)]
enum Token {
    Name(String),
    Str(String),
    Number(f64),
    Symbol(char),
}

fn tokens(source: &str) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let mut chars = source.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '-' {
            chars.next();
            if chars.peek() == Some(&'-') {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            } else {
                return Err("unexpected '-' in table".into());
            }
        } else if c == '"' {
            chars.next();
            let mut s = String::new();
            for c in chars.by_ref() {
                if c == '"' {
                    break;
                }
                s.push(c);
            }
            out.push(Token::Str(s));
        } else if c.is_ascii_digit() || c == '.' {
            let mut s = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() || c == '.' {
                    s.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            out.push(Token::Number(s.parse().map_err(|_| "invalid number")?));
        } else if c.is_alphabetic() || c == '_' {
            let mut s = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_alphanumeric() || c == '_' {
                    s.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            out.push(Token::Name(s));
        } else {
            out.push(Token::Symbol(c));
            chars.next();
        }
    }
    Ok(out)
}

/// The balanced `{ ... }` table text following `prefix`.
fn table<'a>(source: &'a str, prefix: &str) -> Result<&'a str, String> {
    let start = source
        .find(prefix)
        .ok_or_else(|| format!("missing Animate table {prefix:?}"))?
        + prefix.len();
    let tail = &source[start..];
    let open = tail.find('{').ok_or("missing table brace")?;
    let mut depth = 0;
    for (i, c) in tail[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(&tail[open..open + i + 1]);
                }
            }
            _ => {}
        }
    }
    Err("unbalanced Animate table".into())
}

/// `animNames`: name = { { id = "...", weight = N }, ... }.
fn animation_sets(source: &str) -> Result<BTreeMap<String, Vec<(String, u32)>>, String> {
    let t = tokens(table(source, "local animNames =")?)?;
    let mut sets = BTreeMap::new();
    let mut i = 1;
    let symbol = |i: usize, c: char| matches!(t.get(i), Some(Token::Symbol(s)) if *s == c);
    while i < t.len() {
        match &t[i] {
            Token::Name(name) => {
                if !symbol(i + 1, '=') || !symbol(i + 2, '{') {
                    return Err(format!("bad animNames entry {name}"));
                }
                i += 3;
                let mut entries = Vec::new();
                while symbol(i, '{') {
                    let (
                        Some(Token::Name(k1)),
                        Some(Token::Str(id)),
                        Some(Token::Name(k2)),
                        Some(Token::Number(w)),
                    ) = (t.get(i + 1), t.get(i + 3), t.get(i + 5), t.get(i + 7))
                    else {
                        return Err(format!("bad animNames item in {name}"));
                    };
                    if k1 != "id" || k2 != "weight" || !symbol(i + 8, '}') {
                        return Err(format!("bad animNames item in {name}"));
                    }
                    entries.push((id.clone(), *w as u32));
                    i += 9;
                    if symbol(i, ',') {
                        i += 1;
                    }
                }
                if !symbol(i, '}') {
                    return Err(format!("unterminated animNames entry {name}"));
                }
                i += 1;
                sets.insert(name.clone(), entries);
            }
            Token::Symbol(',') | Token::Symbol('}') => i += 1,
            _ => return Err("unexpected animNames token".into()),
        }
    }
    Ok(sets)
}

fn emote_names(source: &str) -> Result<BTreeMap<String, bool>, String> {
    let t = tokens(table(source, "local emoteNames =")?)?;
    let mut out = BTreeMap::new();
    for w in t.windows(3) {
        if let (Token::Name(k), Token::Symbol('='), Token::Name(v)) = (&w[0], &w[1], &w[2]) {
            out.insert(k.clone(), v == "true");
        }
    }
    Ok(out)
}

fn literal(source: &str, prefix: &str) -> Result<f64, String> {
    let tail = source
        .split_once(prefix)
        .ok_or_else(|| format!("missing Animate contract {prefix:?}"))?
        .1;
    tail.trim_start()
        .split(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
        .next()
        .unwrap_or("")
        .parse()
        .map_err(|_| format!("invalid Animate literal {prefix:?}"))
}

fn function<'a>(source: &'a str, name: &str) -> Result<&'a str, String> {
    let body = source
        .split_once(&format!("function {name}("))
        .ok_or_else(|| format!("missing Animate function {name}"))?
        .1;
    Ok(body.split_once("\nend").map_or(body, |b| b.0))
}

/// Asset cache path for an Animation.AnimationId, if it names a remote asset.
pub fn asset_path(id: &str) -> Option<String> {
    let number = id.split_once("asset/?id=")?.1;
    number
        .chars()
        .all(|c| c.is_ascii_digit())
        .then(|| format!("assets/{number}"))
}

pub struct AnimateProfile {
    pub source_sha256: String,
    pub wait_seconds: f64,
    sets: BTreeMap<String, Vec<(String, u32)>>,
    emotes: BTreeMap<String, bool>,
    sequences: BTreeMap<String, KeyframeSequence>,
    pub missing: Vec<String>,
    jump_duration: f64,
    fall_transition: f64,
    running_threshold: f64,
    walk_id: String,
    walk_speed_divisor: f64,
    climb_speed_divisor: f64,
    seated_transition: f64,
    stop_amplitude: f64,
    stop_frequency: f64,
}

impl AnimateProfile {
    pub fn load(bundle: &Bundle) -> Result<Self, String> {
        let bytes = bundle.read(SCRIPT_PATH, 1024 * 1024)?;
        let model = Model::parse(&bytes)?;
        let script = model
            .instances
            .iter()
            .find(|i| i.class == "LocalScript" && i.name() == "Animate")
            .ok_or("missing Animate LocalScript")?;
        let Some(Property::String(source)) = script.properties.get("Source") else {
            return Err("missing Animate source".into());
        };
        let source = source.as_str();
        for contract in [
            "local _, time = wait(",
            "currentAnimTrack = humanoid:LoadAnimation(anim)",
            "currentAnimTrack:Play(transitionTime)",
            "currentAnimTrack:Stop(transitionTime)",
            "currentAnimTrack.KeyframeReached:connect(keyFrameReachedFunc)",
            "if (frameName == \"End\") then",
            "math.random(1, animTable[animName].totalWeight)",
            "Humanoid.Running:connect(onRunning)",
        ] {
            if !source.contains(contract) {
                return Err(format!("unsupported Animate contract {contract}"));
            }
        }
        let sets = animation_sets(source)?;
        for name in ["idle", "walk", "jump", "fall", "climb", "sit"] {
            if sets.get(name).is_none_or(|s| s.is_empty()) {
                return Err(format!("Animate table lacks {name}"));
            }
        }
        let mut sequences = BTreeMap::new();
        let mut missing = Vec::new();
        for (id, _) in sets.values().flatten() {
            if sequences.contains_key(id) {
                continue;
            }
            let sequence = match asset_path(id).map(|p| bundle.read(&p, 16 * 1024 * 1024)) {
                Some(Ok(bytes)) => KeyframeSequence::from_model(&Model::parse(&bytes)?)?,
                _ => {
                    missing.push(id.clone());
                    KeyframeSequence::default()
                }
            };
            sequences.insert(id.clone(), sequence);
        }
        let running = function(source, "onRunning")?;
        let walk_id = running
            .split_once("AnimationId == \"")
            .and_then(|t| t.1.split_once('"'))
            .ok_or("missing walk AnimationId check")?
            .0
            .to_string();
        let stop = source
            .split_once("pose == \"Dead\"")
            .and_then(|s| s.1.split_once("stopAllAnimations()"))
            .map(|s| s.1)
            .ok_or("missing stopped-pose branch")?;
        Ok(Self {
            source_sha256: format!("{:x}", Sha256::digest(&bytes)),
            wait_seconds: literal(source, "local _, time = wait(")?,
            emotes: emote_names(source)?,
            jump_duration: literal(source, "local jumpAnimDuration =")?,
            fall_transition: literal(source, "local fallTransitionTime =")?,
            running_threshold: literal(running, "if speed>")?,
            walk_speed_divisor: literal(running, "setAnimationSpeed(speed /")?,
            climb_speed_divisor: literal(
                function(source, "onClimbing")?,
                "setAnimationSpeed(speed /",
            )?,
            seated_transition: literal(source, "playAnimation(\"sit\",")?,
            stop_amplitude: literal(stop, "amplitude =")?,
            stop_frequency: literal(stop, "frequency =")?,
            walk_id,
            sets,
            sequences,
            missing,
        })
    }
    pub fn sequence(&self, id: &str) -> Option<&KeyframeSequence> {
        self.sequences.get(id)
    }
    pub fn loaded(&self) -> usize {
        self.sequences.len() - self.missing.len()
    }
}

/// Lua 5.1 math.random(l, u) over Darwin rand() (do_rand, seed 1).
/// Not recovered: the native seed and other callers of rand() in the process.
#[derive(Clone, Debug)]
pub struct LuaRandom {
    state: u64,
}
impl Default for LuaRandom {
    fn default() -> Self {
        Self { state: 1 }
    }
}
impl LuaRandom {
    fn rand(&mut self) -> u32 {
        let mut x = self.state as i64;
        if x == 0 {
            x = 123459876;
        }
        let hi = x / 127773;
        let lo = x % 127773;
        x = 16807 * lo - 2836 * hi;
        if x < 0 {
            x += 0x7fff_ffff;
        }
        self.state = x as u64;
        (x as u64 % 0x8000_0000) as u32
    }
    pub fn range(&mut self, l: i64, u: i64) -> i64 {
        const RAND_MAX: f64 = 2147483647.0;
        let r = f64::from(self.rand() % 0x7fff_ffff) / RAND_MAX;
        (r * (u - l + 1) as f64).floor() as i64 + l
    }
}

pub struct AnimateScript {
    pub pose: ScriptPose,
    current_anim: String,
    current_instance: Option<(String, usize)>,
    current_track: Option<TrackId>,
    current_speed: f64,
    jump_anim_time: f64,
    last_tick: f64,
    rng: LuaRandom,
    /// Humanoid:LoadAnimation calls (for the log).
    pub loads: u32,
}

impl Default for AnimateScript {
    fn default() -> Self {
        Self {
            pose: ScriptPose::Standing,
            current_anim: String::new(),
            current_instance: None,
            current_track: None,
            current_speed: 1.0,
            jump_anim_time: 0.0,
            last_tick: 0.0,
            rng: LuaRandom::default(),
            loads: 0,
        }
    }
}

impl AnimateScript {
    pub fn current_animation(&self) -> &str {
        &self.current_anim
    }
    pub fn current_track(&self) -> Option<TrackId> {
        self.current_track
    }
    /// Script body up to the first wait: initialize to idle.
    pub fn start(
        &mut self,
        profile: &AnimateProfile,
        animator: &mut Animator,
    ) -> Result<(), String> {
        self.play_animation(profile, animator, "idle", 0.1)?;
        self.pose = ScriptPose::Standing;
        Ok(())
    }
    fn play_animation(
        &mut self,
        profile: &AnimateProfile,
        animator: &mut Animator,
        name: &str,
        transition: f64,
    ) -> Result<(), String> {
        let set = profile
            .sets
            .get(name)
            .ok_or_else(|| format!("unknown animation set {name}"))?;
        let total: u32 = set.iter().map(|e| e.1).sum();
        let mut roll = self.rng.range(1, i64::from(total));
        let mut index = 0;
        while roll > i64::from(set[index].1) {
            roll -= i64::from(set[index].1);
            index += 1;
        }
        let instance = (name.to_string(), index);
        if self.current_instance.as_ref() == Some(&instance) {
            return Ok(());
        }
        if let Some(track) = self.current_track {
            animator.stop(track, transition as f32);
        }
        self.current_speed = 1.0;
        let id = &set[index].0;
        let sequence = profile.sequence(id).cloned().unwrap_or_default();
        let track = animator.load(id, sequence);
        self.loads += 1;
        animator.play(track, transition as f32, 1.0, 1.0);
        self.current_track = Some(track);
        self.current_anim = name.to_string();
        self.current_instance = Some(instance);
        Ok(())
    }
    fn set_animation_speed(&mut self, animator: &mut Animator, speed: f64) {
        if speed != self.current_speed {
            self.current_speed = speed;
            if let Some(track) = self.current_track {
                animator.adjust_speed(track, speed as f32);
            }
        }
    }
    fn stop_all(&mut self, animator: &mut Animator) {
        self.current_anim.clear();
        self.current_instance = None;
        if let Some(track) = self.current_track.take() {
            animator.stop(track, 0.1);
        }
    }
    /// Humanoid signal handlers (run synchronously when the signal fires).
    pub fn on_event(
        &mut self,
        profile: &AnimateProfile,
        animator: &mut Animator,
        event: HumanoidEvent,
    ) -> Result<(), String> {
        match event {
            HumanoidEvent::Died => self.pose = ScriptPose::Dead,
            HumanoidEvent::Running(speed) => {
                let speed = f64::from(speed);
                if speed > profile.running_threshold {
                    self.play_animation(profile, animator, "walk", 0.1)?;
                    let walking = self
                        .current_track
                        .is_some_and(|t| animator.tracks[t].animation_id == profile.walk_id);
                    if walking {
                        self.set_animation_speed(animator, speed / profile.walk_speed_divisor);
                    }
                    self.pose = ScriptPose::Running;
                } else {
                    self.play_animation(profile, animator, "idle", 0.1)?;
                    self.pose = ScriptPose::Standing;
                }
            }
            HumanoidEvent::Jumping(_) => {
                self.play_animation(profile, animator, "jump", 0.1)?;
                self.jump_anim_time = profile.jump_duration;
                self.pose = ScriptPose::Jumping;
            }
            HumanoidEvent::Climbing(speed) => {
                self.play_animation(profile, animator, "climb", 0.1)?;
                self.set_animation_speed(animator, f64::from(speed) / profile.climb_speed_divisor);
                self.pose = ScriptPose::Climbing;
            }
            HumanoidEvent::GettingUp(_) => self.pose = ScriptPose::GettingUp,
            HumanoidEvent::FreeFalling(_) => {
                if self.jump_anim_time <= 0.0 {
                    self.play_animation(profile, animator, "fall", profile.fall_transition)?;
                }
                self.pose = ScriptPose::FreeFall;
            }
            HumanoidEvent::FallingDown(_) => self.pose = ScriptPose::FallingDown,
            HumanoidEvent::Seated(_) => self.pose = ScriptPose::Seated,
            HumanoidEvent::PlatformStanding(_) => self.pose = ScriptPose::PlatformStanding,
            HumanoidEvent::Swimming(speed) => {
                self.pose = if speed > 0.0 {
                    ScriptPose::Running
                } else {
                    ScriptPose::Standing
                }
            }
            HumanoidEvent::Ragdoll(_) | HumanoidEvent::Strafing(_) => {}
        }
        Ok(())
    }
    /// keyFrameReachedFunc, connected to the current track only.
    pub fn on_track_event(
        &mut self,
        profile: &AnimateProfile,
        animator: &mut Animator,
        track: TrackId,
        event: TrackEvent,
    ) -> Result<(), String> {
        if self.current_track != Some(track) {
            return Ok(());
        }
        if event == TrackEvent::KeyframeReached("End".into()) {
            let mut repeat = self.current_anim.clone();
            if profile.emotes.get(&repeat) == Some(&false) {
                repeat = "idle".into();
            }
            let speed = self.current_speed;
            self.play_animation(profile, animator, &repeat, 0.0)?;
            self.set_animation_speed(animator, speed);
        }
        Ok(())
    }
    /// `move(time)` after each `wait(0.1)` resume.
    pub fn resume(
        &mut self,
        profile: &AnimateProfile,
        animator: &mut Animator,
        rig: &mut R6Rig,
        time: f64,
    ) -> Result<(), String> {
        let delta = time - self.last_tick;
        self.last_tick = time;
        if self.jump_anim_time > 0.0 {
            self.jump_anim_time -= delta;
        }
        match self.pose {
            ScriptPose::FreeFall if self.jump_anim_time <= 0.0 => {
                self.play_animation(profile, animator, "fall", profile.fall_transition)?
            }
            ScriptPose::Seated => {
                return self.play_animation(profile, animator, "sit", profile.seated_transition);
            }
            ScriptPose::Running => self.play_animation(profile, animator, "walk", 0.1)?,
            ScriptPose::Dead
            | ScriptPose::GettingUp
            | ScriptPose::FallingDown
            | ScriptPose::PlatformStanding => {
                self.stop_all(animator);
                let desired =
                    (profile.stop_amplitude * (time * profile.stop_frequency).sin()) as f32;
                // RightShoulder, LeftShoulder, RightHip, LeftHip (climbFudge 0).
                for (joint, value) in [(1, desired), (2, desired), (3, -desired), (4, -desired)] {
                    rig.joints[joint].set_desired_angle(value)?;
                }
            }
            _ => {}
        }
        // No Tool in the character: stopToolAnimations has nothing to stop.
        Ok(())
    }
}

/// Standalone Animate + Animator playback on the native R6 rig (no physics).
pub fn audit(bundle: &Bundle) -> Result<String, String> {
    let profile = AnimateProfile::load(bundle)?;
    let mut report = String::new();
    writeln!(
        report,
        "animate_script source_sha256={} wait={} clips_loaded={} unresolved={:?}",
        profile.source_sha256,
        profile.wait_seconds,
        profile.loaded(),
        profile.missing
    )
    .unwrap();
    for (name, set) in &profile.sets {
        for (id, weight) in set {
            if let Some(s) = profile.sequence(id).filter(|s| s.keyframe_count() > 0) {
                writeln!(
                    report,
                    "animate_clip set={name} id={id} weight={weight} keyframes={} length={} looped={} priority={}",
                    s.keyframe_count(),
                    s.length,
                    s.looped,
                    s.priority
                )
                .unwrap();
            }
        }
    }
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let scene =
        crate::scene::Scene::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let mut rig = R6Rig::character(bundle, &scene)?;
    let joints = rig
        .joints
        .iter()
        .map(|j| (j.part0.clone(), j.part1.clone()))
        .collect();
    let mut animator = Animator::new(joints);
    let mut script = AnimateScript::default();
    script.start(&profile, &mut animator)?;
    // Stepped dt of one 60 Hz PhysicsJob (4 world steps).
    let dt = 4.0
        * crate::kernel::KernelProfile::load(bundle)?
            .physics
            .free_fall_dt;
    let mut keyframe_events = 0;
    let frame = |script: &mut AnimateScript,
                 animator: &mut Animator,
                 rig: &mut R6Rig,
                 events: &mut u32|
     -> Result<(), String> {
        let mut failure = None;
        let poses = animator.on_stepped(dt, &mut |a, track, event| {
            if matches!(event, TrackEvent::KeyframeReached(_)) {
                *events += 1;
            }
            if let Err(e) = script.on_track_event(&profile, a, track, event) {
                failure.get_or_insert(e);
            }
        });
        if let Some(e) = failure {
            return Err(e);
        }
        for pose in poses {
            rig.joints[pose.joint].animate(&pose.pose);
        }
        for joint in &mut rig.joints {
            joint.step()?;
        }
        Ok(())
    };
    let mut shoulder = (f32::INFINITY, f32::NEG_INFINITY);
    for _ in 0..300 {
        frame(&mut script, &mut animator, &mut rig, &mut keyframe_events)?;
        let a = rig.joints[1].current_angle;
        shoulder = (shoulder.0.min(a), shoulder.1.max(a));
    }
    writeln!(
        report,
        "animate_idle seconds=5 keyframe_events={keyframe_events} loads={} right_shoulder_z=[{}, {}] animation={}",
        script.loads,
        shoulder.0,
        shoulder.1,
        script.current_animation()
    )
    .unwrap();
    if keyframe_events < 2 || script.current_animation() != "idle" || shoulder.1 - shoulder.0 <= 0.0
    {
        return Err(format!("idle loop failed: {report}"));
    }
    script.on_event(&profile, &mut animator, HumanoidEvent::Running(16.0))?;
    let walk = script.current_track().ok_or("no walk track")?;
    for _ in 0..12 {
        frame(&mut script, &mut animator, &mut rig, &mut keyframe_events)?;
    }
    let track = &animator.tracks[walk];
    let expected_speed = (16.0f64 / profile.walk_speed_divisor) as f32;
    let sampled = {
        let sequence = profile.sequence(&profile.walk_id).ok_or("no walk clip")?;
        let index = sequence.track("Torso", "Right Arm").ok_or("no arm track")?;
        sequence
            .sample(index, f64::from(track.time))
            .ok_or("no arm sample")?
    };
    let (rotation, translation) = rig.joints[1].link_state();
    writeln!(
        report,
        "animate_walk speed={} expected_speed={expected_speed} weight={} time={} active_tracks={} arm_link_rotation={rotation:?} arm_sample_rotation={:?} arm_link_translation={translation:?}",
        track.speed,
        track.weight,
        track.time,
        animator.active_tracks().len(),
        sampled.rotation
    )
    .unwrap();
    if track.speed != expected_speed
        || track.weight != 1.0
        || animator.active_tracks().len() != 1
        || rotation != sampled.rotation
        || translation != sampled.translation
    {
        return Err(format!("walk playback failed: {report}"));
    }
    script.on_event(&profile, &mut animator, HumanoidEvent::Jumping(true))?;
    let jump = script.current_track().ok_or("no jump track")?;
    let length = animator.tracks[jump].length();
    let mut ended_at = None;
    for f in 0..120 {
        frame(&mut script, &mut animator, &mut rig, &mut keyframe_events)?;
        if ended_at.is_none() && !animator.tracks[jump].playing {
            ended_at = Some(f);
        }
    }
    let rest = rig.joints.iter().all(|j| {
        let (r, t) = j.link_state();
        r == [0.0; 3] && t == [0.0; 3]
    });
    writeln!(
        report,
        "animate_jump length={length} ended_frame={ended_at:?} active_tracks={} rest_pose_after_fade={rest}",
        animator.active_tracks().len()
    )
    .unwrap();
    if ended_at.is_none() || !animator.active_tracks().is_empty() || !rest {
        return Err(format!("jump playback failed: {report}"));
    }
    writeln!(report, "animate_pass clips=animate_keyframe2 idle_loop=true walk_speed_scale=true pose_exact=true jump_end_fade=true lua_random=darwin_rand_seed1_assumed asset_load=immediate_assumed native_pixels=not_tested").unwrap();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_animation_table_and_skips_comments() {
        let source = r#"local animNames = {
	idle = 	{ { id = "a", weight = 9 }, { id = "b", weight = 1 } },
	toolslash = { { id = "c", weight = 10 }
--				{ id = "slash.xml", weight = 10 }
			},
}
local emoteNames = { wave = false, dance1 = true}"#;
        let sets = animation_sets(source).unwrap();
        assert_eq!(sets["idle"], vec![("a".into(), 9), ("b".into(), 1)]);
        assert_eq!(sets["toolslash"], vec![("c".into(), 10)]);
        let emotes = emote_names(source).unwrap();
        assert!(!emotes["wave"]);
        assert!(emotes["dance1"]);
    }
    #[test]
    fn darwin_rand_sequence() {
        let mut r = LuaRandom::default();
        assert_eq!(r.rand(), 16807);
        assert_eq!(r.rand(), 282475249);
        let mut r = LuaRandom::default();
        assert_eq!(r.range(1, 10), 1);
    }
}
