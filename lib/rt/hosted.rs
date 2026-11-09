// hosted -- what rt has for a program with the standard library (feature std, docs/CRATES.md
// CR6); the only file of rt that names std, compiled only with that feature.  std brings the panic
// handler, the allocator and the memory functions then (lib.rs, mem.rs and os.rs leave theirs out).
//
//   panic_hook()          std's panics end as rt's panic handler ends them without std
//   abort_on_unwind(f)    (feature unwind) a panic that leaves f ends the program as an uncaught one
//   panic_take()          (feature unwind) the last panic of the calling thread, as text
//
// With the feature unwind and panic = "unwind" a panic unwinds: the hook only keeps its text (LAST)
// and reports nothing, since the app may catch it.  It is reported where it leaves JFLib's threads
// and the program's start (abort_on_unwind) or reaches a function that cannot unwind (an extern
// callback such as a window procedure: core panics once more there, with one of NOUNWIND's texts).

#[cfg(feature = "unwind")]
use alloc::string::String;

/// (feature std) std's panics end as rt's panic handler ends them without std: one line on stderr,
/// the crash hook, exit code 3.  plat's start of a std program installs it; replacing it with
/// `std::panic::set_hook` gives up JFLib's crash report for panics.  With the feature `unwind` and
/// `panic = "unwind"` it reports only a panic nobody catches: one that leaves the program's start or
/// a thread of JFLib's, or reaches a callback of the system ([`abort_on_unwind`]).
pub fn panic_hook() {
    std::panic::set_hook(alloc::boxed::Box::new(on_panic));
}

#[cfg(not(all(feature = "unwind", panic = "unwind")))]
fn on_panic(info: &std::panic::PanicHookInfo) {
    crate::panic_report(info)
}

// core's texts of a second panic on a thread that is unwinding: it reached a function that cannot
// unwind (extern "C" / "system"), or a destructor panicked during the unwinding
#[cfg(all(feature = "unwind", panic = "unwind"))]
const NOUNWIND: [&str; 2] = ["panic in a function that cannot unwind", "panic in a destructor during cleanup"];

#[cfg(all(feature = "unwind", panic = "unwind"))]
fn on_panic(info: &std::panic::PanicHookInfo) {
    if info.payload_as_str().is_some_and(|m| NOUNWIND.contains(&m)) {
        report_last(None, info) // the panic that was unwinding
    }
    let text = alloc::format!("{info}");
    let _ = LAST.try_with(|l| l.try_borrow_mut().map(|mut s| *s = Some(text)));
}

#[cfg(feature = "unwind")]
std::thread_local! {
    // the last panic of this thread as the hook saw it: "panicked at FILE:LINE:COL:\nMESSAGE"
    static LAST: core::cell::RefCell<Option<String>> = const { core::cell::RefCell::new(None) };
}

/// the end of a panic nobody caught: the text the hook kept for it (when it ends in the payload's
/// message, or the payload has none), else `now`; JFLib's crash report, exit code 3
#[cfg(feature = "unwind")]
fn report_last(message: Option<&str>, now: &dyn core::fmt::Display) -> ! {
    let last = LAST.try_with(|l| l.try_borrow_mut().ok().and_then(|mut s| s.take())).ok().flatten();
    match last {
        Some(text) if message.is_none_or(|m| text.ends_with(m)) => crate::panic_report(&text),
        _ => crate::panic_report(now),
    }
}

/// (feature `unwind`) run `f`; a panic that leaves it ends the program as an uncaught panic:
/// JFLib's crash report (the panic's text on stderr, the crash hook), exit code 3.  plat runs its
/// threads and the program's start through it, so only the app's own `std::panic::catch_unwind`
/// (on any thread) keeps a panic from ending the program.  With `panic = "abort"` it is `f()`.
#[cfg(feature = "unwind")]
pub fn abort_on_unwind<R>(f: impl FnOnce() -> R) -> R {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(payload) => {
            let message = payload.downcast_ref::<&str>().copied().or_else(|| payload.downcast_ref::<String>().map(|s| s.as_str()));
            report_last(message, &message.unwrap_or("panicked (no message)"))
        }
    }
}

/// (feature `unwind`) the last panic on the calling thread as JFLib's crash report shows it
/// (`"panicked at FILE:LINE:COL:\nMESSAGE"`), taken: for a panic the app caught with
/// `std::panic::catch_unwind`, whose payload holds the message alone.  None when no panic came
/// since the last call (or the program is built with `panic = "abort"`).
#[cfg(feature = "unwind")]
pub fn panic_take() -> Option<String> {
    LAST.try_with(|l| l.try_borrow_mut().ok().and_then(|mut s| s.take())).ok().flatten()
}
