//! Note reservation shared by paragraph lines and positioned table fragments.
use super::*;

pub(super) fn queue(anchors:&[Anchor],notes:&BTreeMap<Id,Vec<Flow>>)->VecDeque<Flow>{
    let mut seen=BTreeSet::new();let mut out=VecDeque::new();
    for a in anchors {if seen.insert(a.id){if let Some(lines)=notes.get(&a.id){out.extend(lines.iter().cloned());}}}out
}
pub(super) fn required(queue:&VecDeque<Flow>)->usize{
    queue.iter().rposition(|l|l.note_start).map_or(0,|i|i+1)
}
pub(super) fn minimum(queue:&VecDeque<Flow>,draft:&Draft)->i32{
    queue.iter().take(required(queue)).enumerate().map(|(i,l)|i64::from(l.line.height)+i64::from(if i==0&&draft.footer.is_empty(){l.at_top}else{l.before})).sum::<i64>().min(i64::from(i32::MAX/2)) as i32
}
