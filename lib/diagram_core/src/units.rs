// units -- the unit codes of a cell (attribute U) and conversion to the internal units
//
//   Unit::parse("MM") / u.code()     the code as the file writes it; Unit::Other keeps codes the
//                                    crate does not know
//   u.dimension()                    Length, Angle, Duration, Date, Area, Number, Text ...
//   u.to_internal(25.4) -> 1.0       a value in the unit -> the value V stores (inches, radians,
//                                    days, square inches); u.from_internal(1.0) -> 25.4
//   convert(x, from, to)             between two units of one dimension
//   Implicit { drawing, page, text, angle, duration }   what DL, DP, DT, DA, DE stand for on a page
//
// The file keeps every value in internal units: lengths in inches, angles in radians, durations
// and dates in days; U only says how the value is shown and typed in (FEATURES.md 1.6, [MS-VSDX]
// Cell_Type).  A text size of 8 pt is V="0.1111" U="PT", an angle of 270 degrees
// V="4.7124" U="DEG".  DL, DP, DT, DA and DE are "the page's drawing, page, text, angle and
// duration unit": without the page they convert like inches, inches, points, radians and days.
// Ciceros and didots use the Didot point of 0.376065 mm (1157 dd = 1238 pt of 1/72.27 in).

use crate::strings::Sym;

/// a unit code of the attribute U
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub enum Unit {
    /// IN: inches
    Inches,
    /// IN_F: inches shown as fractions
    InchesFrac,
    /// FT: feet
    Feet,
    /// F_I: feet and inches
    FeetInches,
    /// YD: yards
    Yards,
    /// MI: miles
    Miles,
    /// MI_F: miles shown as fractions
    MilesFrac,
    /// NM: nautical miles (1852 m)
    NauticalMiles,
    /// MM: millimetres
    Millimeters,
    /// CM: centimetres
    Centimeters,
    /// M: metres
    Meters,
    /// KM: kilometres
    Kilometers,
    /// PT: points (1/72 in)
    Points,
    /// P: picas (12 pt)
    Picas,
    /// P_PT: picas and points
    PicasPoints,
    /// D: didots
    Didots,
    /// C: ciceros (12 didots)
    Ciceros,
    /// C_D: ciceros and didots
    CicerosDidots,
    /// DL: the page's drawing unit
    DrawingLength,
    /// DP: the page's page unit
    PageLength,
    /// DT: the text unit (points)
    TextLength,
    /// DEG: degrees
    Degrees,
    /// AD: degrees, minutes, seconds
    DegreesMinSec,
    /// RAD: radians
    Radians,
    /// DA: the default angle unit
    DefaultAngle,
    /// ED: elapsed days
    Days,
    /// EH: elapsed hours
    Hours,
    /// EM: elapsed minutes
    Minutes,
    /// ES: elapsed seconds
    Seconds,
    /// EW: elapsed weeks
    Weeks,
    /// DE: the default duration unit
    DefaultDuration,
    /// DATE: a date (days)
    Date,
    /// AC: acres
    Acres,
    /// HA: hectares
    Hectares,
    /// PER: per cent (the value is a fraction: 0.5 is 50 %)
    Percent,
    /// CY: currency
    Currency,
    /// BOOL: a truth value
    Bool,
    /// COLOR: a colour (index or RGB)
    Color,
    /// STR: a string
    Str,
    /// GUID: a GUID string
    Guid,
    /// PNT: a two-dimensional point
    Point,
    /// NURBS: the result of a NURBS() formula
    Nurbs,
    /// POLYLINE: the result of a POLYLINE() formula
    Polyline,
    /// NUM: a plain number
    Number,
    /// a code the crate does not know, kept as written
    Other(Sym),
}

/// what a unit measures
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Dimension {
    /// internal unit: inches
    Length,
    /// internal unit: radians
    Angle,
    /// internal unit: days
    Duration,
    /// internal unit: days since 30 December 1899
    Date,
    /// internal unit: square inches
    Area,
    /// a plain number (per cent as a fraction, currency)
    Number,
    /// strings, colours, truth values, points, NURBS and polylines: no conversion
    Typed,
    /// an unknown unit
    Unknown,
}

const MM: f64 = 1.0 / 25.4;
// the Didot point in inches: 1238 / 1157 TeX points of 1/72.27 in
const DIDOT: f64 = 1238.0 / (1157.0 * 72.27);

const CODES: &[(&str, Unit)] = &[
    ("IN", Unit::Inches), ("IN_F", Unit::InchesFrac), ("FT", Unit::Feet), ("F_I", Unit::FeetInches),
    ("YD", Unit::Yards), ("MI", Unit::Miles), ("MI_F", Unit::MilesFrac), ("NM", Unit::NauticalMiles),
    ("MM", Unit::Millimeters), ("CM", Unit::Centimeters), ("M", Unit::Meters), ("KM", Unit::Kilometers),
    ("PT", Unit::Points), ("P", Unit::Picas), ("P_PT", Unit::PicasPoints), ("D", Unit::Didots),
    ("C", Unit::Ciceros), ("C_D", Unit::CicerosDidots), ("DL", Unit::DrawingLength), ("DP", Unit::PageLength),
    ("DT", Unit::TextLength), ("DEG", Unit::Degrees), ("AD", Unit::DegreesMinSec), ("RAD", Unit::Radians),
    ("DA", Unit::DefaultAngle), ("ED", Unit::Days), ("EH", Unit::Hours), ("EM", Unit::Minutes),
    ("ES", Unit::Seconds), ("EW", Unit::Weeks), ("DE", Unit::DefaultDuration), ("DATE", Unit::Date),
    ("AC", Unit::Acres), ("HA", Unit::Hectares), ("PER", Unit::Percent), ("CY", Unit::Currency),
    ("BOOL", Unit::Bool), ("COLOR", Unit::Color), ("STR", Unit::Str), ("GUID", Unit::Guid),
    ("PNT", Unit::Point), ("NURBS", Unit::Nurbs), ("POLYLINE", Unit::Polyline), ("NUM", Unit::Number),
];

