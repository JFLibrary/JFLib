// jfWord application entry and module boundary. No codec or renderer implementation here.
use crate::*;
pub(crate) mod commands;
mod agent;
#[cfg(target_arch = "wasm32")]
mod browser;
mod document;
mod lifecycle;
mod state;
pub(crate) use document::style_label;
pub(crate) use state::Word;
pub(crate) fn run() -> i32 {
    let args = base::Args(plat::args());
    if let Some(code) = compat_checks::probe(&args) {
        return code;
    }
    if let Some(code) = library_test_host::run(&args) {
        return code;
    }
    if let Some(code) = clipboard_codec_checks::probe(&args) {
        return code;
    }
    if args.flag("--self-test") {
        return checks::run();
    }
    if args.flag("--self-test-tables") {
        let (passed,failed)=checks::table_checks::run();
        plat::write_stdout(format!("jfWord tables: {passed} passed, {failed} failed\n").as_bytes());
        return i32::from(failed!=0);
    }
    if args.flag("--self-test-clipboard-codecs") {
        let (passed, failed) = clipboard_codec_checks::run();
        plat::write_stdout(format!("jfWord clipboard codecs: {passed} passed, {failed} failed\n").as_bytes());
        return i32::from(failed != 0);
    }
    if let Some(path) = args.value("--check-docx") {
        return checks::probe(path, args.value("--output"), args.flag("--expect-reject"));
    }
    if let Some(path) = args.value("--check-sources") {
        return science_checks::probe(
            path,
            args.value("--output"),
            args.value("--docx-output"),
            args.flag("--expect-reject"),
        );
    }
    ui::launch(&DESC, Word::default(), &style::STYLE)
}
