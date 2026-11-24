// cjk -- the indexes of the WHATWG Encoding Standard's multi-byte encodings for formats::encoding (generated
// by tools/gen_encodings.py, do not edit).  cjk.bin holds them as little-endian 16-bit words; a Table is
// where one starts and how many words it has.  The indexes: Copyright © WHATWG (Apple, Google, Mozilla,
// Microsoft), BSD 3-Clause License (licenses/WHATWG-LICENSE.txt).

static BIN: &[u8; 300844] = include_bytes!("cjk.bin");

#[derive(Clone, Copy)]
pub(crate) struct Table {
    pub at: usize,
    pub len: usize,
}

impl Table {
    /// word i (0 past the end)
    #[inline]
    pub(crate) fn get(self, i: usize) -> u16 {
        if i >= self.len {
            return 0;
        }
        let o = 2 * (self.at + i);
        u16::from_le_bytes([BIN[o], BIN[o + 1]])
    }
}

/// jis0208: pointer -> code point (Shift_JIS, EUC-JP, ISO-2022-JP)
pub(crate) const JIS0208: Table = Table { at: 0, len: 11104 };
/// jis0212: pointer -> code point (EUC-JP after 0x8F)
pub(crate) const JIS0212: Table = Table { at: 11104, len: 7211 };
/// euc-kr: pointer -> code point
pub(crate) const EUC_KR: Table = Table { at: 18315, len: 23750 };
/// big5: pointer -> the low 16 bits of the code point (plus 0x20000 where BIG5_ASTRAL says so)
pub(crate) const BIG5: Table = Table { at: 42065, len: 19782 };
/// big5: bit p % 16 of word p / 16 is set when the code point of pointer p is in plane 2
pub(crate) const BIG5_ASTRAL: Table = Table { at: 61847, len: 1237 };
/// gb18030: pointer -> code point (the two-byte sequences)
pub(crate) const GB18030: Table = Table { at: 63084, len: 23940 };
/// the pointers of jis0208 by code point, then pointer (the encoders search them)
pub(crate) const JIS0208_ORDER: Table = Table { at: 87024, len: 7724 };
/// the pointers of euc-kr by code point, then pointer
pub(crate) const EUC_KR_ORDER: Table = Table { at: 94748, len: 17048 };
/// the pointers of big5 from 5024 on (the encoder's part) by code point, then pointer
pub(crate) const BIG5_ORDER: Table = Table { at: 111796, len: 14686 };
/// the pointers of gb18030 by code point, then pointer
pub(crate) const GB18030_ORDER: Table = Table { at: 126482, len: 23940 };

