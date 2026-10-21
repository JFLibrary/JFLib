// Bounded rich clipboard interchange. New native code, no assembly counterpart.
// RTF control syntax follows Microsoft's RTF specification; this is a supported profile,
// not a general RTF file editor. Internal structured fragments use the DOCX codec instead.
use crate::{Document, Error, Inline, Paragraph, ParagraphFormat, ParagraphStyle, TextFormat};
use alloc::{format, string::String, vec::Vec};
use doc_core::{Alignment, FontFamily, Highlight, ResolvedFormat, TextColor, TextPositioning};

#[path="rtf_tables.rs"] mod rtf_tables;

const MAX_BYTES: usize = 16 << 20;
const MAX_TEXT: usize = 2 << 20;
const MAX_TOKENS: usize = 4 << 20;
const MAX_COLORS: usize = 1024;

fn unsupported(name: &str) -> Error {
    crate::unsupported(&format!("RTF clipboard: {name}"))
}
fn put(out: &mut String, text: &str) -> Result<(), Error> {
    if text.len() > MAX_BYTES.saturating_sub(out.len()) {
        return Err(Error::Limit);
    }
    out.push_str(text);
    Ok(())
}
fn escaped(out: &mut String, text: &str) -> Result<(), Error> {
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => put(out, "\\\\")?,
            '{' => put(out, "\\{")?,
            '}' => put(out, "\\}")?,
            '\n' | '\r' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                put(out, "\\line ")?;
            }
            '\t' => put(out, "\\tab ")?,
            ' '..='~' => {
                if out.len() == MAX_BYTES {
                    return Err(Error::Limit);
                }
                out.push(c);
            }
            _ => {
                let mut units = [0; 2];
                for u in c.encode_utf16(&mut units) {
                    put(out, &format!("\\u{}?", *u as i16))?;
                }
            }
        }
    }
    Ok(())
}
fn shown(d: &Document, p: &Paragraph, node: &Inline) -> String {
    match node {
        Inline::Text(s) | Inline::StyledText { text: s, .. } => s.clone(),
        Inline::Citation { source, locator } => d.citation_text(*source, locator),
        Inline::CitationCluster(items) => d.citation_cluster_text(items),
        Inline::Bibliography => d.bibliography().join("\n"),
        Inline::NoteReference(id) => d.note_marker(*id),
        Inline::Formula(f) => d.evaluate_formula(p.id,f).text,
    }
}
fn rtf_format(out: &mut String, f: ResolvedFormat, colors: &[u32], fonts: &[FontFamily]) -> Result<(), Error> {
    let font = fonts.iter().position(|name| *name == f.font_family).ok_or(Error::Invalid)?;
    let color = colors
        .iter()
        .position(|c| *c == f.color & 0xffffff)
        .ok_or(Error::Invalid)?
        + 1;
    let highlight = match f.highlight.rgb() {
        None => 0,
        Some(rgb) => colors.iter().position(|c| *c == rgb).ok_or(Error::Invalid)? + 1,
    };
    let positioning = match f.positioning { TextPositioning::Baseline => "nosupersub",
        TextPositioning::Superscript => "super", TextPositioning::Subscript => "sub" };
    put(
        out,
        &format!(
            "\\plain\\f{font}\\b{}\\i{}\\ul{}\\strike{}\\fs{}\\cf{color}\\highlight{highlight}\\{positioning} ",
            u8::from(f.bold),
            u8::from(f.italic),
            u8::from(f.underline),
            u8::from(f.strike),
            f.size_half_points
        ),
    )
}

/// Encode body paragraphs as RTF 1 with explicit resolved character formatting, RGB colors,
/// paragraph alignment, twip spacing/indents and Unicode. Hard paragraphs and soft breaks
/// remain distinct. Fields and note anchors become their visible text; source/note metadata
/// and style identities are not transferred. Font families and effective formatting remain.
/// Use DOCX for a structured internal copy including the style graph.
/// CRLF and lone CR inside text normalize to a single soft break.
/// The output is bounded to 16 MiB and color tables to 1024 entries.
pub fn clipboard_rtf_encode(d: &Document) -> Result<Vec<u8>, Error> {
    d.validate()?;
    let mut colors = Vec::new();
    let mut fonts = Vec::new();
    for p in d.body_paragraphs() {
        for f in p
            .content
            .iter()
            .map(Inline::format)
            .chain(core::iter::once(TextFormat::default()))
        {
            let resolved = d.resolve_paragraph_text(p, f);
            if !fonts.contains(&resolved.font_family) { fonts.push(resolved.font_family); }
            if fonts.len() > 256 { return Err(Error::Limit); }
            for color in core::iter::once(resolved.color & 0xffffff).chain(resolved.highlight.rgb()) {
                if !colors.contains(&color) {
                    if colors.len() == MAX_COLORS - 1 { return Err(Error::Limit); }
                    colors.push(color);
                }
            }
        }
    }
    for t in &d.tables {for rgb in core::iter::once(d.table_border(t).1).chain(t.anchors().filter_map(|(r,c,_)|d.table_region(t,r,c).fill)){if !colors.contains(&rgb){if colors.len()==MAX_COLORS-1{return Err(Error::Limit);}colors.push(rgb);}}}
    for t in &d.tables {for (r,c,_) in t.anchors(){for side in doc_core::BorderSide::ALL{let b=d.cell_border(t,r,c,side);if b.visible() && !colors.contains(&b.color){if colors.len()==MAX_COLORS-1{return Err(Error::Limit);}colors.push(b.color);}}}}
    let mut out = String::from("{\\rtf1\\ansi\\ansicpg1252\\deff0\\uc1{\\fonttbl");
    for (i, name) in fonts.iter().enumerate() {
        if name.as_str().contains(';') { return Err(unsupported("font name delimiter")); }
        put(&mut out, &format!("{{\\f{i}\\fnil\\fcharset0 "))?;
        escaped(&mut out, name.as_str())?;
        put(&mut out, ";}")?;
    }
    put(&mut out, "}{\\colortbl;")?;
    for color in &colors {
        put(
            &mut out,
            &format!(
                "\\red{}\\green{}\\blue{};",
                color >> 16,
                (color >> 8) & 255,
                color & 255
            ),
        )?;
    }
    put(&mut out, "}\n")?;
    for p in &d.paragraphs {
        for t in d.tables.iter().filter(|t|t.before==p.id){write_rtf_table(&mut out,d,t,&colors,&fonts,1)?;}
        write_rtf_paragraph(&mut out,d,p,&colors,&fonts,0,false)?;
    }
    put(&mut out, "}")?;
    Ok(out.into_bytes())
}

fn write_rtf_table(out:&mut String,d:&Document,t:&doc_core::Table,colors:&[u32],fonts:&[FontFamily],depth:usize)->Result<(),Error>{
            let (border_width,border_color)=d.table_border(t);
            for (r,row) in t.rows.iter().enumerate() {
                let mut props=String::new();
                let mut edge=t.columns[..usize::from(row.grid_before)].iter().sum::<u32>();
                let height=row.min_height as i32 * if row.height_rule==doc_core::RowHeightRule::Exact{-1}else{1};
                put(&mut props,&format!("\\trowd\\trgaph{}\\trleft{edge}\\trrh{height} ",t.padding))?;
                if let Some(p)=t.position{put(&mut props,&crate::table_position::rtf(p))?;}
                if row.cant_split {put(&mut props,"\\trkeep ")?;}
                if r<usize::from(t.header_rows) {put(&mut props,"\\trhdr ")?;}
                for i in usize::from(row.grid_before)..row.cells.len()-usize::from(row.grid_after){
                    let (ar,ac)=t.origin(r,i).ok_or(Error::Invalid)?;
                    let c=&t.rows[ar].cells[ac];
                    if c.column_span>1 {put(&mut props,if i==ac {"\\clmgf"} else {"\\clmrg"})?;}
                    if c.row_span>1 {put(&mut props,if r==ar {"\\clvmgf"} else {"\\clvmrg"})?;}
                    if let Some(rgb)=d.table_region(t,ar,ac).fill {let color=colors.iter().position(|v|*v==rgb).ok_or(Error::Invalid)?+1;put(&mut props,&format!("\\clcbpat{color}"))?;}
                    let valign=match c.alignment{doc_core::CellAlignment::Top=>"t",doc_core::CellAlignment::Center=>"c",doc_core::CellAlignment::Bottom=>"b"};put(&mut props,&format!("\\clvertal{valign}"))?;
                    if c.borders.iter().any(Option::is_some) {
                        for side in doc_core::BorderSide::ALL {
                            let b=d.cell_border(t,ar,ac,side);let letter=match side{doc_core::BorderSide::Top=>"t",doc_core::BorderSide::Right=>"r",doc_core::BorderSide::Bottom=>"b",doc_core::BorderSide::Left=>"l"};
                            put(&mut props,&format!("\\clbrdr{letter}"))?;
                            if b.visible(){let color=colors.iter().position(|v|*v==b.color).ok_or(Error::Invalid)?+1;let style=match b.style{doc_core::BorderStyle::Double=>"db",doc_core::BorderStyle::Dotted=>"dot",doc_core::BorderStyle::Dashed=>"dash",_=>"s"};put(&mut props,&format!("\\brdr{style}\\brdrw{}\\brdrcf{color}",u32::from(b.width)*5/2))?;}else{put(&mut props,"\\brdrnil")?;}
                        }
                    }else if border_width>0 {let color=colors.iter().position(|v|*v==border_color).ok_or(Error::Invalid)?+1;for side in ["t","l","b","r"] {put(&mut props,&format!("\\clbrdr{side}\\brdrs\\brdrw{}\\brdrcf{color}",u32::from(border_width)*5/2))?;}}
                    edge+=t.columns[i];put(&mut props,&format!("\\cellx{edge}"))?;
                }
                if depth==1 {put(out,&props)?;}
                for c in &row.cells[usize::from(row.grid_before)..row.cells.len()-usize::from(row.grid_after)] {
                    if c.is_covered(){put(out,&format!("\\pard\\intbl\\itap{depth} "))?;put(out,if depth>1{"\\nestcell "}else{"\\cell "})?;}
                    for (i,p) in c.paragraphs.iter().enumerate(){
                        for child in d.tables.iter().filter(|t|t.before==p.id){write_rtf_table(out,d,child,colors,fonts,depth+1)?;}
                        write_rtf_paragraph(out,d,p,colors,fonts,depth,i+1==c.paragraphs.len())?;
                    }
                }
                if depth>1 {put(out,"{\\*\\nesttableprops")?;put(out,&props)?;put(out,"\\nestrow}\n")?;}else{put(out,"\\row\n")?;}
            }
    Ok(())
}

