// out -- the XML of Visio 2003-2010 that the binary objects become (vss/mod.rs): a writer of
// elements, attributes and cells with a limit on its size

use alloc::string::String;

use crate::props::escape;
use crate::vsdx::Error;

pub(super) struct Out {
    pub s: String,
    max: usize,
}

impl Out {
    pub fn new(max: usize) -> Out {
        Out { s: String::with_capacity(1 << 16), max }
    }

    fn check(&self) -> Result<(), Error> {
        if self.s.len() > self.max {
            return Err(Error::Limit("file"));
        }
        Ok(())
    }

    /// `<name` (attributes follow, then `close` or `end_empty`)
    pub fn open(&mut self, name: &str) {
        self.s.push('<');
        self.s.push_str(name);
    }

    /// ` name='value'` (escaped)
    pub fn attr(&mut self, name: &str, value: &str) {
        self.s.push(' ');
        self.s.push_str(name);
        self.s.push_str("='");
        escape(value, &mut self.s);
        self.s.push('\'');
    }

    /// `>`
    pub fn close(&mut self) -> Result<(), Error> {
        self.s.push('>');
        self.check()
    }

    /// `/>`
    pub fn end_empty(&mut self) -> Result<(), Error> {
        self.s.push_str("/>");
        self.check()
    }

    /// `</name>`
    pub fn end(&mut self, name: &str) -> Result<(), Error> {
        self.s.push_str("</");
        self.s.push_str(name);
        self.s.push('>');
        self.check()
    }

    /// text content (escaped)
    pub fn text(&mut self, t: &str) -> Result<(), Error> {
        escape(t, &mut self.s);
        self.check()
    }

    /// raw markup the caller made well-formed
    pub fn raw(&mut self, t: &str) -> Result<(), Error> {
        self.s.push_str(t);
        self.check()
    }

    /// a cell element: `<name F='..' Unit='..'>value</name>`
    pub fn cell(&mut self, name: &str, value: &str, unit: Option<&str>, formula: Option<&str>) -> Result<(), Error> {
        self.open(name);
        if let Some(u) = unit {
            self.attr("Unit", u);
        }
        if let Some(f) = formula {
            self.attr("F", f);
        }
        self.s.push('>');
        escape(value, &mut self.s);
        self.end(name)
    }
}
