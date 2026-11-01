// WGSL "Built-in Functions": bit built-ins: countLeadingZeros, extractBits, insertBits, reverseBits, firstLeadingBit
// expect: valid
fn f(x: u32, y: i32) -> u32 { return countLeadingZeros(x) + extractBits(x, 1u, 3u) + insertBits(x, 1u, 4u, 2u) + reverseBits(x) + u32(firstLeadingBit(y)); }
