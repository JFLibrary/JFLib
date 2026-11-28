// parse -- the universal formula syntax of a ShapeSheet cell into a small tree (FX-01)
//
//   parse("IF(Width>10 mm,Height*0.5,1 in)") -> Ok(Parsed) / Err(ParseError { pos, kind })
//   p.refs()                 the cell references as written (CellRef: sheet part + cell part)
//   p.functions()            the functions it calls (Func; Func::Unknown keeps the name)
//
// What the syntax has ([MS-VSDX] universal syntax, as the files write it):
// * numbers with an optional unit right after them ("180DEG", "0.25DL", "10 mm", "1.5E-3IN");
//   the value is kept in internal units (inches, radians, days), so "10 mm" is 0.3937...;
//   "60%" is 0.6 (% is a postfix operator, also after a bracket)
// * strings in double quotes, "" inside is one quote; TRUE and FALSE; error values (#REF! ...)
// * operators, loosest first ([MS-VSDX] 2.5.2): = <>, then < <= > >=, & (text), + -, * /, ^,
//   unary - and + (so -2^2 is 4); all left to right; % after a number ("60%" is 0.6; the
//   parser also takes it after a bracket)
// * references: Width, Geometry1.X2, Geometry1.NoFill, Char.Size, Char.Size[2], Para.HorzAlign,
//   Prop.Name, Prop.Name.Format, User.Name, User.Name.Prompt, Scratch.X1, Controls.Row_1,
//   Controls.Row_1.Y, Connections.X1, Connections.Name.Y, Actions.Row_1.Action; with a sheet in
//   front: Sheet.5!Width, ThePage!PageScale, TheDoc!DocLangID, Pages[Page-1]!Sheet.3!Width,
//   Masters[Name]!Width, Masters[Name]!Sheet.5!Width, Styles[Normal]!LineColor,
//   'Some name'!Width and Name.7!Width (a shape by its NameU)
// * function calls, also with a sheet in front (Sheet.5!SheetRef()); an empty argument is
//   allowed (SETATREF(User.X,,1)); unknown names parse as Func::Unknown
//
// Errors carry the byte position where the parser stopped.  Nothing here needs a document: the
// engine binds the names to a document's string table once per formula text.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::cell::CellError;
use crate::units::Unit;

/// why a formula did not parse
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ParseErrorKind {
    /// a character that starts nothing
    BadChar,
    /// the formula ended in the middle of an expression
    UnexpectedEnd,
    /// a token where it does not fit
    Unexpected,
    /// a "(" or "[" without its closing bracket
    Unclosed,
    /// a string without its closing quote
    UnclosedString,
    /// a number that does not parse
    BadNumber,
    /// a reference that does not parse (Sheet.x!, Geometry.X1 ...)
    BadReference,
    /// text after the end of the expression
    Trailing,
    /// the formula nests too deep or is too long
    TooComplex,
}

/// a formula that did not parse, with the byte position where it stopped
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct ParseError {
    /// byte offset into the formula text
    pub pos: usize,
    /// what went wrong
    pub kind: ParseErrorKind,
}

/// binary operators
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum BinOp {
    /// +
    Add,
    /// -
    Sub,
    /// *
    Mul,
    /// /
    Div,
    /// ^
    Pow,
    /// & (text concatenation)
    Concat,
    /// =
    Eq,
    /// <>
    Ne,
    /// <
    Lt,
    /// >
    Gt,
    /// <=
    Le,
    /// >=
    Ge,
}

