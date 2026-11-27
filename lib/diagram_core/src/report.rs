// report -- (JD19) shape reports (DA-09, DA-13, AS-05, EX-12): Visio's report definitions as a
// model, run against a drawing into a table, put on a page as a table shape
//
//   ReportDef                     a definition as Visio's VisioReportDefinition describes it:
//                                 name, title, description, which shapes (Scope), the criteria
//                                 (Filter), the columns (Field: built-in fields, shape data,
//                                 user cells; display names and order; summaries), grouping
//                                 with subtotals, grand summary and duplicates (Grouping),
//                                 up to three sort keys (Sort), precision, units, case; what a
//                                 file had and the model does not know stays as data (attrs,
//                                 extra) for the writer
//   run(doc, &def, &Ctx) -> Table   the result: title, columns, item rows, group headers,
//                                 subtotal rows, the grand total; every cell as text (in the
//                                 definition's precision, with units when it says so) and
//                                 typed (for CSV and XLSX)
//   place(tx, eng, owner, at, &table, &Place) -> ShapeRef   the table shape: a group of cell
//                                 shapes (title, header, grid, group and summary rows) on its
//                                 own layer, marked with User.JFD_Report* (report/shape.rs)
//   report_shape(doc, shape) / refresh(tx, eng, shape, &table) / report_layer(..)   find a table
//                                 shape (a cell of it too), build it anew in place (DA-13)
//
// The XML (VisioReportDefinition, .vrd files) and the definitions kept in the document sheet
// (User.visPropRptN, U="STR") are diagram_io::report: this crate holds no file format.  The
// codes below (Scope::code, Condition::code, Func::bit, Property::tag, Builtin::key) are the
// numbers and names that XML carries.
//
// Assumptions where no source says (Annahmen; the user's own Visio file will settle them):
// * SelectDrawingDataMethod: 1 every page, 2 the current page (both seen), 3 the selected
//   shapes (not seen); another number reads as Scope::Other and runs over every page.
// * Condition (1, 3 and 8 seen): 1 =, 2 <>, 3 >, 4 >=, 5 <, 6 <=, 7 does not exist, 8 exists
//   (CONDITIONS below: one table to change).
// * SummaryTypes (0, 1, 3 seen) a bit mask in the order of Visio's dialog: 1 total, 2 average,
//   4 count, 8 minimum, 16 maximum (Func::bit).
// * Tag 1 a built-in field, 2 shape data, 3 a user cell (seen); Type and PropertyDataType are
//   the shape data types of diagram_core::data::kind (0 text, 2 number, 3 Boolean seen).
// * ItemDisplay 1 shows the shapes' rows with the summaries, 0 only the summaries (one row per
//   group); GroupingField 0 groups nothing; a Boolean attribute is 1 or 0 (true / -1 read too).
// * A shape data field is named by its row name (Prop.<Name>); a shape that has no row of that
//   name but one with that label counts too (Visio's dialog lists the fields by label).
// * NAMEINFO is the shape's name as shown (Name), NAMEIDINFO its NameID ("Sheet.5"); lengths
//   (PINXINFO ...) in the unit of their cell, else the page's.
// * Visio's output "Visio-Shape" is an embedded Excel object (NO-22: not here); which cell marks
//   it is not known.  The table shape here is JFDiagrammer's own: User.JFD_Report = 1,
//   JFD_ReportName, JFD_ReportDef (a copy of the definition) or JFD_ReportLink (the
//   definition's name in the drawing or a .vrd path), on the layer the app names ("Bericht").
//
// Which shapes a report sees (run.rs): the top-level shapes of the pages of its scope (the
// foreground pages for "every page") or the selected shapes; a group without master and without
// shape data of its own stands for its members (a user's group of devices counts its devices),
// every other shape counts as one -- an instance's subshapes never.  Table shapes of reports
// are left out.  Summaries: Count counts the rows of the group, the others take the numbers.

use alloc::string::String;
use alloc::vec::Vec;

mod run;
mod shape;

pub use run::{run, Cell as TableCell, Column, Ctx, Labels, Row as TableRow, RowKind, Table, Val};
pub use shape::{place, refresh, report_layer, report_shape, Place, ReportShape, MARK, MARK_DEF, MARK_LINK, MARK_NAME, MAX_ROWS};

