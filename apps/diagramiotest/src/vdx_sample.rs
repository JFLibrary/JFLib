// vdx_sample -- self-written files of Visio's XML format of 2003-2010 (JD16): a drawing (.vdx),
// a stencil (.vsx) and a drawing with a bitmap without file header, written by hand after
// Microsoft's public schema reference ("XML for Visio"); everything in them is JFLib's own
//
//   drawing() / stencil() / bitmap()   the files the tests read and the user opens in Visio
//   png() / dib() / icon()             the pictures inside them (built here)
//   b64(bytes)                         base64 as the format holds it
//
// Lengths in inches (the format's internal unit), shown in mm (Unit='MM'); cells in the order
// of the schema, sections in the order Visio writes them, the text last and a group's Shapes
// after it.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// base64 with padding, in lines of 76 characters as Visio writes it
pub fn b64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for (k, c) in b.chunks(3).enumerate() {
        if k > 0 && k % 19 == 0 {
            s.push('\n');
        }
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(A[(n >> 18) as usize & 63] as char);
        s.push(A[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { A[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { A[n as usize & 63] as char } else { '=' });
    }
    s
}

/// a 16 x 16 PNG: four quarters in the NetLab colours
pub fn png() -> Vec<u8> {
    let (w, h) = (16usize, 16usize);
    let mut px = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            // B, G, R, A (img_png takes the pixels of a gfx surface)
            let c: [u8; 4] = match (x < 8, y < 8) {
                (true, true) => [0xA8, 0x5F, 0x1F, 255],
                (false, true) => [0x2B, 0x79, 0xC9, 255],
                (true, false) => [0x84, 0x8C, 0x3D, 255],
                (false, false) => [0x3F, 0x45, 0xB0, 255],
            };
            px.extend_from_slice(&c);
        }
    }
    img_png::encode(&px, w, h, w * 4, false)
}

/// a device-independent bitmap of 8 x 8 pixels, 24 bits, without file header (BITMAPINFOHEADER
/// and the rows, bottom up): what a ForeignType='Bitmap' without compression may hold
pub fn dib() -> Vec<u8> {
    let (w, h) = (8u32, 8u32);
    let row = (w * 3).div_ceil(4) * 4;
    let mut b = Vec::new();
    for v in [40u32, w, h] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    for v in [0u32, row * h, 2835, 2835, 0, 0] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for y in 0..h {
        let mut r = Vec::new();
        for x in 0..w {
            // BGR: a blue diagonal on white
            r.extend_from_slice(if x == y { &[0xA8, 0x5F, 0x1F] } else { &[0xFF, 0xFF, 0xFF] });
        }
        r.resize(row as usize, 0);
        b.extend_from_slice(&r);
    }
    b
}

/// a master icon as Visio keeps it: a 32 x 32 ICO with 16 colours (766 bytes), here a framed box
pub fn icon() -> Vec<u8> {
    let mut b = Vec::new();
    // ICONDIR, ICONDIRENTRY (32 x 32, 16 colours, 1 plane, 4 bits, 744 bytes at 22)
    b.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    b.extend_from_slice(&[32, 32, 16, 0, 1, 0, 4, 0]);
    b.extend_from_slice(&744u32.to_le_bytes());
    b.extend_from_slice(&22u32.to_le_bytes());
    // BITMAPINFOHEADER: 32 x 64 (colour and mask), 4 bits
    for v in [40u32, 32, 64] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    for v in [0u32, 640, 0, 0, 16, 0] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    // the palette: 0 black, 1 white, 2 the accent (BGRA)
    let mut pal = [[0u8; 4]; 16];
    pal[1] = [0xFF, 0xFF, 0xFF, 0];
    pal[2] = [0xA8, 0x5F, 0x1F, 0];
    pal[3] = [0xF8, 0xEF, 0xE6, 0];
    for p in pal {
        b.extend_from_slice(&p);
    }
    // the colours, rows bottom up, two pixels a byte: a frame in the accent around a light box
    for y in 0..32 {
        for x in (0..32).step_by(2) {
            let c = |x: i32| -> u8 {
                let y = y as i32;
                if !(4..28).contains(&x) || !(8..24).contains(&y) {
                    0
                } else if x == 4 || x == 27 || y == 8 || y == 23 {
                    2
                } else {
                    3
                }
            };
            b.push(c(x) << 4 | c(x + 1));
        }
    }
    // the mask: 1 = transparent outside the box
    for y in 0..32 {
        let mut bits = 0u32;
        for x in 0..32 {
            if !(4..28).contains(&x) || !(8..24).contains(&y) {
                bits |= 1 << (31 - x);
            }
        }
        b.extend_from_slice(&bits.to_be_bytes());
    }
    b
}

/// mm in inches as the file writes them
fn mm(v: f64) -> String {
    format!("{}", v / 25.4)
}

const HEAD: &str = "<?xml version='1.0' encoding='utf-8' ?>\n\
<VisioDocument xmlns='http://schemas.microsoft.com/visio/2003/core' start='190' metric='1' DocLangID='1031' version='14.0' xml:space='preserve'>";

