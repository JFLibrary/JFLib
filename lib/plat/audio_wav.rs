// audio_wav -- (audio.rs) RIFF WAVE files into f32 samples
//
//   parse(bytes) -> Result<(rate, channels, samples), AudioError>
//
// Formats: PCM 8 (unsigned), 16, 24 and 32 bits, IEEE float 32 and 64 bits, also as
// WAVE_FORMAT_EXTENSIBLE (its sub-format decides); 1 ..= 8 channels, 1 kHz ..= 384 kHz.  Chunks
// may come in any order, unknown ones (LIST, fact, cue, ..) are skipped, odd sizes are padded.
// A data chunk that claims more than the file holds is cut to the whole frames that are there
// (common in files written while recording).  Anything else is AudioError::Format: no RIFF /
// WAVE, no fmt or data chunk, other codecs (ADPCM, A-law, MP3 in WAVE), a block size that does
// not match the channels and bits, 0 channels.

use alloc::vec::Vec;

use super::AudioError;

const TAG_PCM: u16 = 1;
const TAG_FLOAT: u16 = 3;
const TAG_EXTENSIBLE: u16 = 0xFFFE;

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// a WAVE file -> (frames per second, channels, interleaved samples in -1 ..= 1)
pub(crate) fn parse(b: &[u8]) -> Result<(u32, u16, Vec<f32>), AudioError> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err(AudioError::Format);
    }
    let mut fmt: Option<&[u8]> = None;
    let mut data: Option<&[u8]> = None;
    let mut at = 12;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let size = u32_at(b, at + 4).ok_or(AudioError::Format)? as usize;
        let body = at + 8;
        let end = body.saturating_add(size).min(b.len());
        match id {
            b"fmt " if fmt.is_none() => fmt = Some(&b[body..end]),
            b"data" if data.is_none() => data = Some(&b[body..end]),
            _ => {}
        }
        at = body.saturating_add(size).saturating_add(size & 1);
    }
    let (f, d) = (fmt.ok_or(AudioError::Format)?, data.ok_or(AudioError::Format)?);
    let mut tag = u16_at(f, 0).ok_or(AudioError::Format)?;
    let ch = u16_at(f, 2).ok_or(AudioError::Format)?;
    let rate = u32_at(f, 4).ok_or(AudioError::Format)?;
    let align = u16_at(f, 12).ok_or(AudioError::Format)? as usize;
    let bits = u16_at(f, 14).ok_or(AudioError::Format)?;
    if tag == TAG_EXTENSIBLE {
        // cbSize, valid bits, channel mask, then the sub-format GUID (its first two bytes)
        tag = u16_at(f, 24).ok_or(AudioError::Format)?;
    }
    if !(1..=8).contains(&ch) || !(1000..=384_000).contains(&rate) {
        return Err(AudioError::Format);
    }
    let bytes = match (tag, bits) {
        (TAG_PCM, 8) => 1,
        (TAG_PCM, 16) => 2,
        (TAG_PCM, 24) => 3,
        (TAG_PCM, 32) | (TAG_FLOAT, 32) => 4,
        (TAG_FLOAT, 64) => 8,
        _ => return Err(AudioError::Format),
    };
    if align != bytes * ch as usize {
        return Err(AudioError::Format);
    }
    let frames = d.len() / align;
    let mut out = Vec::new();
    out.try_reserve_exact(frames * ch as usize).map_err(|_| AudioError::Format)?;
    for s in d[..frames * align].chunks_exact(bytes) {
        out.push(match (tag, bytes) {
            (TAG_PCM, 1) => (s[0] as f32 - 128.0) / 128.0,
            (TAG_PCM, 2) => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
            (TAG_PCM, 3) => (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0,
            (TAG_PCM, _) => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
            (_, 4) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
            _ => f64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]) as f32,
        });
    }
    Ok((rate, ch, out))
}

/// a 16-bit PCM WAVE file of interleaved samples (clipped to -1 ..= 1)
pub(crate) fn write16(rate: u32, ch: u16, samples: &[f32]) -> Vec<u8> {
    let data = samples.len() * 2;
    let mut b = Vec::with_capacity(44 + data);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&((36 + data) as u32).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&TAG_PCM.to_le_bytes());
    b.extend_from_slice(&ch.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * ch as u32 * 2).to_le_bytes());
    b.extend_from_slice(&(ch * 2).to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data as u32).to_le_bytes());
    for &s in samples {
        let v = rt::fx::cvt(s.clamp(-1.0, 1.0) * 32767.0) as i16;
        b.extend_from_slice(&v.to_le_bytes());
    }
    b
}
