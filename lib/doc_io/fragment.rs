// Versioned, self-contained clipboard transport. Payload is the validated native DOCX profile.
use crate::{Error, decode_docx, encode_docx};
use alloc::vec::Vec;
use doc_core::{Document, DocumentFragment, Inline};

const MAGIC: &[u8; 8] = b"JFWCLIP\x01";

/// Encode a rich selection for application/x-jfgui-docx-fragment. The version prefix
/// prevents future fragment semantics from being interpreted as the current model.
pub fn encode_document_fragment(fragment: &DocumentFragment) -> Result<Vec<u8>, Error> {
    let docx = encode_docx(&fragment.document)?;
    if docx.len() > 32 << 20 {
        return Err(Error::Limit);
    }
    let mut bytes = Vec::with_capacity(MAGIC.len() + docx.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&docx);
    Ok(bytes)
}

/// Decode bounded clipboard data without mutating the receiving document. Unknown versions,
/// unsupported DOCX content and invalid source/note references are rejected atomically.
pub fn decode_document_fragment(bytes: &[u8]) -> Result<DocumentFragment, Error> {
    if bytes.len() > (32 << 20) + MAGIC.len() {
        return Err(Error::Limit);
    }
    let payload = bytes.strip_prefix(MAGIC).ok_or(Error::Invalid)?;
    Ok(DocumentFragment {
        document: decode_docx(payload)?,
    })
}

/// Materialize field results for external text formats using the originating document.
/// In particular a bibliography-only selection and note numbers must use the source
/// context, even though the native fragment carries only selected references.
pub fn clipboard_display_fragment(
    fragment: &DocumentFragment,
    context: &Document,
) -> Result<Document, Error> {
    fragment.document.validate()?;
    context.validate()?;
    let mut document = fragment.document.clone();
    for p in document.paragraphs.iter_mut().chain(document.tables.iter_mut().flat_map(|t| &mut t.rows).flat_map(|r| &mut r.cells).flat_map(|c| &mut c.paragraphs)) {
        for inline in &mut p.content {
            let text = match inline {
                Inline::Citation { source, locator } => {
                    Some(context.citation_text(*source, locator))
                }
                Inline::CitationCluster(items) => Some(context.citation_cluster_text(items)),
                Inline::Bibliography => Some(context.bibliography().join("\n")),
                Inline::NoteReference(id) => Some(context.note_marker(*id)),
                Inline::Formula(f) => Some(context.evaluate_formula(p.id,f).text),
                _ => None,
            };
            if let Some(text) = text {
                *inline = Inline::Text(text);
            }
        }
    }
    document.notes.clear();
    document.sources.clear();
    document.validate()?;
    Ok(document)
}
