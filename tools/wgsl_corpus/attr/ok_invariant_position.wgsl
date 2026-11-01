// WGSL "Attributes": @invariant on the position output
// expect: valid
@vertex fn vs() -> @invariant @builtin(position) vec4f { return vec4f(); }
