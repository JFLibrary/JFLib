#!/usr/bin/env python3
"""Generates ASMLIB/src/img/jxl_tab.s: the constant tables of the JPEG XL decoder (jxl*.s),
taken from the model tools/jxl_ref.py so that both use identical float32 values.

    python tools/jxl_gen.py
"""

import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import jxl_ref as J  # noqa: E402

OUT = ROOT.parent / "legacy" / "ASMLIB" / "src" / "img" / "jxl_tab.s"


def f32bits(v):
    return struct.unpack("<I", struct.pack("<f", float(J.F32(v))))[0]


class Out:
    def __init__(self):
        self.lines = []
        self.names = []

    def label(self, name, comment=None, align=2):
        if comment:
            self.lines.append("# " + comment)
        self.lines.append("        .p2align %d" % align)
        self.lines.append("        .globl %s" % name)
        self.lines.append("%s:" % name)
        self.names.append(name)

    def floats(self, vals, per=6):
        vals = list(vals)
        for i in range(0, len(vals), per):
            chunk = vals[i:i + per]
            self.lines.append("        .long   " + ", ".join("0x%08x" % f32bits(v) for v in chunk))

    def doubles(self, vals, per=4):
        vals = [struct.unpack("<Q", struct.pack("<d", float(v)))[0] for v in vals]
        for i in range(0, len(vals), per):
            self.lines.append("        .quad   " + ", ".join("0x%016x" % v for v in vals[i:i + per]))

    def ints(self, vals, kind="long", per=12):
        vals = list(vals)
        for i in range(0, len(vals), per):
            self.lines.append("        .%-6s " % kind + ", ".join(str(int(v)) for v in vals[i:i + per]))


