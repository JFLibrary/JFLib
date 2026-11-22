// stream -- compression and decompression in pieces, with bounded memory
//
//   Decode::decode(input, out) -> Ok(Step)   take input, write into the caller's buffer
//   Decode::finish()                          Ok when the input so far ended a whole stream
//   Encode::encode(input, out, flush) -> Ok(Step)
//   Step { read, written, status }            Status::Input (all input taken, give more),
//                                             Output (out is full, call again), End
//   Flush::None / Sync / Finish               encoders: buffer, make all input so far
//                                             decodable, or end the stream
//   decode_all(dec, read, write, limit)       pump a whole stream from a reader to a writer
//   encode_all(enc, read, write)
//
// The decoders and encoders are deflate::, zlib::, gzip::, zstd::, lz4:: (FrameDecoder,
// FrameEncoder) and brotli::Decoder.  Each keeps only what its format needs to go on: the
// window a back reference may reach (32 KiB for DEFLATE, the frame's for zstd and Brotli, 64
// KiB for LZ4), the bytes of one block where the format sizes its blocks (zstd, LZ4) and at
// most a kilobyte of input otherwise.  Every call writes at most out.len() bytes, so the
// caller decides how much output a piece of input may produce.
//
// The rules for a decoder:
//   - Status::Input: every byte of input was taken (read == input.len()); call again with
//     more.  If there is no more, the stream is truncated (finish() says so).
//   - Status::Output: out is full; call again with the input from `read` on.
//   - Status::End: a whole stream ends at input[read]; every byte of it has been written.
//     DEFLATE, zlib and Brotli hold one stream, so input after it is not taken.  gzip, zstd
//     and LZ4 files may hold several in a row: more input goes on with the next member or
//     frame, as their tools read them (Decoder::single() stops after the first).
//   - An error is final; the decoder must not be used again (reset() starts it anew).
// Decoders take untrusted input: damaged data gives an Error, never a panic, a read outside
// the input or memory beyond the format's window.

/// how far one call got
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    /// bytes of input taken
    pub read: usize,
    /// bytes written to the output buffer
    pub written: usize,
    pub status: Status,
}

/// why a call returned
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// all input was taken: give more, or end (finish / Flush::Finish)
    Input,
    /// the output buffer is full: call again with room, and the input from `read` on
    Output,
    /// the stream is complete: everything of it is written (decoders: `read` stops at its end)
    End,
}

/// what an encoder does with the input it has
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flush {
    /// collect input as the format compresses best (output comes in blocks)
    None,
    /// compress everything so far so that a decoder can give all of it back (the stream goes
    /// on: DEFLATE ends it with an empty stored block, zstd and LZ4 with a whole block)
    Sync,
    /// compress the rest and end the stream (call again while it returns Status::Output)
    Finish,
}

/// why a stream could not be read or written
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// the input ended inside a stream
    Truncated,
    /// the data is damaged, or is no stream of the format
    Corrupt,
    /// a checksum or a stored size does not match the data
    Checksum,
    /// the stream needs something this crate does not read: a zstd or LZ4 dictionary, a
    /// Brotli large window
    Unsupported,
    /// more memory or output than allowed: a zstd window above the decoder's limit, more
    /// output than a pump's limit
    Limit,
    /// the encoder got more or less input than the size it was given
    Size,
    /// a pump's reader or writer failed
    Io,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Error::Truncated => "the stream is truncated",
            Error::Corrupt => "the stream is damaged",
            Error::Checksum => "the checksum does not match",
            Error::Unsupported => "the stream uses a feature that is not supported",
            Error::Limit => "the stream needs more than allowed",
            Error::Size => "the input does not have the size given",
            Error::Io => "reading or writing failed",
        })
    }
}

impl From<crate::deflate::Error> for Error {
    fn from(e: crate::deflate::Error) -> Error {
        use crate::deflate::Error as D;
        match e {
            D::Truncated => Error::Truncated,
            D::Checksum => Error::Checksum,
            _ => Error::Corrupt,
        }
    }
}

