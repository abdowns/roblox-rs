//! Joints and the spanning tree that groups Parts into clumps, assemblies and
//! mechanisms: v8world Joint.cpp, GroundStage.cpp, TreeStage.cpp and
//! util/SpanningTree.cpp.
//!
//! - Every Part gets one ground joint from GroundStage (onPrimitiveAdded
//!   00b0f1a8, reevaluate 00b0f208): an AnchorJoint when it is fixed
//!   (anchored or dragged, Primitive +0xca/+0xcb) or rigidly jointed to a
//!   fixed Part (00b0f7ac), otherwise a FreeJoint. Both have no second Part.
//! - TreeStage keeps a spanning tree over all joints (00ba3e46; 009241f4
//!   admits joint types 0..=20). Insertion (006cc3d6) finds the worst edge on
//!   the path between the two ends (006ccc72) and swaps it for the new edge
//!   when the new edge ranks better (comparator 00b216e2); the cut-off piece is
//!   re-rooted at the new edge (006cc70a, 006cc9ca, 006ccb3e). Removal
//!   (006cc580) reconnects the cut-off subtree with its best outside edge
//!   (006cc624, 006cd18c).
//! - The comparator ranks by joint type (lower first), then by the larger
//!   root priority of the joint's Parts (00b216ad, 00b63fbe: multiplier x
//!   floor(planar x 50) + 1), then by the Parts' GUIDs (006721c8), then by the
//!   joints' addresses.
//! - TreeStage's callbacks (00ba2e24 after a tree edge is linked, 00ba36a6
//!   after it is unlinked) build the Body tree: a rigid joint (Weld 1,
//!   ManualWeld 2, Snap 3) or a motor (Motor 4, Motor6D 5) makes the child
//!   Part's Body a child of its parent's Body. A rigid child's offset is the
//!   parent's joint coordinate times the inverse of the child's (00b69dd6),
//!   orthonormalized when needed. Kernel joints (6..=19) and ground joints
//!   start a new assembly; ground joints also start a new mechanism.
//!
//! A clump is a set of Parts joined by rigid joints, an assembly a set of
//! clumps joined by motors (one SimBody), a mechanism a set of assemblies
//! joined by kernel joints.
//!
//! Not modelled: the World's own address order. GUIDs are the Part creation
//! order (Guid's index counter), the address tie-break uses the joint
//! creation order, and the subtree node set of 006cc624 (a std::set of node
//! pointers) is walked in Part creation order.
use crate::{g3d, scene::CoordinateFrame};
use std::collections::BTreeSet;

/// Joint::JointType (getJointType, vtable +0x1c).
pub const ANCHOR: u32 = 0; // AnchorJoint 00b0fa30
pub const WELD: u32 = 1; // WeldJoint 00ba6508
pub const MANUAL_WELD: u32 = 2; // ManualWeldJoint 00925126
pub const SNAP: u32 = 3; // JointInstance.cpp:314
pub const MOTOR: u32 = 4; // MotorJoint 00b3858a
pub const MOTOR_6D: u32 = 5; // Motor6DJoint 00b379d8
pub const ROTATE: u32 = 6; // JointInstance.cpp:616
pub const GLUE: u32 = 9; // JointInstance.cpp:577
pub const FREE: u32 = 20; // FreeJoint 00b0fbf2

pub fn is_rigid(kind: u32) -> bool {
    kind.wrapping_sub(1) < 3
}
pub fn is_motor(kind: u32) -> bool {
    kind & !1 == 4
}
pub fn is_kernel(kind: u32) -> bool {
    kind.wrapping_sub(6) < 14
}
pub fn is_ground(kind: u32) -> bool {
    kind == ANCHOR || kind == FREE
}
/// 009241f4: the joints TreeStage puts in the spanning tree.
pub fn in_spanning_tree(kind: u32) -> bool {
    kind.wrapping_sub(1) < 5 || is_kernel(kind) || is_ground(kind)
}

/// Primitive +0xd1, set by PartInstance::setName 00983a3e.
pub fn name_class(name: &str) -> u8 {
    match name {
        "HumanoidRootPart" => 2,
        "Torso" => 1,
        _ => 0,
    }
}
/// 017a87e0, indexed by Primitive +0xd1 (00b61aca).
const MULTIPLIER: [u32; 4] = [1, 5, 10, 20];

/// Primitive root priority 00b63fbe (cached at +0x90): the planar size is the
/// largest axis times the larger of the other two.
pub fn root_priority(size: [f32; 3], class: u8) -> u32 {
    let [x, y, z] = size;
    let planar = if y <= x {
        x * if z <= y { y } else { z }
    } else {
        y * if z <= x { x } else { z }
    };
    let floor = (planar * 50.0).floor();
    let half = if floor >= 0.0 { 0.5 } else { -0.5 };
    let scaled = (f64::from(floor) + half) as i32;
    MULTIPLIER[usize::from(class.min(3))]
        .wrapping_mul(scaled as u32)
        .wrapping_add(1)
}