macro_rules! funcs {
    ($($id:ident = $s:literal),* $(,)?) => {
        /// the functions the engine knows by name; Unknown(i) keeps any other name (index into
        /// Parsed::names)
        #[allow(clippy::upper_case_acronyms)]
        #[derive(Copy, Clone, PartialEq, Eq, Debug)]
        pub enum Func {
            $(#[doc = $s] $id,)*
            /// a name the engine does not know
            Unknown(u32),
        }
        const FUNC_NAMES: &[(&str, Func)] = &[$(($s, Func::$id)),*];
    };
}

funcs! {
    // math and logic (FX-06)
    IfError = "IFERROR", Abs = "ABS", And = "AND", Or = "OR", Not = "NOT", If = "IF", Min = "MIN", Max = "MAX", Sum = "SUM",
    Sqrt = "SQRT", Pow = "POW", Exp = "EXP", Ln = "LN", Log10 = "LOG10", Sin = "SIN", Cos = "COS",
    Tan = "TAN", Asin = "ASIN", Acos = "ACOS", Atan = "ATAN", Atan2 = "ATAN2", Sinh = "SINH",
    Cosh = "COSH", Tanh = "TANH", Pi = "PI", Round = "ROUND", Int = "INT", IntUp = "INTUP",
    Trunc = "TRUNC", Floor = "FLOOR", Ceiling = "CEILING", Modulus = "MODULUS", Sign = "SIGN",
    Magnitude = "MAGNITUDE", Ang360 = "ANG360", Rad = "RAD", Deg = "DEG", BitAnd = "BITAND",
    BitOr = "BITOR", BitXor = "BITXOR", BitNot = "BITNOT", Index = "INDEX", Lookup = "LOOKUP",
    Rand = "RAND", True = "TRUE", False = "FALSE",
    // geometry (FX-07)
    Pnt = "PNT", PntX = "PNTX", PntY = "PNTY", Par = "PAR", Loc = "LOC", Nurbs = "NURBS",
    Polyline = "POLYLINE", Bound = "BOUND", AngleToLoc = "ANGLETOLOC", AngleToPar = "ANGLETOPAR",
    LocToLoc = "LOCTOLOC", LocToPar = "LOCTOPAR",
    // (JD21) the meeting point of two lines (an end glued to a guide: Visio writes
    // INTERSECTX/Y(guide's pin and angle, the end's own point, the guide's angle + 90 deg))
    IntersectX = "INTERSECTX", IntersectY = "INTERSECTY",
    // colour and theme (FX-08)
    Rgb = "RGB", Hsl = "HSL", Red = "RED", Green = "GREEN", Blue = "BLUE", Hue = "HUE", Sat = "SAT",
    Lum = "LUM", HueDiff = "HUEDIFF", SatDiff = "SATDIFF", LumDiff = "LUMDIFF", Tint = "TINT",
    Shade = "SHADE", Tone = "TONE", Blend = "BLEND", MsoTint = "MSOTINT", MsoShade = "MSOSHADE",
    ThemeVal = "THEMEVAL", ThemeProp = "THEMEPROP", ThemeRestore = "THEMERESTORE",
    IsThemed = "ISTHEMED", CellIsThemed = "CELLISTHEMED", ThemeGuard = "THEMEGUARD",
    // text (FX-09)
    ShapeText = "SHAPETEXT", TextWidth = "TEXTWIDTH", TextHeight = "TEXTHEIGHT", Font = "FONT",
    StrSame = "STRSAME", StrSameEx = "STRSAMEEX", Len = "LEN", Left = "LEFT", Right = "RIGHT",
    Mid = "MID", Find = "FIND", Substitute = "SUBSTITUTE", Replace = "REPLACE", Rept = "REPT",
    Lower = "LOWER", Upper = "UPPER", Trim = "TRIM", Char = "CHAR", Format = "FORMAT",
    FormatEx = "FORMATEX", EvalText = "EVALTEXT",
    // document and shape information (FX-11)
    MasterName = "MASTERNAME", PageName = "PAGENAME", PageNumber = "PAGENUMBER",
    PageCount = "PAGECOUNT", Id = "ID", Name = "NAME", Type = "TYPE", TypeDesc = "TYPEDESC",
    Data1 = "DATA1", Data2 = "DATA2", Data3 = "DATA3", Is1D = "IS1D", UserUi = "USERUI", Ref = "REF",
    Arg = "ARG", Na = "NA", IsErr = "ISERR", IsError = "ISERROR", IsErrNa = "ISERRNA", IsErrValue = "ISERRVALUE",
    DefaultEvent = "DEFAULTEVENT",
    // document information of text fields (TX-06, field.rs): the clock, the file, the
    // document's properties
    Now = "NOW", DocCreation = "DOCCREATION", DocLastSave = "DOCLASTSAVE", DocLastPrint = "DOCLASTPRINT",
    DocLastEdit = "DOCLASTEDIT", Creator = "CREATOR", Description = "DESCRIPTION", Directory = "DIRECTORY",
    FileName = "FILENAME", Keywords = "KEYWORDS", Subject = "SUBJECT", Title = "TITLE", Manager = "MANAGER",
    Company = "COMPANY", Category = "CATEGORY", HyperlinkBase = "HYPERLINKBASE",
    // protection and side effects (FX-04)
    Guard = "GUARD", SetF = "SETF", SetAtRef = "SETATREF", SetAtRefExpr = "SETATREFEXPR",
    SetAtRefEval = "SETATREFEVAL", DependsOn = "DEPENDSON", GetRef = "GETREF", SheetRef = "SHEETREF",
    // actions (FX-14, FX-15)
    OpenTextWin = "OPENTEXTWIN", OpenSheetWin = "OPENSHEETWIN", OpenGroupWin = "OPENGROUPWIN",
    GotoPage = "GOTOPAGE", Hyperlink = "HYPERLINK", PlaySound = "PLAYSOUND", DoCmd = "DOCMD",
    RunAddon = "RUNADDON", RunAddonWArgs = "RUNADDONWARGS", RunMacro = "RUNMACRO",
    CallThis = "CALLTHIS",
    // structure: containers, lists, callouts (FX-12, sfuncs.rs)
    ContainerCount = "CONTAINERCOUNT", ContainerSheetRef = "CONTAINERSHEETREF",
    ContainerMemberCount = "CONTAINERMEMBERCOUNT", ListMemberCount = "LISTMEMBERCOUNT",
    ListOrder = "LISTORDER", ListSheetRef = "LISTSHEETREF", CalloutCount = "CALLOUTCOUNT",
    CalloutTargetRef = "CALLOUTTARGETREF", HasCategory = "HASCATEGORY",
    // Visio's own helpers in glue and trigger cells
    XfTrigger = "_XFTRIGGER", WalkGlue = "_WALKGLUE",
}

// older spellings Visio still reads
const FUNC_ALIASES: &[(&str, Func)] =
    &[("_DEFAULTEVENT", Func::DefaultEvent), ("_OPENTEXTWIN", Func::OpenTextWin), ("_RUNADDON", Func::RunAddon), ("_OPENSHEETWIN", Func::OpenSheetWin)];

impl Func {
    /// the function of a name (any case); None for names the engine does not know
    pub fn by_name(name: &str) -> Option<Func> {
        FUNC_NAMES.iter().chain(FUNC_ALIASES).find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|&(_, f)| f)
    }

    /// the universal name (None for Unknown: Parsed::name has it)
    pub fn name(self) -> Option<&'static str> {
        FUNC_NAMES.iter().find(|(_, f)| *f == self).map(|&(n, _)| n)
    }
}

