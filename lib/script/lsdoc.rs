// lsdoc -- the descriptions of the built-ins and of the standard library for the language
// services (ls.rs): parameters and a short German text per name, in our own words
//
//   DOCS                 (name, receiver, parameters, description): "str.split" with the
//                        receiver "s" is the method s.split(sep, n); an empty receiver is a
//                        function of the module (str.from_code, math.sqrt) or a constant
//                        (math.pi, parameters None)
//   doc(name)            the entry of a name ("print", "str.split", "math.pi")
//   methods(ty)          the methods of a type ("str", "list", "map", "bytes")
//   MODULES              the names that hold functions (str, list, map, bytes, math, json,
//                        int, float)
//
// coretest (group scriptls) checks that every name of script::library() has an entry.

/// a name of the library: name, the receiver of a method ("" for module functions and
/// constants), the parameters (None: a constant), a short description
pub(crate) struct Doc {
    pub name: &'static str,
    pub recv: &'static str,
    pub params: Option<&'static str>,
    pub text: &'static str,
}

const fn d(name: &'static str, recv: &'static str, params: &'static str, text: &'static str) -> Doc {
    Doc { name, recv, params: Some(params), text }
}

const fn c(name: &'static str, text: &'static str) -> Doc {
    Doc { name, recv: "", params: None, text }
}

/// the modules of the library (their functions are module.name)
pub(crate) const MODULES: &[&str] = &["str", "list", "map", "bytes", "math", "json", "int", "float"];