// the settings, the 24 colours, two fonts, four styles (No Style, Normal, Connector, Guide), the
// document sheet
fn common() -> String {
    let mut s = String::from(
        "<DocumentSettings TopPage='0' DefaultTextStyle='1' DefaultLineStyle='1' DefaultFillStyle='1' DefaultGuideStyle='3'>\
<GlueSettings>9</GlueSettings><SnapSettings>295</SnapSettings><SnapExtensions>34</SnapExtensions><SnapAngles/>\
<DynamicGridEnabled>1</DynamicGridEnabled><ProtectStyles>0</ProtectStyles><ProtectShapes>0</ProtectShapes>\
<ProtectMasters>0</ProtectMasters><ProtectBkgnds>0</ProtectBkgnds></DocumentSettings>\n<Colors>",
    );
    let colors = [
        "#000000", "#FFFFFF", "#FF0000", "#00FF00", "#0000FF", "#FFFF00", "#FF00FF", "#00FFFF", "#800000", "#008000", "#000080", "#808000", "#800080", "#008080",
        "#C0C0C0", "#E6E6E6", "#CDCDCD", "#B3B3B3", "#9A9A9A", "#808080", "#666666", "#4D4D4D", "#333333", "#1A1A1A",
    ];
    for (i, c) in colors.iter().enumerate() {
        s.push_str(&format!("<ColorEntry IX='{i}' RGB='{c}'/>"));
    }
    s.push_str(
        "</Colors>\n<FaceNames>\
<FaceName ID='1' Name='Arial' UnicodeRanges='-536858881 -1073711037 9 0' CharSets='1073742335 -65536' Panos='2 11 6 4 2 2 2 2 2 4' Flags='325'/>\
<FaceName ID='2' Name='Times New Roman' UnicodeRanges='-536858881 -1073711013 9 0' CharSets='1073742335 -65536' Panos='2 2 6 3 5 4 5 2 3 4' Flags='325'/>\
</FaceNames>\n<StyleSheets>\
<StyleSheet ID='0' NameU='No Style' Name='Kein Stil'>\
<StyleProp><EnableLineProps>1</EnableLineProps><EnableFillProps>1</EnableFillProps><EnableTextProps>1</EnableTextProps><HideForApply>0</HideForApply></StyleProp>\
<Line><LineWeight Unit='PT'>0.01041666666666667</LineWeight><LineColor>0</LineColor><LinePattern>1</LinePattern><Rounding>0</Rounding>\
<EndArrowSize>2</EndArrowSize><BeginArrow>0</BeginArrow><EndArrow>0</EndArrow><LineCap>0</LineCap><BeginArrowSize>2</BeginArrowSize><LineColorTrans>0</LineColorTrans></Line>\
<Fill><FillForegnd>1</FillForegnd><FillBkgnd>0</FillBkgnd><FillPattern>1</FillPattern><ShdwForegnd>0</ShdwForegnd><ShdwBkgnd>1</ShdwBkgnd><ShdwPattern>0</ShdwPattern>\
<FillForegndTrans>0</FillForegndTrans><FillBkgndTrans>0</FillBkgndTrans><ShdwForegndTrans>0</ShdwForegndTrans><ShdwBkgndTrans>0</ShdwBkgndTrans>\
<ShapeShdwType>0</ShapeShdwType><ShapeShdwOffsetX>0</ShapeShdwOffsetX><ShapeShdwOffsetY>0</ShapeShdwOffsetY><ShapeShdwObliqueAngle>0</ShapeShdwObliqueAngle>\
<ShapeShdwScaleFactor>1</ShapeShdwScaleFactor></Fill>\
<TextBlock><LeftMargin Unit='PT'>0</LeftMargin><RightMargin Unit='PT'>0</RightMargin><TopMargin Unit='PT'>0</TopMargin><BottomMargin Unit='PT'>0</BottomMargin>\
<VerticalAlign>1</VerticalAlign><TextBkgnd>0</TextBkgnd><DefaultTabStop Unit='MM'>0.5</DefaultTabStop><TextDirection>0</TextDirection><TextBkgndTrans>0</TextBkgndTrans></TextBlock>\
<Char IX='0'><Font>1</Font><Color>0</Color><Style>0</Style><Case>0</Case><Pos>0</Pos><FontScale>1</FontScale><Size Unit='PT'>0.1666666666666667</Size>\
<DblUnderline>0</DblUnderline><Overline>0</Overline><Strikethru>0</Strikethru><DoubleStrikethrough>0</DoubleStrikethrough><RTLText>0</RTLText>\
<UseVertical>0</UseVertical><Letterspace>0</Letterspace><ColorTrans>0</ColorTrans><AsianFont>1</AsianFont><ComplexScriptFont>1</ComplexScriptFont>\
<LocalizeFont>0</LocalizeFont><ComplexScriptSize>-1</ComplexScriptSize><LangID>1031</LangID></Char>\
<Para IX='0'><IndFirst>0</IndFirst><IndLeft>0</IndLeft><IndRight>0</IndRight><SpLine>-1.2</SpLine><SpBefore>0</SpBefore><SpAfter>0</SpAfter>\
<HorzAlign>1</HorzAlign><Bullet>0</Bullet><BulletStr/><BulletFont>1</BulletFont><LocalizeBulletFont>0</LocalizeBulletFont><BulletFontSize>-1</BulletFontSize>\
<TextPosAfterBullet>0</TextPosAfterBullet><Flags>0</Flags></Para>\
<Tabs IX='0'/>\
</StyleSheet>\
<StyleSheet ID='1' NameU='Normal' Name='Normal' LineStyle='0' FillStyle='0' TextStyle='0'>\
<Line><LineWeight Unit='PT'>0.01388888888888889</LineWeight><LineColor>#1F5FA8</LineColor></Line>\
<Fill><FillForegnd>#E6EFF8</FillForegnd></Fill>\
<TextBlock><LeftMargin Unit='PT'>0.05555555555555555</LeftMargin><RightMargin Unit='PT'>0.05555555555555555</RightMargin>\
<TopMargin Unit='PT'>0.05555555555555555</TopMargin><BottomMargin Unit='PT'>0.05555555555555555</BottomMargin></TextBlock>\
<Char IX='0'><Color>#1D2228</Color><Size Unit='PT'>0.1388888888888889</Size></Char>\
</StyleSheet>\
<StyleSheet ID='2' NameU='Connector' Name='Verbinder' LineStyle='1' FillStyle='1' TextStyle='1'>\
<Line><LineColor>#4D5560</LineColor><EndArrow>13</EndArrow></Line><Fill><FillPattern>0</FillPattern></Fill>\
<TextBlock><TextBkgnd>2</TextBkgnd></TextBlock>\
</StyleSheet>",
    );
    // the guide style the settings name
    {
        s.push_str(
            "<StyleSheet ID='3' NameU='Guide' Name='Führungslinie' LineStyle='0' FillStyle='0' TextStyle='0'>\
<Line><LineWeight Unit='PT'>0</LineWeight><LineColor>#6FA3E0</LineColor><LinePattern>23</LinePattern></Line><Fill><FillPattern>0</FillPattern></Fill>\
</StyleSheet>",
        );
    }
    s.push_str(
        "</StyleSheets>\n<DocumentSheet NameU='TheDoc' Name='TheDoc' LineStyle='0' FillStyle='0' TextStyle='0'>\
<DocProps><OutputFormat>0</OutputFormat><LockPreview>0</LockPreview><AddMarkup>0</AddMarkup><ViewMarkup>0</ViewMarkup>\
<PreviewQuality>0</PreviewQuality><PreviewScope>0</PreviewScope><DocLangID>1031</DocLangID></DocProps>\
<User NameU='Version' Name='Version' ID='1'><Value Unit='STR'>JD16</Value><Prompt F='No Formula'></Prompt></User>\
</DocumentSheet>\n",
    );
    s
}