/// index gb18030 ranges: (pointer, code point), both ascending; a pointer from one on counts up from its code point
pub(crate) static GB18030_RANGES: [(u32, u32); 207] = [
    (0, 0x0080), (36, 0x00A5), (38, 0x00A9), (45, 0x00B2), (50, 0x00B8), (81, 0x00D8), (89, 0x00E2), (95, 0x00EB),
    (96, 0x00EE), (100, 0x00F4), (103, 0x00F8), (104, 0x00FB), (105, 0x00FD), (109, 0x0102), (126, 0x0114), (133, 0x011C),
    (148, 0x012C), (172, 0x0145), (175, 0x0149), (179, 0x014E), (208, 0x016C), (306, 0x01CF), (307, 0x01D1), (308, 0x01D3),
    (309, 0x01D5), (310, 0x01D7), (311, 0x01D9), (312, 0x01DB), (313, 0x01DD), (341, 0x01FA), (428, 0x0252), (443, 0x0262),
    (544, 0x02C8), (545, 0x02CC), (558, 0x02DA), (741, 0x03A2), (742, 0x03AA), (749, 0x03C2), (750, 0x03CA), (805, 0x0402),
    (819, 0x0450), (820, 0x0452), (7922, 0x2011), (7924, 0x2017), (7925, 0x201A), (7927, 0x201E), (7934, 0x2027), (7943, 0x2031),
    (7944, 0x2034), (7945, 0x2036), (7950, 0x203C), (8062, 0x20AD), (8148, 0x2104), (8149, 0x2106), (8152, 0x210A), (8164, 0x2117),
    (8174, 0x2122), (8236, 0x216C), (8240, 0x217A), (8262, 0x2194), (8264, 0x219A), (8374, 0x2209), (8380, 0x2210), (8381, 0x2212),
    (8384, 0x2216), (8388, 0x221B), (8390, 0x2221), (8392, 0x2224), (8393, 0x2226), (8394, 0x222C), (8396, 0x222F), (8401, 0x2238),
    (8406, 0x223E), (8416, 0x2249), (8419, 0x224D), (8424, 0x2253), (8437, 0x2262), (8439, 0x2268), (8445, 0x2270), (8482, 0x2296),
    (8485, 0x229A), (8496, 0x22A6), (8521, 0x22C0), (8603, 0x2313), (8936, 0x246A), (8946, 0x249C), (9046, 0x254C), (9050, 0x2574),
    (9063, 0x2590), (9066, 0x2596), (9076, 0x25A2), (9092, 0x25B4), (9100, 0x25BE), (9108, 0x25C8), (9111, 0x25CC), (9113, 0x25D0),
    (9131, 0x25E6), (9162, 0x2607), (9164, 0x260A), (9218, 0x2641), (9219, 0x2643), (11329, 0x2E82), (11331, 0x2E85), (11334, 0x2E89),
    (11336, 0x2E8D), (11346, 0x2E98), (11361, 0x2EA8), (11363, 0x2EAB), (11366, 0x2EAF), (11370, 0x2EB4), (11372, 0x2EB8), (11375, 0x2EBC),
    (11389, 0x2ECB), (11682, 0x2FFC), (11686, 0x3004), (11687, 0x3018), (11692, 0x301F), (11694, 0x302A), (11714, 0x303F), (11716, 0x3094),
    (11723, 0x309F), (11725, 0x30F7), (11730, 0x30FF), (11736, 0x312A), (11982, 0x322A), (11989, 0x3232), (12102, 0x32A4), (12336, 0x3390),
    (12348, 0x339F), (12350, 0x33A2), (12384, 0x33C5), (12393, 0x33CF), (12395, 0x33D3), (12397, 0x33D6), (12510, 0x3448), (12553, 0x3474),
    (12851, 0x359F), (12962, 0x360F), (12973, 0x361B), (13738, 0x3919), (13823, 0x396F), (13919, 0x39D1), (13933, 0x39E0), (14080, 0x3A74),
    (14298, 0x3B4F), (14585, 0x3C6F), (14698, 0x3CE1), (15583, 0x4057), (15847, 0x4160), (16318, 0x4338), (16434, 0x43AD), (16438, 0x43B2),
    (16481, 0x43DE), (16729, 0x44D7), (17102, 0x464D), (17122, 0x4662), (17315, 0x4724), (17320, 0x472A), (17402, 0x477D), (17418, 0x478E),
    (17859, 0x4948), (17909, 0x497B), (17911, 0x497E), (17915, 0x4984), (17916, 0x4987), (17936, 0x499C), (17939, 0x49A0), (17961, 0x49B8),
    (18664, 0x4C78), (18703, 0x4CA4), (18814, 0x4D1A), (18962, 0x4DAF), (19043, 0x9FA6), (33469, 0xE76C), (33470, 0xE7C8), (33471, 0xE7E7),
    (33484, 0xE815), (33485, 0xE819), (33490, 0xE81F), (33497, 0xE827), (33501, 0xE82D), (33505, 0xE833), (33513, 0xE83C), (33520, 0xE844),
    (33536, 0xE856), (33550, 0xE865), (37845, 0xF92D), (37921, 0xF97A), (37948, 0xF996), (38029, 0xF9E8), (38038, 0xF9F2), (38064, 0xFA10),
    (38065, 0xFA12), (38066, 0xFA15), (38069, 0xFA19), (38075, 0xFA22), (38076, 0xFA25), (38078, 0xFA2A), (39108, 0xFE32), (39109, 0xFE45),
    (39113, 0xFE53), (39114, 0xFE58), (39115, 0xFE67), (39116, 0xFE6C), (39265, 0xFF5F), (39394, 0xFFE6), (189000, 0x10000),
];

/// the old code points of the 18 characters GB18030-2022 moved out of the Private Use Area, which the encoder
/// still writes with their two bytes: (code point, pointer), by code point
pub(crate) static GB18030_PUA: [(u16, u16); 18] = [
    (0xE78D, 7182), (0xE78E, 7183), (0xE78F, 7184), (0xE790, 7185), (0xE791, 7186), (0xE792, 7187),
    (0xE793, 7188), (0xE794, 7201), (0xE795, 7202), (0xE796, 7208), (0xE81E, 23775), (0xE826, 23783),
    (0xE82B, 23788), (0xE82C, 23789), (0xE832, 23795), (0xE843, 23812), (0xE854, 23829), (0xE864, 23845),
];

/// index ISO-2022-JP katakana: U+FF61..U+FF9F (half-width) -> the full-width character the encoder writes
pub(crate) static ISO2022JP_KATAKANA: [u16; 63] = [
    0x3002, 0x300C, 0x300D, 0x3001, 0x30FB, 0x30F2, 0x30A1, 0x30A3, 0x30A5, 0x30A7, 0x30A9, 0x30E3,
    0x30E5, 0x30E7, 0x30C3, 0x30FC, 0x30A2, 0x30A4, 0x30A6, 0x30A8, 0x30AA, 0x30AB, 0x30AD, 0x30AF,
    0x30B1, 0x30B3, 0x30B5, 0x30B7, 0x30B9, 0x30BB, 0x30BD, 0x30BF, 0x30C1, 0x30C4, 0x30C6, 0x30C8,
    0x30CA, 0x30CB, 0x30CC, 0x30CD, 0x30CE, 0x30CF, 0x30D2, 0x30D5, 0x30D8, 0x30DB, 0x30DE, 0x30DF,
    0x30E0, 0x30E1, 0x30E2, 0x30E4, 0x30E6, 0x30E8, 0x30E9, 0x30EA, 0x30EB, 0x30EC, 0x30ED, 0x30EF,
    0x30F3, 0x309B, 0x309C,
];
