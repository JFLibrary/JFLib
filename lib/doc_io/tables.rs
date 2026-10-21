//! Standard rectangular WordprocessingML tables; application identities use a separate custom XML part.
use super::*;
use doc_core::{CellAlignment, Table, TableCell, TableRow};
pub(super) const PATH: &str = "customXml/jfTables.xml";
const NS: &str = "urn:jfoffice:table-identities:1";
pub(super) fn metadata(name: &str, base: String, present: bool) -> String {
    if present && name == "word/_rels/document.xml.rels" {
        base.replacen("</Relationships>","<Relationship Id=\"rId6\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml\" Target=\"../customXml/jfTables.xml\"/></Relationships>",1)
    } else {
        base
    }
}
pub(super) fn identities(d: &Document) -> String {
    let mut s = format!("<tables xmlns=\"{NS}\">");
    for t in d.ordered_tables() {
            s.push_str(&format!("<table id=\"{}\">", t.id));
            for r in &t.rows {
                s.push_str("<row>");
                for c in &r.cells {
                    s.push_str(&format!("<cell id=\"{}\"/>", c.id));
                }
                s.push_str("</row>");
            }
            s.push_str("</table>");
    }
    s.push_str("</tables>");
    s
}
pub(super) struct Identities {
    table: Id,
    cells: Vec<Vec<Id>>,
}
fn reserve(n: &Node, used: &mut BTreeSet<Id>) -> Result<Id, Error> {
    if !n.text.is_empty() || n.attrs.len() != 1 {
        return Err(Error::Invalid);
    }
    let id = n
        .attr("", "id")
        .ok_or(Error::Invalid)?
        .parse::<Id>()
        .map_err(|_| Error::Invalid)?;
    if id == 0 || id >= 0x7fff_ffff || !used.insert(id) {
        return Err(Error::Invalid);
    }
    Ok(id)
}
pub(super) fn read_identities(
    entries: &[Entry],
    used: &mut BTreeSet<Id>,
) -> Result<Vec<Identities>, Error> {
    let Some(part) = entries.iter().find(|p| p.name == PATH) else {
        return Ok(Vec::new());
    };
    let root = parse(&part.data)?;
    container(&root, NS, "tables")?;
    if root.children.len() > 128 {
        return Err(Error::Limit);
    }
    let mut out = Vec::new();
    for t in &root.children {
        if !t.is(NS, "table") || t.children.is_empty() || t.children.len() > 256 {
            return Err(Error::Invalid);
        }
        let table = reserve(t, used)?;
        let mut cells = Vec::new();
        for r in &t.children {
            container(r, NS, "row")?;
            if r.children.is_empty() || r.children.len() > 32 {
                return Err(Error::Invalid);
            }
            let mut row = Vec::new();
            for c in &r.children {
                if !c.is(NS, "cell") || !c.children.is_empty() {
                    return Err(Error::Invalid);
                }
                row.push(reserve(c, used)?);
            }
            cells.push(row);
        }
        out.push(Identities { table, cells });
    }
    Ok(out)
}
pub(super) fn reserve_paragraphs(n: &Node, used: &mut BTreeSet<Id>) -> Result<(), Error> {
    if n.is(W, "p") {
        let id = paragraph_id(n, false)?;
        if id != 0 && !used.insert(id) {
            return Err(Error::Invalid);
        }
    } else {
        for c in &n.children {
            reserve_paragraphs(c, used)?;
        }
    }
    Ok(())
}
fn valign(a: CellAlignment) -> &'static str {
    match a {
        CellAlignment::Top => "top",
        CellAlignment::Center => "center",
        CellAlignment::Bottom => "bottom",
    }
}
pub(super) fn write(d: &Document, t: &Table) -> Result<String, Error> {
    let mut s = String::from("<w:tbl><w:tblPr>");
    if let Some(id)=&t.style {s.push_str(&format!("<w:tblStyle w:val=\"{}\"/>",crate::esc(id)?));}
    if let Some(p)=t.position{s.push_str(&crate::table_position::write(p));}
    s.push_str(&format!("<w:tblW w:w=\"{}\" w:type=\"dxa\"/>",t.width()));
    if t.border_override {s.push_str(&crate::table_styles::border(t.border_width,t.border_color));}
    s.push_str("<w:tblLayout w:type=\"fixed\"/><w:tblCellMar>");
    for side in ["top", "left", "bottom", "right"] {
        s.push_str(&format!("<w:{side} w:w=\"{}\" w:type=\"dxa\"/>", t.padding));
    }
    s.push_str("</w:tblCellMar>");
    if t.style.is_some() || t.look != doc_core::TableLook::default() {s.push_str(&crate::table_styles::look(t.look));}
    s.push_str("</w:tblPr><w:tblGrid>");
    for width in &t.columns {
        s.push_str(&format!("<w:gridCol w:w=\"{width}\"/>"));
    }
    s.push_str("</w:tblGrid>");
    for (r, row) in t.rows.iter().enumerate() {
        s.push_str("<w:tr>");
        if row.min_height>0 || row.cant_split || row.grid_before>0 || row.grid_after>0 || r<usize::from(t.header_rows) {
            s.push_str("<w:trPr>");
            if row.grid_before>0{s.push_str(&format!("<w:gridBefore w:val=\"{}\"/>",row.grid_before));}
            if row.grid_after>0{s.push_str(&format!("<w:gridAfter w:val=\"{}\"/>",row.grid_after));}
            if row.cant_split {s.push_str("<w:cantSplit/>");}
            if row.min_height>0 {s.push_str(&format!("<w:trHeight w:val=\"{}\" w:hRule=\"{}\"/>",row.min_height,if row.height_rule==doc_core::RowHeightRule::Exact {"exact"}else{"atLeast"}));}
            if r<usize::from(t.header_rows) {s.push_str("<w:tblHeader/>");}
            s.push_str("</w:trPr>");
        }
        for i in 0..t.columns.len() {
            if t.omitted(r,i){continue;}
            let (ar, ac) = t.origin(r, i).ok_or(Error::Invalid)?;
            if ac != i {
                continue;
            }
            let c = &t.rows[ar].cells[ac];
            s.push_str(&format!(
                "<w:tc><w:tcPr><w:tcW w:w=\"{}\" w:type=\"dxa\"/>",
                t.cell_width(r, i).ok_or(Error::Invalid)?
            ));
            if c.column_span > 1 {
                s.push_str(&format!("<w:gridSpan w:val=\"{}\"/>", c.column_span));
            }
            if c.row_span > 1 {
                s.push_str(if ar == r {
                    "<w:vMerge w:val=\"restart\"/>"
                } else {
                    "<w:vMerge/>"
                });
            }
            s.push_str(&crate::cell_borders::write(&c.borders));
            if let Some(rgb) = c.fill {
                s.push_str(&format!(
                    "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{rgb:06X}\"/>"
                ));
            }
            s.push_str(&format!(
                "<w:vAlign w:val=\"{}\"/></w:tcPr>",
                valign(c.alignment)
            ));
            if ar != r {
                s.push_str("<w:p/>");
            }
            for p in c.paragraphs.iter().filter(|_| ar == r) {
                for child in d.tables.iter().filter(|t| t.before == p.id) { s.push_str(&write(d, child)?); }
                s.push_str(&write_paragraph(d, p, None)?);
            }
            s.push_str("</w:tc>");
        }
        s.push_str("</w:tr>");
    }
    s.push_str("</w:tbl>");
    Ok(s)
}
fn attrs(n: &Node, names: &[&str]) -> Result<(), Error> {
    if !n.children.is_empty()
        || !n.text.is_empty()
        || n.attrs.len() != names.len()
        || n.attrs
            .iter()
            .any(|(k, _)| k.ns != W || !names.contains(&k.local.as_str()))
    {
        return Err(unsupported("Tabelleneigenschaft"));
    }
    Ok(())
}
fn num(n: &Node, a: &str) -> Result<u32, Error> {
    n.attr(W, a)
        .ok_or(Error::Invalid)?
        .parse()
        .map_err(|_| Error::Invalid)
}
fn color(n: &Node, a: &str) -> Result<u32, Error> {
    let s = n.attr(W, a).ok_or(Error::Invalid)?;
    if s.len() != 6 {
        return Err(Error::Invalid);
    }
    u32::from_str_radix(s, 16).map_err(|_| Error::Invalid)
}
fn width(n: &Node) -> Result<u32, Error> {
    attrs(n, &["w", "type"])?;
    if n.attr(W, "type") != Some("dxa") {
        return Err(unsupported("relative Tabellenbreite"));
    }
    num(n, "w")
}
pub(super) fn read(
    n: &Node,
    identities: &[Identities],
    cursor: &mut usize,
    nested: &mut Vec<Table>,
    depth: usize,
    mapping: &[notes::Mapping],
    used: &mut BTreeSet<Id>,
    next: &mut Id,
) -> Result<Table, Error> {
    if depth > doc_core::MAX_TABLE_DEPTH || *cursor >= 128 { return Err(Error::Limit); }
    let ids = identities.get(*cursor); *cursor += 1;
    container(n, W, "tbl")?;
    if n.children.len() < 3 || !n.children[0].is(W, "tblPr") || !n.children[1].is(W, "tblGrid") {
        return Err(unsupported("Tabellenaufbau"));
    }
    let mut t = Table {
        id: if let Some(i) = ids {
            i.table
        } else {
            notes::allocate(used, next)?
        },
        before: 0,
        position: None,
        header_rows:0,style:None,look:doc_core::TableLook::default(),border_override:true,
        columns: Vec::new(),
        rows: Vec::new(),
        padding: 90,
        border_width: 4,
        border_color: 0x808080,
    };
    let grid = &n.children[1];
    container(grid, W, "tblGrid")?;
    for c in &grid.children {
        if !c.is(W, "gridCol") {
            return Err(unsupported("Tabellenraster"));
        }
        attrs(c, &["w"])?;
        let w = num(c, "w")?;
        if w > 31680 {
            return Err(Error::Invalid);
        }
        t.columns.push(w);
    }
    if t.columns.is_empty() || t.columns.len() > 32 {
        return Err(Error::Limit);
    }
    let pr = &n.children[0];
    container(pr, W, "tblPr")?;
    let mut seen = BTreeSet::new();
    let mut overlap=None;
    for p in &pr.children {
        if p.name.ns != W || !seen.insert(&p.name.local) {
            return Err(Error::Invalid);
        }
        match p.name.local.as_str() {
            "tblpPr"=>t.position=Some(crate::table_position::read(p)?),
            "tblOverlap"=>{attrs(p,&["val"])?;overlap=Some(match p.attr(W,"val"){Some("overlap")=>true,Some("never")=>false,_=>return Err(Error::Invalid)});},
            "tblStyle" => {attrs(p,&["val"])?;t.style=Some(String::from(p.attr(W,"val").ok_or(Error::Invalid)?));},
            "tblLook" => t.look=crate::table_styles::read_look(p)?,
            "tblW" => {
                if width(p)? != t.width() {
                    return Err(unsupported("abweichende Tabellenbreite"));
                }
            }
            "tblLayout" => {
                attrs(p, &["type"])?;
                if p.attr(W, "type") != Some("fixed") {
                    return Err(unsupported("AutoFit"));
                }
            }
            "tblCellMar" => {
                container(p, W, "tblCellMar")?;
                let mut margins = BTreeSet::new();
                let mut v = None;
                for c in &p.children {
                    if c.name.ns != W
                        || !["top", "left", "bottom", "right"].contains(&c.name.local.as_str())
                        || !margins.insert(&c.name.local)
                    {
                        return Err(Error::Invalid);
                    }
                    let w = width(c)?;
                    if v.is_some_and(|v| v != w) {
                        return Err(unsupported("unterschiedliche Zellränder"));
                    }
                    v = Some(w);
                }
                if margins.len() != 4 {
                    return Err(Error::Invalid);
                }
                t.padding = u16::try_from(v.unwrap()).map_err(|_| Error::Invalid)?;
            }
            "tblBorders" => {
                container(p, W, "tblBorders")?;
                let mut edges = BTreeSet::new();
                let mut v = None;
                for c in &p.children {
                    if c.name.ns != W
                        || !["top", "left", "bottom", "right", "insideH", "insideV"]
                            .contains(&c.name.local.as_str())
                        || !edges.insert(&c.name.local)
                    {
                        return Err(Error::Invalid);
                    }
                    attrs(c, &["val", "sz", "color"])?;
                    let w = num(c, "sz")?;
                    if !matches!(c.attr(W, "val"), Some("single"))
                        && !(c.attr(W, "val") == Some("nil") && w == 0)
                    {
                        return Err(unsupported("Tabellenrahmen"));
                    }
                    let pair = (
                        u16::try_from(w).map_err(|_| Error::Invalid)?,
                        color(c, "color")?,
                    );
                    if v.is_some_and(|v| v != pair) {
                        return Err(unsupported("unterschiedliche Rahmen"));
                    }
                    v = Some(pair);
                }
                if edges.len() != 6 {
                    return Err(Error::Invalid);
                }
                let (w, c) = v.unwrap();
                t.border_width = w;
                t.border_color = c;
            }
            _ => return Err(unsupported(&p.name.local)),
        }
    }
    t.border_override=t.style.is_none() || seen.contains(&String::from("tblBorders"));
    if let Some(allow)=overlap {t.position.as_mut().ok_or_else(||unsupported("overlap on inline table"))?.overlap=allow;}
    // Each continuation must refer to the immediately preceding row and span the same columns.
    let nc = t.columns.len();
    let mut vertical: Vec<Option<(usize, usize)>> = alloc::vec![None;nc];
    for (r, row) in n.children[2..].iter().enumerate() {
        container(row, W, "tr")?;
        if r >= 256 {
            return Err(Error::Limit);
        }
        let mut out = TableRow {
            cells: Vec::new(),
            min_height: 0,
            height_rule: doc_core::RowHeightRule::AtLeast,
            cant_split: false,
            grid_before:0,grid_after:0,
        };
        for col in 0..nc {
            let id = if let Some(i) = ids {
                *i.cells
                    .get(r)
                    .and_then(|r| r.get(col))
                    .ok_or(Error::Invalid)?
            } else {
                notes::allocate(used, next)?
            };
            out.cells.push(TableCell {
                id,
                row_span: 0,
                column_span: 0,
                paragraphs: Vec::new(),
                fill: None,
                alignment: CellAlignment::Top,
                borders: [None;4],
            });
        }
        let mut children = row.children.as_slice();
        if children.first().is_some_and(|n| n.is(W, "trPr")) {
            let pr = &children[0];
            container(pr, W, "trPr")?;
            let mut seen=BTreeSet::new();
            for h in &pr.children {
                if h.name.ns!=W || !seen.insert(&h.name.local){return Err(Error::Invalid);}
                match h.name.local.as_str() {
                    "cantSplit"=>{attrs(h,if h.attrs.is_empty(){&[]}else{&["val"]})?;out.cant_split=match h.attr(W,"val"){None|Some("1"|"true"|"on")=>true,Some("0"|"false"|"off")=>false,_=>return Err(Error::Invalid)};},
                    "gridBefore"=>{attrs(h,&["val"])?;out.grid_before=u16::try_from(num(h,"val")?).map_err(|_|Error::Invalid)?;},
                    "gridAfter"=>{attrs(h,&["val"])?;out.grid_after=u16::try_from(num(h,"val")?).map_err(|_|Error::Invalid)?;},
                    "trHeight"=>{
                        let mut names=Vec::new();
                        for name in ["val","hRule"] {if h.attr(W,name).is_some(){names.push(name);}}
                        attrs(h,&names)?;
                        let height=if h.attr(W,"val").is_some(){num(h,"val")?}else{0};
                        if height>31680{return Err(Error::Limit);}
                        match h.attr(W,"hRule").unwrap_or("auto") {
                            "atLeast"=>out.min_height=height,
                            "exact"=>{out.min_height=height;out.height_rule=doc_core::RowHeightRule::Exact;},
                            "auto"=>(),
                            _=>return Err(unsupported("Zeilenhöhenregel")),
                        }
                    },
                    "tblHeader"=>{
                        let enabled=if h.attrs.is_empty(){attrs(h,&[])?;true}else{attrs(h,&["val"])?;match h.attr(W,"val"){Some("1"|"true"|"on")=>true,Some("0"|"false"|"off")=>false,_=>return Err(Error::Invalid)}};
                        // Word ignores a noncontiguous marker. Preserve such packages instead of dropping it.
                        if enabled {if usize::from(t.header_rows)!=r {return Err(unsupported("nicht zusammenhängende Tabellenköpfe"));}t.header_rows+=1;}
                    },
                    _=>return Err(unsupported("Zeileneigenschaft")),
                }
            }
            children = &children[1..];
        }
        let mut next_vertical = alloc::vec![None;nc];
        if usize::from(out.grid_before)+usize::from(out.grid_after)>=nc{return Err(Error::Invalid);}
        let mut col = usize::from(out.grid_before);
        for c in children {
            container(c, W, "tc")?;
            if col >= nc {
                return Err(Error::Invalid);
            }
            let mut span = 1usize;
            let mut merge = 0;
            let mut preferred = None;
            let mut fill = None;
            let mut alignment = CellAlignment::Top;
            let mut borders = [None;4];
            let mut content = c.children.as_slice();
            if content.first().is_some_and(|n| n.is(W, "tcPr")) {
                let pr = &content[0];
                container(pr, W, "tcPr")?;
                let mut seen = BTreeSet::new();
                for p in &pr.children {
                    if p.name.ns != W || !seen.insert(&p.name.local) {
                        return Err(Error::Invalid);
                    }
                    match p.name.local.as_str() {
                        "tcW" => preferred = Some(width(p)?),
                        "gridSpan" => {
                            attrs(p, &["val"])?;
                            span = num(p, "val")? as usize;
                        }
                        "vMerge" => {
                            if p.attrs.is_empty() {
                                attrs(p, &[])?;
                                merge = 2;
                            } else {
                                attrs(p, &["val"])?;
                                merge = match p.attr(W, "val") {
                                    Some("restart") => 1,
                                    Some("continue") => 2,
                                    _ => return Err(Error::Invalid),
                                };
                            }
                        }
                        "tcBorders" => borders=crate::cell_borders::read(p)?,
                        "shd" => {
                            attrs(p, &["val", "color", "fill"])?;
                            if p.attr(W, "val") != Some("clear")
                                || p.attr(W, "color") != Some("auto")
                            {
                                return Err(unsupported("Zellschattierung"));
                            }
                            fill = Some(color(p, "fill")?);
                        }
                        "vAlign" => {
                            attrs(p, &["val"])?;
                            alignment = match p.attr(W, "val") {
                                Some("top") => CellAlignment::Top,
                                Some("center") => CellAlignment::Center,
                                Some("bottom") => CellAlignment::Bottom,
                                _ => return Err(unsupported("Zellausrichtung")),
                            };
                        }
                        _ => return Err(unsupported(&p.name.local)),
                    }
                }
                content = &content[1..];
            }
            if span == 0 || span > nc - col {
                return Err(Error::Invalid);
            }
            if preferred.is_some_and(|w| w != t.columns[col..col + span].iter().sum::<u32>()) {
                return Err(unsupported("Zellbreite"));
            }
            if merge == 2 {
                let (ar, expected) = vertical[col].ok_or(Error::Invalid)?;
                if span != expected {
                    return Err(Error::Invalid);
                }
                let anchor = &mut t.rows[ar].cells[col];
                if fill != anchor.fill || alignment != anchor.alignment || borders != anchor.borders {
                    return Err(unsupported("Formatierung der Zellfortsetzung"));
                }
                if content.is_empty()
                    || content.iter().any(|p| {
                        !p.is(W, "p")
                            || !p.children.is_empty()
                            || !p.text.is_empty()
                            || !p.attrs.is_empty()
                    })
                {
                    return Err(unsupported("Inhalt einer Zellfortsetzung"));
                }
                anchor.row_span += 1;
                next_vertical[col] = Some((ar, span));
            } else {
                let cell = &mut out.cells[col];
                cell.row_span = 1;
                cell.column_span = span as u16;
                cell.fill = fill;
                cell.alignment = alignment;
                cell.borders = borders;
                let mut pending = Vec::new();
                for p in content {
                    if p.is(W, "tbl") {
                        let mut descendants=Vec::new();
                        let child=read(p,identities,cursor,&mut descendants,depth+1,mapping,used,next)?;
                        pending.push(child.id);nested.push(child);nested.extend(descendants);continue;
                    }
                    let mut p = read_paragraph(p, mapping, None)?;
                    if p.id == 0 { p.id = notes::allocate(used, next)?; }
                    for t in nested.iter_mut().filter(|t|pending.contains(&t.id)){t.before=p.id;}
                    pending.clear(); cell.paragraphs.push(p);
                }
                if !pending.is_empty() {
                    let id=notes::allocate(used,next)?;
                    for t in nested.iter_mut().filter(|t|pending.contains(&t.id)){t.before=id;}
                    cell.paragraphs.push(Paragraph{id,style:ParagraphStyle::Normal,format:ParagraphFormat::default(),content:vec![Inline::Text(String::new())]});
                }
                if cell.paragraphs.is_empty() { return Err(Error::Invalid); }
                if merge == 1 {
                    next_vertical[col] = Some((r, span));
                }
            }
            col += span;
        }
        if col != nc-usize::from(out.grid_after) {
            return Err(Error::Invalid);
        }
        t.rows.push(out);
        vertical = next_vertical;
    }
    if ids.is_some_and(|i| i.cells.len() != t.rows.len() || i.cells.iter().any(|r| r.len() != nc)) {
        return Err(Error::Invalid);
    }
    Ok(t)
}
