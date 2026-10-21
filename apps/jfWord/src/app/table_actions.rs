// Semantic table commands share the library transactions used by direct page editing.
use super::*;
use doc_core::{CellAlignment, TableChange};
const SMALL: Schema = Schema::Unsigned { min: 0, max: 31680 };
pub(super) static TABLE_ID:Schema=Schema::Object{properties:&[property("table",&U64,true)],additional:false};
static EDGE:Schema=Schema::Object{properties:&[
    property("style",&Schema::String{min:1,max:8,choices:&["inherit","nil","none","single","double","dotted","dashed"]},true),
    property("width",&Schema::Unsigned{min:0,max:96},false),property("color",&Schema::Unsigned{min:0,max:16777215},false),
],additional:false};
static BORDERS:Schema=Schema::Object{properties:&[property("top",&EDGE,false),property("right",&EDGE,false),property("bottom",&EDGE,false),property("left",&EDGE,false)],additional:false};
pub(super) static INSERT: Schema = Schema::Object {
    properties: &[
        property("paragraph", &U64, true),
        property("offset", &U64, true),
        property("rows", &Schema::Unsigned { min: 1, max: 256 }, true),
        property("columns", &Schema::Unsigned { min: 1, max: 32 }, true),
    ],
    additional: false,
};
pub(super) static INSPECT: Schema = Schema::Object {
    properties: &[
        property("table", &U64, true),
        property("offset", &U64, false),
        property("limit", &Schema::Unsigned { min: 1, max: 32 }, false),
    ],
    additional: false,
};
pub(super) static CHANGE: Schema = Schema::Object {
    properties: &[
        property("table", &U64, true),
        property(
            "operation",
            &Schema::String {
                min: 1,
                max: 20,
                choices: &[
                    "position", "borders", "headerRows", "style", "look",
                    "merge",
                    "split",
                    "insertRow",
                    "removeRow",
                    "insertColumn",
                    "removeColumn",
                    "columns",
                    "rowHeight",
                    "rowBreak",
                    "cell",
                    "appearance",
                    "delete",
                ],
            },
            true,
        ),
        property("headerRows", &Schema::Unsigned{min:0,max:256}, false),
        property("floating",&Schema::Boolean,false),
        property("horizontal",&Schema::String{min:1,max:9,choices:&["page","margin","column"]},false),
        property("vertical",&Schema::String{min:1,max:9,choices:&["page","margin","paragraph"]},false),
        property("x",&Schema::Integer{min:-31680,max:31680},false),
        property("y",&SMALL,false),
        property("distance",&Schema::Array{min:4,max:4,items:&SMALL},false),
        property("overlap",&Schema::Boolean,false),
        property("borders",&BORDERS,false),
        property("style", &Schema::String{min:0,max:64,choices:&[]}, false),
        property("look", &table_style_actions::LOOK, false),
        property("index", &SMALL, false),
        property("row", &SMALL, false),
        property("column", &SMALL, false),
        property("height", &SMALL, false),
        property("heightRule", &Schema::String{min:5,max:7,choices:&["atLeast","exact"]}, false),
        property("allowSplit", &Schema::Boolean, false),
        property("rows", &Schema::Unsigned { min: 1, max: 256 }, false),
        property("columns", &Schema::Unsigned { min: 1, max: 32 }, false),
        property("padding", &Schema::Unsigned { min: 0, max: 720 }, false),
        property("borderWidth", &Schema::Unsigned { min: 0, max: 80 }, false),
        property(
            "borderColor",
            &Schema::Unsigned {
                min: 0,
                max: 16777215,
            },
            false,
        ),
        property(
            "widths",
            &Schema::Array {
                min: 1,
                max: 32,
                items: &SMALL,
            },
            false,
        ),
        property(
            "fill",
            &Schema::String {
                min: 0,
                max: 6,
                choices: &[],
            },
            false,
        ),
        property(
            "alignment",
            &Schema::String {
                min: 1,
                max: 6,
                choices: &["top", "center", "bottom"],
            },
            false,
        ),
    ],
    additional: false,
};
fn required(v: &Value, key: &str) -> Result<u64, ActionError> {
    v.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid(format!("Missing {key}")))
}
pub(super) fn change(d: &Document,v: &Value) -> Result<TableChange, ActionError> {
    Ok(match text(v, "operation").as_str() {
        "position"=>{
            use doc_core::{TablePosition,TableHorizontal as H,TableVertical as V};
            let floating=v.get("floating").and_then(Value::as_bool).ok_or_else(||invalid("Missing floating"))?;
            if !floating {
                if ["horizontal","vertical","x","y","distance","overlap"].iter().any(|k|v.get(k).is_some()){return Err(invalid("Inline positioning takes only floating:false"));}
                TableChange::Position(None)
            }else{
                let t=d.table(id(v,"table")).ok_or_else(||invalid("Missing table"))?;
                let mut p:TablePosition=t.position.unwrap_or_default();
                if let Some(h)=v.get("horizontal").and_then(Value::as_str){p.horizontal=match h{"page"=>H::Page,"column"=>H::Column,_=>H::Margin};}
                if let Some(h)=v.get("vertical").and_then(Value::as_str){p.vertical=match h{"page"=>V::Page,"margin"=>V::Margin,_=>V::Paragraph};}
                if let Some(x)=v.get("x").and_then(Value::as_i64){p.x=x as i32;}
                if let Some(y)=v.get("y").and_then(Value::as_u64){p.y=y as i32;}
                if let Some(a)=v.get("distance").and_then(Value::as_array){for(i,n)in a.iter().enumerate(){p.distance[i]=n.as_u64().unwrap() as u16;}}
                if let Some(b)=v.get("overlap").and_then(Value::as_bool){p.overlap=b;}
                TableChange::Position(Some(p))
            }
        },
        "borders"=>{
            let(row,column)=(required(v,"row")? as usize,required(v,"column")? as usize);
            let t=d.table(id(v,"table")).ok_or_else(||invalid("Missing table"))?;
            let mut borders=t.cell(row,column).ok_or_else(||invalid("Missing cell"))?.borders;
            let input=v.get("borders").ok_or_else(||invalid("Missing borders"))?;let mut changed=false;
            for side in doc_core::BorderSide::ALL {if let Some(edge)=input.get(side.name()){
                changed=true;let style=text(edge,"style");
                borders[side as usize]=if style=="inherit"{if edge.get("width").is_some() || edge.get("color").is_some(){return Err(invalid("Inherited borders take no width/color"));}None}else{
                    let style=doc_core::BorderStyle::parse(&style).ok_or_else(||invalid("Unknown border style"))?;
                    let b=doc_core::CellBorder{style,width:edge.get("width").and_then(Value::as_u64).unwrap_or(if matches!(style,doc_core::BorderStyle::Nil|doc_core::BorderStyle::None){0}else{4}) as u16,color:edge.get("color").and_then(Value::as_u64).unwrap_or(0) as u32};
                    b.validate().map_err(|_|invalid("Visible border width is 2..96 eighth-points; nil/none have no width/color"))?;Some(b)
                };
            }}
            if !changed{return Err(invalid("Choose at least one cell edge"));}TableChange::Borders{row,column,borders}
        },
        "headerRows"=>TableChange::HeaderRows(required(v,"headerRows")? as u16),
        "style"|"look"=>{
            let t=d.table(id(v,"table")).ok_or_else(||invalid("Missing table"))?;
            let look=v.get("look").map_or(t.look,|v|table_style_actions::look(v,t.look));
            if text(v,"operation")=="look" {if v.get("look").is_none(){return Err(invalid("Missing look"));}TableChange::Look(look)}
            else {let key=v.get("style").and_then(Value::as_str).ok_or_else(||invalid("Missing style; empty removes the template"))?;TableChange::Style{style:if key.is_empty(){None}else{Some(String::from(key))},look}}
        },
        "merge" => TableChange::Merge {
            row: required(v, "row")? as usize,
            column: required(v, "column")? as usize,
            rows: required(v, "rows")? as usize,
            columns: required(v, "columns")? as usize,
        },
        "split" => TableChange::Split {
            row: required(v, "row")? as usize,
            column: required(v, "column")? as usize,
        },
        "insertRow" => TableChange::InsertRow(required(v, "index")? as usize),
        "removeRow" => TableChange::RemoveRow(required(v, "index")? as usize),
        "insertColumn" => TableChange::InsertColumn(required(v, "index")? as usize),
        "removeColumn" => TableChange::RemoveColumn(required(v, "index")? as usize),
        "columns" => TableChange::Columns(
            v.get("widths")
                .and_then(Value::as_array)
                .ok_or_else(|| invalid("Missing widths"))?
                .iter()
                .map(|n| n.as_u64().unwrap() as u32)
                .collect(),
        ),
        "rowHeight" => {
            let row=required(v,"row")? as usize;
            let height=required(v,"height")? as u32;
            if let Some(rule)=v.get("heightRule").and_then(Value::as_str) {
                TableChange::RowSizing{row,height,rule:if rule=="exact"{doc_core::RowHeightRule::Exact}else{doc_core::RowHeightRule::AtLeast}}
            }else{TableChange::RowHeight{row,height}}
        },
        "rowBreak" => TableChange::RowBreak {
            row: required(v, "row")? as usize,
            allow_split: v.get("allowSplit").and_then(Value::as_bool).ok_or_else(|| invalid("Missing allowSplit"))?,
        },
        "cell" => {
            let fill = v
                .get("fill")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("Missing fill (empty string removes shading)"))?;
            let fill = if fill.is_empty() {
                None
            } else {
                if fill.len() != 6 {
                    return Err(invalid("Expected six hexadecimal RGB digits"));
                }
                Some(u32::from_str_radix(fill, 16).map_err(|_| invalid("Invalid RGB"))?)
            };
            let alignment = match v.get("alignment").and_then(Value::as_str) {
                Some("top") => CellAlignment::Top,
                Some("center") => CellAlignment::Center,
                Some("bottom") => CellAlignment::Bottom,
                _ => return Err(invalid("Missing alignment")),
            };
            TableChange::Cell {
                row: required(v, "row")? as usize,
                column: required(v, "column")? as usize,
                fill,
                alignment,
            }
        }
        "appearance" => TableChange::Appearance {
            padding: required(v, "padding")? as u16,
            border_width: required(v, "borderWidth")? as u16,
            border_color: required(v, "borderColor")? as u32,
        },
        "delete" => TableChange::Delete,
        _ => return Err(invalid("Unknown table operation")),
    })
}
pub(super) fn summary(d:&Document,t: &doc_core::Table) -> Value {
    Value::object([
        ("id", Value::u64(t.id)),
        ("before", Value::u64(t.before)),
        ("position",t.position.map(|p|{use doc_core::{TableHorizontal as H,TableVertical as V};Value::object([
            ("horizontal",Value::string(match p.horizontal{H::Page=>"page",H::Margin=>"margin",H::Column=>"column"})),
            ("vertical",Value::string(match p.vertical{V::Page=>"page",V::Margin=>"margin",V::Paragraph=>"paragraph"})),
            ("x",Value::i64(p.x as i64)),("y",Value::i64(p.y as i64)),
            ("distance",Value::Array(p.distance.iter().map(|n|Value::u64(*n as u64)).collect())),("overlap",Value::Bool(p.overlap))
        ])}).unwrap_or(Value::Null)),
        ("depth",Value::u64(d.table_depth(t.id).unwrap_or(0) as u64)),
        ("parent",d.table_parent(t.id).map(|(id,_,_)|Value::u64(id)).unwrap_or(Value::Null)),
        ("headerRows",Value::u64(t.header_rows.into())),
        ("style",t.style.as_ref().map(Value::string).unwrap_or(Value::Null)),
        ("look",table_style_actions::look_json(t.look)),
        ("borderOverride",Value::Bool(t.border_override)),
        (
            "columns",
            Value::Array(t.columns.iter().map(|w| Value::u64((*w).into())).collect()),
        ),
        ("rowCount", Value::u64(t.rows.len() as u64)),
        ("padding", Value::u64(t.padding.into())),
        ("borderWidth", Value::u64(t.border_width.into())),
        ("borderColor", Value::u64(t.border_color.into())),
    ])
}
fn border_json(b:doc_core::CellBorder)->Value{Value::object([("style",Value::string(b.style.name())),("width",Value::u64(b.width.into())),("color",Value::u64(b.color.into()))])}
pub(super) fn inspect(d: &Document, v: &Value) -> Result<Value, ActionError> {
    let t = d
        .table(id(v, "table"))
        .ok_or_else(|| invalid("Missing table"))?;
    let offset = index(v, "offset")?;
    let limit = v.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
    let rows = t
        .rows
        .iter()
        .enumerate()
        .skip(offset)
        .take(limit)
        .map(|(r, row)| {
            Value::object([
                ("index", Value::u64(r as u64)),
                ("height", Value::u64(row.min_height.into())),
                ("heightRule", Value::string(if row.height_rule==doc_core::RowHeightRule::Exact {"exact"} else {"atLeast"})),
                ("allowSplit", Value::Bool(!row.cant_split)),
                ("gridBefore",Value::u64(row.grid_before.into())),("gridAfter",Value::u64(row.grid_after.into())),
                (
                    "cells",
                    Value::Array(
                        row.cells
                            .iter()
                            .enumerate().map(|(column,c)| {
                                Value::object([
                                    ("id", Value::u64(c.id)),
                                    ("omitted",Value::Bool(t.omitted(r,column))),
                                    ("borders",Value::object(doc_core::BorderSide::ALL.map(|side|(side.name(),c.borders[side as usize].map(border_json).unwrap_or(Value::Null))))),
                                    ("effectiveFill",d.table_region(t,r,column).fill.map(|rgb|Value::string(format!("{rgb:06X}"))).unwrap_or(Value::Null)),
                                    ("rowSpan", Value::u64(c.row_span.into())),
                                    ("columnSpan", Value::u64(c.column_span.into())),
                                    ("covered", Value::Bool(c.is_covered())),
                                    (
                                        "paragraphs",
                                        Value::Array(
                                            c.paragraphs.iter().map(|p| Value::u64(p.id)).collect(),
                                        ),
                                    ),
                                    (
                                        "fill",
                                        c.fill
                                            .map(|v| Value::string(format!("{v:06X}")))
                                            .unwrap_or(Value::Null),
                                    ),
                                    (
                                        "alignment",
                                        Value::string(match c.alignment {
                                            CellAlignment::Top => "top",
                                            CellAlignment::Center => "center",
                                            CellAlignment::Bottom => "bottom",
                                        }),
                                    ),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ])
        })
        .collect();
    Ok(Value::object([
        ("table", summary(d,t)),
        ("rows", Value::Array(rows)),
        (
            "nextOffset",
            if offset + limit < t.rows.len() {
                Value::u64((offset + limit) as u64)
            } else {
                Value::Null
            },
        ),
    ]))
}

pub(super) static FIT: Schema = Schema::Object {
    properties: &[
        property("table", &U64, true),
        property(
            "mode",
            &Schema::String {
                min: 1,
                max: 8,
                choices: &["contents", "page", "equal"],
            },
            true,
        ),
    ],
    additional: false,
};

impl Word {
    pub(super) fn table_layout(&mut self,ui:&mut Ui,v:&Value)->Result<Value,ActionError>{
        let table=id(v,"table");
        self.editor.document().table(table).ok_or_else(||invalid("Missing table"))?;
        let fonts=self.preview.prepare_fonts(&mut ui.text).map_err(|e|invalid(format!("Font resolution: {e:?}")))?;
        let layout=doc_view::paginate(self.editor.document(),&mut ui.text,&fonts).map_err(|e|invalid(format!("Table layout: {e:?}; header rows must leave room for body text")))?;
        let offset=index(v,"offset")?;let limit=(v.get("limit").and_then(Value::as_u64).unwrap_or(2) as usize).min(8);
        let pages=layout.pages.iter().enumerate().skip(offset).take(limit).map(|(i,p)|Value::object([
            ("bodyLines",Value::Array(p.lines.iter().filter(|l|l.cell.is_none()).take(512).map(|l|Value::object([
                ("paragraph",Value::u64(l.paragraph)),("start",Value::u64(l.start as u64)),("end",Value::u64(l.end as u64)),
                ("bounds",Value::Array([l.x,l.top,l.fragments.iter().map(|f|f.x+f.width).max().unwrap_or(l.x)-l.x,l.height].iter().map(|v|Value::i64(*v as i64)).collect()))
            ])).collect())),("totalBodyLines",Value::u64(p.lines.iter().filter(|l|l.cell.is_none()).count() as u64)),
            ("index",Value::u64(i as u64)),("cells",Value::Array(p.cells.iter().filter(|c|c.table==table).map(|c|Value::object([
                ("id",Value::u64(c.cell)),("paragraph",Value::u64(c.paragraph)),("row",Value::u64(c.row as u64)),("column",Value::u64(c.column as u64)),
                ("repeated",Value::Bool(c.repeated)),("rowEnd",Value::Bool(c.row_end)),("bounds",Value::Array(c.rect.iter().map(|v|Value::i64(i64::from(*v))).collect())),
                ("fill",c.fill.map(|v|Value::string(format!("{v:06X}"))).unwrap_or(Value::Null)),
                ("edges",Value::Array(c.edges.iter().map(|e|Value::object([("side",Value::string(e.side.name())),("offset",Value::i64(e.offset.into())),("length",Value::i64(e.length.into())),("border",border_json(e.border))])).collect())),
            ])).collect()))
        ])).collect();
        Ok(Value::object([("pages",Value::Array(pages)),("pageCount",Value::u64(layout.pages.len() as u64)),
            ("nextOffset",if offset.saturating_add(limit)<layout.pages.len(){Value::u64((offset+limit) as u64)}else{Value::Null})]))
    }
}
