//! zwp_text_input_v3 version 1: committed text, preedit and candidate positioning.
//! Surrounding-text editing is not advertised until the editor supplies that state.
use alloc::string::String;
use super::{Wl, proto::{Ev,Iface,Req}, wire::{A,Args}};
pub(crate) const MANAGER: Iface=Iface { name:"zwp_text_input_manager_v3",evs:&[] };
const INPUT: Iface=Iface { name:"zwp_text_input_v3",evs:&[
    Ev{sig:"enter:o",h:Some(enter)},Ev{sig:"leave:o",h:Some(leave)},Ev{sig:"preedit_string:?sii",h:Some(preedit)},
    Ev{sig:"commit_string:?s",h:Some(commit)},Ev{sig:"delete_surrounding_text:uu",h:None},Ev{sig:"done:u",h:Some(done)},
] };
const GET:Req=Req{op:1,sig:"get_text_input:no",destroy:false};
const ENABLE:Req=Req{op:1,sig:"enable:",destroy:false};
const DISABLE:Req=Req{op:2,sig:"disable:",destroy:false};
const CONTENT:Req=Req{op:5,sig:"set_content_type:uu",destroy:false};
const RECT:Req=Req{op:6,sig:"set_cursor_rectangle:iiii",destroy:false};
const COMMIT:Req=Req{op:7,sig:"commit:",destroy:false};
pub(crate) struct Ime {
    pub manager:u32, id:u32, focus:i32, active:bool, wanted:bool, rect:[i32;3], serial:u32,
    preedit:Option<(String,usize)>, commit:String,
    at:[i32;3], // the window the caret rectangle was sent for and its offset in the focus
    pre_win:i32, // the window that shows preedit text (-1 = none)
}
impl Default for Ime { fn default()->Self { Self{manager:0,id:0,focus:-1,active:false,wanted:false,rect:[0;3],serial:0,preedit:None,commit:String::new(),at:[-1,0,0],pre_win:-1} } }
impl Wl {
    pub(crate) fn ime_setup(&mut self) {
        if self.ime.id==0 && self.ime.manager!=0 && self.g.seat!=0 {
            self.ime.id=self.w.send(self.ime.manager,&GET,&[A::N(&INPUT),A::O(self.g.seat)]);
        }
    }
    pub(crate) fn ime_pos(&mut self,x:i32,y:i32,h:i32) {
        self.ime_setup();
        let changed=self.ime.rect!=[x,y,h] || self.ime.wanted!=(h>0);
        self.ime.rect=[x,y,h]; self.ime.wanted=h>0;
        if self.ime.id==0 || self.ime.focus<0 { return; }
        let id=self.ime.id;
        if !self.ime.wanted {
            if self.ime.active { self.w.send(id,&DISABLE,&[]); self.ime.active=false; self.ime_commit(); self.ime_preedit_clear(); }
            return;
        }
        // the caret of a child window (the keys go there) lies in the focused toplevel at its offset
        let t=self.ime_target();
        let (ox,oy)=if t!=self.ime.focus { self.child_offset(t as usize) } else { (0,0) };
        let moved=self.ime.at!=[t,ox,oy];
        if moved && self.ime.pre_win>=0 && self.ime.pre_win!=t { self.ime_preedit_clear(); }
        self.ime.at=[t,ox,oy];
        if !self.ime.active {
            self.w.send(id,&ENABLE,&[]); self.w.send(id,&CONTENT,&[A::U(0),A::U(0)]); self.ime.active=true;
        } else if !changed && !moved { return; }
        let scale=self.win[self.ime.focus as usize].s120.max(120);
        self.w.send(id,&RECT,&[A::I((x+ox)*120/scale),A::I((y+oy)*120/scale),A::I(2),A::I((h*120/scale).max(1))]); self.ime_commit();
    }
    /// the window that gets committed text: the child window that has the keys, else the focus
    fn ime_target(&self)->i32 {
        let t=self.kb_target();
        if self.ime.focus>=0 && t>=0 && self.root_of(t as usize) as i32==self.ime.focus { t } else { self.ime.focus }
    }
    /// the preedit text goes away (where it is shown)
    fn ime_preedit_clear(&mut self) {
        let w=if self.ime.pre_win>=0 { self.ime.pre_win } else { self.ime_target() };
        if w>=0 { crate::ime_preedit_set(w as u8,"",0); }
        self.ime.pre_win=-1;
    }
    fn ime_commit(&mut self) { self.w.send(self.ime.id,&COMMIT,&[]); self.ime.serial=self.ime.serial.wrapping_add(1); self.w.flush(); }
}
fn enter(wl:&mut Wl,_:u32,a:&Args,_:u64) {
    wl.ime.focus=wl.find(a.u(0)); wl.ime.active=false;
    let [x,y,h]=wl.ime.rect; wl.ime_pos(x,y,h);
}
fn leave(wl:&mut Wl,_:u32,_:&Args,_:u64) {
    if wl.ime.focus>=0 { wl.ime_preedit_clear(); }
    wl.ime.focus=-1; wl.ime.active=false; wl.ime.preedit=None; wl.ime.commit.clear(); wl.ime.at=[-1,0,0];
}
fn preedit(wl:&mut Wl,_:u32,a:&Args,_:u64) {
    let text=a.s(0).and_then(|s| core::str::from_utf8(s).ok()).unwrap_or("");
    wl.ime.preedit=Some((if text.len()<=4096 { String::from(text) } else { String::new() },a.i(1).max(0) as usize));
}
fn commit(wl:&mut Wl,_:u32,a:&Args,_:u64) {
    let text=a.s(0).and_then(|s| core::str::from_utf8(s).ok()).unwrap_or("");
    wl.ime.commit=if text.len()<=4096 { String::from(text) } else { String::new() };
}
fn done(wl:&mut Wl,_:u32,_:&Args,_:u64) {
    let committed=core::mem::take(&mut wl.ime.commit); let preedit=wl.ime.preedit.take();
    // Even an older serial must apply the input events. A newer caret commit
    // may already be in flight; discarding these events would lose user text.
    if wl.ime.focus<0 || !wl.ime.active { return; }
    let t=wl.ime_target();
    if wl.ime.pre_win>=0 && wl.ime.pre_win!=t { wl.ime_preedit_clear(); }
    crate::ime_commit_set(t as u8,&committed);
    match preedit {
        Some((text,cursor)) if !text.is_empty() => { crate::ime_preedit_set(t as u8,&text,cursor); wl.ime.pre_win=t; }
        _ => { crate::ime_preedit_set(t as u8,"",0); wl.ime.pre_win=-1; }
    }
}