#[derive(Clone, Debug)]
pub struct TreePart {
    pub size: [f32; 3],
    /// `name_class` of the Part's name.
    pub class: u8,
    /// Anchored or being dragged.
    pub fixed: bool,
}

#[derive(Clone, Debug)]
pub struct Joint {
    pub kind: u32,
    /// The second Part is None for ground joints.
    pub parts: [Option<usize>; 2],
    /// Joint +0x38 / +0x68 (C0, C1).
    pub coords: [CoordinateFrame; 2],
}

/// A Body link made by TreeStage.
#[derive(Clone, Debug, PartialEq)]
pub enum Link {
    /// A rigid joint's fixed offset in the parent Body.
    Rigid(CoordinateFrame),
    /// A motor; its offset follows the motor angle.
    Motor(usize),
}

#[derive(Clone, Debug)]
struct Node {
    part: TreePart,
    priority: u32,
    /// Spanning node (IndexedTree) parent and edge to it.
    parent: Option<usize>,
    edge: Option<usize>,
    children: Vec<usize>,
    /// The Part's joints (Primitive +0x70 list), in the order they were added.
    joints: Vec<usize>,
    body_parent: Option<usize>,
    body_children: Vec<usize>,
    link: Option<Link>,
}

/// One TreeStage or GroundStage change, for logs.
#[derive(Clone, Debug, PartialEq)]
pub enum TreeEvent {
    Linked {
        joint: usize,
        child: usize,
    },
    Unlinked {
        joint: usize,
        child: usize,
    },
    GroundJoint {
        part: usize,
        joint: usize,
        kind: u32,
    },
    GroundJointRemoved {
        part: usize,
        joint: usize,
        kind: u32,
    },
}

#[derive(Clone, Debug, Default)]
pub struct JointTree {
    nodes: Vec<Node>,
    joints: Vec<Option<Joint>>,
    pub log: Vec<TreeEvent>,
}

const IDENTITY: CoordinateFrame = CoordinateFrame {
    position: [0.0; 3],
    rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
};

/// IndexArray removal: the last element moves into the hole.
fn swap_remove(list: &mut Vec<usize>, item: usize) {
    if let Some(at) = list.iter().position(|&x| x == item) {
        list.swap_remove(at);
    }
}

/// Guid compare (006721c8's inner step). Every Part has the same scope, so
/// the creation index decides; the null GUID of a missing Part ranks first
/// (assumed: it never decides a comparison that reaches it).
fn guid(a: Option<usize>, b: Option<usize>) -> i64 {
    match (a, b) {
        (Some(a), Some(b)) => a as i64 - b as i64,
        (None, None) => 0,
        (None, Some(_)) => -1,
        (Some(_), None) => 1,
    }
}
/// Guid::compare(j00, j01, j10, j11) 006721c8: the larger GUIDs of each pair
/// first, then the smaller ones.
fn guid_pairs(a0: Option<usize>, a1: Option<usize>, b0: Option<usize>, b1: Option<usize>) -> i64 {
    let left = guid(a0, a1);
    let right = guid(b0, b1);
    let (max_a, min_a) = if left > 0 { (a0, a1) } else { (a1, a0) };
    let (max_b, min_b) = if right > 0 { (b0, b1) } else { (b1, b0) };
    let result = guid(max_a, max_b);
    if result != 0 {
        result
    } else {
        guid(min_a, min_b)
    }
}

impl JointTree {
    pub fn new() -> Self {
        Self::default()
    }
    fn joint(&self, j: usize) -> &Joint {
        self.joints[j].as_ref().expect("live joint")
    }
    pub fn joint_kind(&self, j: usize) -> Option<u32> {
        self.joints.get(j)?.as_ref().map(|j| j.kind)
    }

    /// 00b216ad: the larger root priority of the joint's Parts.
    fn edge_priority(&self, j: usize) -> u32 {
        let joint = self.joint(j);
        let first = self.nodes[joint.parts[0].expect("joint part 0")].priority;
        match joint.parts[1] {
            Some(p) => first.max(self.nodes[p].priority),
            None => first,
        }
    }
    /// Joint comparator 00b216e2 (SpanningEdge vtable +8): `a` ranks before
    /// `b`, so `b` is the worse edge.
    fn better(&self, a: usize, b: usize) -> bool {
        if a == b {
            return false;
        }
        let (ja, jb) = (self.joint(a), self.joint(b));
        if ja.kind != jb.kind {
            return ja.kind < jb.kind;
        }
        let (pa, pb) = (self.edge_priority(a), self.edge_priority(b));
        if pa != pb {
            return pa > pb;
        }
        let c = guid_pairs(ja.parts[0], ja.parts[1], jb.parts[0], jb.parts[1]);
        if c != 0 {
            return c > 0;
        }
        a < b
    }