impl Unit {
    /// the unit of a known code (None: unknown, keep it as Unit::Other)
    pub fn parse(code: &str) -> Option<Unit> {
        CODES.iter().find(|(c, _)| *c == code).map(|&(_, u)| u)
    }

    /// the code as the file writes it (None for Other: its Sym holds the code)
    pub fn code(self) -> Option<&'static str> {
        CODES.iter().find(|(_, u)| *u == self).map(|&(c, _)| c)
    }

    /// what the unit measures
    pub fn dimension(self) -> Dimension {
        use Unit::*;
        match self {
            Inches | InchesFrac | Feet | FeetInches | Yards | Miles | MilesFrac | NauticalMiles | Millimeters
            | Centimeters | Meters | Kilometers | Points | Picas | PicasPoints | Didots | Ciceros
            | CicerosDidots | DrawingLength | PageLength | TextLength => Dimension::Length,
            Degrees | DegreesMinSec | Radians | DefaultAngle => Dimension::Angle,
            Days | Hours | Minutes | Seconds | Weeks | DefaultDuration => Dimension::Duration,
            Date => Dimension::Date,
            Acres | Hectares => Dimension::Area,
            Percent | Currency | Number => Dimension::Number,
            Bool | Color | Str | Guid | Point | Nurbs | Polyline => Dimension::Typed,
            Other(_) => Dimension::Unknown,
        }
    }

    /// how many internal units one of this unit is (None: the unit has no factor)
    pub fn factor(self) -> Option<f64> {
        use Unit::*;
        Some(match self {
            Inches | InchesFrac | DrawingLength | PageLength => 1.0,
            Feet | FeetInches => 12.0,
            Yards => 36.0,
            Miles | MilesFrac => 63_360.0,
            NauticalMiles => 1_852_000.0 * MM,
            Millimeters => MM,
            Centimeters => 10.0 * MM,
            Meters => 1000.0 * MM,
            Kilometers => 1_000_000.0 * MM,
            Points | TextLength => 1.0 / 72.0,
            Picas | PicasPoints => 1.0 / 6.0,
            Didots => DIDOT,
            Ciceros | CicerosDidots => 12.0 * DIDOT,
            Degrees | DegreesMinSec => core::f64::consts::PI / 180.0,
            Radians | DefaultAngle => 1.0,
            Days | DefaultDuration | Date => 1.0,
            Hours => 1.0 / 24.0,
            Minutes => 1.0 / 1440.0,
            Seconds => 1.0 / 86_400.0,
            Weeks => 7.0,
            Acres => 43_560.0 * 144.0,
            Hectares => 10_000.0 * 1_000_000.0 * MM * MM,
            Percent => 0.01,
            Currency | Number => 1.0,
            Bool | Color | Str | Guid | Point | Nurbs | Polyline | Other(_) => return None,
        })
    }

    /// a value given in this unit -> the internal value V stores (unchanged without a factor)
    pub fn to_internal(self, value: f64) -> f64 {
        self.factor().map_or(value, |f| value * f)
    }

    /// an internal value -> the value in this unit (unchanged without a factor)
    pub fn from_internal(self, internal: f64) -> f64 {
        self.factor().map_or(internal, |f| internal / f)
    }
}

/// `value` in unit `from` -> unit `to`; None when the dimensions differ or a unit has no factor
pub fn convert(value: f64, from: Unit, to: Unit) -> Option<f64> {
    if from.dimension() != to.dimension() {
        return None;
    }
    Some(value * from.factor()? / to.factor()?)
}

/// what the implicit units stand for on a page (its drawing, page, text, angle and duration
/// units); `resolve` replaces DL, DP, DT, DA and DE by them
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Implicit {
    /// DL: the drawing unit (shown lengths of the drawing)
    pub drawing: Unit,
    /// DP: the page unit
    pub page: Unit,
    /// DT: the text unit
    pub text: Unit,
    /// DA: the angle unit
    pub angle: Unit,
    /// DE: the duration unit
    pub duration: Unit,
}

impl Default for Implicit {
    fn default() -> Self {
        Implicit {
            drawing: Unit::Inches,
            page: Unit::Inches,
            text: Unit::Points,
            angle: Unit::Degrees,
            duration: Unit::Days,
        }
    }
}

impl Implicit {
    /// the explicit unit behind `unit` (other units unchanged)
    pub fn resolve(&self, unit: Unit) -> Unit {
        match unit {
            Unit::DrawingLength => self.drawing,
            Unit::PageLength => self.page,
            Unit::TextLength => self.text,
            Unit::DefaultAngle => self.angle,
            Unit::DefaultDuration => self.duration,
            u => u,
        }
    }
}
