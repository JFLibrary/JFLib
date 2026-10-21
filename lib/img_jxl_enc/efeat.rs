// efeat -- the image features a frame of the new writer may carry, written at the start of
// its LfGlobal section in the order the decoder reads them: patches (epatch.rs), splines
// (test-only, espline.rs), noise (photon noise, enoise.rs)  (libjxl enc_frame.cc
// EncodeGlobalDCInfo's callers: PatchDictionaryEncoder::Encode, EncodeSplines, EncodeNoise)
//                                                                         [jxl-tools: no asm]

use crate::ewr::Jw;
use crate::frm::{JFF_NOISE, JFF_PATCHES, JFF_SPLINES};

/// the features of one frame
#[derive(Default, Clone, Copy)]
pub struct Feat<'a> {
    pub patches: Option<&'a crate::epatch::Dict>,
    pub splines: Option<&'a crate::spline::Quantized>,
    pub noise: Option<[f32; 8]>,
}

impl Feat<'_> {
    /// the FrameHeader flags of the features
    pub fn flags(&self) -> u64 {
        let mut f = 0u64;
        if self.patches.is_some() {
            f |= JFF_PATCHES;
        }
        if self.splines.is_some() {
            f |= JFF_SPLINES;
        }
        if self.noise.is_some() {
            f |= JFF_NOISE;
        }
        f
    }

    /// the features' part of LfGlobal
    pub fn write(&self, lf: &mut Jw) {
        if let Some(p) = self.patches {
            p.write(lf);
        }
        if let Some(s) = self.splines {
            crate::espline::write(s, lf);
        }
        if let Some(n) = self.noise.as_ref() {
            crate::enoise::write(n, lf);
        }
    }
}
