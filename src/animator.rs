//! RBX::Animator, RBX::AnimationTrack and the KeyframeSequence pose cache
//! (v8datamodel/AnimationTrack.cpp, Animator.cpp, KeyframeSequence.cpp).
//!
//! Recovered: cache build 009384b0 (Pose CFrame -> translation + toAxisAngle
//! vector, weight clamped to [0,1], easing disabled by its flag, tracks keyed
//! by (parent Pose name or "__Root", Pose name), length = max keyframe time),
//! pose sampling 00938650/00938852, rotation-vector interpolation 0093922e,
//! weighted blend 009389fe, track step 006e971a (time wrap 006e9856, linear
//! fade, KeyframeReached 006e98f6/006e9c24 for names other than "Keyframe",
//! non-looped end: last keyframe, fade out 0.3 s, Stopped), Play 006e9a8c,
//! Stop 006e8c10, AdjustSpeed 006e8ddc, removal test 006e9b96 and
//! Animator::onStepped 006eee6a (priority layers Core, Idle, Movement, Action).
// Negated comparisons keep the native NaN behavior of the original branches.
#![allow(clippy::neg_cmp_op_on_partial_ord)]
use crate::{g3d, rbxm::Model};

/// Animator::onStepped priority order (0174d3d8): later layers override.
pub const PRIORITY_ORDER: [u32; 4] = [1000, 0, 1, 2];

/// Cached pose, 9 floats in the native layout; easing fields are always zero.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CachedPose {
    pub translation: [f32; 3],
    pub rotation: [f32; 3],
    pub weight: f32,
}

#[derive(Clone, Debug)]
struct CachedKeyframe {
    time: f32,
    name: String,
    /// Indexed by track; None where this keyframe has no pose for it.
    poses: Vec<Option<CachedPose>>,
}

#[derive(Clone, Debug, Default)]
pub struct KeyframeSequence {
    pub looped: bool,
    pub priority: u32,
    pub length: f32,
    tracks: Vec<(String, String)>,
    keyframes: Vec<CachedKeyframe>,
}

/// Rotation-vector interpolation 0093922e: out = r1*w1 + r0*w0, taking the
/// short way around when the vectors point apart.
pub fn interpolate_rotation(r0: [f32; 3], r1: [f32; 3], w0: f32, w1: f32) -> [f32; 3] {
    // 3.1415927f and 6.2831855f in the binary.
    const PI_F: f32 = std::f32::consts::PI;
    const TAU_F: f32 = std::f32::consts::TAU;
    let (mut r0, mut r1, mut w0, mut w1) = (r0, r1, w0, w1);
    loop {
        let d = r0[2] * r1[2] + r0[1] * r1[1] + r0[0] * r1[0];
        let linear = |r0: [f32; 3], r1: [f32; 3]| -> [f32; 3] {
            std::array::from_fn(|a| r1[a] * w1 + r0[a] * w0)
        };
        if 0.0 <= d {
            return linear(r0, r1);
        }
        let l0 = (r0[2] * r0[2] + r0[1] * r0[1] + r0[0] * r0[0]).sqrt();
        let l1 = (r1[2] * r1[2] + r1[1] * r1[1] + r1[0] * r1[0]).sqrt();
        if l1 <= l0 {
            if PI_F < l0 - d / l0 {
                let k = (l0 + -TAU_F) / l0;
                let mut t: [f32; 3] = std::array::from_fn(|a| r1[a] * w1 + r0[a] * k * w0);
                let s = t[2] * t[2] + t[1] * t[1] + t[0] * t[0];
                if 9.869_605 < s {
                    let m = s.sqrt();
                    let k = (m + -TAU_F) / m;
                    t = t.map(|v| v * k);
                }
                return t;
            }
            return linear(r0, r1);
        }
        if g3d::fuzzy_zero(l1) {
            return [0.0; 3];
        }
        std::mem::swap(&mut r0, &mut r1);
        std::mem::swap(&mut w0, &mut w1);
    }
}

/// 00938852: pose at a time between keyframe poses `a` (before) and `b`.
fn interpolate(a: &CachedPose, b: &CachedPose, to_next: f32, from_previous: f32) -> CachedPose {
    if to_next <= 0.0 {
        return *b;
    }
    if !(0.0 < from_previous) {
        return *a;
    }
    // Linear easing (style 0) returns the fraction unchanged.
    let wa = to_next / (from_previous + to_next);
    let wb = 1.0 - wa;
    CachedPose {
        translation: std::array::from_fn(|i| b.translation[i] * wb + a.translation[i] * wa),
        rotation: interpolate_rotation(a.rotation, b.rotation, wa, wb),
        weight: b.weight * wb + a.weight * wa,
    }
}

