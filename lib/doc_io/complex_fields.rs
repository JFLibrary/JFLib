// Word complex fields: normalize supported paragraph-local instructions to fldSimple.
// ECMA-376 17.16.18/19/45; unknown, nested or cross-paragraph fields stay preserved.
use crate::{Error, W, XML, container, format, read_run, unsupported};
use alloc::{string::String, vec::Vec};
use base::xml::{Name, Node};

struct Field {
    instruction: String,
    result: Vec<Node>,
    separated: bool,
}
fn marker(n: &Node) -> Result<&str, Error> {
    if !n.children.is_empty() || !n.text.is_empty() {
        return Err(unsupported("Komplexe Feldattribute"));
    }
    field_attributes(n, "fldCharType")?;
    n.attr(W, "fldCharType").ok_or(Error::Invalid)
}
// Generated results are recalculated on import, so dirty is naturally satisfied.
// A locked field would require retained cache/lock state and must stay preserved.
pub(crate) fn field_attributes(n: &Node, primary: &str) -> Result<(), Error> {
    for (name, value) in &n.attrs {
        if name.ns != W {
            return Err(unsupported("Feldattribute"));
        }
        if name.local == primary {
            continue;
        }
        let enabled = match value.as_str() {
            "1" | "true" | "on" => true,
            "0" | "false" | "off" => false,
            _ => return Err(unsupported("Feldattribute")),
        };
        if name.local != "dirty" && !(name.local == "fldLock" && !enabled) {
            return Err(unsupported("Gesperrtes Feld oder unbekanntes Feldattribut"));
        }
    }
    n.attr(W, primary).ok_or(Error::Invalid)?;
    Ok(())
}
fn simple(field: Field) -> Node {
    Node {
        name: Name {
            ns: String::from(W),
            local: String::from("fldSimple"),
        },
        attrs: alloc::vec![(
            Name {
                ns: String::from(W),
                local: String::from("instr")
            },
            field.instruction
        )],
        text: String::new(),
        children: field.result,
    }
}

pub(crate) fn normalize(paragraph: &Node) -> Result<Vec<Node>, Error> {
    let mut out = Vec::new();
    let mut field: Option<Field> = None;
    for node in &paragraph.children {
        if !node.is(W, "r") {
            if field.is_some() {
                return Err(unsupported("Verschachteltes oder strukturiertes Feld"));
            }
            out.push(node.clone());
            continue;
        }
        container(node, W, "r")?;
        if !node
            .children
            .iter()
            .any(|n| n.is(W, "fldChar") || n.is(W, "instrText"))
        {
            if let Some(f) = &mut field {
                if !f.separated {
                    return Err(unsupported("Sichtbarer Text in Feldanweisung"));
                }
                read_run(node)?;
                f.result.push(node.clone());
            } else {
                out.push(node.clone());
            }
            continue;
        }
        let mut props = None;
        for (index, child) in node.children.iter().enumerate() {
            if child.is(W, "rPr") {
                if index != 0 {
                    return Err(Error::Invalid);
                }
                format::read_run(child)?;
                props = Some(child.clone());
                continue;
            }
            if child.is(W, "fldChar") {
                match marker(child)? {
                    "begin" if field.is_none() => {
                        field = Some(Field {
                            instruction: String::new(),
                            result: Vec::new(),
                            separated: false,
                        })
                    }
                    "separate" if field.as_ref().is_some_and(|f| !f.separated) => {
                        field.as_mut().unwrap().separated = true
                    }
                    "end" if field.is_some() => out.push(simple(field.take().unwrap())),
                    _ => return Err(unsupported("Feldgrenzen oder verschachtelte Felder")),
                }
                continue;
            }
            if child.is(W, "instrText") {
                let f = field
                    .as_mut()
                    .filter(|f| !f.separated)
                    .ok_or(Error::Invalid)?;
                if !child.children.is_empty()
                    || child.attrs.iter().any(|(n, v)| {
                        n.ns != XML
                            || n.local != "space"
                            || !matches!(v.as_str(), "preserve" | "default")
                    })
                {
                    return Err(unsupported("Feldanweisung"));
                }
                if f.instruction.len() + child.text.len() > 16 << 10 {
                    return Err(Error::Limit);
                }
                f.instruction.push_str(&child.text);
                continue;
            }
            // Keep run properties attached to every text fragment when a single run
            // includes field boundaries and adjacent ordinary text.
            let mut run = Node {
                name: node.name.clone(),
                attrs: Vec::new(),
                text: String::new(),
                children: Vec::new(),
            };
            if let Some(props) = &props {
                run.children.push(props.clone());
            }
            run.children.push(child.clone());
            if let Some(f) = &mut field {
                if !f.separated {
                    return Err(unsupported("Sichtbarer Text in Feldanweisung"));
                }
                read_run(&run)?;
                f.result.push(run);
            } else {
                out.push(run);
            }
        }
        // Empty runs can contain insertion formatting and must not disappear.
        if node.children.is_empty() || node.children.iter().all(|n| n.is(W, "rPr")) {
            if let Some(f) = &mut field {
                if f.separated {
                    f.result.push(node.clone());
                }
            } else {
                out.push(node.clone());
            }
        }
    }
    if field.is_some() {
        return Err(unsupported(
            "Absatzübergreifendes oder unvollständiges Feld",
        ));
    }
    Ok(out)
}
