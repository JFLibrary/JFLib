// cull_reset: draw = DrawCmd::reset() -- 36 indices, no instances yet (one invocation; the
// alternative is vkCmdFillBuffer).  DrawCmd without atomics: this shader only stores.

struct DrawCmd {
    index_count: u32,
    instance_count: u32,
    first_index: u32,
    vertex_offset: i32,
    first_instance: u32,
}

@group(0) @binding(3) var<storage, read_write> draw: DrawCmd;

@compute @workgroup_size(1)
fn main() {
    draw.index_count = 36u;
    draw.instance_count = 0u;
    draw.first_index = 0u;
    draw.vertex_offset = 0i;
    draw.first_instance = 0u;
}
