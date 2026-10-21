#!/usr/bin/env python3
"""Generate the constant tables of the assembly AV1 decoder and encoder from tools/av1_tables.py.

    python tools/av1_gen.py

writes ASMLIB/src/img/av1_tab.s (data, .rodata), ASMLIB/inc/av1_tab.inc (CDF context layout:
CDF_* byte offsets of every CDF array in the context, CDFN_* symbol counts), ASMLIB/src/img/av1_txfn.s
(inverse butterflies of the decoder, from tools/av1_ref.py) and ASMLIB/src/img/av1_ftx.s (forward
butterflies and tables of the encoder, from tools/av1_enc_ref.py).  Rerun after changing the table
selection or the models; the generated files are checked in.

CDF context: the non-coefficient CDFs used by intra frames, then the coefficient CDFs (CDF_COEF ..
CDF_SIZE); each CDF array is stored as in the specification (N values, the last 32768, then the
adaptation counter) as u16.  av1t_cdf_nc holds the defaults of the first part, av1t_cdf_coef four
copies of the second part (the init_coeff_cdfs index).
"""

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
import av1_tables as T  # noqa: E402

OUT_S = ROOT.parent / "legacy" / "ASMLIB" / "src" / "img" / "av1_tab.s"
OUT_TX = ROOT.parent / "legacy" / "ASMLIB" / "src" / "img" / "av1_txfn.s"
OUT_INC = ROOT.parent / "legacy" / "ASMLIB" / "inc" / "av1_tab.inc"
OUT_FTX = ROOT.parent / "legacy" / "ASMLIB" / "src" / "img" / "av1_ftx.s"

# (context name, default table) -- non-coefficient part
NC = [
    ("IFYMODE", "Default_Intra_Frame_Y_Mode_Cdf"),
    ("UVMODE_NC", "Default_Uv_Mode_Cfl_Not_Allowed_Cdf"),
    ("UVMODE_C", "Default_Uv_Mode_Cfl_Allowed_Cdf"),
    ("ANGLE", "Default_Angle_Delta_Cdf"),
    ("PART8", "Default_Partition_W8_Cdf"),
    ("PART16", "Default_Partition_W16_Cdf"),
    ("PART32", "Default_Partition_W32_Cdf"),
    ("PART64", "Default_Partition_W64_Cdf"),
    ("PART128", "Default_Partition_W128_Cdf"),
    ("SEGID", "Default_Segment_Id_Cdf"),
    ("TX8", "Default_Tx_8x8_Cdf"),
    ("TX16", "Default_Tx_16x16_Cdf"),
    ("TX32", "Default_Tx_32x32_Cdf"),
    ("TX64", "Default_Tx_64x64_Cdf"),
    ("FIMODE", "Default_Filter_Intra_Mode_Cdf"),
    ("FILTERINTRA", "Default_Filter_Intra_Cdf"),
    ("SKIP", "Default_Skip_Cdf"),
    ("PALYMODE", "Default_Palette_Y_Mode_Cdf"),
    ("PALUVMODE", "Default_Palette_Uv_Mode_Cdf"),
    ("PALYSIZE", "Default_Palette_Y_Size_Cdf"),
    ("PALUVSIZE", "Default_Palette_Uv_Size_Cdf"),
] + [("PALY%d" % n, "Default_Palette_Size_%d_Y_Color_Cdf" % n) for n in range(2, 9)] \
  + [("PALUV%d" % n, "Default_Palette_Size_%d_Uv_Color_Cdf" % n) for n in range(2, 9)] + [
    ("DELTAQ", "Default_Delta_Q_Cdf"),
    ("DELTALF", "Default_Delta_Lf_Cdf"),
    ("DELTALFM", None),                 # 4 copies of Default_Delta_Lf_Cdf
    ("ITX1", "Default_Intra_Tx_Type_Set1_Cdf"),
    ("ITX2", "Default_Intra_Tx_Type_Set2_Cdf"),
    ("CFLSIGN", "Default_Cfl_Sign_Cdf"),
    ("CFLALPHA", "Default_Cfl_Alpha_Cdf"),
    ("USEWIENER", "Default_Use_Wiener_Cdf"),
    ("USESGR", "Default_Use_Sgrproj_Cdf"),
    ("RESTYPE", "Default_Restoration_Type_Cdf"),
]
COEF = [
    ("TXBSKIP", "Default_Txb_Skip_Cdf"),
    ("EOB16", "Default_Eob_Pt_16_Cdf"),
    ("EOB32", "Default_Eob_Pt_32_Cdf"),
    ("EOB64", "Default_Eob_Pt_64_Cdf"),
    ("EOB128", "Default_Eob_Pt_128_Cdf"),
    ("EOB256", "Default_Eob_Pt_256_Cdf"),
    ("EOB512", "Default_Eob_Pt_512_Cdf"),
    ("EOB1024", "Default_Eob_Pt_1024_Cdf"),
    ("EOBEXTRA", "Default_Eob_Extra_Cdf"),
    ("DCSIGN", "Default_Dc_Sign_Cdf"),
    ("CBASEEOB", "Default_Coeff_Base_Eob_Cdf"),
    ("CBASE", "Default_Coeff_Base_Cdf"),
    ("CBR", "Default_Coeff_Br_Cdf"),
]


