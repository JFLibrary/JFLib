// settings -- typed settings: a schema of keys (type, default, limits, label) over a settings
// file (Ini) with a format version and migrations
//
//   const DARK: Key<bool> = Key::bool("view.dark", false).label("Dunkles Design");
//   const ZOOM: Key<i64> = Key::int("view.zoom", 100, 10, 800).label("Zoom (%)").step(10);
//   static SCHEMA: Schema = Schema::new(2, &[DARK.d, ZOOM.d]).sections(&[("view", "Ansicht")])
//       .migrate(migrate);
//   let mut s = Settings::open("config://settings.ini", &SCHEMA);
//   let zoom: i64 = s.get(&ZOOM);   s.set(&ZOOM, 120);   s.save();
//
// Key types: Key<bool> (Key::bool), Key<i64> (Key::int: min..max, step), Key<f64> (Key::num:
// min..max, decimals), Key<String> (Key::text: at most max bytes of UTF-8), Key<usize>
// (Key::choice: an index into (stored word, label) items; the file keeps the word),
// Key<u32> (Key::color: 0xRRGGBB; the file "#RRGGBB").  Builders: label(s), help(s),
// hidden() (not on the settings page), restart() (takes effect after a restart: the page
// says so), step(n) (int: the number box's step).
//
//   Settings::open(path, &schema)   load, migrate, read (a missing file: the defaults)
//   Settings::new(&schema)          only in memory (save does nothing)
//   Settings::from_ini(ini, &schema)  in memory, from an Ini (tests)
//   s.get(&key) -> T   s.text(&key) -> &str   s.set(&key, v) -> changed   s.reset(&key)
//   s.reset_all()   s.is_default(&key)   s.save() -> ok   s.dirty()   s.revision()
//   s.ini() / s.ini_mut()   the file for free-form data the schema does not describe (a
//                           list of recent files); ini_mut marks the settings dirty
//   s.file_version()        the version the file had (0: none or older than versions)
//   s.schema(), s.at(i), s.set_at(i, val), s.default_at(i)   by schema index (the page)
//   rename(&mut ini, "old.key", "new.key")   for migrations
//
// Names are "section.key" (no '.': the top of the file).  The file keeps only values that
// differ from the default (setting the default removes the line), so a new default reaches
// users who never changed it; comments, unknown keys and other sections survive (Ini).
// The format version is "version=N" at the top: a file of an older version (no line: 0)
// runs migrate(from, &mut ini) once before its values are read (rename keys, convert units)
// and is saved with the new version by the next save; a missing file needs no migration; a
// file of a newer version is read as far as the schema goes and keeps its number.  A value
// the schema cannot use (not a number, an unknown choice, a bad colour) reads as the default
// and a WARN goes to the log; numbers out of range are clamped, texts cut to their maximum.
// Keys that are not in the schema still work through get / set (read from the file each
// time) but are not on the page.

use alloc::string::String;
use alloc::vec::Vec;
use core::marker::PhantomData;

use crate::ini::Ini;
use crate::num::{f64_format, f64_parse, parse_i64};

/// Desc::flags: not on the settings page
pub const KF_HIDDEN: u32 = 1;
/// Desc::flags: takes effect after a restart (the page says so)
pub const KF_RESTART: u32 = 2;

