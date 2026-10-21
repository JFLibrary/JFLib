// jfWord executable entry; reusable document/codec/rendering logic lives in libraries.
#![no_std]
#![no_main]
#![deny(unsafe_code)]
extern crate alloc;
use alloc::{format, string::String, vec::Vec};
use doc_core::{Document, Edit, Editor, Inline, ParagraphStyle, Source};
use ui::*;
mod app;
mod checks;
#[path = "ui/mod.rs"]
mod views;
mod workflows;
use app::{Word, style_label};
use app::commands::*;
use checks::*;
use views::*;
use workflows::*;
use source_form::SourceForm;
ui::main!(app::run());
