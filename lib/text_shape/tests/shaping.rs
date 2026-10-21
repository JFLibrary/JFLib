use text_shape::Engine;

fn font() -> Engine {
    let file=std::env::var("PAINTCRAFT_SHAPING_FONT").expect("set PAINTCRAFT_SHAPING_FONT to a font with Latin/Arabic/Hebrew coverage");
    Engine::new(&std::fs::read(file).unwrap(),0).unwrap()
}
#[test]
fn arabic_joining_rtl_and_clusters() {
    let engine=font(); let text="سلام"; let glyphs=engine.shape(text).unwrap();
    assert!(glyphs.len()<text.chars().count(), "lam-alef ligature must form");
    assert!(glyphs.iter().all(|g| g.id!=0 && g.rtl && text.is_char_boundary(g.cluster) && text.is_char_boundary(g.end)));
    assert!(glyphs.windows(2).all(|w| w[0].cluster>=w[1].cluster));
    let isolated=engine.shape("س").unwrap();
    assert_ne!(glyphs.last().unwrap().id,isolated[0].id,"initial joining form");
}
#[test]
fn mixed_bidi_keeps_latin_order() {
    let engine=font(); let text="abc אבג 123"; let glyphs=engine.shape(text).unwrap();
    assert!(glyphs.iter().any(|g|g.rtl));
    assert_eq!(glyphs[0].cluster,0); assert_eq!(glyphs[1].cluster,1); assert_eq!(glyphs[2].cluster,2);
    let mut clusters:Vec<_>=glyphs.iter().map(|g|g.cluster).collect(); clusters.sort_unstable(); clusters.dedup();
    assert_eq!(clusters,text.char_indices().map(|(i,_)|i).collect::<Vec<_>>());
}
#[test]
fn combining_mark_shares_cluster() {
    let engine=font(); let glyphs=engine.shape("a\u{301}").unwrap();
    assert!(glyphs.iter().all(|g|g.cluster==0));
    assert!(glyphs.iter().all(|g|g.id!=0));
}
#[test]
fn malformed_font_is_rejected() { assert!(Engine::new(b"bad font",0).is_none()); }
#[test]
fn fallback_shapes_an_entire_arabic_run() {
    let path=std::env::var("PAINTCRAFT_SHAPING_FONT").unwrap();
    let mut data=std::fs::read(path).unwrap();
    let count=u16::from_be_bytes([data[4],data[5]]) as usize;
    let record=(0..count).map(|i|12+i*16).find(|&i|&data[i..i+4]==b"cmap").unwrap();
    // A valid test font with no character map forces a deterministic fallback.
    data[record..record+4].copy_from_slice(b"zzzz");
    let primary=Engine::new(&data,0).unwrap(); let fallback=font();
    let glyphs=primary.shape_with_fallback("سلام",&[&fallback]).unwrap();
    assert!(glyphs.len()<4 && glyphs.iter().all(|g|g.font==1 && g.id!=0 && g.rtl));
}
