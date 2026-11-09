// apple_ios_motion -- iOS's motion sensors (Core Motion) and location (Core Location) for
// plat::sensors, through web_sensors.rs's host interface as Android's GallerySensors.java
//
//   sensors(mask) -> the bits this device has   CMMotionManager's device motion at 30 Hz, read on the
//                                main thread by a dispatch timer while something runs: the accelerometer
//                                (gravity + the user's acceleration), gravity, the rotation rate, the
//                                calibrated magnetic field and the attitude as the rotation estimate;
//                                with a magnetometer the reference frame is magnetic north (the
//                                estimate is absolute), else an arbitrary heading
//   location(op) -> LocationState   CLLocationManager: LOC_START asks "when in use" once (Info.plist's
//                                NSLocationWhenInUseUsageDescription), LOC_RESUME asks nobody, LOC_STOP
//                                stops; the answer and every fix come later through jfgui_location
//
// The library's conventions (sensors.rs): m/s^2 with +9.81 on z when the screen faces up (Core Motion
// says -1 g there), rad/s, microtesla, the rotation estimate a quaternion from the device's axes to
// east, north, up.  Core Motion's attitude is relative to its reference frame (x north, z up); which
// way its rotation matrix maps is decided against gravity and the magnetic field of the same sample
// (once sure, kept).  Everything on the main thread.

use core::cell::Cell;
use core::ffi::c_void;

use super::super::objc::{Id, NIL, Sel, class, msg, subclass};
use crate::sensors::{SENSOR_ACCEL, SENSOR_GRAVITY, SENSOR_GYRO, SENSOR_MAGNET, SENSOR_ROTATION};
use crate::web::{jfgui_location, jfgui_sensor};