/// the sheet part of a reference (in front of "!")
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SheetName {
    /// none: the sheet the formula belongs to
    Local,
    /// the sheet a function gives (CONTAINERSHEETREF(1,"Swimlane")!User.visHeadingText): the
    /// node of the call
    Call(u32),
    /// Sheet.5!: the shape with this ID on the same page or in the same master
    Id(u32),
    /// a shape by its NameU ("Name.7!", "'My name'!")
    Named(Box<str>),
    /// ThePage!: the page sheet (in a master: the master's page sheet)
    ThePage,
    /// TheDoc!: the document sheet
    TheDoc,
    /// Pages[name]! with an optional Sheet.N! after it
    Page(Box<str>, Option<u32>),
    /// Masters[name]! (the master's page sheet) with an optional Sheet.N! after it
    Master(Box<str>, Option<u32>),
    /// Styles[name]!
    Style(Box<str>),
}

/// the cell part of a reference
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CellPart {
    /// no cell: the sheet itself (Sheet.5!SheetRef())
    None,
    /// a single cell: Width, PinX, EventXFMod, TheText
    Single(Box<str>),
    /// a cell of a section itself: Geometry1.NoFill (section index 0-based)
    Section {
        /// the section name as the file writes it (Geometry)
        section: &'static str,
        /// the section index (Geometry1 is 0)
        index: u32,
        /// the cell name
        cell: Box<str>,
    },
    /// a cell of an indexed row: Geometry1.X2, Connections.X1, Char.Size, Scratch.A1
    Indexed {
        /// the section name as the file writes it
        section: &'static str,
        /// the section index
        index: u32,
        /// the row's IX in the file
        row: u32,
        /// the cell name
        cell: Box<str>,
    },
    /// a cell of a named row: User.Name, Prop.Name.Format, Controls.Row_1.Y
    Named {
        /// the section name as the file writes it
        section: &'static str,
        /// the row name (N)
        row: Box<str>,
        /// the cell name
        cell: Box<str>,
    },
    /// a dotted name of a section the engine does not know, as written
    Unknown(Box<str>),
}

/// a reference as written: sheet part and cell part
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CellRef {
    /// the sheet
    pub sheet: SheetName,
    /// the cell
    pub cell: CellPart,
}

/// one node of the tree (Parsed keeps them in one vector, children by index)
#[derive(Copy, Clone, PartialEq, Debug)]
pub(crate) enum Node {
    Num(f64),
    Str(u32),
    Bool(bool),
    Err(CellError),
    Missing,
    Ref(u32),
    Neg(u32),
    Pct(u32),
    Bin(BinOp, u32, u32),
    // function, first argument in args, number of arguments, sheet reference (u32::MAX: none)
    Call(Func, u32, u32, u32),
}

/// a parsed formula
#[derive(Clone, Debug)]
pub struct Parsed {
    pub(crate) nodes: Vec<Node>,
    pub(crate) args: Vec<u32>,
    pub(crate) refs: Vec<CellRef>,
    pub(crate) strs: Vec<Box<str>>,
    pub(crate) names: Vec<Box<str>>,
    pub(crate) root: u32,
    // the references bound to a document's names (the engine fills it; empty: not bound)
    pub(crate) addrs: Vec<Option<crate::sheet::CellAddr>>,
}

impl Parsed {
    /// the references in the order they are written
    pub fn refs(&self) -> &[CellRef] {
        &self.refs
    }

    /// the functions called, in the order they are written (with repeats)
    pub fn functions(&self) -> impl Iterator<Item = Func> + '_ {
        self.nodes.iter().filter_map(|n| match *n {
            Node::Call(f, ..) => Some(f),
            _ => None,
        })
    }

    /// the name of a function call (also of an unknown one)
    pub fn func_name(&self, f: Func) -> &str {
        match f {
            Func::Unknown(i) => self.names.get(i as usize).map_or("", |s| s),
            f => f.name().unwrap_or(""),
        }
    }

    /// the names of the unknown functions it calls
    pub fn unknown(&self) -> impl Iterator<Item = &str> + '_ {
        self.functions().filter_map(move |f| match f {
            Func::Unknown(i) => Some(self.names.get(i as usize).map_or("", |s| &**s)),
            _ => None,
        })
    }

    /// whether the whole formula is a call of `f` (GUARD(...), THEMEGUARD(...), SETATREF(...))
    pub fn top_is(&self, f: Func) -> bool {
        matches!(self.nodes.get(self.root as usize), Some(Node::Call(g, ..)) if *g == f)
    }

    /// whether the formula calls `f` anywhere
    pub fn calls(&self, f: Func) -> bool {
        self.functions().any(|g| g == f)
    }

    /// whether the formula is a plain number (with or without unit), and which
    pub fn constant(&self) -> Option<f64> {
        match self.nodes.get(self.root as usize)? {
            Node::Num(x) => Some(*x),
            Node::Neg(i) => match self.nodes.get(*i as usize)? {
                Node::Num(x) => Some(-x),
                _ => None,
            },
            _ => None,
        }
    }
}