/// which shapes a report takes (SelectDrawingDataMethod)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    /// the shapes of every (foreground) page: 1
    AllPages,
    /// the shapes of the current page: 2
    Page,
    /// the selected shapes: 3 (assumed)
    Selection,
    /// a number not known here (runs like AllPages, written back as read)
    Other(u32),
}

impl Scope {
    /// the code of SelectDrawingDataMethod
    pub fn code(self) -> u32 {
        match self {
            Scope::AllPages => 1,
            Scope::Page => 2,
            Scope::Selection => 3,
            Scope::Other(n) => n,
        }
    }

    /// the scope of a code
    pub fn of(code: u32) -> Scope {
        match code {
            1 => Scope::AllPages,
            2 => Scope::Page,
            3 => Scope::Selection,
            n => Scope::Other(n),
        }
    }
}

/// a built-in field (Tag 1): its name in the definition
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Builtin {
    /// TEXTINFO: the shape's text as shown (fields included)
    Text,
    /// MASTERINFO: the name of the shape's master
    Master,
    /// NAMEINFO: the shape's name
    Name,
    /// NAMEIDINFO: the shape's NameID ("Sheet.5")
    NameId,
    /// PINXINFO
    PinX,
    /// PINYINFO
    PinY,
    /// WIDTHINFO
    Width,
    /// HEIGHTINFO
    Height,
    /// a name not known here (kept; it shows nothing)
    Other(String),
}

/// the built-in fields in the order the wizard offers them
pub const BUILTINS: [Builtin; 8] = [Builtin::Text, Builtin::Master, Builtin::Name, Builtin::NameId, Builtin::PinX, Builtin::PinY, Builtin::Width, Builtin::Height];

impl Builtin {
    /// the name in the definition (TEXTINFO ...)
    pub fn key(&self) -> &str {
        match self {
            Builtin::Text => "TEXTINFO",
            Builtin::Master => "MASTERINFO",
            Builtin::Name => "NAMEINFO",
            Builtin::NameId => "NAMEIDINFO",
            Builtin::PinX => "PINXINFO",
            Builtin::PinY => "PINYINFO",
            Builtin::Width => "WIDTHINFO",
            Builtin::Height => "HEIGHTINFO",
            Builtin::Other(s) => s,
        }
    }

    /// the field of a name
    pub fn of(key: &str) -> Builtin {
        BUILTINS.iter().find(|b| b.key().eq_ignore_ascii_case(key)).cloned().unwrap_or_else(|| Builtin::Other(String::from(key)))
    }

    /// whether its values are lengths
    pub fn is_length(&self) -> bool {
        matches!(self, Builtin::PinX | Builtin::PinY | Builtin::Width | Builtin::Height)
    }
}

/// what a column or a criterion reads from a shape
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Property {
    /// a built-in field (Tag / PropertyType 1)
    Builtin(Builtin),
    /// a shape data field by its row name (Prop.<name>; Tag 2)
    Data(String),
    /// a user cell by its row name (User.<name>; Tag 3)
    User(String),
}

impl Property {
    /// Tag / PropertyType
    pub fn tag(&self) -> u32 {
        match self {
            Property::Builtin(_) => 1,
            Property::Data(_) => 2,
            Property::User(_) => 3,
        }
    }

    /// the name the definition writes (TEXTINFO, the row name)
    pub fn key(&self) -> &str {
        match self {
            Property::Builtin(b) => b.key(),
            Property::Data(n) | Property::User(n) => n,
        }
    }

    /// the property of a tag and a name (an unknown tag reads as shape data)
    pub fn of(tag: u32, key: &str) -> Property {
        match tag {
            1 => Property::Builtin(Builtin::of(key)),
            3 => Property::User(String::from(key)),
            _ => Property::Data(String::from(key)),
        }
    }
}

/// a summary function of a column (SummaryTypes, a bit each)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Func {
    /// the sum: bit 1
    Total,
    /// the mean: bit 2
    Average,
    /// the number of rows: bit 4
    Count,
    /// the smallest number: bit 8
    Min,
    /// the largest number: bit 16
    Max,
}

/// the functions in the order of the dialog (and of the summary rows)
pub const FUNCS: [Func; 5] = [Func::Total, Func::Average, Func::Count, Func::Min, Func::Max];

