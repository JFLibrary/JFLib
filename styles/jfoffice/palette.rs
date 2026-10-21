// jfOffice: light/dark interface tokens measured against the supplied WordCraft reference.
// Only visual values are shared with the reference; painting and widget logic are our own.
// Token order follows ui::consts. Document colors never come from this palette.

use ui::consts::T_COUNT;

pub(crate) const LIGHT: [u32; T_COUNT] = [
    0xfff0f0f0, // title bar / tabs / status
    0xffe6e6e6, // document workspace
    0xfff9f9f9, // ribbon
    0xffe1e1e1, // layer border
    0xfffafafa, // panel
    0xffe1e1e1, // panel border
    0xffffffff, // control
    0xffe6e6e6, // hover
    0xffd6d6d6, // pressed
    0xfff9f9f9, // disabled control
    0xffc7c7c7, // input border
    0xffc7c7c7, // same border on all edges
    0xffc8c8c8, // strong border
    0,          // transparent tool
    0xffe6e6e6, // tool hover
    0xffd6d6d6, // tool pressed
    0,          // disabled tool
    0xff242424, // primary text
    0xff616161, // secondary text
    0xff707070, // captions / group labels
    0xffa8a8a8, // disabled text
    0, 0, 0,  // accent set by Style::accent
    0xffa8a8a8, // disabled accent
    0xffffffff, // on accent
    0xfff0f0f0, // disabled on accent
    0,          // accent text set by Style::accent
    0xff3b5bdb, // focus
    0xffffffff, // inner focus
    0xffffffff, // menu
    0xffc8c8c8, // menu border
    0x48000000, // modal dim
    0xffe1e1e1, // divider
    0xffffffff, // input
    0xffd13b3b, // error
    0xff2e8b57, // success
    0xffe07b1f, // warning
    0xff2f6fd0, // information
    0xffffecee, // error surface
    0xffe8f5ec, // success surface
    0xfffff2da, // warning surface
    0xffdde3f8, // information surface / checked fill
    0xffc42b3b, // close hover
    0x22000000, // page shadow
    0xffa0a0a0, // scrollbar
    0xffc8c8c8, // slider track
    0xffffffff, // slider thumb
    0xffffffff, // unchecked control
    0xff616161, // strong fill
];

pub(crate) const DARK: [u32; T_COUNT] = [
    0xff1f1f1f, // title bar / tabs / status
    0xff141414, // document workspace
    0xff292929, // ribbon
    0xff3a3a3a, // layer border
    0xff262626, // panel
    0xff3a3a3a, // panel border
    0xff1e1e1e, // control
    0xff3d3d3d, // hover
    0xff4a4a4a, // pressed
    0xff292929, // disabled control
    0xff585858, // input border
    0xff585858, // same border on all edges
    0xff555555, // strong border
    0,          // transparent tool
    0xff3d3d3d, // tool hover
    0xff4a4a4a, // tool pressed
    0,          // disabled tool
    0xffeeeeee, // primary text
    0xffc8c8c8, // secondary text
    0xffa0a0a0, // captions / group labels
    0xff6a6a6a, // disabled text
    0, 0, 0,  // accent set by Style::accent
    0xff6a6a6a, // disabled accent
    0xffffffff, // on accent
    0xffc8c8c8, // disabled on accent
    0,          // accent text set by Style::accent
    0xff7a93f0, // focus
    0xff1f1f1f, // inner focus
    0xff2b2b2b, // menu
    0xff555555, // menu border
    0x90000000, // modal dim
    0xff3a3a3a, // divider
    0xff1e1e1e, // input
    0xffee6a6a, // error
    0xff5bc08a, // success
    0xfff0a050, // warning
    0xff6c9bea, // information
    0xff4c2d39, // error surface
    0xff294238, // success surface
    0xff494032, // warning surface
    0xff2e3a66, // information surface / checked fill
    0xffc42b3b, // close hover
    0x60000000, // page shadow
    0xff6a6a6a, // scrollbar
    0xff555555, // slider track
    0xfff2f2f2, // slider thumb
    0xff1e1e1e, // unchecked control
    0xffc8c8c8, // strong fill
];

pub(crate) fn icon(dark: bool) -> u32 {
    if dark { 0xffdddddd } else { 0xff424242 }
}

pub(crate) fn checked(dark: bool) -> u32 {
    if dark { 0xff2e3a66 } else { 0xffdde3f8 }
}
