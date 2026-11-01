// WGSL "Literals": a suffixed literal without a dot or exponent must not have leading zeros (01f)
// expect: error
const a = 01f;