// the cells of a rectangle's geometry
fn rect_geom(w: f64, h: f64) -> String {
    format!(
        "<Geom IX='0'><NoFill>0</NoFill><NoLine>0</NoLine><NoShow>0</NoShow><NoSnap>0</NoSnap>\
<MoveTo IX='1'><X F='Width*0'>0</X><Y F='Height*0'>0</Y></MoveTo>\
<LineTo IX='2'><X F='Width*1'>{w}</X><Y F='Height*0'>0</Y></LineTo>\
<LineTo IX='3'><X F='Width*1'>{w}</X><Y F='Height*1'>{h}</Y></LineTo>\
<LineTo IX='4'><X F='Width*0'>0</X><Y F='Height*1'>{h}</Y></LineTo>\
<LineTo IX='5'><X F='Geometry1.X1'>0</X><Y F='Geometry1.Y1'>0</Y></LineTo></Geom>"
    )
}

// a transform: pin, size, local pin in the middle
fn xform(px: f64, py: f64, w: f64, h: f64) -> String {
    format!(
        "<XForm><PinX Unit='MM'>{}</PinX><PinY Unit='MM'>{}</PinY><Width Unit='MM'>{}</Width><Height Unit='MM'>{}</Height>\
<LocPinX F='Width*0.5' Unit='MM'>{}</LocPinX><LocPinY F='Height*0.5' Unit='MM'>{}</LocPinY><Angle>0</Angle><FlipX>0</FlipX><FlipY>0</FlipY><ResizeMode>0</ResizeMode></XForm>",
        mm(px),
        mm(py),
        mm(w),
        mm(h),
        mm(w / 2.0),
        mm(h / 2.0)
    )
}

