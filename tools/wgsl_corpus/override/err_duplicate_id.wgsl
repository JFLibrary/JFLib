// WGSL "override Declarations": two overrides with one id
// expect: error
@id(1) override a: u32 = 1;
@id(1) override b: u32 = 2;
