// wgsl -- JFLib's WGSL front end: source text -> a checked, fully typed module (step G2a of
// the GPU plan; docs/WGSL.md).  The back end (G2c) lowers the IR to SPIR-V with lib/shader;
// the browser path (G4) hands the same text to WebGPU.  The reference is the W3C WGSL
// Candidate Recommendation (2026-10-26); where Chrome's Tint differs, the docs say so.
//
//   parse(src) -> (Ast, Vec<Diagnostic>)        the syntax tree of everything that parsed and
//                                               the lexer / parser errors (several per run)
//   check(src) -> Result<Module, Vec<Diagnostic>>   parse + semantic check with the default
//                                               Options; Ok carries the warnings in
//                                               Module::warnings
//   check_with(src, &Options)                   the same with a chosen set of language features
//   print(&ast) -> String                       the AST as WGSL text (parse -> print -> parse
//                                               gives the same tree)
//   lex::tokens(src)                            the tokens (template lists discovered)
//   ir                                          the checked module: types with layout, globals,
//                                               overrides, functions (expression arenas,
//                                               statements), entry points with their interface
//   Diagnostic                                  1-based line, column in characters, end column,
//                                               error / warning, German message, notes
//   bench                                       the six benchmark shaders of docs/SHADER.md
//                                               as WGSL text
//
// Never panics on any input: nesting deeper than parse::MAX_DEPTH is an error, constant
// values are capped in size (consteval::MAX_ELEMS), every count is checked.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

pub mod ast;
pub mod bench;
mod builtins;
mod check;
mod check_expr;
mod check_stmt;
mod consteval;
mod entry;
pub mod ir;
pub mod lex;
mod math;
pub mod parse;
mod print;
mod types;
pub mod xid;

pub use ast::{Ast, Span};
pub use ir::Module;
pub use print::print;

/// a note attached to a diagnostic: where the related declaration is
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Note {
    pub line: u32,
    pub col: u32,
    pub end_col: u32,
    pub msg: String,
}

/// a message for the editor: position (1-based; columns count characters, end_col is
/// exclusive and on the same line), error or warning, German text; start / end: the byte
/// range in the source; notes point at related places ("hier deklariert")
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    pub line: u32,
    pub col: u32,
    pub end_col: u32,
    pub error: bool,
    pub msg: String,
    pub start: u32,
    pub end: u32,
    pub notes: Vec<Note>,
}

impl core::fmt::Display for Diagnostic {
    /// "3:5: Fehler: erwartet `;` nach der Anweisung" plus one line per note
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "{}:{}: {}: {}", self.line, self.col, if self.error { "Fehler" } else { "Warnung" }, self.msg)?;
        for n in &self.notes {
            write!(f, "\n  {}:{}: Hinweis: {}", n.line, n.col, n.msg)?;
        }
        Ok(())
    }
}

/// the language extensions (`requires`) and enable extensions this front end implements; a
/// module may use an extension only when its bit is set here (Options::features)
pub mod feature {
    /// storage textures with read and read_write access, textureBarrier
    pub const READONLY_AND_READWRITE_STORAGE_TEXTURES: u32 = 1 << 0;
    /// dot4U8Packed, dot4I8Packed, pack4xI8 ... unpack4xU8
    pub const PACKED_4X8_INTEGER_DOT_PRODUCT: u32 = 1 << 1;
    /// pointer parameters in storage / uniform / workgroup, to parts of variables
    pub const UNRESTRICTED_POINTER_PARAMETERS: u32 = 1 << 2;
    /// p.x and p[i] on pointers
    pub const POINTER_COMPOSITE_ACCESS: u32 = 1 << 3;
    /// uniform buffers with the storage layout rules
    pub const UNIFORM_BUFFER_STANDARD_LAYOUT: u32 = 1 << 4;
    /// let of texture and sampler type
    pub const TEXTURE_AND_SAMPLER_LET: u32 = 1 << 5;
    /// the further storage texel formats (r8unorm, rg16float ...)
    pub const TEXTURE_FORMATS_TIER1: u32 = 1 << 6;
    /// @builtin(global_invocation_index) and @builtin(workgroup_index)
    pub const LINEAR_INDEXING: u32 = 1 << 7;
    /// var<immediate> (push constants)
    pub const IMMEDIATE_ADDRESS_SPACE: u32 = 1 << 8;
    /// @builtin(frag_depth, less | greater)
    pub const FRAGMENT_DEPTH: u32 = 1 << 9;
    /// v.xy = ... (swizzle views)
    pub const SWIZZLE_ASSIGNMENT: u32 = 1 << 10;
    /// enable-extensions the device may offer (the WebGPU features)
    pub const F16: u32 = 1 << 16;
    /// enable clip_distances: @builtin(clip_distances)
    pub const CLIP_DISTANCES: u32 = 1 << 17;
    /// enable dual_source_blending: @blend_src
    pub const DUAL_SOURCE_BLENDING: u32 = 1 << 18;
    /// enable primitive_index: @builtin(primitive_index)
    pub const PRIMITIVE_INDEX: u32 = 1 << 19;
    /// enable atomic_vec2u_min_max: atomicStoreMin / Max on atomic<vec2<u32>>
    pub const ATOMIC_VEC2U_MIN_MAX: u32 = 1 << 20;
    /// everything this front end implements (the language features of Chrome 156 except
    /// buffer_view, subgroup_id and subgroup_uniformity; the enable-extensions except
    /// subgroups and subgroup_size_control)
    pub const ALL: u32 = 0x7ff | 0x1f << 16;
    /// (name, bit, is a language extension) for every extension name WGSL defines; bit 0:
    /// known but not implemented here
    pub const NAMES: &[(&str, u32, bool)] = &[
        ("readonly_and_readwrite_storage_textures", READONLY_AND_READWRITE_STORAGE_TEXTURES, true),
        ("packed_4x8_integer_dot_product", PACKED_4X8_INTEGER_DOT_PRODUCT, true),
        ("unrestricted_pointer_parameters", UNRESTRICTED_POINTER_PARAMETERS, true),
        ("pointer_composite_access", POINTER_COMPOSITE_ACCESS, true),
        ("uniform_buffer_standard_layout", UNIFORM_BUFFER_STANDARD_LAYOUT, true),
        ("texture_and_sampler_let", TEXTURE_AND_SAMPLER_LET, true),
        ("texture_formats_tier1", TEXTURE_FORMATS_TIER1, true),
        ("linear_indexing", LINEAR_INDEXING, true),
        ("immediate_address_space", IMMEDIATE_ADDRESS_SPACE, true),
        ("fragment_depth", FRAGMENT_DEPTH, true),
        ("swizzle_assignment", SWIZZLE_ASSIGNMENT, true),
        ("buffer_view", 0, true),
        ("subgroup_id", 0, true),
        ("subgroup_uniformity", 0, true),
        ("f16", F16, false),
        ("clip_distances", CLIP_DISTANCES, false),
        ("dual_source_blending", DUAL_SOURCE_BLENDING, false),
        ("primitive_index", PRIMITIVE_INDEX, false),
        ("atomic_vec2u_min_max", ATOMIC_VEC2U_MIN_MAX, false),
        ("subgroups", 0, false),
        ("subgroup_size_control", 0, false),
    ];
}