// the master "Kasten": 30 x 20 mm, a connection point left and right, a shape data field
fn master_box() -> String {
    let (w, h) = (30.0 / 25.4, 20.0 / 25.4);
    format!(
        "<Master ID='2' NameU='Box' Name='Kasten' Prompt='Ein Kasten mit zwei Verbindungspunkten' IconSize='1' AlignName='2' MatchByName='0' \
IconUpdate='0' UniqueID='{{6A1D3E10-16D0-4E4B-9C1A-000000000002}}' BaseID='{{6A1D3E10-16D0-4E4B-9C1A-100000000002}}' PatternFlags='0' Hidden='0'>\
<PageSheet LineStyle='0' FillStyle='0' TextStyle='0'><PageProps><PageWidth Unit='MM'>{w}</PageWidth><PageHeight Unit='MM'>{h}</PageHeight>\
<ShdwOffsetX Unit='MM'>0.1181102362204724</ShdwOffsetX><ShdwOffsetY Unit='MM'>-0.1181102362204724</ShdwOffsetY><PageScale Unit='MM'>0.03937007874015748</PageScale>\
<DrawingScale Unit='MM'>0.03937007874015748</DrawingScale><DrawingSizeType>1</DrawingSizeType><DrawingScaleType>0</DrawingScaleType><InhibitSnap>0</InhibitSnap></PageProps></PageSheet>\
<Icon>{icon}</Icon>\
<Shapes><Shape ID='5' NameU='Box' Name='Kasten' Type='Shape' LineStyle='1' FillStyle='1' TextStyle='1'>\
<XForm><PinX F='Width*0.5' Unit='MM'>{hw}</PinX><PinY F='Height*0.5' Unit='MM'>{hh}</PinY><Width Unit='MM'>{w}</Width><Height Unit='MM'>{h}</Height>\
<LocPinX F='Width*0.5' Unit='MM'>{hw}</LocPinX><LocPinY F='Height*0.5' Unit='MM'>{hh}</LocPinY><Angle>0</Angle><FlipX>0</FlipX><FlipY>0</FlipY><ResizeMode>0</ResizeMode></XForm>\
<Event><TheData F='No Formula'>0</TheData><TheText F='No Formula'>0</TheText><EventDblClick F='OPENTEXTWIN()'>0</EventDblClick><EventXFMod>0</EventXFMod><EventDrop>0</EventDrop></Event>\
<Connection IX='0'><X F='Width*0'>0</X><Y F='Height*0.5'>{hh}</Y><DirX>0</DirX><DirY>0</DirY><Type>0</Type><AutoGen>0</AutoGen><Prompt F='No Formula'></Prompt></Connection>\
<Connection IX='1'><X F='Width*1'>{w}</X><Y F='Height*0.5'>{hh}</Y><DirX>0</DirX><DirY>0</DirY><Type>0</Type><AutoGen>0</AutoGen><Prompt F='No Formula'></Prompt></Connection>\
{geom}\
<Prop NameU='Cost' Name='Kosten' ID='1'><Value Unit='CUR'>0</Value><Prompt>Kosten in Euro</Prompt><Label>Kosten</Label><Format>@</Format><SortKey></SortKey>\
<Type>7</Type><Invisible>0</Invisible><Verify>0</Verify><LangID>1031</LangID><Calendar>0</Calendar></Prop>\
</Shape></Shapes></Master>",
        icon = b64(&icon()),
        hw = w / 2.0,
        hh = h / 2.0,
        geom = rect_geom(w, h),
    )
}