/// what a key holds, with its default and limits
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Bool(bool),
    Int { def: i64, min: i64, max: i64, step: i64 },
    Num { def: f64, min: f64, max: f64, dec: u32 },
    Text { def: &'static str, max: usize },
    Choice { def: usize, items: &'static [(&'static str, &'static str)] },
    Color(u32),
}

/// a key of a schema (Key::d)
#[derive(Clone, Copy, Debug)]
pub struct Desc {
    pub name: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub flags: u32,
    pub kind: Kind,
}

impl Desc {
    /// (section, key) of the name
    pub fn split(&self) -> (&'static str, &'static str) {
        split(self.name)
    }

    /// the default value
    pub fn default_val(&self) -> Val {
        match self.kind {
            Kind::Bool(d) => Val::Bool(d),
            Kind::Int { def, .. } => Val::Int(def),
            Kind::Num { def, .. } => Val::Num(def),
            Kind::Text { def, .. } => Val::Text(String::from(def)),
            Kind::Choice { def, .. } => Val::Choice(def),
            Kind::Color(d) => Val::Color(d & 0xFF_FFFF),
        }
    }

    /// v made valid for this key (clamped, cut, the default when it is of another type)
    pub fn fit(&self, v: Val) -> Val {
        match (self.kind, v) {
            (Kind::Bool(_), Val::Bool(b)) => Val::Bool(b),
            (Kind::Int { min, max, .. }, Val::Int(x)) => Val::Int(x.clamp(min, max.max(min))),
            (Kind::Num { min, max, .. }, Val::Num(x)) if !x.is_nan() => Val::Num(x.clamp(min, max.max(min))),
            (Kind::Text { max, .. }, Val::Text(mut s)) => {
                cut(&mut s, max);
                Val::Text(s)
            }
            (Kind::Choice { items, .. }, Val::Choice(i)) if i < items.len() => Val::Choice(i),
            (Kind::Color(_), Val::Color(c)) => Val::Color(c & 0xFF_FFFF),
            _ => self.default_val(),
        }
    }

    /// the value written into the file for v
    pub fn format(&self, v: &Val) -> String {
        match v {
            Val::Bool(b) => String::from(if *b { "true" } else { "false" }),
            Val::Int(x) => alloc::format!("{}", x),
            Val::Num(x) => f64_format(*x),
            Val::Text(s) => s.clone(),
            Val::Choice(i) => match self.kind {
                Kind::Choice { items, .. } => String::from(items.get(*i).map_or("", |it| it.0)),
                _ => String::new(),
            },
            Val::Color(c) => alloc::format!("#{:06X}", c & 0xFF_FFFF),
        }
    }

    /// the text of the file read as this key: Some(valid value) (clamped / cut) or None
    pub fn parse(&self, s: &str) -> Option<Val> {
        let t = s.trim();
        let v = match self.kind {
            Kind::Bool(_) => Val::Bool(parse_i64(t.as_bytes())? != 0),
            Kind::Int { .. } => Val::Int(parse_i64(t.as_bytes())?),
            Kind::Num { .. } => {
                let (x, used) = f64_parse(t.as_bytes());
                if used == 0 || used != t.len() || x.is_nan() {
                    return None;
                }
                Val::Num(x)
            }
            Kind::Text { .. } => Val::Text(String::from(s)),
            Kind::Choice { items, .. } => Val::Choice(items.iter().position(|it| it.0.eq_ignore_ascii_case(t))?),
            Kind::Color(_) => {
                let h = t.strip_prefix('#').unwrap_or(t);
                if h.len() != 6 {
                    return None;
                }
                Val::Color(u32::from_str_radix(h, 16).ok()?)
            }
        };
        Some(self.fit(v))
    }
}

/// a value of a key
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Bool(bool),
    Int(i64),
    Num(f64),
    Text(String),
    Choice(usize),
    Color(u32),
}

/// the Rust types a key can have: bool, i64, f64, String, usize (choice), u32 (colour)
pub trait Value: Sized {
    #[doc(hidden)]
    fn from_val(v: &Val) -> Option<Self>;
    #[doc(hidden)]
    fn into_val(self) -> Val;
}

impl Value for bool {
    fn from_val(v: &Val) -> Option<bool> {
        if let Val::Bool(b) = v { Some(*b) } else { None }
    }
    fn into_val(self) -> Val {
        Val::Bool(self)
    }
}

impl Value for i64 {
    fn from_val(v: &Val) -> Option<i64> {
        if let Val::Int(x) = v { Some(*x) } else { None }
    }
    fn into_val(self) -> Val {
        Val::Int(self)
    }
}

impl Value for f64 {
    fn from_val(v: &Val) -> Option<f64> {
        if let Val::Num(x) = v { Some(*x) } else { None }
    }
    fn into_val(self) -> Val {
        Val::Num(self)
    }
}

