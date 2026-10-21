#!/usr/bin/env python3
"""Generates ASMLIB/src/img/jxl_epf.s: the JPEG XL edge preserving filter (jr_epf, ep_run) with the
per-row SSE kernels ep_k0 .. ep_k2 written out in the operation order of the model tools/jxl_ref.py
(epf0, epf1, epf2), so that the decoder stays bit-exact to it.

    python tools/jxl_epfgen.py
"""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'legacy/ASMLIB/src/img/jxl_epf.s'

ROWREG = {0: None, 1: "rax", 2: "rax * 2", -1: "rdx", -2: "rdx * 2", 3: "rcx", -3: "r11"}
M = ["r8", "r9", "r10"]
ACC = ["xmm6", "xmm7", "xmm8"]


def addr(c, dy, dx):
    parts = [M[c]]
    if ROWREG[dy]:
        parts.append(ROWREG[dy])
    if dx:
        parts.append(str(dx * 4) if dx > 0 else None)
    s = " + ".join(p for p in parts if p)
    if dx < 0:
        s += " - %d" % (-dx * 4)
    return "[%s]" % s


L = []
emit = L.append


def kernel_head(name, comment):
    emit("# %s" % comment)
    emit("%s:" % name)
    emit("        PROLOGX 0")
    emit("        mov     r15, rcx                        # EPC")
    emit("        mov     r8, [r15 + EPC_M]")
    emit("        mov     r9, [r15 + EPC_M + 8]")
    emit("        mov     r10, [r15 + EPC_M + 16]")
    emit("        mov     r12, [r15 + EPC_O]")
    emit("        mov     r13, [r15 + EPC_O + 8]")
    emit("        mov     r14, [r15 + EPC_O + 16]")
    emit("        mov     rbx, [r15 + EPC_ISG]")
    emit("        mov     rsi, [r15 + EPC_SKIP]")
    emit("        mov     edi, [r15 + EPC_N]")
    emit("        mov     eax, [r15 + EPC_PS]             # row offsets: ps, -ps, 3 ps, -3 ps")
    emit("        mov     rdx, rax")
    emit("        neg     rdx")
    emit("        lea     rcx, [rax + rax * 2]")
    emit("        mov     r11, rcx")
    emit("        neg     r11")
    emit("        test    edi, edi")
    emit("        jz      .%s_out" % name[-2:])
    emit(".%s_l:" % name[-2:])
    for c in range(3):
        emit("        movups  %s, [%s]" % (ACC[c], M[c]))
    emit("        movaps  xmm9, [rip + ep_ones]           # wsum")
    emit("        movups  xmm10, [rbx]                    # inv sigma")


def weight_and_acc(src, oy, ox):
    """wt = max( S inv_sigma + 1, 0 ) from src; wsum += wt; acc = wt * q( oy, ox ) + acc"""
    if src != "xmm0":
        emit("        movaps  xmm0, %s" % src)
    emit("        mulps   xmm0, xmm10")
    emit("        addps   xmm0, [rip + ep_ones]")
    emit("        maxps   xmm0, [rip + ep_zero]")
    emit("        addps   xmm9, xmm0")
    for c in range(3):
        emit("        movups  xmm2, %s" % addr(c, oy, ox))
        emit("        mulps   xmm2, xmm0")
        emit("        addps   %s, xmm2" % ACC[c])


def kernel_tail(name):
    t = name[-2:]
    emit("        movaps  xmm0, [rip + ep_ones]           # finish: skip ? p : acc / wsum")
    emit("        divps   xmm0, xmm9")
    emit("        movups  xmm1, [rsi]")
    for c, o in enumerate(["r12", "r13", "r14"]):
        emit("        mulps   %s, xmm0" % ACC[c])
        emit("        movups  xmm2, [%s]" % M[c])
        emit("        andps   xmm2, xmm1")
        emit("        movaps  xmm3, xmm1")
        emit("        andnps  xmm3, %s" % ACC[c])
        emit("        orps    xmm2, xmm3")
        emit("        movups  [%s], xmm2" % o)
    for r in M + ["r12", "r13", "r14", "rbx", "rsi"]:
        emit("        add     %s, 16" % r)
    emit("        dec     edi")
    emit("        jnz     .%s_l" % t)
    emit(".%s_out:" % t)
    emit("        EPILOGX")
    emit("")