/// see the module comment
pub(crate) const DOCS: &[Doc] = &[
    d("print", "", "x, ..", "Schreibt die Werte mit Leerzeichen dazwischen als eine Zeile in die Ausgabe."),
    d("len", "", "x", "Die Länge: Zeichen eines str, Bytes von bytes, Einträge einer list oder map."),
    d("type", "", "x", "Der Typ als Text: nil, bool, int, float, str, bytes, list, map oder fn."),
    d("str", "", "x", "Der Wert als Text, wie print ihn schreibt."),
    d("int", "", "x", "Eine ganze Zahl: schneidet einen float ab, liest Text wie \"42\" oder \"0x1F\"."),
    d("float", "", "x", "Der Wert als Gleitkommazahl (auch aus Text)."),
    d("bytes", "", "x, ..", "Neue Bytes: leer, aus Text (UTF-8), aus einer Liste von Zahlen oder aus einzelnen Zahlen."),
    // str
    d("str.len", "s", "", "Die Zahl der Zeichen."),
    d("str.slice", "s", "start, end", "Der Teil von start bis vor end (Zeichen); negative Werte zählen vom Ende."),
    d("str.split", "s", "sep, n", "Zerlegt den Text an sep in eine Liste, höchstens n Teile; ohne sep an Leerraum."),
    d("str.join", "sep", "list", "Die Werte der Liste als ein Text, mit sep dazwischen."),
    d("str.trim", "s", "chars", "Ohne Leerraum an beiden Enden (oder ohne die Zeichen aus chars)."),
    d("str.trim_start", "s", "chars", "Ohne Leerraum am Anfang (oder ohne die Zeichen aus chars)."),
    d("str.trim_end", "s", "chars", "Ohne Leerraum am Ende (oder ohne die Zeichen aus chars)."),
    d("str.find", "s", "sub, from", "Wo sub zuerst vorkommt (ab Zeichen from), nil wenn nirgends."),
    d("str.find_last", "s", "sub", "Wo sub zuletzt vorkommt, nil wenn nirgends."),
    d("str.contains", "s", "sub", "Ob sub im Text vorkommt."),
    d("str.starts_with", "s", "p", "Ob der Text mit p anfängt."),
    d("str.ends_with", "s", "p", "Ob der Text mit p aufhört."),
    d("str.count", "s", "sub", "Wie oft sub vorkommt, ohne Überlappung."),
    d("str.replace", "s", "from, to, n", "Setzt to für from ein, überall oder nur n Mal."),
    d("str.upper", "s", "", "Der Text in Großbuchstaben (ß wird SS)."),
    d("str.lower", "s", "", "Der Text in Kleinbuchstaben."),
    d("str.repeat", "s", "n", "Der Text n Mal hintereinander."),
    d("str.chars", "s", "", "Die Zeichen als Liste von Texten."),
    d("str.bytes", "s", "", "Der Text als bytes in UTF-8."),
    d("str.code", "s", "i", "Der Unicode-Wert des Zeichens an Stelle i (ohne i: des ersten)."),
    d("str.from_code", "", "n, ..", "Ein Text aus den Zeichen mit diesen Unicode-Werten."),
    d("str.format", "s", "args, ..", "Setzt die Argumente für {}, {0} oder {name} ein; nach einem Doppelpunkt Breite, Ausrichtung und Genauigkeit."),
    d("str.hash", "s", "", "Ein fester 32-Bit-Hash (FNV-1a) der UTF-8-Bytes, gleich auf jedem Rechner."),
    // list
    d("list.len", "l", "", "Die Zahl der Werte."),
    d("list.push", "l", "x, ..", "Hängt die Werte hinten an."),
    d("list.pop", "l", "", "Nimmt den letzten Wert heraus und gibt ihn zurück (nil bei leerer Liste)."),
    d("list.insert", "l", "i, x", "Fügt x an Stelle i ein (0 bis len)."),
    d("list.remove", "l", "i", "Nimmt den Wert an Stelle i heraus und gibt ihn zurück."),
    d("list.slice", "l", "start, end", "Eine neue Liste von start bis vor end; negative Werte zählen vom Ende."),
    d("list.reverse", "l", "", "Dreht die Reihenfolge um (in der Liste selbst) und gibt sie zurück."),
    d("list.sort", "l", "cmp", "Sortiert stabil (in der Liste selbst); cmp(a, b) gibt eine Zahl oder true für a vor b."),
    d("list.map", "l", "f", "Eine neue Liste mit f(x) für jeden Wert."),
    d("list.filter", "l", "f", "Eine neue Liste mit den Werten, für die f(x) wahr ist."),
    d("list.reduce", "l", "f, init", "Faltet die Liste mit f(acc, x) zusammen, ab init (ohne init: ab dem ersten Wert)."),
    d("list.contains", "l", "x", "Ob x in der Liste steht (Vergleich mit ==)."),
    d("list.index", "l", "x", "Die erste Stelle von x, nil wenn es fehlt."),
    d("list.join", "l", "sep", "Die Werte als ein Text, mit sep dazwischen."),
    // map
    d("map.len", "m", "", "Die Zahl der Einträge."),
    d("map.keys", "m", "", "Die Schlüssel als Liste, in der Reihenfolge des Einfügens."),
    d("map.values", "m", "", "Die Werte als Liste, in der Reihenfolge des Einfügens."),
    d("map.entries", "m", "", "Die Einträge als Liste von [Schlüssel, Wert]."),
    d("map.has", "m", "k", "Ob es den Schlüssel k gibt."),
    d("map.get", "m", "k, default", "Der Wert zu k, default wenn der Schlüssel fehlt."),
    d("map.delete", "m", "k", "Entfernt den Eintrag zu k; ob es ihn gab."),
    d("map.merge", "m", "other, ..", "Eine neue Map mit den Einträgen aller Maps; spätere Werte gewinnen."),
    // bytes
    d("bytes.len", "b", "", "Die Zahl der Bytes."),
    d("bytes.slice", "b", "start, end", "Die Bytes von start bis vor end; negative Werte zählen vom Ende."),
    d("bytes.hex", "b", "", "Die Bytes als Hex-Text."),
    d("bytes.from_hex", "", "hex", "Bytes aus einem Hex-Text."),
    d("bytes.text", "b", "lossy", "Die Bytes als Text (UTF-8); lossy ersetzt ungültige Folgen statt zu werfen."),
    d("bytes.hash", "b", "", "Ein fester 32-Bit-Hash (FNV-1a) der Bytes."),
    // math
    d("math.abs", "", "x", "Der Betrag (ein int bleibt ein int)."),
    d("math.min", "", "a, ..", "Der kleinste der Werte (oder einer Liste)."),
    d("math.max", "", "a, ..", "Der größte der Werte (oder einer Liste)."),
    d("math.clamp", "", "x, lo, hi", "x begrenzt auf lo bis hi."),
    d("math.floor", "", "x", "Abgerundet auf eine ganze Zahl."),
    d("math.ceil", "", "x", "Aufgerundet auf eine ganze Zahl."),
    d("math.round", "", "x", "Gerundet, Hälften weg von der Null."),
    d("math.trunc", "", "x", "Die Nachkommastellen abgeschnitten."),
    d("math.sqrt", "", "x", "Die Quadratwurzel."),
    d("math.pow", "", "x, y", "x hoch y."),
    d("math.exp", "", "x", "e hoch x."),
    d("math.log", "", "x", "Der natürliche Logarithmus."),
    d("math.log2", "", "x", "Der Logarithmus zur Basis 2."),
    d("math.log10", "", "x", "Der Logarithmus zur Basis 10."),
    d("math.sin", "", "x", "Der Sinus (x im Bogenmaß)."),
    d("math.cos", "", "x", "Der Kosinus (x im Bogenmaß)."),
    d("math.is_nan", "", "x", "Ob x keine Zahl (nan) ist."),
    d("math.is_finite", "", "x", "Ob x endlich ist (weder inf noch nan)."),
    c("math.pi", "Die Kreiszahl π."),
    c("math.e", "Die Eulersche Zahl e."),
    c("math.tau", "2π, ein voller Kreis im Bogenmaß."),
    c("math.inf", "Unendlich als float."),
    c("math.nan", "Keine Zahl (nan) als float."),
    // json, int, float
    d("json.encode", "", "v, indent", "Der Wert als JSON-Text: kompakt oder mit indent Leerzeichen je Ebene."),
    d("json.decode", "", "text", "Die Werte aus einem JSON-Text (Objekte werden Maps in ihrer Reihenfolge)."),
    d("int.parse", "", "s, base", "Die ganze Zahl in s (auch 0x, 0b, 0o), nil wenn s keine ist."),
    c("int.max", "Die größte ganze Zahl."),
    c("int.min", "Die kleinste ganze Zahl."),
    d("float.parse", "", "s", "Die Gleitkommazahl in s, nil wenn s keine ist."),
];

/// the entry of a name
pub(crate) fn doc(name: &str) -> Option<&'static Doc> {
    DOCS.iter().find(|x| x.name == name)
}

/// the methods of type ty (entries with a receiver)
pub(crate) fn methods(ty: &str) -> impl Iterator<Item = &'static Doc> + '_ {
    DOCS.iter().filter(move |x| !x.recv.is_empty() && x.name.split_once('.').is_some_and(|(t, _)| t == ty))
}