// ---- sections in references

#[derive(Copy, Clone)]
enum SecKind {
    // Geometry<n>.<Cell><row> (rows as the file numbers them, from 1), Geometry<n>.<cell>
    Geometry,
    // <Sec>.<Cell><n>: row IX n - 1 (Connections, Scratch); a named row: <Sec>.<Row>[.<Cell>]
    Numbered(&'static str),
    // <Sec>.<Cell>[n]: row IX n - 1, without [n] row 0 (Char, Para, Tabs, Fields ...)
    Bracket,
    // <Sec>.<Row>[.<Cell>] with this default cell
    Named(&'static str),
}

const SECTIONS: &[(&str, &str, SecKind)] = &[
    ("Geometry", "Geometry", SecKind::Geometry),
    ("Connections", "Connection", SecKind::Numbered("X")),
    ("Scratch", "Scratch", SecKind::Numbered("X")),
    ("Char", "Character", SecKind::Bracket),
    ("Para", "Paragraph", SecKind::Bracket),
    ("Tabs", "Tabs", SecKind::Bracket),
    ("Fields", "Field", SecKind::Bracket),
    ("FillGradient", "FillGradient", SecKind::Bracket),
    ("LineGradient", "LineGradient", SecKind::Bracket),
    ("FillGradientStops", "FillGradient", SecKind::Bracket),
    ("LineGradientStops", "LineGradient", SecKind::Bracket),
    ("Layers", "Layer", SecKind::Bracket),
    ("Controls", "Control", SecKind::Named("X")),
    ("User", "User", SecKind::Named("Value")),
    ("Prop", "Property", SecKind::Named("Value")),
    ("Actions", "Actions", SecKind::Named("Menu")),
    ("Hyperlink", "Hyperlink", SecKind::Named("Description")),
    ("SmartTags", "ActionTag", SecKind::Named("X")),
];

// split "X12" into ("X", 12)
fn split_number(s: &str) -> Option<(&str, u32)> {
    let cut = s.trim_end_matches(|c: char| c.is_ascii_digit());
    if cut.is_empty() || cut.len() == s.len() {
        return None;
    }
    s[cut.len()..].parse().ok().map(|n| (cut, n))
}

/// the cell part of a dotted name ("Geometry1.X2", "User.Name.Prompt", "Width") with an
/// optional [n] after it; None when the name does not fit a section's form
pub fn cell_part(name: &str, bracket: Option<u32>) -> Option<CellPart> {
    let Some((head, rest)) = name.split_once('.') else {
        return match bracket {
            None => Some(CellPart::Single(name.into())),
            Some(_) => None,
        };
    };
    // Geometry<n> carries its index in the head
    let (head_name, head_index) = match split_number(head) {
        Some((h, n)) if h == "Geometry" => (h, Some(n)),
        _ => (head, None),
    };
    let Some(&(_, section, kind)) = SECTIONS.iter().find(|(p, _, _)| *p == head_name) else {
        // a section the engine does not know (Reviewer, Annotation ...): kept as written
        return Some(CellPart::Unknown(name.into()));
    };
    match kind {
        SecKind::Geometry => {
            let index = head_index?.checked_sub(1)?;
            if bracket.is_some() || rest.contains('.') {
                return None;
            }
            match split_number(rest) {
                Some((cell, row)) => Some(CellPart::Indexed { section, index, row, cell: cell.into() }),
                None => Some(CellPart::Section { section, index, cell: rest.into() }),
            }
        }
        _ if head_index.is_some() => None,
        SecKind::Numbered(default) => {
            if bracket.is_some() {
                return None;
            }
            match split_number(rest) {
                Some((cell, row)) if !rest.contains('.') => {
                    Some(CellPart::Indexed { section, index: 0, row: row.checked_sub(1)?, cell: cell.into() })
                }
                _ => {
                    let (row, cell) = rest.split_once('.').unwrap_or((rest, default));
                    Some(CellPart::Named { section, row: row.into(), cell: cell.into() })
                }
            }
        }
        SecKind::Bracket => {
            if rest.contains('.') {
                return None;
            }
            let row = match bracket {
                Some(n) => n.checked_sub(1)?,
                None => 0,
            };
            Some(CellPart::Indexed { section, index: 0, row, cell: rest.into() })
        }
        SecKind::Named(default) => {
            if bracket.is_some() {
                return None;
            }
            let (row, cell) = rest.split_once('.').unwrap_or((rest, default));
            if row.is_empty() || cell.is_empty() || cell.contains('.') {
                return None;
            }
            Some(CellPart::Named { section, row: row.into(), cell: cell.into() })
        }
    }
}

// ---- lexer

#[derive(Clone, PartialEq, Debug)]
enum Tok {
    Num(f64),
    Str(String),
    // a name: letters, digits, '_', '.', and what follows it directly ('!' or '[n]')
    Name(String),
    Err(CellError),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
    Bang,
    LBracket,
    RBracket,
    // 'quoted name'
    Quoted(String),
    End,
}

const MAX_NODES: usize = 1 << 20;
const MAX_DEPTH: u32 = 200;
// the height of the tree: MAX_DEPTH counts nesting, but a chain like 1+1+1+... grows the
// tree by one per operator without nesting, and evaluation recurses as deep as the tree is
const MAX_HEIGHT: u32 = 512;

struct Lexer<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
}

const UNITS: &[(&str, Unit)] = &[
    ("IN", Unit::Inches), ("IN.", Unit::Inches), ("INCH", Unit::Inches), ("INCHES", Unit::Inches),
    ("FT", Unit::Feet), ("FT.", Unit::Feet), ("FEET", Unit::Feet), ("YD", Unit::Yards), ("MI", Unit::Miles),
    ("NM", Unit::NauticalMiles), ("MM", Unit::Millimeters), ("CM", Unit::Centimeters), ("M", Unit::Meters),
    ("KM", Unit::Kilometers), ("PT", Unit::Points), ("P", Unit::Picas), ("PICA", Unit::Picas), ("D", Unit::Didots),
    ("C", Unit::Ciceros), ("DL", Unit::DrawingLength), ("DP", Unit::PageLength), ("DT", Unit::TextLength),
    ("DEG", Unit::Degrees), ("RAD", Unit::Radians), ("DA", Unit::DefaultAngle), ("ED", Unit::Days),
    ("EH", Unit::Hours), ("EM", Unit::Minutes), ("ES", Unit::Seconds), ("EW", Unit::Weeks), ("DE", Unit::DefaultDuration),
    ("AC", Unit::Acres), ("HA", Unit::Hectares), ("CY", Unit::Currency), ("NUM", Unit::Number),
];

/// the unit of a code as written after a number ("MM", "mm", "Deg"); None: not a unit
pub fn unit_code(code: &str) -> Option<Unit> {
    UNITS.iter().find(|(c, _)| c.eq_ignore_ascii_case(code)).map(|&(_, u)| u)
}

fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.'
}

