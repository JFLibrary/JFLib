// WGSL "Entry Points": a fragment output at location 8 (beyond maxColorAttachments)
// expect: pipeline-error
@fragment fn fs() -> @location(8) vec4f { return vec4f(); }