    /// SpanningEdge::otherNode (00b21a44).
    fn other(&self, j: usize, node: Option<usize>) -> Option<usize> {
        let parts = self.joint(j).parts;
        if parts[0] == node { parts[1] } else { parts[0] }
    }
    /// SpanningEdge child node (006cba60): the end whose edge to its parent
    /// is this joint.
    fn child(&self, j: usize) -> Option<usize> {
        self.joint(j)
            .parts
            .into_iter()
            .flatten()
            .find(|&n| self.nodes[n].edge == Some(j))
    }
    /// 006cbd36.
    pub fn in_tree(&self, j: usize) -> bool {
        self.joints.get(j).is_some_and(|x| x.is_some()) && self.child(j).is_some()
    }
    /// 006cceec: the null node is 0, a root 1.
    fn depth(&self, node: Option<usize>) -> usize {
        let mut depth = 0;
        let mut at = node;
        while let Some(n) = at {
            depth += 1;
            at = self.nodes[n].parent;
        }
        depth
    }

    /// 006ccc72: the worst tree edge on the path between two nodes, and the
    /// side it is on (0 for the first node's chain).
    fn worst_on_path(&self, n0: Option<usize>, n1: Option<usize>) -> (Option<usize>, usize) {
        let mut worst: Option<usize> = None;
        let mut side = 0;
        let mut consider = |edge: Option<usize>, s: usize, worst: &mut Option<usize>| {
            if let Some(e) = edge
                && worst.is_none_or(|w| self.better(w, e))
            {
                *worst = Some(e);
                side = s;
            }
        };
        let (mut a, mut b) = (n0, n1);
        let (mut da, mut db) = (self.depth(n0), self.depth(n1));
        loop {
            while da != db {
                if db < da {
                    let node = a.expect("deeper node");
                    consider(self.nodes[node].edge, 0, &mut worst);
                    da -= 1;
                    a = self.nodes[node].parent;
                } else {
                    let node = b.expect("deeper node");
                    consider(self.nodes[node].edge, 1, &mut worst);
                    b = self.nodes[node].parent;
                    db -= 1;
                }
            }
            if a == b {
                break;
            }
            if let Some(node) = a {
                consider(self.nodes[node].edge, 0, &mut worst);
                a = self.nodes[node].parent;
            }
            if let Some(node) = b {
                consider(self.nodes[node].edge, 1, &mut worst);
                b = self.nodes[node].parent;
            }
            da -= 1;
            db = da;
        }
        (worst, side)
    }

    /// IndexedTree::setParent 00675752 for spanning nodes.
    fn set_node_parent(&mut self, n: usize, parent: Option<usize>) {
        let old = self.nodes[n].parent;
        if old == parent {
            return;
        }
        if let Some(p) = old {
            swap_remove(&mut self.nodes[p].children, n);
        }
        self.nodes[n].parent = parent;
        if let Some(p) = parent {
            self.nodes[p].children.push(n);
        }
    }
    /// IndexedTree::setParent 00675752 for Bodies.
    fn set_body_parent(&mut self, n: usize, parent: Option<usize>) {
        let old = self.nodes[n].body_parent;
        if old == parent {
            return;
        }
        if let Some(p) = old {
            swap_remove(&mut self.nodes[p].body_children, n);
        }
        self.nodes[n].body_parent = parent;
        if let Some(p) = parent {
            self.nodes[p].body_children.push(n);
        }
    }