/// a decoder that takes its input in pieces
pub trait Decode {
    /// decode from `input` into `out` (see the module's rules for the Step)
    fn decode(&mut self, input: &[u8], out: &mut [u8]) -> Result<Step, Error>;
    /// Ok when the input so far ended a whole stream (and everything is written), else
    /// Error::Truncated
    fn finish(&self) -> Result<(), Error>;
}

/// an encoder that takes its input in pieces
pub trait Encode {
    /// take `input` (fewer bytes when out fills up), write compressed bytes into `out`.
    /// Status::Input: all input taken and, for Sync and Finish, all output of it written;
    /// Output: call again (with the same flush); End: Finish is done.
    fn encode(&mut self, input: &[u8], out: &mut [u8], flush: Flush) -> Result<Step, Error>;
}

const PUMP: usize = 64 << 10;

/// decode a whole stream: `read(buf)` gives input (Some(0) at its end, None on an error),
/// `write(bytes)` takes output (false on an error); at most `limit` bytes are written.  The
/// input must be complete streams and nothing else -> the number of bytes written.
pub fn decode_all<D: Decode + ?Sized>(
    dec: &mut D,
    mut read: impl FnMut(&mut [u8]) -> Option<usize>,
    mut write: impl FnMut(&[u8]) -> bool,
    limit: u64,
) -> Result<u64, Error> {
    let mut inb = alloc::vec![0u8; PUMP];
    let mut outb = alloc::vec![0u8; PUMP];
    let (mut have, mut off, mut eof, mut total) = (0usize, 0usize, false, 0u64);
    loop {
        if off == have && !eof {
            have = read(&mut inb).ok_or(Error::Io)?.min(PUMP);
            off = 0;
            eof = have == 0;
        }
        let s = dec.decode(&inb[off..have], &mut outb)?;
        off += s.read;
        if s.written > 0 {
            total += s.written as u64;
            if total > limit {
                return Err(Error::Limit);
            }
            if !write(&outb[..s.written]) {
                return Err(Error::Io);
            }
        }
        match s.status {
            Status::Output => {}
            Status::Input if !eof => {}
            Status::Input => return dec.finish().map(|_| total),
            Status::End if off < have => {
                if s.read == 0 && s.written == 0 {
                    return Err(Error::Corrupt); // bytes after a stream that holds one
                }
            }
            Status::End if eof => return Ok(total),
            Status::End => {}
        }
    }
}

/// compress everything `read(buf)` gives (Some(0) at the end, None on an error) into
/// `write(bytes)` (false on an error) and end the stream -> the number of bytes written
pub fn encode_all<E: Encode + ?Sized>(
    enc: &mut E,
    mut read: impl FnMut(&mut [u8]) -> Option<usize>,
    mut write: impl FnMut(&[u8]) -> bool,
) -> Result<u64, Error> {
    let mut inb = alloc::vec![0u8; PUMP];
    let mut outb = alloc::vec![0u8; PUMP];
    let (mut have, mut off, mut eof, mut total) = (0usize, 0usize, false, 0u64);
    loop {
        if off == have && !eof {
            have = read(&mut inb).ok_or(Error::Io)?.min(PUMP);
            off = 0;
            eof = have == 0;
        }
        let flush = if eof { Flush::Finish } else { Flush::None };
        let s = enc.encode(&inb[off..have], &mut outb, flush)?;
        off += s.read;
        if s.written > 0 {
            total += s.written as u64;
            if !write(&outb[..s.written]) {
                return Err(Error::Io);
            }
        }
        if s.status == Status::End {
            return Ok(total);
        }
    }
}

// ---- shared parts of the decoders and encoders

/// output that waits for the caller: `buf[from..]`; deliver() copies as much as fits
pub(crate) fn deliver(buf: &[u8], from: &mut usize, end: usize, out: &mut [u8], written: &mut usize) {
    let n = (end - *from).min(out.len() - *written);
    out[*written..*written + n].copy_from_slice(&buf[*from..*from + n]);
    *from += n;
    *written += n;
}