/// what the checker accepts beyond core WGSL
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// feature::* bits: the language extensions the implementation supports (they apply
    /// without `requires`, as in a browser) and the enable-extensions the device offers
    /// (they apply only after `enable`)
    pub features: u32,
}

impl Default for Options {
    fn default() -> Options {
        Options { features: feature::ALL }
    }
}

/// the syntax tree and the syntax errors (empty when the source parses)
pub fn parse(src: &str) -> (Ast, Vec<Diagnostic>) {
    parse::parse(src)
}

/// parse and check with the default options: the module (warnings in Module::warnings) or
/// every error found (and the warnings)
pub fn check(src: &str) -> Result<Module, Vec<Diagnostic>> {
    check_with(src, &Options::default())
}

/// parse and check with the given options
pub fn check_with(src: &str, opt: &Options) -> Result<Module, Vec<Diagnostic>> {
    let (ast, diags) = parse::parse(src);
    if diags.iter().any(|d| d.error) {
        return Err(diags);
    }
    check::check(src, &ast, opt)
}

/// the start of every line (byte offsets); line breaks as WGSL defines them: LF, VT, FF, CR
/// (alone or before LF), NEL, LS, PS
pub struct LineMap {
    starts: Vec<u32>,
}

impl LineMap {
    /// the line starts of a source
    pub fn new(src: &str) -> LineMap {
        let mut starts = alloc::vec![0u32];
        let b = src.as_bytes();
        let mut i = 0;
        while i < b.len() {
            let c = b[i];
            let n = match c {
                b'\n' | 0x0b | 0x0c => 1,
                b'\r' => {
                    if b.get(i + 1) == Some(&b'\n') {
                        2
                    } else {
                        1
                    }
                }
                0xc2 if b.get(i + 1) == Some(&0x85) => 2,
                0xe2 if b.get(i + 1) == Some(&0x80) && matches!(b.get(i + 2), Some(0xa8) | Some(0xa9)) => 3,
                _ => 0,
            };
            if n > 0 {
                i += n;
                starts.push(i as u32);
            } else {
                i += 1;
            }
        }
        LineMap { starts }
    }
    /// 1-based line and character column of a byte offset
    pub fn pos(&self, src: &str, at: u32) -> (u32, u32) {
        let at = (at as usize).min(src.len());
        let line = self.starts.partition_point(|&s| s as usize <= at).max(1);
        let ls = self.starts[line - 1] as usize;
        let col = src.get(ls..at).map(|s| s.chars().count()).unwrap_or_else(|| at - ls) as u32 + 1;
        (line as u32, col)
    }
    /// a diagnostic for the byte range start..end (end_col stays on the first line)
    pub fn diag(&self, src: &str, start: u32, end: u32, error: bool, msg: String) -> Diagnostic {
        let (line, col) = self.pos(src, start);
        let end = end.max(start);
        let line_end = self.starts.get(line as usize).map(|&s| s as usize).unwrap_or(src.len());
        let e = (end as usize).min(line_end).max(start as usize);
        let w = src.get(start as usize..e).map(|s| s.trim_end_matches(['\n', '\r']).chars().count()).unwrap_or(1) as u32;
        Diagnostic { line, col, end_col: col + w.max(1), error, msg, start, end, notes: Vec::new() }
    }
    /// a note for the byte range start..end
    pub fn note(&self, src: &str, start: u32, end: u32, msg: String) -> Note {
        let d = self.diag(src, start, end, false, msg);
        Note { line: d.line, col: d.col, end_col: d.end_col, msg: d.msg }
    }
}
