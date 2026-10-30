// win_winrt -- the few WinRT calls of plat through raw COM (no projection, no libc): the
// StartupTask of an MSIX package (win_autostart.rs) and its update state (pkg.rs)
//
//   (crate) mta(f) -> Option<T>    f on a new thread in the multithreaded apartment: WinRT's
//                                  asynchronous operations complete there without a message
//                                  loop, and the caller's apartment (the UI thread's STA) is
//                                  left alone; None when no thread could be made
//   (crate) Obj                    an owned interface pointer: vfn(slot) for its methods,
//                                  query(iid) for another interface, released when dropped
//   (crate) factory(class, iid)    RoGetActivationFactory of a runtime class -> Obj / HRESULT
//   (crate) HStr::new(s)           an HSTRING for a call (deleted when dropped); text(h) reads one
//   (crate) wait(op, ms)           IAsyncInfo.Status polled every 5 ms until Completed (Ok),
//                                  Error (its ErrorCode) or Canceled; E_TIMEOUT after ms
//
// combase.dll is linked like kernel32 (raw-dylib); every object made here is released.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use rt::sync::SpinLock;

#[link(name = "combase", kind = "raw-dylib")]
unsafe extern "system" {
    fn RoInitialize(kind: u32) -> i32;
    fn RoUninitialize();
    fn RoGetActivationFactory(class: isize, iid: *const Guid, out: *mut usize) -> i32;
    fn WindowsCreateString(s: *const u16, n: u32, out: *mut isize) -> i32;
    fn WindowsDeleteString(h: isize) -> i32;
    fn WindowsGetStringRawBuffer(h: isize, n: *mut u32) -> *const u16;
}

/// a COM GUID
#[repr(C)]
pub(crate) struct Guid(pub u32, pub u16, pub u16, pub [u8; 8]);

static IID_IASYNCINFO: Guid = Guid(0x0000_0036, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);

const RO_INIT_MULTITHREADED: u32 = 1;
/// HRESULT_FROM_WIN32(ERROR_TIMEOUT)
pub(crate) const E_TIMEOUT: i32 = 0x8007_05B4u32 as i32;
/// E_ABORT (a cancelled operation)
const E_ABORT: i32 = 0x8000_4004u32 as i32;
/// E_FAIL
pub(crate) const E_FAIL: i32 = 0x8000_4005u32 as i32;

// IUnknown / IAsyncInfo vtable slots (IInspectable takes 3..5)
const VT_QI: usize = 0;
const VT_RELEASE: usize = 2;
const AI_STATUS: usize = 7;
const AI_ERRORCODE: usize = 8;
// AsyncStatus
const ST_STARTED: i32 = 0;
const ST_COMPLETED: i32 = 1;
const ST_CANCELED: i32 = 2;

/// an owned COM interface pointer (released when dropped)
pub(crate) struct Obj(usize);

impl Obj {
    /// take over a pointer a call returned (0: None)
    pub(crate) fn own(p: usize) -> Option<Obj> {
        // not then_some(Obj(p)): that would make Obj(0) and drop it (Release on null)
        if p == 0 { None } else { Some(Obj(p)) }
    }

    pub(crate) fn ptr(&self) -> usize {
        self.0
    }

    /// the function in vtable slot `slot`
    ///
    /// SAFETY: the object's vtable has more than `slot` entries; the caller transmutes the
    /// result to the slot's real signature.
    pub(crate) unsafe fn vfn(&self, slot: usize) -> usize {
        unsafe { *(*(self.0 as *const *const usize)).add(slot) }
    }

    /// IUnknown::QueryInterface (None: not supported)
    pub(crate) fn query(&self, iid: &Guid) -> Option<Obj> {
        let mut p = 0usize;
        // SAFETY: slot 0 of every COM object is QueryInterface(this, riid, out).
        let hr = unsafe {
            let qi: unsafe extern "system" fn(usize, *const Guid, *mut usize) -> i32 = core::mem::transmute(self.vfn(VT_QI));
            qi(self.0, iid, &mut p)
        };
        if hr < 0 { None } else { Obj::own(p) }
    }

    /// a method without arguments that returns an interface pointer (a getter, an async call)
    pub(crate) fn get_obj(&self, slot: usize) -> Result<Obj, i32> {
        let mut p = 0usize;
        // SAFETY: the caller names a slot of the form HRESULT (this, out pointer).
        let hr = unsafe {
            let f: unsafe extern "system" fn(usize, *mut usize) -> i32 = core::mem::transmute(self.vfn(slot));
            f(self.0, &mut p)
        };
        if hr < 0 {
            return Err(hr);
        }
        Obj::own(p).ok_or(E_FAIL)
    }

    /// a method without arguments that returns a 32-bit value (an enum, a bool as 0 / 1)
    pub(crate) fn get_i32(&self, slot: usize) -> Result<i32, i32> {
        let mut v = 0i32;
        // SAFETY: the caller names a slot of the form HRESULT (this, out 32-bit value).
        let hr = unsafe {
            let f: unsafe extern "system" fn(usize, *mut i32) -> i32 = core::mem::transmute(self.vfn(slot));
            f(self.0, &mut v)
        };
        if hr < 0 { Err(hr) } else { Ok(v) }
    }