impl Func {
    /// its bit in SummaryTypes
    pub fn bit(self) -> u32 {
        match self {
            Func::Total => 1,
            Func::Average => 2,
            Func::Count => 4,
            Func::Min => 8,
            Func::Max => 16,
        }
    }

    /// the functions of a SummaryTypes mask, in order
    pub fn of_mask(mask: u32) -> Vec<Func> {
        FUNCS.iter().copied().filter(|f| mask & f.bit() != 0).collect()
    }
}

/// a column of the report (VisioRptDefField)
#[derive(Clone, PartialEq, Debug)]
pub struct Field {
    /// ID (groups and sort keys name it)
    pub id: u32,
    /// what it shows (Tag with Name)
    pub property: Property,
    /// DisplayName: the column's heading ("" -- the property's name)
    pub display_name: String,
    /// Display: shown as a column
    pub display: bool,
    /// DisplayOrder: the column's place (smaller first)
    pub order: u32,
    /// Type: the data type (data::kind: 0 text, 2 number ...)
    pub data_type: u16,
    /// SummaryTypes (Func::bit)
    pub summary: u32,
    /// attributes not known here, kept
    pub attrs: Vec<(String, String)>,
}

impl Field {
    /// a shown column of a property with a heading
    pub fn new(id: u32, property: Property, display_name: &str) -> Field {
        Field { id, property, display_name: String::from(display_name), display: true, order: id, data_type: 0, summary: 0, attrs: Vec::new() }
    }

    /// the heading: DisplayName, else the property's name
    pub fn heading(&self) -> &str {
        if self.display_name.is_empty() { self.property.key() } else { &self.display_name }
    }
}

/// a condition of a criterion (Condition; codes assumed, see the file head)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Condition {
    /// =
    Eq,
    /// <>
    Ne,
    /// >
    Gt,
    /// >=
    Ge,
    /// <
    Lt,
    /// <=
    Le,
    /// the shape has the property
    Exists,
    /// the shape does not have it
    NotExists,
    /// a code not known here (the criterion holds for no shape)
    Other(u32),
}

/// the conditions with their codes (one table: the assumption lives here)
pub const CONDITIONS: [(Condition, u32); 8] = [
    (Condition::Eq, 1),
    (Condition::Ne, 2),
    (Condition::Gt, 3),
    (Condition::Ge, 4),
    (Condition::Lt, 5),
    (Condition::Le, 6),
    (Condition::NotExists, 7),
    (Condition::Exists, 8),
];

impl Condition {
    /// the code of Condition
    pub fn code(self) -> u32 {
        match self {
            Condition::Other(n) => n,
            c => CONDITIONS.iter().find(|x| x.0 == c).map_or(0, |x| x.1),
        }
    }

    /// the condition of a code
    pub fn of(code: u32) -> Condition {
        CONDITIONS.iter().find(|x| x.1 == code).map_or(Condition::Other(code), |x| x.0)
    }

    /// whether it compares with a value (Exists and NotExists do not)
    pub fn has_value(self) -> bool {
        !matches!(self, Condition::Exists | Condition::NotExists)
    }
}

/// a criterion: which shapes the report takes (VisioRptDefFilter)
#[derive(Clone, PartialEq, Debug)]
pub struct Filter {
    /// ID
    pub id: u32,
    /// the property compared (PropertyType with Property)
    pub property: Property,
    /// Condition
    pub condition: Condition,
    /// Value: what it compares with
    pub value: String,
    /// Unit: the value's unit code ("MM"; "" none)
    pub unit: String,
    /// PropertyDataType (data::kind)
    pub data_type: u16,
    /// AndOr: 1 and (Visio's criteria are all "and"), 0 or with the ones before
    pub and: bool,
    /// attributes not known here, kept
    pub attrs: Vec<(String, String)>,
}

impl Filter {
    /// a criterion "property condition value"
    pub fn new(id: u32, property: Property, condition: Condition, value: &str) -> Filter {
        Filter { id, property, condition, value: String::from(value), unit: String::new(), data_type: 0, and: true, attrs: Vec::new() }
    }
}

