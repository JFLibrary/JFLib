//! Named table formatting, independent of application themes and pagination.
use crate::{Document, Error, Paragraph, ResolvedFormat, Table, TextColor, TextFormat};
use alloc::{string::String, vec::Vec};

/// Conditional regions enabled on a table. Header styling and repeated rows are independent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableLook {
    pub first_row: bool,
    pub last_row: bool,
    pub first_column: bool,
    pub last_column: bool,
    pub banded_rows: bool,
    pub banded_columns: bool,
}
impl Default for TableLook {
    fn default() -> Self {
        Self { first_row: true, last_row: false, first_column: false, last_column: false, banded_rows: true, banded_columns: false }
    }
}
/// Absent values inherit the whole-table region. Explicit white removes inherited shading.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TableRegion {
    pub fill: Option<u32>,
    pub text: TextFormat,
}
impl TableRegion {
    fn over(self, base: Self) -> Self {
        Self { fill: self.fill.or(base.fill), text: base.text.overlay(self.text) }
    }
}
/// Stable conditional regions shared by editors, codecs and automation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableRegionKind { Body, FirstRow, LastRow, FirstColumn, LastColumn, Band1, Band2, Band1Vertical, Band2Vertical, TopLeft, TopRight, BottomLeft, BottomRight }
impl TableRegionKind {
    pub const ALL: [Self; 13] = [Self::Body,Self::FirstRow,Self::LastRow,Self::FirstColumn,Self::LastColumn,Self::Band1,Self::Band2,Self::Band1Vertical,Self::Band2Vertical,Self::TopLeft,Self::TopRight,Self::BottomLeft,Self::BottomRight];
    pub fn key(self)->&'static str {match self {
        Self::Body=>"body",Self::FirstRow=>"firstRow",Self::LastRow=>"lastRow",Self::FirstColumn=>"firstColumn",Self::LastColumn=>"lastColumn",Self::Band1=>"band1",Self::Band2=>"band2",Self::Band1Vertical=>"band1Vertical",Self::Band2Vertical=>"band2Vertical",Self::TopLeft=>"topLeft",Self::TopRight=>"topRight",Self::BottomLeft=>"bottomLeft",Self::BottomRight=>"bottomRight",
    }}
}
/// A document-owned style. IDs remain stable when its display name or formatting changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableStyle {
    pub id: String,
    pub name: String,
    pub based_on: Option<String>,
    /// Omit the border pair and inherit it. Raw values remain 4 / 808080 in this case.
    pub border_inherit: bool,
    pub border_width: u16,
    pub border_color: u32,
    /// None inherits; without a base, one row/column constitutes a band.
    pub row_band_size: Option<u16>,
    pub column_band_size: Option<u16>,
    pub body: TableRegion,
    pub first_row: TableRegion,
    pub last_row: TableRegion,
    pub first_column: TableRegion,
    pub last_column: TableRegion,
    pub band1: TableRegion,
    pub band2: TableRegion,
    pub band1_vertical: TableRegion,
    pub band2_vertical: TableRegion,
    pub top_left: TableRegion,
    pub top_right: TableRegion,
    pub bottom_left: TableRegion,
    pub bottom_right: TableRegion,
}
impl TableStyle {
    pub fn new(id: &str, name: &str) -> Self {
        Self { id: String::from(id), name: String::from(name), based_on: None, border_inherit: false, border_width: 4, border_color: 0x808080,
            row_band_size: None, column_band_size: None,
            body: TableRegion::default(), first_row: TableRegion::default(), last_row: TableRegion::default(),
            first_column: TableRegion::default(), last_column: TableRegion::default(),
            band1: TableRegion::default(), band2: TableRegion::default(),
            band1_vertical: TableRegion::default(), band2_vertical: TableRegion::default(),
            top_left: TableRegion::default(), top_right: TableRegion::default(), bottom_left: TableRegion::default(), bottom_right: TableRegion::default() }
    }
    /// Start a live derived style without freezing any properties from its parent.
    pub fn derived(id:&str,name:&str,base:&str)->Self {
        let mut s=Self::new(id,name);s.based_on=Some(String::from(base));s.border_inherit=true;s
    }
    pub fn region_format(&self,kind:TableRegionKind)->TableRegion {match kind {
        TableRegionKind::Body=>self.body,TableRegionKind::FirstRow=>self.first_row,TableRegionKind::LastRow=>self.last_row,
        TableRegionKind::FirstColumn=>self.first_column,TableRegionKind::LastColumn=>self.last_column,
        TableRegionKind::Band1=>self.band1,TableRegionKind::Band2=>self.band2,
        TableRegionKind::Band1Vertical=>self.band1_vertical,TableRegionKind::Band2Vertical=>self.band2_vertical,
        TableRegionKind::TopLeft=>self.top_left,TableRegionKind::TopRight=>self.top_right,
        TableRegionKind::BottomLeft=>self.bottom_left,TableRegionKind::BottomRight=>self.bottom_right,
    }}
    pub fn region_format_mut(&mut self,kind:TableRegionKind)->&mut TableRegion {match kind {
        TableRegionKind::Body=>&mut self.body,TableRegionKind::FirstRow=>&mut self.first_row,TableRegionKind::LastRow=>&mut self.last_row,
        TableRegionKind::FirstColumn=>&mut self.first_column,TableRegionKind::LastColumn=>&mut self.last_column,
        TableRegionKind::Band1=>&mut self.band1,TableRegionKind::Band2=>&mut self.band2,
        TableRegionKind::Band1Vertical=>&mut self.band1_vertical,TableRegionKind::Band2Vertical=>&mut self.band2_vertical,
        TableRegionKind::TopLeft=>&mut self.top_left,TableRegionKind::TopRight=>&mut self.top_right,
        TableRegionKind::BottomLeft=>&mut self.bottom_left,TableRegionKind::BottomRight=>&mut self.bottom_right,
    }}
    pub fn validate(&self) -> Result<(), Error> {
        if self.id.is_empty() || self.id.len() > 64 || !self.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || self.name.is_empty() || self.name.len() > 128 || self.name.trim() != self.name || self.name.chars().any(char::is_control)
            || self.border_width > 80 || self.border_color > 0xffffff { return Err(Error::Invalid); }
        if ["Normal","Title","Heading1","Heading2"].contains(&self.id.as_str()) || self.id.starts_with("jfStyle") { return Err(Error::Invalid); }
        if self.based_on.as_ref().is_some_and(|v|v.is_empty() || v.len()>64 || !v.bytes().all(|b|b.is_ascii_alphanumeric() || b==b'_' || b==b'-'))
            || self.border_inherit && (self.border_width!=4 || self.border_color!=0x808080)
            || self.row_band_size.is_some_and(|n|n==0 || n>256) || self.column_band_size.is_some_and(|n|n==0 || n>32) {return Err(Error::Invalid);}
        crate::check_text(&self.name)?;
        for kind in TableRegionKind::ALL {let r=self.region_format(kind);
            if r.fill.is_some_and(|v| v > 0xffffff) { return Err(Error::Invalid); }
            r.text.validate()?;
        }
        Ok(())
    }
    /// Row/column banding, edge columns, edge rows, corners; direct cell/run formatting wins later.
    /// Call on a resolved definition when it has a parent.
    pub fn region(&self, table: &Table, row: usize, column: usize) -> TableRegion {
        let Some(cell) = table.cell(row, column) else { return self.body; };
        let (row, column) = table.origin(row, column).unwrap();
        let mut r = self.body;
        let look = table.look;
        let first_rows=usize::from(table.header_rows).max(1);
        if look.banded_rows && !(look.first_row && row < first_rows) && !(look.last_row && row + usize::from(cell.row_span) == table.rows.len()) {
            let offset = if look.first_row { first_rows } else { 0 };
            r = if row.saturating_sub(offset)/usize::from(self.row_band_size.unwrap_or(1).max(1)) % 2 == 0 { self.band1 } else { self.band2 }.over(r);
        }
        let left=look.first_column && column==0;
        let right=look.last_column && column+usize::from(cell.column_span)==table.columns.len();
        let bottom=look.last_row && row+usize::from(cell.row_span)==table.rows.len();
        if look.banded_columns && !left && !right {
            let offset=usize::from(look.first_column);
            r=if column.saturating_sub(offset)/usize::from(self.column_band_size.unwrap_or(1).max(1))%2==0 {self.band1_vertical}else{self.band2_vertical}.over(r);
        }
        if left { r = self.first_column.over(r); }
        if right { r = self.last_column.over(r); }
        if look.first_row && row < first_rows { r = self.first_row.over(r); }
        if bottom { r = self.last_row.over(r); }
        if look.first_row && row==0 && left {r=self.top_left.over(r);}
        if look.first_row && row==0 && right {r=self.top_right.over(r);}
        if bottom && left {r=self.bottom_left.over(r);}
        if bottom && right {r=self.bottom_right.over(r);}
        r
    }
}

