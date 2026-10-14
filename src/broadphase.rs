//! SpatialHash<Primitive, Contact, ContactManager, 4> (SpatialHashMultiRes):
//! the broadphase that decides which primitive pairs have a Contact.
//!
//! Recovered from:
//! - construction 00af0ddc (called with 100 from 00af0dbc, ContactManager
//!   ctor 00ae6b92): four levels, 0x10000 buckets per level;
//! - primitive extents 00b61c9a: body position -/+ the geometry's world half
//!   extent (Block 00adf0a0: the largest |R v| over the block's corners),
//!   -/+ 0.01;
//! - level 00af1350: the first level i with
//!   (dx + 16)(dy + 16)(dz + 16) < 8^3 * 100 * 8^i; at most 3, or at most 2
//!   when the primitive requests a fixed body (+0xca dragging, +0xcb
//!   anchored). The constants 8 (017a9c50) and 2 (017a9c54) are read from
//!   the binary;
//! - cells 00b88880: extents clamped to about +-1002456.7 (0073c85c adds a
//!   rand() % 0x10000 jitter that is not modelled), then
//!   floor(v / (8 << level));
//! - bucket hash 00b8885c: (y * 0x127409f ^ x * 0x466f45d ^ z * 0x4f9ffb7) &
//!   0xffff. A bucket chain is newest first (00af21cc pushes at the head);
//!   only nodes whose grid id matches count, so collisions never change a
//!   result;
//! - insert 00aee438 (no contacts from onPrimitiveAdded 00aeca1c / 00ae97aa,
//!   contacts from a reinsert in 00aecaea), remove 00aef058, extents changed
//!   00aecaea with the move 00af1400 when the new level is the stored one or
//!   one below, the cell walk 00aee5ec, add node 00af21cc, descend 00af19dc,
//!   remove node 00af2714, the overlap check after a move 00aef1d8 / 00aef442;
//! - findAllContacts 00af1684 (SpatialFilter phase changes, 00af6e2c);
//! - the primitive contact list (+0x64): appended on creation, the edge keeps
//!   its index (+0x18/+0x1c), so a removal moves the last entry into the hole
//!   (IndexArray). The swap is assumed from that layout; the removal routine
//!   itself was not decompiled.
//!
//! The caller owns the primitives (ids are its indices) and decides in
//! `accept` whether ContactManager::createContact 00ae9612 makes a contact:
//! not for the same assembly, not when both are grounded, and not when the
//! geometry pair makes none (CanCollide off, 00ae714a).
use crate::scene::CoordinateFrame;
use std::collections::HashMap;

/// 017a9c50: the level-0 cell size.
const CELL: i32 = 8;
/// 00af0dbc: the volume factor at hash +0xc.
const VOLUME_FACTOR: i32 = 100;
/// 00aecaea / 00af21cc: levels 0..=3.
const LEVELS: i32 = 4;
/// 00af1350: the deepest level of a movable primitive.
const MAX_LEVEL: i32 = 3;
/// 017a9c54: the deepest level of a primitive that requests a fixed body.
const MAX_FIXED_LEVEL: i32 = 2;
/// 0073c85c without its rand() jitter.
const BOUND: f32 = 1002456.7;

/// World extents (min, max), as Primitive::getFastFuzzyExtents 00b61df2.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extents {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// Block::getWorldExtents half sizes 00adf0a0 for a block of `size` and
/// rotation `r`: |R v| per axis, the largest over the corners. Each row dot
/// is summed z, then y, then x, as the native loop does.
pub fn block_half_extent(r: &[[f32; 3]; 3], size: [f32; 3]) -> [f32; 3] {
    let h = size.map(|v| v * 0.5);
    let mut best = [0.0f32; 3];
    let corners = [
        [h[0], h[1], h[2]],
        [-h[0], h[1], h[2]],
        [h[0], -h[1], h[2]],
        [h[0], h[1], -h[2]],
    ];
    for (k, v) in corners.iter().enumerate() {
        let rotated: [f32; 3] =
            std::array::from_fn(|i| (r[i][2] * v[2] + r[i][1] * v[1]) + r[i][0] * v[0]);
        for a in 0..3 {
            let value = rotated[a].abs();
            if k == 0 || best[a] <= value {
                best[a] = value;
            }
        }
    }
    best
}

