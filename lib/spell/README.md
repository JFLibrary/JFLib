# jflib-spell

Spell checking and hyphenation for [JFLib](https://jflib.org). The crate reads Hunspell
dictionaries itself, the `.aff` and `.dic` files that LibreOffice and the Linux distributions
ship, in plain Rust without the standard library and without `unsafe` code. Where the system has
a spell checker of its own, it uses that one first. A personal word list per language and
hyphenation after Liang's algorithm complete it.

## How it is used

Most apps never call it by name. A text box with `TB_SPELL` underlines misspelled words, and a
right click offers suggestions and lets the user add the word to the dictionary or ignore it;
`ui.speller()` hands out the `Speller` behind it. The crate's own types are there for checking
text outside a text box:

```rust
use jflib::spell::{Speller, SRC_ALL};

if let Some(mut sp) = Speller::open("de-DE", SRC_ALL) {
    for (start, end) in sp.misspelled(&text) {
        let better = sp.suggest(&text[start..end], 5); // best first
        // ...
    }
}
```

`spell::find` and `spell::languages` tell where the checker for a language lives and which
languages there are. A checker looks in three places, in this order: the system (`SRC_SYSTEM`),
the user's dictionaries in `config://hunspell` (`SRC_USER`), and the dictionaries the app ships
in `app://hunspell` (`SRC_APP`). A dictionary named after the tag serves it (`de_DE.aff` with
`de_DE.dic` beside it for `de-DE`), and so does a variant (`de_DE_frami`), the bare language
(`de`) or the same language in another region (`de_AT`). Headless runs and tests use only the
app's own dictionaries, so their results never depend on the machine.

Words the user adds go to the personal list, one word per line in
`config://spell/<language>.dic`; a word added in lower case is also right Capitalised and in
UPPER case. Ignored words last for the session. Neither ever reaches the system's checker.
Suggestions from a Hunspell dictionary can take a few tenths of a second for a long German
compound, so `Speller::suggester()` gives a `Suggester` that makes them on another thread.

`spell::Dictionary` is the Hunspell dictionary itself. It follows Hunspell 1.7, with its affix
rules, compounds and case rules, and was compared with real dictionaries (de_DE, en_US, en_GB
among them). It has no clock limits, so its answers never depend on the machine's speed.
Morphology and phonetic suggestions are not supported.

`spell::Hyphenator` hyphenates with libhyphen's dictionaries (`hyph_de_DE.dic`, as LibreOffice
has them) or with TeX's patterns. On the 21 dictionaries of the Freedesktop SDK its results equal
libhyphen 2.8's for 214,000 words. JFLib's UI uses it for `TX_HYPHENATE` texts and
`TB_HYPHENATE` boxes.

## Platforms

- **Windows:** the system's checker (the Spell Checking API) for the languages whose language
  packs are installed, then Hunspell files in `DICPATH`. Hyphenation patterns also come from
  LibreOffice's dictionary extensions.
- **macOS:** `NSSpellChecker`, then Hunspell files in `~/Library/Spelling`, `/Library/Spelling`
  and `DICPATH`.
- **iOS:** `UITextChecker`.
- **Linux:** no system checker; the crate reads the Hunspell dictionaries of `/usr/share/hunspell`,
  `myspell`, `~/.local/share/hunspell` and `DICPATH` itself, and the hyphenation patterns of
  `/usr/share/hyphen`.
- **Browser and Android:** the dictionaries of the user and of the app.

## How to get it

Through the one dependency [`jflib`](https://crates.io/crates/jflib), as `jflib::spell`. All of
JFLib's crates share one version and are released together.

## What's new in 0.2.0

On macOS and iOS the crate now uses the system's own spell checker, and on macOS it finds
Hunspell dictionaries in the Spelling folders of the system and the user. Everything is in the
[changelog](https://docs.rs/crate/jflib-spell/latest/source/CHANGELOG.md).

## Documentation

- [Spell checking in text boxes](https://docs.rs/jflib/latest/jflib/guide/index.html#5-widgets)
  and [hyphenation](https://docs.rs/jflib/latest/jflib/guide/index.html#11-styles-theme-fonts)
  in JFLib's guide
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#spell): every public
  item of `spell`, also on [docs.rs](https://docs.rs/jflib-spell)

## License

MIT or Apache-2.0, at your choice.