impl<'a> Lexer<'a> {
    fn peek_char(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek_char() {
            if c.is_whitespace() {
                self.i += c.len_utf8();
            } else {
                break;
            }
        }
    }

    fn err(&self, kind: ParseErrorKind) -> ParseError {
        ParseError { pos: self.i, kind }
    }

    // the next token and where it starts
    fn next(&mut self) -> Result<(usize, Tok), ParseError> {
        self.skip_ws();
        let start = self.i;
        let Some(c) = self.peek_char() else { return Ok((start, Tok::End)) };
        let tok = match c {
            '0'..='9' | '.' => self.number()?,
            '"' => self.string()?,
            '\'' => {
                self.i += 1;
                let from = self.i;
                let Some(n) = self.s[from..].find('\'') else {
                    return Err(ParseError { pos: start, kind: ParseErrorKind::UnclosedString });
                };
                self.i = from + n + 1;
                Tok::Quoted(String::from(&self.s[from..from + n]))
            }
            '#' => self.error_value()?,
            '(' => self.one(Tok::LParen),
            ')' => self.one(Tok::RParen),
            ',' => self.one(Tok::Comma),
            '!' => self.one(Tok::Bang),
            '[' => self.one(Tok::LBracket),
            ']' => self.one(Tok::RBracket),
            '+' | '-' | '*' | '/' | '^' | '&' | '=' | '%' => {
                self.i += 1;
                Tok::Op(match c {
                    '+' => "+",
                    '-' => "-",
                    '*' => "*",
                    '/' => "/",
                    '^' => "^",
                    '&' => "&",
                    '=' => "=",
                    _ => "%",
                })
            }
            '<' | '>' => {
                self.i += 1;
                let two = self.b.get(self.i).copied();
                Tok::Op(match (c, two) {
                    ('<', Some(b'>')) => {
                        self.i += 1;
                        "<>"
                    }
                    ('<', Some(b'=')) => {
                        self.i += 1;
                        "<="
                    }
                    ('>', Some(b'=')) => {
                        self.i += 1;
                        ">="
                    }
                    ('<', _) => "<",
                    _ => ">",
                })
            }
            c if is_name_start(c) => {
                while let Some(c) = self.peek_char() {
                    if is_name_char(c) {
                        self.i += c.len_utf8();
                    } else {
                        break;
                    }
                }
                Tok::Name(String::from(&self.s[start..self.i]))
            }
            _ => return Err(self.err(ParseErrorKind::BadChar)),
        };
        Ok((start, tok))
    }

    fn one(&mut self, t: Tok) -> Tok {
        self.i += 1;
        t
    }

    fn digits(&mut self) -> usize {
        let from = self.i;
        while self.b.get(self.i).is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        self.i - from
    }