/// 00b61c9a for a block Part.
pub fn block_extents(frame: &CoordinateFrame, size: [f32; 3]) -> Extents {
    let e = block_half_extent(&frame.rotation, size);
    let p = frame.position;
    Extents {
        min: std::array::from_fn(|a| (p[a] - e[a]) + -0.01),
        max: std::array::from_fn(|a| p[a] + e[a] + 0.01),
    }
}

/// 00af1350.
pub fn level(extents: &Extents, fixed: bool) -> i32 {
    let max_level = if fixed { MAX_FIXED_LEVEL } else { MAX_LEVEL };
    let pad = CELL as f32 + CELL as f32;
    let mut threshold = (CELL * CELL * CELL * VOLUME_FACTOR) as f32;
    let (a, b) = (extents.min, extents.max);
    for level in 0..max_level {
        if ((b[1] - a[1]) + pad) * ((b[0] - a[0]) + pad) * ((b[2] - a[2]) + pad) < threshold {
            return level;
        }
        threshold *= 8.0;
    }
    max_level
}

/// A box of grid cells, inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellBox {
    pub min: [i32; 3],
    pub max: [i32; 3],
}

impl CellBox {
    fn contains(&self, c: [i32; 3]) -> bool {
        (0..3).all(|a| self.min[a] <= c[a] && c[a] <= self.max[a])
    }
    /// 00aef442.
    fn intersects(&self, other: &CellBox) -> bool {
        !(other.max[0] < self.min[0]
            || other.max[1] < self.min[1]
            || other.max[2] < self.min[2]
            || self.max[1] < other.min[1]
            || self.max[0] < other.min[0])
            && other.min[2] <= self.max[2]
    }
    fn shifted(&self, by: i32) -> CellBox {
        CellBox {
            min: self.min.map(|v| v >> by),
            max: self.max.map(|v| v >> by),
        }
    }
    /// 00aee5ec's x, then y, then z walk.
    fn cells(&self) -> Vec<[i32; 3]> {
        let mut out = Vec::new();
        for x in self.min[0]..=self.max[0] {
            for y in self.min[1]..=self.max[1] {
                for z in self.min[2]..=self.max[2] {
                    out.push([x, y, z]);
                }
            }
        }
        out
    }
}

/// 00b88880.
pub fn cells(level: i32, extents: &Extents) -> CellBox {
    // 0066d084: clamp into the bounds (min first, then max).
    let clamp = |v: f32| {
        if -BOUND < v {
            if BOUND <= v { BOUND } else { v }
        } else {
            -BOUND
        }
    };
    let inverse = 1.0 / (CELL << level) as f32;
    // (int)(floor(v) +- 0.5) is floor(v) for these magnitudes.
    let cell = |v: f32| (clamp(v) * inverse).floor() as i32;
    CellBox {
        min: extents.min.map(cell),
        max: extents.max.map(cell),
    }
}

/// 00b8885c (only used for the audit log).
pub fn bucket(cell: [i32; 3]) -> u32 {
    ((cell[1].wrapping_mul(0x127409f))
        ^ (cell[0].wrapping_mul(0x466f45d))
        ^ (cell[2].wrapping_mul(0x4f9ffb7))) as u32
        & 0xffff
}

#[derive(Clone, Debug)]
struct Entry {
    level: i32,
    cells: CellBox,
    fixed: bool,
}

/// A contact created or destroyed, with the primitives in the order the
/// broadphase passes them to createContact (searching primitive first).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Create(usize, usize),
    Destroy(usize, usize),
}

#[derive(Clone, Default)]
pub struct SpatialHash {
    entries: Vec<Option<Entry>>,
    /// The nodes in each (level, cell), newest first.
    nodes: HashMap<(i32, [i32; 3]), Vec<usize>>,
    /// Nodes below each (level, cell) at finer levels: the tree node's
    /// child mask, as counts so empty subtrees are skipped.
    below: HashMap<(i32, [i32; 3]), u32>,
    /// Each primitive's contact list (+0x64): the other primitives.
    contacts: Vec<Vec<usize>>,
    /// Every contact change, in order.
    pub events: Vec<Event>,
}