/// 009389fe: blend `b` over the accumulated `a`; `b` keeps its weight.
pub fn blend(a: &CachedPose, b: &CachedPose) -> CachedPose {
    if a.weight != 0.0 || 1.0 < b.weight {
        let mut wa = 1.0 - b.weight;
        if wa <= 0.0 {
            wa = 0.0;
        }
        if a.weight <= wa {
            wa = a.weight;
        }
        let s = 1.0 / (b.weight + wa);
        let fa = wa * s;
        let fb = s * b.weight;
        let total = a.weight + b.weight;
        CachedPose {
            translation: std::array::from_fn(|i| b.translation[i] * fb + a.translation[i] * fa),
            rotation: interpolate_rotation(a.rotation, b.rotation, fa, fb),
            weight: if 1.0 < total { 1.0 } else { total },
        }
    } else {
        *b
    }
}

impl KeyframeSequence {
    /// Parse the first KeyframeSequence in a model file.
    pub fn from_model(model: &Model) -> Result<Self, String> {
        let root = model
            .roots
            .iter()
            .copied()
            .find(|&i| model.instances[i].class == "KeyframeSequence")
            .ok_or("model has no KeyframeSequence")?;
        let sequence = &model.instances[root];
        let mut result = Self {
            looped: sequence.bool("Loop").unwrap_or(false),
            priority: sequence.token("Priority").unwrap_or(1000),
            ..Default::default()
        };
        let keyframes: Vec<usize> = sequence
            .children
            .iter()
            .copied()
            .filter(|&i| model.instances[i].class == "Keyframe")
            .collect();
        // Pass 1 (009390c6/00938b9c): length and the track list.
        let mut visits: Vec<Vec<(String, String, usize)>> = Vec::new();
        for &k in &keyframes {
            let keyframe = &model.instances[k];
            let time = keyframe.float("Time").unwrap_or(0.0);
            if !(time <= result.length) {
                result.length = time;
            }
            let mut poses = Vec::new();
            fn walk(
                model: &Model,
                node: usize,
                parent: &str,
                out: &mut Vec<(String, String, usize)>,
            ) {
                for &c in &model.instances[node].children {
                    let child = &model.instances[c];
                    if child.class == "Pose" {
                        out.push((parent.to_string(), child.name().to_string(), c));
                        walk(model, c, child.name(), out);
                    }
                }
            }
            walk(model, k, "__Root", &mut poses);
            for (parent, name, _) in &poses {
                if !result.tracks.iter().any(|t| t.0 == *parent && t.1 == *name) {
                    result.tracks.push((parent.clone(), name.clone()));
                }
            }
            visits.push(poses);
        }
        // Pass 2 (00938fde/00938d3c): cached keyframes.
        for (&k, poses) in keyframes.iter().zip(visits) {
            let keyframe = &model.instances[k];
            let mut cached = CachedKeyframe {
                time: keyframe.float("Time").unwrap_or(0.0),
                name: keyframe.name().to_string(),
                poses: vec![None; result.tracks.len()],
            };
            for (parent, name, node) in poses {
                let pose = &model.instances[node];
                let frame = pose.cframe("CFrame").ok_or("Pose without CFrame")?;
                let w = pose.float("Weight").unwrap_or(1.0);
                let (axis, angle) = g3d::to_axis_angle(&frame.rotation);
                let index = result
                    .tracks
                    .iter()
                    .position(|t| t.0 == parent && t.1 == name)
                    .ok_or("missing pose track")?;
                cached.poses[index] = Some(CachedPose {
                    translation: frame.position,
                    rotation: [axis[0] * angle, axis[1] * angle, angle * axis[2]],
                    weight: if 0.0 < w { w.min(1.0) } else { 0.0 },
                });
            }
            result.keyframes.push(cached);
        }
        result.keyframes.sort_by(|a, b| {
            a.time
                .partial_cmp(&b.time)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(result)
    }
    pub fn keyframe_count(&self) -> usize {
        self.keyframes.len()
    }
    /// 00937f54: track for a joint's (Part0.Name, Part1.Name).
    pub fn track(&self, part0: &str, part1: &str) -> Option<usize> {
        self.tracks
            .iter()
            .position(|t| t.0 == part0 && t.1 == part1)
    }
    /// 00938098: number of keyframes strictly before `time`.
    fn lower_bound(&self, time: f32) -> usize {
        self.keyframes.iter().filter(|k| k.time < time).count()
    }
    /// 00938650 per joint: pose at `time`, or None without a keyframe for it.
    pub fn sample(&self, track: usize, time: f64) -> Option<CachedPose> {
        if self.keyframes.is_empty() {
            return None;
        }
        // 00938034: last keyframe at or before (float)time.
        let at = (time as f32).to_bits();
        let t = f32::from_bits(at);
        let count = self.keyframes.iter().filter(|k| !(t < k.time)).count();
        let index = count.saturating_sub(1);
        let before = (0..=index)
            .rev()
            .find(|&i| self.keyframes[i].poses[track].is_some())?;
        let after = (index + 1..self.keyframes.len())
            .find(|&i| self.keyframes[i].poses[track].is_some())
            .unwrap_or(before);
        let (a, b) = (&self.keyframes[before], &self.keyframes[after]);
        Some(interpolate(
            a.poses[track].as_ref().unwrap(),
            b.poses[track].as_ref().unwrap(),
            (f64::from(b.time) - time) as f32,
            (time - f64::from(a.time)) as f32,
        ))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TrackEvent {
    KeyframeReached(String),
    Stopped,
}

#[derive(Clone, Debug)]
pub struct AnimationTrack {
    pub animation_id: String,
    sequence: KeyframeSequence,
    /// +0x64 TimePosition, +0x68 speed, +0x6c weight, +0x70 target weight,
    /// +0x74 fade time remaining, +0x7d playing.
    pub time: f32,
    pub speed: f32,
    pub weight: f32,
    pub target_weight: f32,
    pub fade: f32,
    pub playing: bool,
}

impl AnimationTrack {
    fn new(animation_id: &str, sequence: KeyframeSequence) -> Self {
        Self {
            animation_id: animation_id.into(),
            sequence,
            time: 0.0,
            speed: 1.0,
            weight: 0.0,
            target_weight: 0.0,
            fade: 0.0,
            playing: false,
        }
    }
    pub fn priority(&self) -> u32 {
        self.sequence.priority
    }
    pub fn length(&self) -> f32 {
        self.sequence.length
    }
    /// 006e9856.
    fn wrap(&self, t: f32) -> f32 {
        let length = self.sequence.length;
        if 0.0 < length {
            if !self.sequence.looped {
                return if 0.0 < t {
                    if length <= t { length } else { t }
                } else {
                    0.0
                };
            }
            return (if t < 0.0 { length } else { 0.0 }) + t % length;
        }
        t
    }
    /// 006e98f6: fire keyframes passed between the old and new time.
    fn keyframe_events(&self, old: f32, new: f32, events: &mut Vec<TrackEvent>) {
        let n = self.sequence.keyframes.len();
        if n == 0 {
            return;
        }
        let i0 = self.sequence.lower_bound(old);
        let i1 = self.sequence.lower_bound(new);
        let mut fire = |i: usize| {
            let name = &self.sequence.keyframes[i % n].name;
            if name != "Keyframe" {
                events.push(TrackEvent::KeyframeReached(name.clone()));
            }
        };
        if 0.0 <= self.speed {
            let wrap = if i1 < i0 && self.sequence.looped {
                n
            } else {
                0
            };
            for i in i0..i1 + wrap {
                fire(i);
            }
        } else {
            let wrap = if i0 < i1 && self.sequence.looped {
                n
            } else {
                0
            };
            let mut i = wrap + i0;
            while i1 < i {
                fire(i);
                i -= 1;
            }
        }
    }
    /// 006e971a, accumulating this track into `layer` per joint track index.
    fn step(
        &mut self,
        joints: &[Option<usize>],
        layer: &mut [CachedPose],
        dt: f32,
    ) -> Vec<TrackEvent> {
        let old = self.time;
        self.time = self.wrap(self.speed * dt + old);
        let previous_fade = self.fade;
        let mut remaining = previous_fade - dt;
        if remaining <= 0.0 {
            remaining = 0.0;
        }
        self.fade = remaining;
        let mut weight = self.target_weight;
        if 0.0 < remaining {
            weight = weight + (self.weight - weight) * (remaining / previous_fade);
        }
        self.weight = weight;
        if 0.0 < weight {
            for (slot, joint) in layer.iter_mut().zip(joints) {
                let Some(track) = *joint else { continue };
                if let Some(mut pose) = self.sequence.sample(track, f64::from(self.time)) {
                    pose.weight *= weight;
                    *slot = blend(slot, &pose);
                }
            }
        }
        let mut events = Vec::new();
        if self.playing {
            self.keyframe_events(old, self.time, &mut events);
            let length = self.sequence.length;
            if !self.sequence.looped && 0.0 < length {
                let ended = if 0.0 <= self.speed {
                    !(self.time < length)
                } else {
                    !(0.0 < self.time)
                };
                if ended {
                    let n = self.sequence.keyframes.len();
                    if n > 0 {
                        let last = if 0.0 <= self.speed { n - 1 } else { 0 };
                        let name = &self.sequence.keyframes[last].name;
                        if name != "Keyframe" {
                            events.push(TrackEvent::KeyframeReached(name.clone()));
                        }
                    }
                    self.target_weight = 0.0;
                    self.fade = 0.3;
                    self.playing = false;
                    events.push(TrackEvent::Stopped);
                }
            }
        }
        events
    }
}

pub type TrackId = usize;

/// Animator (the Humanoid's): active track list and joint list.
#[derive(Clone, Debug, Default)]
pub struct Animator {
    pub tracks: Vec<AnimationTrack>,
    /// +0x80 std::list of tracks to step, in insertion order.
    active: Vec<TrackId>,
    /// (Part0.Name, Part1.Name) per animatable joint.
    joints: Vec<(String, String)>,
}

pub struct JointPose {
    pub joint: usize,
    pub pose: CachedPose,
}

impl Animator {
    pub fn new(joints: Vec<(String, String)>) -> Self {
        Self {
            joints,
            ..Default::default()
        }
    }
    /// Humanoid:LoadAnimation -> a new AnimationTrack.
    pub fn load(&mut self, animation_id: &str, sequence: KeyframeSequence) -> TrackId {
        self.tracks
            .push(AnimationTrack::new(animation_id, sequence));
        self.tracks.len() - 1
    }
    /// AnimationTrack:Play(fadeTime, weight, speed) 006e9a8c.
    pub fn play(&mut self, track: TrackId, fade: f32, weight: f32, speed: f32) {
        if !self.active.contains(&track) {
            self.active.push(track);
        }
        let t = &mut self.tracks[track];
        t.time = 0.0;
        t.speed = speed;
        t.target_weight = weight;
        t.fade = fade;
        t.playing = true;
    }
    /// AnimationTrack:Stop(fadeTime) 006e8c10 (fires Stopped; no handler here).
    pub fn stop(&mut self, track: TrackId, fade: f32) {
        let t = &mut self.tracks[track];
        t.target_weight = 0.0;
        t.fade = fade;
        t.playing = false;
    }
    /// AnimationTrack:AdjustSpeed 006e8ddc.
    pub fn adjust_speed(&mut self, track: TrackId, speed: f32) {
        self.tracks[track].speed = speed;
    }
    pub fn active_tracks(&self) -> &[TrackId] {
        &self.active
    }
    /// Animator::onStepped 006eee6a. `handler` receives track events as they
    /// fire (Lua signal handlers run synchronously) and may play/stop tracks.
    pub fn on_stepped(
        &mut self,
        dt: f32,
        handler: &mut dyn FnMut(&mut Animator, TrackId, TrackEvent),
    ) -> Vec<JointPose> {
        if self.joints.is_empty() {
            return Vec::new();
        }
        // 006e9b96: faded out and stopped.
        let tracks = &self.tracks;
        self.active.retain(|&t| {
            let track = &tracks[t];
            !(track.fade <= 0.0 && g3d::fuzzy_zero(track.target_weight))
        });
        let mut result = vec![CachedPose::default(); self.joints.len()];
        for priority in PRIORITY_ORDER {
            if !self
                .active
                .iter()
                .any(|&t| self.tracks[t].priority() == priority)
            {
                continue;
            }
            let mut layer = vec![CachedPose::default(); self.joints.len()];
            let mut i = 0;
            while i < self.active.len() {
                let id = self.active[i];
                i += 1;
                if self.tracks[id].priority() != priority {
                    continue;
                }
                let joints: Vec<Option<usize>> = self
                    .joints
                    .iter()
                    .map(|(p0, p1)| self.tracks[id].sequence.track(p0, p1))
                    .collect();
                let events = self.tracks[id].step(&joints, &mut layer, dt);
                for event in events {
                    handler(self, id, event);
                }
            }
            for (slot, pose) in result.iter_mut().zip(&layer) {
                *slot = blend(slot, pose);
            }
        }
        result
            .into_iter()
            .enumerate()
            .map(|(joint, pose)| JointPose { joint, pose })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blend_over_empty_copies_and_caps_weight() {
        let empty = CachedPose::default();
        let b = CachedPose {
            translation: [1.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.5],
            weight: 0.4,
        };
        assert_eq!(blend(&empty, &b), b);
        let full = CachedPose {
            weight: 1.0,
            ..Default::default()
        };
        let mixed = blend(&full, &b);
        assert!((mixed.weight - 1.0).abs() < 1e-6);
        assert!((mixed.translation[0] - 0.4).abs() < 1e-6);
    }
    #[test]
    fn rotation_interpolation_takes_short_way() {
        let r = interpolate_rotation([0.0, 0.0, 3.0], [0.0, 0.0, -3.0], 0.5, 0.5);
        // Halfway between +3 and -3 rad about Z the short way is near pi.
        assert!(r[2].abs() > 3.0, "{r:?}");
    }
}