impl Value for String {
    fn from_val(v: &Val) -> Option<String> {
        if let Val::Text(s) = v { Some(s.clone()) } else { None }
    }
    fn into_val(self) -> Val {
        Val::Text(self)
    }
}

impl Value for usize {
    fn from_val(v: &Val) -> Option<usize> {
        if let Val::Choice(i) = v { Some(*i) } else { None }
    }
    fn into_val(self) -> Val {
        Val::Choice(self)
    }
}

impl Value for u32 {
    fn from_val(v: &Val) -> Option<u32> {
        if let Val::Color(c) = v { Some(*c) } else { None }
    }
    fn into_val(self) -> Val {
        Val::Color(self)
    }
}

/// a typed key: its description (d) and the Rust type of its value
pub struct Key<T> {
    pub d: Desc,
    t: PhantomData<fn() -> T>,
}

impl<T> Clone for Key<T> {
    fn clone(&self) -> Key<T> {
        *self
    }
}
impl<T> Copy for Key<T> {}

const fn key<T>(name: &'static str, kind: Kind) -> Key<T> {
    Key { d: Desc { name, label: name, help: "", flags: 0, kind }, t: PhantomData }
}

impl Key<bool> {
    pub const fn bool(name: &'static str, def: bool) -> Key<bool> {
        key(name, Kind::Bool(def))
    }
}

impl Key<i64> {
    /// a whole number in min..=max (step 1)
    pub const fn int(name: &'static str, def: i64, min: i64, max: i64) -> Key<i64> {
        key(name, Kind::Int { def, min, max, step: 1 })
    }

    /// the step of the number box's arrows and arrow keys
    pub const fn step(self, step: i64) -> Key<i64> {
        let mut k = self;
        if let Kind::Int { def, min, max, .. } = k.d.kind {
            k.d.kind = Kind::Int { def, min, max, step };
        }
        k
    }
}

impl Key<f64> {
    /// a number in min..=max, shown with dec decimals
    pub const fn num(name: &'static str, def: f64, min: f64, max: f64, dec: u32) -> Key<f64> {
        key(name, Kind::Num { def, min, max, dec })
    }
}

impl Key<String> {
    /// a text of at most max bytes (UTF-8)
    pub const fn text(name: &'static str, def: &'static str, max: usize) -> Key<String> {
        key(name, Kind::Text { def, max })
    }
}

impl Key<usize> {
    /// one of items (stored word, label); the value is its index
    pub const fn choice(name: &'static str, def: usize, items: &'static [(&'static str, &'static str)]) -> Key<usize> {
        key(name, Kind::Choice { def, items })
    }
}

impl Key<u32> {
    /// a colour 0xRRGGBB
    pub const fn color(name: &'static str, def: u32) -> Key<u32> {
        key(name, Kind::Color(def))
    }
}

impl<T> Key<T> {
    /// the text of the settings page (default: the name)
    pub const fn label(self, label: &'static str) -> Key<T> {
        let mut k = self;
        k.d.label = label;
        k
    }

    /// a line of explanation below the label
    pub const fn help(self, help: &'static str) -> Key<T> {
        let mut k = self;
        k.d.help = help;
        k
    }

    /// not on the settings page (state the app keeps)
    pub const fn hidden(self) -> Key<T> {
        let mut k = self;
        k.d.flags |= KF_HIDDEN;
        k
    }

    /// takes effect after a restart
    pub const fn restart(self) -> Key<T> {
        let mut k = self;
        k.d.flags |= KF_RESTART;
        k
    }
}

/// the keys of a settings file, its format version, the page's section titles and the
/// migration of older files
pub struct Schema {
    pub version: u32,
    pub keys: &'static [Desc],
    /// (section, title) for the settings page, in this order (sections without a title come
    /// after them, titled by their name)
    pub sections: &'static [(&'static str, &'static str)],
    /// migrate(from, &mut ini): bring a file of version `from` up to `version`
    pub migrate: Option<fn(u32, &mut Ini)>,
}

impl Schema {
    pub const fn new(version: u32, keys: &'static [Desc]) -> Schema {
        Schema { version, keys, sections: &[], migrate: None }
    }

