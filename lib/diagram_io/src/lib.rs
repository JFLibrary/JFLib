// diagram_io -- Visio file formats (the VSDX family) and diagram exports
//
// Reads and writes the model of diagram_core (apps/JFDiagrammer, plan in its PLAN.md).  The
// format follows the public specification [MS-VSDX]; no code of other implementations is used.
//
// What is to come here, by stage of the plan (none of it exists yet):
//   JD3   reading the VSDX family into the model: .vsdx, .vssx, .vstx and the macro-enabled
//         .vsdm / .vssm, whose macro part is passed through unchanged and never run; document.xml
//         (styles, the document's ShapeSheet, colours, fonts), pages with their shapes and
//         connects, masters, themes, windows.xml, properties and media; comments and data
//         recordsets are kept
//   JD5   writing the model back with everything it does not know; new documents with the own
//         default theme; .vssx stencils
//   JD12  exports: SVG, PNG and PDF through the library's writers
//   later the XML formats of 2003-2010 (.vdx, .vsx) and importing the binary .vss / .vsd
//
// The ZIP and OPC parts come from archive (archive::opc, JD1), shared with doc_io's DOCX.
// Every file is untrusted input: sizes, counts and nesting are bounded, a file that breaks a
// rule is an error and never a panic, external links are kept as data and never followed, and
// nothing in a file is ever run.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

// the model this crate reads and writes (JD3 on); named so the dependency is one already
use diagram_core as _;
