// hl -- highlighting selected and hovered entities (G6d, decision 30; docs/RENDER2D.md "Hervorheben").  A child of
// scene.rs.
//
// Two bits per entity on a layer (an insert as a whole): selected, hovered.  On the GPU a mask of two bits per record of the
// pool and per instance of a block (u32 words: a header word with the instances' first word, then the records', then the
// instances'); changing a highlight sends the changed words only (runs, a copy each), never geometry.  While anything is
// highlighted the write pass of the visibility is write_hl, which puts a visible record's bits into bits 29 and 30 of its
// word in the visible list, and the layers holding a highlighted entity draw with vertex shaders that read them there
// (hl_wgsl: the same shaders with the colour mixed toward the highlight and strokes wider); every other pipeline, and
// every frame without a highlight, stays as it was.

use alloc::vec::Vec;

use gpu::*;

use super::{upload, EntityId, Geom, Scene, NONE};

/// selected: the colour three quarters of the way to HL_SELECTED_RGB, strokes a quarter pixel wider on either side (a
/// hairline stays on the fast way of short thin strokes)
pub const HL_SELECTED: u8 = 1;
/// hovered: the colour seven eighths of the way to HL_HOVER_RGB, strokes a pixel wider on either side (over a selection)
pub const HL_HOVER: u8 = 2;
/// the highlights' colours (sRGB, 0xRRGGBB) and the least alpha a highlighted colour gets (hl.wgsl)
pub const HL_SELECTED_RGB: u32 = 0x1E64FF;
pub const HL_HOVER_RGB: u32 = 0xFF8C1A;
pub const HL_ALPHA: u32 = 0xB0;

/// what the last prepare sent of the highlights (Scene::highlight_info).  It may grow
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HighlightInfo {
    /// entities highlighted (selected, hovered or both)
    pub entities: u32,
    /// the mask's words on the GPU; the words and runs the last prepare sent; it made the mask anew
    pub words: u32,
    pub sent_words: u32,
    pub runs: u32,
    pub full: bool,
}

/// the highlight state of a scene
#[derive(Default)]
pub(crate) struct Hl {
    /// per entity slot its bits
    pub bits: Vec<u8>,
    pub count: u32,
    /// per layer slot its highlighted entities
    pub per_layer: Vec<u32>,
    /// entities whose bits changed since the last prepare
    pub changed: Vec<u32>,
    /// the mask anew with the next prepare
    pub full: bool,
    /// the mask (header, records, instances) and the instances' first word
    pub words: Vec<u32>,
    pub inst_base: u32,
    pub buf: Option<Buffer>,
    pub cap: u32,
    pub info: HighlightInfo,
}

impl Scene {
    /// an entity on a layer is highlighted (HL_SELECTED, HL_HOVER, both, or 0: not); an insert as a whole.  Only the
    /// mask's changed words go up with the next prepare
    pub fn set_highlight(&mut self, id: EntityId, bits: u8) -> Result<()> {
        let i = self.get(id)?;
        if self.ents[i].block != NONE {
            return Err(GpuError::Invalid("set_highlight: an entity of a block (highlight its insert)"));
        }
        if bits & !(HL_SELECTED | HL_HOVER) != 0 {
            return Err(GpuError::Invalid("set_highlight: HL_SELECTED, HL_HOVER or both"));
        }
        self.hl_set(i as u32, bits);
        Ok(())
    }

    /// many at once (the same bits for each)
    pub fn set_highlights(&mut self, ids: &[EntityId], bits: u8) -> Result<()> {
        for &id in ids {
            self.set_highlight(id, bits)?;
        }
        Ok(())
    }

    /// an entity's highlight bits (0 for a dead id)
    pub fn highlight(&self, id: EntityId) -> u8 {
        match self.get(id) {
            Ok(i) => self.hl.bits.get(i).copied().unwrap_or(0),
            Err(_) => 0,
        }
    }

    /// these bits off on every entity (HL_HOVER: no hover left; HL_SELECTED | HL_HOVER: nothing highlighted)
    pub fn clear_highlights(&mut self, bits: u8) {
        for i in 0..self.hl.bits.len() {
            if self.hl.bits[i] & bits != 0 {
                let b = self.hl.bits[i] & !bits;
                self.hl_set(i as u32, b);
            }
        }
    }

    /// the entities highlighted now
    pub fn highlighted(&self) -> Vec<EntityId> {
        let mut v = Vec::new();
        for (i, &b) in self.hl.bits.iter().enumerate() {
            if b != 0 && self.ents[i].live {
                v.push(EntityId { slot: i as u32, era: self.ents[i].era });
            }
        }
        v
    }

    /// what the last prepare sent of the highlights
    pub fn highlight_info(&self) -> HighlightInfo {
        self.hl.info
    }

    fn hl_set(&mut self, slot: u32, bits: u8) {
        let i = slot as usize;
        if i >= self.hl.bits.len() {
            if bits == 0 {
                return;
            }
            self.hl.bits.resize(i + 1, 0);
        }
        let old = self.hl.bits[i];
        if old == bits {
            return;
        }
        self.hl.count = self.hl.count + (bits != 0) as u32 - (old != 0) as u32;
        let li = self.ents[i].layer as usize;
        if li >= self.hl.per_layer.len() {
            self.hl.per_layer.resize(li + 1, 0);
        }
        self.hl.per_layer[li] = self.hl.per_layer[li] + (bits != 0) as u32 - (old != 0) as u32;
        self.hl.bits[i] = bits;
        self.hl.changed.push(slot);
        self.touch();
    }

