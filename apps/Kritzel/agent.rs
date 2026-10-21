// Semantic graphics actions use the existing image kernels and undo history.
use crate::{Kritzel, doc, history, imageops, tools, view};
use agent_core::{ActionError, ActionResult, ActionSpec, ErrorCode, Value};
use agent_core::schema::{Schema, Property, EMPTY_OBJECT, OBJECT, U64};
use ui::Ui;

static SIZE: Schema = Schema::Object { properties: &[
    Property { name: "width", schema: &Schema::Unsigned { min:1,max:4096 }, required:true },
    Property { name: "height", schema: &Schema::Unsigned { min:1,max:4096 }, required:true },
], additional:false };
static PIXEL: Schema = Schema::Object { properties: &[
    Property { name:"x",schema:&U64,required:true }, Property { name:"y",schema:&U64,required:true },
],additional:false };
static COLOR: Schema = Schema::Object { properties: &[
    Property { name:"argb",schema:&Schema::Unsigned { min:0,max:0xffffffff },required:true },
],additional:false };
const fn edit(name:&'static str,description:&'static str,input:&'static Schema)->ActionSpec {
    ActionSpec { undo:true, ..ActionSpec::write(name,description).schemas(input,&OBJECT) }
}
pub(crate) static ACTIONS: &[ActionSpec] = &[
    ActionSpec::read("image.inspect","Inspect canvas dimensions and paginated layer state").schemas(&OBJECT,&OBJECT),
    ActionSpec::read("image.pixel","Read a composited straight ARGB pixel at image coordinates").schemas(&PIXEL,&OBJECT),
    edit("image.resize","Resample all layers using the existing image kernel",&SIZE),
    edit("image.canvas.resize","Resize all layer canvases without scaling their pixels",&SIZE),
    edit("image.flip.horizontal","Mirror every layer horizontally",&EMPTY_OBJECT),
    edit("image.flip.vertical","Mirror every layer vertically",&EMPTY_OBJECT),
    edit("image.rotate.right","Rotate all layers clockwise by 90 degrees",&EMPTY_OBJECT),
    edit("image.layer.add","Insert a transparent layer above the active layer",&EMPTY_OBJECT),
    edit("image.layer.duplicate","Duplicate the active layer including pixels and opacity",&EMPTY_OBJECT),
    edit("image.layer.remove","Delete the active layer; preserve at least one layer",&EMPTY_OBJECT),
    edit("image.layer.merge","Merge the active layer into the layer below",&EMPTY_OBJECT),
    edit("image.layer.fill","Fill the active layer with an explicit straight ARGB color",&COLOR),
    edit("image.undo","Undo through the same graphics history used by the GUI",&EMPTY_OBJECT),
    edit("image.redo","Redo through the same graphics history used by the GUI",&EMPTY_OBJECT),
];
fn unavailable(text:&str)->ActionError { ActionError::new(ErrorCode::Unavailable,text) }
fn number(v:&Value,key:&str)->u64 { v.get(key).and_then(Value::as_u64).unwrap_or(0) }
impl Kritzel {
    pub(crate) fn agent_state(&self,query:&Value)->Value {
        let offset=number(query,"offset").min(self.doc.nlayers as u64) as usize;
        let limit=query.get("limit").and_then(Value::as_u64).unwrap_or(32).clamp(1,64) as usize;
        let layers=(offset..self.doc.nlayers as usize).take(limit).map(|i|Value::object([
            ("index",Value::u64(i as u64)),("active",Value::Bool(self.doc.cur_layer==i as i32)),
            ("visible",Value::Bool(self.doc.layer_vis[i]!=0)),("opacity",Value::u64(self.doc.layer_opac[i] as u64)),
        ])).collect();
        Value::object([
            ("revision",Value::u64(doc::doc_version(&self.doc) as u64)),
            ("width",Value::u64(self.doc.w as u64)),("height",Value::u64(self.doc.h as u64)),
            ("dirty",Value::Bool(doc::doc_modified(&self.doc))), ("totalLayers",Value::u64(self.doc.nlayers as u64)),
            ("layers",Value::Array(layers)), ("canUndo",Value::Bool(history::hist_can_undo(&self.hist))),
            ("canRedo",Value::Bool(history::hist_can_redo(&self.hist))),
        ])
    }
    pub(crate) fn agent_available(&self,spec:&ActionSpec)->Result<(),ActionError> {
        if self.doc.nlayers==0 { return Err(unavailable("No image is open")); }
        match spec.name {
            "image.undo" if !history::hist_can_undo(&self.hist) => Err(unavailable("Nothing to undo")),
            "image.redo" if !history::hist_can_redo(&self.hist) => Err(unavailable("Nothing to redo")),
            "image.layer.remove" if self.doc.nlayers<=1 => Err(unavailable("The last layer cannot be removed")),
            "image.layer.merge" if self.doc.cur_layer<=0 => Err(unavailable("No layer below the active layer")),
            "image.layer.add" | "image.layer.duplicate" if self.doc.nlayers as usize>=crate::consts::MAX_LAYERS => Err(unavailable("Layer limit reached")),
            _=>Ok(())
        }
    }
    pub(crate) fn agent_action(&mut self,ui:&mut Ui,spec:&ActionSpec,input:&Value)->Result<ActionResult,ActionError> {
        self.agent_available(spec)?;
        view::view_reclaim(self);
        let result=self.agent_apply(spec,input);
        view::view_frame_end(self,ui);
        ui.invalidate();
        result.map(|value|ActionResult::completed(value,doc::doc_version(&self.doc) as u64))
    }
    fn agent_apply(&mut self,spec:&ActionSpec,input:&Value)->Result<Value,ActionError> {
        if spec.name=="image.inspect" { return Ok(self.agent_state(input)); }
        if spec.name=="image.pixel" {
            let (x,y)=(number(input,"x"),number(input,"y"));
            if x>=self.doc.w as u64 || y>=self.doc.h as u64 { return Err(ActionError::new(ErrorCode::InvalidInput,"Pixel is outside the image")); }
            return Ok(Value::object([("argb",Value::u64(doc::doc_pick(&mut self.doc,x as i32,y as i32) as u64))]));
        }
        tools::tool_abort(self);
        let failed=match spec.name {
            "image.resize" => imageops::op_scale(self,number(input,"width") as i32,number(input,"height") as i32)!=0,
            "image.canvas.resize" => imageops::op_canvas(self,number(input,"width") as i32,number(input,"height") as i32)!=0,
            "image.flip.horizontal" => { imageops::op_flip_h(self); false }
            "image.flip.vertical" => { imageops::op_flip_v(self); false }
            "image.rotate.right" => { imageops::op_rotate90(self,1); false }
            "image.layer.add" => imageops::layer_add(self)<0,
            "image.layer.duplicate" => imageops::layer_duplicate(self)<0,
            "image.layer.remove" => { imageops::layer_delete(self); false }
            "image.layer.merge" => { imageops::layer_merge_down(self); false }
            "image.layer.fill" => {
                if history::hist_begin(self,self.doc.cur_layer)!=0 { return Err(unavailable("Unable to allocate undo snapshot")); }
                doc::clear_sf(&mut self.doc.layers[self.doc.cur_layer as usize],tools::premul(number(input,"argb") as u32));
                history::hist_end(self,0,0,self.doc.w,self.doc.h); doc::doc_mark_all(&mut self.doc); false
            }
            "image.undo" => !history::hist_undo(self), "image.redo" => !history::hist_redo(self),
            _ => return Err(ActionError::new(ErrorCode::NotFound,"Unknown graphics action")),
        };
        if failed { return Err(unavailable("Image operation failed")); }
        Ok(self.agent_state(&Value::object([])))
    }
}
