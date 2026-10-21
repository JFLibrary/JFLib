// jfWord semantic actions: thin adapters over the same Editor/Word workflows as the GUI.
use crate::*;
use agent_core::{ActionContext, ActionError, ActionResult, ActionSpec, ErrorCode, Value};
use agent_core::schema::{Property, Schema, BOOLEAN, EMPTY_OBJECT, OBJECT, STRING, U64};
use doc_core::{CitationItem, Formula, NoteKind, Person, SourceKind, TextColor, TextFormat};

#[path = "style_actions.rs"]
mod style_actions;
#[path = "paragraph_actions.rs"]
mod paragraph_actions;
#[path="table_actions.rs"]
mod table_actions;
#[path="table_style_actions.rs"]
mod table_style_actions;

const fn property(name: &'static str, schema: &'static Schema, required: bool) -> Property { Property { name, schema, required } }
static LONG_TEXT: Schema = Schema::String { min: 0, max: 262144, choices: &[] };
static PATH: Schema = Schema::String { min: 1, max: 4096, choices: &[] };
static NEW_INPUT: Schema = Schema::Object { properties: &[property("discardChanges", &BOOLEAN, false)], additional: false };
static FILE_INPUT: Schema = Schema::Object { properties: &[property("path", &PATH, true), property("discardChanges", &BOOLEAN, false), property("overwrite", &BOOLEAN, false)], additional: false };
static ID_INPUT: Schema = Schema::Object { properties: &[property("id", &U64, true)], additional: false };
static RANGE: Schema = Schema::Object { properties: &[property("paragraph", &U64, true), property("start", &U64, true), property("end", &U64, true)], additional: false };
static REPLACE: Schema = Schema::Object { properties: &[property("paragraph", &U64, true), property("start", &U64, true), property("end", &U64, true), property("text", &LONG_TEXT, true), property("paragraphBreaks", &BOOLEAN, false)], additional: false };
static RICH_IMPORT: Schema = Schema::Object {properties:&[property("paragraph",&U64,true),property("start",&U64,true),property("end",&U64,true),property("format",&Schema::String{min:3,max:4,choices:&["rtf","html"]},true),property("text",&LONG_TEXT,true)],additional:false};
static EXPORT:Schema=Schema::Object{properties:&[property("format",&Schema::String{min:3,max:4,choices:&["docx","html","rtf"]},false)],additional:false};
static CHARACTER: Schema = Schema::Object { properties: &[
    property("fontFamily", &Schema::String { min: 1, max: 96, choices: &[] }, false),
    property("bold", &BOOLEAN, false), property("italic", &BOOLEAN, false), property("underline", &BOOLEAN, false), property("strike", &BOOLEAN, false),
    property("sizeHalfPoints", &Schema::Unsigned { min: 8, max: 288 }, false),
    property("color", &Schema::Unsigned { min: 0, max: 0xffffff }, false),
    property("highlight", &Schema::String { min: 1, max: 12, choices: &["none", "yellow", "green", "cyan", "magenta", "blue", "red", "darkBlue", "darkCyan", "darkGreen", "darkMagenta", "darkRed", "darkYellow", "darkGray", "lightGray", "black", "white"] }, false),
    property("positioning", &Schema::String { min: 1, max: 12, choices: &["baseline", "superscript", "subscript"] }, false),
], additional: false };
static FORMAT: Schema = Schema::Object { properties: &[property("paragraph", &U64, true), property("start", &U64, true), property("end", &U64, true), property("format", &CHARACTER, true)], additional: false };
static STYLE: Schema = Schema::Object { properties: &[property("paragraph", &U64, true), property("style", &style_actions::STYLE_KEY, true)], additional: false };
static PERSON: Schema = Schema::Object { properties: &[property("given", &STRING, false), property("middle", &STRING, false), property("family", &STRING, false)], additional: false };
static SOURCE: Schema = Schema::Object { properties: &[
    property("id", &U64, false), property("kind", &Schema::String { min: 1, max: 20, choices: &["book", "article", "website"] }, false),
    property("authors", &Schema::Array { items: &PERSON, min: 0, max: 64 }, false),
    property("corporate", &STRING, false), property("title", &PATH, true), property("year", &STRING, false),
    property("publisher", &STRING, false), property("city", &STRING, false), property("edition", &STRING, false),
    property("journal", &STRING, false), property("volume", &STRING, false), property("issue", &STRING, false),
    property("pages", &STRING, false), property("website", &STRING, false), property("url", &STRING, false),
    property("standardNumber", &STRING, false), property("yearAccessed", &STRING, false), property("monthAccessed", &STRING, false),
    property("dayAccessed", &STRING, false), property("shortTitle", &STRING, false), property("comments", &STRING, false),
    property("externalId", &STRING, false), property("language", &STRING, false),
], additional: false };
static IMPORT: Schema = Schema::Object { properties: &[property("format", &Schema::String { min: 1, max: 10, choices: &["bibtex", "ris", "word-xml"] }, true), property("text", &LONG_TEXT, true)], additional: false };
static CITE_ITEM: Schema = Schema::Object { properties: &[property("source", &U64, true), property("locator", &STRING, false), property("prefix", &STRING, false), property("suffix", &STRING, false), property("suppressAuthor", &BOOLEAN, false), property("suppressYear", &BOOLEAN, false), property("suppressTitle", &BOOLEAN, false)], additional: false };
static CITATION: Schema = Schema::Object { properties: &[property("paragraph", &U64, true), property("inline", &U64, true), property("offset", &U64, false), property("items", &Schema::Array { items: &CITE_ITEM, min: 1, max: 32 }, true)], additional: false };
static FIELD: Schema = Schema::Object { properties: &[property("paragraph", &U64, true), property("inline", &U64, true)], additional: false };
static EXPRESSION:Schema=Schema::String{min:1,max:2048,choices:&[]};
static NUMBER_PICTURE:Schema=Schema::String{min:0,max:32,choices:&[]};
static FORMULA:Schema=Schema::Object{properties:&[property("paragraph",&U64,true),property("inline",&U64,true),property("offset",&U64,false),property("expression",&EXPRESSION,true),property("numberFormat",&NUMBER_PICTURE,false)],additional:false};
static EVALUATE:Schema=Schema::Object{properties:&[property("paragraph",&U64,true),property("expression",&EXPRESSION,true),property("numberFormat",&NUMBER_PICTURE,false)],additional:false};
static NOTE: Schema = Schema::Object { properties: &[property("paragraph", &U64, true), property("inline", &U64, true), property("offset", &U64, true), property("kind", &Schema::String { min: 1, max: 10, choices: &["footnote", "endnote"] }, true), property("text", &LONG_TEXT, false)], additional: false };
static NOTE_KIND: Schema = Schema::Object { properties: &[property("id", &U64, true), property("kind", &Schema::String { min: 1, max: 10, choices: &["footnote", "endnote"] }, true)], additional: false };
const fn edit(name: &'static str, description: &'static str, input: &'static Schema) -> ActionSpec { ActionSpec { undo: true, ..ActionSpec::write(name, description).schemas(input, &OBJECT) } }
static ACTIONS: &[ActionSpec] = &[
    ActionSpec::read("formulas.list","Inspect live calculation fields with table-local results and structured error codes").schemas(&OBJECT,&OBJECT),
    ActionSpec::read("formulas.evaluate","Preview a supported formula in the current paragraph/table without changing the document").schemas(&EVALUATE,&OBJECT),
    edit("formulas.insert","Insert a live calculation field at a text caret; checked decimal arithmetic is shared by native and WASM",&FORMULA),
    edit("formulas.update","Replace a live calculation field and number picture as one undo transaction",&FORMULA),
    edit("document.import", "Insert bounded RTF/HTML blocks, including nested tables, at a story selection as one undo step", &RICH_IMPORT),
    ActionSpec::read("document.table.styles.list","Discover named table styles, conditional regions and document colors").schemas(&table_style_actions::LIST,&OBJECT),
    edit("document.table.styles.put","Create or update a named table style, optionally apply to a table in the same undo transaction. cloneFrom copies a definition",&table_style_actions::PUT),
    ActionSpec::read("document.table.layout","Read page instances and repeated header cell bounds in twips, using document fonts").schemas(&table_actions::INSPECT,&OBJECT),
    edit("document.table.fit","Compute column widths from document fonts; apply once as one undo step. Modes: contents, page, equal",&table_actions::FIT),
    edit("document.table.insert","Insert a rectangular table at a main-body paragraph UTF-8 offset",&table_actions::INSERT),
    edit("document.table.change","Change rows, columns, cell shading/alignment or geometry atomically; sizes are twips, borderWidth is eighth-points",&table_actions::CHANGE),
    ActionSpec::read("document.table.inspect","Inspect paginated table rows and stable cell paragraph identities").schemas(&table_actions::INSPECT,&OBJECT),
    ActionSpec::read("document.inspect", "Read paginated document paragraphs, sources, notes and file state").schemas(&OBJECT, &OBJECT),
    ActionSpec::read("document.export", "Encode DOCX (default), HTML or RTF as a separate resource without marking the document saved; HTML retains table positions").schemas(&EXPORT,&OBJECT),
    ActionSpec { command_id: Some(NEW), ..ActionSpec::write("document.new", "Create a blank document; refuse unsaved changes unless explicitly discarded").schemas(&NEW_INPUT, &OBJECT) },
    ActionSpec { capabilities: plat::CAP_FILE_READ, ..ActionSpec::write("file.open", "Open DOCX through the loss-aware file session").schemas(&FILE_INPUT, &OBJECT) },
    ActionSpec { capabilities: plat::CAP_FILE_WRITE | plat::CAP_PERSISTENT_STORAGE, ..ActionSpec::write("file.save", "Save the current DOCX, retaining unsupported package parts").schemas(&FILE_INPUT, &OBJECT) },
    ActionSpec { command_id: Some(UNDO), ..edit("document.undo", "Undo through the same history/caret restoration as the GUI", &EMPTY_OBJECT) },
    ActionSpec { command_id: Some(REDO), ..edit("document.redo", "Redo through the same history/caret restoration as the GUI", &EMPTY_OBJECT) },
    edit("document.text.replace", "Replace a UTF-8 byte range in a complete story; generated fields remain protected", &REPLACE),
    edit("document.text.format", "Apply explicit character formatting to a UTF-8 story range", &FORMAT),
    edit("document.text.format.replace", "Replace all character overrides in a story range; omitted properties inherit from its paragraph style", &FORMAT),
    edit("document.text.format.reset", "Clear character overrides in a UTF-8 story range, preserving paragraph styles, text and fields", &RANGE),
    ActionSpec::read("document.text.format.inspect", "Read the effective character properties at the beginning of an editable story selection").schemas(&RANGE, &OBJECT),
    ActionSpec::read("document.text.format.summary", "Read shared effective properties of the entire editable story selection; null means mixed").schemas(&RANGE, &OBJECT),
    ActionSpec { revision_required: false, ..ActionSpec::write("document.select", "Select a UTF-8 range on the page for keyboard and clipboard operations").schemas(&RANGE, &OBJECT) },
    ActionSpec::read("document.styles.list", "List named style definitions, inheritance and effective character properties").schemas(&OBJECT, &OBJECT),
    edit("document.styles.put", "Create or replace a named paragraph style; reject cycles and dangling bases atomically", &style_actions::STYLE_PUT),
    edit("document.paragraph.style", "Apply one of the currently supported document paragraph styles", &STYLE),
    edit("document.paragraph.format", "Patch paragraph properties in a UTF-8 story range; tabs merge by position, clear suppresses inheritance; all units are twips", &paragraph_actions::PARAGRAPH_RANGE),
    edit("document.paragraph.tabs", "Replace effective tab stops in selected paragraphs (empty list clears all inherited stops); units are twips from the left text margin", &paragraph_actions::TAB_RANGE),
    edit("document.page.set", "Set page dimensions and margins in twips through the same validated transaction as the rulers", &paragraph_actions::PAGE),
    edit("document.paragraph.add", "Add a paragraph after a stable paragraph identity", &ID_INPUT),
    edit("document.paragraph.remove", "Remove a paragraph through the validated editor transaction", &ID_INPUT),
    edit("sources.put", "Create or update a bibliographic source (id zero creates one)", &SOURCE),
    edit("sources.remove", "Remove an unused source; cited sources cannot be removed", &ID_INPUT),
    edit("sources.import", "Import BibTeX, RIS or Sources.xml bibliography XML with the shared codec", &IMPORT),
    edit("citations.insert", "Insert a source-linked citation at a text-run caret", &CITATION),
    edit("citations.update", "Edit the items and affixes of an existing citation field", &CITATION),
    edit("fields.remove", "Remove one field using the shared editor's field-removal transaction", &FIELD),
    edit("bibliography.insert", "Insert a generated bibliography after a paragraph", &ID_INPUT),
    edit("notes.insert", "Create an anchored footnote or endnote at a text caret", &NOTE),
    edit("notes.kind", "Convert an existing note between footnote and endnote", &NOTE_KIND),
    edit("notes.remove", "Remove a note and its reference as a single undo step", &ID_INPUT),
    ActionSpec { capabilities: plat::CAP_CLIPBOARD_RICH, ..ActionSpec::read("clipboard.copy", "Copy the current selection in native, RTF, HTML and plain-text forms") },
    ActionSpec { capabilities: plat::CAP_CLIPBOARD_RICH, ..edit("clipboard.cut", "Cut the current selection through the rich-text clipboard", &EMPTY_OBJECT) },
    ActionSpec { capabilities: plat::CAP_CLIPBOARD_RICH, ..edit("clipboard.paste", "Paste the native or interoperable rich-text clipboard at the selection", &EMPTY_OBJECT) },
    ActionSpec { capabilities:plat::CAP_CLIPBOARD_RICH,input:&table_actions::TABLE_ID,..ActionSpec::read("clipboard.table.copy","Copy an entire table including spans, headers, borders, sources and live style graphs") },
    ActionSpec { capabilities:plat::CAP_CLIPBOARD_RICH,..edit("clipboard.table.cut","Copy a complete table and remove it in one undo step",&table_actions::TABLE_ID) },
];
pub(crate) fn actions() -> &'static [ActionSpec] { ACTIONS }
fn text(input: &Value, key: &str) -> String { String::from(input.get(key).and_then(Value::as_str).unwrap_or("")) }
fn id(input: &Value, key: &str) -> u64 { input.get(key).and_then(Value::as_u64).unwrap_or(0) }
fn flag(input: &Value, key: &str) -> bool { input.get(key).and_then(Value::as_bool).unwrap_or(false) }
fn index(input: &Value, key: &str) -> Result<usize, ActionError> { usize::try_from(id(input, key)).map_err(|_| invalid("Offset exceeds the platform address range")) }
fn invalid(message: impl Into<String>) -> ActionError { ActionError::new(ErrorCode::InvalidInput, message) }
fn io_error(error: doc_io::Error) -> ActionError { ActionError::new(ErrorCode::Unavailable, format!("File/codec operation failed: {error:?}")) }
fn kind(input: &Value) -> NoteKind { if text(input, "kind") == "endnote" { NoteKind::Endnote } else { NoteKind::Footnote } }
fn citation_items(input: &Value) -> Vec<CitationItem> {
    input.get("items").and_then(Value::as_array).unwrap_or(&[]).iter().map(|v| CitationItem {
        source: id(v, "source"), locator: text(v, "locator"), prefix: text(v, "prefix"), suffix: text(v, "suffix"),
        suppress_author: flag(v, "suppressAuthor"), suppress_year: flag(v, "suppressYear"), suppress_title: flag(v, "suppressTitle"),
    }).collect()
}
fn source(input: &Value) -> Source {
    Source {
        id: id(input, "id"), kind: match text(input, "kind").as_str() { "article" => SourceKind::JournalArticle, "website" => SourceKind::Website, _ => SourceKind::Book },
        authors: input.get("authors").and_then(Value::as_array).unwrap_or(&[]).iter().map(|p| Person { given: text(p, "given"), middle: text(p, "middle"), family: text(p, "family") }).collect(),
        corporate: text(input, "corporate"), title: text(input, "title"), year: text(input, "year"), publisher: text(input, "publisher"), city: text(input, "city"), edition: text(input, "edition"),
        journal: text(input, "journal"), volume: text(input, "volume"), issue: text(input, "issue"), pages: text(input, "pages"), website: text(input, "website"), url: text(input, "url"), standard_number: text(input, "standardNumber"),
        year_accessed: text(input, "yearAccessed"), month_accessed: text(input, "monthAccessed"), day_accessed: text(input, "dayAccessed"), short_title: text(input, "shortTitle"), comments: text(input, "comments"), external_id: text(input, "externalId"), language: text(input, "language"),
    }
}
fn format_json(format: TextFormat) -> Value {
    Value::object([("fontFamily", format.font_family.map(|f| Value::string(f.as_str())).unwrap_or(Value::Null)), ("bold", format.bold.map(Value::Bool).unwrap_or(Value::Null)), ("italic", format.italic.map(Value::Bool).unwrap_or(Value::Null)),
        ("underline", format.underline.map(Value::Bool).unwrap_or(Value::Null)), ("strike", format.strike.map(Value::Bool).unwrap_or(Value::Null)),
        ("sizeHalfPoints", format.size_half_points.map(|v| Value::u64(v as u64)).unwrap_or(Value::Null)),
        ("highlight", format.highlight.map(|v| Value::string(v.name())).unwrap_or(Value::Null)),
        ("positioning", format.positioning.map(|v| Value::string(v.name())).unwrap_or(Value::Null)),
        ("color", match format.color { None => Value::Null, Some(TextColor::Auto) => Value::string("auto"), Some(TextColor::Rgb(v)) => Value::u64(v as u64) })])
}

impl Word {
    pub(crate) fn document_resource_info(&self, uri: &str) -> Result<agent_core::Resource, ActionError> {
        let (_, bytes) = self.exported_document.as_ref().filter(|(name,_)| name == uri)
            .ok_or_else(|| ActionError::new(ErrorCode::NotFound, "Document resource expired or unknown"))?;
        let (name,mime)=if uri.ends_with(".html"){("Document.html","text/html")}else if uri.ends_with(".rtf"){("Document.rtf","application/rtf")}else{("Document.docx","application/vnd.openxmlformats-officedocument.wordprocessingml.document")};
        Ok(agent_core::Resource { uri: String::from(uri), name: String::from(name),
            mime_type: String::from(mime), size: Some(bytes.len() as u64) })
    }
    pub(crate) fn document_resource_read(&self, uri: &str, offset: u64, limit: usize) -> Result<Vec<u8>, ActionError> {
        self.document_resource_info(uri)?;
        let bytes = &self.exported_document.as_ref().unwrap().1;
        let offset = usize::try_from(offset).ok().filter(|n| *n <= bytes.len()).ok_or_else(|| invalid("Resource offset is out of range"))?;
        Ok(bytes[offset..offset.saturating_add(limit.min(262144)).min(bytes.len())].to_vec())
    }
    pub(crate) fn document_revision(&self) -> u64 {
        let revision = if self.preserved() { self.document_file.as_ref().map_or(0, doc_io::DocxFile::revision) } else { self.editor.revision() };
        (self.document_epoch << 32).wrapping_add(revision)
    }
    pub(crate) fn domain_available(&self, spec: &ActionSpec) -> Result<(), ActionError> {
        if self.preserved() && !matches!(spec.name, "document.inspect" | "document.export" | "document.new" | "file.open" | "file.save") {
            return Err(ActionError::new(ErrorCode::Unavailable, "This file is in package-preservation mode; structured editing is unavailable"));
        }
        if spec.name.starts_with("clipboard.") && (self.preview_mode || self.structured_edit) {
            return Err(ActionError::new(ErrorCode::Unavailable, "Switch to on-page rich-text editing before using its clipboard"));
        }
        if (spec.name == "document.undo" && !self.editor.can_undo()) || (spec.name == "document.redo" && !self.editor.can_redo()) {
            return Err(ActionError::new(ErrorCode::Unavailable, "No history step is available in this direction"));
        }
        Ok(())
    }
    pub(crate) fn document_inspect(&self, query: &Value) -> Result<Value, ActionError> {
        let revision = self.document_revision();
        if query.get("sinceRevision").and_then(Value::as_u64) == Some(revision) {
            return Ok(Value::object([("revision", Value::u64(revision)), ("unchanged", Value::Bool(true))]));
        }
        let offset = index(query, "offset")?;
        let limit = query.get("limit").and_then(Value::as_u64).unwrap_or(50).clamp(1, 100) as usize;
        let document = self.editor.document();
        let filter = text(query, "filter").to_lowercase();
        let paragraphs: Vec<_> = document.all_paragraphs().filter(|p| filter.is_empty() || document.paragraph_text(p).to_lowercase().contains(&filter)).collect();
        let items = paragraphs.iter().skip(offset).take(limit).map(|p| Value::object([
            ("id", Value::u64(p.id)), ("style", Value::string(style_actions::key(p.style))),
            ("text", Value::string(document.paragraph_text(p))),
            ("paragraphFormat", style_actions::paragraph(p.format)),
            ("effectiveParagraphFormat", style_actions::paragraph(doc_core::ParagraphFormat { explicit: 511, ..document.resolve_paragraph(p) })),
            ("inlines", Value::Array(p.content.iter().enumerate().map(|(i, node)| Value::object([
                ("index", Value::u64(i as u64)), ("kind", Value::string(match node { Inline::Text(_) | Inline::StyledText {..} => "text", Inline::Citation {..} | Inline::CitationCluster(_) => "citation", Inline::Bibliography => "bibliography", Inline::NoteReference(_) => "note",Inline::Formula(_)=>"formula" })),
                ("text", node.text().map(Value::string).unwrap_or(Value::Null)), ("format", format_json(node.format())),
            ])).collect())),
        ])).collect();
        let sources = document.sources.iter().skip(offset).take(limit).map(|s| Value::object([
            ("id", Value::u64(s.id)), ("title", Value::string(&s.title)), ("year", Value::string(&s.year)), ("authors", Value::string(s.author_text())),
        ])).collect();
        let notes = document.notes.iter().skip(offset).take(limit).map(|n| Value::object([
            ("id", Value::u64(n.id)), ("kind", Value::string(if n.kind == NoteKind::Footnote { "footnote" } else { "endnote" })),
            ("number", document.note_number(n.id).map(|v|Value::u64(v as u64)).unwrap_or(Value::Null)),
            ("anchor", document.note_location(n.id).map(|at|Value::object([
                ("paragraph",Value::u64(at.paragraph)),("inline",Value::u64(at.inline as u64)),
                ("cell",document.cell_for_paragraph(at.paragraph).map(|c|Value::u64(c.id)).unwrap_or(Value::Null)),
            ])).unwrap_or(Value::Null)),
            ("paragraphs", Value::Array(n.paragraphs.iter().map(|p| Value::u64(p.id)).collect())),
        ])).collect();
        Ok(Value::object([
            ("revision", Value::u64(revision)), ("path", Value::string(&self.path)), ("dirty", Value::Bool(self.document_dirty())), ("mode", Value::string(if self.preserved() { "preserved" } else { "native" })),
            ("page", paragraph_actions::page_json(document.page)),
            ("tables",Value::Array(document.tables.iter().skip(offset).take(limit).map(|t|table_actions::summary(document,t)).collect())),("totalTables",Value::u64(document.tables.len() as u64)),
            ("paragraphs", Value::Array(items)), ("totalParagraphs", Value::u64(paragraphs.len() as u64)),
            ("nextOffset", if offset.saturating_add(limit) < paragraphs.len() { Value::u64((offset + limit) as u64) } else { Value::Null }),
            ("sources", Value::Array(sources)), ("totalSources", Value::u64(document.sources.len() as u64)),
            ("notes", Value::Array(notes)), ("totalNotes", Value::u64(document.notes.len() as u64)),
        ]))
    }
    pub(crate) fn domain_action(&mut self, ui: &mut Ui, spec: &ActionSpec, input: &Value, _context: &ActionContext) -> Result<ActionResult, ActionError> {
        self.domain_available(spec)?;
        let mut result_id = None;
        let paragraph = id(input, "paragraph");
        let start = index(input, "start")?;
        let end = index(input, "end")?;
        let edit = match spec.name {
            "formulas.evaluate"=>{
                if self.editor.document().paragraph(paragraph).is_none(){return Err(invalid("Unknown formula paragraph"));}
                let f=Formula::new(&text(input,"expression"),&text(input,"numberFormat")).map_err(|e|invalid(e.code()))?;
                let result=self.editor.document().evaluate_formula(paragraph,&f);
                return Ok(ActionResult::completed(Value::object([("result",Value::string(result.text)),("error",result.error.map(|e|Value::string(e.code())).unwrap_or(Value::Null))]),self.document_revision()));
            },
            "formulas.list"=>{
                let d=self.editor.document();let mut fields=Vec::new();
                for p in d.body_paragraphs().chain(d.notes.iter().flat_map(|n|&n.paragraphs)) {for (i,node) in p.content.iter().enumerate(){if let Inline::Formula(f)=node{
                    if paragraph!=0&&paragraph!=p.id{continue;}
                    let result=d.evaluate_formula(p.id,f);
                    fields.push(Value::object([("paragraph",Value::u64(p.id)),("inline",Value::u64(i as u64)),
                        ("table",d.table_location(p.id).map(|(id,_,_)|Value::u64(id)).unwrap_or(Value::Null)),
                        ("expression",Value::string(&f.expression)),("numberFormat",Value::string(&f.number_format)),
                        ("result",Value::string(result.text)),("error",result.error.map(|e|Value::string(e.code())).unwrap_or(Value::Null))]));
                }}}
                let total=fields.len();let offset=index(input,"offset")?;let limit=input.get("limit").and_then(Value::as_u64).unwrap_or(50).clamp(1,100) as usize;
                return Ok(ActionResult::completed(Value::object([("total",Value::u64(total as u64)),("offset",Value::u64(offset as u64)),("fields",Value::Array(fields.into_iter().skip(offset).take(limit).collect()))]),self.document_revision()));
            },
            "formulas.insert"|"formulas.update"=>{
                let formula=Formula::new(&text(input,"expression"),&text(input,"numberFormat")).map_err(|e|invalid(e.code()))?;let inline=index(input,"inline")?;
                Some(if spec.name=="formulas.insert"{Edit::InsertFormula{paragraph,inline,offset:index(input,"offset")?,formula}}else{Edit::SetFormula{paragraph,inline,formula}})
            },
            "document.import"=>{let value=text(input,"text");let document=if text(input,"format")=="html"{doc_io::clipboard_html_decode(value.as_bytes())}else{doc_io::clipboard_rtf_decode(value.as_bytes())}.map_err(io_error)?;Some(Edit::ReplaceStoryFragment{paragraph,start,end,fragment:doc_core::DocumentFragment{document}})},
            "document.table.insert"=>Some(Edit::InsertTable{paragraph,offset:index(input,"offset")?,rows:index(input,"rows")?,columns:index(input,"columns")?}),
            "document.table.fit"=>Some(Edit::ChangeTable {table:id(input,"table"),change:self.table_fit_change(ui,id(input,"table"),match text(input,"mode").as_str(){"contents"=>doc_layout::TableFit::Contents,"page"=>doc_layout::TableFit::Page,_=>doc_layout::TableFit::Equal}).map_err(invalid)?}),
            "document.table.styles.list"=>return Ok(ActionResult::completed(table_style_actions::list(self.editor.document(),input),self.document_revision())),
            "document.table.styles.put"=>Some(table_style_actions::put(self.editor.document(),input)?),
            "document.table.change"=>Some(Edit::ChangeTable{table:id(input,"table"),change:table_actions::change(self.editor.document(),input)?}),
            "document.table.layout"=>{let value=self.table_layout(ui,input)?;return Ok(ActionResult::completed(value,self.document_revision()));},
            "document.table.inspect"=>return Ok(ActionResult::completed(table_actions::inspect(self.editor.document(),input)?,self.document_revision())),
            "document.export" => {
                let format=input.get("format").and_then(Value::as_str).unwrap_or("docx");
                if self.preserved()&&format!="docx"{return Err(ActionError::new(ErrorCode::Unavailable,"HTML/RTF export requires an editable document"));}
                let bytes=match format {
                    "html"=>doc_io::clipboard_html_encode(self.editor.document()),
                    "rtf"=>doc_io::clipboard_rtf_encode(self.editor.document()),
                    _=>if let Some(file) = &self.document_file { file.encode(if self.preserved() { None } else { Some(self.editor.document()) }) }
                        else { doc_io::encode_docx(self.editor.document()) },
                }.map_err(io_error)?;
                let uri = format!("jfgui://document/{}.{format}", self.document_revision());
                self.exported_document = Some((uri.clone(), bytes));
                let resource = self.document_resource_info(&uri)?;
                return Ok(ActionResult { resources: alloc::vec![resource], ..ActionResult::completed(Value::object([]), self.document_revision()) });
            }
            "document.styles.list" => return Ok(ActionResult::completed(style_actions::list(self.editor.document(), input), self.document_revision())),
            "document.styles.put" => Some(Edit::SetStyleDefinition(style_actions::definition(self.editor.document(), input)?)),
            "document.inspect" => return Ok(ActionResult::completed(self.document_inspect(input)?, self.document_revision())),
            "document.new" | "file.open" => {
                if self.document_dirty() && !flag(input, "discardChanges") { return Err(ActionError::new(ErrorCode::Unavailable, "Save current changes or explicitly set discardChanges")); }
                if spec.name == "document.new" { self.perform(ui, NEW); }
                else { self.open_document(ui, &text(input, "path")).map_err(io_error)?; }
                None
            }
            "file.save" => {
                let path = text(input, "path");
                if path != self.path && plat::file_exists(&path) != 0 && !flag(input, "overwrite") { return Err(ActionError::new(ErrorCode::Unavailable, "Destination exists; set overwrite explicitly")); }
                if !self.save(ui, path) { return Err(ActionError::new(ErrorCode::Unavailable, self.message.clone())); }
                #[cfg(target_arch = "wasm32")]
                if let Some(save) = &self.browser_save { return Ok(ActionResult::deferred(save.id as u64, self.document_revision())); }
                None
            }
            "document.undo" | "document.redo" => { self.history(ui, spec.name == "document.redo"); None }
            "document.select" => { self.rich_select(ui, paragraph, start, end).map_err(|e| invalid(format!("Invalid selection: {e:?}")))?; None }
            "document.text.replace" => Some(Edit::ReplaceStoryText { paragraph, start, end, text: text(input, "text"), paragraph_breaks: input.get("paragraphBreaks").and_then(Value::as_bool).unwrap_or(true) }),
            "document.text.format.reset" => Some(Edit::ReplaceStoryFormat { paragraph, start, end, format: TextFormat::default() }),
            "document.text.format.inspect" | "document.text.format.summary" => {
                let format = if spec.name == "document.text.format.summary" { self.editor.document().story_character_common(paragraph, start, end) }
                    else { self.editor.document().story_character_format(paragraph, start, end) }
                    .map_err(|e| invalid(format!("No editable format at this range: {e:?}")))?;
                return Ok(ActionResult::completed(format_json(format), self.document_revision()));
            }
            "document.text.format" | "document.text.format.replace" => {
                let format = input.get("format").unwrap();
                let format = style_actions::character(format)?;
                Some(if spec.name == "document.text.format.replace" {
                    Edit::ReplaceStoryFormat { paragraph, start, end, format }
                } else { Edit::FormatStoryText { paragraph, start, end, format } })
            }
            "document.paragraph.style" => Some(Edit::SetStyle { paragraph, style: style_actions::parse_key(&text(input, "style"))? }),
            "document.paragraph.format" => Some(Edit::FormatStoryParagraphs { paragraph, start, end, change: doc_core::ParagraphChange::Properties(paragraph_actions::parse(input.get("format").unwrap())?) }),
            "document.paragraph.tabs" => Some(Edit::FormatStoryParagraphs { paragraph, start, end, change: doc_core::ParagraphChange::Tabs(paragraph_actions::tabs(input.get("tabs").unwrap())?) }),
            "document.page.set" => Some(Edit::SetPage(paragraph_actions::page(input))),
            "document.paragraph.add" => Some(Edit::AddParagraph { after: id(input, "id") }),
            "document.paragraph.remove" => Some(Edit::RemoveParagraph(id(input, "id"))),
            "sources.put" => Some(Edit::PutSource(source(input))),
            "sources.remove" => Some(Edit::RemoveSource(id(input, "id"))),
            "sources.import" => {
                let content = text(input, "text");
                let sources = match text(input, "format").as_str() { "bibtex" => doc_io::decode_bibtex(content.as_bytes()), "ris" => doc_io::decode_ris(content.as_bytes()), _ => doc_io::decode_sources(content.as_bytes()) }.map_err(io_error)?;
                Some(Edit::ImportSources(sources))
            }
            "citations.insert" => Some(Edit::InsertCitation { paragraph, inline: index(input, "inline")?, offset: index(input, "offset")?, items: citation_items(input) }),
            "citations.update" => Some(Edit::UpdateCitation { paragraph, inline: index(input, "inline")?, items: citation_items(input) }),
            "fields.remove" => { self.editor.remove_field(paragraph, index(input, "inline")?).map_err(|e| invalid(format!("Field cannot be removed: {e:?}")))?; self.widget_ids.clear(); None }
            "bibliography.insert" => Some(Edit::InsertBibliography { after: id(input, "id") }),
            "notes.insert" => Some(Edit::InsertNote { paragraph, inline: index(input, "inline")?, offset: index(input, "offset")?, kind: kind(input), text: text(input, "text") }),
            "notes.kind" => Some(Edit::SetNoteKind { note: id(input, "id"), kind: kind(input) }),
            "notes.remove" => Some(Edit::RemoveNote(id(input, "id"))),
            "clipboard.table.copy"|"clipboard.table.cut"=>{
                if !self.table_clipboard(ui,id(input,"table"),spec.name=="clipboard.table.cut"){return Err(ActionError::new(ErrorCode::Unavailable,self.message.clone()));}None
            },
            "clipboard.copy" | "clipboard.cut" | "clipboard.paste" => {
                let before = self.message.clone();
                if !self.rich_clipboard(ui, match spec.name { "clipboard.copy" => 70, "clipboard.cut" => 71, _ => 72 }) {
                    return Err(ActionError::new(ErrorCode::Unavailable, if self.message == before {
                        String::from("Clipboard action needs a valid selection and supported clipboard data")
                    } else { self.message.clone() }));
                }
                None
            }
            _ => return Err(ActionError::new(ErrorCode::NotFound, "Unknown jfWord action")),
        };
        if let Some(edit) = edit {
            result_id = Some(self.apply_page_edit(ui,edit).ok_or_else(|| invalid(self.message.clone()))?);
            self.shell.home = false;
            self.shell.has_document = true;
            self.widget_ids.clear();
            self.selected = self.selected.min(self.current_paragraphs().len().saturating_sub(1));
        }
        ui.invalidate();
        Ok(ActionResult::completed(Value::object([("id", result_id.map(Value::u64).unwrap_or(Value::Null)), ("dirty", Value::Bool(self.document_dirty()))]), self.document_revision()))
    }
}