fn write_rtf_paragraph(out:&mut String,d:&Document,p:&Paragraph,colors:&[u32],fonts:&[FontFamily],depth:usize,last_cell:bool)->Result<(),Error>{
        let f = d.resolve_paragraph(p);
        let align = match f.alignment {
            Alignment::Left => "ql",
            Alignment::Center => "qc",
            Alignment::Right => "qr",
            Alignment::Justify => "qj",
        };
        put(
            out,
            &format!(
                "\\pard\\{align}\\sb{}\\sa{}\\sl{}\\slmult1\\li{}\\ri{}\\fi{}\\pagebb{} ",
                f.before,
                f.after,
                f.line,
                f.left,
                f.right,
                f.first_line,
                u8::from(f.page_break_before)
            ),
        )?;
        for tab in f.tabs.as_slice() {
            use doc_core::{TabAlignment, TabLeader};
            let alignment = match tab.alignment { TabAlignment::Center => "\\tqc", TabAlignment::Right => "\\tqr", TabAlignment::Decimal => "\\tqdec", _ => "" };
            let leader = match tab.leader { TabLeader::None => "", TabLeader::Dot => "\\tldot", TabLeader::Hyphen => "\\tlhyph", TabLeader::Underscore => "\\tlul", TabLeader::Heavy => "\\tlth", TabLeader::MiddleDot => "\\tlmdot" };
            put(out, &format!("{alignment}{leader}\\{}{} ", if tab.alignment == TabAlignment::Bar { "tb" } else { "tx" }, tab.position))?;
        }
        if depth>0 { put(out,&format!("\\intbl\\itap{depth} "))?; }
        rtf_format(out, d.resolve_paragraph_text(p, TextFormat::default()), colors, fonts)?;
        for node in &p.content {
            let f = d.resolve_paragraph_text(p, node.format());
            rtf_format(out, f, colors, fonts)?;
            escaped(out, &shown(d, p, node))?;
        }
        put(out, if last_cell {if depth>1{"\\nestcell\n"}else{"\\cell\n"}} else {"\\par\n"})?;
    Ok(())
}

fn css_family(family: FontFamily) -> String {
    family.as_str().chars().map(|c| if c.is_ascii_alphanumeric() || c == ' ' || c == '-' { alloc::format!("{c}") }
        else { alloc::format!("\\{:x} ", c as u32) }).collect()
}
fn points(twips: i32) -> String {
    let magnitude = twips.unsigned_abs();
    format!(
        "{}{}.{:02}pt",
        if twips < 0 { "-" } else { "" },
        magnitude / 20,
        magnitude % 20 * 5
    )
}
fn html_text(out: &mut String, text: &str) -> Result<(), Error> {
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '&' => put(out, "&amp;")?,
            '<' => put(out, "&lt;")?,
            '>' => put(out, "&gt;")?,
            '"' => put(out, "&quot;")?,
            '\'' => put(out, "&#39;")?,
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                put(out, "<br>")?;
            }
            _ => {
                let mut utf8 = [0; 4];
                put(out, c.encode_utf8(&mut utf8))?;
            }
        }
    }
    Ok(())
}
/// Encode a raw UTF-8 HTML clipboard fragment with inline CSS for resolved character and
/// paragraph formatting. No CF_HTML envelope is included: that belongs to the platform
/// clipboard layer. Font families use escaped CSS names; fields/anchors become visible text and
/// source/note metadata stays in the private DOCX format. The result is bounded to 16 MiB.
/// Floating tables carry versioned exact anchors plus CSS float fallback; page units are twips.
pub fn clipboard_html_encode(d: &Document) -> Result<Vec<u8>, Error> {
    d.validate()?;
    let p=d.page;
    let mut out = format!("<div data-jf-page=\"1 {} {} {} {} {} {}\" style=\"font-family:DejaVu Sans,sans-serif;white-space:pre-wrap\">",p.width,p.height,p.top,p.right,p.bottom,p.left);
    for p in &d.paragraphs {for t in d.tables.iter().filter(|t|t.before==p.id){html_table(&mut out,d,t)?;}html_paragraph(&mut out,d,p)?;}
    put(&mut out, "</div>")?;
    Ok(out.into_bytes())
}

fn html_table(out:&mut String,d:&Document,t:&doc_core::Table)->Result<(),Error>{
            let (border_width,border_color)=d.table_border(t);
            let mut attrs=String::new();let mut css=String::new();
            if let Some(p)=t.position {
                let h=match p.horizontal{doc_core::TableHorizontal::Page=>"page",doc_core::TableHorizontal::Margin=>"margin",doc_core::TableHorizontal::Column=>"column"};
                let v=match p.vertical{doc_core::TableVertical::Page=>"page",doc_core::TableVertical::Margin=>"margin",doc_core::TableVertical::Paragraph=>"paragraph"};
                let [top,right,bottom,left]=p.distance;
                attrs=format!(" data-jf-table-position=\"1 {h} {v} {} {} {top} {right} {bottom} {left} {}\"",p.x,p.y,u8::from(p.overlap));
                // CSS float is a continuous-page fallback. The attribute retains exact anchors.
                let x=d.table_x(t)? as i32-d.page.left as i32;
                let free_right=d.page.content_width() as i32-x-t.width() as i32;
                let (side,l,r)=if x-i32::from(left)>free_right-i32::from(right){("right",i32::from(left),free_right.max(0))}else{("left",x.max(0),i32::from(right))};
                css=format!(";float:{side};margin:{} {} {} {}",points(p.y+i32::from(top)),points(r),points(i32::from(bottom)),points(l));
            }
            put(out,&format!("<table{attrs} style=\"border-collapse:collapse;table-layout:fixed;width:{}{css}\"><colgroup>",points(t.width() as i32)))?;
            for width in &t.columns {put(out,&format!("<col style=\"width:{}\">",points(*width as i32)))?;}put(out,"</colgroup>")?;
            if t.header_rows>0 {put(out,"<thead>")?;}
            for (r,row) in t.rows.iter().enumerate() {
                if r==usize::from(t.header_rows) {if r>0 {put(out,"</thead>")?;}put(out,"<tbody>")?;}
                let exact=row.height_rule==doc_core::RowHeightRule::Exact;
                let height_attr=if exact{format!(" data-jf-row-height=\"1 exact {}\"",row.min_height)}else{String::new()};
                put(out,&format!("<tr{height_attr} style=\"height:{}{}{}\">",points(row.min_height as i32),if exact{";mso-height-rule:exactly"}else{""},if row.cant_split {";break-inside:avoid;page-break-inside:avoid"}else{""}))?;
                if row.grid_before>0{put(out,&format!("<td data-jf-grid-gap=\"before\" colspan=\"{}\" style=\"border:none;padding:0\"></td>",row.grid_before))?;}
                for (col,c) in row.cells.iter().enumerate().filter(|(_,c)|!c.is_covered()) {
                    let fill=d.table_region(t,r,col).fill.map_or_else(||String::from("transparent"),|rgb|format!("#{rgb:06X}"));
                    let align=match c.alignment{doc_core::CellAlignment::Top=>"top",doc_core::CellAlignment::Center=>"middle",doc_core::CellAlignment::Bottom=>"bottom"};
                    let span=if c.row_span>1 || c.column_span>1 {format!(" rowspan=\"{}\" colspan=\"{}\"",c.row_span,c.column_span)} else {String::new()};
                    let tag=if r<usize::from(t.header_rows) {"th"} else {"td"};
                    let mut borders=String::new();
                    if c.borders.iter().any(Option::is_some){for side in doc_core::BorderSide::ALL{let b=d.cell_border(t,r,col,side);let style=match b.style{doc_core::BorderStyle::Nil=>"hidden",doc_core::BorderStyle::None=>"none",doc_core::BorderStyle::Double=>"double",doc_core::BorderStyle::Dotted=>"dotted",doc_core::BorderStyle::Dashed=>"dashed",_=>"solid"};
                        // CSS double uses total width (two strokes plus a gap), Word uses stroke width.
                        let width=b.width*if b.style==doc_core::BorderStyle::Double{3}else{1};
                        borders.push_str(&format!(";border-{}:{}.{:03}pt {style} #{:06X}",side.name(),width/8,width%8*125,b.color));}}
                    put(out,&format!("<{tag}{span} style=\"padding:{};border:{} solid #{:06X}{borders};background:{fill};vertical-align:{align}\">",points(t.padding.into()),points(i32::from(border_width)*5/2),border_color))?;
                    // A block wrapper supplies a CSS clipping fallback without discarding
                    // any content. Mixed-height vertical merges may still grow naturally.
                    let rows=&t.rows[r..r+usize::from(c.row_span)];
                    let clip=rows.iter().all(|r|r.height_rule==doc_core::RowHeightRule::Exact);
                    if clip {let height=rows.iter().map(|r|r.min_height).sum::<u32>().saturating_sub(u32::from(t.padding)*2);put(out,&format!("<div style=\"max-height:{};overflow:hidden\">",points(height as i32)))?;}
                    for p in &c.paragraphs {for child in d.tables.iter().filter(|t|t.before==p.id){html_table(out,d,child)?;}html_paragraph(out,d,p)?;}
                    if clip {put(out,"</div>")?;}put(out,&format!("</{tag}>"))?;
                }if row.grid_after>0{put(out,&format!("<td data-jf-grid-gap=\"after\" colspan=\"{}\" style=\"border:none;padding:0\"></td>",row.grid_after))?;}put(out,"</tr>")?;
            }put(out,if usize::from(t.header_rows)==t.rows.len(){"</thead></table>"}else{"</tbody></table>"})?;
    Ok(())
}