def absdiff(dst, a, b, tmp):
    emit("        movups  %s, %s" % (dst, a))
    emit("        movups  %s, %s" % (tmp, b))
    emit("        subps   %s, %s" % (dst, tmp))
    emit("        andps   %s, [rip + ep_absmask]" % dst)


# ---------------------------------------------------------------- EPF0
OFFS0 = [(-2, 0), (-1, -1), (-1, 0), (-1, 1), (0, -2), (0, -1), (0, 1), (0, 2), (1, -1), (1, 0), (1, 1), (2, 0)]
PLUS = [(0, 0), (-1, 0), (0, -1), (1, 0), (0, 1)]
kernel_head("ep_k0", "ep_k0(epc) -- EPF stage 0 on one row: 12 directions, SAD over a plus of 5 samples")
for (oy, ox) in OFFS0:
    emit("        xorps   xmm0, xmm0                      # direction (%d, %d)" % (oy, ox))
    for c in range(3):
        emit("        xorps   xmm1, xmm1")
        for (py, px) in PLUS:
            absdiff("xmm2", addr(c, py, px), addr(c, oy + py, ox + px), "xmm3")
            emit("        addps   xmm1, xmm2")
        emit("        mulps   xmm1, [r15 + EPC_SC + %d]" % (16 * c))
        emit("        addps   xmm0, xmm1")
    weight_and_acc("xmm0", oy, ox)
kernel_tail("ep_k0")

# ---------------------------------------------------------------- EPF1
kernel_head("ep_k1", "ep_k1(epc) -- EPF stage 1 on one row: 4 directions with shared SAD terms")
emit("        xorps   xmm11, xmm11                    # S0..S3: xmm11, EPC_T + 0 .. 47")
emit("        movaps  [r15 + EPC_T], xmm11")
emit("        movaps  [r15 + EPC_T + 16], xmm11")
emit("        movaps  [r15 + EPC_T + 32], xmm11")
P = {"p20": (-2, 0), "p21": (-1, 0), "p11": (-1, -1), "p31": (-1, 1), "p02": (0, -2), "p12": (0, -1),
     "p22": (0, 0), "p32": (0, 1), "p42": (0, 2), "p13": (1, -1), "p23": (1, 0), "p33": (1, 1), "p24": (2, 0)}
for c in range(3):
    A = lambda n: addr(c, *P[n])
    # s0 xmm0, s1 xmm1, s2 xmm2, s3 xmm3, t xmm4, tmp xmm5
    absdiff("xmm0", A("p20"), A("p21"), "xmm5")
    absdiff("xmm1", A("p11"), A("p21"), "xmm5")
    absdiff("xmm2", A("p31"), A("p21"), "xmm5")
    absdiff("xmm4", A("p02"), A("p12"), "xmm5")
    emit("        addps   xmm1, xmm4")
    absdiff("xmm4", A("p11"), A("p12"), "xmm5")
    emit("        addps   xmm0, xmm4")
    absdiff("xmm4", A("p12"), A("p22"), "xmm5")
    emit("        addps   xmm1, xmm4")
    emit("        addps   xmm2, xmm4")
    absdiff("xmm4", A("p22"), A("p21"), "xmm5")
    emit("        movaps  xmm3, xmm4")
    emit("        addps   xmm0, xmm4")
    absdiff("xmm4", A("p31"), A("p32"), "xmm5")
    emit("        addps   xmm0, xmm4")
    absdiff("xmm4", A("p22"), A("p32"), "xmm5")
    emit("        addps   xmm1, xmm4")
    emit("        addps   xmm2, xmm4")
    absdiff("xmm4", A("p42"), A("p32"), "xmm5")
    emit("        addps   xmm2, xmm4")
    absdiff("xmm4", A("p13"), A("p12"), "xmm5")
    emit("        addps   xmm3, xmm4")
    absdiff("xmm4", A("p22"), A("p23"), "xmm5")
    emit("        addps   xmm0, xmm4")
    emit("        addps   xmm3, xmm4")
    absdiff("xmm4", A("p13"), A("p23"), "xmm5")
    emit("        addps   xmm1, xmm4")
    absdiff("xmm4", A("p33"), A("p23"), "xmm5")
    emit("        addps   xmm2, xmm4")
    absdiff("xmm4", A("p33"), A("p32"), "xmm5")
    emit("        addps   xmm3, xmm4")
    absdiff("xmm4", A("p24"), A("p23"), "xmm5")
    emit("        addps   xmm3, xmm4")
    emit("        movaps  xmm5, [r15 + EPC_SC + %d]" % (16 * c))
    emit("        mulps   xmm0, xmm5")
    emit("        addps   xmm11, xmm0")
    for i, r in ((1, "xmm1"), (2, "xmm2"), (3, "xmm3")):
        emit("        mulps   %s, xmm5" % r)
        emit("        addps   %s, [r15 + EPC_T + %d]" % (r, 16 * (i - 1)))
        emit("        movaps  [r15 + EPC_T + %d], %s" % (16 * (i - 1), r))