/// collect `want` bytes in `buf` from input[*read..] -> true when they are all there
pub(crate) fn gather(buf: &mut alloc::vec::Vec<u8>, want: usize, input: &[u8], read: &mut usize) -> bool {
    if buf.len() < want {
        let k = (want - buf.len()).min(input.len() - *read);
        buf.extend_from_slice(&input[*read..*read + k]);
        *read += k;
    }
    buf.len() >= want
}

/// the bit-level decoders' input: a step (a block header, a symbol, ...) runs on the input
/// only when it can finish there; when it runs out it starts again with more.  `Stage`
/// keeps the bytes of the step that ran out (at most STAGE_MAX, more than any step of
/// DEFLATE needs) and the bits of the first one already used.
pub(crate) struct Stage {
    pub buf: alloc::vec::Vec<u8>,
    pub bit0: usize,
}

/// the outcome of an engine's run
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Stop {
    /// the next step needs more input (the bit position is at its start)
    Need,
    /// no room for the next step's output
    Full,
    /// the stream ended (the bit position after its last bit)
    End,
}

/// a bit-level decoder engine: run steps on src from bit *bit on
pub(crate) trait Engine {
    fn run(&mut self, src: &[u8], bit: &mut usize) -> Result<Stop, Error>;
    /// make room for output (the caller has taken everything); false when there is none
    fn room(&mut self) -> bool;
    /// copy waiting output into out
    fn deliver(&mut self, out: &mut [u8], written: &mut usize);
    fn waiting(&self) -> bool;
    fn ended(&self) -> bool;
}

/// one Decode::decode call of a bit-level decoder: the staged bytes first (the step that
/// ran out, with as much new input as it may need), then the input itself.  A staged byte
/// is always one of the stream's (the step that ran out needed every one of them), so at
/// the end of a stream `read` stops where it does.
pub(crate) fn drive<E: Engine>(eng: &mut E, stage: &mut Stage, max: usize, input: &[u8], out: &mut [u8]) -> Result<Step, Error> {
    let mut written = 0;
    let mut ib = 0usize; // bits of input taken
    loop {
        eng.deliver(out, &mut written);
        if eng.waiting() {
            return Ok(Step { read: finish_bits(input, &mut ib, stage), written, status: Status::Output });
        }
        if eng.ended() {
            return Ok(Step { read: ib.div_ceil(8), written, status: Status::End });
        }
        if !eng.room() {
            return Ok(Step { read: finish_bits(input, &mut ib, stage), written, status: Status::Output });
        }
        if !stage.buf.is_empty() {
            let old = stage.buf.len();
            let t = (max - old).min(input.len());
            stage.buf.extend_from_slice(&input[..t]);
            let mut bit = stage.bit0;
            let r = eng.run(&stage.buf, &mut bit)?;
            if bit >= old * 8 {
                // past the staged bytes: go on in the input
                ib = bit - old * 8;
                stage.buf.clear();
                stage.bit0 = 0;
                continue;
            }
            if r == Stop::Need {
                if t < input.len() {
                    return Err(Error::Corrupt); // a step longer than any of the format's
                }
                stage.buf.drain(..bit / 8);
                stage.bit0 = bit % 8;
                return Ok(Step { read: input.len(), written, status: Status::Input });
            }
            stage.buf.truncate(old);
            stage.buf.drain(..bit / 8);
            stage.bit0 = bit % 8;
            continue;
        }
        let mut bit = ib;
        let r = eng.run(input, &mut bit)?;
        ib = bit;
        if r == Stop::Need {
            stage.buf.clear();
            stage.buf.extend_from_slice(&input[ib / 8..]);
            stage.bit0 = ib % 8;
            eng.deliver(out, &mut written);
            let status = if eng.waiting() { Status::Output } else { Status::Input };
            return Ok(Step { read: input.len(), written, status });
        }
    }
}

// the bytes taken when a call stops mid-stream: a partly used byte goes to the stage
fn finish_bits(input: &[u8], ib: &mut usize, stage: &mut Stage) -> usize {
    if !stage.buf.is_empty() {
        return 0; // nothing of this input taken yet (ib is 0)
    }
    if *ib % 8 != 0 {
        stage.buf.push(input[*ib / 8]);
        stage.bit0 = *ib % 8;
        return *ib / 8 + 1;
    }
    *ib / 8
}