    pub const fn sections(self, sections: &'static [(&'static str, &'static str)]) -> Schema {
        Schema { sections, ..self }
    }

    pub const fn migrate(self, f: fn(u32, &mut Ini)) -> Schema {
        Schema { migrate: Some(f), ..self }
    }

    /// the index of a key name
    pub fn index(&self, name: &str) -> Option<usize> {
        self.keys.iter().position(|d| d.name == name)
    }
}

/// "section.key" -> (section, key); no '.': ("", name)
pub fn split(name: &str) -> (&str, &str) {
    name.split_once('.').unwrap_or(("", name))
}

/// for migrations: move a value from one name to another ("section.key"); false: not there
pub fn rename(ini: &mut Ini, old: &str, new: &str) -> bool {
    let (os, ok) = split(old);
    let Some(v) = ini.get(os, ok).map(String::from) else { return false };
    let (ns, nk) = split(new);
    ini.remove(os, ok);
    ini.set(ns, nk, &v)
}

// cut s to at most max bytes at a character boundary
fn cut(s: &mut String, max: usize) {
    if s.len() > max {
        let mut n = max;
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        s.truncate(n);
    }
}

/// typed settings over a file (see the module header)
pub struct Settings {
    schema: &'static Schema,
    path: String,
    ini: Ini,
    vals: Vec<Val>,
    dirty: bool,
    rev: u32,
    file_version: u32,
}

impl Settings {
    /// the settings of a file (a missing or unreadable file: the defaults)
    pub fn open(path: &str, schema: &'static Schema) -> Settings {
        let exists = plat::file_exists(path) == 1;
        let mut s = Settings::from_ini_ex(Ini::load(path), schema, exists);
        s.path = String::from(path);
        s
    }

    /// settings only in memory (save does nothing)
    pub fn new(schema: &'static Schema) -> Settings {
        Settings::from_ini_ex(Ini::new(), schema, false)
    }

    /// settings in memory, read from an Ini (migrated like a file)
    pub fn from_ini(ini: Ini, schema: &'static Schema) -> Settings {
        Settings::from_ini_ex(ini, schema, true)
    }

    fn from_ini_ex(mut ini: Ini, schema: &'static Schema, existed: bool) -> Settings {
        let mut dirty = false;
        let file_version = ini.get_int("", "version", 0).clamp(0, u32::MAX as i64) as u32;
        if existed && file_version < schema.version {
            if let Some(m) = schema.migrate {
                m(file_version, &mut ini);
            }
            ini.set_int("", "version", schema.version as i64);
            dirty = true;
            plat::log_info("settings", &alloc::format!("migrated from version {} to {}", file_version, schema.version));
        } else if file_version > schema.version {
            plat::log_warn("settings", &alloc::format!("the file has version {}, the program knows {}", file_version, schema.version));
        }
        let vals = schema.keys.iter().map(|d| read(&ini, d)).collect();
        Settings { schema, path: String::new(), ini, vals, dirty, rev: 0, file_version }
    }