    /// 00b69dd6: the child Part's Body offset in its parent's Body.
    fn rigid_offset(&self, j: usize, parent: usize, child: usize) -> CoordinateFrame {
        let joint = self.joint(j);
        let coord = |n: usize| &joint.coords[usize::from(joint.parts[0] != Some(n))];
        let mut offset = g3d::frame_multiply(coord(parent), &g3d::frame_inverse(coord(child)));
        if !g3d::is_orthonormal(&offset.rotation) {
            g3d::orthonormalize(&mut offset.rotation);
        }
        offset
    }
    /// TreeStage after link (00ba2e24).
    fn linked(&mut self, j: usize) {
        let child = self.child(j).expect("linked child");
        let parent = self.nodes[child].parent;
        let kind = self.joint(j).kind;
        if let Some(parent) = parent {
            if is_rigid(kind) {
                self.set_body_parent(child, Some(parent));
                let offset = self.rigid_offset(j, parent, child);
                self.nodes[child].link = Some(Link::Rigid(offset));
            } else if is_motor(kind) {
                self.set_body_parent(child, Some(parent));
                self.nodes[child].link = Some(Link::Motor(j));
            }
        }
        self.log.push(TreeEvent::Linked { joint: j, child });
    }
    /// TreeStage after unlink (00ba36a6): the child's Body leaves its parent.
    fn unlinked(&mut self, j: usize, child: usize) {
        self.set_body_parent(child, None);
        self.nodes[child].link = None;
        self.log.push(TreeEvent::Unlinked { joint: j, child });
    }
    /// 006cc8b4: link an edge with `parent` as the parent end.
    fn link_edge(&mut self, j: usize, parent: Option<usize>) {
        let child = self.other(j, parent).expect("edge child");
        self.nodes[child].edge = Some(j);
        self.set_node_parent(child, parent);
        self.linked(j);
    }
    /// 006cc7a4.
    fn unlink_edge(&mut self, j: usize) {
        let child = self.child(j).expect("tree edge child");
        self.set_node_parent(child, None);
        self.nodes[child].edge = None;
        self.unlinked(j, child);
    }
    /// 006cc9ca: unlink the edges from `n` up to the edge being removed.
    fn detach_path(&mut self, n: usize, remove: Option<usize>, stack: &mut Vec<usize>) {
        let Some(parent) = self.nodes[n].parent else {
            return;
        };
        let edge = self.nodes[n].edge.expect("tree node edge");
        if Some(edge) != remove {
            stack.push(edge);
            self.unlink_edge(edge);
            self.detach_path(parent, remove, stack);
        }
    }
    /// 006cc70a: remove one edge and insert another at `node`, re-rooting the
    /// piece between them.
    fn replace(&mut self, remove: Option<usize>, insert: Option<usize>, node: Option<usize>) {
        let mut stack = Vec::new();
        if let Some(i) = insert {
            let n = self.other(i, node).expect("inserted edge end");
            self.detach_path(n, remove, &mut stack);
        }
        if let Some(r) = remove {
            self.unlink_edge(r);
        }
        if let Some(i) = insert {
            self.link_edge(i, node);
            let mut previous = self.other(i, node);
            for e in stack {
                self.link_edge(e, previous);
                previous = self.other(e, previous);
            }
        }
    }
    /// SpanningTree::insertEdge 006cc3d6.
    fn insert_edge(&mut self, j: usize) {
        let parts = self.joint(j).parts;
        let (worst, side) = self.worst_on_path(parts[0], parts[1]);
        if let Some(w) = worst
            && !self.better(j, w)
        {
            return;
        }
        self.replace(worst, Some(j), parts[(side + 1) % 2]);
    }
    /// 006ccdc0: the nodes of a subtree.
    fn subtree(&self, root: usize) -> BTreeSet<usize> {
        let mut out = BTreeSet::new();
        let mut pending = vec![root];
        while let Some(n) = pending.pop() {
            out.insert(n);
            pending.extend(&self.nodes[n].children);
        }
        out
    }
    /// SpanningTree::removeEdge 006cc580 with the replacement search 006cc624.
    fn remove_edge(&mut self, j: usize) {
        let root = self.child(j).expect("removed tree edge child");
        let nodes = self.subtree(root);
        let mut best: Option<usize> = None;
        let mut outside: Option<usize> = None;
        for &n in &nodes {
            for &e in &self.nodes[n].joints {
                if self.in_tree(e) {
                    continue;
                }
                let other = self.other(e, Some(n));
                if other.is_some_and(|o| nodes.contains(&o)) {
                    continue;
                }
                if best.is_some_and(|b| !self.better(e, b)) {
                    continue;
                }
                best = Some(e);
                outside = other;
            }
        }
        self.replace(Some(j), best, outside);
    }

