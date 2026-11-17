// diagram_io -- Visio file formats (the VSDX family) and diagram exports
//
// Reads and writes the model of diagram_core (apps/JFDiagrammer, plan in its PLAN.md).  The
// format follows the public specification [MS-VSDX]; no code of other implementations is used.
//
//   vsdx::read(&bytes, &vsdx::Limits::VSDX) -> Result<vsdx::Loaded, vsdx::Error>
//                       a .vsdx, .vsdm, .vstx, .vstm, .vssx or .vssm file into a Document:
//                       styles, the document sheet, colours and fonts, pages with shapes and
//                       connects, masters, themes (raw), windows.xml, properties, media,
//                       comments and data recordsets (kept as parts); unknown items in the
//                       Extras where they stood (DF-03); nothing recomputed or rerouted
//   vsdx::write(&doc, &vsdx::WriteOptions::DEFAULT) -> Result<Vec<u8>, vsdx::WriteError>
//                       the model as a file of any of the six formats, with everything it does
//                       not know where it stood; deterministic, DEFLATE; a file read and written
//                       unchanged reads back as the same model
//   vsdx::new_document(kind, Measure::Metric)   a new drawing, template or stencil with JFLib's
//                       own styles, document sheet and DrawingML theme "NetLab"
//   vsdx::Format        the six formats by main content type or extension
//
// What is to come here, by stage of the plan:
//   JD12  exports: SVG, PNG and PDF through the library's writers
//   later the XML formats of 2003-2010 (.vdx, .vsx) and importing the binary .vss / .vsd
//
// The ZIP and OPC parts come from archive (archive::opc, JD1), shared with doc_io's DOCX.
// Every file is untrusted input: sizes, counts and nesting are bounded, a file that breaks a
// rule is an error and never a panic, external links are kept as data and never followed, and
// nothing in a file is ever run (the macro project of a .vsdm is kept as bytes).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod blank;
mod pack;
mod read;
mod theme;
pub mod vsdx;
mod write;
mod xml;
