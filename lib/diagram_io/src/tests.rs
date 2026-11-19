// the crate's unit tests (cargo test -p jflib-diagram-io --lib).  The modules keep theirs beside
// them (template/tests.rs, the tests in template_src.rs); this module in the crate root is what
// build_rs.py's suite looks for (unit_test_packages), so the suite runs all of them.  Tests of
// the crate as a whole come here.

use crate::vsdx::{self, Measure};
use diagram_core::DocKind;

// a new template reads back as a template, its windows part as Visio needs it
#[test]
fn new_template_round_trip() {
    let doc = vsdx::new_document(DocKind::Template, Measure::Metric);
    let bytes = vsdx::write(&doc, &vsdx::WriteOptions::DEFAULT).expect("write");
    let back = vsdx::read(&bytes, &vsdx::Limits::VSDX).expect("read");
    assert_eq!(back.doc.kind, DocKind::Template);
    assert!(crate::template::stencil_windows(&back.doc).is_empty());
}