    fn number(&mut self) -> Result<Tok, ParseError> {
        let start = self.i;
        let mut n = self.digits();
        if self.b.get(self.i) == Some(&b'.') {
            self.i += 1;
            n += self.digits();
        }
        if n == 0 {
            return Err(ParseError { pos: start, kind: ParseErrorKind::BadNumber });
        }
        // exponent: E, optional sign, digits (else the E belongs to a unit or name)
        if matches!(self.b.get(self.i), Some(b'E' | b'e')) {
            let save = self.i;
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if self.digits() == 0 {
                self.i = save;
            }
        }
        let x: f64 = self.s[start..self.i].parse().map_err(|_| ParseError { pos: start, kind: ParseErrorKind::BadNumber })?;
        let mut total = x;
        // unit right after it or after spaces; feet-and-inches pairs add up (1FT 6IN)
        if let Some(u) = self.unit() {
            total = u.to_internal(x);
            loop {
                let save = self.i;
                self.skip_ws();
                if !self.b.get(self.i).is_some_and(|c| c.is_ascii_digit()) {
                    self.i = save;
                    break;
                }
                let s2 = self.i;
                self.digits();
                if self.b.get(self.i) == Some(&b'.') {
                    self.i += 1;
                    self.digits();
                }
                let y: f64 = match self.s[s2..self.i].parse() {
                    Ok(y) => y,
                    Err(_) => {
                        self.i = save;
                        break;
                    }
                };
                match self.unit() {
                    Some(u2) if u2.dimension() == u.dimension() => total += u2.to_internal(y),
                    _ => {
                        self.i = save;
                        break;
                    }
                }
            }
        }
        Ok(Tok::Num(total))
    }

    // a unit code after a number: letters (with an optional '.') not followed by '(' / '!' /
    // another name character
    fn unit(&mut self) -> Option<Unit> {
        let save = self.i;
        self.skip_ws();
        let from = self.i;
        while self.b.get(self.i).is_some_and(|c| c.is_ascii_alphabetic()) {
            self.i += 1;
        }
        if self.b.get(self.i) == Some(&b'.') && self.b.get(self.i + 1).is_none_or(|c| !c.is_ascii_alphanumeric()) {
            self.i += 1;
        }
        let code = &self.s[from..self.i];
        let next = self.peek_char();
        let ok = !code.is_empty() && next.is_none_or(|c| !(is_name_char(c) || c == '(' || c == '!' || c == '['));
        match unit_code(code) {
            Some(u) if ok => Some(u),
            _ => {
                self.i = save;
                None
            }
        }
    }

    fn string(&mut self) -> Result<Tok, ParseError> {
        let start = self.i;
        self.i += 1;
        let mut out = String::new();
        loop {
            let Some(n) = self.s[self.i..].find('"') else {
                return Err(ParseError { pos: start, kind: ParseErrorKind::UnclosedString });
            };
            out.push_str(&self.s[self.i..self.i + n]);
            self.i += n + 1;
            if self.b.get(self.i) == Some(&b'"') {
                out.push('"');
                self.i += 1;
            } else {
                return Ok(Tok::Str(out));
            }
        }
    }

    fn error_value(&mut self) -> Result<Tok, ParseError> {
        const ERRS: &[(&str, CellError)] = &[
            ("#DIV/0!", CellError::DivZero), ("#VALUE!", CellError::Value), ("#REF!", CellError::Ref),
            ("#NAME?", CellError::Name), ("#NUM!", CellError::Num), ("#N/A", CellError::NotAvailable),
            ("#DIM!", CellError::Dim),
        ];
        for (t, e) in ERRS {
            if self.s[self.i..].len() >= t.len() && self.s[self.i..self.i + t.len()].eq_ignore_ascii_case(t) {
                self.i += t.len();
                return Ok(Tok::Err(*e));
            }
        }
        Err(self.err(ParseErrorKind::BadChar))
    }
}

// ---- parser

struct Parser<'a> {
    lex: Lexer<'a>,
    tok: Tok,
    pos: usize,
    out: Parsed,
    depth: u32,
    // the height of each node's subtree (beside out.nodes)
    height: Vec<u32>,
}

/// parse a formula in the universal syntax
pub fn parse(text: &str) -> Result<Parsed, ParseError> {
    let mut lex = Lexer { s: text, b: text.as_bytes(), i: 0 };
    let (pos, tok) = lex.next()?;
    let out = Parsed { nodes: Vec::new(), args: Vec::new(), refs: Vec::new(), strs: Vec::new(), names: Vec::new(), root: 0, addrs: Vec::new() };
    let mut p = Parser { lex, tok, pos, out, depth: 0, height: Vec::new() };
    if p.tok == Tok::End {
        return Err(p.err(ParseErrorKind::UnexpectedEnd));
    }
    let root = p.expr(0)?;
    if p.tok != Tok::End {
        return Err(p.err(ParseErrorKind::Trailing));
    }
    p.out.root = root;
    Ok(p.out)
}

// binding powers: (left, right) of infix operators
fn infix(op: &str) -> Option<(u8, u8, BinOp)> {
    Some(match op {
        "=" => (1, 2, BinOp::Eq),
        "<>" => (1, 2, BinOp::Ne),
        "<" => (3, 4, BinOp::Lt),
        ">" => (3, 4, BinOp::Gt),
        "<=" => (3, 4, BinOp::Le),
        ">=" => (3, 4, BinOp::Ge),
        "&" => (5, 6, BinOp::Concat),
        "+" => (7, 8, BinOp::Add),
        "-" => (7, 8, BinOp::Sub),
        "*" => (9, 10, BinOp::Mul),
        "/" => (9, 10, BinOp::Div),
        "^" => (11, 12, BinOp::Pow),
        _ => return None,
    })
}
// unary minus binds tighter than ^ ([MS-VSDX] 2.5.2: -2^2 = 4)
const PREFIX_BP: u8 = 13;
const POSTFIX_BP: u8 = 15;