/// the drawing: a page and its background page, an instance of the master, a rectangle with
/// two character rows and a shape data field, an ellipse with a field, a group of two, a
/// connector glued to two shapes, a PNG picture; properties, windows, header and footer
pub fn drawing() -> String {
    let mut s = String::from(HEAD);
    s.push_str(
        "\n<DocumentProperties><Title>JD16-Gegenprobe</Title><Subject>Ältere XML-Formate</Subject><Creator>JFDiagrammer-Test</Creator>\
<Manager>Leitung</Manager><Company>JFLib</Company><Category>Test</Category><Keywords>vdx; xml</Keywords>\
<Desc>Eine von Hand nach Schema geschriebene VDX-Datei.</Desc><HyperlinkBase>https://example.org/</HyperlinkBase>\
<AlternateNames></AlternateNames><Template></Template><BuildNumberCreated>671351309</BuildNumberCreated><BuildNumberEdited>671351309</BuildNumberEdited>\
<CustomProps><CustomProp Name='Projekt' PropType='string'>JD16</CustomProp><CustomProp Name='Seiten' PropType='number'>2</CustomProp></CustomProps>\
<TimeCreated>2026-11-23T10:00:00</TimeCreated><TimeSaved>2026-11-23T11:00:00</TimeSaved><TimeEdited>2026-11-23T10:30:00</TimeEdited>\
</DocumentProperties>\n",
    );
    s.push_str(&common());
    s.push_str("<Masters>");
    s.push_str(&master_box());
    s.push_str("</Masters>\n<Pages>");
    // page 0: A4 landscape, background page 1
    let (pw, ph) = (297.0, 210.0);
    s.push_str(&format!(
        "<Page ID='0' NameU='Page-1' Name='Seite-1' BackPage='1' ViewScale='-1' ViewCenterX='{}' ViewCenterY='{}'>\
<PageSheet LineStyle='0' FillStyle='0' TextStyle='0'><PageProps><PageWidth Unit='MM'>{}</PageWidth><PageHeight Unit='MM'>{}</PageHeight>\
<ShdwOffsetX Unit='MM'>0.1181102362204724</ShdwOffsetX><ShdwOffsetY Unit='MM'>-0.1181102362204724</ShdwOffsetY><PageScale Unit='MM'>0.03937007874015748</PageScale>\
<DrawingScale Unit='MM'>0.03937007874015748</DrawingScale><DrawingSizeType>0</DrawingSizeType><DrawingScaleType>0</DrawingScaleType><InhibitSnap>0</InhibitSnap></PageProps>\
<PrintProps><PageLeftMargin Unit='MM'>0.25</PageLeftMargin><PageRightMargin Unit='MM'>0.25</PageRightMargin><PageTopMargin Unit='MM'>0.25</PageTopMargin>\
<PageBottomMargin Unit='MM'>0.25</PageBottomMargin><ScaleX>1</ScaleX><ScaleY>1</ScaleY><PagesX>1</PagesX><PagesY>1</PagesY><CenterX>0</CenterX><CenterY>0</CenterY>\
<OnPage>0</OnPage><PrintGrid>0</PrintGrid><PrintPageOrientation>2</PrintPageOrientation><PaperKind>9</PaperKind><PaperSource>7</PaperSource></PrintProps>\
<Layer IX='0'><Name>Verbinder</Name><Color>255</Color><Status>0</Status><Visible>1</Visible><Print>1</Print><Active>0</Active><Lock>0</Lock><Snap>1</Snap>\
<Glue>1</Glue><NameUniv>Connector</NameUniv><ColorTrans>0</ColorTrans></Layer></PageSheet><Shapes>",
        mm(pw / 2.0),
        mm(ph / 2.0),
        mm(pw),
        mm(ph)
    ));
    // 1: the master's instance, a text and its own cost
    s.push_str(&format!(
        "<Shape ID='1' NameU='Box' Name='Kasten' Type='Shape' Master='2'>\
<XForm><PinX Unit='MM'>{}</PinX><PinY Unit='MM'>{}</PinY><Width F='Inh' Unit='MM'>{}</Width><Height F='Inh' Unit='MM'>{}</Height>\
<LocPinX F='Inh' Unit='MM'>{}</LocPinX><LocPinY F='Inh' Unit='MM'>{}</LocPinY><Angle F='Inh'>0</Angle><FlipX F='Inh'>0</FlipX><FlipY F='Inh'>0</FlipY><ResizeMode F='Inh'>0</ResizeMode></XForm>\
<Prop NameU='Cost' ID='1'><Value Unit='CUR'>12.5</Value></Prop>\
<Text>Prüfen &amp; weiter</Text></Shape>",
        mm(60.0),
        mm(150.0),
        mm(30.0),
        mm(20.0),
        mm(15.0),
        mm(10.0)
    ));
    // 2: a rectangle with two character rows, tab stops, scratch, user, hyperlink, an action
    let (w, h) = (50.0 / 25.4, 25.0 / 25.4);
    s.push_str(&format!(
        "<Shape ID='2' NameU='Rectangle' Name='Rechteck' Type='Shape' LineStyle='1' FillStyle='1' TextStyle='1'>{xf}\
<Fill><FillForegnd>#FFF2CC</FillForegnd></Fill>\
<Char IX='0'><Font>1</Font><Color>0</Color><Style>0</Style><Locale>0</Locale></Char>\
<Char IX='1'><Font>2</Font><Color>2</Color><Style>1</Style></Char>\
<Para IX='0'><HorzAlign>0</HorzAlign></Para>\
<Tabs IX='0'><Tab IX='0'><Position Unit='MM'>0.5</Position><Alignment>0</Alignment></Tab></Tabs>\
<Scratch IX='0'><X F='Width*0.25' Unit='MM'>{qw}</X><Y Unit='MM'>0</Y><A>0</A><B>0</B><C>0</C><D>0</D></Scratch>\
<Connection IX='0'><X F='Width*0'>0</X><Y F='Height*0.5'>{hh}</Y><DirX>0</DirX><DirY>0</DirY><Type>0</Type><AutoGen>0</AutoGen><Prompt F='No Formula'></Prompt></Connection>\
{geom}\
<Act ID='1'><Menu>Rechteck &amp;markieren</Menu><Action F='SETF(GetRef(FillForegnd),\"RGB(255,0,0)\")'>0</Action><Checked>0</Checked><Disabled>0</Disabled>\
<ReadOnly>0</ReadOnly><Invisible>0</Invisible><BeginGroup>0</BeginGroup><TagName></TagName><ButtonFace></ButtonFace><SortKey></SortKey></Act>\
<User NameU='Faktor' Name='Faktor' ID='2'><Value F='Width/Height'>2</Value><Prompt F='No Formula'></Prompt></User>\
<Hyperlink NameU='Link' Name='Verweis' ID='1'><Description>Beispiel</Description><Address>https://example.org/</Address><SubAddress></SubAddress>\
<ExtraInfo></ExtraInfo><Frame></Frame><NewWindow>0</NewWindow><Default>1</Default><Invisible>0</Invisible><SortKey></SortKey></Hyperlink>\
<Text><cp IX='0'/>Normal <cp IX='1'/>fett und rot</Text></Shape>",
        xf = xform(140.0, 150.0, 50.0, 25.0),
        qw = w / 4.0,
        hh = h / 2.0,
        geom = rect_geom(w, h),
    ));
    // 3: an ellipse with a text field (its width)
    let (w, h) = (40.0 / 25.4, 25.0 / 25.4);
    s.push_str(&format!(
        "<Shape ID='3' NameU='Ellipse' Name='Ellipse' Type='Shape' LineStyle='1' FillStyle='1' TextStyle='1'>{xf}\
<Fill><FillForegnd>#C9792B</FillForegnd></Fill>\
<Field IX='0'><Value F='Width' Unit='MM'>{w}</Value><EditMode>0</EditMode><Format F='FIELDPICTURE(0)'>0.## u</Format><Type>2</Type>\
<UICat>2</UICat><UICod>0</UICod><UIFmt>0</UIFmt><Calendar>0</Calendar><ObjectKind>0</ObjectKind></Field>\
<Geom IX='0'><NoFill>0</NoFill><NoLine>0</NoLine><NoShow>0</NoShow><NoSnap>0</NoSnap>\
<Ellipse IX='1'><X F='Width*0.5'>{hw}</X><Y F='Height*0.5'>{hh}</Y><A F='Width*1'>{w}</A><B F='Height*0.5'>{hh}</B><C F='Width*0.5'>{hw}</C><D F='Height*1'>{h}</D></Ellipse></Geom>\
<Text>Breite: <fld IX='0'>40 mm</fld></Text></Shape>",
        xf = xform(230.0, 150.0, 40.0, 25.0),
        hw = w / 2.0,
        hh = h / 2.0,
    ));
    // 4: a group of two rectangles
    let (gw, gh) = (50.0 / 25.4, 30.0 / 25.4);
    let (sw, sh) = (25.0 / 25.4, 30.0 / 25.4);
    s.push_str(&format!(
        "<Shape ID='4' NameU='Group' Name='Gruppe' Type='Group' LineStyle='1' FillStyle='1' TextStyle='1'>{xf}\
<Group><SelectMode>1</SelectMode><DisplayMode>2</DisplayMode><IsDropTarget>0</IsDropTarget><IsSnapTarget>1</IsSnapTarget><IsTextEditTarget>1</IsTextEditTarget><DontMoveChildren>0</DontMoveChildren></Group>\
<Shapes>\
<Shape ID='6' NameU='Left' Name='Links' Type='Shape' LineStyle='1' FillStyle='1' TextStyle='1'>{x6}<Fill><FillForegnd>#3D8C84</FillForegnd></Fill>{g}<Text>A</Text></Shape>\
<Shape ID='7' NameU='Right' Name='Rechts' Type='Shape' LineStyle='1' FillStyle='1' TextStyle='1'>{x7}<Fill><FillForegnd>#7A5C99</FillForegnd></Fill>{g}<Text>B</Text></Shape>\
</Shapes></Shape>",
        xf = xform(60.0, 80.0, 50.0, 30.0),
        x6 = xform(12.5, 15.0, 25.0, 30.0),
        x7 = xform(37.5, 15.0, 25.0, 30.0),
        g = rect_geom(sw, sh),
    ));
    let _ = (gw, gh);
    // 8: a connector from the instance's right point to the rectangle's left point
    let (bx, by, ex, ey) = (75.0 / 25.4, 150.0 / 25.4, 115.0 / 25.4, 150.0 / 25.4);
    let len = ex - bx;
    s.push_str(&format!(
        "<Shape ID='8' NameU='Connector' Name='Verbinder' Type='Shape' LineStyle='2' FillStyle='2' TextStyle='2'>\
<XForm><PinX F='(BeginX+EndX)/2' Unit='MM'>{px}</PinX><PinY F='(BeginY+EndY)/2' Unit='MM'>{by}</PinY>\
<Width F='SQRT((EndX-BeginX)^2+(EndY-BeginY)^2)' Unit='MM'>{len}</Width><Height Unit='MM'>0</Height>\
<LocPinX F='Width*0.5' Unit='MM'>{hl}</LocPinX><LocPinY F='Height*0.5' Unit='MM'>0</LocPinY><Angle F='ATAN2(EndY-BeginY,EndX-BeginX)'>0</Angle>\
<FlipX>0</FlipX><FlipY>0</FlipY><ResizeMode>0</ResizeMode></XForm>\
<XForm1D><BeginX F='PAR(PNT(Sheet.1!Connections.X2,Sheet.1!Connections.Y2))' Unit='MM'>{bx}</BeginX>\
<BeginY F='PAR(PNT(Sheet.1!Connections.X2,Sheet.1!Connections.Y2))' Unit='MM'>{by}</BeginY>\
<EndX F='PAR(PNT(Sheet.2!Connections.X1,Sheet.2!Connections.Y1))' Unit='MM'>{ex}</EndX>\
<EndY F='PAR(PNT(Sheet.2!Connections.X1,Sheet.2!Connections.Y1))' Unit='MM'>{ey}</EndY></XForm1D>\
<LayerMem><LayerMember>0</LayerMember></LayerMem>\
<Misc><ObjType>2</ObjType></Misc>\
<Geom IX='0'><NoFill>1</NoFill><NoLine>0</NoLine><NoShow>0</NoShow><NoSnap>0</NoSnap>\
<MoveTo IX='1'><X F='Width*0'>0</X><Y F='Height*0'>0</Y></MoveTo><LineTo IX='2'><X F='Width*1'>{len}</X><Y F='Height*0'>0</Y></LineTo></Geom>\
</Shape>",
        px = (bx + ex) / 2.0,
        hl = len / 2.0,
    ));
    // 9: a PNG picture
    s.push_str(&format!(
        "<Shape ID='9' NameU='Picture' Name='Bild' Type='Foreign' LineStyle='1' FillStyle='1' TextStyle='1'>{xf}\
<Line><LinePattern>0</LinePattern></Line><Fill><FillPattern>0</FillPattern></Fill>\
<Foreign><ImgOffsetX F='ImgWidth*0' Unit='MM'>0</ImgOffsetX><ImgOffsetY F='ImgHeight*0' Unit='MM'>0</ImgOffsetY>\
<ImgWidth F='Width*1' Unit='MM'>{w}</ImgWidth><ImgHeight F='Height*1' Unit='MM'>{w}</ImgHeight></Foreign>\
<ForeignData ForeignType='Bitmap' CompressionType='PNG'>{png}</ForeignData></Shape>",
        xf = xform(180.0, 60.0, 20.0, 20.0),
        w = 20.0 / 25.4,
        png = b64(&png()),
    ));
    s.push_str("</Shapes><Connects>");
    s.push_str("<Connect FromSheet='8' FromCell='BeginX' FromPart='9' ToSheet='1' ToCell='Connections.X2' ToPart='101'/>");
    s.push_str("<Connect FromSheet='8' FromCell='EndX' FromPart='12' ToSheet='2' ToCell='Connections.X1' ToPart='100'/>");
    s.push_str("</Connects></Page>");
    // page 1: the background page
    s.push_str(&format!(
        "<Page ID='1' NameU='Background-1' Name='Hintergrund-1' Background='1'>\
<PageSheet LineStyle='0' FillStyle='0' TextStyle='0'><PageProps><PageWidth Unit='MM'>{}</PageWidth><PageHeight Unit='MM'>{}</PageHeight>\
<PageScale Unit='MM'>0.03937007874015748</PageScale><DrawingScale Unit='MM'>0.03937007874015748</DrawingScale><DrawingSizeType>0</DrawingSizeType>\
<DrawingScaleType>0</DrawingScaleType></PageProps></PageSheet><Shapes>\
<Shape ID='1' NameU='Note' Name='Hinweis' Type='Shape' LineStyle='1' FillStyle='1' TextStyle='1'>{xf}\
<Line><LinePattern>0</LinePattern></Line><Fill><FillPattern>0</FillPattern></Fill><Text>Entwurf (Hintergrundseite)</Text></Shape>\
</Shapes></Page>",
        mm(pw),
        mm(ph),
        xf = xform(148.5, 20.0, 100.0, 10.0),
    ));
    s.push_str("</Pages>\n");
    s.push_str(&format!(
        "<Windows ClientWidth='1600' ClientHeight='900'>\
<Window ID='0' WindowType='Drawing' WindowState='1073741824' WindowLeft='-8' WindowTop='-31' WindowWidth='1616' WindowHeight='939' ContainerType='Page' Page='0' \
ViewScale='1' ViewCenterX='{}' ViewCenterY='{}'><ShowRulers>1</ShowRulers><ShowGrid>1</ShowGrid><ShowPageBreaks>0</ShowPageBreaks><ShowGuides>1</ShowGuides>\
<ShowConnectionPoints>1</ShowConnectionPoints><GlueSettings>9</GlueSettings><SnapSettings>295</SnapSettings><SnapExtensions>34</SnapExtensions><SnapAngles/>\
<DynamicGridEnabled>1</DynamicGridEnabled><TabSplitterPos>0.5</TabSplitterPos></Window>\
<Window ID='1' WindowType='Stencil' WindowState='67109889' WindowLeft='0' WindowTop='0' WindowWidth='250' WindowHeight='600' ParentWindow='0' Document='jd16-schablone.vsx'>\
<StencilGroup>10</StencilGroup><StencilGroupPos>0</StencilGroupPos></Window></Windows>\n",
        mm(100.0),
        mm(140.0)
    ));
    s.push_str(
        "<HeaderFooter HeaderFooterColor='#000000'><HeaderMargin Unit='MM'>0.25</HeaderMargin><FooterMargin Unit='MM'>0.25</FooterMargin>\
<HeaderLeft>JD16</HeaderLeft><HeaderCenter></HeaderCenter><HeaderRight>&amp;p</HeaderRight><FooterLeft></FooterLeft><FooterCenter>&amp;f</FooterCenter>\
<FooterRight></FooterRight><HeaderFooterFont Height='-13' Width='0' Escapement='0' Orientation='0' Weight='400' Italic='0' Underline='0' StrikeOut='0' \
CharSet='0' OutPrecision='0' ClipPrecision='0' Quality='0' PitchAndFamily='0' FaceName='Arial'/></HeaderFooter>\n",
    );
    s.push_str("</VisioDocument>\n");
    s
}

