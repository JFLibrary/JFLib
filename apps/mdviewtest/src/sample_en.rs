// the English copies of the sample documents (main.rs has the German ones, which scripts check;
// the app picks these when i18n::language_code() is not "de")

/// SAMPLE_MD in English
pub(crate) const SAMPLE_MD_EN: &str = r#"# Markdown viewer

A paragraph with *italic*, **bold**, ***both***, ~~strikethrough~~, `code in text` and a [link](https://example.org/jflib "Title"). Bare addresses become links: https://example.org/docs and www.example.com. A hard break\
after a backslash and a soft
break.

## Lists

- First item
- Second item with a sublist
  - Sub-item
    - Even deeper
- Third item

3. Step three
4. Step four

- [x] Finished task
- [ ] Open task

> A quote with **bold text**
> and a second line.
>
> > A nested quote.

## Code

```rust
fn main() {
    let n = 42; // a comment
    println!("Hello, {}!", n);
}
```

    indented code
    second line

## Table

| Name | Quantity | Price |
|:-----|:-----:|------:|
| Apples | 3 | €1.20 |
| Pears with a long name | 12 | €0.80 |
| `a\|b` | **7** | [Price list](https://example.org/preis) |

## Images

![Logo](logo.svg) and a raster image ![Raster](raster.png)

![Not loaded](https://example.org/bild.png)

---

#### Heading 4
##### Heading 5
###### Heading 6

Escapes: \*no star\*, &copy; &amp; &#x263A;. HTML in text: <b>bold</b>, <kbd>Ctrl</kbd>+<kbd>C</kbd>,<br>a new line.

[ref]: https://example.org/ref
A [reference][ref] and [ref] as a short form.

שלום עולם, זהו טקסט בעברית.

مرحبا بالعالم، هذا نص عربي.
"#;

/// SAMPLE_HTML in English
pub(crate) const SAMPLE_HTML_EN: &str = r#"<h1>HTML viewer</h1>
<p>A paragraph with <i>italic</i>, <b>bold</b>, <s>strikethrough</s>, <u>underlined</u>,
<code>code</code>, <span style="color:#c00000">red</span> and a
<a href="https://example.org/html">link</a>.</p>
<h2>Lists</h2>
<ul><li>First item</li><li>Second item<ul><li>Sub-item</li></ul></li></ul>
<ol start="5"><li>Five</li><li>Six</li></ol>
<blockquote><p>A quote.</p></blockquote>
<pre><code class="language-json">{ "name": "jflib", "n": 1 }</code></pre>
<table>
<thead><tr><th>Column A</th><th align="right">Column B</th></tr></thead>
<tbody><tr><td>one</td><td align="right">1</td></tr><tr><td>two</td><td align="right">22</td></tr></tbody>
</table>
<hr>
<p><img src="logo.svg" alt="Logo" width="64"> An image in the text.</p>
<h4>Heading 4</h4>
<p>End.</p>
"#;

/// SAMPLE_FORM in English
pub(crate) const SAMPLE_FORM_EN: &str = r#"<h2>Sign in</h2>
<form action="/login" method="post">
<p><label for="user">Name</label> <input id="user" name="user" placeholder="User name" size="24"></p>
<p><label>Password <input type="password" name="pw"></label></p>
<p><label><input type="checkbox" name="keep" value="ja"> Stay signed in</label></p>
<p>Color: <label><input type="radio" name="farbe" value="rot" checked> Red</label>
<label><input type="radio" name="farbe" value="gruen"> Green</label>
<label><input type="radio" name="farbe" value="blau"> Blue</label></p>
<p><label for="land">Country</label> <select id="land" name="land"><option value="de">Germany</option>
<option value="at" selected>Austria</option><option value="ch">Switzerland</option></select></p>
<p><label for="text">Message</label><br><textarea id="text" name="text" rows="3" cols="30">
Hello</textarea></p>
<p><label for="ort">עיר</label> <input id="ort" name="ort" size="10"></p>
<input type="hidden" name="token" value="abc123">
<p><button type="submit" name="aktion" value="senden">Send</button> <input type="submit" name="aktion" value="Save">
<input type="reset" value="Reset"> <a href="https://example.org/hilfe">Help</a></p>
</form>
<h2>Images from the network</h2>
<p><img src="http://example.org/logo.svg" alt="Logo from the network" width="96" height="48">
<img src="https://example.org/raster.png" alt="Raster from the network">
<img src="https://example.org/fehlt.png" alt="Missing"> and local <img src="logo.svg" alt="Local logo" width="48"></p>
"#;

/// SAMPLE_CSS in English (the style sheet and the class names are the same)
pub(crate) const SAMPLE_CSS_EN: &str = r#"<style>
h1 { color: #0b3d91; border-bottom: 3px solid #0b3d91; padding-bottom: 4px }
.kopf { background: #0b3d91; color: #ffffff; padding: 6px 12px; margin-bottom: 4px }
.kopf a { color: #ffd166 }
.karte { border: 1px solid #b8c4d2; background: #f4f7fb; padding: 8px 14px; margin: 4px 0 }
.karte p { margin: 2px 0 }
.hinweis { border-left: 4px solid #e09f3e; background: #fff4e0; padding: 4px 10px }
#wichtig { color: #b00020; font-weight: bold }
ul.menu { background: #e8eef6; padding: 4px 8px }
ul.menu > li { color: #0b3d91 }
.dunkel { background: #222831; color: #eeeeee; padding: 6px 10px }
.dunkel .hell { color: #ffd166; font-style: italic }
table.daten { border: 2px solid #8899aa }
tr.kopfzeile { background: #dde6f0 }
.rechts { text-align: right }
.versteckt { display: none }
</style>
<div class="kopf"><p>Network lab · <a href="https://example.org/start">Start</a></p></div>
<h1>A styled page</h1>
<div class="karte"><p>A card with a border, padding and a background.</p><p id="wichtig">An important sentence in red.</p><p>A <a href="https://example.org/karte">link</a> in the card.</p></div>
<div class="hinweis"><p>A note with a colored border on the left.</p></div>
<ul class="menu"><li>Router</li><li>Switch</li><li>Server</li></ul>
<div class="dunkel"><p>A dark box with <span class="hell">light</span> text.</p></div>
<table class="daten"><tr class="kopfzeile"><th>Device</th><th>Address</th></tr><tr><td>R1</td><td>10.0.0.1</td></tr></table>
<p class="rechts">Aligned to the right.</p>
<p class="versteckt">Invisible.</p>
<div class="karte"><form action="/los"><p><label for="q">Search</label> <input id="q" name="q" size="16"> <button>Go</button></p></form></div>
"#;

/// SAMPLE_DOM in English (the ids are the same)
pub(crate) const SAMPLE_DOM_EN: &str = r#"<h2 id="kopf">Page with a script</h2>
<p id="stand">Count: <b>0</b> steps</p>
<ul id="liste"><li>First entry</li><li class="zwei">Second entry</li></ul>
<form action="/suche"><p id="zeile"><label for="name">Name</label> <input id="name" name="name" size="16">
<label><input type="checkbox" name="merken"> Remember</label></p></form>
<p id="hinweis">A note that disappears at the fourth step.</p>
<p><a id="link" href="https://example.org/eins">First link</a></p>
<div id="kasten"></div>
"#;