#[link(name = "CoreMotion", kind = "framework")]
unsafe extern "C" {}
#[link(name = "CoreLocation", kind = "framework")]
unsafe extern "C" {}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct V3 {
    x: f64,
    y: f64,
    z: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CalibratedField {
    field: V3,
    accuracy: i32, // -1 uncalibrated, 0 low, 1 medium, 2 high
}

/// CMRotationMatrix: m11, m12, m13, m21 ... m33
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RotationMatrix {
    m: [f64; 9],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Coordinate {
    lat: f64,
    lon: f64,
}

const G: f64 = 9.80665;
const PERIOD_MS: i64 = 33;
// CMAttitudeReferenceFrame
const FRAME_ARBITRARY_Z: usize = 1 << 0;
const FRAME_MAGNETIC_NORTH: usize = 1 << 2;
// plat::sensors::LocationState
const SEARCHING: u32 = 1;
const FIX: u32 = 2;
const DENIED: u32 = 4;
const DISABLED: u32 = 5;
const UNAVAILABLE: u32 = 6;
// LOC_* (sensors.rs)
const OP_STOP: u32 = 0;
const OP_START: u32 = 1;
// CLAuthorizationStatus
const AUTH_NOT_DETERMINED: i32 = 0;
const AUTH_ALWAYS: i32 = 3;
const AUTH_WHEN_IN_USE: i32 = 4;

/// main thread only
struct Motion {
    manager: Cell<Id>,
    running: Cell<u32>,
    magnetic: Cell<bool>,
    timer: Cell<u32>,
    last: Cell<f64>,
    /// how Core Motion's rotation matrix maps: 0 not known yet, 1 its rows are the reference's axes
    /// in the device's coordinates, 2 its columns
    rows: Cell<u8>,
    locator: Cell<Id>,
    locating: Cell<bool>,
}
struct Main(Motion);
// SAFETY: only the main thread touches it (sensors.rs calls from the UI thread, the timer and the
// delegate run on the main queue).
unsafe impl Sync for Main {}
static M: Main = Main(Motion {
    manager: Cell::new(NIL),
    running: Cell::new(0),
    magnetic: Cell::new(false),
    timer: Cell::new(0),
    last: Cell::new(0.0),
    rows: Cell::new(0),
    locator: Cell::new(NIL),
    locating: Cell::new(false),
});

fn m() -> &'static Motion {
    &M.0
}

fn manager() -> Id {
    let s = m();
    if s.manager.get().is_null() {
        // SAFETY: a CMMotionManager of ours, kept for the process (Apple: one per app).
        s.manager.set(unsafe { msg!(msg!(class(c"CMMotionManager"), c"alloc"; Id), c"init"; Id) });
    }
    s.manager.get()
}

// ---------------------------------------------------------------- the motion sensors

/// the sensors of `mask` run, the others stop -> the bits of `mask` this device has
pub(crate) fn sensors(mask: u32) -> u32 {
    let s = m();
    let mgr = manager();
    if mgr.is_null() || !super::on_main() {
        return 0;
    }
    // SAFETY: the motion manager's queries and its start / stop on the main thread.
    unsafe {
        let motion = msg!(mgr, c"isDeviceMotionAvailable"; bool);
        let frames: usize = msg!(class(c"CMMotionManager"), c"availableAttitudeReferenceFrames"; usize);
        let magnet = motion && msg!(mgr, c"isMagnetometerAvailable"; bool) && frames & FRAME_MAGNETIC_NORTH != 0;
        let have = if motion { SENSOR_ACCEL | SENSOR_GRAVITY | SENSOR_GYRO | SENSOR_ROTATION | if magnet { SENSOR_MAGNET } else { 0 } } else { 0 };
        let want = mask & have;
        let magnetic = magnet && want & (SENSOR_MAGNET | SENSOR_ROTATION) != 0;
        let was = s.running.replace(want);
        if want == 0 {
            if was != 0 {
                msg!(mgr, c"stopDeviceMotionUpdates"; ());
            }
            s.timer.set(s.timer.get().wrapping_add(1)); // the poll ends
            return 0;
        }
        if was == 0 || magnetic != s.magnetic.get() {
            if was != 0 {
                msg!(mgr, c"stopDeviceMotionUpdates"; ());
            }
            msg!(mgr, c"setDeviceMotionUpdateInterval:", PERIOD_MS as f64 / 1000.0 => f64; ());
            let frame = if magnetic { FRAME_MAGNETIC_NORTH } else { FRAME_ARBITRARY_Z };
            msg!(mgr, c"startDeviceMotionUpdatesUsingReferenceFrame:", frame => usize; ());
            s.magnetic.set(magnetic);
            s.last.set(0.0);
        }
        if was == 0 {
            schedule();
        }
        want
    }
}

fn schedule() {
    let s = m();
    let id = s.timer.get().wrapping_add(1);
    s.timer.set(id);
    // SAFETY: a plain function on the main queue; its context is the timer's number.
    unsafe {
        super::dispatch_after_f(super::dispatch_time(0, PERIOD_MS * 1_000_000), super::main_queue(), id as usize as *mut c_void, poll)
    };
}

extern "C" fn poll(id: *mut c_void) {
    let s = m();
    if id as usize as u32 != s.timer.get() || s.running.get() == 0 {
        return;
    }
    read();
    schedule();
}

/// the newest sample of device motion, as the readings that run
fn read() {
    let s = m();
    let mgr = s.manager.get();
    let want = s.running.get();
    // SAFETY: the manager's newest device motion (nil before the first) and its properties.
    unsafe {
        let dm: Id = msg!(mgr, c"deviceMotion"; Id);
        if dm.is_null() {
            return;
        }
        let ts: f64 = msg!(dm, c"timestamp"; f64);
        if ts == s.last.replace(ts) {
            return;
        }
        let g: V3 = msg!(dm, c"gravity"; V3);
        let ua: V3 = msg!(dm, c"userAcceleration"; V3);
        let f = |v: f64| v as f32;
        if want & SENSOR_ACCEL != 0 {
            jfgui_sensor(SENSOR_ACCEL, f(-(g.x + ua.x) * G), f(-(g.y + ua.y) * G), f(-(g.z + ua.z) * G), 0.0, 3);
        }
        if want & SENSOR_GRAVITY != 0 {
            jfgui_sensor(SENSOR_GRAVITY, f(-g.x * G), f(-g.y * G), f(-g.z * G), 0.0, 3);
        }
        if want & SENSOR_GYRO != 0 {
            let r: V3 = msg!(dm, c"rotationRate"; V3);
            jfgui_sensor(SENSOR_GYRO, f(r.x), f(r.y), f(r.z), 0.0, 3);
        }
        let field: CalibratedField = if s.magnetic.get() { msg!(dm, c"magneticField"; CalibratedField) } else { CalibratedField { accuracy: -1, ..Default::default() } };
        if want & SENSOR_MAGNET != 0 && s.magnetic.get() {
            let a = (field.accuracy + 1).clamp(0, 3) as u32;
            jfgui_sensor(SENSOR_MAGNET, f(field.field.x), f(field.field.y), f(field.field.z), 0.0, a);
        }
        if want & SENSOR_ROTATION != 0 {
            let att: Id = msg!(dm, c"attitude"; Id);
            if !att.is_null() {
                let r: RotationMatrix = msg!(att, c"rotationMatrix"; RotationMatrix);
                if let Some(q) = quaternion(&r.m, g, &field) {
                    let absolute = if s.magnetic.get() { 4 } else { 0 };
                    jfgui_sensor(SENSOR_ROTATION, f(q[0]), f(q[1]), f(q[2]), f(q[3]), 3 | absolute);
                }
            }
        }
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(v: [f64; 3]) -> Option<[f64; 3]> {
    let n = rt::fx::sqrtd(dot(v, v));
    (n > 1e-9).then(|| [v[0] / n, v[1] / n, v[2] / n])
}

/// the rotation estimate (x, y, z, w): from the device's axes to east, north, up
fn quaternion(mat: &[f64; 9], g: V3, field: &CalibratedField) -> Option<[f64; 4]> {
    let s = m();
    // the reference frame's x (north) and z (up) in the device's coordinates: rows or columns
    let rows = ([mat[0], mat[1], mat[2]], [mat[6], mat[7], mat[8]]);
    let cols = ([mat[0], mat[3], mat[6]], [mat[2], mat[5], mat[8]]);
    let pick = match s.rows.get() {
        1 => true,
        2 => false,
        _ => {
            // which one agrees with gravity (tilted) and the magnetic field's horizontal part (flat)
            let up = unit([-g.x, -g.y, -g.z])?;
            let score = |(x, z): ([f64; 3], [f64; 3])| {
                let mut sc = dot(z, up);
                if field.accuracy >= 0 {
                    let fv = [field.field.x, field.field.y, field.field.z];
                    let d = dot(fv, up);
                    if let Some(h) = unit([fv[0] - d * up[0], fv[1] - d * up[1], fv[2] - d * up[2]]) {
                        sc += dot(x, h);
                    }
                }
                sc
            };
            let (a, b) = (score(rows), score(cols));
            if (a - b).abs() > 0.3 {
                s.rows.set(if a > b { 1 } else { 2 });
            }
            a >= b
        }
    };
    let (x, z) = if pick { rows } else { cols };
    let north = unit(x)?;
    let up = unit(z)?;
    let east = unit(cross(north, up))?;
    // R: device -> world, its rows the world's axes (east, north, up) in the device's coordinates
    let r = [east, north, up];
    let trace = r[0][0] + r[1][1] + r[2][2];
    let q = if trace > 0.0 {
        let t = rt::fx::sqrtd(trace + 1.0) * 2.0;
        [(r[2][1] - r[1][2]) / t, (r[0][2] - r[2][0]) / t, (r[1][0] - r[0][1]) / t, 0.25 * t]
    } else if r[0][0] > r[1][1] && r[0][0] > r[2][2] {
        let t = rt::fx::sqrtd(1.0 + r[0][0] - r[1][1] - r[2][2]) * 2.0;
        [0.25 * t, (r[0][1] + r[1][0]) / t, (r[0][2] + r[2][0]) / t, (r[2][1] - r[1][2]) / t]
    } else if r[1][1] > r[2][2] {
        let t = rt::fx::sqrtd(1.0 + r[1][1] - r[0][0] - r[2][2]) * 2.0;
        [(r[0][1] + r[1][0]) / t, 0.25 * t, (r[1][2] + r[2][1]) / t, (r[0][2] - r[2][0]) / t]
    } else {
        let t = rt::fx::sqrtd(1.0 + r[2][2] - r[0][0] - r[1][1]) * 2.0;
        [(r[0][2] + r[2][0]) / t, (r[1][2] + r[2][1]) / t, 0.25 * t, (r[1][0] - r[0][1]) / t]
    };
    Some(q)
}

// ---------------------------------------------------------------- the location

fn locator() -> Id {
    let s = m();
    if s.locator.get().is_null() {
        type Imp = *const c_void;
        let (cls, _) = subclass(c"NSObject", c"JFLocationDelegate", &[
            (c"locationManagerDidChangeAuthorization:", loc_auth as Imp, c"v@:@"),
            (c"locationManager:didUpdateLocations:", loc_update as Imp, c"v@:@@"),
            (c"locationManager:didFailWithError:", loc_fail as Imp, c"v@:@@"),
        ], &[c"CLLocationManagerDelegate"]);
        // SAFETY: a CLLocationManager and its delegate of ours, kept for the process (the manager
        // holds its delegate weakly, so the delegate is never released either).
        unsafe {
            let d: Id = msg!(msg!(cls, c"alloc"; Id), c"init"; Id);
            let lm: Id = msg!(msg!(class(c"CLLocationManager"), c"alloc"; Id), c"init"; Id);
            if !lm.is_null() {
                msg!(lm, c"setDelegate:", d => Id; ());
                msg!(lm, c"setDesiredAccuracy:", -1.0f64 => f64; ()); // kCLLocationAccuracyBest
            }
            s.locator.set(lm);
        }
    }
    s.locator.get()
}

fn report(state: u32) {
    jfgui_location(state, f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN, 0.0, 0);
}

fn status(lm: Id) -> i32 {
    // SAFETY: the manager's authorization (iOS 14 and later).
    unsafe { msg!(lm, c"authorizationStatus"; i32) }
}

/// LOC_STOP / LOC_START / LOC_RESUME -> LocationState (the fixes come later)
pub(crate) fn location(op: u32) -> u32 {
    let s = m();
    let lm = locator();
    if lm.is_null() || !super::on_main() {
        return UNAVAILABLE;
    }
    // SAFETY: the location manager's calls on the main thread.
    unsafe {
        if op == OP_STOP {
            s.locating.set(false);
            msg!(lm, c"stopUpdatingLocation"; ());
            return 0;
        }
        s.locating.set(true);
        if !msg!(class(c"CLLocationManager"), c"locationServicesEnabled"; bool) {
            return DISABLED;
        }
        match status(lm) {
            AUTH_ALWAYS | AUTH_WHEN_IN_USE => start(lm),
            AUTH_NOT_DETERMINED if op == OP_START => {
                msg!(lm, c"requestWhenInUseAuthorization"; ()); // the answer: locationManagerDidChangeAuthorization
                SEARCHING
            }
            _ => DENIED,
        }
    }
}

/// updates on; a fix the system knows already goes out at once -> Searching / Fix
fn start(lm: Id) -> u32 {
    // SAFETY: the location manager's calls on the main thread.
    unsafe {
        msg!(lm, c"startUpdatingLocation"; ());
        let known: Id = msg!(lm, c"location"; Id);
        if !known.is_null() && deliver(known) { FIX } else { SEARCHING }
    }
}

/// a CLLocation as a fix -> it was one (horizontal accuracy not negative)
fn deliver(l: Id) -> bool {
    // SAFETY: the properties of a live CLLocation.
    unsafe {
        let c: Coordinate = msg!(l, c"coordinate"; Coordinate);
        let acc: f64 = msg!(l, c"horizontalAccuracy"; f64);
        if acc < 0.0 {
            return false;
        }
        let valid = |v: f64| if v >= 0.0 { v } else { f64::NAN };
        let alt = if msg!(l, c"verticalAccuracy"; f64) >= 0.0 { msg!(l, c"altitude"; f64) } else { f64::NAN };
        let speed = valid(msg!(l, c"speed"; f64));
        let course = valid(msg!(l, c"course"; f64));
        let ts: Id = msg!(l, c"timestamp"; Id);
        let age = if ts.is_null() { 0.0 } else { (-msg!(ts, c"timeIntervalSinceNow"; f64) * 1000.0).max(0.0) };
        jfgui_location(FIX, c.lat, c.lon, acc, alt, speed, course, age, 0);
    }
    true
}

extern "C" fn loc_auth(_this: Id, _sel: Sel, lm: Id) {
    if !m().locating.get() {
        return;
    }
    match status(lm) {
        AUTH_ALWAYS | AUTH_WHEN_IN_USE => report(start(lm)),
        AUTH_NOT_DETERMINED => {}
        _ => report(DENIED),
    }
}

extern "C" fn loc_update(_this: Id, _sel: Sel, _lm: Id, locations: Id) {
    if !m().locating.get() || locations.is_null() {
        return;
    }
    // SAFETY: the newest CLLocation of the array UIKit hands over.
    let last: Id = unsafe { msg!(locations, c"lastObject"; Id) };
    if !last.is_null() {
        deliver(last);
    }
}

extern "C" fn loc_fail(_this: Id, _sel: Sel, _lm: Id, error: Id) {
    // SAFETY: the error's code (kCLErrorDenied = 1; kCLErrorLocationUnknown = 0: it keeps trying).
    let code: isize = if error.is_null() { 0 } else { unsafe { msg!(error, c"code"; isize) } };
    if m().locating.get() && code == 1 {
        report(DENIED);
    }
}