import av1_ref as R  # noqa: E402


def record_ops(fn, n):
    """the B / H calls of one model transform (the permutation steps are not recorded)"""
    ops = []
    oB, oH = R._B, R._H

    def B(T_, a, b, angle, flip):
        assert 0 <= a < 64 and 0 <= b < 64 and 0 <= angle < 256
        ops.append((a, b, angle, 2 if flip else 0))
        oB(T_, a, b, angle, flip)

    def H(T_, a, b, flip, r):
        if flip:
            a, b = b, a
        ops.append((a, b, 0, 1))
        oH(T_, a, b, 0, r)
    R._B, R._H = B, H
    try:
        T_ = [0] * (1 << n)
        if fn == "idct":
            R.idct(T_, n, 20)
        else:
            getattr(R, fn)(T_, 20)
    finally:
        R._B, R._H = oB, oH
    return ops


def flat(v):
    return [x for y in v for x in flat(y)] if isinstance(v, list) else [v]


def shape(v):
    s = []
    while isinstance(v, list):
        s.append(len(v))
        v = v[0]
    return s


def emit(f, label, vals, kind, per=16, glob=True):
    d = {"u8": ".byte", "i8": ".byte", "u16": ".short", "i16": ".short", "u32": ".long", "i32": ".long"}[kind]
    lo, hi = {"u8": (0, 255), "i8": (-128, 127), "u16": (0, 65535), "i16": (-32768, 32767),
              "u32": (0, 2**32 - 1), "i32": (-2**31, 2**31 - 1)}[kind]
    for v in vals:
        assert lo <= v <= hi, (label, v)
    if glob:
        f.write("        .globl  %s\n" % label)
    f.write("%s:\n" % label)
    for i in range(0, len(vals), per):
        f.write("        %s   %s\n" % (d, ", ".join(str(v) for v in vals[i:i + per])))