// a master: an ellipse (Kreis) or a 1-D arrow (Pfeil)
fn master_circle() -> String {
    let d = 20.0 / 25.4;
    format!(
        "<Master ID='3' NameU='Circle' Name='Kreis' Prompt='Ein Kreis' IconSize='1' AlignName='2' MatchByName='0' IconUpdate='1' \
UniqueID='{{6A1D3E10-16D0-4E4B-9C1A-000000000003}}' BaseID='{{6A1D3E10-16D0-4E4B-9C1A-100000000003}}' PatternFlags='0' Hidden='0'>\
<Shapes><Shape ID='5' NameU='Circle' Name='Kreis' Type='Shape' LineStyle='1' FillStyle='1' TextStyle='1'>\
<XForm><PinX F='Width*0.5' Unit='MM'>{r}</PinX><PinY F='Height*0.5' Unit='MM'>{r}</PinY><Width Unit='MM'>{d}</Width><Height F='Width*1' Unit='MM'>{d}</Height>\
<LocPinX F='Width*0.5' Unit='MM'>{r}</LocPinX><LocPinY F='Height*0.5' Unit='MM'>{r}</LocPinY><Angle>0</Angle><FlipX>0</FlipX><FlipY>0</FlipY><ResizeMode>0</ResizeMode></XForm>\
<Geom IX='0'><NoFill>0</NoFill><NoLine>0</NoLine><NoShow>0</NoShow><NoSnap>0</NoSnap>\
<Ellipse IX='1'><X F='Width*0.5'>{r}</X><Y F='Height*0.5'>{r}</Y><A F='Width*1'>{d}</A><B F='Height*0.5'>{r}</B><C F='Width*0.5'>{r}</C><D F='Height*1'>{d}</D></Ellipse></Geom>\
</Shape></Shapes></Master>",
        r = d / 2.0,
    )
}