def main():
    o = Out()
    # ---- transform data defaults: opsin inverse matrix (9), opsin biases (3), quant biases (4)
    o.label("jxt_opsin_def", "TransformData defaults: opsin inverse matrix, opsin biases, quant biases (f32)")
    o.floats([v for row in J.DEFAULT_INV_OPSIN for v in row] + list(J.DEFAULT_OPSIN_BIAS) + list(J.DEFAULT_QUANT_BIAS))
    o.label("jxt_up2", "default upsampling weights 2x (15), 4x (55), 8x (210), contiguous (f32)")
    o.floats(list(J.UP2_W) + list(J.UP4_W) + list(J.UP8_W))

    # ---- entropy coding
    o.label("jxt_histhuff", "ReadHistogram: (bits, log count) per 7-bit peek (u8 pairs)", 0)
    o.ints([v for pair in J.HIST_HUFF for v in pair], "byte", 16)
    hc = {}
    for idx, (nb, lc) in enumerate(J.HIST_HUFF):
        if lc not in hc:
            hc[lc] = (idx & ((1 << nb) - 1), nb)
    o.label("jxt_histcode", "ReadHistogram static code by log count 0..13: (LSB-first pattern, bits) (writer)", 0)
    o.ints([v for lc in range(14) for v in hc[lc]], "byte", 14)
    o.label("jxt_special", "LZ77 special distances: (dx, dy) per entry (i8 pairs), dist = dx + dy * mult", 0)
    o.ints([v for pair in J.SPECIAL_DIST for v in pair], "byte", 16)

    # ---- ICC profile prediction
    o.label("jxt_iccheader", "ICC header prediction (128 bytes)", 0)
    o.ints(list(J.ICC_HEADER_PRED), "byte", 16)
    o.label("jxt_icckind1", "ICC byte kinds for the context (previous byte)", 0)
    o.ints([J.byte_kind1(b) for b in range(256)], "byte", 32)
    o.label("jxt_icckind2", "ICC byte kinds for the context (byte before)", 0)
    o.ints([J.byte_kind2(b) for b in range(256)], "byte", 32)
    o.label("jxt_icctags", "ICC tag strings (u32 in memory order)")
    o.ints([int.from_bytes(t, "little") for t in J.TAG_STRINGS], "long", 6)
    o.label("jxt_iccxyz", "ICC tags with an implicit size of 20")
    o.ints([int.from_bytes(t, "little") for t in (b"rXYZ", b"gXYZ", b"bXYZ", b"kXYZ", b"wtpt", b"bkpt", b"lumi")],
           "long", 7)
    o.label("jxt_icctypes", "ICC type strings")
    o.ints([int.from_bytes(t, "little") for t in J.TYPE_STRINGS], "long", 8)

    # ---- loop filter defaults in JF layout (JF_GAB .. JF_SIGMOD)
    lf = J.LoopFilter().read(J.Bits(b"\x01"), False)
    o.label("jxt_lfdef", "LoopFilter defaults: gab (u32), gab weights, epf iters (u32), sharpness lut, channel"
            " scale, zeroflush 1 / 2, quant mul, sigma scale 0 / 2, border sad mul, sigma for modular")
    words = [("i", 1)] + [("f", v) for pair in lf.gab_w for v in pair] + [("i", 2)]
    words += [("f", v) for v in lf.epf_sharp_lut] + [("f", v) for v in lf.epf_channel_scale]
    words += [("f", v) for v in (lf.epf_pass1_zeroflush, lf.epf_pass2_zeroflush, lf.epf_quant_mul,
                                 lf.epf_pass0_sigma_scale, lf.epf_pass2_sigma_scale, lf.epf_border_sad_mul,
                                 lf.epf_sigma_for_modular)]
    for kind, v in words:
        o.lines.append("        .long   %s" % (str(v) if kind == "i" else "0x%08x" % f32bits(v)))

    # ---- modular palette
    o.label("jxt_deltapal", "delta palette (i16 x 3 per entry)", 1)
    o.ints([v for e in J.DELTA_PALETTE for v in e], "short", 18)

    # ---- VarDCT
    o.label("jxt_acslog2", "AC strategy: log2 of the covered blocks", 0)
    o.ints(J.ACS_LOG2, "byte", 27)
    o.label("jxt_strategyorder", "AC strategy -> coefficient order bucket", 0)
    o.ints(J.STRATEGY_ORDER, "byte", 27)
    o.label("jxt_acsqt", "AC strategy -> quantization table", 0)
    o.ints(J.ACS_QTABLE, "byte", 27)
    o.label("jxt_qtsx", "quantization table: required size x / y (blocks)", 0)
    o.ints(J.QT_SIZE_X + J.QT_SIZE_Y, "byte", 17)
    o.label("jxt_nzctx", "kCoeffNumNonzeroContext (u16)", 1)
    o.ints([0] + J.COEFF_NZ_CTX[1:], "short", 16)
    o.label("jxt_freqctx", "kCoeffFreqContext", 0)
    o.ints([0] + J.COEFF_FREQ_CTX[1:], "byte", 32)
    o.label("jxt_dm", "x / b dm multiplier for qm_scale 0..7: f32( f32( 1 / 1.25 ) ** (qm - 2) )")
    o.floats([float(J.F32(1) / J.F32(1.25)) ** (k - 2) for k in range(8)], 8)
    o.label("jxt_fastpow", "FastLog2f p0..p2, q0..q2; FastPow2f numerator c0..c2, denominator d0..d3 (f32)")
    o.floats([-1.8503833400518310E-06, 1.4287160470083755E+00, 7.4245873327820566E-01,
              9.9032814277590719E-01, 1.0096718572241148E+00, 1.7409343003366853E-01,
              1.01749063e+01, 4.88687798e+01, 9.85506591e+01,
              2.10242958e-01, -2.22328856e-02, -1.94414990e+01, 9.85506633e+01], 7)
    lo = J.F32(0.8517778890324296)
    o.label("jxt_afv", "AFV quantization: lo, hi = (12.97166202570235 - lo) + 1e-6, frequencies[16] (f32)")
    o.floats([lo, (J.F32(12.97166202570235) - lo) + J.F32(1e-6)] + list(J.AFV_FREQS), 6)
    o.label("jxt_wc", "WcMultipliers<n> for n = 4 .. 256 (n / 2 values each, n at offset n / 2 - 2)")
    wc = []
    n = 4
    while n <= 256:
        wc += list(J.wc_mult(n, 1))
        n *= 2
    o.floats(wc, 8)
    o.label("jxt_rs", "DCTResampleScales< k, 8 k > for k = 1 .. 32 (k values at offset k - 1)")
    rs = []
    for k in (1, 2, 4, 8, 16, 32):
        rs += list(J.RESAMPLE_F32[(k, 8 * k)])
    o.floats(rs, 8)
    o.label("jxt_rsd", "DCTResampleScales< 8 k, k > for k = 1 .. 32 (k values at offset k - 1)")
    rs = []
    for k in (1, 2, 4, 8, 16, 32):
        rs += list(J.RESAMPLE_F32[(8 * k, k)])
    o.floats(rs, 8)
    o.label("jxt_afvbasis", "AFV 4x4 basis [16 coefficients][16 pixels] (f32)", 4)
    o.floats([v for row in J.AFV_BASIS_F32 for v in row], 8)
    # ---- render pipeline
    o.label("jxt_srgb", "TF_SRGB EncodedFromDisplay: p0..p4, q0..q4, threshold 0.0031308, slope 12.92, "
            "gamma floor 1e-5, DCI gamma 2.6 (f32)")
    o.floats([-5.135152395e-04, 5.287254571e-03, 3.903842876e-01, 1.474205315e+00, 7.352629620e-01,
              1.004519624e-02, 3.036675394e-01, 1.340816930e+00, 9.258482155e-01, 2.424867759e-02,
              0.0031308, 12.92, 1e-5, 2.6], 5)
    o.label("jxt_ycc", "YCbCr to RGB: 1.402, 1.772, -0.114, 0.587, -0.299 (f32)")
    o.floats([1.402, 1.772, -0.114, 0.587, -0.299])
    o.label("jxt_noise", "noise: convolution 0.16, -3.84; 0.22, 0.0078125, 0.9921875, 0.5, 6, 7 (f32)")
    o.floats([0.16, -3.84, 0.22, 0.0078125, 0.9921875, 0.5, 6.0, 7.0], 8)
    o.label("jxt_epf", "EPF: f32( 1.65 ), INV_SIGMA_NUM, MIN_SIGMA (f32)")
    o.floats([1.65, J.INV_SIGMA_NUM, J.MIN_SIGMA])
    o.label("jxt_rndd", "render constants (f64): 1.65, 1e-7, luminance 0.2126, 0.7152, 0.0722, 255, 0.5, 1/3", 3)
    o.doubles([1.65, 1e-7, 0.2126, 0.7152, 0.0722, 255.0, 0.5, 1.0 / 3.0])
    # ---- VarDCT writer (tools/jxl_enc_ref.py)
    import jxl_enc_ref as E
    o.label("jxt_srgblin", "writer: sRGB 8-bit -> linear (f32 x 256)")
    o.floats(list(E.srgb_lut()), 8)
    mats = J.quant_table(J.QUANT_LIBRARY[0], 0)
    o.label("jxt_dq8", "writer: DCT8 dequantization multipliers X, Y, B (library table 0, f32 x 3 x 64)")
    o.floats([v for c in range(3) for v in mats[c]], 8)
    x_dm = J.F32(float(J.F32(1) / J.F32(1.25)) ** (3 - 2))
    b_dm = J.F32(float(J.F32(1) / J.F32(1.25)) ** (2 - 2))
    o.label("jxt_venc", "writer (f32): opsin matrix [9], bias, 1/3, 0.5, x_dm, b_dm, kDcQuant, kAcQuant, "
            "0.83, 1/0.3, 0.3, 50, 65536, 5, 6553.6, 1/4096, 1/512, 1/256, thr_y [4], thr_xb [4], 1.125, 1, 32768")
    o.floats([v for row in E.OPSIN_M for v in row] + [E.OPSIN_BIAS, 1.0 / 3.0, 0.5, x_dm, b_dm, E.K_DC_QUANT,
              E.K_AC_QUANT, E.K_DC_POW, 1.0 / 0.3, 0.3, 50.0, 65536.0, 5.0, 4096 * 1.6, 1.0 / 4096, 1.0 / 512,
              1.0 / 256] + E.THR_Y + E.THR_XB + [1.125, 1.0, 32768.0], 6)
    o.label("jxt_vencd", "writer (f64): quality -> distance 0.1, 0.09, 53 / 3000, 23 / 20, 25, 65536", 3)
    o.doubles([0.1, 0.09, 53.0 / 3000.0, 23.0 / 20.0, 25.0, 65536.0])
    o.label("jxt_acclus", "writer: AC context clusters of one block context (458 coefficient contexts)", 0)
    m = E.ac_cluster_map(1)
    o.ints(m[J.NONZERO_BUCKETS:], "byte", 32)
    o.label("jxt_qlib", "DequantMatricesLibraryDef as QE records (528 bytes each)", 3)
    modes = {"id": 1, "dct2": 2, "dct4": 3, "dct4x8": 4, "afv": 5, "dct": 6}
    for enc in J.QUANT_LIBRARY:
        rec = [0.0] * (528 // 4)
        ints = {0: modes[enc[0]], 1: 0, 2: 0}
        mode = enc[0]

        def bands(off, params):
            bl, nb = params
            for c in range(3):
                for i in range(nb):
                    rec[off // 4 + c * 16 + i] = bl[c][i]
            return nb
        if mode == "id":
            for c in range(3):
                for i in range(3):
                    rec[400 // 4 + c * 3 + i] = enc[1][c][i]
        elif mode == "dct2":
            for c in range(3):
                for i in range(6):
                    rec[400 // 4 + c * 6 + i] = enc[1][c][i]
        elif mode == "dct4":
            ints[1] = bands(16, enc[1])
            for c in range(3):
                for i in range(2):
                    rec[400 // 4 + c * 2 + i] = enc[2][c][i]
        elif mode == "dct4x8":
            ints[1] = bands(16, enc[1])
            for c in range(3):
                rec[400 // 4 + c] = enc[2][c]
        elif mode == "afv":
            for c in range(3):
                for i in range(9):
                    rec[400 // 4 + c * 9 + i] = enc[1][c][i]
            ints[1] = bands(16, enc[2])
            ints[2] = bands(208, enc[3])
        else:
            ints[1] = bands(16, enc[1])
        words = []
        for k, v in enumerate(rec):
            words.append("%d" % ints[k] if k in ints else "0x%08x" % f32bits(v))
        for i in range(0, len(words), 8):
            o.lines.append("        .long   " + ", ".join(words[i:i + 8]))

    head = [
        "# jxl_tab.s -- JPEG XL decoder tables (GENERATED by tools/jxl_gen.py from tools/jxl_ref.py;",
        "# do not edit)",
        "#",
        "# " + ", ".join(o.names),
        "",
        "        .intel_syntax noprefix",
        "        .include \"jfgui.inc\"",
        "",
        "        .section .rodata",
    ]
    OUT.write_text("\n".join(head + o.lines) + "\n", encoding="utf-8", newline="\n")
    print("wrote", OUT, len(o.lines), "lines")


if __name__ == "__main__":
    main()