def main():
    inc = ["# av1_tab.inc -- generated by tools/av1_gen.py: layout of the AV1 CDF context (do not edit)", ""]
    nc_vals = []
    off = 0
    for name, tab in NC:
        if tab is None:
            v = [T.Default_Delta_Lf_Cdf] * T.CONST["FRAME_LF_COUNT"]
        else:
            v = getattr(T, tab)
        fl = flat(v)
        sh = shape(v)
        inc.append("CDF_%-14s = %5d   # %s  N = %d" % (name, off, sh, sh[-1] - 1))
        inc.append("CDFN_%-13s = %5d" % (name, sh[-1] - 1))
        nc_vals += fl
        off += 2 * len(fl)
    nc_size = off
    inc.append("CDF_COEF           = %5d   # start of the coefficient CDFs" % nc_size)
    coef_vals = [[] for _ in range(4)]
    for name, tab in COEF:
        v = getattr(T, tab)
        sh = shape(v[0])
        inc.append("CDF_%-14s = %5d   # %s  N = %d" % (name, off, sh, sh[-1] - 1))
        inc.append("CDFN_%-13s = %5d" % (name, sh[-1] - 1))
        for q in range(4):
            coef_vals[q] += flat(v[q])
        off += 2 * len(flat(v[0]))
    inc.append("CDF_COEFSIZE       = %5d" % (off - nc_size))
    inc.append("CDF_SIZE           = %5d   # bytes of a CDF context" % ((off + 15) & -16))
    inc.append("")
    OUT_INC.write_text("\n".join(inc) + "\n", encoding="utf-8", newline="\n")

    with open(OUT_S, "w", encoding="utf-8", newline="\n") as f:
        f.write("# av1_tab.s -- constant tables of the AV1 decoder (generated by tools/av1_gen.py from the\n"
                "# AV1 specification tables in tools/av1_tables.py; do not edit)\n\n"
                "        .intel_syntax noprefix\n\n        .section .rodata\n        .p2align 4\n")
        f.write("# default non-coefficient CDFs (layout: av1_tab.inc)\n")
        emit(f, "av1t_cdf_nc", nc_vals, "u16")
        f.write("        .p2align 4\n# default coefficient CDFs, 4 x CDF_COEFSIZE bytes (base_q_idx classes)\n")
        emit(f, "av1t_cdf_coef", sum(coef_vals, []), "u16")
        u8 = ["Num_4x4_Blocks_Wide", "Num_4x4_Blocks_High", "Mi_Width_Log2", "Mi_Height_Log2", "Size_Group",
              "Max_Tx_Size_Rect", "Max_Tx_Depth", "Partition_Subsize", "Subsampled_Size", "Split_Tx_Size",
              "Tx_Width", "Tx_Height", "Tx_Width_Log2", "Tx_Height_Log2", "Tx_Size_Sqr", "Tx_Size_Sqr_Up",
              "Adjusted_Tx_Size", "Transform_Row_Shift", "Mode_To_Txfm", "Intra_Mode_Context",
              "Filter_Intra_Mode_To_Intra_Dir", "Tx_Type_In_Set_Intra", "Tx_Type_Intra_Inv_Set1",
              "Tx_Type_Intra_Inv_Set2", "Palette_Color_Hash_Multipliers",
              "Coeff_Base_Ctx_Offset", "Coeff_Base_Pos_Ctx_Offset", "Sm_Weights_Tx_4x4", "Sm_Weights_Tx_8x8",
              "Sm_Weights_Tx_16x16", "Sm_Weights_Tx_32x32", "Sm_Weights_Tx_64x64", "Intra_Edge_Kernel",
              "Quantizer_Matrix", "Cdef_Uv_Dir", "Cdef_Pri_Taps", "Cdef_Sec_Taps", "Sgr_Params",
              "Segmentation_Feature_Bits", "Segmentation_Feature_Signed", "Segmentation_Feature_Max",
              "Remap_Lr_Type", "Wiener_Taps_K"]
        i8 = ["Palette_Color_Context", "Intra_Filter_Taps", "Mag_Ref_Offset_With_Tx_Class", "Sig_Ref_Diff_Offset", "Cdef_Directions",
              "Wiener_Taps_Min", "Wiener_Taps_Max", "Wiener_Taps_Mid", "Sgrproj_Xqd_Min", "Sgrproj_Xqd_Max",
              "Sgrproj_Xqd_Mid"]
        u16 = ["Mode_To_Angle", "Dr_Intra_Derivative", "Dc_Qlookup", "Ac_Qlookup", "Qm_Offset", "Cos128_Lookup",
               "Div_Table"]
        i16 = ["Upscale_Filter", "Gaussian_Sequence"]
        for kind, names in (("u8", u8), ("i8", i8), ("u16", u16), ("i16", i16)):
            for n in names:
                f.write("        .p2align 4\n# %s %s\n" % (n, T.DIMS[n]))
                emit(f, "av1t_" + n.lower(), flat(getattr(T, n)), kind)
        for n in range(2, 7):
            f.write("        .p2align 2\n")
            emit(f, "av1t_brev%d" % n, [R.brev(n, i) for i in range(1 << n)], "u8")
        # ADST permutations: input index; output index | 0x80 when negated
        for n in (3, 4):
            n0 = 1 << n
            emit(f, "av1t_adst_in%d" % n0, [(i - 1) if (i & 1) else (n0 - i - 1) for i in range(n0)], "u8")
            outp = []
            for i in range(n0):
                a = (i >> 3) & 1
                b = ((i >> 2) & 1) ^ ((i >> 3) & 1)
                c = ((i >> 1) & 1) ^ ((i >> 2) & 1)
                d = (i & 1) ^ ((i >> 1) & 1)
                idx = ((d << 3) | (c << 2) | (b << 1) | a) >> (4 - n)
                outp.append(idx | (0x80 if i & 1 else 0))
            emit(f, "av1t_adst_out%d" % n0, outp, "u8")
        # scan tables (u16), one label each
        for n in T.DIMS:
            if "_Scan_" in n:
                f.write("        .p2align 4\n# %s\n" % n)
                emit(f, "av1t_" + n.lower(), flat(getattr(T, n)), "u16")
    write_txfn()
    write_ftx()
    print("CDF context: %d bytes (non-coefficient %d) -> %s, %s, %s, %s" % ((off + 15) & -16, nc_size, OUT_S,
                                                                            OUT_INC, OUT_TX, OUT_FTX))