    /// (scene.rs) an entity's records or instances may move: its bits are written again with the next prepare
    pub(crate) fn hl_touch(&mut self, slot: u32) {
        if self.hl.bits.get(slot as usize).is_some_and(|&b| b != 0) {
            self.hl.changed.push(slot);
        }
    }

    /// (scene.rs) an entity goes: its highlight with it
    pub(crate) fn hl_forget(&mut self, slot: u32) {
        if self.hl.bits.get(slot as usize).is_some_and(|&b| b != 0) {
            self.hl_set(slot, 0);
        }
    }

    /// (prepare, after the layout and the instances) the mask for this frame: anew when the pool or the instances were laid
    /// out anew, else the changed entities' words; their runs into the frame's upload buffer.  Returns the bytes
    pub(crate) fn hl_prepare(&mut self, slot: usize, laid_out: bool) -> Result<u64> {
        self.hl.info.sent_words = 0;
        self.hl.info.runs = 0;
        self.hl.info.full = false;
        self.hl.info.entities = self.hl.count;
        if self.hl.count == 0 && self.hl.changed.is_empty() && !self.hl.full {
            return Ok(0);
        }
        let recs = self.pool_cap;
        let inst_base = 1 + recs.div_ceil(16);
        let words = inst_base + (self.insts.len() as u32).div_ceil(16) + 1;
        let mut full = self.hl.full || laid_out || self.hl.inst_base != inst_base || self.hl.words.len() as u32 != words || self.hl.buf.is_none();
        if words > self.hl.cap {
            self.hl.cap = words.max(self.hl.cap * 2).max(64);
            self.hl.buf = Some(self.dev.create_buffer(&BufferDesc { size: self.hl.cap as u64 * 4, usage: BufferUsage::STORAGE, memory: MemoryKind::Device })?);
            self.hl.full = true;
            full = true;
            // (write_hl's bind group names the buffer)
            self.hl_group = None;
        }
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        if full {
            self.hl.words.clear();
            self.hl.words.resize(words as usize, 0);
            self.hl.words[0] = inst_base;
            self.hl.inst_base = inst_base;
            for i in 0..self.hl.bits.len() {
                let b = self.hl.bits[i];
                if b != 0 && self.ents[i].live && self.ents[i].block == NONE {
                    self.hl_write(i, b);
                }
            }
            ranges.push((0, words));
            self.hl.changed.clear();
        } else {
            let mut changed = core::mem::take(&mut self.hl.changed);
            changed.sort_unstable();
            changed.dedup();
            for slot in changed {
                let i = slot as usize;
                if !(self.ents[i].live && self.ents[i].block == NONE) {
                    continue;
                }
                let b = self.hl.bits.get(i).copied().unwrap_or(0);
                if let Some(r) = self.hl_write(i, b) {
                    ranges.push(r);
                }
            }
        }
        self.hl.full = false;
        self.hl.info.words = words;
        self.hl.info.full = full;
        if ranges.is_empty() {
            return Ok(0);
        }
        ranges.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
        for (a, e) in ranges {
            match merged.last_mut() {
                Some(m) if m.1 >= a => m.1 = m.1.max(e),
                _ => merged.push((a, e)),
            }
        }
        if merged.len() > 256 {
            merged = alloc::vec![(0, words)];
        }
        let total: u32 = merged.iter().map(|m| m.1 - m.0).sum();
        let size = total as u64 * 4;
        let f = &mut self.frames[slot];
        if f.hl.as_ref().is_none_or(|b| b.size() < size) {
            f.hl = Some(upload(&self.dev, size.next_power_of_two().max(256))?);
        }
        let Some(buf) = f.hl.clone() else { return Err(GpuError::Invalid("Scene::prepare")) };
        let mut data: Vec<u32> = Vec::with_capacity(total as usize);
        let mut list = Vec::with_capacity(merged.len());
        for &(a, e) in &merged {
            list.push((data.len() as u64 * 4, a as u64 * 4, (e - a) as u64 * 4));
            data.extend_from_slice(&self.hl.words[a as usize..e as usize]);
        }
        buf.write(0, &data)?;
        self.up.hl = Some((buf, list));
        self.hl.info.sent_words = total;
        self.hl.info.runs = merged.len() as u32;
        Ok(size)
    }

    /// an entity's bits into the mask (its records' or its instances'); the words it touched
    fn hl_write(&mut self, i: usize, b: u8) -> Option<(u32, u32)> {
        let e = &self.ents[i];
        let (first, n, base) = match e.geom {
            Geom::Insert(_) => {
                let (f, c) = self.ins_range.get(i).copied().unwrap_or((0, 0));
                (f, c, self.hl.inst_base)
            }
            _ => (self.layers[e.layer as usize].base + e.at, e.alloc.max(e.count), 1),
        };
        if n == 0 {
            return None;
        }
        let w = &mut self.hl.words;
        for k in first..first + n {
            let at = (base + k / 16) as usize;
            if at >= w.len() {
                break;
            }
            let sh = 2 * (k % 16);
            w[at] = (w[at] & !(3 << sh)) | ((b as u32) << sh);
        }
        Some((base + first / 16, base + (first + n - 1) / 16 + 1))
    }

    /// (prepare) per visible layer in draw order: it holds a highlighted entity (its pipelines read the bits)
    pub(crate) fn hl_layers(&self) -> Vec<bool> {
        self.drawn.iter().map(|&l| self.hl.count > 0 && self.hl.per_layer.get(l as usize).is_some_and(|&n| n > 0)).collect()
    }
}