fn html_paragraph(out:&mut String,d:&Document,p:&Paragraph)->Result<(),Error>{
        let f = d.resolve_paragraph(p);
        let align = match f.alignment {
            Alignment::Left => "left",
            Alignment::Center => "center",
            Alignment::Right => "right",
            Alignment::Justify => "justify",
        };
        let line = u32::from(f.line) * 1_000_000 / 240;
        let end = d.resolve_paragraph_text(p, p
            .content
            .last()
            .map(Inline::format)
            .unwrap_or_default());
        let mut tab_css = String::new();
        if !f.tabs.is_empty() {
            tab_css.push_str(";tab-stops:");
            for s in f.tabs.as_slice() {
                let leader = match s.leader { doc_core::TabLeader::Dot => " dotted", doc_core::TabLeader::Hyphen => " dashed", doc_core::TabLeader::Underscore | doc_core::TabLeader::Heavy => " solid", _ => "" };
                tab_css.push_str(&format!(" {}{leader} {}", s.alignment.name(), points(s.position)));
            }
        }
        put(
            out,
            &format!(
                "<p style=\"margin:{} {} {} {};text-align:{align};text-indent:{};line-height:{}.{:06};page-break-before:{};font-size:{}{tab_css}\">",
                points(i32::from(f.before)),
                points(i32::from(f.right)),
                points(i32::from(f.after)),
                points(i32::from(f.left)),
                points(f.first_line),
                line / 1_000_000,
                line % 1_000_000,
                if f.page_break_before {
                    "always"
                } else {
                    "auto"
                },
                points(i32::from(end.size_half_points) * 10)
            ),
        )?;
        let mut empty = true;
        for node in &p.content {
            let f = d.resolve_paragraph_text(p, node.format());
            let decoration = match (f.underline, f.strike) {
                (true, true) => "underline line-through",
                (true, false) => "underline",
                (false, true) => "line-through",
                _ => "none",
            };
            let background = f.highlight.rgb().map_or_else(|| String::from("transparent"), |rgb| format!("#{rgb:06X}"));
            // CSS vertical-align is a displacement from the parent's baseline (positive up).
            // Keep the shared numeric script metrics, rather than shrink twice via <sup>.
            let shift = if f.positioning == TextPositioning::Baseline { String::from("baseline") }
                else { points(-f.baseline_shift()) };
            put(
                out,
                &format!(
                    "<span style=\"font-family:'{}';font-weight:{};font-style:{};text-decoration:{decoration};font-size:{};color:#{:06X};background-color:{background};vertical-align:{shift}\">",
                    css_family(f.font_family),
                    if f.bold { "bold" } else { "normal" },
                    if f.italic { "italic" } else { "normal" },
                    points(i32::from(f.display_size_half_points()) * 10),
                    f.color & 0xffffff
                ),
            )?;
            let text = shown(d, p, node);
            empty &= text.is_empty();
            html_text(out, &text)?;
            put(out, "</span>")?;
        }
        if empty {
            put(out, "<br>")?;
        }
        put(out, "</p>")?;
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Destination {
    Body,
    Skip,
    Colors,
    Fonts,
    Field,
}
#[derive(Clone, Copy)]
struct State {
    table_depth: usize,
    tab_alignment: doc_core::TabAlignment,
    tab_leader: doc_core::TabLeader,
    destination: Destination,
    text: TextFormat,
    paragraph: ParagraphFormat,
    font: i32,
    uc: usize,
    start: bool,
    starred: bool,
    line: i32,
    multiple: bool,
    field_result: bool,
}
fn rtf_text() -> TextFormat {
    TextFormat {
        font_family: None,
        bold: Some(false),
        italic: Some(false),
        underline: Some(false),
        strike: Some(false),
        color: Some(TextColor::Auto),
        size_half_points: Some(24),
        highlight: Some(Highlight::None), positioning: Some(TextPositioning::Baseline),
    }
}
fn rtf_paragraph() -> ParagraphFormat {
    ParagraphFormat {
        before: 0,
        after: 0,
        line: 240,
        ..ParagraphFormat::default()
    }
}
impl Default for State {
    fn default() -> Self {
        Self {
            table_depth: 0, tab_alignment: Default::default(), tab_leader: Default::default(),
            destination: Destination::Body,
            text: rtf_text(),
            paragraph: rtf_paragraph(),
            font: 0,
            uc: 1,
            start: true,
            starred: false,
            line: 0,
            multiple: false,
            field_result: false,
        }
    }
}
struct Reader<'a> {
    tables: rtf_tables::Tables,
    current_depth: usize,
    bytes: &'a [u8],
    pos: usize,
    tokens: usize,
    state: State,
    stack: Vec<State>,
    document: Document,
    current: Vec<Inline>,
    paragraph: ParagraphFormat,
    paragraph_line: i32,
    paragraph_multiple: bool,
    text_bytes: usize,
    skip: usize,
    high: Option<(u16, TextFormat)>,
    colors: Vec<TextColor>,
    rgb: [u8; 3],
    color_set: bool,
    color_table: bool,
    fonts: Vec<(i32, i32)>,
    font_names: Vec<(i32, String)>,
    codepage: i32,
    default_font: i32,
    header: bool,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            tables: Default::default(), current_depth: 0,
            pos: 0,
            tokens: 0,
            state: State::default(),
            stack: Vec::new(),
            document: Document {
                paragraphs: Vec::new(),
                ..Document::default()
            },
            current: Vec::new(),
            paragraph: rtf_paragraph(),
            paragraph_line: 0,
            paragraph_multiple: false,
            text_bytes: 0,
            skip: 0,
            high: None,
            colors: Vec::new(),
            rgb: [0; 3],
            color_set: false,
            color_table: false,
            fonts: Vec::new(),
            font_names: Vec::new(),
            codepage: 1252,
            default_font: 0,
            header: false,
        }
    }
    fn text(&mut self, c: char) -> Result<(), Error> {
        if self.high.is_some() {
            return Err(Error::Invalid);
        }
        self.append(c)
    }
    fn append(&mut self, c: char) -> Result<(), Error> {
        if self.state.destination == Destination::Fonts {
            let (_, name) = self.font_names.iter_mut().find(|f| f.0 == self.state.font).ok_or(Error::Invalid)?;
            if c == ';' { return Ok(()); }
            if name.len() + c.len_utf8() > 96 { return Err(Error::Limit); }
            name.push(c);
            return Ok(());
        }
        self.state.text.font_family = self.font_names.iter().find(|f| f.0 == self.state.font)
            .map(|f| FontFamily::new(f.1.trim())).transpose()?;

        if !(c == '\t' || c == '\n' || c >= ' ' && c != '\u{fffe}' && c != '\u{ffff}') {
            return Err(Error::Invalid);
        }
        if self.text_bytes + c.len_utf8() > MAX_TEXT {
            return Err(Error::Limit);
        }
        self.text_bytes += c.len_utf8();
        if self.current.is_empty() {
            self.current_depth = self.state.table_depth;
            self.paragraph = self.paragraph_format()?;
            self.paragraph_line = self.state.line;
            self.paragraph_multiple = self.state.multiple;
        }
        if let Some(Inline::StyledText { text, format }) = self.current.last_mut() {
            if *format == self.state.text {
                text.push(c);
                return Ok(());
            }
        }
        if self.current.len() == 8192 {
            return Err(Error::Limit);
        }
        let mut text = String::new();
        text.push(c);
        self.current.push(Inline::StyledText {
            text,
            format: self.state.text,
        });
        Ok(())
    }
    fn unicode(&mut self, value: i32) -> Result<(), Error> {
        if !(-32768..=65535).contains(&value) {
            return Err(Error::Invalid);
        }
        let u = value as u16;
        if let Some((high, format)) = self.high.take() {
            if !(0xdc00..=0xdfff).contains(&u) || format != self.state.text {
                return Err(Error::Invalid);
            }
            return self.append(
                char::from_u32(
                    0x10000 + ((u32::from(high) - 0xd800) << 10) + u32::from(u) - 0xdc00,
                )
                .ok_or(Error::Invalid)?,
            );
        }
        if (0xd800..=0xdbff).contains(&u) {
            self.high = Some((u, self.state.text));
            Ok(())
        } else {
            self.text(char::from_u32(u32::from(u)).ok_or(Error::Invalid)?)
        }
    }
    fn byte(&mut self, value: u8) -> Result<(), Error> {
        if self.skip != 0 {
            self.skip -= 1;
            return Ok(());
        }
        match self.state.destination {
            Destination::Body | Destination::Fonts => {
                let cp = self
                    .fonts
                    .iter()
                    .find(|f| f.0 == self.state.font)
                    .map(|f| f.1)
                    .unwrap_or(0);
                let cp = if cp == 0 { self.codepage } else { cp };
                if cp == -2 || value >= 128 && !supported_codepage(cp) {
                    return Err(unsupported("character code page/font"));
                }
                self.text(codepage_character(value, cp).ok_or(Error::Invalid)?)
            }
            Destination::Colors if value == b';' => {
                if self.colors.len() == MAX_COLORS {
                    return Err(Error::Limit);
                }
                self.colors.push(if self.color_set {
                    TextColor::Rgb(
                        (u32::from(self.rgb[0]) << 16)
                            | (u32::from(self.rgb[1]) << 8)
                            | u32::from(self.rgb[2]),
                    )
                } else {
                    TextColor::Auto
                });
                self.rgb = [0; 3];
                self.color_set = false;
                Ok(())
            }
            Destination::Colors if value != b' ' => Err(Error::Invalid),
            Destination::Field if value != b' ' => Err(Error::Invalid),
            _ => Ok(()),
        }
    }
    fn paragraph_format(&self) -> Result<ParagraphFormat, Error> {
        let mut f = self.state.paragraph;
        if self.state.line != 0 {
            if !self.state.multiple || !(240..=720).contains(&self.state.line) {
                return Err(unsupported("absolute or unsupported line spacing"));
            }
            f.line = self.state.line as u16;
        }
        f.validate(self.document.page)
            .map_err(|_| unsupported("paragraph dimensions"))?;
        Ok(f)
    }
    fn paragraph_end(&mut self) -> Result<(), Error> {
        if self.high.is_some() {
            return Err(Error::Invalid);
        }
        if self.document.paragraphs.len() == 2048 {
            return Err(Error::Limit);
        }
        let format = if self.current.is_empty() {
            self.paragraph_format()?
        } else {
            if self.paragraph_line != 0
                && (!self.paragraph_multiple || !(240..=720).contains(&self.paragraph_line))
            {
                return Err(unsupported("absolute or unsupported line spacing"));
            }
            self.paragraph
        };
        let mut content = core::mem::take(&mut self.current);
        if content.is_empty() {
            let mut text = self.state.text;
            text.font_family = self.font_names.iter().find(|f| f.0 == self.state.font).map(|f| FontFamily::new(f.1.trim())).transpose()?;
            content.push(Inline::with_format(String::new(), text));
        }
        let depth=if self.current.is_empty() && content.iter().all(|i|i.text()==Some("")){self.state.table_depth}else{self.current_depth};
        self.tables.paragraph(depth,Paragraph {
            id: 0,
            style: ParagraphStyle::Normal,
            format,
            content,
        })?;
        Ok(())
    }
    fn run(mut self) -> Result<Document, Error> {
        while self.pos < self.bytes.len() {
            self.tokens += 1;
            if self.tokens > MAX_TOKENS {
                return Err(Error::Limit);
            }
            let c = self.bytes[self.pos];
            self.pos += 1;
            match c {
                b'{' => {
                    if self.stack.len() == 64 || self.stack.is_empty() && self.header {
                        return Err(Error::Invalid);
                    }
                    self.stack.push(self.state);
                    self.state.start = true;
                    self.state.starred = false;
                    self.skip = 0;
                }
                b'}' => {
                    if self.state.destination == Destination::Colors && self.color_set {
                        return Err(Error::Invalid);
                    }
                    if self.state.destination == Destination::Field && !self.state.field_result {
                        return Err(unsupported("field without cached result"));
                    }
                    self.state = self.stack.pop().ok_or(Error::Invalid)?;
                    self.skip = 0;
                }
                b'\r' | b'\n' => (),
                b' ' | b'\t' if self.stack.is_empty() => (),
                _ if self.stack.is_empty() => return Err(Error::Invalid),
                b'\\' => self.control()?,
                _ => {
                    if !self.header {
                        return Err(Error::Invalid);
                    }
                    self.state.start = false;
                    self.byte(c)?;
                }
            }
        }
        if !self.header || !self.stack.is_empty() || self.high.is_some() {
            return Err(Error::Invalid);
        }
        if !self.current.is_empty() { self.paragraph_end()?; }
        self.tables.finish()
    }
    fn control(&mut self) -> Result<(), Error> {
        let c = *self.bytes.get(self.pos).ok_or(Error::Invalid)?;
        self.pos += 1;
        if !c.is_ascii_alphabetic() {
            if c == b'\'' {
                let a = hex(*self.bytes.get(self.pos).ok_or(Error::Invalid)?)?;
                let b = hex(*self.bytes.get(self.pos + 1).ok_or(Error::Invalid)?)?;
                self.pos += 2;
                self.state.start = false;
                return self.byte(a * 16 + b);
            }
            if self.skip != 0 {
                self.skip -= 1;
                return Ok(());
            }
            if c == b'*' && self.state.start && !self.state.starred {
                self.state.starred = true;
                return Ok(());
            }
            self.state.start = false;
            if self.state.destination != Destination::Body && self.state.destination != Destination::Fonts {
                return Ok(());
            }
            match c {
                b'\\' | b'{' | b'}' => self.byte(c),
                b'~' => self.text('\u{a0}'),
                b'_' => self.text('\u{2011}'),
                b'-' => self.text('\u{ad}'),
                b'\r' | b'\n' => self.text('\n'),
                _ => Err(unsupported("control symbol")),
            }
        } else {
            let start = self.pos - 1;
            while self
                .bytes
                .get(self.pos)
                .is_some_and(u8::is_ascii_alphabetic)
            {
                self.pos += 1;
            }
            if self.pos - start > 32 {
                return Err(Error::Invalid);
            }
            let name =
                core::str::from_utf8(&self.bytes[start..self.pos]).map_err(|_| Error::Invalid)?;
            let negative = self.bytes.get(self.pos) == Some(&b'-');
            if negative {
                self.pos += 1;
            }
            let digits = self.pos;
            let mut number = 0i32;
            while let Some(v @ b'0'..=b'9') = self.bytes.get(self.pos) {
                number = number
                    .checked_mul(10)
                    .and_then(|n| n.checked_add(i32::from(v - b'0')))
                    .ok_or(Error::Invalid)?;
                self.pos += 1;
            }
            if negative && self.pos == digits {
                return Err(Error::Invalid);
            }
            let value = (self.pos != digits).then_some(if negative { -number } else { number });
            if self.bytes.get(self.pos) == Some(&b' ') {
                self.pos += 1;
            }
            if name == "bin" {
                let n =
                    usize::try_from(value.ok_or(Error::Invalid)?).map_err(|_| Error::Invalid)?;
                self.pos = self
                    .pos
                    .checked_add(n)
                    .filter(|n| *n <= self.bytes.len())
                    .ok_or(Error::Invalid)?;
                if self.skip != 0 {
                    self.skip -= 1;
                    return Ok(());
                }
                return if self.state.destination == Destination::Skip {
                    Ok(())
                } else {
                    Err(unsupported("binary content"))
                };
            }
            if self.skip != 0 {
                self.skip -= 1;
                return Ok(());
            }
            self.word(name, value)
        }
    }
    fn word(&mut self, name: &str, value: Option<i32>) -> Result<(), Error> {
        let initial = self.state.start;
        let starred = self.state.starred;
        self.state.start = false;
        self.state.starred = false;
        if !self.header {
            if name != "rtf" || value != Some(1) || self.stack.len() != 1 || !initial {
                return Err(Error::Invalid);
            }
            self.header = true;
            return Ok(());
        }
        if self.state.destination == Destination::Skip {
            return Ok(());
        }
        if initial {
            match name {
                "nesttableprops" => { if self.state.table_depth<2{return Err(Error::Invalid);}return Ok(()); },
                "nonesttables" => {self.state.destination=Destination::Skip;return Ok(());},
                "fonttbl" => {
                    self.state.destination = Destination::Fonts;
                    return Ok(());
                }
                "colortbl" => {
                    if self.color_table {
                        return Err(Error::Invalid);
                    }
                    self.color_table = true;
                    self.state.destination = Destination::Colors;
                    return Ok(());
                }
                "field" => {
                    self.state.destination = Destination::Field;
                    self.state.field_result = false;
                    return Ok(());
                }
                "fldrslt" if self.state.destination == Destination::Field => {
                    let parent = self.stack.last_mut().ok_or(Error::Invalid)?;
                    if parent.destination != Destination::Field || parent.field_result {
                        return Err(Error::Invalid);
                    }
                    parent.field_result = true;
                    self.state.destination = Destination::Body;
                    return Ok(());
                }
                "fldinst" | "info" | "stylesheet" | "generator" | "listtable"
                | "listoverridetable" | "rsidtbl" | "revtbl" | "filetbl" | "themedata"
                | "colorschememapping" | "datastore" | "xmlnstbl" | "latentstyles" => {
                    self.state.destination = Destination::Skip;
                    return Ok(());
                }
                "pict" | "shp" | "shppict" | "nonshppict" | "object" | "footnote"
                | "annotation" | "listtext" | "pntext" | "header" | "footer" | "headerl"
                | "headerr" | "footerl" | "footerr" => return Err(unsupported(name)),
                _ if starred => {
                    self.state.destination = Destination::Skip;
                    return Ok(());
                }
                _ => (),
            }
        }
        if self.state.destination == Destination::Colors {
            let channel = match name {
                "red" => 0,
                "green" => 1,
                "blue" => 2,
                _ => return Err(unsupported(name)),
            };
            self.rgb[channel] =
                u8::try_from(value.ok_or(Error::Invalid)?).map_err(|_| Error::Invalid)?;
            self.color_set = true;
            return Ok(());
        }
        if self.state.destination == Destination::Fonts {
            if name == "u" { self.unicode(value.ok_or(Error::Invalid)?)?; self.skip = self.state.uc; return Ok(()); }
            if name == "uc" { self.state.uc = usize::try_from(value.ok_or(Error::Invalid)?).ok().filter(|n| *n <= 16).ok_or(Error::Invalid)?; return Ok(()); }
            if name == "f" {
                let n = value.filter(|n| *n >= 0).ok_or(Error::Invalid)?;
                if self.fonts.len() == 256 || self.fonts.iter().any(|f| f.0 == n) {
                    return Err(Error::Limit);
                }
                self.fonts.push((n, 0));
                self.font_names.push((n, String::new()));
                self.state.font = n;
            } else if name == "fcharset" || name == "cpg" {
                let n = value.ok_or(Error::Invalid)?;
                let font = self
                    .fonts
                    .iter_mut()
                    .find(|f| f.0 == self.state.font)
                    .ok_or(Error::Invalid)?;
                font.1 = if name == "cpg" {
                    n
                } else {
                    match n {
                        0 | 1 => 0,
                        2 => -2,
                        161 => 1253,
                        162 => 1254,
                        163 => 1258,
                        177 => 1255,
                        178 => 1256,
                        186 => 1257,
                        204 => 1251,
                        222 => 874,
                        238 => 1250,
                        _ => -1,
                    }
                };
            }
            return Ok(());
        }
        if self.state.destination == Destination::Field {
            return Ok(());
        }
        if matches!(name,"cell"|"nestcell") {
            let depth=if name=="cell"{1}else{self.state.table_depth};
            if name=="nestcell" && depth<2{return Err(Error::Invalid);}
            if !self.current.is_empty() && self.current_depth!=depth{return Err(Error::Invalid);}
            self.state.table_depth=depth;self.paragraph_end()?;self.tables.cell_end(depth)?;return Ok(());
        }
        if self.tables.control(self.state.table_depth,name,value,&self.colors)? {return Ok(());}
        let n = value.unwrap_or(1);
        let required = || value.ok_or(Error::Invalid);
        let mut paragraph_changed = false;
        match name {
            "intbl" => {self.state.table_depth=if toggle(n)?{self.state.table_depth.max(1)}else{0};},
            "itap" => {let depth=usize::try_from(required()?).map_err(|_|Error::Invalid)?;if depth>doc_core::MAX_TABLE_DEPTH{return Err(Error::Limit);}self.state.table_depth=depth;},
            "rtf" => return Err(Error::Invalid),
            "ansi" => (),
            "ansicpg" => {
                self.codepage = required()?;
            }
            "mac" | "pc" | "pca" => return Err(unsupported("character set")),
            "deff" => {
                self.default_font = required()?;
                self.state.font = self.default_font;
            }
            "f" => {
                self.state.font = required()?;
            }
            "uc" => {
                self.state.uc = usize::try_from(required()?)
                    .ok()
                    .filter(|n| *n <= 16)
                    .ok_or(Error::Invalid)?;
            }
            "u" => {
                self.unicode(required()?)?;
                self.skip = self.state.uc;
            }
            "plain" => {
                self.state.text = rtf_text();
                self.state.font = self.default_font;
            }
            "b" => self.state.text.bold = Some(toggle(n)?),
            "i" => self.state.text.italic = Some(toggle(n)?),
            "ul" => self.state.text.underline = Some(toggle(n)?),
            "ulnone" => self.state.text.underline = Some(false),
            "strike" => self.state.text.strike = Some(toggle(n)?),
            "super" | "sub" => self.state.text.positioning = Some(if !toggle(n)? { TextPositioning::Baseline }
                else if name == "super" { TextPositioning::Superscript } else { TextPositioning::Subscript }),
            "nosupersub" => self.state.text.positioning = Some(TextPositioning::Baseline),
            "highlight" => {
                let index = usize::try_from(required()?).map_err(|_| Error::Invalid)?;
                self.state.text.highlight = Some(if index == 0 { Highlight::None } else {
                    match self.colors.get(index).ok_or(Error::Invalid)? {
                        TextColor::Auto => Highlight::None,
                        TextColor::Rgb(rgb) => Highlight::from_rgb(*rgb).ok_or_else(|| unsupported("highlight color outside Word palette"))?,
                    }
                });
            }
            "fs" => {
                self.state.text.size_half_points =
                    Some(u16::try_from(required()?).map_err(|_| Error::Invalid)?);
                self.state.text.validate()?;
            }
            "cf" => {
                let color = usize::try_from(required()?).map_err(|_| Error::Invalid)?;
                self.state.text.color = Some(if color == 0 && self.colors.is_empty() {
                    TextColor::Auto
                } else {
                    *self.colors.get(color).ok_or(Error::Invalid)?
                });
            }
            "tab" => self.text('\t')?,
            "line" => self.text('\n')?,
            "par" => self.paragraph_end()?,
            "emdash" => self.text('—')?,
            "endash" => self.text('–')?,
            "bullet" => self.text('•')?,
            "lquote" => self.text('‘')?,
            "rquote" => self.text('’')?,
            "ldblquote" => self.text('“')?,
            "rdblquote" => self.text('”')?,
            "emspace" => self.text('\u{2003}')?,
            "enspace" => self.text('\u{2002}')?,
            "qmspace" => self.text('\u{2005}')?,
            "pard" => {
                self.state.table_depth=0;
                self.state.tab_alignment = Default::default();
                self.state.tab_leader = Default::default();
                self.state.paragraph = rtf_paragraph();
                self.state.line = 0;
                self.state.multiple = false;
                paragraph_changed = true;
            }
            "tql" | "tqc" | "tqr" | "tqdec" => {
                self.state.tab_alignment = match name { "tqc" => doc_core::TabAlignment::Center, "tqr" => doc_core::TabAlignment::Right, "tqdec" => doc_core::TabAlignment::Decimal, _ => doc_core::TabAlignment::Left };
            }
            "tldot" | "tlhyph" | "tlul" | "tlth" | "tlmdot" => {
                self.state.tab_leader = match name { "tldot" => doc_core::TabLeader::Dot, "tlhyph" => doc_core::TabLeader::Hyphen, "tlul" => doc_core::TabLeader::Underscore, "tlth" => doc_core::TabLeader::Heavy, _ => doc_core::TabLeader::MiddleDot };
            }
            "tx" | "tb" => {
                let position = required()?;
                self.state.paragraph.tabs.set(doc_core::TabStop { position,
                    alignment: if name == "tb" { doc_core::TabAlignment::Bar } else { self.state.tab_alignment },
                    leader: if name == "tb" { doc_core::TabLeader::None } else { self.state.tab_leader } })?;
                self.state.tab_alignment = Default::default();
                self.state.tab_leader = Default::default();
                paragraph_changed = true;
            }
            "ql" | "qc" | "qr" | "qj" => {
                self.state.paragraph.alignment = match name {
                    "qc" => Alignment::Center,
                    "qr" => Alignment::Right,
                    "qj" => Alignment::Justify,
                    _ => Alignment::Left,
                };
                paragraph_changed = true;
            }
            "sb" | "sa" | "li" | "ri" => {
                let value = u16::try_from(required()?).map_err(|_| unsupported(name))?;
                match name {
                    "sb" => self.state.paragraph.before = value,
                    "sa" => self.state.paragraph.after = value,
                    "li" => self.state.paragraph.left = value,
                    _ => self.state.paragraph.right = value,
                };
                paragraph_changed = true;
            }
            "fi" => {
                self.state.paragraph.first_line = required()?;
                paragraph_changed = true;
            }
            "sl" => {
                self.state.line = required()?;
                paragraph_changed = true;
            }
            "slmult" => {
                self.state.multiple = toggle(n)?;
                paragraph_changed = true;
            }
            "pagebb" => {
                self.state.paragraph.page_break_before = toggle(n)?;
                paragraph_changed = true;
            }
            // These do not describe body content. Font family/language and pagination hints
            // deliberately map to the document model's defaults for clipboard interchange.
            "deflang" | "deflangfe" | "adeflang" | "lang" | "langfe" | "langnp" | "langfenp"
            | "noproof" | "viewkind" | "viewscale" | "viewzk" | "deftab" | "paperw" | "paperh"
            | "margl" | "margr" | "margt" | "margb" | "gutter" | "widowctrl" | "nowidctlpar"
            | "widctlpar" | "keep" | "keepn" | "hyphpar" | "hyphauto" | "hyphhotz" | "fet"
            | "ftnbj" | "aenddoc" | "formshade" | "horzdoc" | "jexpand" | "jcompress"
            | "jcompressjexp" | "donotembedsysfont" | "donotembedlingdata" | "trackmoves"
            | "trackformatting" | "pgwsxn" | "pghsxn" | "marglsxn" | "margrsxn" | "margtsxn"
            | "margbsxn" | "sectd" | "sbknone" | "sbkpage" | "sectdefaultcl" | "ltrdoc"
            | "ltrsect" | "ltrpar" | "ltrch" | "loch" | "hich" | "dbch" | "af" | "alang"
            | "afs" | "adeff" | "nouicompat" | "fromtext" | "fet0" | "fet2" | "facingp"
            | "cgrid" | "gridtbl" | "ftnlytwntbl" | "nospaceforul" | "noultrlspc"
            | "allowfieldendsel" | "noxlattoyen" | "dghspace" | "dgvspace" | "dghorigin"
            | "dgvorigin" | "dghshow" | "dgvshow" | "flomajor" | "flominor" | "fdbmajor"
            | "fdbminor" | "fhimajor" | "fhiminor" | "fbimajor" | "fbiminor" => (),
            _ if name.starts_with("rsid") => (),
            "s" | "cs" | "ds" | "cb" | "chcbpat" | "shading" | "expnd"
            | "expndtw" | "charscalex" | "kerning" | "outl" | "shad" | "v" | "deleted"
            | "revised" | "up" | "dn"
                if n == 0 || name == "charscalex" && n == 100 =>
            {
                ()
            }
            _ => return Err(unsupported(name)),
        }
        // sl and slmult commonly occur as a pair: validate at text/paragraph boundaries,
        // not between those two controls. Already collected text uses the final properties.
        if paragraph_changed {
            self.paragraph = self.state.paragraph;
            self.paragraph_line = self.state.line;
            self.paragraph_multiple = self.state.multiple;
            if self.state.line > 0 {
                self.paragraph.line = self.state.line.min(u16::MAX as i32) as u16;
            }
        }
        Ok(())
    }
}
fn toggle(n: i32) -> Result<bool, Error> {
    match n {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Error::Invalid),
    }
}
fn hex(n: u8) -> Result<u8, Error> {
    match n {
        b'0'..=b'9' => Ok(n - b'0'),
        b'a'..=b'f' => Ok(n - b'a' + 10),
        b'A'..=b'F' => Ok(n - b'A' + 10),
        _ => Err(Error::Invalid),
    }
}
fn supported_codepage(cp: i32) -> bool {
    matches!(cp, 874 | 1250..=1258)
}
fn codepage_character(n: u8, cp: i32) -> Option<char> {
    if n < 128 {
        return Some(char::from(n));
    }
    let index = match cp {
        874 => 0,
        1250..=1258 => (cp - 1249) as usize,
        _ => return None,
    };
    let value = WINDOWS_PAGES[index][usize::from(n - 128)];
    if value == 0 {
        None
    } else {
        char::from_u32(u32::from(value))
    }
}
// Unicode mappings for Windows single-byte code pages: 874, 1250 through 1258.
// Zero entries are undefined bytes and are rejected, never replaced silently.
const WINDOWS_PAGES: [[u16; 128]; 10] = [
    [
        // Windows-874
        0x20ac, 0x0000, 0x0000, 0x0000, 0x0000, 0x2026, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000,
        0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022,
        0x2013, 0x2014, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x00a0,
        0x0e01, 0x0e02, 0x0e03, 0x0e04, 0x0e05, 0x0e06, 0x0e07, 0x0e08, 0x0e09, 0x0e0a, 0x0e0b,
        0x0e0c, 0x0e0d, 0x0e0e, 0x0e0f, 0x0e10, 0x0e11, 0x0e12, 0x0e13, 0x0e14, 0x0e15, 0x0e16,
        0x0e17, 0x0e18, 0x0e19, 0x0e1a, 0x0e1b, 0x0e1c, 0x0e1d, 0x0e1e, 0x0e1f, 0x0e20, 0x0e21,
        0x0e22, 0x0e23, 0x0e24, 0x0e25, 0x0e26, 0x0e27, 0x0e28, 0x0e29, 0x0e2a, 0x0e2b, 0x0e2c,
        0x0e2d, 0x0e2e, 0x0e2f, 0x0e30, 0x0e31, 0x0e32, 0x0e33, 0x0e34, 0x0e35, 0x0e36, 0x0e37,
        0x0e38, 0x0e39, 0x0e3a, 0x0000, 0x0000, 0x0000, 0x0000, 0x0e3f, 0x0e40, 0x0e41, 0x0e42,
        0x0e43, 0x0e44, 0x0e45, 0x0e46, 0x0e47, 0x0e48, 0x0e49, 0x0e4a, 0x0e4b, 0x0e4c, 0x0e4d,
        0x0e4e, 0x0e4f, 0x0e50, 0x0e51, 0x0e52, 0x0e53, 0x0e54, 0x0e55, 0x0e56, 0x0e57, 0x0e58,
        0x0e59, 0x0e5a, 0x0e5b, 0x0000, 0x0000, 0x0000, 0x0000,
    ],
    [
        // Windows-1250
        0x20ac, 0x0000, 0x201a, 0x0000, 0x201e, 0x2026, 0x2020, 0x2021, 0x0000, 0x2030, 0x0160,
        0x2039, 0x015a, 0x0164, 0x017d, 0x0179, 0x0000, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022,
        0x2013, 0x2014, 0x0000, 0x2122, 0x0161, 0x203a, 0x015b, 0x0165, 0x017e, 0x017a, 0x00a0,
        0x02c7, 0x02d8, 0x0141, 0x00a4, 0x0104, 0x00a6, 0x00a7, 0x00a8, 0x00a9, 0x015e, 0x00ab,
        0x00ac, 0x00ad, 0x00ae, 0x017b, 0x00b0, 0x00b1, 0x02db, 0x0142, 0x00b4, 0x00b5, 0x00b6,
        0x00b7, 0x00b8, 0x0105, 0x015f, 0x00bb, 0x013d, 0x02dd, 0x013e, 0x017c, 0x0154, 0x00c1,
        0x00c2, 0x0102, 0x00c4, 0x0139, 0x0106, 0x00c7, 0x010c, 0x00c9, 0x0118, 0x00cb, 0x011a,
        0x00cd, 0x00ce, 0x010e, 0x0110, 0x0143, 0x0147, 0x00d3, 0x00d4, 0x0150, 0x00d6, 0x00d7,
        0x0158, 0x016e, 0x00da, 0x0170, 0x00dc, 0x00dd, 0x0162, 0x00df, 0x0155, 0x00e1, 0x00e2,
        0x0103, 0x00e4, 0x013a, 0x0107, 0x00e7, 0x010d, 0x00e9, 0x0119, 0x00eb, 0x011b, 0x00ed,
        0x00ee, 0x010f, 0x0111, 0x0144, 0x0148, 0x00f3, 0x00f4, 0x0151, 0x00f6, 0x00f7, 0x0159,
        0x016f, 0x00fa, 0x0171, 0x00fc, 0x00fd, 0x0163, 0x02d9,
    ],
    [
        // Windows-1251
        0x0402, 0x0403, 0x201a, 0x0453, 0x201e, 0x2026, 0x2020, 0x2021, 0x20ac, 0x2030, 0x0409,
        0x2039, 0x040a, 0x040c, 0x040b, 0x040f, 0x0452, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022,
        0x2013, 0x2014, 0x0000, 0x2122, 0x0459, 0x203a, 0x045a, 0x045c, 0x045b, 0x045f, 0x00a0,
        0x040e, 0x045e, 0x0408, 0x00a4, 0x0490, 0x00a6, 0x00a7, 0x0401, 0x00a9, 0x0404, 0x00ab,
        0x00ac, 0x00ad, 0x00ae, 0x0407, 0x00b0, 0x00b1, 0x0406, 0x0456, 0x0491, 0x00b5, 0x00b6,
        0x00b7, 0x0451, 0x2116, 0x0454, 0x00bb, 0x0458, 0x0405, 0x0455, 0x0457, 0x0410, 0x0411,
        0x0412, 0x0413, 0x0414, 0x0415, 0x0416, 0x0417, 0x0418, 0x0419, 0x041a, 0x041b, 0x041c,
        0x041d, 0x041e, 0x041f, 0x0420, 0x0421, 0x0422, 0x0423, 0x0424, 0x0425, 0x0426, 0x0427,
        0x0428, 0x0429, 0x042a, 0x042b, 0x042c, 0x042d, 0x042e, 0x042f, 0x0430, 0x0431, 0x0432,
        0x0433, 0x0434, 0x0435, 0x0436, 0x0437, 0x0438, 0x0439, 0x043a, 0x043b, 0x043c, 0x043d,
        0x043e, 0x043f, 0x0440, 0x0441, 0x0442, 0x0443, 0x0444, 0x0445, 0x0446, 0x0447, 0x0448,
        0x0449, 0x044a, 0x044b, 0x044c, 0x044d, 0x044e, 0x044f,
    ],
    [
        // Windows-1252
        0x20ac, 0x0000, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160,
        0x2039, 0x0152, 0x0000, 0x017d, 0x0000, 0x0000, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022,
        0x2013, 0x2014, 0x02dc, 0x2122, 0x0161, 0x203a, 0x0153, 0x0000, 0x017e, 0x0178, 0x00a0,
        0x00a1, 0x00a2, 0x00a3, 0x00a4, 0x00a5, 0x00a6, 0x00a7, 0x00a8, 0x00a9, 0x00aa, 0x00ab,
        0x00ac, 0x00ad, 0x00ae, 0x00af, 0x00b0, 0x00b1, 0x00b2, 0x00b3, 0x00b4, 0x00b5, 0x00b6,
        0x00b7, 0x00b8, 0x00b9, 0x00ba, 0x00bb, 0x00bc, 0x00bd, 0x00be, 0x00bf, 0x00c0, 0x00c1,
        0x00c2, 0x00c3, 0x00c4, 0x00c5, 0x00c6, 0x00c7, 0x00c8, 0x00c9, 0x00ca, 0x00cb, 0x00cc,
        0x00cd, 0x00ce, 0x00cf, 0x00d0, 0x00d1, 0x00d2, 0x00d3, 0x00d4, 0x00d5, 0x00d6, 0x00d7,
        0x00d8, 0x00d9, 0x00da, 0x00db, 0x00dc, 0x00dd, 0x00de, 0x00df, 0x00e0, 0x00e1, 0x00e2,
        0x00e3, 0x00e4, 0x00e5, 0x00e6, 0x00e7, 0x00e8, 0x00e9, 0x00ea, 0x00eb, 0x00ec, 0x00ed,
        0x00ee, 0x00ef, 0x00f0, 0x00f1, 0x00f2, 0x00f3, 0x00f4, 0x00f5, 0x00f6, 0x00f7, 0x00f8,
        0x00f9, 0x00fa, 0x00fb, 0x00fc, 0x00fd, 0x00fe, 0x00ff,
    ],
    [
        // Windows-1253
        0x20ac, 0x0000, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x0000, 0x2030, 0x0000,
        0x2039, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022,
        0x2013, 0x2014, 0x0000, 0x2122, 0x0000, 0x203a, 0x0000, 0x0000, 0x0000, 0x0000, 0x00a0,
        0x0385, 0x0386, 0x00a3, 0x00a4, 0x00a5, 0x00a6, 0x00a7, 0x00a8, 0x00a9, 0x0000, 0x00ab,
        0x00ac, 0x00ad, 0x00ae, 0x2015, 0x00b0, 0x00b1, 0x00b2, 0x00b3, 0x0384, 0x00b5, 0x00b6,
        0x00b7, 0x0388, 0x0389, 0x038a, 0x00bb, 0x038c, 0x00bd, 0x038e, 0x038f, 0x0390, 0x0391,
        0x0392, 0x0393, 0x0394, 0x0395, 0x0396, 0x0397, 0x0398, 0x0399, 0x039a, 0x039b, 0x039c,
        0x039d, 0x039e, 0x039f, 0x03a0, 0x03a1, 0x0000, 0x03a3, 0x03a4, 0x03a5, 0x03a6, 0x03a7,
        0x03a8, 0x03a9, 0x03aa, 0x03ab, 0x03ac, 0x03ad, 0x03ae, 0x03af, 0x03b0, 0x03b1, 0x03b2,
        0x03b3, 0x03b4, 0x03b5, 0x03b6, 0x03b7, 0x03b8, 0x03b9, 0x03ba, 0x03bb, 0x03bc, 0x03bd,
        0x03be, 0x03bf, 0x03c0, 0x03c1, 0x03c2, 0x03c3, 0x03c4, 0x03c5, 0x03c6, 0x03c7, 0x03c8,
        0x03c9, 0x03ca, 0x03cb, 0x03cc, 0x03cd, 0x03ce, 0x0000,
    ],
    [
        // Windows-1254
        0x20ac, 0x0000, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0160,
        0x2039, 0x0152, 0x0000, 0x0000, 0x0000, 0x0000, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022,
        0x2013, 0x2014, 0x02dc, 0x2122, 0x0161, 0x203a, 0x0153, 0x0000, 0x0000, 0x0178, 0x00a0,
        0x00a1, 0x00a2, 0x00a3, 0x00a4, 0x00a5, 0x00a6, 0x00a7, 0x00a8, 0x00a9, 0x00aa, 0x00ab,
        0x00ac, 0x00ad, 0x00ae, 0x00af, 0x00b0, 0x00b1, 0x00b2, 0x00b3, 0x00b4, 0x00b5, 0x00b6,
        0x00b7, 0x00b8, 0x00b9, 0x00ba, 0x00bb, 0x00bc, 0x00bd, 0x00be, 0x00bf, 0x00c0, 0x00c1,
        0x00c2, 0x00c3, 0x00c4, 0x00c5, 0x00c6, 0x00c7, 0x00c8, 0x00c9, 0x00ca, 0x00cb, 0x00cc,
        0x00cd, 0x00ce, 0x00cf, 0x011e, 0x00d1, 0x00d2, 0x00d3, 0x00d4, 0x00d5, 0x00d6, 0x00d7,
        0x00d8, 0x00d9, 0x00da, 0x00db, 0x00dc, 0x0130, 0x015e, 0x00df, 0x00e0, 0x00e1, 0x00e2,
        0x00e3, 0x00e4, 0x00e5, 0x00e6, 0x00e7, 0x00e8, 0x00e9, 0x00ea, 0x00eb, 0x00ec, 0x00ed,
        0x00ee, 0x00ef, 0x011f, 0x00f1, 0x00f2, 0x00f3, 0x00f4, 0x00f5, 0x00f6, 0x00f7, 0x00f8,
        0x00f9, 0x00fa, 0x00fb, 0x00fc, 0x0131, 0x015f, 0x00ff,
    ],
    [
        // Windows-1255
        0x20ac, 0x0000, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0000,
        0x2039, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022,
        0x2013, 0x2014, 0x02dc, 0x2122, 0x0000, 0x203a, 0x0000, 0x0000, 0x0000, 0x0000, 0x00a0,
        0x00a1, 0x00a2, 0x00a3, 0x20aa, 0x00a5, 0x00a6, 0x00a7, 0x00a8, 0x00a9, 0x00d7, 0x00ab,
        0x00ac, 0x00ad, 0x00ae, 0x00af, 0x00b0, 0x00b1, 0x00b2, 0x00b3, 0x00b4, 0x00b5, 0x00b6,
        0x00b7, 0x00b8, 0x00b9, 0x00f7, 0x00bb, 0x00bc, 0x00bd, 0x00be, 0x00bf, 0x05b0, 0x05b1,
        0x05b2, 0x05b3, 0x05b4, 0x05b5, 0x05b6, 0x05b7, 0x05b8, 0x05b9, 0x0000, 0x05bb, 0x05bc,
        0x05bd, 0x05be, 0x05bf, 0x05c0, 0x05c1, 0x05c2, 0x05c3, 0x05f0, 0x05f1, 0x05f2, 0x05f3,
        0x05f4, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x05d0, 0x05d1, 0x05d2,
        0x05d3, 0x05d4, 0x05d5, 0x05d6, 0x05d7, 0x05d8, 0x05d9, 0x05da, 0x05db, 0x05dc, 0x05dd,
        0x05de, 0x05df, 0x05e0, 0x05e1, 0x05e2, 0x05e3, 0x05e4, 0x05e5, 0x05e6, 0x05e7, 0x05e8,
        0x05e9, 0x05ea, 0x0000, 0x0000, 0x200e, 0x200f, 0x0000,
    ],
    [
        // Windows-1256
        0x20ac, 0x067e, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0679,
        0x2039, 0x0152, 0x0686, 0x0698, 0x0688, 0x06af, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022,
        0x2013, 0x2014, 0x06a9, 0x2122, 0x0691, 0x203a, 0x0153, 0x200c, 0x200d, 0x06ba, 0x00a0,
        0x060c, 0x00a2, 0x00a3, 0x00a4, 0x00a5, 0x00a6, 0x00a7, 0x00a8, 0x00a9, 0x06be, 0x00ab,
        0x00ac, 0x00ad, 0x00ae, 0x00af, 0x00b0, 0x00b1, 0x00b2, 0x00b3, 0x00b4, 0x00b5, 0x00b6,
        0x00b7, 0x00b8, 0x00b9, 0x061b, 0x00bb, 0x00bc, 0x00bd, 0x00be, 0x061f, 0x06c1, 0x0621,
        0x0622, 0x0623, 0x0624, 0x0625, 0x0626, 0x0627, 0x0628, 0x0629, 0x062a, 0x062b, 0x062c,
        0x062d, 0x062e, 0x062f, 0x0630, 0x0631, 0x0632, 0x0633, 0x0634, 0x0635, 0x0636, 0x00d7,
        0x0637, 0x0638, 0x0639, 0x063a, 0x0640, 0x0641, 0x0642, 0x0643, 0x00e0, 0x0644, 0x00e2,
        0x0645, 0x0646, 0x0647, 0x0648, 0x00e7, 0x00e8, 0x00e9, 0x00ea, 0x00eb, 0x0649, 0x064a,
        0x00ee, 0x00ef, 0x064b, 0x064c, 0x064d, 0x064e, 0x00f4, 0x064f, 0x0650, 0x00f7, 0x0651,
        0x00f9, 0x0652, 0x00fb, 0x00fc, 0x200e, 0x200f, 0x06d2,
    ],
    [
        // Windows-1257
        0x20ac, 0x0000, 0x201a, 0x0000, 0x201e, 0x2026, 0x2020, 0x2021, 0x0000, 0x2030, 0x0000,
        0x2039, 0x0000, 0x00a8, 0x02c7, 0x00b8, 0x0000, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022,
        0x2013, 0x2014, 0x0000, 0x2122, 0x0000, 0x203a, 0x0000, 0x00af, 0x02db, 0x0000, 0x00a0,
        0x0000, 0x00a2, 0x00a3, 0x00a4, 0x0000, 0x00a6, 0x00a7, 0x00d8, 0x00a9, 0x0156, 0x00ab,
        0x00ac, 0x00ad, 0x00ae, 0x00c6, 0x00b0, 0x00b1, 0x00b2, 0x00b3, 0x00b4, 0x00b5, 0x00b6,
        0x00b7, 0x00f8, 0x00b9, 0x0157, 0x00bb, 0x00bc, 0x00bd, 0x00be, 0x00e6, 0x0104, 0x012e,
        0x0100, 0x0106, 0x00c4, 0x00c5, 0x0118, 0x0112, 0x010c, 0x00c9, 0x0179, 0x0116, 0x0122,
        0x0136, 0x012a, 0x013b, 0x0160, 0x0143, 0x0145, 0x00d3, 0x014c, 0x00d5, 0x00d6, 0x00d7,
        0x0172, 0x0141, 0x015a, 0x016a, 0x00dc, 0x017b, 0x017d, 0x00df, 0x0105, 0x012f, 0x0101,
        0x0107, 0x00e4, 0x00e5, 0x0119, 0x0113, 0x010d, 0x00e9, 0x017a, 0x0117, 0x0123, 0x0137,
        0x012b, 0x013c, 0x0161, 0x0144, 0x0146, 0x00f3, 0x014d, 0x00f5, 0x00f6, 0x00f7, 0x0173,
        0x0142, 0x015b, 0x016b, 0x00fc, 0x017c, 0x017e, 0x02d9,
    ],
    [
        // Windows-1258
        0x20ac, 0x0000, 0x201a, 0x0192, 0x201e, 0x2026, 0x2020, 0x2021, 0x02c6, 0x2030, 0x0000,
        0x2039, 0x0152, 0x0000, 0x0000, 0x0000, 0x0000, 0x2018, 0x2019, 0x201c, 0x201d, 0x2022,
        0x2013, 0x2014, 0x02dc, 0x2122, 0x0000, 0x203a, 0x0153, 0x0000, 0x0000, 0x0178, 0x00a0,
        0x00a1, 0x00a2, 0x00a3, 0x00a4, 0x00a5, 0x00a6, 0x00a7, 0x00a8, 0x00a9, 0x00aa, 0x00ab,
        0x00ac, 0x00ad, 0x00ae, 0x00af, 0x00b0, 0x00b1, 0x00b2, 0x00b3, 0x00b4, 0x00b5, 0x00b6,
        0x00b7, 0x00b8, 0x00b9, 0x00ba, 0x00bb, 0x00bc, 0x00bd, 0x00be, 0x00bf, 0x00c0, 0x00c1,
        0x00c2, 0x0102, 0x00c4, 0x00c5, 0x00c6, 0x00c7, 0x00c8, 0x00c9, 0x00ca, 0x00cb, 0x0300,
        0x00cd, 0x00ce, 0x00cf, 0x0110, 0x00d1, 0x0309, 0x00d3, 0x00d4, 0x01a0, 0x00d6, 0x00d7,
        0x00d8, 0x00d9, 0x00da, 0x00db, 0x00dc, 0x01af, 0x0303, 0x00df, 0x00e0, 0x00e1, 0x00e2,
        0x0103, 0x00e4, 0x00e5, 0x00e6, 0x00e7, 0x00e8, 0x00e9, 0x00ea, 0x00eb, 0x0301, 0x00ed,
        0x00ee, 0x00ef, 0x0111, 0x00f1, 0x0323, 0x00f3, 0x00f4, 0x01a1, 0x00f6, 0x00f7, 0x00f8,
        0x00f9, 0x00fa, 0x00fb, 0x00fc, 0x01b0, 0x20ab, 0x00ff,
    ],
];

/// Decode the bounded RTF clipboard profile into validated, directly formatted paragraphs.
/// Accepts group-scoped formatting, Windows single-byte text (874/1250–1258), Unicode/surrogate escapes, colors,
/// relative line spacing, twip paragraph metrics, font family names and cached field results.
/// Language, style identities and metadata map to model defaults. Unknown ignorable destinations are
/// skipped; unsupported visible content (including pictures, lists and objects),
/// unsupported encodings or typography and malformed input are rejected atomically.
/// Only direct formatting is read: implicit Normal-style/default-character properties in
/// metadata are not resolved and fall back to RTF defaults (12pt, automatic color). Other
/// named-style references are rejected. This profile is intended for resolved rich selections.
/// Rectangular and nested table grids, row/column merges, headers, fills and cell borders are read.
/// Limits: 16 MiB input, 2 MiB expanded text, 64 groups, 1024 colors, 256 fonts, 4M tokens.
pub fn clipboard_rtf_decode(bytes: &[u8]) -> Result<Document, Error> {
    if bytes.len() > MAX_BYTES {
        return Err(Error::Limit);
    }
    Reader::new(bytes).run()
}