    fn fixed(&self, n: usize) -> bool {
        self.nodes[n].part.fixed
    }
    fn find(&self, n: usize, kind: u32) -> Option<usize> {
        self.nodes[n]
            .joints
            .iter()
            .copied()
            .find(|&j| self.joint(j).kind == kind)
    }
    /// 00b0f7ac: the best rigid joint from `n` to a fixed Part.
    fn fixed_rigid_joint(&self, n: usize) -> Option<usize> {
        let mut best = None;
        for &j in &self.nodes[n].joints {
            if !is_rigid(self.joint(j).kind) {
                continue;
            }
            let other = self.other(j, Some(n)).expect("rigid joint end");
            if self.fixed(other) && best.is_none_or(|b| self.better(j, b)) {
                best = Some(j);
            }
        }
        best
    }
    fn add_edge(&mut self, joint: Joint) -> usize {
        let j = self.joints.len();
        for n in joint.parts.into_iter().flatten() {
            self.nodes[n].joints.push(j);
        }
        let kind = joint.kind;
        self.joints.push(Some(joint));
        // GroundStage forwards the edge (00b1e7ba); TreeStage::onEdgeAdded
        // 00ba3e46 inserts tree joints.
        if in_spanning_tree(kind) {
            self.insert_edge(j);
        }
        j
    }
    fn delete_edge(&mut self, j: usize) {
        if self.in_tree(j) {
            self.remove_edge(j);
        }
        for n in self.joint(j).parts.into_iter().flatten() {
            swap_remove(&mut self.nodes[n].joints, j);
        }
        self.joints[j] = None;
    }
    /// GroundStage 00b0ed74: add the Part's AnchorJoint or FreeJoint.
    fn add_ground(&mut self, n: usize, anchor: bool) {
        let kind = if anchor { ANCHOR } else { FREE };
        let j = self.add_edge(Joint {
            kind,
            parts: [Some(n), None],
            coords: [IDENTITY, IDENTITY],
        });
        self.log.push(TreeEvent::GroundJoint {
            part: n,
            joint: j,
            kind,
        });
    }
    /// GroundStage 00b0efde.
    fn remove_ground(&mut self, n: usize, anchor: bool) {
        let kind = if anchor { ANCHOR } else { FREE };
        let j = self.find(n, kind).expect("ground joint");
        self.delete_edge(j);
        self.log.push(TreeEvent::GroundJointRemoved {
            part: n,
            joint: j,
            kind,
        });
    }
    /// GroundStage::reevaluate 00b0f208.
    fn reevaluate(&mut self, n: usize) {
        if self.fixed(n) {
            return;
        }
        if self.fixed_rigid_joint(n).is_none() {
            if self.find(n, FREE).is_some() {
                return;
            }
            self.remove_ground(n, true);
            self.add_ground(n, false);
        } else {
            if self.find(n, ANCHOR).is_some() {
                return;
            }
            self.remove_ground(n, false);
            self.add_ground(n, true);
        }
    }

