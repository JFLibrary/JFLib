// Responsive ribbon acceptance checks against the shared native layout planner.
// Exercises room for real document commands and overflow at supported widths.
use alloc::format;
use ui::{RibbonGroup, RibbonLayout, RibbonMetrics};

fn occupied(layout: &RibbonLayout, count: usize) -> i64 {
    (0..count)
        .map(|index| i64::from(layout.group_width(index).unwrap()))
        .sum::<i64>()
        + count.saturating_sub(1) as i64 * 8
}

pub(crate) fn run() -> (usize, usize) {
    let (mut passed, mut failed) = (0, 0);
    macro_rules! check {
        ($ok:expr, $name:expr) => {
            if $ok {
                passed += 1;
            } else {
                failed += 1;
                plat::write_stdout(format!("FAIL ribbon {}\n", $name).as_bytes());
            }
        };
    }
    let groups = [
        RibbonGroup::new(1350, "Zwischenablage", 148, 144),
        RibbonGroup::new(1300, "Schrift", 216, 190),
        RibbonGroup::new(1310, "Absatz", 176, 158),
        RibbonGroup::new(1320, "Formatvorlagen", 448, 360),
        RibbonGroup::new(1330, "Dokument", 174, 164),
        RibbonGroup::new(1340, "Bearbeiten", 164, 145),
    ];
    let wide = RibbonLayout::new(1904, &groups);
    check!(
        (0..groups.len())
            .all(|i| !wide.is_collapsed(i) && wide.group_width(i) == Some(groups[i].width)),
        "wide window shows complete groups"
    );
    let desktop = RibbonLayout::new(1224, &groups);
    check!(
        (0..groups.len()).all(|i| !desktop.is_collapsed(i))
            && occupied(&desktop, groups.len()) <= 1224,
        "1240 DIP window needs no overflow"
    );
    let small = RibbonLayout::new(984, &groups);
    check!(
        occupied(&small, groups.len()) <= 984,
        "1000 DIP window contains ribbon"
    );
    check!(
        (0..3).all(|i| !small.is_collapsed(i)) && small.is_collapsed(4) && small.is_collapsed(5),
        "format and paragraph controls retain priority over secondary groups"
    );
    check!(
        (0..groups.len()).all(|i| small.group_width(i).is_some_and(|w| w >= 48)),
        "every command group retains an overflow target"
    );
    let narrow = RibbonLayout::new(784, &groups);
    check!(
        occupied(&narrow, groups.len()) <= 784 && narrow.is_collapsed(3),
        "gallery becomes a named overflow before clipping"
    );
    let tiny = RibbonLayout::new(0, &groups);
    check!(
        (0..groups.len()).all(|i| tiny.is_collapsed(i)) && occupied(&tiny, groups.len()) == 616,
        "extreme narrow widths retain scrollable reachable groups"
    );
    check!(
        small.group_width(usize::MAX).is_none() && !small.is_collapsed(usize::MAX),
        "missing group lookup is bounded"
    );
    let empty = RibbonLayout::new(-1, &[]);
    check!(
        empty.group_width(0).is_none() && occupied(&empty, 0) == 0,
        "empty ribbon does not invent an overflow"
    );
    let invalid = [
        RibbonGroup::new(1, "A", -50, 500),
        RibbonGroup::new(5, "B", 120, -5),
    ];
    let repaired = RibbonLayout::new(200, &invalid);
    check!(
        repaired.group_width(0) == Some(48) && repaired.group_width(1) == Some(120),
        "invalid widths preserve positive hit targets"
    );
    let huge = [
        RibbonGroup::new(1, "A", i32::MAX, 48),
        RibbonGroup::new(5, "B", i32::MAX, 48),
    ];
    let bounded = RibbonLayout::new(200, &huge);
    check!(
        occupied(&bounded, huge.len()) <= 200 && (0..2).all(|i| !bounded.is_collapsed(i)),
        "large width sums do not overflow before compacting"
    );
    // Office keeps complete groups in a scroll strip instead of changing command layout.
    let office = RibbonMetrics {
        group_gap: 14,
        collapse_groups: false,
        ..RibbonMetrics::default()
    };
    for width in [0, 584, 984, 1224, 1904] {
        let layout = RibbonLayout::with_metrics(width, &groups, office);
        check!(
            (0..groups.len())
                .all(|i| !layout.is_collapsed(i) && layout.group_width(i) == Some(groups[i].width)),
            "scrolling groups keep the same complete command arrangement at every window width"
        );
    }
    let zoom = ui::Range::new(10, 500, 1, ui::RG_LOG);
    // Independent logarithm reference values (ln(value/10) / ln(50), not the
    // renderer's own output). Real percentages remain the semantic values.
    for (value, position) in [(10, 0), (25, 234224), (50, 411408), (75, 515053),
        (100, 588591), (125, 645632), (250, 822816), (500, 1000000)] {
        check!((zoom.position(value, 1000000) - position).abs() <= 1, "logarithmic zoom reference");
        check!(zoom.value_at(position, 1000000) == value, "logarithmic pointer inversion");
    }
    check!((10..=500).all(|v| zoom.value_at(zoom.position(v, 1000000), 1000000) == v),
        "all integer zoom values round-trip at sufficient precision");
    check!((0..110).all(|p| zoom.value_at(p, 110) <= zoom.value_at(p + 1, 110)),
        "pointer mapping is monotone at actual control width");
    let small = ui::Range::new(1, 4, 1, ui::RG_LOG);
    check!(small.value_at(270, 1000) == 1 && small.value_at(293, 1000) == 2,
        "logarithmic inverse rounds in value space rather than log space");
    check!(zoom.value_at(-20, 110) == 10 && zoom.value_at(200, 110) == 500
        && zoom.position(0, 110) == 0 && zoom.position(999, 110) == 110,
        "pointer and paint clamp at endpoints");
    let adjacent = ui::Range::new(i32::MAX - 1, i32::MAX, 1, ui::RG_LOG);
    check!(adjacent.position(i32::MAX, i32::MAX) == i32::MAX
        && adjacent.value_at(0, 10) == i32::MAX - 1
        && adjacent.value_at(10, 10) == i32::MAX, "adjacent large bounds retain precision");
    let signed = ui::Range::new(i32::MIN, i32::MAX, 1, ui::RG_LOG);
    check!(signed.position(i32::MAX, i32::MAX) == i32::MAX
        && signed.value_at(i32::MAX, i32::MAX) == i32::MAX
        && signed.value_at(0, i32::MAX) == i32::MIN, "nonpositive range falls back without overflow");
    for range in [ui::Range::new(10, 10, 1, ui::RG_LOG), ui::Range::new(20, 10, 1, ui::RG_LOG)] {
        check!(range.position(15, 110) == 0 && range.value_at(50, 110) == range.min,
            "empty and inverted ranges do not divide by zero");
    }
    check!(zoom.position(100, 0) == 0 && zoom.value_at(0, 0) == 10, "zero travel is bounded");
    (passed, failed)
}
