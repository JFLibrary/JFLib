//! Portable floating-table placement. Coordinates and text distances are document twips.
use crate::{Document, Error, Table};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TableHorizontal { Page, #[default] Margin, Column }
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TableVertical { Page, Margin, #[default] Paragraph }
/// A floating table remains anchored to `Table.before`; its content is a normal cell story.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TablePosition {
    pub horizontal: TableHorizontal,
    pub vertical: TableVertical,
    pub x: i32,
    pub y: i32,
    /// Top, right, bottom, left distances from surrounding text.
    pub distance: [u16;4],
    pub overlap: bool,
}
impl Default for TablePosition {
    fn default()->Self { Self{horizontal:TableHorizontal::Margin,vertical:TableVertical::Paragraph,x:0,y:0,distance:[120;4],overlap:false} }
}
impl Document {
    /// Absolute horizontal origin, including a floating ancestor's displacement.
    pub fn table_x(&self, table:&Table)->Result<u32,Error> {
        if let Some(p)=table.position {
            let base=if p.horizontal==TableHorizontal::Page {0}else{self.page.left as i32};
            u32::try_from(base.checked_add(p.x).ok_or(Error::Limit)?).map_err(|_|Error::Invalid)
        } else { Ok(self.table_bounds(table.id)?.0) }
    }
    pub(crate) fn validate_table_position(&self,t:&Table)->Result<(),Error> {
        if let Some(p)=t.position {
            if self.table_parent(t.id).is_some() || p.distance.iter().any(|v|*v>31680)
                || !(-31680..=31680).contains(&p.x) || !(0..=31680).contains(&p.y) {return Err(Error::Invalid);}
            let x=self.table_x(t)?;
            if x.checked_add(t.width()).is_none_or(|r|r>self.page.width){return Err(Error::Invalid);}
            let top=if p.vertical==TableVertical::Page{0}else{self.page.top};
            if top+p.y as u32>=self.page.height-self.page.bottom {return Err(Error::Invalid);}
        } Ok(())
    }
}
