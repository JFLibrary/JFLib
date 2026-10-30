// win_audio -- (audio.rs) WASAPI in shared mode: one thread per stream, event-driven
//
//   open(feed, name) -> Dev    the thread (COM multithreaded, "Audio" MMCSS class) opens the
//                              default render device (IMMDeviceEnumerator, eRender / eConsole),
//                              IAudioClient::Initialize in f32 with the stream's rate and
//                              channels (AUTOCONVERTPCM + SRC_DEFAULT_QUALITY: the engine
//                              converts to its mix format), a buffer of Core::dev_ms; open waits
//                              for that first answer (no device: Unavailable)
//   the loop                   waits for the buffer event or a kick; tops the buffer up from
//                              the Feed (GetCurrentPadding / GetBuffer / ReleaseBuffer); pause:
//                              Stop, stop(): Stop + Reset, the end: drained, then Stop
//   default device             an IMMNotificationClient of the thread: OnDefaultDeviceChanged
//                              (render, console) reopens on the new device; an invalidated
//                              device (unplugged) the same; none at all: State::Lost, retried
//                              every second and on the next change
//   system_sound(kind)         PlaySoundW(alias, SND_ALIAS | SND_ASYNC | SND_NODEFAULT |
//                              SND_SYSTEM): the sound scheme's own sound (none assigned: false)
//   Loopback                   (tests) the default render device's mix captured
//                              (AUDCLNT_STREAMFLAGS_LOOPBACK) on a thread of its own
//
// COM objects are used through their vtable slots (as win_spell.rs, win_taskbar.rs).  The
// stream's name is not shown: the volume mixer names the program.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

use super::{AudioError, D_LOST, D_RUNNING, Feed, SOUND_ERROR, SOUND_INFO, SOUND_NOTIFY, SOUND_QUESTION, SOUND_WARNING};
use crate::{Sem, Thread, thread_spawn};

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: usize, flags: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(clsid: *const Guid, outer: usize, ctx: u32, iid: *const Guid, out: *mut usize) -> i32;
    fn CoTaskMemFree(p: *mut u16);
    fn PropVariantClear(pv: *mut PropVariant) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateEventW(sa: usize, manual: i32, initial: i32, name: usize) -> isize;
    fn SetEvent(h: isize) -> i32;
    fn CloseHandle(h: isize) -> i32;
    fn WaitForMultipleObjects(n: u32, handles: *const isize, all: i32, ms: u32) -> u32;
    fn Sleep(ms: u32);
}

#[link(name = "avrt", kind = "raw-dylib")]
unsafe extern "system" {
    fn AvSetMmThreadCharacteristicsW(task: *const u16, index: *mut u32) -> isize;
    fn AvRevertMmThreadCharacteristics(h: isize) -> i32;
}

#[link(name = "winmm", kind = "raw-dylib")]
unsafe extern "system" {
    fn PlaySoundW(sound: *const u16, module: isize, flags: u32) -> i32;
}

/// a COM GUID
#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);

static CLSID_MMDEVICEENUMERATOR: Guid = Guid(0xBCDE_0395, 0xE52F, 0x467C, [0x8E, 0x3D, 0xC4, 0x57, 0x92, 0x91, 0x69, 0x2E]);
static IID_IMMDEVICEENUMERATOR: Guid = Guid(0xA956_64D2, 0x9614, 0x4F35, [0xA7, 0x46, 0xDE, 0x8D, 0xB6, 0x36, 0x17, 0xE6]);
static IID_IAUDIOCLIENT: Guid = Guid(0x1CB9_AD4C, 0xDBFA, 0x4C32, [0xB1, 0x78, 0xC2, 0xF5, 0x68, 0xA7, 0x03, 0xB2]);
static IID_IAUDIORENDERCLIENT: Guid = Guid(0xF294_ACFC, 0x3146, 0x4483, [0xA7, 0xBF, 0xAD, 0xDC, 0xA7, 0xC2, 0x60, 0xE2]);
static IID_IAUDIOCAPTURECLIENT: Guid = Guid(0xC8AD_BD64, 0xE71E, 0x48A0, [0xA4, 0xDE, 0x18, 0x5C, 0x39, 0x5C, 0xD3, 0x17]);
static IID_IUNKNOWN: Guid = Guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
static IID_IMMNOTIFICATIONCLIENT: Guid = Guid(0x7991_EEC9, 0x7E89, 0x4D85, [0x83, 0x90, 0x6C, 0x70, 0x3C, 0xEC, 0x60, 0xC0]);
// PKEY_Device_FriendlyName
static PKEY_FRIENDLY_NAME: PropertyKey =
    PropertyKey { fmtid: Guid(0xA45C_254E, 0xDF1C, 0x4EFD, [0x80, 0x20, 0x67, 0xD1, 0x46, 0xA8, 0x50, 0xE0]), pid: 14 };
// KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
const SUBTYPE_FLOAT: [u8; 16] = [3, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xAA, 0, 0x38, 0x9B, 0x71];

#[repr(C)]
struct PropertyKey {
    fmtid: Guid,
    pid: u32,
}

#[repr(C)]
struct PropVariant {
    vt: u16,
    reserved: [u16; 3],
    val: usize,
    extra: usize,
}

const CLSCTX_ALL: u32 = 0x17;
const COINIT_MULTITHREADED: u32 = 0;
const E_RENDER: u32 = 0;
const E_CONSOLE: u32 = 0;
const SHAREMODE_SHARED: u32 = 0;
const FLAG_LOOPBACK: u32 = 0x0002_0000;
const FLAG_EVENTCALLBACK: u32 = 0x0004_0000;
const FLAG_NOPERSIST: u32 = 0x0008_0000;
const FLAG_SRC_DEFAULT_QUALITY: u32 = 0x0800_0000;
const FLAG_AUTOCONVERTPCM: u32 = 0x8000_0000;
const BUFFERFLAGS_SILENT: u32 = 2;
const E_NOTFOUND: i32 = 0x8007_0490u32 as i32;
const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;
const AUDCLNT_E_DEVICE_INVALIDATED: i32 = 0x8889_0004u32 as i32;
const AUDCLNT_E_ENDPOINT_CREATE_FAILED: i32 = 0x8889_000Fu32 as i32;
const AUDCLNT_E_SERVICE_NOT_RUNNING: i32 = 0x8889_0010u32 as i32;
const VT_LPWSTR: u16 = 31;
const INFINITE: u32 = 0xFFFF_FFFF;

// vtable slots
const VT_RELEASE: usize = 2;
const EN_GET_DEFAULT: usize = 4; // IMMDeviceEnumerator::GetDefaultAudioEndpoint
const EN_REGISTER: usize = 6; // ::RegisterEndpointNotificationCallback
const EN_UNREGISTER: usize = 7;
const DEV_ACTIVATE: usize = 3; // IMMDevice::Activate
const DEV_OPEN_PROPS: usize = 4; // ::OpenPropertyStore
const PS_GET_VALUE: usize = 5; // IPropertyStore::GetValue
const AC_INITIALIZE: usize = 3; // IAudioClient
const AC_GET_BUFFER_SIZE: usize = 4;
const AC_GET_STREAM_LATENCY: usize = 5;
const AC_GET_PADDING: usize = 6;
const AC_GET_MIX_FORMAT: usize = 8;
const AC_START: usize = 10;
const AC_STOP: usize = 11;
const AC_RESET: usize = 12;
const AC_SET_EVENT: usize = 13;
const AC_GET_SERVICE: usize = 14;
const RC_GET_BUFFER: usize = 3; // IAudioRenderClient
const RC_RELEASE_BUFFER: usize = 4;
const CC_GET_BUFFER: usize = 3; // IAudioCaptureClient
const CC_RELEASE_BUFFER: usize = 4;
const CC_NEXT_PACKET: usize = 5;

/// the function in vtable slot `slot` of COM object `obj`
///
/// SAFETY: obj is a live COM interface pointer whose vtable has more than `slot` entries.
unsafe fn vfn(obj: usize, slot: usize) -> usize {
    unsafe { *(*(obj as *const *const usize)).add(slot) }
}

/// release a COM object (0: nothing)
fn release(obj: usize) {
    if obj != 0 {
        // SAFETY: obj is a live COM pointer we own; Release is slot 2 of every interface.
        unsafe {
            let f: unsafe extern "system" fn(usize) -> u32 = core::mem::transmute(vfn(obj, VT_RELEASE));
            f(obj);
        }
    }
}

// a method taking no argument (Start, Stop, Reset)
fn call0(obj: usize, slot: usize) -> i32 {
    // SAFETY: obj is a live IAudioClient; the slots used take only `this`.
    unsafe {
        let f: unsafe extern "system" fn(usize) -> i32 = core::mem::transmute(vfn(obj, slot));
        f(obj)
    }
}