fn master_arrow() -> String {
    let l = 30.0 / 25.4;
    format!(
        "<Master ID='4' NameU='Arrow' Name='Pfeil' Prompt='Eine Linie mit Pfeilspitze' IconSize='1' AlignName='2' MatchByName='0' IconUpdate='1' \
UniqueID='{{6A1D3E10-16D0-4E4B-9C1A-000000000004}}' BaseID='{{6A1D3E10-16D0-4E4B-9C1A-100000000004}}' PatternFlags='0' Hidden='0'>\
<Shapes><Shape ID='5' NameU='Arrow' Name='Pfeil' Type='Shape' LineStyle='2' FillStyle='2' TextStyle='2'>\
<XForm><PinX F='(BeginX+EndX)/2' Unit='MM'>{h}</PinX><PinY F='(BeginY+EndY)/2' Unit='MM'>0</PinY><Width F='SQRT((EndX-BeginX)^2+(EndY-BeginY)^2)' Unit='MM'>{l}</Width>\
<Height Unit='MM'>0</Height><LocPinX F='Width*0.5' Unit='MM'>{h}</LocPinX><LocPinY F='Height*0.5' Unit='MM'>0</LocPinY><Angle F='ATAN2(EndY-BeginY,EndX-BeginX)'>0</Angle>\
<FlipX>0</FlipX><FlipY>0</FlipY><ResizeMode>0</ResizeMode></XForm>\
<XForm1D><BeginX Unit='MM'>0</BeginX><BeginY Unit='MM'>0</BeginY><EndX Unit='MM'>{l}</EndX><EndY Unit='MM'>0</EndY></XForm1D>\
<Line><EndArrow>13</EndArrow></Line>\
<Geom IX='0'><NoFill>1</NoFill><NoLine>0</NoLine><NoShow>0</NoShow><NoSnap>0</NoSnap>\
<MoveTo IX='1'><X F='Width*0'>0</X><Y F='Height*0'>0</Y></MoveTo><LineTo IX='2'><X F='Width*1'>{l}</X><Y F='Height*0'>0</Y></LineTo></Geom>\
</Shape></Shapes></Master>",
        h = l / 2.0,
    )
}