impl Document {
    /// Available document definitions, including the shared built-in palette.
    pub fn table_style_ids(&self) -> Vec<String> {
        let mut ids: Vec<_> = ["jfTableBlue", "jfTableTeal", "jfTableGray", "jfTablePlain"].iter().map(|s| String::from(*s)).collect();
        for s in &self.table_styles { if !ids.contains(&s.id) { ids.push(s.id.clone()); } }
        ids
    }
    pub fn table_style(&self, id: &str) -> Option<TableStyle> {
        if let Some(s) = self.table_styles.iter().find(|s| s.id == id) { return Some(s.clone()); }
        let (name, ink, band) = match id {
            "jfTableBlue" => ("Blau · Zeilenbänder", 0x196180, 0xdceef5),
            "jfTableTeal" => ("Petrol · Zeilenbänder", 0x267569, 0xe0f0ec),
            "jfTableGray" => ("Grau · Zeilenbänder", 0x525b66, 0xeceef1),
            "jfTablePlain" => ("Schlicht · Raster", 0x808080, 0xffffff),
            _ => return None,
        };
        let mut s = TableStyle::new(id, name);
        s.border_color = if id == "jfTablePlain" { ink } else { band };
        s.body.fill = Some(0xffffff);
        s.band1.fill = Some(band);
        s.first_row = TableRegion { fill: Some(if id == "jfTablePlain" { 0xffffff } else { ink }),
            text: TextFormat { bold: Some(true), color: Some(TextColor::Rgb(if id == "jfTablePlain" { 0x20272b } else { 0xffffff })), ..TextFormat::default() } };
        s.last_row.text.bold = Some(true);
        s.first_column.text.bold = Some(true);
        s.last_column.text.bold = Some(true);
        Some(s)
    }
    /// Resolve at most 32 definitions; missing parents, cycles and excessive depth are errors.
    pub fn resolved_table_style(&self,id:&str)->Result<TableStyle,Error> {
        let mut chain=Vec::new();let mut key=String::from(id);
        loop {
            if chain.len()>=32 {return Err(Error::Limit);}
            if chain.iter().any(|s:&TableStyle|s.id==key) {return Err(Error::Invalid);}
            let s=self.table_style(&key).ok_or(Error::Missing)?;
            if chain.is_empty() && s.based_on.is_none(){let mut s=s;s.border_inherit=false;return Ok(s);}
            let base=s.based_on.clone();chain.push(s);
            let Some(base)=base else {break;};key=base;
        }
        let mut result=TableStyle::new(id,&chain[0].name);
        for s in chain.iter().rev() {
            if !s.border_inherit {result.border_width=s.border_width;result.border_color=s.border_color;}
            result.row_band_size=s.row_band_size.or(result.row_band_size);
            result.column_band_size=s.column_band_size.or(result.column_band_size);
            for kind in TableRegionKind::ALL {
                let inherited=result.region_format(kind);
                *result.region_format_mut(kind)=s.region_format(kind).over(inherited);
            }
        }Ok(result)
    }
    /// Whether an existing style depends on a candidate parent, including itself.
    pub fn table_style_depends_on(&self,id:&str,ancestor:&str)->bool {
        let mut key=String::from(id);
        for _ in 0..32 {
            if key==ancestor{return true;}
            let Some(base)=self.table_style(&key).and_then(|s|s.based_on) else{return false;};key=base;
        }true
    }
    /// Effective cell shading and character defaults; direct shading takes precedence.
    pub fn table_region(&self, table: &Table, row: usize, column: usize) -> TableRegion {
        let mut region = table.style.as_deref().and_then(|id| self.resolved_table_style(id).ok()).map_or_else(TableRegion::default, |s| s.region(table, row, column));
        if let Some(cell) = table.cell(row, column) { region.fill = cell.fill.or(region.fill); }
        region
    }
    pub fn table_border(&self, table: &Table) -> (u16, u32) {
        if !table.border_override {
            if let Some(s) = table.style.as_deref().and_then(|id| self.resolved_table_style(id).ok()) { return (s.border_width, s.border_color); }
        }
        (table.border_width, table.border_color)
    }
    /// Table defaults precede paragraph styles and direct character overrides.
    pub fn resolve_paragraph_text(&self, p: &Paragraph, direct: TextFormat) -> ResolvedFormat {
        let base = self.table_location(p.id).and_then(|(id, r, c)| self.table(id).map(|t| self.table_region(t, r, c).text)).unwrap_or_default();
        self.resolve_text_base(base, direct, p.style)
    }
    pub(crate) fn validate_table_styles(&self) -> Result<(), Error> {
        if self.table_styles.len() > 128 { return Err(Error::Limit); }
        for (i, s) in self.table_styles.iter().enumerate() {
            s.validate()?;
            self.resolved_table_style(&s.id)?;
            if self.table_styles[..i].iter().any(|v| v.id == s.id) { return Err(Error::Invalid); }
        }
        let mut names: Vec<String> = Vec::new();
        for id in self.table_style_ids() {
            let s = self.table_style(&id).ok_or(Error::Missing)?;
            if names.iter().any(|n| n.eq_ignore_ascii_case(&s.name)) { return Err(Error::Invalid); }
            names.push(s.name);
        }
        Ok(())
    }
    pub(crate) fn set_table_style(&mut self, style: TableStyle) -> Result<(), Error> {
        style.validate()?;
        if let Some(s) = self.table_styles.iter_mut().find(|s| s.id == style.id) { *s = style; }
        else { self.table_styles.push(style); }
        Ok(())
    }
}