// a method with one out-pointer (GetBufferSize, GetCurrentPadding, GetStreamLatency, ..)
fn call_out<T>(obj: usize, slot: usize, out: &mut T) -> i32 {
    // SAFETY: obj is a live COM object whose method in `slot` takes `this` and one pointer to T.
    unsafe {
        let f: unsafe extern "system" fn(usize, *mut T) -> i32 = core::mem::transmute(vfn(obj, slot));
        f(obj, out)
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().filter(|&c| c != 0).chain(core::iter::once(0)).collect()
}

/// an IMMDeviceEnumerator (COM must be initialised on this thread)
fn enumerator() -> Result<usize, i32> {
    let mut p = 0usize;
    // SAFETY: valid GUIDs and an out-pointer.
    let hr = unsafe { CoCreateInstance(&CLSID_MMDEVICEENUMERATOR, 0, CLSCTX_ALL, &IID_IMMDEVICEENUMERATOR, &mut p) };
    if hr < 0 || p == 0 { Err(hr) } else { Ok(p) }
}

/// the default render device (console role)
fn default_device(en: usize) -> Result<usize, i32> {
    let mut dev = 0usize;
    // SAFETY: GetDefaultAudioEndpoint(flow, role, IMMDevice**) on a live enumerator.
    let hr = unsafe {
        let f: unsafe extern "system" fn(usize, u32, u32, *mut usize) -> i32 = core::mem::transmute(vfn(en, EN_GET_DEFAULT));
        f(en, E_RENDER, E_CONSOLE, &mut dev)
    };
    if hr < 0 || dev == 0 { Err(if hr < 0 { hr } else { E_NOTFOUND }) } else { Ok(dev) }
}

/// a device's friendly name ("" unknown)
fn device_name(dev: usize) -> String {
    let mut store = 0usize;
    // SAFETY: OpenPropertyStore(STGM_READ, IPropertyStore**) on a live device.
    let hr = unsafe {
        let f: unsafe extern "system" fn(usize, u32, *mut usize) -> i32 = core::mem::transmute(vfn(dev, DEV_OPEN_PROPS));
        f(dev, 0, &mut store)
    };
    if hr < 0 || store == 0 {
        return String::new();
    }
    let mut pv = PropVariant { vt: 0, reserved: [0; 3], val: 0, extra: 0 };
    let mut name = String::new();
    // SAFETY: GetValue(&key, PROPVARIANT*) on the live store; a VT_LPWSTR value is a
    // NUL-terminated string the PROPVARIANT owns until PropVariantClear.
    unsafe {
        let f: unsafe extern "system" fn(usize, *const PropertyKey, *mut PropVariant) -> i32 = core::mem::transmute(vfn(store, PS_GET_VALUE));
        if f(store, &PKEY_FRIENDLY_NAME, &mut pv) >= 0 && pv.vt == VT_LPWSTR && pv.val != 0 {
            let p = pv.val as *const u16;
            let mut n = 0;
            while n < 512 && *p.add(n) != 0 { // the limit first: no read past it
                n += 1;
            }
            name = String::from_utf16_lossy(core::slice::from_raw_parts(p, n));
        }
        PropVariantClear(&mut pv);
    }
    release(store);
    name
}

/// IMMDevice::Activate(IID_IAudioClient)
fn activate_client(dev: usize) -> Result<usize, i32> {
    let mut client = 0usize;
    // SAFETY: Activate(iid, clsctx, NULL, void**) on a live device.
    let hr = unsafe {
        let f: unsafe extern "system" fn(usize, *const Guid, u32, usize, *mut usize) -> i32 = core::mem::transmute(vfn(dev, DEV_ACTIVATE));
        f(dev, &IID_IAUDIOCLIENT, CLSCTX_ALL, 0, &mut client)
    };
    if hr < 0 || client == 0 { Err(hr) } else { Ok(client) }
}

/// WAVEFORMATEXTENSIBLE: f32, rate, channels (40 bytes)
#[repr(C, align(8))]
struct WaveFormat([u8; 40]);

fn float_format(rate: u32, ch: u16) -> WaveFormat {
    let mut b = [0u8; 40];
    let block = ch * 4;
    b[0..2].copy_from_slice(&0xFFFEu16.to_le_bytes());
    b[2..4].copy_from_slice(&ch.to_le_bytes());
    b[4..8].copy_from_slice(&rate.to_le_bytes());
    b[8..12].copy_from_slice(&(rate * block as u32).to_le_bytes());
    b[12..14].copy_from_slice(&block.to_le_bytes());
    b[14..16].copy_from_slice(&32u16.to_le_bytes());
    b[16..18].copy_from_slice(&22u16.to_le_bytes());
    b[18..20].copy_from_slice(&32u16.to_le_bytes());
    // the WAVE channel masks: mono = centre, stereo, quad, 5.0, 5.1, 6.1, 7.1
    let mask: u32 = match ch {
        1 => 0x4,
        2 => 0x3,
        3 => 0x7,
        4 => 0x33,
        5 => 0x37,
        6 => 0x3F,
        7 => 0x13F,
        _ => 0x63F,
    };
    b[20..24].copy_from_slice(&mask.to_le_bytes());
    b[24..40].copy_from_slice(&SUBTYPE_FLOAT);
    WaveFormat(b)
}

// ---------------------------------------------------------------- the notification client

#[repr(C)]
struct NotifyVtbl {
    qi: unsafe extern "system" fn(*mut Notify, *const Guid, *mut usize) -> i32,
    add_ref: unsafe extern "system" fn(*mut Notify) -> u32,
    release: unsafe extern "system" fn(*mut Notify) -> u32,
    state_changed: unsafe extern "system" fn(*mut Notify, *const u16, u32) -> i32,
    added: unsafe extern "system" fn(*mut Notify, *const u16) -> i32,
    removed: unsafe extern "system" fn(*mut Notify, *const u16) -> i32,
    default_changed: unsafe extern "system" fn(*mut Notify, u32, u32, *const u16) -> i32,
    // the PROPERTYKEY (20 bytes) arrives by reference on x64
    property_changed: unsafe extern "system" fn(*mut Notify, *const u16, *const u8) -> i32,
}

/// our IMMNotificationClient: the default render device changed -> `changed`, the event set
#[repr(C)]
struct Notify {
    vtbl: *const NotifyVtbl,
    refs: AtomicU32,
    event: isize,
    changed: AtomicBool,
}

fn guid_eq(a: &Guid, b: &Guid) -> bool {
    a.0 == b.0 && a.1 == b.1 && a.2 == b.2 && a.3 == b.3
}

unsafe extern "system" fn n_qi(this: *mut Notify, iid: *const Guid, out: *mut usize) -> i32 {
    // SAFETY: COM passes valid pointers; `this` is our live Notify.
    unsafe {
        if guid_eq(&*iid, &IID_IUNKNOWN) || guid_eq(&*iid, &IID_IMMNOTIFICATIONCLIENT) {
            *out = this as usize;
            (*this).refs.fetch_add(1, Ordering::AcqRel);
            0
        } else {
            *out = 0;
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn n_add_ref(this: *mut Notify) -> u32 {
    // SAFETY: `this` is our live Notify (it outlives its registration).
    unsafe { (*this).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn n_release(this: *mut Notify) -> u32 {
    // SAFETY: as n_add_ref; the owner frees it after unregistering, not the count.
    unsafe { (*this).refs.fetch_sub(1, Ordering::AcqRel).saturating_sub(1) }
}

unsafe extern "system" fn n_ignore_state(_this: *mut Notify, _id: *const u16, _state: u32) -> i32 {
    0
}

unsafe extern "system" fn n_ignore(_this: *mut Notify, _id: *const u16) -> i32 {
    0
}

unsafe extern "system" fn n_default(this: *mut Notify, flow: u32, role: u32, _id: *const u16) -> i32 {
    if flow == E_RENDER && role == E_CONSOLE {
        // SAFETY: `this` is our live Notify; its event handle lives as long as it does.
        unsafe {
            (*this).changed.store(true, Ordering::Release);
            SetEvent((*this).event);
        }
    }
    0
}

unsafe extern "system" fn n_ignore_prop(_this: *mut Notify, _id: *const u16, _key: *const u8) -> i32 {
    0
}

static NOTIFY_VTBL: NotifyVtbl = NotifyVtbl {
    qi: n_qi,
    add_ref: n_add_ref,
    release: n_release,
    state_changed: n_ignore_state,
    added: n_ignore,
    removed: n_ignore,
    default_changed: n_default,
    property_changed: n_ignore_prop,
};

// ---------------------------------------------------------------- the stream thread

struct Ctl {
    /// kicks (auto-reset): state changes, closing, a new default device
    event: isize,
    closing: AtomicBool,
    /// the first open: 1 pending, 0 done, else the HRESULT
    result: AtomicI32,
    ready: Sem,
}

impl Drop for Ctl {
    fn drop(&mut self) {
        // SAFETY: our event handle, closed once (after the thread ended).
        unsafe { CloseHandle(self.event) };
    }
}

/// a running stream: its thread
pub(crate) struct Dev {
    ctl: Arc<Ctl>,
    thread: Option<Thread>,
}

fn map_err(hr: i32) -> AudioError {
    match hr {
        E_NOTFOUND | AUDCLNT_E_DEVICE_INVALIDATED | AUDCLNT_E_ENDPOINT_CREATE_FAILED | AUDCLNT_E_SERVICE_NOT_RUNNING => AudioError::Unavailable,
        _ => AudioError::Failed(hr),
    }
}

pub(crate) fn open(feed: Feed, _name: &str) -> Result<Dev, AudioError> {
    // SAFETY: an unnamed auto-reset event.
    let event = unsafe { CreateEventW(0, 0, 0, 0) };
    if event == 0 {
        return Err(AudioError::Failed(-1));
    }
    let ready = Sem::new(0).ok_or(AudioError::Failed(-1))?;
    let ctl = Arc::new(Ctl { event, closing: AtomicBool::new(false), result: AtomicI32::new(1), ready });
    let c = ctl.clone();
    let thread = thread_spawn(256 * 1024, Box::new(move || run(feed, c))).ok_or(AudioError::Failed(-1))?;
    ctl.ready.wait(10_000);
    match ctl.result.load(Ordering::Acquire) {
        0 => Ok(Dev { ctl, thread: Some(thread) }),
        r => {
            ctl.closing.store(true, Ordering::Release);
            // SAFETY: our event.
            unsafe { SetEvent(ctl.event) };
            thread.join();
            Err(if r == 1 { AudioError::Failed(-1) } else { map_err(r) })
        }
    }
}

pub(crate) fn kick(d: &Dev) {
    // SAFETY: the stream's event, alive while Dev is.
    unsafe { SetEvent(d.ctl.event) };
}

pub(crate) fn close(mut d: Dev) {
    d.ctl.closing.store(true, Ordering::Release);
    kick(&d);
    if let Some(t) = d.thread.take() {
        t.join();
    }
}

/// an initialised IAudioClient with its render client
struct Client {
    client: usize,
    render: usize,
    frames: u32,
    extra_us: u32,
    running: bool,
}

impl Client {
    fn open(en: usize, feed: &Feed, audio_event: isize) -> Result<Client, i32> {
        let core = &feed.core;
        let dev = default_device(en)?;
        core.set_device(&device_name(dev));
        let client = activate_client(dev);
        release(dev);
        let client = client?;
        let mut c = Client { client, render: 0, frames: 0, extra_us: 0, running: false };
        let fmt = float_format(core.rate, core.channels);
        let flags = FLAG_EVENTCALLBACK | FLAG_AUTOCONVERTPCM | FLAG_SRC_DEFAULT_QUALITY | FLAG_NOPERSIST;
        let dur = core.dev_ms as i64 * 10_000;
        // SAFETY: Initialize(mode, flags, duration, periodicity, format, NULL) on our client; the
        // format lives during the call.
        let hr = unsafe {
            let f: unsafe extern "system" fn(usize, u32, u32, i64, i64, *const u8, usize) -> i32 = core::mem::transmute(vfn(client, AC_INITIALIZE));
            f(client, SHAREMODE_SHARED, flags, dur, 0, fmt.0.as_ptr(), 0)
        };
        if hr < 0 {
            return Err(hr);
        }
        let hr = call_out(client, AC_GET_BUFFER_SIZE, &mut c.frames);
        if hr < 0 {
            return Err(hr);
        }
        let mut lat = 0i64;
        if call_out(client, AC_GET_STREAM_LATENCY, &mut lat) >= 0 {
            c.extra_us = (lat / 10).clamp(0, u32::MAX as i64) as u32;
        }
        // SAFETY: GetService(riid, void**) and SetEventHandle(handle) on our client.
        let hr = unsafe {
            let f: unsafe extern "system" fn(usize, *const Guid, *mut usize) -> i32 = core::mem::transmute(vfn(client, AC_GET_SERVICE));
            f(client, &IID_IAUDIORENDERCLIENT, &mut c.render)
        };
        if hr < 0 || c.render == 0 {
            return Err(if hr < 0 { hr } else { E_NOINTERFACE });
        }
        // SAFETY: as above.
        let hr = unsafe {
            let f: unsafe extern "system" fn(usize, isize) -> i32 = core::mem::transmute(vfn(client, AC_SET_EVENT));
            f(client, audio_event)
        };
        if hr < 0 {
            return Err(hr);
        }
        core.dev_extra_us.store(c.extra_us, Ordering::Relaxed);
        Ok(c)
    }

    /// frames waiting in the device's buffer
    fn padding(&self) -> Result<u32, i32> {
        let mut p = 0u32;
        let hr = call_out(self.client, AC_GET_PADDING, &mut p);
        if hr < 0 { Err(hr) } else { Ok(p) }
    }

    /// fill the free part of the buffer from the feed -> (frames written, more to come)
    fn top_up(&self, feed: &mut Feed) -> Result<(u32, bool), i32> {
        let free = self.frames - self.padding()?.min(self.frames);
        if free == 0 {
            return Ok((0, true));
        }
        let ch = feed.core.channels as usize;
        let mut p: *mut u8 = core::ptr::null_mut();
        // SAFETY: GetBuffer(frames, BYTE**) on our render client.
        let hr = unsafe {
            let f: unsafe extern "system" fn(usize, u32, *mut *mut u8) -> i32 = core::mem::transmute(vfn(self.render, RC_GET_BUFFER));
            f(self.render, free, &mut p)
        };
        if hr < 0 || p.is_null() {
            return Err(if hr < 0 { hr } else { E_NOINTERFACE });
        }
        // SAFETY: the engine hands out `free` frames of our format (f32 x channels), aligned for
        // f32, ours until ReleaseBuffer.
        let buf = unsafe { core::slice::from_raw_parts_mut(p as *mut f32, free as usize * ch) };
        let more = feed.render(buf, 0);
        // SAFETY: ReleaseBuffer(frames, flags) for the buffer just filled.
        let hr = unsafe {
            let f: unsafe extern "system" fn(usize, u32, u32) -> i32 = core::mem::transmute(vfn(self.render, RC_RELEASE_BUFFER));
            f(self.render, free, 0)
        };
        if hr < 0 { Err(hr) } else { Ok((free, more)) }
    }

    fn start(&mut self) -> i32 {
        let hr = call0(self.client, AC_START);
        self.running = hr >= 0;
        hr
    }

    fn stop(&mut self) {
        call0(self.client, AC_STOP);
        self.running = false;
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if self.running {
            call0(self.client, AC_STOP);
        }
        release(self.render);
        release(self.client);
    }
}

// the stream's thread
fn run(mut feed: Feed, ctl: Arc<Ctl>) {
    // SAFETY: COM for this thread (multithreaded apartment), undone at the end.
    unsafe { CoInitializeEx(0, COINIT_MULTITHREADED) };
    let task = wide("Audio");
    let mut task_index = 0u32;
    // SAFETY: a valid task name and out-pointer; reverted at the end.
    let mm = unsafe { AvSetMmThreadCharacteristicsW(task.as_ptr(), &mut task_index) };
    // SAFETY: an unnamed auto-reset event for the engine.
    let audio_event = unsafe { CreateEventW(0, 0, 0, 0) };
    let en = enumerator();
    let notify = Box::new(Notify { vtbl: &NOTIFY_VTBL, refs: AtomicU32::new(1), event: ctl.event, changed: AtomicBool::new(false) });
    let notify_ptr = &*notify as *const Notify as usize;
    let mut registered = false;
    let first = match en {
        Ok(en) => {
            // SAFETY: RegisterEndpointNotificationCallback(client) with our live Notify.
            registered = unsafe {
                let f: unsafe extern "system" fn(usize, usize) -> i32 = core::mem::transmute(vfn(en, EN_REGISTER));
                f(en, notify_ptr) >= 0
            };
            Client::open(en, &feed, audio_event)
        }
        Err(hr) => Err(hr),
    };
    let core = feed.core.clone();
    match first {
        Ok(client) => {
            ctl.result.store(0, Ordering::Release);
            ctl.ready.post(1);
            core.dev.store(D_RUNNING, Ordering::Release);
            if let Ok(en) = en {
                stream_loop(&mut feed, &ctl, &notify, en, audio_event, Some(client));
            }
        }
        Err(hr) => {
            ctl.result.store(hr, Ordering::Release);
            ctl.ready.post(1);
        }
    }
    if let Ok(en) = en {
        if registered {
            // SAFETY: the same enumerator and client as registered.
            unsafe {
                let f: unsafe extern "system" fn(usize, usize) -> i32 = core::mem::transmute(vfn(en, EN_UNREGISTER));
                f(en, notify_ptr);
            }
        }
        release(en);
    }
    drop(notify);
    // SAFETY: our handles / registrations, undone once.
    unsafe {
        CloseHandle(audio_event);
        if mm != 0 {
            AvRevertMmThreadCharacteristics(mm);
        }
        CoUninitialize();
    }
}

fn stream_loop(feed: &mut Feed, ctl: &Ctl, notify: &Notify, en: usize, audio_event: isize, mut client: Option<Client>) {
    let core = feed.core.clone();
    // the feed has more after its last buffer (false: drain, then the end)
    let mut more = true;
    loop {
        if ctl.closing.load(Ordering::Acquire) {
            break;
        }
        // a new default device, or the old one went away: reopen (the ring stays)
        if notify.changed.swap(false, Ordering::AcqRel) || core.test_change.swap(false, Ordering::AcqRel) || client.is_none() {
            let had = client.is_some();
            client = None;
            match Client::open(en, feed, audio_event) {
                Ok(c) => {
                    client = Some(c);
                    if had || core.dev.load(Ordering::Acquire) == D_LOST {
                        core.changes.fetch_add(1, Ordering::Relaxed);
                    }
                    core.dev.store(D_RUNNING, Ordering::Release);
                }
                Err(_) => {
                    core.dev.store(D_LOST, Ordering::Release);
                    core.dev_queued.store(0, Ordering::Relaxed);
                    // SAFETY: our event; wakes for kicks, closing and device changes.
                    unsafe { WaitForMultipleObjects(1, &ctl.event, 0, 1000) };
                    continue;
                }
            }
        }
        let Some(c) = client.as_mut() else { continue };
        let mut failed = false;
        if core.take_flush_device() {
            feed.apply_flush();
            c.stop();
            call0(c.client, AC_RESET);
            more = true;
        }
        let want = feed.wants_device();
        if want && !c.running {
            // prefill, then start
            match c.top_up(feed) {
                Ok((_, m)) => more = m,
                Err(_) => failed = true,
            }
            failed = failed || c.start() < 0;
        } else if !want && c.running {
            c.stop();
        } else if c.running && more {
            match c.top_up(feed) {
                Ok((_, m)) => more = m,
                Err(_) => failed = true,
            }
        } else if c.running {
            match c.padding() {
                Ok(0) => {
                    // everything played: the end; the device stops until start()
                    core.ended.store(true, Ordering::Release);
                    c.stop();
                    more = true;
                }
                Ok(_) => {}
                Err(_) => failed = true,
            }
        }
        if !failed {
            match c.padding() {
                Ok(p) => core.dev_queued.store(p, Ordering::Relaxed),
                Err(_) => failed = true,
            }
        }
        if failed {
            // invalidated (unplugged, a format change): open the default device again
            client = None;
            continue;
        }
        let handles = [audio_event, ctl.event];
        let ms = if c.running { 200 } else { INFINITE };
        // SAFETY: two valid event handles.
        unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, ms) };
    }
}

pub(crate) fn available() -> bool {
    // SAFETY: COM for this thread if it has none (an existing apartment stays).
    let hr = unsafe { CoInitializeEx(0, COINIT_MULTITHREADED) };
    let ok = match enumerator() {
        Ok(en) => {
            let r = default_device(en);
            if let Ok(d) = r {
                release(d);
            }
            release(en);
            r.is_ok()
        }
        Err(_) => false,
    };
    if hr >= 0 {
        // SAFETY: balances the successful CoInitializeEx above.
        unsafe { CoUninitialize() };
    }
    ok
}

pub(crate) fn system_sound(kind: u32) -> Option<bool> {
    let alias = match kind {
        SOUND_INFO => "SystemAsterisk",
        SOUND_WARNING => "SystemExclamation",
        SOUND_ERROR => "SystemHand",
        SOUND_QUESTION => "SystemQuestion",
        SOUND_NOTIFY => "Notification.Default",
        _ => ".Default",
    };
    let w = wide(alias);
    const SND_ASYNC: u32 = 0x1;
    const SND_NODEFAULT: u32 = 0x2;
    const SND_ALIAS: u32 = 0x1_0000;
    const SND_SYSTEM: u32 = 0x20_0000;
    // SAFETY: a NUL-terminated alias; asynchronous, the string is copied.
    Some(unsafe { PlaySoundW(w.as_ptr(), 0, SND_ALIAS | SND_ASYNC | SND_NODEFAULT | SND_SYSTEM) } != 0)
}

pub(crate) fn dialog_sounds() -> bool {
    true
}

// ---------------------------------------------------------------- loopback (tests)

struct LoopCtl {
    stop: AtomicBool,
    result: AtomicI32,
    ready: Sem,
    out: rt::sync::SpinLock<(u32, u16, Vec<f32>)>,
}

/// (tests) the default render device's mix captured while it runs (WASAPI loopback)
#[doc(hidden)]
pub struct Loopback {
    ctl: Arc<LoopCtl>,
    thread: Option<Thread>,
}

impl Loopback {
    /// start capturing -> Err(Unavailable): no render device
    pub fn start() -> Result<Loopback, AudioError> {
        let ctl = Arc::new(LoopCtl {
            stop: AtomicBool::new(false),
            result: AtomicI32::new(1),
            ready: Sem::new(0).ok_or(AudioError::Failed(-1))?,
            out: rt::sync::SpinLock::new((0, 0, Vec::new())),
        });
        let c = ctl.clone();
        let thread = thread_spawn(256 * 1024, Box::new(move || capture(&c))).ok_or(AudioError::Failed(-1))?;
        ctl.ready.wait(10_000);
        match ctl.result.load(Ordering::Acquire) {
            0 => Ok(Loopback { ctl, thread: Some(thread) }),
            r => {
                ctl.stop.store(true, Ordering::Release);
                thread.join();
                Err(if r == 1 { AudioError::Failed(-1) } else { map_err(r) })
            }
        }
    }

    /// stop -> (frames per second, channels, interleaved samples)
    pub fn finish(mut self) -> (u32, u16, Vec<f32>) {
        self.ctl.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            t.join();
        }
        core::mem::take(&mut *self.ctl.out.lock())
    }
}

fn capture(ctl: &LoopCtl) {
    // SAFETY: COM for this thread, undone at the end.
    unsafe { CoInitializeEx(0, COINIT_MULTITHREADED) };
    let r = capture_run(ctl);
    if let Err(hr) = r {
        ctl.result.store(hr, Ordering::Release);
        ctl.ready.post(1);
    }
    // SAFETY: balances CoInitializeEx.
    unsafe { CoUninitialize() };
}

fn capture_run(ctl: &LoopCtl) -> Result<(), i32> {
    let en = enumerator()?;
    let dev = default_device(en);
    release(en);
    let dev = dev?;
    let client = activate_client(dev);
    release(dev);
    let client = client?;
    let mut fmt: *mut u8 = core::ptr::null_mut();
    let hr = call_out(client, AC_GET_MIX_FORMAT, &mut fmt);
    if hr < 0 || fmt.is_null() {
        release(client);
        return Err(if hr < 0 { hr } else { E_NOINTERFACE });
    }
    // SAFETY: the mix format is a WAVEFORMATEX(TENSIBLE) the engine allocated (>= 18 bytes;
    // the sub-format only when cbSize >= 22).
    let (tag, ch, rate, bits) = unsafe {
        let b = core::slice::from_raw_parts(fmt, 18);
        let mut tag = u16::from_le_bytes([b[0], b[1]]);
        let cb = u16::from_le_bytes([b[16], b[17]]);
        if tag == 0xFFFE && cb >= 22 {
            let e = core::slice::from_raw_parts(fmt, 40);
            tag = u16::from_le_bytes([e[24], e[25]]);
        }
        (tag, u16::from_le_bytes([b[2], b[3]]), u32::from_le_bytes([b[4], b[5], b[6], b[7]]), u16::from_le_bytes([b[14], b[15]]))
    };
    // SAFETY: Initialize(shared, loopback, 200 ms, 0, the mix format, NULL), then the format is
    // freed (CoTaskMemAlloc'd by GetMixFormat).
    let hr = unsafe {
        let f: unsafe extern "system" fn(usize, u32, u32, i64, i64, *const u8, usize) -> i32 = core::mem::transmute(vfn(client, AC_INITIALIZE));
        let hr = f(client, SHAREMODE_SHARED, FLAG_LOOPBACK, 2_000_000, 0, fmt, 0);
        CoTaskMemFree(fmt as *mut u16);
        hr
    };
    let float = tag == 3 && bits == 32;
    if hr < 0 || !(float || (tag == 1 && bits == 16)) {
        release(client);
        return Err(if hr < 0 { hr } else { AUDCLNT_E_ENDPOINT_CREATE_FAILED });
    }
    let mut cap = 0usize;
    // SAFETY: GetService(IID_IAudioCaptureClient, void**) on our client.
    let hr = unsafe {
        let f: unsafe extern "system" fn(usize, *const Guid, *mut usize) -> i32 = core::mem::transmute(vfn(client, AC_GET_SERVICE));
        f(client, &IID_IAUDIOCAPTURECLIENT, &mut cap)
    };
    if hr < 0 || cap == 0 || call0(client, AC_START) < 0 {
        release(cap);
        release(client);
        return Err(if hr < 0 { hr } else { E_NOINTERFACE });
    }
    ctl.result.store(0, Ordering::Release);
    ctl.ready.post(1);
    let mut data: Vec<f32> = Vec::new();
    let chn = ch as usize;
    while !ctl.stop.load(Ordering::Acquire) {
        loop {
            let mut next = 0u32;
            if call_out(cap, CC_NEXT_PACKET, &mut next) < 0 || next == 0 {
                break;
            }
            let (mut p, mut frames, mut flags) = (core::ptr::null_mut::<u8>(), 0u32, 0u32);
            let (mut pos, mut qpc) = (0u64, 0u64);
            // SAFETY: GetBuffer(BYTE**, frames*, flags*, pos*, qpc*) on our capture client.
            let hr = unsafe {
                let f: unsafe extern "system" fn(usize, *mut *mut u8, *mut u32, *mut u32, *mut u64, *mut u64) -> i32 =
                    core::mem::transmute(vfn(cap, CC_GET_BUFFER));
                f(cap, &mut p, &mut frames, &mut flags, &mut pos, &mut qpc)
            };
            if hr < 0 {
                break;
            }
            let n = frames as usize * chn;
            if flags & BUFFERFLAGS_SILENT != 0 || p.is_null() {
                data.resize(data.len() + n, 0.0);
            } else if float {
                // SAFETY: `frames` frames of f32 x channels from the engine until ReleaseBuffer.
                data.extend_from_slice(unsafe { core::slice::from_raw_parts(p as *const f32, n) });
            } else {
                // SAFETY: the same for 16-bit samples.
                let s = unsafe { core::slice::from_raw_parts(p as *const i16, n) };
                data.extend(s.iter().map(|&v| v as f32 / 32768.0));
            }
            // SAFETY: releases the packet just read.
            unsafe {
                let f: unsafe extern "system" fn(usize, u32) -> i32 = core::mem::transmute(vfn(cap, CC_RELEASE_BUFFER));
                f(cap, frames);
            }
        }
        // SAFETY: no memory involved.
        unsafe { Sleep(5) };
    }
    call0(client, AC_STOP);
    release(cap);
    release(client);
    *ctl.out.lock() = (rate, ch, data);
    Ok(())
}