    /// World::insertPrimitive and GroundStage::onPrimitiveAdded 00b0f1a8.
    /// Returns the Part's index, which is also its GUID order.
    pub fn add_part(&mut self, part: TreePart) -> usize {
        let n = self.nodes.len();
        let fixed = part.fixed;
        self.nodes.push(Node {
            priority: root_priority(part.size, part.class),
            part,
            parent: None,
            edge: None,
            children: Vec::new(),
            joints: Vec::new(),
            body_parent: None,
            body_children: Vec::new(),
            link: None,
        });
        self.add_ground(n, fixed);
        if !fixed {
            self.reevaluate(n);
        }
        // 00b0f30a: reevaluate the Parts rigidly jointed to this one.
        let joints = self.nodes[n].joints.clone();
        for j in joints {
            if is_rigid(self.joint(j).kind)
                && let Some(other) = self.other(j, Some(n))
            {
                self.reevaluate(other);
            }
        }
        n
    }
    /// World::insertJoint, then GroundStage::onEdgeAdded 00b0f358: the joint
    /// enters the tree, then a rigid joint between a fixed and a free Part
    /// re-evaluates the free one (00b0f4e8).
    pub fn add_joint(
        &mut self,
        kind: u32,
        part0: usize,
        part1: usize,
        c0: CoordinateFrame,
        c1: CoordinateFrame,
    ) -> Result<usize, String> {
        if part0 == part1 || part0.max(part1) >= self.nodes.len() {
            return Err("joint needs two different Parts in the tree".into());
        }
        if is_ground(kind) {
            return Err("ground joints come from GroundStage".into());
        }
        let j = self.add_edge(Joint {
            kind,
            parts: [Some(part0), Some(part1)],
            coords: [c0, c1],
        });
        if is_rigid(kind) && self.fixed(part0) != self.fixed(part1) {
            self.reevaluate(if self.fixed(part0) { part1 } else { part0 });
        }
        Ok(j)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    /// The joint linking the Part to its spanning-tree parent.
    pub fn tree_joint(&self, n: usize) -> Option<usize> {
        self.nodes[n].edge
    }
    pub fn tree_parent(&self, n: usize) -> Option<usize> {
        self.nodes[n].parent
    }
    pub fn body_parent(&self, n: usize) -> Option<usize> {
        self.nodes[n].body_parent
    }
    /// The Body's children in IndexArray order.
    pub fn body_children(&self, n: usize) -> &[usize] {
        &self.nodes[n].body_children
    }
    pub fn link(&self, n: usize) -> Option<&Link> {
        self.nodes[n].link.as_ref()
    }
    pub fn priority(&self, n: usize) -> u32 {
        self.nodes[n].priority
    }
    /// The root Part of the Part's assembly: up the Body tree.
    pub fn assembly_root(&self, n: usize) -> usize {
        let mut at = n;
        while let Some(p) = self.nodes[at].body_parent {
            at = p;
        }
        at
    }
    /// True when the assembly's root Part hangs from an AnchorJoint (it
    /// never moves). A kernel joint (Glue, hinges) above the root starts a
    /// new assembly, so a Part glued under an anchored one still moves.
    pub fn grounded(&self, n: usize) -> bool {
        let root = self.assembly_root(n);
        self.nodes[root]
            .edge
            .is_some_and(|j| self.joint(j).kind == ANCHOR)
    }
    /// The assembly's Parts, breadth first through the Body children (each
    /// Body's children in IndexArray order).
    pub fn assembly_parts(&self, root: usize) -> Vec<usize> {
        let mut out = vec![root];
        let mut i = 0;
        while i < out.len() {
            out.extend(self.nodes[out[i]].body_children.iter().copied());
            i += 1;
        }
        out
    }
    /// Each Part's CFrame as its Body places it (Body::updatePV 00a8eabe
    /// down the rigid links): an assembly root keeps its own CFrame from
    /// `frames`, a child is its parent's CFrame times the link offset.
    pub fn body_frames(&self, frames: &[CoordinateFrame]) -> Result<Vec<CoordinateFrame>, String> {
        if frames.len() != self.nodes.len() {
            return Err("one CFrame per Part expected".into());
        }
        let mut out: Vec<Option<CoordinateFrame>> = vec![None; frames.len()];
        for root in (0..self.nodes.len()).filter(|&n| self.nodes[n].body_parent.is_none()) {
            for n in self.assembly_parts(root) {
                let frame = match (self.nodes[n].body_parent, &self.nodes[n].link) {
                    (None, _) => frames[n].clone(),
                    (Some(parent), Some(Link::Rigid(offset))) => {
                        let parent = out[parent].as_ref().ok_or("Body child before its parent")?;
                        g3d::frame_multiply(parent, offset)
                    }
                    _ => return Err("motor links need the motor angle".into()),
                };
                out[n] = Some(frame);
            }
        }
        out.into_iter()
            .map(|f| f.ok_or_else(|| "Part outside the Body tree".to_string()))
            .collect()
    }
    /// One line per Part: tree joint, Body parent and offset.
    pub fn describe(&self, names: &[&str]) -> String {
        let name = |n: Option<usize>| n.map_or("ground", |n| names.get(n).copied().unwrap_or("?"));
        let mut out = String::new();
        for n in 0..self.nodes.len() {
            let joint = self.nodes[n].edge.map(|j| self.joint(j).kind);
            let offset = match &self.nodes[n].link {
                Some(Link::Rigid(f)) => format!("{:?}", f.position),
                Some(Link::Motor(j)) => format!("motor{j}"),
                None => "-".into(),
            };
            out.push_str(&format!(
                "joint_tree_part name={} priority={} tree_joint_kind={:?} tree_parent={} body_parent={} body_children={:?} root={} grounded={} offset={}\n",
                name(Some(n)),
                self.nodes[n].priority,
                joint,
                name(self.nodes[n].parent),
                self.nodes[n].body_parent.map_or("-", |p| names.get(p).copied().unwrap_or("?")),
                self.nodes[n]
                    .body_children
                    .iter()
                    .map(|&c| names.get(c).copied().unwrap_or("?"))
                    .collect::<Vec<_>>(),
                name(Some(self.assembly_root(n))),
                self.grounded(n),
                offset,
            ));
        }
        out
    }
}

/// audit-joints: the spanning tree on the R6 character (Motor6Ds in the
/// rig's order) and on three weld scenes.
pub fn audit(bundle: &crate::bundle::Bundle) -> Result<String, String> {
    use crate::{rig::R6Rig, scene::Scene};
    use std::fmt::Write;
    let bytes = bundle.read(
        "Contents/Resources/content/fonts/character3.rbxm",
        16 * 1024 * 1024,
    )?;
    let scene = Scene::parse(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)?;
    let rig = R6Rig::character(bundle, &scene)?;
    let mut tree = JointTree::new();
    let mut names = Vec::new();
    for instance in &scene.instances {
        let Some(part) = &instance.part else { continue };
        tree.add_part(TreePart {
            size: part.size,
            class: name_class(&instance.name),
            fixed: part.anchored,
        });
        names.push(instance.name.as_str());
    }
    let index = |name: &str| {
        names
            .iter()
            .position(|n| *n == name)
            .ok_or_else(|| format!("missing rig Part {name}"))
    };
    for joint in &rig.joints {
        tree.add_joint(
            MOTOR_6D,
            index(&joint.part0)?,
            index(&joint.part1)?,
            joint.c0.clone(),
            joint.c1.clone(),
        )?;
    }
    let mut report = tree.describe(&names);
    let root = index("HumanoidRootPart")?;
    let mut checks = Vec::new();
    for (n, name) in names.iter().enumerate() {
        if tree.assembly_root(n) != root {
            checks.push(format!("{name}_not_in_root_assembly"));
        }
    }
    // from_rig's tree: each Motor6D's part0 is the parent, children in rig order.
    for (n, name) in names.iter().enumerate() {
        let expected: Vec<usize> = rig
            .joints
            .iter()
            .filter(|j| j.part0 == *name)
            .map(|j| index(&j.part1))
            .collect::<Result<_, _>>()?;
        if tree.body_children(n) != expected.as_slice() {
            checks.push(format!("{name}_children_differ_from_rig_order"));
        }
    }
    writeln!(
        report,
        "joint_tree_r6 root={} parts={} checks={checks:?}",
        names[tree.assembly_root(0)],
        names.len()
    )
    .unwrap();

    let block = |size: [f32; 3], fixed: bool| TreePart {
        size,
        class: 0,
        fixed,
    };
    let offset = |y: f32| CoordinateFrame {
        position: [0.0, y, 0.0],
        ..IDENTITY
    };
    // A small block welded under a large one.
    let mut pair = JointTree::new();
    let small = pair.add_part(block([1.0, 1.0, 1.0], false));
    let large = pair.add_part(block([4.0, 1.0, 4.0], false));
    pair.add_joint(WELD, small, large, offset(1.0), IDENTITY)?;
    report.push_str(&pair.describe(&["Small", "Large"]));
    // A block welded onto an anchored base, a second one snapped on top.
    let mut stack = JointTree::new();
    let base = stack.add_part(block([10.0, 1.0, 10.0], true));
    let middle = stack.add_part(block([1.0, 1.0, 1.0], false));
    let top = stack.add_part(block([1.0, 1.0, 1.0], false));
    stack.add_joint(WELD, base, middle, IDENTITY, offset(-1.0))?;
    stack.add_joint(SNAP, middle, top, IDENTITY, offset(-1.0))?;
    report.push_str(&stack.describe(&["Base", "Middle", "Top"]));
    // A chain that re-roots at its largest Part.
    let mut chain = JointTree::new();
    let a = chain.add_part(block([1.0, 1.0, 1.0], false));
    let b = chain.add_part(block([2.0, 1.0, 2.0], false));
    let c = chain.add_part(block([4.0, 1.0, 4.0], false));
    chain.add_joint(WELD, a, b, IDENTITY, IDENTITY)?;
    chain.add_joint(WELD, b, c, IDENTITY, IDENTITY)?;
    report.push_str(&chain.describe(&["A", "B", "C"]));
    for event in &chain.log {
        writeln!(report, "joint_tree_chain_event {event:?}").unwrap();
    }
    let pass = checks.is_empty()
        && pair.assembly_root(small) == large
        && stack.grounded(top)
        && chain.assembly_parts(c) == [c, b, a];
    if !pass {
        return Err(format!("{report}joint_tree_failed"));
    }
    writeln!(
        report,
        "joint_tree_pass r6_root=HumanoidRootPart pair_root=Large stack_grounded=true chain_root=C"
    )
    .unwrap();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(size: [f32; 3]) -> TreePart {
        TreePart {
            size,
            class: 0,
            fixed: false,
        }
    }
    fn at(x: f32, y: f32, z: f32) -> CoordinateFrame {
        CoordinateFrame {
            position: [x, y, z],
            ..IDENTITY
        }
    }

    #[test]
    fn priority_uses_the_two_largest_axes() {
        // 4 x 1.2 x 2: planar 4 * 2 = 8, floor(400) + 1.
        assert_eq!(root_priority([4.0, 1.2, 2.0], 0), 401);
        assert_eq!(root_priority([1.0, 6.0, 4.0], 0), 1201);
        // HumanoidRootPart 2 x 2 x 1: 10 * 200 + 1.
        assert_eq!(root_priority([2.0, 2.0, 1.0], 2), 2001);
        assert_eq!(name_class("Torso"), 1);
    }

    #[test]
    fn welded_pair_roots_at_the_larger_part() {
        let mut tree = JointTree::new();
        let small = tree.add_part(part([1.0, 1.0, 1.0]));
        let large = tree.add_part(part([4.0, 1.0, 4.0]));
        tree.add_joint(WELD, small, large, at(0.0, 1.0, 0.0), IDENTITY)
            .unwrap();
        assert_eq!(tree.assembly_root(small), large);
        assert_eq!(tree.body_children(large), &[small]);
        // The child is `small` (part0): C1 * inverse(C0).
        assert_eq!(tree.link(small), Some(&Link::Rigid(at(0.0, -1.0, 0.0))));
        // Exactly one free joint stays in the tree: the large Part's.
        let free = tree.tree_joint(large).unwrap();
        assert_eq!(tree.joint_kind(free), Some(FREE));
        assert_eq!(tree.joint_kind(tree.tree_joint(small).unwrap()), Some(WELD));
        assert!(!tree.grounded(small));
    }

    #[test]
    fn body_frames_place_children_by_the_weld() {
        let mut tree = JointTree::new();
        let small = tree.add_part(part([1.0, 1.0, 1.0]));
        let large = tree.add_part(part([4.0, 1.0, 4.0]));
        tree.add_joint(WELD, large, small, at(0.0, 1.0, 0.0), IDENTITY)
            .unwrap();
        // The child's own CFrame is ignored: it hangs from the root's Body.
        let frames = tree
            .body_frames(&[at(50.0, 50.0, 50.0), at(2.0, 3.0, 4.0)])
            .unwrap();
        assert_eq!(frames[large], at(2.0, 3.0, 4.0));
        assert_eq!(frames[small], at(2.0, 4.0, 4.0));
        assert!(tree.body_frames(&[IDENTITY]).is_err());
    }

    #[test]
    fn equal_parts_root_at_the_later_one() {
        let mut tree = JointTree::new();
        let a = tree.add_part(part([2.0, 2.0, 2.0]));
        let b = tree.add_part(part([2.0, 2.0, 2.0]));
        tree.add_joint(WELD, a, b, IDENTITY, IDENTITY).unwrap();
        assert_eq!(tree.assembly_root(a), b);
    }

    #[test]
    fn welded_to_anchored_is_grounded() {
        let mut tree = JointTree::new();
        let base = tree.add_part(TreePart {
            fixed: true,
            ..part([10.0, 1.0, 10.0])
        });
        let block = tree.add_part(part([1.0, 1.0, 1.0]));
        let top = tree.add_part(part([1.0, 1.0, 1.0]));
        tree.add_joint(WELD, base, block, IDENTITY, at(0.0, -1.0, 0.0))
            .unwrap();
        tree.add_joint(SNAP, block, top, IDENTITY, at(0.0, -1.0, 0.0))
            .unwrap();
        // GroundStage swapped the block's FreeJoint for an AnchorJoint.
        assert!(tree.find(block, ANCHOR).is_some() && tree.find(block, FREE).is_none());
        assert!(tree.find(top, FREE).is_some());
        assert!(tree.grounded(top) && tree.grounded(block));
        assert_eq!(tree.joint_kind(tree.tree_joint(top).unwrap()), Some(SNAP));
    }

    #[test]
    fn glued_under_anchored_still_moves() {
        let mut tree = JointTree::new();
        let base = tree.add_part(TreePart {
            fixed: true,
            ..part([10.0, 1.0, 10.0])
        });
        let hung = tree.add_part(part([2.0, 1.0, 2.0]));
        let tail = tree.add_part(part([1.0, 1.0, 1.0]));
        tree.add_joint(GLUE, base, hung, IDENTITY, at(0.0, 1.0, 0.0))
            .unwrap();
        tree.add_joint(WELD, hung, tail, IDENTITY, at(0.0, 1.0, 0.0))
            .unwrap();
        // The Glue beats the FreeJoint in the tree but makes no Body link:
        // the hung Part roots its own assembly, which is not grounded.
        assert_eq!(tree.joint_kind(tree.tree_joint(hung).unwrap()), Some(GLUE));
        assert!(tree.find(hung, FREE).is_some());
        assert_eq!(tree.body_parent(hung), None);
        assert_eq!(tree.assembly_root(tail), hung);
        assert!(!tree.grounded(hung) && !tree.grounded(tail));
        assert!(tree.grounded(base));
    }

    #[test]
    fn chain_reroots_at_the_largest_part() {
        let mut tree = JointTree::new();
        let a = tree.add_part(part([1.0, 1.0, 1.0]));
        let b = tree.add_part(part([2.0, 1.0, 2.0]));
        let c = tree.add_part(part([4.0, 1.0, 4.0]));
        tree.add_joint(WELD, a, b, IDENTITY, IDENTITY).unwrap();
        tree.add_joint(WELD, b, c, IDENTITY, IDENTITY).unwrap();
        assert_eq!(tree.assembly_root(a), c);
        assert_eq!(tree.body_parent(a), Some(b));
        assert_eq!(tree.body_parent(b), Some(c));
        assert_eq!(tree.assembly_parts(c), vec![c, b, a]);
    }

    #[test]
    fn humanoid_root_part_wins_over_a_larger_torso() {
        let mut tree = JointTree::new();
        let torso = tree.add_part(TreePart {
            class: 1,
            ..part([2.0, 2.0, 1.0])
        });
        let root = tree.add_part(TreePart {
            class: 2,
            ..part([2.0, 2.0, 1.0])
        });
        tree.add_joint(MOTOR_6D, root, torso, IDENTITY, IDENTITY)
            .unwrap();
        assert_eq!(tree.assembly_root(torso), root);
        assert_eq!(tree.link(torso), Some(&Link::Motor(2)));
    }
}
