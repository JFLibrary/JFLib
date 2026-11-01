// WGSL "override Declarations": override ids are below 65536
// expect: error
@id(65536) override a: u32 = 1;
