// a11ytest -- Barrierefreiheit: a web page's accessibility tree (section 19 of docs/GUIDE.md)
//
// A canvas draws a small web page (src/page.rs) and publishes its accessibility tree as an engine
// would: records of the kinds K_TABLE ..= K_FIGURE with an id64 each, parents named instead of
// rectangles, descriptions, link addresses, cells, levels, places in a set, form states, live
// regions, and the content below the view as records out of view (AS_OFFSCREEN).  A screen
// reader's requests (focus, click, scroll into view, expand / collapse, values, texts,
// selections) come back through ui.a11y_request_take; "Ansage" says a text (ui.announce).
//
//   python build_rs.py run a11ytest                  the demo, for a screen reader by hand
//   python build_rs.py test a11ytest                 tests/*.uis (the page, its texts)
//   node apps/a11ytest/tests/a11y_web.mjs [build]    the shadow tree in Chromium (web build)
// plattest --test a11y-rich / uia-rich / a11y-rich-bus read the same page through the model,
// UI Automation and AT-SPI2 (python3 tools/platshared/atspi_bus.py --rich in WSL).
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod page;

use ui::*;

const DESC: AppDesc = AppDesc { title: "Barrierefreiheit", flags: AF_RESIZABLE, w: 560, h: 620, min_w: 400, min_h: 360, ..AppDesc::NONE };

#[derive(Default)]
struct A11ytest {
    page: page::Page,
}

impl App for A11ytest {
    fn frame(&mut self, ui: &mut Ui) {
        self.page.frame(ui);
    }
}

ui::main!(ui::launch(&DESC, A11ytest::default(), &style::STYLE));