impl Parser<'_> {
    fn err(&self, kind: ParseErrorKind) -> ParseError {
        ParseError { pos: self.pos, kind }
    }

    fn bump(&mut self) -> Result<Tok, ParseError> {
        let (pos, tok) = self.lex.next()?;
        self.pos = pos;
        Ok(core::mem::replace(&mut self.tok, tok))
    }

    fn push(&mut self, n: Node) -> Result<u32, ParseError> {
        if self.out.nodes.len() >= MAX_NODES {
            return Err(self.err(ParseErrorKind::TooComplex));
        }
        let h = 1 + self.below(&n);
        if h > MAX_HEIGHT {
            return Err(self.err(ParseErrorKind::TooComplex));
        }
        self.out.nodes.push(n);
        self.height.push(h);
        Ok(self.out.nodes.len() as u32 - 1)
    }

    // the height of the highest child of a node about to be pushed (its children are pushed)
    fn below(&self, n: &Node) -> u32 {
        let h = |i: u32| self.height.get(i as usize).copied().unwrap_or(0);
        // a reference whose sheet a call gives (CONTAINERSHEETREF(1)!Width): that call's height
        let by_ref = |r: u32| match self.out.refs.get(r as usize).map(|r| &r.sheet) {
            Some(SheetName::Call(c)) => h(*c),
            _ => 0,
        };
        match *n {
            Node::Neg(x) | Node::Pct(x) => h(x),
            Node::Bin(_, l, r) => h(l).max(h(r)),
            // (the last field: the reference of a sheet in front of the call, u32::MAX: none)
            Node::Call(_, s, k, r) => {
                let args = self.out.args.get(s as usize..(s as usize).saturating_add(k as usize)).unwrap_or(&[]);
                args.iter().map(|&a| h(a)).max().unwrap_or(0).max(by_ref(r))
            }
            Node::Ref(r) => by_ref(r),
            Node::Num(_) | Node::Str(_) | Node::Bool(_) | Node::Err(_) | Node::Missing => 0,
        }
    }

    fn expr(&mut self, min_bp: u8) -> Result<u32, ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.err(ParseErrorKind::TooComplex));
        }
        let mut lhs = self.prefix()?;
        loop {
            match self.tok {
                Tok::Op("%") if POSTFIX_BP >= min_bp => {
                    self.bump()?;
                    lhs = self.push(Node::Pct(lhs))?;
                }
                Tok::Op(op) => {
                    let Some((l, r, bin)) = infix(op) else { break };
                    if l < min_bp {
                        break;
                    }
                    self.bump()?;
                    let rhs = self.expr(r)?;
                    lhs = self.push(Node::Bin(bin, lhs, rhs))?;
                }
                _ => break,
            }
        }
        self.depth -= 1;
        Ok(lhs)
    }

    fn prefix(&mut self) -> Result<u32, ParseError> {
        let at = self.pos;
        match self.bump()? {
            Tok::Num(x) => self.push(Node::Num(x)),
            Tok::Str(s) => {
                self.out.strs.push(s.into());
                let i = self.out.strs.len() as u32 - 1;
                self.push(Node::Str(i))
            }
            Tok::Err(e) => self.push(Node::Err(e)),
            Tok::Op("-") => {
                let e = self.expr(PREFIX_BP)?;
                self.push(Node::Neg(e))
            }
            Tok::Op("+") => self.expr(PREFIX_BP),
            Tok::LParen => {
                let e = self.expr(0)?;
                if self.tok != Tok::RParen {
                    return Err(ParseError { pos: at, kind: ParseErrorKind::Unclosed });
                }
                self.bump()?;
                Ok(e)
            }
            Tok::Name(n) => self.name(at, n),
            Tok::Quoted(n) => {
                if self.tok != Tok::Bang {
                    return Err(ParseError { pos: at, kind: ParseErrorKind::BadReference });
                }
                self.bump()?;
                self.after_sheet(at, SheetName::Named(n.into()))
            }
            Tok::End => Err(self.err(ParseErrorKind::UnexpectedEnd)),
            _ => Err(ParseError { pos: at, kind: ParseErrorKind::Unexpected }),
        }
    }

    // a name: function call, TRUE/FALSE, sheet prefix or cell reference
    fn name(&mut self, at: usize, n: String) -> Result<u32, ParseError> {
        if self.tok == Tok::LParen && !n.contains('.') {
            let c = self.call(at, &n, u32::MAX)?;
            // a function that gives a sheet, then a cell of it
            if self.tok == Tok::Bang {
                self.bump()?;
                return self.after_sheet(at, SheetName::Call(c));
            }
            return Ok(c);
        }
        if self.tok == Tok::Bang {
            self.bump()?;
            let sheet = sheet_name(&n).ok_or(ParseError { pos: at, kind: ParseErrorKind::BadReference })?;
            return self.after_sheet(at, sheet);
        }
        if self.tok == Tok::LBracket {
            // Pages[..]!, Masters[..]!, Styles[..]! or Char.Size[2]
            return self.bracket(at, n);
        }
        if n.eq_ignore_ascii_case("TRUE") {
            return self.push(Node::Bool(true));
        }
        if n.eq_ignore_ascii_case("FALSE") {
            return self.push(Node::Bool(false));
        }
        let cell = cell_part(&n, None).ok_or(ParseError { pos: at, kind: ParseErrorKind::BadReference })?;
        self.reference(SheetName::Local, cell)
    }

    fn reference(&mut self, sheet: SheetName, cell: CellPart) -> Result<u32, ParseError> {
        self.out.refs.push(CellRef { sheet, cell });
        let i = self.out.refs.len() as u32 - 1;
        self.push(Node::Ref(i))
    }

    // [ ... ] after a name
    fn bracket(&mut self, at: usize, n: String) -> Result<u32, ParseError> {
        let bad = ParseError { pos: at, kind: ParseErrorKind::BadReference };
        // the text up to "]" as written
        let from = self.pos + 1;
        let close = self.lex.s[from..].find(']').ok_or(ParseError { pos: at, kind: ParseErrorKind::Unclosed })?;
        let inner = String::from(self.lex.s[from..from + close].trim());
        self.lex.i = from + close + 1;
        self.bump()?; // the token after "]"
        let kind = n.to_ascii_lowercase();
        if matches!(kind.as_str(), "pages" | "masters" | "styles") {
            if self.tok != Tok::Bang {
                return Err(bad);
            }
            self.bump()?;
            // an optional Sheet.N! after it
            let mut sub = None;
            if let Tok::Name(m) = &self.tok {
                if let Some(id) = m.strip_prefix("Sheet.").and_then(|d| d.parse::<u32>().ok()) {
                    let m_at = self.pos;
                    let save_tok = self.tok.clone();
                    let save_i = self.lex.i;
                    self.bump()?;
                    if self.tok == Tok::Bang {
                        self.bump()?;
                        sub = Some(id);
                    } else {
                        // not a sheet: back to the name
                        self.lex.i = save_i;
                        self.tok = save_tok;
                        self.pos = m_at;
                    }
                }
            }
            let name: Box<str> = inner.into();
            let sheet = match kind.as_str() {
                "pages" => SheetName::Page(name, sub),
                "masters" => SheetName::Master(name, sub),
                _ if sub.is_none() => SheetName::Style(name),
                _ => return Err(bad),
            };
            return self.after_sheet(at, sheet);
        }
        let idx: u32 = inner.parse().map_err(|_| bad)?;
        let cell = cell_part(&n, Some(idx)).ok_or(bad)?;
        self.reference(SheetName::Local, cell)
    }

    // after "<sheet>!": a cell or a function call
    fn after_sheet(&mut self, at: usize, sheet: SheetName) -> Result<u32, ParseError> {
        let bad = ParseError { pos: at, kind: ParseErrorKind::BadReference };
        let n_at = self.pos;
        let Tok::Name(n) = self.bump()? else { return Err(bad) };
        if self.tok == Tok::LParen && !n.contains('.') {
            self.out.refs.push(CellRef { sheet, cell: CellPart::None });
            let r = self.out.refs.len() as u32 - 1;
            return self.call(n_at, &n, r);
        }
        let cell = if self.tok == Tok::LBracket {
            let from = self.pos + 1;
            let close = self.lex.s[from..].find(']').ok_or(bad)?;
            let idx: u32 = self.lex.s[from..from + close].trim().parse().map_err(|_| bad)?;
            self.lex.i = from + close + 1;
            self.bump()?;
            cell_part(&n, Some(idx))
        } else {
            cell_part(&n, None)
        };
        let cell = cell.ok_or(bad)?;
        self.reference(sheet, cell)
    }

    fn call(&mut self, at: usize, name: &str, sheet: u32) -> Result<u32, ParseError> {
        let func = match Func::by_name(name) {
            Some(f) => f,
            None => {
                self.out.names.push(name.into());
                Func::Unknown(self.out.names.len() as u32 - 1)
            }
        };
        self.bump()?; // (
        let mut args = Vec::new();
        if self.tok == Tok::RParen {
            self.bump()?;
        } else {
            loop {
                let a = if matches!(self.tok, Tok::Comma | Tok::RParen) { self.push(Node::Missing)? } else { self.expr(0)? };
                args.push(a);
                match self.tok {
                    Tok::Comma => {
                        self.bump()?;
                    }
                    Tok::RParen => {
                        self.bump()?;
                        break;
                    }
                    Tok::End => return Err(ParseError { pos: at, kind: ParseErrorKind::Unclosed }),
                    _ => return Err(self.err(ParseErrorKind::Unexpected)),
                }
            }
        }
        let start = self.out.args.len() as u32;
        let n = args.len() as u32;
        self.out.args.extend(args);
        self.push(Node::Call(func, start, n, sheet))
    }
}

// the sheet named in front of "!"
fn sheet_name(n: &str) -> Option<SheetName> {
    if n.eq_ignore_ascii_case("ThePage") {
        return Some(SheetName::ThePage);
    }
    if n.eq_ignore_ascii_case("TheDoc") {
        return Some(SheetName::TheDoc);
    }
    if let Some(id) = n.strip_prefix("Sheet.") {
        return id.parse().ok().map(SheetName::Id);
    }
    if n.is_empty() {
        return None;
    }
    Some(SheetName::Named(n.into()))
}
