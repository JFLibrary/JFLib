// Shared page coordinate conversion. Rendering, pointer input and thumbnails use one scale.

fn bounded(value: i64) -> i32 {
    value.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// An isotropic page transform from twips to an absolute pixel rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageTransform {
    width_twips: u32,
    rect: [i32; 4],
}

impl PageTransform {
    /// Create a transform for a nonempty page and pixel rectangle.
    pub fn new(width_twips: u32, rect: [i32; 4]) -> Option<Self> {
        if width_twips == 0 || rect[2] <= 0 || rect[3] <= 0 {
            None
        } else {
            Some(Self { width_twips, rect })
        }
    }

    /// Scale a twip length to pixels, truncating as in the existing document renderer.
    pub fn scale(&self, twips: i32) -> i32 {
        bounded(twips as i64 * self.rect[2] as i64 / self.width_twips as i64)
    }

    /// Convert a pixel offset from the page edge to twips.
    pub fn unscale(&self, pixels: i32) -> i32 {
        bounded(pixels as i64 * self.width_twips as i64 / self.rect[2] as i64)
    }

    /// Convert a page-relative twip point to an absolute pixel point.
    pub fn to_pixels(&self, point: [i32; 2]) -> [i32; 2] {
        [
            bounded(self.rect[0] as i64 + self.scale(point[0]) as i64),
            bounded(self.rect[1] as i64 + self.scale(point[1]) as i64),
        ]
    }

    /// Convert an absolute pixel point to page-relative twips, including points outside it.
    pub fn to_page(&self, point: [i32; 2]) -> [i32; 2] {
        // Two i32 coordinates may differ by more than i32::MAX. Compute the
        // multiplication in i128 so even public, adversarial geometry stays bounded.
        let convert = |axis: usize| {
            let twips = (point[axis] as i128 - self.rect[axis] as i128) * self.width_twips as i128
                / self.rect[2] as i128;
            twips.clamp(i32::MIN as i128, i32::MAX as i128) as i32
        };
        [convert(0), convert(1)]
    }

    /// Scale a half-point font size to 24.8 pixel units.
    pub fn font_size(&self, half_points: u16) -> i32 {
        bounded(half_points as i64 * 10 * 256 * self.rect[2] as i64 / self.width_twips as i64)
    }
}

/// Page width in DIP at a zoom percentage, clamped to the supported 10–500 percent range.
pub fn zoom_width(width_twips: u32, percent: i32) -> i32 {
    bounded(width_twips as i64 * percent.clamp(10, 500) as i64 / 1500).max(1)
}

/// Preserve a page's aspect ratio when its width is changed; invalid page widths return zero.
pub fn page_height(width_twips: u32, height_twips: u32, width: i32) -> i32 {
    if width_twips == 0 || width <= 0 {
        0
    } else {
        bounded(height_twips as i64 * width as i64 / width_twips as i64).max(1)
    }
}
