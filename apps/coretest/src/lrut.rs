// lrut -- checks of base::lru: use order, eviction by count and by weight (oldest first, the
// new entry itself when it alone is too heavy), replacing, removing, peek without a use

use alloc::string::String;
use alloc::vec::Vec;

use base::lru::Lru;

use crate::T;

pub fn run(t: &mut T) {
    let mut c: Lru<u32, &str> = Lru::new(3, 0);
    t.check("empty", c.is_empty() && c.get(&1).is_none());
    t.eq("insert a", c.insert(1, "a", 1), Vec::new());
    c.insert(2, "b", 1);
    c.insert(3, "c", 1);
    t.eq("keys oldest first", c.keys().copied().collect::<Vec<_>>(), alloc::vec![1, 2, 3]);
    t.eq("get uses", c.get(&1), Some(&"a"));
    t.eq("order after get", c.keys().copied().collect::<Vec<_>>(), alloc::vec![2, 3, 1]);
    t.eq("peek does not use", c.peek(&2), Some(&"b"));
    t.eq("evicts the least recently used", c.insert(4, "d", 1), alloc::vec![(2, "b")]);
    t.check("2 gone, others there", !c.contains(&2) && c.contains(&1) && c.contains(&3) && c.contains(&4));
    t.eq("replace keeps the count", c.insert(3, "C", 1), Vec::new());
    t.eq("replaced value", (c.len(), c.peek(&3)), (3, Some(&"C")));
    t.eq("replace makes it young", c.keys().copied().collect::<Vec<_>>(), alloc::vec![1, 4, 3]);
    if let Some(v) = c.get_mut(&1) {
        *v = "A";
    }
    t.eq("get_mut", (c.peek(&1), c.keys().last().copied()), (Some(&"A"), Some(1)));
    t.eq("remove", (c.remove(&4), c.remove(&4), c.len()), (Some("d"), None, 2));
    t.eq("pop_lru", c.pop_lru(), Some((3, "C")));
    c.clear();
    t.check("clear", c.is_empty() && c.weight() == 0 && c.keys().next().is_none());
    // by weight
    let mut w: Lru<String, u32> = Lru::new(0, 100);
    w.insert(String::from("a"), 1, 40);
    w.insert(String::from("b"), 2, 40);
    t.eq("weight", w.weight(), 80);
    let ev = w.insert(String::from("c"), 3, 50);
    t.eq("evicted by weight", ev, alloc::vec![(String::from("a"), 1)]);
    t.eq("weight after", (w.weight(), w.len()), (90, 2));
    let ev = w.insert(String::from("d"), 4, 95);
    t.eq("evicts several", ev.iter().map(|e| e.1).collect::<Vec<_>>(), alloc::vec![2, 3]);
    let ev = w.insert(String::from("big"), 5, 150);
    t.eq("too heavy alone goes too", ev.iter().map(|e| e.1).collect::<Vec<_>>(), alloc::vec![4, 5]);
    t.check("nothing left", w.is_empty() && w.weight() == 0);
    // both limits
    let mut both: Lru<u8, u8> = Lru::new(2, 10);
    both.insert(1, 1, 3);
    both.insert(2, 2, 3);
    t.eq("count limit with weight room", both.insert(3, 3, 3), alloc::vec![(1, 1)]);
    t.eq("iter in key order", both.iter().map(|e| *e.0).collect::<Vec<_>>(), alloc::vec![2, 3]);
}
