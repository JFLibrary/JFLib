# jflib-img-av1

AV1 intra coding for [JFLib](https://jflib.org): the decoder and the encoder for the still
pictures inside AVIF files, in plain Rust, without the standard library and without a line of
`unsafe` code. The crate works on AV1 data and sample planes, not on files. The AVIF container
around it is read by [`jflib-img-heif`](https://crates.io/crates/jflib-img-heif) and written by
[`jflib-img-avif`](https://crates.io/crates/jflib-img-avif).

## What it decodes

The decoder follows the AV1 Bitstream and Decoding Process Specification for everything a still
picture can use: profiles 0 to 2, so 8, 10 and 12 bits in 4:0:0, 4:2:0, 4:2:2 and 4:4:4, key
frames and intra-only frames with every intra tool, and the whole filter chain of deblocking,
CDEF, superres, loop restoration and film grain (`Av1Dec::set_nograin` leaves the grain out).
It decodes the first shown frame of operating point 0 and ignores whatever follows.

What a still image does not need is refused with `ERR_UNSUPPORTED` rather than decoded wrongly:
inter and switch frames, frames that are not shown, intra block copy and large-scale tiles. A
frame of more than 2^27 luma samples is refused with `ERR_LIMIT` before anything is allocated,
and memory is reserved so that running out of it is `ERR_NOMEM`, not an abort. The loop filters
run on JFLib's worker pool, and the pixels are the same for any number of threads. A cancel flag
is checked once per superblock.

## What it encodes

The encoder writes one key frame with a reduced still-picture header, the form AVIF items use:
profile 0 for 4:0:0 and 4:2:0, profile 1 for 4:4:4, 8 or 10 bits, with the colour description it
is given. It searches the partition and the intra mode of every block with estimated rates, cuts
the picture into uniform tiles and encodes them in parallel. A quantizer index of 0 is lossless;
otherwise the deblocking filter's strength follows the quantizer. CDEF, loop restoration,
superres and film grain are not used. The result is the OBUs of the sequence header and the
frame, with the length of the sequence header (the `av1C` box needs it), the profile and the
level.

## How it is used

Apps rarely call this crate by name. Once `img_all::use_all_codecs()` has registered AVIF,
`img_core::io` opens and saves AVIF files like any other format, and this crate does the AV1
part underneath (see [`jflib-img-avif`](https://crates.io/crates/jflib-img-avif)).

The crate's own API is there for work on AV1 data itself. `Av1Dec::new()` makes a decoder,
`obus(data)` takes the OBUs of the sequence header and the frame, and `finish()` returns the
decoded `Pic`: 16-bit sample planes with their bit depth, subsampling and colour description.
`img_av1::encode(&EncIn)` goes the other way, from sample planes to OBUs.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_av1`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-img-av1/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [How JFLib reads and writes images](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_av1), also on
  [docs.rs](https://docs.rs/jflib-img-av1)

## License

MIT or Apache-2.0, at your choice. The encoder's range coder, its lossless transform and its
choice of the deblocking strength follow libaom, which is under the BSD 2-Clause License with the
Alliance for Open Media Patent License 1.0; both texts are in the crate's
[`licenses/`](https://github.com/JFLibrary/JFLib/tree/main/lib/img_av1/licenses) folder.