impl SpatialHash {
    pub fn new() -> Self {
        Self::default()
    }
    fn grow(&mut self, id: usize) {
        if self.entries.len() <= id {
            self.entries.resize(id + 1, None);
            self.contacts.resize(id + 1, Vec::new());
        }
    }
    pub fn level_of(&self, id: usize) -> Option<i32> {
        self.entries.get(id)?.as_ref().map(|e| e.level)
    }
    pub fn cells_of(&self, id: usize) -> Option<CellBox> {
        self.entries.get(id)?.as_ref().map(|e| e.cells)
    }
    /// The primitive's contact list in order.
    pub fn contacts_of(&self, id: usize) -> &[usize] {
        self.contacts.get(id).map_or(&[], Vec::as_slice)
    }
    /// Primitive::getContact 00b62686.
    pub fn has_contact(&self, a: usize, b: usize) -> bool {
        self.contacts_of(a).contains(&b)
    }
    pub fn contact_count(&self) -> usize {
        self.contacts.iter().map(Vec::len).sum::<usize>() / 2
    }
    fn create(&mut self, p: usize, other: usize, accept: &mut impl FnMut(usize, usize) -> bool) {
        if self.has_contact(p, other) || !accept(p, other) {
            return;
        }
        self.contacts[p].push(other);
        self.contacts[other].push(p);
        self.events.push(Event::Create(p, other));
    }
    /// ContactManager::destroyContact 00ae96bc.
    fn destroy(&mut self, p: usize, other: usize) {
        let swap_remove = |list: &mut Vec<usize>, item: usize| {
            if let Some(at) = list.iter().position(|v| *v == item) {
                list.swap_remove(at);
            }
        };
        if !self.has_contact(p, other) {
            return;
        }
        swap_remove(&mut self.contacts[p], other);
        swap_remove(&mut self.contacts[other], p);
        self.events.push(Event::Destroy(p, other));
    }
    /// The nodes of (level, cell) other than `p`, newest first.
    fn others(&self, level: i32, cell: [i32; 3], p: usize) -> Vec<usize> {
        self.nodes
            .get(&(level, cell))
            .map(|v| v.iter().copied().filter(|o| *o != p).collect())
            .unwrap_or_default()
    }
    /// 00af19dc from (level, cell): children by offset 0..7 (x bit, y bit,
    /// z bit), each child's nodes, then its own children.
    fn descend(
        &mut self,
        level: i32,
        cell: [i32; 3],
        p: usize,
        accept: &mut impl FnMut(usize, usize) -> bool,
    ) {
        if self.below.get(&(level, cell)).copied().unwrap_or(0) == 0 {
            return;
        }
        for offset in 0..8 {
            let child = [
                (offset & 1) | (cell[0] << 1),
                ((offset >> 1) & 1) | (cell[1] << 1),
                ((offset >> 2) & 1) | (cell[2] << 1),
            ];
            let child_level = level - 1;
            let has_nodes = self
                .nodes
                .get(&(child_level, child))
                .is_some_and(|v| !v.is_empty());
            let has_below = self.below.get(&(child_level, child)).copied().unwrap_or(0) > 0;
            if !has_nodes && !has_below {
                continue;
            }
            for other in self.others(child_level, child, p) {
                self.create(p, other, accept);
            }
            if child_level > 0 {
                self.descend(child_level, child, p, accept);
            }
        }
    }
    /// 00af21cc: add `p`'s node to `cell` at its level; with `create`,
    /// contacts with the cell, the ancestor cells and the descendant cells.
    fn add_node(
        &mut self,
        p: usize,
        cell: [i32; 3],
        create: bool,
        accept: &mut impl FnMut(usize, usize) -> bool,
    ) {
        let level = self.entries[p].as_ref().map_or(0, |e| e.level);
        let previous = self.others(level, cell, p);
        self.nodes.entry((level, cell)).or_default().insert(0, p);
        let mut c = cell;
        for l in level + 1..LEVELS {
            c = c.map(|v| v >> 1);
            *self.below.entry((l, c)).or_default() += 1;
        }
        if !create {
            return;
        }
        for other in previous {
            self.create(p, other, accept);
        }
        let mut c = cell;
        for l in level + 1..LEVELS {
            c = c.map(|v| v >> 1);
            for other in self.others(l, c, p) {
                self.create(p, other, accept);
            }
        }
        if level > 0 {
            self.descend(level, cell, p, accept);
        }
    }
    /// 00af2714.
    fn remove_node(&mut self, p: usize, level: i32, cell: [i32; 3]) {
        if let Some(list) = self.nodes.get_mut(&(level, cell)) {
            list.retain(|v| *v != p);
            if list.is_empty() {
                self.nodes.remove(&(level, cell));
            }
        }
        let mut c = cell;
        for l in level + 1..LEVELS {
            c = c.map(|v| v >> 1);
            if let Some(n) = self.below.get_mut(&(l, c)) {
                *n -= 1;
                if *n == 0 {
                    self.below.remove(&(l, c));
                }
            }
        }
    }
    /// 00aee5ec: walk `walk`; add cells in `new` but not `old`, remove cells
    /// in `old` but not `new`.
    fn walk(
        &mut self,
        p: usize,
        walk: CellBox,
        old: Option<CellBox>,
        new: Option<CellBox>,
        create: bool,
        accept: &mut impl FnMut(usize, usize) -> bool,
    ) {
        let level = self.entries[p].as_ref().map_or(0, |e| e.level);
        for cell in walk.cells() {
            let in_new = new.is_some_and(|b| b.contains(cell));
            let in_old = old.is_some_and(|b| b.contains(cell));
            if in_new && !in_old {
                self.add_node(p, cell, create, accept);
            } else if !in_new && in_old {
                self.remove_node(p, level, cell);
            }
        }
    }
    /// 00aee438.
    pub fn insert(
        &mut self,
        p: usize,
        extents: Extents,
        fixed: bool,
        create: bool,
        mut accept: impl FnMut(usize, usize) -> bool,
    ) {
        self.grow(p);
        let level = level(&extents, fixed);
        let box_ = cells(level, &extents);
        self.entries[p] = Some(Entry {
            level,
            cells: box_,
            fixed,
        });
        self.walk(p, box_, None, Some(box_), create, &mut accept);
    }
    /// 00aef058: leave every cell, then destroy the contacts in list order.
    pub fn remove(&mut self, p: usize) {
        let Some(entry) = self.entries.get(p).cloned().flatten() else {
            return;
        };
        self.walk(
            p,
            entry.cells,
            Some(entry.cells),
            None,
            false,
            &mut |_, _| false,
        );
        self.entries[p] = None;
        for other in self.contacts[p].clone() {
            self.destroy(p, other);
        }
    }
    /// 00aecaea: the primitive's extents changed.
    pub fn update(
        &mut self,
        p: usize,
        extents: Extents,
        mut accept: impl FnMut(usize, usize) -> bool,
    ) {
        let Some(entry) = self.entries.get(p).cloned().flatten() else {
            return;
        };
        let new_level = level(&extents, entry.fixed);
        if ((new_level + 1 - entry.level) as u32) < 2 {
            self.moved(p, &entry, extents, &mut accept);
        } else {
            self.remove(p);
            self.insert(p, extents, entry.fixed, true, accept);
        }
    }
    /// 00af1400 at the stored level, then 00aef1d8.
    fn moved(
        &mut self,
        p: usize,
        entry: &Entry,
        extents: Extents,
        accept: &mut impl FnMut(usize, usize) -> bool,
    ) {
        let new = cells(entry.level, &extents);
        let old = entry.cells;
        if new == old {
            return;
        }
        if let Some(e) = self.entries[p].as_mut() {
            e.cells = new;
        }
        let disjoint = new.max[0] < old.min[0]
            || new.max[1] < old.min[1]
            || new.max[2] < old.min[2]
            || old.max[1] < new.min[1]
            || old.max[0] < new.min[0]
            || old.max[2] < new.min[2];
        if disjoint {
            self.walk(p, old, Some(old), None, true, accept);
            self.walk(p, new, None, Some(new), true, accept);
        } else {
            let union = CellBox {
                min: std::array::from_fn(|a| new.min[a].min(old.min[a])),
                max: std::array::from_fn(|a| new.max[a].max(old.max[a])),
            };
            self.walk(p, union, Some(old), Some(new), true, accept);
        }
        self.separate(p);
    }
    /// 00aef1d8: destroy the contacts whose cell boxes no longer overlap at
    /// the coarser primitive's level.
    fn separate(&mut self, p: usize) {
        let mut gone = Vec::new();
        for &other in &self.contacts[p] {
            let (Some(me), Some(them)) = (
                self.entries[p].as_ref(),
                self.entries.get(other).and_then(Option::as_ref),
            ) else {
                continue;
            };
            let overlap = if me.level == them.level {
                me.cells.intersects(&them.cells)
            } else {
                let (coarse, fine) = if them.level <= me.level {
                    (me, them)
                } else {
                    (them, me)
                };
                coarse
                    .cells
                    .intersects(&fine.cells.shifted(coarse.level - fine.level))
            };
            if !overlap {
                gone.push(other);
            }
        }
        for other in gone {
            self.destroy(p, other);
        }
    }
    /// 00af1684 (SpatialHash::findAllContacts).
    pub fn find_all_contacts(
        &mut self,
        p: usize,
        extents: Extents,
        mut accept: impl FnMut(usize, usize) -> bool,
    ) {
        let Some(entry) = self.entries.get(p).cloned().flatten() else {
            return;
        };
        let level = level(&extents, entry.fixed);
        for cell in cells(level, &extents).cells() {
            // 00aee7de looks the node up in the stored level's table.
            let has_node = self
                .nodes
                .get(&(entry.level, cell))
                .is_some_and(|v| v.contains(&p));
            if !has_node {
                continue;
            }
            let mut c = cell;
            for l in level..LEVELS {
                if l > level {
                    c = c.map(|v| v >> 1);
                }
                for other in self.others(l, c, p) {
                    self.create(p, other, &mut accept);
                }
            }
            if entry.level > 0 {
                self.descend(entry.level, cell, p, &mut accept);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(position: [f32; 3], size: [f32; 3]) -> Extents {
        block_extents(
            &CoordinateFrame {
                position,
                rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            },
            size,
        )
    }
    #[test]
    fn levels_follow_the_volume_rule() {
        // A 4x1.2x2 brick: (4.02+16)(1.22+16)(2.02+16) < 51200.
        assert_eq!(level(&at([0.0; 3], [4.0, 1.2, 2.0]), false), 0);
        // 6x6x6: (6.02+16)^3 = 10690 < 51200, level 0.
        assert_eq!(level(&at([0.0; 3], [6.0, 6.0, 6.0]), false), 0);
        // 24^3: 40^3 = 64000 >= 51200, < 409600: level 1.
        assert_eq!(level(&at([0.0; 3], [24.0, 24.0, 24.0]), false), 1);
        // The Studio Baseplate (512x20x512), anchored: capped at 2.
        assert_eq!(level(&at([0.0, -10.0, 0.0], [512.0, 20.0, 512.0]), true), 2);
        assert_eq!(
            level(&at([0.0, -10.0, 0.0], [512.0, 20.0, 512.0]), false),
            3
        );
    }
    #[test]
    fn rotated_extents_use_the_absolute_rotation() {
        let r = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        assert_eq!(block_half_extent(&r, [4.0, 2.0, 6.0]), [1.0, 2.0, 3.0]);
    }
    #[test]
    fn contacts_follow_cell_overlap_and_separation() {
        let mut hash = SpatialHash::new();
        let all = |_: usize, _: usize| true;
        hash.insert(0, at([0.0, 0.0, 0.0], [2.0; 3]), false, false, all);
        hash.insert(1, at([1.5, 0.0, 0.0], [2.0; 3]), false, false, all);
        // onPrimitiveAdded makes no contacts; findAllContacts does.
        assert_eq!(hash.contact_count(), 0);
        hash.find_all_contacts(1, at([1.5, 0.0, 0.0], [2.0; 3]), all);
        assert_eq!(hash.events, vec![Event::Create(1, 0)]);
        // Moving 1 far away leaves its cells and destroys the contact.
        hash.update(1, at([40.0, 0.0, 0.0], [2.0; 3]), all);
        assert_eq!(hash.events.last(), Some(&Event::Destroy(1, 0)));
        // Moving back into 0's cells creates it again, 1 first.
        hash.update(1, at([0.5, 0.0, 0.0], [2.0; 3]), all);
        assert_eq!(hash.events.last(), Some(&Event::Create(1, 0)));
    }
    #[test]
    fn coarse_primitives_meet_fine_ones_through_the_tree() {
        let mut hash = SpatialHash::new();
        let all = |_: usize, _: usize| true;
        // A level-2 slab and a level-0 brick on top of it.
        let slab = at([0.0, -10.0, 0.0], [512.0, 20.0, 512.0]);
        hash.insert(0, slab, true, false, all);
        let brick = at([0.0, 0.6, 0.0], [4.0, 1.2, 2.0]);
        hash.insert(1, brick, false, false, all);
        hash.find_all_contacts(1, brick, all);
        assert!(hash.has_contact(0, 1));
        // From the slab's side, the descent reaches the brick as well.
        let mut other = SpatialHash::new();
        other.insert(1, brick, false, false, all);
        other.insert(0, slab, true, false, all);
        other.find_all_contacts(0, slab, all);
        assert_eq!(other.events, vec![Event::Create(0, 1)]);
    }
}
