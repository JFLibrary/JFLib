
// hl_write -- the write pass with highlights (G6d; docs/RENDER2D.md "Hervorheben"): lib.rs puts it behind cull.wgsl.  As
// write, and each visible record's two bits (selected, hovered) into bits 29 and 30 of its word in the visible list (a
// pool's places stay below 2^29): a record's own bits, or an insert's record its instance's.  hl: u32 words of 16 pairs,
// word 0 the first word of the instances', the records' from word 1 (scene hl.rs)

@group(0) @binding(15) var<storage, read> hl: array<u32>;

fn hl_bits(at: u32, k: u32) -> u32 {
    return (hl[at + k / 16u] >> (2u * (k % 16u))) & 3u;
}

@compute @workgroup_size(256)
fn write_hl(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wid: vec3u) {
    let w = wid.x;
    let word = masks[w * 8u + (li >> 5u)];
    let bit = li & 31u;
    if ((word >> bit) & 1u) == 0u {
        return;
    }
    var at = offsets[w];
    for (var k = 0u; k < (li >> 5u); k++) {
        at += countOneBits(masks[w * 8u + k]);
    }
    at += countOneBits(word & ((1u << bit) - 1u));
    let ei = einfo[w];
    let cnt = entry_count(ei);
    let k = li / cnt;
    let i = ei.x * PAGE + (ei.z & 255u) + (li - k * cnt);
    if ei.y != 0u {
        let x = ei.y - 1u + k;
        visible[at] = ITEM_BLOCK | (hl_bits(hl[0], x) << 29u) | i;
        vinst[at] = x;
    } else {
        visible[at] = (hl_bits(1u, i) << 29u) | i;
    }
}