/// grouping and the summaries of the whole (VisioRptDefGroup)
#[derive(Clone, PartialEq, Debug)]
pub struct Grouping {
    /// ID
    pub id: u32,
    /// GroupingField: the ID of the field the rows are grouped by (0: none)
    pub field: u32,
    /// ItemDisplay: the shapes' rows are shown (else only the summaries)
    pub show_items: bool,
    /// GrandSummary: summaries over every row at the end
    pub grand_summary: bool,
    /// ExcludeDuplicates: a row the same as one before is left out
    pub exclude_duplicates: bool,
    /// attributes not known here, kept
    pub attrs: Vec<(String, String)>,
}

impl Default for Grouping {
    fn default() -> Grouping {
        Grouping { id: 1, field: 0, show_items: true, grand_summary: false, exclude_duplicates: false, attrs: Vec::new() }
    }
}

/// a sort key (VisioRptDefSort)
#[derive(Clone, PartialEq, Debug)]
pub struct Sort {
    /// ID
    pub id: u32,
    /// SortField: the field's ID
    pub field: u32,
    /// Ascending
    pub ascending: bool,
    /// CaseSensitive
    pub case_sensitive: bool,
    /// attributes not known here, kept
    pub attrs: Vec<(String, String)>,
}

impl Sort {
    /// a sort key on a field
    pub fn new(id: u32, field: u32, ascending: bool) -> Sort {
        Sort { id, field, ascending, case_sensitive: false, attrs: Vec::new() }
    }
}

/// a report definition (VisioReportDefinition; a .vrd file holds the same)
#[derive(Clone, PartialEq, Debug)]
pub struct ReportDef {
    /// Name: what the list of reports shows
    pub name: String,
    /// Title: the heading of the result ("" -- the name)
    pub title: String,
    /// Description
    pub description: String,
    /// SelectDrawingDataMethod
    pub scope: Scope,
    /// Precision: decimals of numbers
    pub precision: u32,
    /// ShowUnits: lengths, angles and durations with their unit
    pub show_units: bool,
    /// CaseSensitive: criteria compare texts with case
    pub case_sensitive: bool,
    /// UserCells: the wizard offers user cells (Visio's "Show all properties")
    pub user_cells: bool,
    /// HiddenProperties: the wizard offers hidden shape data
    pub hidden_properties: bool,
    /// AdvancedFilter: criteria were given (Visio's dialog Erweitert)
    pub advanced_filter: bool,
    /// FilterExpression as read (written back unchanged)
    pub filter_expression: String,
    /// ReportExists
    pub exists: bool,
    /// Version ("3")
    pub version: String,
    /// the columns in file order
    pub fields: Vec<Field>,
    /// the criteria
    pub filters: Vec<Filter>,
    /// grouping and summaries (None: the file has no group element)
    pub grouping: Option<Grouping>,
    /// the sort keys, the first one first
    pub sorts: Vec<Sort>,
    /// root attributes not known here, kept
    pub attrs: Vec<(String, String)>,
    /// child elements not known here, as XML text, written back after the known ones
    pub extra: Vec<String>,
}

impl ReportDef {
    /// an empty definition named `name` (every page, two decimals, units shown)
    pub fn new(name: &str) -> ReportDef {
        ReportDef {
            name: String::from(name),
            title: String::from(name),
            description: String::new(),
            scope: Scope::AllPages,
            precision: 2,
            show_units: true,
            case_sensitive: false,
            user_cells: false,
            hidden_properties: false,
            advanced_filter: false,
            filter_expression: String::new(),
            exists: true,
            version: String::from("3"),
            fields: Vec::new(),
            filters: Vec::new(),
            grouping: Some(Grouping::default()),
            sorts: Vec::new(),
            attrs: Vec::new(),
            extra: Vec::new(),
        }
    }

    /// a field by ID
    pub fn field(&self, id: u32) -> Option<&Field> {
        self.fields.iter().find(|f| f.id == id)
    }

    /// the shown fields in their display order (DisplayOrder, then file order)
    pub fn columns(&self) -> Vec<&Field> {
        let mut v: Vec<&Field> = self.fields.iter().filter(|f| f.display).collect();
        v.sort_by_key(|f| f.order);
        v
    }

    /// the title shown: Title, else Name
    pub fn heading(&self) -> &str {
        if self.title.trim().is_empty() { &self.name } else { &self.title }
    }

    /// the next free field ID
    pub fn next_field_id(&self) -> u32 {
        self.fields.iter().map(|f| f.id).max().unwrap_or(0) + 1
    }
}

#[cfg(test)]
mod tests;
