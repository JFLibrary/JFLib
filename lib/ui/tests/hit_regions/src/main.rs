//! Standalone no_std regression app: one disjoint surface is not duplicate widgets.
#![no_std]
#![no_main]
#![deny(unsafe_code)]
extern crate alloc;
use alloc::{string::String,vec};
use ui::*;

struct Check { mode:u32, text:String }
impl Check {
    fn surface(&mut self,ui:&mut Ui,id:u32,y:i32){
        let lines=[
            TextSurfaceLine{start:0,end:1,rect:[20,y,220,20],carets:vec![(0,20),(1,30)]},
            TextSurfaceLine{start:2,end:3,rect:[20,y+40,220,20],carets:vec![(2,20),(3,30)]},
        ];
        ui.textbox_surface(id,&mut self.text,0,&lines);
    }
}
impl App for Check {
    fn frame(&mut self,ui:&mut Ui){
        self.surface(ui,700,20);
        if self.mode==1 {self.surface(ui,701,120);self.surface(ui,701,220);}
        if self.mode==2 {ui.button(702,"First button");ui.button(702,"Different button, same ID");}
    }
    fn command(&mut self,ui:&mut Ui,id:u32,_arg:u32)->bool{
        if id==1||id==2 {self.mode=id;ui.repass();true}else{false}
    }
}
const DESC:AppDesc=AppDesc{title:"Hit region regression",w:480,h:360,..AppDesc::NONE};
ui::main!(ui::launch(&DESC,Check{mode:0,text:String::from("a\nb")},&style::STYLE));
