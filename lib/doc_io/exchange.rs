// Format dispatch for standalone bibliography exchange; codecs remain independent of the app.
use crate::Error;
use alloc::vec::Vec;
use doc_core::Source;

/// Supported standalone source-list formats. Each codec has an explicit editable subset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BibliographyFormat {
    #[default]
    WordXml,
    Ris,
    Bibtex,
}
impl BibliographyFormat {
    /// Select by filename extension, case-insensitively; never guess from malformed content.
    pub fn from_path(path: &str) -> Result<Self, Error> {
        let path = path.to_ascii_lowercase();
        if path.ends_with(".xml") {
            Ok(Self::WordXml)
        } else if path.ends_with(".ris") {
            Ok(Self::Ris)
        } else if path.ends_with(".bib") || path.ends_with(".bibtex") {
            Ok(Self::Bibtex)
        } else {
            Err(crate::unsupported(
                "Quellenformat: .xml, .ris oder .bib erwartet",
            ))
        }
    }
    /// Default filename extension without a dot.
    pub fn extension(self) -> &'static str {
        match self {
            Self::WordXml => "xml",
            Self::Ris => "ris",
            Self::Bibtex => "bib",
        }
    }
}
/// Decode one complete source list. Errors never return a partially decoded list.
pub fn decode_bibliography(bytes: &[u8], format: BibliographyFormat) -> Result<Vec<Source>, Error> {
    if bytes.len() > 4 << 20 {
        return Err(Error::Limit);
    }
    match format {
        BibliographyFormat::WordXml => crate::decode_sources(bytes),
        BibliographyFormat::Ris => crate::decode_ris(bytes),
        BibliographyFormat::Bibtex => crate::decode_bibtex(bytes),
    }
}
/// Encode all records before any file is touched; unrepresentable metadata is an error.
pub fn encode_bibliography(
    sources: &[Source],
    format: BibliographyFormat,
) -> Result<Vec<u8>, Error> {
    match format {
        BibliographyFormat::WordXml => crate::encode_sources(sources),
        BibliographyFormat::Ris => crate::encode_ris(sources),
        BibliographyFormat::Bibtex => crate::encode_bibtex(sources),
    }
}
/// Load a bounded source file, choosing the codec from its extension.
pub fn load_bibliography(path: &str) -> Result<Vec<Source>, Error> {
    let format = BibliographyFormat::from_path(path)?;
    decode_bibliography(&crate::read_file(path, 4 << 20)?, format)
}
/// Save through the platform's temporary-file/rename writer after complete encoding.
pub fn save_bibliography(path: &str, sources: &[Source]) -> Result<(), Error> {
    if path.is_empty() || path.contains('\0') {
        return Err(Error::Invalid);
    }
    let bytes = encode_bibliography(sources, BibliographyFormat::from_path(path)?)?;
    if plat::write_file(path, &bytes) {
        Ok(())
    } else {
        Err(Error::Io)
    }
}
