// WGSL "Comments": a block comment must be closed (nesting counts)
// expect: error
/* outer /* inner */
const a = 1;