/// the stencil: three masters (a box with an icon, a circle and an arrow whose icons Visio
/// makes), one page without shapes
pub fn stencil() -> String {
    let mut s = String::from(HEAD);
    s.push_str("\n<DocumentProperties><Title>JD16-Schablone</Title><Creator>JFDiagrammer-Test</Creator></DocumentProperties>\n");
    s.push_str(&common());
    s.push_str("<Masters>");
    s.push_str(&master_box());
    s.push_str(&master_circle());
    s.push_str(&master_arrow());
    s.push_str("</Masters>\n<Pages><Page ID='0' NameU='Page-1' Name='Seite-1'><PageSheet LineStyle='0' FillStyle='0' TextStyle='0'><PageProps>");
    s.push_str(&format!("<PageWidth Unit='MM'>{}</PageWidth><PageHeight Unit='MM'>{}</PageHeight>", mm(210.0), mm(297.0)));
    s.push_str("</PageProps></PageSheet></Page></Pages>\n</VisioDocument>\n");
    s
}

/// a drawing with one picture: a bitmap without file header (ForeignType='Bitmap', no
/// compression)
pub fn bitmap() -> String {
    let mut s = String::from(HEAD);
    s.push('\n');
    s.push_str(&common());
    s.push_str(&format!(
        "<Pages><Page ID='0' NameU='Page-1' Name='Seite-1'><PageSheet LineStyle='0' FillStyle='0' TextStyle='0'><PageProps>\
<PageWidth Unit='MM'>{}</PageWidth><PageHeight Unit='MM'>{}</PageHeight></PageProps></PageSheet><Shapes>\
<Shape ID='1' NameU='Picture' Name='Bild' Type='Foreign' LineStyle='1' FillStyle='1' TextStyle='1'>{xf}\
<Line><LinePattern>0</LinePattern></Line><Fill><FillPattern>0</FillPattern></Fill>\
<Foreign><ImgOffsetX F='ImgWidth*0' Unit='MM'>0</ImgOffsetX><ImgOffsetY F='ImgHeight*0' Unit='MM'>0</ImgOffsetY>\
<ImgWidth F='Width*1' Unit='MM'>{w}</ImgWidth><ImgHeight F='Height*1' Unit='MM'>{w}</ImgHeight></Foreign>\
<ForeignData ForeignType='Bitmap'>{dib}</ForeignData></Shape></Shapes></Page></Pages>\n</VisioDocument>\n",
        mm(210.0),
        mm(297.0),
        xf = xform(105.0, 200.0, 40.0, 40.0),
        w = 40.0 / 25.4,
        dib = b64(&dib()),
    ));
    s
}
