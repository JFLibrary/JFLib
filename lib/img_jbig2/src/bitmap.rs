// bitmap -- the bilevel images the decoder works on: a byte per pixel (1 = black), outside the
// image 0; composing one onto another with the five combination operators (7.4.1.5)

use crate::Error;
use alloc::vec::Vec;

#[derive(Clone, Debug, Default)]
pub(crate) struct Bmp {
    pub w: usize,
    pub h: usize,
    pub d: Vec<u8>,
}

/// the largest number of pixels of one bitmap (the page, a region, a collective bitmap)
pub(crate) const MAX_PIXELS: u64 = 1 << 30;

/// `d` grown to n bytes of `fill`; Limit when the allocator refuses them.  The growth is amortized
/// (a page of unknown height grows by a stripe at a time and must not be copied at every one)
fn fill_to(d: &mut Vec<u8>, n: usize, fill: u8) -> Result<(), Error> {
    d.try_reserve(n.saturating_sub(d.len())).map_err(|_| Error::Limit)?;
    d.resize(n, fill);
    Ok(())
}

impl Bmp {
    pub fn new(w: usize, h: usize, fill: u8, max: u64) -> Result<Bmp, Error> {
        let n = (w as u64).checked_mul(h as u64).ok_or(Error::Limit)?;
        if n > max.min(MAX_PIXELS) {
            return Err(Error::Limit);
        }
        let mut d = Vec::new();
        fill_to(&mut d, n as usize, fill)?;
        Ok(Bmp { w, h, d })
    }

    #[inline(always)]
    pub fn get(&self, x: i64, y: i64) -> u8 {
        if x < 0 || y < 0 || x >= self.w as i64 || y >= self.h as i64 {
            0
        } else {
            self.d[y as usize * self.w + x as usize]
        }
    }

    /// `src` onto this one at (x, y) with combination operator `op` (0 OR, 1 AND, 2 XOR, 3 XNOR,
    /// 4 REPLACE), clipped to this one
    pub fn compose(&mut self, src: &Bmp, x: i64, y: i64, op: u8) {
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + src.w as i64).min(self.w as i64);
        let y1 = (y + src.h as i64).min(self.h as i64);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        for yy in y0..y1 {
            let srow = &src.d[(yy - y) as usize * src.w + (x0 - x) as usize..][..(x1 - x0) as usize];
            let drow = &mut self.d[yy as usize * self.w + x0 as usize..][..(x1 - x0) as usize];
            match op {
                0 => drow.iter_mut().zip(srow).for_each(|(d, &s)| *d |= s),
                1 => drow.iter_mut().zip(srow).for_each(|(d, &s)| *d &= s),
                2 => drow.iter_mut().zip(srow).for_each(|(d, &s)| *d ^= s),
                3 => drow.iter_mut().zip(srow).for_each(|(d, &s)| *d = (*d ^ s) ^ 1),
                _ => drow.copy_from_slice(srow),
            }
        }
    }

    /// the part w x h at (x, y) (outside: 0), at most `max` pixels
    pub fn sub(&self, x: i64, y: i64, w: usize, h: usize, max: u64) -> Result<Bmp, Error> {
        let mut b = Bmp::new(w, h, 0, max)?;
        if w == 0 || h == 0 {
            return Ok(b);
        }
        for yy in 0..h {
            for xx in 0..w {
                b.d[yy * w + xx] = self.get(x + xx as i64, y + yy as i64);
            }
        }
        Ok(b)
    }

    /// the rows grown to h (new rows filled with `fill`)
    pub fn grow(&mut self, h: usize, fill: u8, max: u64) -> Result<(), Error> {
        if h <= self.h {
            return Ok(());
        }
        let n = (self.w as u64).saturating_mul(h as u64);
        if n > max.min(MAX_PIXELS) {
            return Err(Error::Limit);
        }
        fill_to(&mut self.d, n as usize, fill)?;
        self.h = h;
        Ok(())
    }
}