TXFNS = [("dct4", "idct", 2), ("dct8", "idct", 3), ("dct16", "idct", 4), ("dct32", "idct", 5),
         ("dct64", "idct", 6), ("adst8", "iadst8", 3), ("adst16", "iadst16", 4)]


def write_txfn():
    """av1_txfn.s: the butterfly part of the 1D inverse DCT / ADST8 / ADST16 as straight-line code"""
    with open(OUT_TX, "w", encoding="utf-8", newline="\n") as f:
        f.write("# av1_txfn.s -- butterfly stages of the AV1 inverse DCT4 .. DCT64, ADST8 and ADST16 as straight-line\n"
                "# code (generated by tools/av1_gen.py from the B / H calls of tools/av1_ref.py; do not edit)\n#\n")
        for name, _, n in TXFNS:
            f.write("#   av1_tx_%s(T, hi)\n" % name)
        f.write("#\n# T: i32[ 1 << n ], already permuted (DCT: bit reversal, ADST: input permutation; the ADST\n"
                "# output permutation is the caller's); hi = ( 1 << ( r - 1 ) ) - 1, H( ) results are clamped\n"
                "# to [ -hi - 1, hi ].  B( a, b, angle ): Round2( Ta * cos - Tb * sin, 12 ), Round2( Ta * sin + Tb * cos, 12 )\n"
                "# (64-bit products), flip = exchange.  Leaf functions.\n\n"
                "        .intel_syntax noprefix\n\n        .text\n")
        for name, fn, n in TXFNS:
            ops = record_ops(fn, n)
            f.write("\n# av1_tx_%s(T, hi) -- %d butterflies; leaf\n        .globl  av1_tx_%s\nav1_tx_%s:\n"
                    % (name, len(ops), name, name))
            f.write("        mov     r11d, edx\n        mov     r10d, edx\n        not     r10d\n")
            for a, b, angle, flags in ops:
                if flags & 1:
                    f.write("        mov     eax, [rcx + %d]\n        mov     edx, [rcx + %d]\n" % (a * 4, b * 4))
                    f.write("        lea     r8d, [rax + rdx]\n        sub     eax, edx\n")
                    for r in ("r8d", "eax"):
                        f.write("        cmp     %s, r10d\n        cmovl   %s, r10d\n        cmp     %s, r11d\n"
                                "        cmovg   %s, r11d\n" % (r, r, r, r))
                    f.write("        mov     [rcx + %d], r8d\n        mov     [rcx + %d], eax\n" % (a * 4, b * 4))
                else:
                    c = R.cos128(angle)
                    sn = R.sin128(angle)
                    f.write("        movsxd  r8, DWORD PTR [rcx + %d]\n        movsxd  r9, DWORD PTR [rcx + %d]\n"
                            % (a * 4, b * 4))
                    f.write("        imul    rax, r8, %d\n        imul    rdx, r9, %d\n        sub     rax, rdx\n"
                            "        add     rax, 2048\n        sar     rax, 12\n" % (c, sn))
                    f.write("        imul    r8, r8, %d\n        imul    r9, r9, %d\n        add     r8, r9\n"
                            "        add     r8, 2048\n        sar     r8, 12\n" % (sn, c))
                    if flags & 2:
                        f.write("        mov     [rcx + %d], r8d\n        mov     [rcx + %d], eax\n" % (a * 4, b * 4))
                    else:
                        f.write("        mov     [rcx + %d], eax\n        mov     [rcx + %d], r8d\n" % (a * 4, b * 4))
            f.write("        ret\n")