    /// a method without arguments that returns an HSTRING, as text
    pub(crate) fn get_text(&self, slot: usize) -> Result<String, i32> {
        let mut h = 0isize;
        // SAFETY: the caller names a slot of the form HRESULT (this, out HSTRING).
        let hr = unsafe {
            let f: unsafe extern "system" fn(usize, *mut isize) -> i32 = core::mem::transmute(self.vfn(slot));
            f(self.0, &mut h)
        };
        if hr < 0 {
            return Err(hr);
        }
        let s = text(h);
        // SAFETY: the HSTRING the call handed over (0 is the empty string).
        unsafe { WindowsDeleteString(h) };
        Ok(s)
    }

    /// a method without arguments and results
    pub(crate) fn call0(&self, slot: usize) -> Result<(), i32> {
        // SAFETY: the caller names a slot of the form HRESULT (this).
        let hr = unsafe {
            let f: unsafe extern "system" fn(usize) -> i32 = core::mem::transmute(self.vfn(slot));
            f(self.0)
        };
        if hr < 0 { Err(hr) } else { Ok(()) }
    }
}

impl Drop for Obj {
    fn drop(&mut self) {
        // SAFETY: slot 2 of every COM object is Release(this); the pointer is ours.
        unsafe {
            let f: unsafe extern "system" fn(usize) -> u32 = core::mem::transmute(self.vfn(VT_RELEASE));
            f(self.0);
        }
    }
}

/// an HSTRING made for a call
pub(crate) struct HStr(isize);

impl HStr {
    pub(crate) fn new(s: &str) -> Option<HStr> {
        let w: Vec<u16> = s.encode_utf16().collect();
        let mut h = 0isize;
        // SAFETY: w.len() UTF-16 units; WindowsCreateString copies them.
        let hr = unsafe { WindowsCreateString(w.as_ptr(), w.len() as u32, &mut h) };
        (hr >= 0).then_some(HStr(h))
    }

    pub(crate) fn raw(&self) -> isize {
        self.0
    }
}

impl Drop for HStr {
    fn drop(&mut self) {
        // SAFETY: our HSTRING.
        unsafe { WindowsDeleteString(self.0) };
    }
}

/// the text of an HSTRING (0: "")
pub(crate) fn text(h: isize) -> String {
    if h == 0 {
        return String::new();
    }
    let mut n = 0u32;
    // SAFETY: a live HSTRING; the buffer holds n UTF-16 units and lives as long as h.
    unsafe {
        let p = WindowsGetStringRawBuffer(h, &mut n);
        if p.is_null() {
            return String::new();
        }
        String::from_utf16_lossy(core::slice::from_raw_parts(p, n as usize))
    }
}

/// the activation factory (or the statics) of a runtime class, as the interface iid
pub(crate) fn factory(class: &str, iid: &Guid) -> Result<Obj, i32> {
    let name = HStr::new(class).ok_or(E_FAIL)?;
    let mut p = 0usize;
    // SAFETY: a valid HSTRING and GUID, an out-pointer.
    let hr = unsafe { RoGetActivationFactory(name.raw(), iid, &mut p) };
    if hr < 0 {
        return Err(hr);
    }
    Obj::own(p).ok_or(E_FAIL)
}

/// wait for an asynchronous operation (IAsyncOperation<T>: its IAsyncInfo) -> Ok when it
/// completed; its ErrorCode, E_ABORT when cancelled, E_TIMEOUT after ms
pub(crate) fn wait(op: &Obj, ms: u32) -> Result<(), i32> {
    let info = op.query(&IID_IASYNCINFO).ok_or(E_FAIL)?;
    let end = crate::time_us() + ms as u64 * 1000;
    loop {
        match info.get_i32(AI_STATUS)? {
            ST_COMPLETED => return Ok(()),
            ST_CANCELED => return Err(E_ABORT),
            ST_STARTED => {}
            _ => {
                let code = info.get_i32(AI_ERRORCODE).unwrap_or(E_FAIL);
                return Err(if code < 0 { code } else { E_FAIL });
            }
        }
        if crate::time_us() >= end {
            return Err(E_TIMEOUT);
        }
        crate::sleep_ms(5);
    }
}

/// f on a new MTA thread (RoInitialize there, RoUninitialize after it) -> its result; None
/// when no thread could be made
pub(crate) fn mta<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let out: Arc<SpinLock<Option<T>>> = Arc::new(SpinLock::new(None));
    let slot = out.clone();
    let t = crate::thread_spawn(256 * 1024, Box::new(move || {
        // SAFETY: a fresh thread; S_FALSE / RPC_E_CHANGED_MODE cannot happen here, and WinRT
        // stays usable either way.
        let hr = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let r = f();
        *slot.lock() = Some(r);
        if hr >= 0 {
            // SAFETY: pairs the successful RoInitialize above; every object of f is released.
            unsafe { RoUninitialize() };
        }
    }))?;
    t.join();
    out.lock().take()
}
