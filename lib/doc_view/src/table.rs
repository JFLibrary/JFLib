//! Table hit testing and reversible resizing; the same coordinates drive native and web input.
use alloc::vec::Vec;
use doc_core::{Document, Id, TableChange};
use doc_layout::{CellBox, Page};
use ui::{BTN_LEFT, MOD_ALT, UiInput, VK_ESCAPE};
pub fn table_cell_at(page: &Page, x: i32, y: i32) -> Option<&CellBox> {
    page.cells.iter().rev().find(|c| {
        x >= c.rect[0] && x < c.rect[0] + c.rect[2] && y >= c.rect[1] && y < c.rect[1] + c.rect[3]
    })
}
/// Horizontal text hit regions in twips, excluding floating tables and their text distances.
/// The full paragraph width must not intercept clicks on a neighboring floating cell.
pub fn text_line_bounds(d:&Document,page:&Page,line:&doc_layout::Line)->[i32;4]{
    if let Some(c)=page.cells.iter().find(|c|Some(c.cell)==line.cell){return [c.rect[0],line.top,c.rect[2],line.height];}
    let ink_left=line.fragments.iter().map(|f|f.x).min().unwrap_or(line.x);
    let ink_right=line.fragments.iter().map(|f|f.x+f.width).max().unwrap_or(line.x);
    let mut left=(d.page.left as i32).min(ink_left);let mut right=((d.page.width-d.page.right) as i32).max(ink_right);
    for table in d.tables.iter().filter(|t|t.position.is_some()) {if let Some(r)=float_rect(d,table,page){
        let [up,after,down,before]=table.position.unwrap().distance.map(i32::from);
        if line.top<r[1]+r[3]+down&&line.top+line.height>r[1]-up {
            if ink_left>=r[0]+r[2]+after {left=left.max(r[0]+r[2]+after);}
            else if ink_right<=r[0]-before {right=right.min(r[0]-before);}
        }
    }}[left,line.top,(right-left).max(1),line.height]
}
#[derive(Clone)]
struct Drag {
    table: Id,
    page: usize,
    revision: u64,
    column: usize,
    row: usize,
    horizontal: bool,
    origin: i32,
    current: i32,
    columns: Vec<u32>,
    row_height: u32,
}
struct Move {table:Id,page:usize,revision:u64,origin:[i32;2],delta:[i32;2],position:doc_core::TablePosition,rect:[i32;4]}
#[derive(Default)]
pub struct TableController {
    drag: Option<Drag>,
    moving:Option<Move>,
}
impl TableController {
    pub fn active(&self) -> bool {
        self.drag.is_some()||self.moving.is_some()
    }
    pub fn cancel(&mut self) {
        self.drag = None;
        self.moving=None;
    }
    /// Returns one command on release. Escape and revision changes cancel a gesture.
    pub fn update(
        &mut self,
        d: &Document,
        page: &Page,
        index: usize,
        input: &UiInput,
        revision: u64,
    ) -> Option<(Id, TableChange)> {
        if self.drag.as_ref().is_some_and(|v| v.revision != revision) || self.moving.as_ref().is_some_and(|v|v.revision!=revision)
            || input.key & 0xffff == VK_ESCAPE
        {
            self.cancel();
            return None;
        }
        if self.drag.as_ref().is_some_and(|v| v.page != index) || self.moving.as_ref().is_some_and(|v|v.page!=index) || input.rect[2] <= 0 {
            return None;
        }
        let physical =
            |p: i32| (i64::from(p) * i64::from(d.page.width) / i64::from(input.rect[2])) as i32;
        let x = physical(input.x);
        let y = physical(input.y);
        let tolerance = physical(4).max(20);
        if input.pressed & (1 << BTN_LEFT) != 0 {
            for t in d.tables.iter().filter(|t|t.position.is_some()).rev(){
                if let Some(rect)=float_rect(d,t,page){let h=handle(rect);
                    if x>=h[0]&&x<h[0]+h[2]&&y>=h[1]&&y<h[1]+h[3]{
                        self.moving=Some(Move{table:t.id,page:index,revision,origin:[x,y],delta:[0;2],position:t.position.unwrap(),rect});break;
                    }
                }
            }
        }
        if let Some(v)=&mut self.moving {
            let snap=|n:i32|if input.mods&MOD_ALT!=0{n}else{(n+45).div_euclid(90)*90};
            let delta=[snap(x-v.origin[0]),snap(y-v.origin[1])];
            let px=v.position.x+delta[0];let py=v.position.y+delta[1];
            if (-31680..=31680).contains(&px)&&(0..=31680).contains(&py)&&v.rect[0]+delta[0]>=0&&v.rect[0]+delta[0]+v.rect[2]<=d.page.width as i32
                &&v.rect[1]+delta[1]>=0&&v.rect[1]+delta[1]+i32::from(v.position.distance[2])<(d.page.height-d.page.bottom) as i32 {v.delta=delta;}
            if input.released&(1<<BTN_LEFT)!=0 {let v=self.moving.take().unwrap();if v.delta==[0,0]{return None;}let mut p=v.position;p.x+=v.delta[0];p.y+=v.delta[1];return Some((v.table,TableChange::Position(Some(p))));}
            return None;
        }
        if input.pressed & (1 << BTN_LEFT) != 0 {
            if let Some(c) = page.cells.iter().rev().find(|c| {
                y >= c.rect[1]
                    && y < c.rect[1] + c.rect[3]
                    && (x - c.rect[0] - c.rect[2]).abs() <= tolerance
            }) {
                let t = d.table(c.table)?;
                let origin = c.rect[0] + c.rect[2];
                self.drag = Some(Drag {
                    table: t.id,
                    page: index,
                    revision,
                    column: c.column + c.column_span - 1,
                    row: c.row + c.row_span - 1,
                    horizontal: true,
                    origin,
                    current: origin,
                    columns: t.columns.clone(),
                    row_height: c.last_row_height,
                });
            } else if let Some(c) = page.cells.iter().rev().find(|c| {
                c.row_end && x >= c.rect[0]
                    && x < c.rect[0] + c.rect[2]
                    && (y - c.rect[1] - c.rect[3]).abs() <= tolerance
                    // A very short fixed row still needs an interior selection target.
                    && (y >= c.rect[1] + c.rect[3] || c.rect[1] + c.rect[3] - y <= tolerance.min((c.rect[3] / 3).max(1)))
            }) {
                let t = d.table(c.table)?;
                let origin = c.rect[1] + c.rect[3];
                self.drag = Some(Drag {
                    table: t.id,
                    page: index,
                    revision,
                    column: c.column + c.column_span - 1,
                    row: c.row + c.row_span - 1,
                    horizontal: false,
                    origin,
                    current: origin,
                    columns: t.columns.clone(),
                    row_height: c.last_row_height,
                });
            }
        }
        if let Some(v) = &mut self.drag {
            let raw = if v.horizontal { x } else { y };
            let delta = raw - v.origin;
            let delta = if input.mods & MOD_ALT != 0 {
                delta
            } else {
                ((delta + 45).div_euclid(90)) * 90
            };
            let table = d.table(v.table)?;
            let minimum = u32::from(table.padding) * 2 + 240;
            let mut valid = true;
            if v.horizontal {
                let a = i64::from(v.columns[v.column]) + i64::from(delta);
                valid = a > 0 && a <= 31680;
                if let Some(b) = v.columns.get(v.column + 1) {
                    valid &= i64::from(*b) - i64::from(delta) > 0;
                } else {
                    valid &= i64::from(table.width()) + i64::from(delta)
                        <= if table.position.is_some(){d.table_x(table).map(|x|i64::from(d.page.width-x)).unwrap_or(0)}else{d.table_bounds(table.id).map(|(a,b)|i64::from(b-a)).unwrap_or(0)};
                }
                valid &= table.anchors().all(|(_,c,cell)|{
                    let end=c+usize::from(cell.column_span);
                    let mut width:i64=v.columns[c..end].iter().map(|v|i64::from(*v)).sum();
                    if (c..end).contains(&v.column){width+=i64::from(delta);}
                    if v.column+1<v.columns.len()&&(c..end).contains(&(v.column+1)){width-=i64::from(delta);}
                    width>=i64::from(minimum)
                });
            } else {
                let h = i64::from(v.row_height) + i64::from(delta);
                // A real row end may be on a continuation page. Use the model's
                // height limit; pagination is validated when the command commits.
                valid &= (120..=31680).contains(&h);
            }
            if valid {
                v.current = v.origin + delta;
            }
            if input.released & (1 << BTN_LEFT) != 0 {
                let v = self.drag.take().unwrap();
                let delta = v.current - v.origin;
                if delta == 0 {
                    return None;
                }
                let change = if v.horizontal {
                    let mut widths = v.columns;
                    widths[v.column] = (widths[v.column] as i32 + delta) as u32;
                    if v.column + 1 < widths.len() {
                        widths[v.column + 1] = (widths[v.column + 1] as i32 - delta) as u32;
                    }
                    TableChange::Columns(widths)
                } else {
                    TableChange::RowHeight {
                        row: v.row,
                        height: (v.row_height as i32 + delta) as u32,
                    }
                };
                return Some((v.table, change));
            }
        }
        None
    }
    pub fn draw_guide(&self, ui: &mut ui::Ui, page: usize, rect: [i32; 4], page_width: u32) {
        if let Some(d)=self.moving.as_ref().filter(|v|v.page==page){
            let px=|v:i32|(i64::from(v)*i64::from(rect[2])/i64::from(page_width)) as i32;
            let x=rect[0]+px(d.rect[0]+d.delta[0]);let y=rect[1]+px(d.rect[1]+d.delta[1]);let w=px(d.rect[2]);let h=px(d.rect[3]);let stroke=ui.dp(1).max(1);
            ui.draw_color(ui.color(ui::T_ACCENT));ui.draw_rect(x,y,w,stroke);ui.draw_rect(x,y+h-stroke,w,stroke);ui.draw_rect(x,y,stroke,h);ui.draw_rect(x+w-stroke,y,stroke,h);
        }
        if let Some(d) = self.drag.as_ref().filter(|d| d.page == page) {
            let p = (i64::from(d.current) * i64::from(rect[2]) / i64::from(page_width)) as i32;
            ui.draw_color(ui.color(ui::T_ACCENT));
            if d.horizontal {
                ui.draw_rect(rect[0] + p, rect[1], ui.dp(1).max(1), rect[3]);
            } else {
                ui.draw_rect(rect[0], rect[1] + p, rect[2], ui.dp(1).max(1));
            }
        }
    }
    /// Paint stable move handles in the same page coordinates used by hit testing.
    pub fn draw_handles(&self,ui:&mut ui::Ui,d:&Document,page:&Page,rect:[i32;4]){
        let px=|v:i32|(i64::from(v)*i64::from(rect[2])/i64::from(d.page.width)) as i32;
        for t in d.tables.iter().filter(|t|t.position.is_some()) {if let Some(r)=float_rect(d,t,page){let h=handle(r);let x=rect[0]+px(h[0]);let y=rect[1]+px(h[1]);let size=px(h[2]).max(ui.dp(8));let stroke=ui.dp(1).max(1);
            ui.draw_color(ui.color(ui::T_ACCENT));ui.draw_rect(x,y,size,stroke);ui.draw_rect(x,y+size-stroke,size,stroke);ui.draw_rect(x,y,stroke,size);ui.draw_rect(x+size-stroke,y,stroke,size);
            ui.draw_rect(x+size/2,y+2*stroke,stroke,size-4*stroke);ui.draw_rect(x+2*stroke,y+size/2,size-4*stroke,stroke);
        }}
    }
}
fn float_rect(d:&Document,t:&doc_core::Table,page:&Page)->Option<[i32;4]>{
    let top=page.cells.iter().filter(|c|c.table==t.id).map(|c|c.rect[1]).min()?;
    let bottom=page.cells.iter().filter(|c|c.table==t.id).map(|c|c.rect[1]+c.rect[3]).max()?;
    Some([d.table_x(t).ok()? as i32,top,t.width() as i32,bottom-top])
}
fn handle(r:[i32;4])->[i32;4]{[(r[0]-220).max(0),(r[1]-220).max(0),180,180]}
