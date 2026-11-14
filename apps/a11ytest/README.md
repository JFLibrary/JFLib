# Barrierefreiheit

A test app of the library: a canvas that draws a small web page and publishes its accessibility
tree as a browser engine would (docs/GUIDE.md, section 19, "A web page's tree"): records of the
kinds `K_TABLE` ..= `K_FIGURE` with an id64 each, parents named instead of rectangles,
descriptions, link addresses, cells, levels, form states, live regions, and the content below the
view as records out of view (`AS_OFFSCREEN`). A screen reader's requests come back through
`ui.a11y_request_take`; "Ansage" says a text (`ui.announce`). Made with `python build_rs.py new
a11ytest --title "Barrierefreiheit"` on 2026-11-13.

`src/page.rs` is the page; plattest takes the same file (`plattest --test a11y-rich`, `uia-rich`,
`a11y-rich-bus`).

| Test | What it tests |
|---|---|
| `tests/smoke.uis` | the records of the page by kind (`table:"Preise"`, `cell:"1,20 €"`, `landmark:"Hauptmenü"` ...), those out of view, a live region that changes, the scroll bar's value |
| `tests/a11y.uis` | the accessibility tree through the command `a11y`: roles, parents, places, levels, states, actions; requests (expand, collapse, scroll into view, select / add / remove, increment, a value, a click) reaching the page; an announcement |
| `tests/a11y_web.mjs` | the web build in Chromium: its accessibility tree of the rich format, requests through the DOM, the announcement's live region (`node apps/a11ytest/tests/a11y_web.mjs build` after `python build_rs.py app a11ytest --target web`) |

## Build, run, test

```sh
python build_rs.py app a11ytest                 # build/win/a11ytest.exe and build/linux/a11ytest
python build_rs.py run a11ytest                 # the demo, for a screen reader by hand
python build_rs.py test a11ytest                # build it and run tests/*.uis (Windows and Linux)
```
