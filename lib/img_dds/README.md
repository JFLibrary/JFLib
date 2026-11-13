# jflib-img-dds

DDS reading and writing for [JFLib](https://jflib.org), with the BC1 to BC7 block codecs, in plain
Rust, without the standard library and without a line of `unsafe` code. DDS (DirectDraw Surface)
is the texture file of Direct3D. It holds a texture as the GPU wants it, usually block-compressed,
with its mipmap levels and, for an environment map, the six faces of a cube.

## What it reads

The reader takes the classic header and the newer DX10 header. It knows the legacy pixel formats
in the order DirectXTex recognises them: RGB with any contiguous bit masks in pixels of 8, 16, 24
or 32 bits, luminance, alpha only, signed bump maps, DXT1 to DXT5 (DXT2 and DXT4 premultiplied),
and ATI1, ATI2 and their BC4 and BC5 names, signed or not. Of the DX10 formats it reads BC1 to BC7,
8-bit RGBA, BGRA and BGRX, signed formats, 10:10:10:2, the packed floats R11G11B10 and RGB9E5,
16-bit integers and halves, 32-bit floats, the one- and two-channel formats and the 16-bit 5:6:5,
5:5:5:1 and 4:4:4:4 layouts. Anything else is `ERR_UNSUPPORTED`.

Everything becomes JFLib's 8-bit pixels: integers are scaled, signed values mapped onto 0 to 255,
floats clamped to 0 to 1, and sRGB formats keep their bytes. One channel turns grey, two give red
and green. Premultiplied files come out premultiplied. Each mipmap level is a frame of the file
(`ImgDec::frame`, and `ImgMeta::frames` counts them). A cube map comes out as a horizontal cross of
4 x 3 faces, +Y above, -X, +Z, +X and -Z in the middle row, -Y below; of a texture array the reader
takes the first element, of a volume texture the first slice. Before it allocates a surface it
checks that the data of the chosen level is really there (`ERR_TRUNC`).

## What it writes

The writer writes the header DirectXTex writes for the chosen format (`ImgOpt::ddsfmt`): a legacy
header where one exists, else DX10. The formats are BC1 to BC7, BC4 and BC5 signed, BC6H unsigned
and signed, and uncompressed BGRA, BGRX, RGBA, BGR, 5:5:5:1, 4:4:4:4, 5:6:5, luminance, alpha,
red-green and red; with `IOF_SRGB` the formats that have an sRGB variant are marked as sRGB.
`IOF_MIPMAPS` adds every level down to 1 x 1, each made from the one before with the filter in
`mipfilter`: area average (the default, and for box), bilinear, bicubic, nearest or Lanczos-3,
on premultiplied pixels and, for sRGB textures, in linear light. `IOF_CUBE_CROSS` takes an image
laid out as the reader's cross and writes a cube map of six faces, each with its levels.

The effort chooses the block encoders' speed: 1 to 3 fast, 4 to 6 (and 0) normal, 7 to 9 slow.
For BC7 these are DirectXTex's levels, from mode 6 alone to every mode, rotation and p-bit; BC1 to
BC3 weigh the colour channels as the eye does unless `IOF_DDS_UNIFORM` asks for a plain error, and
BC1's transparency threshold comes from `athresh`. The block rows are encoded in parallel on the
worker pool, and the bytes are the same for any number of threads.

## The block codecs on their own

The BC codecs are usable without a DDS file, for textures a program hands to the GPU itself.
`img_dds::bc_encode(&surface, BCF_BC7, BCQ_NORMAL)` returns the blocks of a whole surface,
`img_dds::decode(&blocks, w, h, BCF_BC7)` turns them back into a surface, and `encode_block` and
`decode_block` work on one 4 x 4 block. `BCF_*` names the ten block formats, `BCQ_*` the speed and
the error metric.

## How it is used

Most apps never call this crate by name. Once the codecs are registered, the image functions of
`img_core` open and save DDS files like any other format:

```rust
use jflib::img_all;
use jflib::img_core::{io, DDSF_BC7, IMGF_DDS, IOF_MIPMAPS, IOF_SRGB};

img_all::use_all_codecs();                        // once, for example in App::init

let mut opt = io::opt_init(IMGF_DDS);             // BC1 without mipmaps
opt.ddsfmt = DDSF_BC7;
opt.flags |= IOF_MIPMAPS | IOF_SRGB;              // all levels, an sRGB texture
opt.effort = 9;                                   // the slowest, best block search
io::save_opt("albedo.dds", &surface, Some(&opt));
```

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::img_dds`. JFLib registers PNG, JPEG, BMP,
GIF and SVG at start-up by itself; DDS needs `img_all::use_all_codecs()` once. All of JFLib's
crates share one version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.0

Nothing in this crate. It is released again as 0.2.0 only because all of JFLib's crates move
together. The [changelog](https://docs.rs/crate/jflib-img-dds/latest/source/CHANGELOG.md) says
the same.

## Documentation

- [How JFLib reads and writes images](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  in the guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#img_dds), also on
  [docs.rs](https://docs.rs/jflib-img-dds)

## License

MIT or Apache-2.0, at your choice. The crate follows DirectXTex, which is under the MIT License,
for the BC7 speed levels, the channel weights of BC1 to BC3, the order in which legacy pixel
formats are recognised and the header values it writes; the block codecs themselves are JFLib's
own. DirectXTex's license text is in the crate's
[`licenses/`](https://github.com/JFLibrary/JFLib/tree/main/lib/img_dds/licenses) folder.