    pub fn schema(&self) -> &'static Schema {
        self.schema
    }

    /// the file's path ("" in memory)
    pub fn path(&self) -> &str {
        &self.path
    }

    /// the format version the file had before a migration (0: none / no version line)
    pub fn file_version(&self) -> u32 {
        self.file_version
    }

    /// a value (the key's default when the key is not in the schema and not in the file)
    pub fn get<T: Value>(&self, k: &Key<T>) -> T {
        let v = match self.schema.index(k.d.name) {
            Some(i) => T::from_val(&self.vals[i]),
            None => T::from_val(&read(&self.ini, &k.d)),
        };
        v.or_else(|| T::from_val(&k.d.default_val())).expect("settings: the type of a key")
    }

    /// a text without copying it
    pub fn text(&self, k: &Key<String>) -> &str {
        match self.schema.index(k.d.name).map(|i| &self.vals[i]) {
            Some(Val::Text(s)) => s,
            _ => {
                let (s, n) = k.d.split();
                match self.ini.get(s, n) {
                    Some(t) => t,
                    None => match k.d.kind {
                        Kind::Text { def, .. } => def,
                        _ => "",
                    },
                }
            }
        }
    }

    /// set a value (made valid for the key) -> it changed
    pub fn set<T: Value>(&mut self, k: &Key<T>, v: T) -> bool {
        match self.schema.index(k.d.name) {
            Some(i) => self.set_at(i, v.into_val()),
            None => {
                let v = k.d.fit(v.into_val());
                if read(&self.ini, &k.d) == v {
                    return false;
                }
                write(&mut self.ini, &k.d, &v);
                self.changed();
                true
            }
        }
    }

    /// back to the default (the line leaves the file)
    pub fn reset<T>(&mut self, k: &Key<T>) -> bool {
        match self.schema.index(k.d.name) {
            Some(i) => self.set_at(i, k.d.default_val()),
            None => {
                let (s, n) = k.d.split();
                let gone = self.ini.remove(s, n);
                if gone {
                    self.changed();
                }
                gone
            }
        }
    }

    /// every key of the schema back to its default
    pub fn reset_all(&mut self) -> bool {
        let mut any = false;
        for i in 0..self.schema.keys.len() {
            any |= self.set_at(i, self.schema.keys[i].default_val());
        }
        any
    }

    /// the key has its default value
    pub fn is_default<T>(&self, k: &Key<T>) -> bool {
        match self.schema.index(k.d.name) {
            Some(i) => self.vals[i] == self.default_at(i),
            None => read(&self.ini, &k.d) == k.d.default_val(),
        }
    }

    /// the value of schema key i
    pub fn at(&self, i: usize) -> &Val {
        &self.vals[i]
    }

    /// the default of schema key i
    pub fn default_at(&self, i: usize) -> Val {
        self.schema.keys[i].default_val()
    }

    /// set schema key i (made valid) -> it changed
    pub fn set_at(&mut self, i: usize, v: Val) -> bool {
        let d = &self.schema.keys[i];
        let v = d.fit(v);
        if self.vals[i] == v {
            return false;
        }
        write(&mut self.ini, d, &v);
        self.vals[i] = v;
        self.changed();
        true
    }

    fn changed(&mut self) {
        self.dirty = true;
        self.rev = self.rev.wrapping_add(1);
    }

    /// unsaved changes
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// counts every change (redraw or apply what depends on the settings when it moved)
    pub fn revision(&self) -> u32 {
        self.rev
    }

    /// the file's data (free-form sections the schema does not describe)
    pub fn ini(&self) -> &Ini {
        &self.ini
    }

    /// change free-form data (marks the settings dirty; keys of the schema: use set)
    pub fn ini_mut(&mut self) -> &mut Ini {
        self.changed();
        &mut self.ini
    }

    /// write the file when something changed (atomic; in memory: nothing) -> ok
    pub fn save(&mut self) -> bool {
        if !self.dirty {
            return true;
        }
        if self.path.is_empty() {
            self.dirty = false;
            return true;
        }
        if self.ini.get("", "version").is_none() || self.file_version < self.schema.version {
            self.ini.set_int("", "version", self.schema.version.max(self.file_version) as i64);
        }
        let ok = self.ini.save(&self.path);
        if ok {
            self.dirty = false;
        } else {
            plat::log_warn("settings", &alloc::format!("could not write {}", self.path));
        }
        ok
    }
}

// the key's value in the file (invalid: the default and a WARN)
fn read(ini: &Ini, d: &Desc) -> Val {
    let (s, n) = d.split();
    match ini.get(s, n) {
        None => d.default_val(),
        Some(t) => match d.parse(t) {
            Some(v) => v,
            None => {
                plat::log_warn("settings", &alloc::format!("{}: \"{}\" is not a valid value, the default is used", d.name, t));
                d.default_val()
            }
        },
    }
}

// the key's line: removed at the default, else written
fn write(ini: &mut Ini, d: &Desc, v: &Val) {
    let (s, n) = d.split();
    if *v == d.default_val() {
        ini.remove(s, n);
    } else {
        ini.set(s, n, &d.format(v));
    }
}