def write_ftx():
    """av1_ftx.s: the encoder's forward butterflies (tools/av1_enc_ref.py: the model's B / H calls in
    reverse order, rotations by the negated angle, H without the halving; inverse( forward( x ) ) =
    N / 2 * x) and the encoder's tables"""
    import av1_enc_ref as E
    with open(OUT_FTX, "w", encoding="utf-8", newline="\n") as f:
        f.write("# av1_ftx.s -- forward DCT4 .. DCT64, ADST8 and ADST16 butterflies of the AV1 encoder as\n"
                "# straight-line code, and the encoder's tables (generated by tools/av1_gen.py from\n"
                "# tools/av1_enc_ref.py; do not edit)\n#\n")
        for name, _, n in TXFNS:
            f.write("#   av1_ftx_%s(T)\n" % name)
        f.write("#   av1e_cost       u16[ 1024 ] Q8 bits of a symbol of probability ( i + 0.5 ) / 1024\n"
                "#   av1e_kq         u32[ TX_SIZES_ALL ] quantizer scale K (Q16) per transform size\n"
                "#\n# T: i32[ 1 << n ] in place, no clamping (exact for 8 / 10-bit residuals); the permutations are\n"
                "# the caller's (DCT: result i belongs to coefficient brev( n, i ); ADST: input index outp( i )\n"
                "# with the sign of odd i, result i to coefficient i - 1 (odd i) / N - 1 - i).  B( a, b, -angle )\n"
                "# with 64-bit products, H( a, b ): Ta + Tb, Ta - Tb.  Leaf functions.\n\n"
                "        .intel_syntax noprefix\n\n        .text\n")
        for name, _, n in TXFNS:
            kind = "dct" if name.startswith("dct") else "adst"
            ops = E.FWD[(kind, n)]
            f.write("\n# av1_ftx_%s(T) -- %d butterflies; leaf\n        .globl  av1_ftx_%s\nav1_ftx_%s:\n"
                    % (name, len(ops), name, name))
            for k, a, b, angle, flip in ops:
                if k == 1:
                    f.write("        mov     eax, [rcx + %d]\n        mov     edx, [rcx + %d]\n" % (a * 4, b * 4))
                    f.write("        lea     r8d, [rax + rdx]\n        sub     eax, edx\n")
                    f.write("        mov     [rcx + %d], r8d\n        mov     [rcx + %d], eax\n" % (a * 4, b * 4))
                else:
                    ang = (-angle) & 255
                    c = R.cos128(ang)
                    sn = R.sin128(ang)
                    u, v = (b, a) if flip else (a, b)
                    f.write("        movsxd  r8, DWORD PTR [rcx + %d]\n        movsxd  r9, DWORD PTR [rcx + %d]\n"
                            % (u * 4, v * 4))
                    f.write("        imul    rax, r8, %d\n        imul    rdx, r9, %d\n        sub     rax, rdx\n"
                            "        add     rax, 2048\n        sar     rax, 12\n" % (c, sn))
                    f.write("        imul    r8, r8, %d\n        imul    r9, r9, %d\n        add     r8, r9\n"
                            "        add     r8, 2048\n        sar     r8, 12\n" % (sn, c))
                    f.write("        mov     [rcx + %d], eax\n        mov     [rcx + %d], r8d\n" % (a * 4, b * 4))
            f.write("        ret\n")
        f.write("\n        .section .rodata\n        .p2align 4\n")
        emit(f, "av1e_cost", E.COST, "u16")
        f.write("        .p2align 4\n")
        emit(f, "av1e_kq", E.KQ16, "u32")


if __name__ == "__main__":
    main()
