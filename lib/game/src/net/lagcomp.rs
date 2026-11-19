// lagcomp -- hits with lag compensation: the server keeps where the figures were over the last ticks and,
// for a shot, puts them back to the time the shooter saw them, at most 300 ms back unless set (the interpolation's
// 100 ms and 200 ms beyond them, the user's decision of 2026-11-19)
//
//   view_time(tick, view, max_back)   the shooter's view time from its input (NetInput::view: the lag of
//                                     its picture in 1/256 ticks), clamped to max_back ticks before tick
//   HitHistory      the centres of the figures' hit volumes per tick (a ring); rewind(view time) blends the
//                   two recorded ticks around it as a client blends two snapshots
//   hitscan         a ray against the map (physics::World::ray_cast) and the rewound figures
//                   (physics::query::ray against each capsule): the nearest hit, the shooter left out
//
// What the client saw and what the server rewinds differ by the quantisation of the snapshots (under a
// millimetre) and, when a figure turns within a snapshot's interval, by blending ticks one apart where
// the client blended snapshots two apart (docs/NETCODE.md, "Treffer").

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use physics::{Filter, Iso, Shape, Vec3};

use super::clock::floor;
use super::snapshot::tick_diff;

/// the view time (a tick and the fraction towards the next) of an input of `tick` whose picture lagged
/// `view` / 256 ticks, at most `max_back` ticks back
pub fn view_time(tick: u32, view: u16, max_back: u32) -> (u32, f64) {
    let back = (view as f64 / 256.0).min(max_back as f64);
    let w = floor(back);
    let f = back - w;
    if f == 0.0 {
        (tick.wrapping_sub(w as u32), 0.0)
    } else {
        (tick.wrapping_sub(w as u32 + 1), 1.0 - f)
    }
}

/// the figures' hit volumes over the last ticks (see the module header)
#[derive(Clone, Debug)]
pub struct HitHistory {
    ticks: VecDeque<(u32, Vec<(u16, Vec3)>)>,
    cap: usize,
}

impl HitHistory {
    /// remember `cap` ticks
    pub fn new(cap: usize) -> HitHistory {
        HitHistory { ticks: VecDeque::new(), cap: cap.max(2) }
    }

    /// where the figures (id, centre of the hit volume) are after `tick`; ids ascending
    pub fn record(&mut self, tick: u32, figures: &[(u16, Vec3)]) {
        let mut v = figures.to_vec();
        v.sort_by_key(|f| f.0);
        self.ticks.push_back((tick, v));
        while self.ticks.len() > self.cap {
            self.ticks.pop_front();
        }
    }

    /// the oldest and newest tick remembered
    pub fn span(&self) -> Option<(u32, u32)> {
        Some((self.ticks.front()?.0, self.ticks.back()?.0))
    }

    fn find(&self, tick: u32) -> Option<&Vec<(u16, Vec3)>> {
        let (first, _) = self.span()?;
        let k = tick_diff(tick, first);
        if k < 0 {
            return None;
        }
        let e = self.ticks.get(k as usize)?;
        if e.0 == tick { Some(&e.1) } else { self.ticks.iter().find(|x| x.0 == tick).map(|x| &x.1) }
    }

    /// the figures at view time (tick, frac): blended between tick and tick + 1 where both have them,
    /// clamped to the ticks remembered; false when nothing is remembered
    pub fn rewind(&self, tick: u32, frac: f64, out: &mut Vec<(u16, Vec3)>) -> bool {
        out.clear();
        let Some((first, last)) = self.span() else { return false };
        let (t0, f) = if tick_diff(tick, first) < 0 {
            (first, 0.0)
        } else if tick_diff(tick, last) >= 0 {
            (last, 0.0)
        } else {
            (tick, frac)
        };
        let Some(a) = self.find(t0) else { return false };
        let b = if f > 0.0 { self.find(t0.wrapping_add(1)) } else { None };
        for &(id, p) in a {
            let q = b.and_then(|b| b.binary_search_by_key(&id, |x| x.0).ok().map(|i| b[i].1));
            out.push((id, match q {
                Some(q) => Vec3::new(p.x + (q.x - p.x) * f, p.y + (q.y - p.y) * f, p.z + (q.z - p.z) * f),
                None => p,
            }));
        }
        true
    }
}

/// what a shot hit
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    /// a figure: its id, the distance along the ray and the point
    Figure { id: u16, t: f64, point: Vec3 },
    /// the map
    World { t: f64, point: Vec3 },
}

/// a ray from `origin` along `dir` (unit length) up to `max_t` against the map and the figures (`shape`
/// at each centre), the figure `shooter` left out: the nearest hit
pub fn hitscan(map: &physics::World, figures: &[(u16, Vec3)], shape: &Shape, shooter: u16, origin: Vec3, dir: Vec3, max_t: f64) -> Option<Hit> {
    let wall = map.ray_cast(origin, dir, max_t, &Filter::ALL).map(|(_, h)| h.t);
    let mut best: Option<Hit> = wall.map(|t| Hit::World { t, point: origin + dir * t });
    let mut limit = wall.unwrap_or(max_t);
    for &(id, c) in figures {
        if id == shooter {
            continue;
        }
        if let Some(h) = physics::query::ray(shape, &Iso::at(c), origin, dir, limit) {
            // the nearest; at the same distance the smaller id (figures come in id order)
            if h.t < limit || (h.t == limit && !matches!(best, Some(Hit::Figure { .. }))) {
                limit = h.t;
                best = Some(Hit::Figure { id, t: h.t, point: h.point });
            }
        }
    }
    best
}
