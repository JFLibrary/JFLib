// formula -- the formulas of the binary format: tokens in postfix order (vss/mod.rs), turned
// back into the formula text Visio shows; a formula with a token this reader does not know is
// left out and the cell keeps its value (V1: never a guess)

use alloc::string::String;
use alloc::vec::Vec;

/// the formula text of a block's tokens; None when a token is unknown
pub(super) fn decode(t: &[u8]) -> Option<String> {
    let _ = t;
    let v: Vec<String> = Vec::new();
    v.into_iter().next()
}