for i, (oy, ox) in enumerate(((-1, 0), (0, -1), (0, 1), (1, 0))):
    src = "xmm11" if i == 0 else "[r15 + EPC_T + %d]" % (16 * (i - 1))
    if i:
        emit("        movaps  xmm0, %s" % src)
        weight_and_acc("xmm0", oy, ox)
    else:
        weight_and_acc("xmm11", oy, ox)
kernel_tail("ep_k1")

# ---------------------------------------------------------------- EPF2
kernel_head("ep_k2", "ep_k2(epc) -- EPF stage 2 on one row: 4 directions, SAD of the 3 channels")
for (oy, ox) in ((-1, 0), (0, -1), (0, 1), (1, 0)):
    for c in range(3):
        absdiff("xmm1", addr(c, oy, ox), addr(c, 0, 0), "xmm3")
        emit("        mulps   xmm1, [r15 + EPC_SC + %d]" % (16 * c))
        if c == 0:
            emit("        movaps  xmm0, xmm1")
        else:
            emit("        addps   xmm0, xmm1")
    weight_and_acc("xmm0", oy, ox)
kernel_tail("ep_k2")

KERNELS = "\n".join(L)

HEAD = r'''# jxl_epf.s -- JPEG XL edge preserving filter (stage_epf.cc; model: tools/jxl_ref.py epf0, epf1, epf2,
# epf_sigma_pixels, epf_weight, epf_finish)
#
#   jr_epf(jx, jf, jrn) -> eax 0 / ERR_*    the frame's EPF iterations on the 3 colour planes (in place)
#
# Generated kernels (ep_k0 .. ep_k2, four samples per SSE vector) follow the model's operation order:
# weights max( sad * inv_sigma + 1, 0 ), accumulation wt * q + acc, result acc * (1 / wsum).

        .intel_syntax noprefix
        .include "jfgui.inc"
        .include "jxl_int.inc"

        .text
        .globl jr_epf

# row context of the kernels
EPC_M           = 0             # u64[3] padded planes at (row y, column 0)
EPC_O           = 24            # u64[3] output rows
EPC_ISG         = 48            # u64 f32 inverse sigma * SAD multiplier per sample
EPC_SKIP        = 56            # u64 u32 masks (sigma below the minimum: keep the sample)
EPC_N           = 64            # u32 vectors per row
EPC_PS          = 68            # u32 padded row bytes
EPC_SC          = 80            # f32x4[3] channel scales
EPC_T           = 128           # f32x4[3] scratch
EPC_SIZE        = 176

# jr_epf(jx, jf, jrn) -> eax 0 / ERR_*
jr_epf:
        PROLOG  0
        mov     rbx, rdx
        mov     r12, r8
        cmp     dword ptr [rbx + JF_EPFIT], 3
        jb      .ef_1
        mov     rcx, rbx
        mov     rdx, r12
        xor     r8d, r8d
        call    ep_run
        test    eax, eax
        jnz     .ef_out
.ef_1:
        mov     rcx, rbx
        mov     rdx, r12
        mov     r8d, 1
        call    ep_run
        test    eax, eax
        jnz     .ef_out
        xor     eax, eax
        cmp     dword ptr [rbx + JF_EPFIT], 2
        jb      .ef_out
        mov     rcx, rbx
        mov     rdx, r12
        mov     r8d, 2
        call    ep_run
.ef_out:
        EPILOG

# ep_run(jf, jrn, stage) -> eax 0 / ERR_NOMEM -- one EPF stage over the colour planes
ER_EPC          = 64            # EPC (16-aligned)
ER_PAD          = ER_EPC + EPC_SIZE     # u64[3] padded planes
ER_ROWS         = ER_PAD + 24   # u64 inverse sigma row + skip row
ER_SM           = ER_ROWS + 8   # f32 multiplier inside blocks
ER_BSM          = ER_SM + 4     # f32 multiplier on block borders
ER_SIG          = ER_BSM + 4    # f32 constant inverse sigma (modular)
ER_P            = ER_SIG + 4    # u32 border
ER_STAGE        = ER_P + 4      # u32
ER_Y            = ER_STAGE + 4  # u32
ER_S            = ER_Y + 4      # u32 row floats (multiple of 4)
ER_END          = (ER_S + 4 + 15) & -16
ep_run:
        PROLOG  ER_END - 64
        mov     rbx, rcx                        # jf
        mov     r15, rdx                        # jrn
        mov     [rsp + ER_STAGE], r8d
        mov     eax, 3
        sub     eax, r8d
        mov     [rsp + ER_P], eax
        xor     eax, eax
        mov     [rsp + ER_PAD], rax
        mov     [rsp + ER_PAD + 8], rax
        mov     [rsp + ER_PAD + 16], rax
        mov     [rsp + ER_ROWS], rax
        # SAD multipliers: stage 0 f32( f32( pass0 scale ) * 1.65 ), stage 1 f32( 1.65 ), stage 2 pass2
        movss   xmm0, [rip + jxt_epf]
        cmp     r8d, 1
        je      .er_sm
        movss   xmm0, [rbx + JF_P0SS]
        test    r8d, r8d
        jz      .er_smd
        movss   xmm0, [rbx + JF_P2SS]
.er_smd:
        cvtss2sd xmm0, xmm0
        mulsd   xmm0, [rip + jxt_rndd]
        cvtsd2ss xmm0, xmm0
.er_sm:
        movss   [rsp + ER_SM], xmm0
        mulss   xmm0, [rbx + JF_BSAD]
        movss   [rsp + ER_BSM], xmm0
        movss   xmm0, [rip + jxt_epf + 4]       # modular: INV_SIGMA_NUM / sigma_for_modular
        divss   xmm0, [rbx + JF_SIGMOD]
        movss   [rsp + ER_SIG], xmm0
        xor     esi, esi                        # channel scales, padded planes
.er_pd:
        movss   xmm0, [rbx + JF_EPFCS + rsi * 4]
        shufps  xmm0, xmm0, 0
        mov     eax, esi
        shl     eax, 4
        movaps  [rsp + ER_EPC + EPC_SC + rax], xmm0
        imul    ecx, esi, JRP_SIZE
        lea     rcx, [r15 + JRN_P + rcx]
        mov     edx, [rsp + ER_P]
        call    jr_pad
        test    rax, rax
        jz      .er_nomem
        mov     [rsp + ER_PAD + rsi * 8], rax
        inc     esi
        cmp     esi, 3
        jb      .er_pd
        mov     eax, [r15 + JRN_P + JRP_W]
        add     eax, 3
        and     eax, -4
        mov     [rsp + ER_S], eax
        shr     eax, 2
        mov     [rsp + ER_EPC + EPC_N], eax
        mov     eax, [r15 + JRN_P + JRP_S]      # padded row bytes (S + 2 p) * 4
        add     eax, [rsp + ER_P]
        add     eax, [rsp + ER_P]
        shl     eax, 2
        mov     [rsp + ER_EPC + EPC_PS], eax
        mov     ecx, [rsp + ER_S]
        shl     ecx, 3
        add     ecx, 64
        call    mem_alloc
        test    rax, rax
        jz      .er_nomem
        mov     [rsp + ER_ROWS], rax
        mov     [rsp + ER_EPC + EPC_ISG], rax
        mov     ecx, [rsp + ER_S]
        lea     rax, [rax + rcx * 4]
        mov     [rsp + ER_EPC + EPC_SKIP], rax
        mov     dword ptr [rsp + ER_Y], 0
.er_y:
        mov     eax, [rsp + ER_Y]
        cmp     eax, [r15 + JRN_P + JRP_H]
        jae     .er_ok
        # inverse sigma row: sig( block ) * (row or column on a block border ? bsm : sm), skip mask
        mov     r12, [rsp + ER_EPC + EPC_ISG]
        mov     r13, [rsp + ER_EPC + EPC_SKIP]
        mov     r14d, eax
        and     r14d, 7                         # iy
        xor     edi, edi                        # x
.er_x:
        cmp     edi, [rsp + ER_S]
        jae     .er_rows
        movss   xmm0, [rsp + ER_SIG]
        cmp     dword ptr [rbx + JF_MODULAR], 0
        jne     .er_sg
        mov     rcx, [rbx + JF_VD]
        mov     eax, [rsp + ER_Y]
        shr     eax, 3
        imul    eax, [rbx + JF_XB]
        mov     edx, edi
        shr     edx, 3
        add     eax, edx
        mov     rcx, [rcx + JV_ISIG]
        movss   xmm0, [rcx + rax * 4]
.er_sg:
        xor     eax, eax
        comiss  xmm0, [rip + jxt_epf + 8]       # sig < MIN_SIGMA
        jae     .er_ns
        dec     eax
.er_ns:
        mov     [r13 + rdi * 4], eax
        movss   xmm1, [rsp + ER_BSM]
        test    r14d, r14d
        jz      .er_mul
        cmp     r14d, 7
        je      .er_mul
        mov     eax, edi
        and     eax, 7
        jz      .er_mul
        cmp     eax, 7
        je      .er_mul
        movss   xmm1, [rsp + ER_SM]
.er_mul:
        mulss   xmm0, xmm1
        movss   [r12 + rdi * 4], xmm0
        inc     edi
        jmp     .er_x
.er_rows:
        xor     esi, esi                        # row pointers
.er_rp:
        mov     eax, [rsp + ER_Y]
        add     eax, [rsp + ER_P]
        mov     ecx, [rsp + ER_EPC + EPC_PS]
        imul    rax, rcx
        mov     ecx, [rsp + ER_P]
        lea     rax, [rax + rcx * 4]
        add     rax, [rsp + ER_PAD + rsi * 8]
        mov     [rsp + ER_EPC + EPC_M + rsi * 8], rax
        imul    ecx, esi, JRP_SIZE
        lea     rcx, [r15 + JRN_P + rcx]
        mov     eax, [rcx + JRP_S]
        mov     edx, [rsp + ER_Y]
        imul    rax, rdx
        shl     rax, 2
        add     rax, [rcx + JRP_D]
        mov     [rsp + ER_EPC + EPC_O + rsi * 8], rax
        inc     esi
        cmp     esi, 3
        jb      .er_rp
        lea     rcx, [rsp + ER_EPC]
        mov     eax, [rsp + ER_STAGE]
        test    eax, eax
        jz      .er_k0
        cmp     eax, 1
        je      .er_k1
        call    ep_k2
        jmp     .er_yn
.er_k0:
        call    ep_k0
        jmp     .er_yn
.er_k1:
        call    ep_k1
.er_yn:
        inc     dword ptr [rsp + ER_Y]
        jmp     .er_y
.er_ok:
        xor     eax, eax
        jmp     .er_out
.er_nomem:
        mov     eax, ERR_NOMEM
.er_out:
        mov     esi, eax
        xor     edi, edi
.er_fr:
        mov     rcx, [rsp + ER_PAD + rdi * 8]
        call    mem_free
        inc     edi
        cmp     edi, 3
        jb      .er_fr
        mov     rcx, [rsp + ER_ROWS]
        call    mem_free
        mov     eax, esi
        EPILOG

'''

TAIL = '''
        .section .rodata
        .p2align 4
ep_ones:        .long   0x3f800000, 0x3f800000, 0x3f800000, 0x3f800000
ep_zero:        .long   0, 0, 0, 0
ep_absmask:     .long   0x7FFFFFFF, 0x7FFFFFFF, 0x7FFFFFFF, 0x7FFFFFFF
'''

OUT.write_text(HEAD + KERNELS + TAIL, encoding='utf-8', newline='\n')
print("wrote", OUT, len((HEAD + KERNELS + TAIL).splitlines()), "lines")
