// sensors -- the device's motion sensors and its location for the app
//
//   watch(SENSOR_*) -> SensorWatch   these sensors run while the guard lives (counted per sensor:
//                                    the last guard stops them); nothing runs without a guard.
//                                    Needs `sensors` in the manifest (else state() is Denied,
//                                    "PERMDENY sensors")
//   state(SENSOR_X) -> SensorState   Off, Waiting (started, no reading yet), Running, Paused (the
//                                    app is in the background), Unavailable (this device has no
//                                    such sensor, or the platform none at all), Denied
//   read(SENSOR_X) -> Option<Reading>   the last reading (kept while Paused, gone when stopped)
//   attitude() -> Option<Attitude>   pitch, roll and heading from what runs: gravity (the
//                                    gravity sensor, the rotation estimate, the accelerometer, in
//                                    this order), heading from the rotation estimate, otherwise
//                                    computed from gravity and the magnetic field
//   attitude_of(gravity, rotation, magnet) -> Option<Attitude>   the same from given values
//   location_watch() -> LocationWatch   the location runs while the guard lives; the first start
//                                    may ask the user (call it from a user action: Android's
//                                    runtime permission, the browser's prompt).  Needs `location`
//                                    in the manifest (else Denied, "PERMDENY location")
//   location_state() -> LocationState   Off, Searching, Fix, Paused, Denied (the user or the
//                                    manifest refused), Disabled (the system's location switch
//                                    is off), Unavailable (no location service here)
//   location_fix() -> Option<Fix>    the last fix: latitude, longitude, and what the provider
//                                    gave of accuracy, altitude, speed, bearing; age_ms()
//   take() -> (bits, location)       (ui) what is new since the last call
//   suspend(on)                      (ui) the app went to the background / came back: everything
//                                    stops, Paused, and starts again without asking
//
// New readings reach the UI thread like the host's other events: plat::wake(), then E_WAKE, and
// the UI turns them into App::command(UIC_SENSOR, bits) and App::command(UIC_LOCATION, state);
// nothing polls.  A sensor's axes are the device's: x to the right of the screen, y to its top, z
// out of it; accelerometer and gravity in m/s^2 (lying flat, screen up: z = +9.81), gyroscope in
// rad/s, magnetic field in microtesla, the rotation estimate a unit quaternion (x, y, z, w) from
// the device's axes to east, north, up (Reading::absolute: its zero is north; false: wherever the
// device pointed when it started).
//
// Android (android/GallerySensors.java, web_sensors.rs): SensorManager and LocationManager (GPS
// and network providers, no Google Play services), ACCESS_FINE_LOCATION / ACCESS_COARSE_LOCATION
// asked when location_watch starts.  Browser (web/sensors.js, web_sensors.rs): DeviceMotionEvent,
// DeviceOrientationEvent (Safari's requestPermission from the click), navigator.geolocation; no
// magnetic field.  Windows and Linux: everything is Unavailable (docs/ROADMAP.md names what a
// native version would use).  Scripts feed readings (`sensor ...`, `location ...`; lib/ui/auto.rs):
// test_sensor, test_sensor_none, test_location.

use rt::sync::SpinLock;

/// acceleration including gravity, m/s^2
pub const SENSOR_ACCEL: u32 = 1;
/// gravity alone, m/s^2 (a virtual sensor of the system)
pub const SENSOR_GRAVITY: u32 = 2;
/// angular speed around x, y, z, rad/s
pub const SENSOR_GYRO: u32 = 4;
/// magnetic field, microtesla
pub const SENSOR_MAGNET: u32 = 8;
/// the rotation estimate: a unit quaternion from the device's axes to east, north, up
pub const SENSOR_ROTATION: u32 = 16;
/// every sensor
pub const SENSOR_ALL: u32 = 31;

const COUNT: usize = 5;

/// what a sensor does now
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum SensorState {
    /// nobody asks for it
    Off = 0,
    /// started, no reading yet
    Waiting = 1,
    /// readings arrive
    Running = 2,
    /// the app is in the background (or the system sleeps): stopped until it is back
    Paused = 3,
    /// the manifest refuses it (`sensors`) or the user did (Safari's question)
    Denied = 4,
    /// this device has no such sensor, or the platform has none
    Unavailable = 6,
}

/// what the location does now
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum LocationState {
    /// nobody asks for it
    Off = 0,
    /// started (or asking the user), no fix yet
    Searching = 1,
    /// a fix arrived: location_fix
    Fix = 2,
    /// the app is in the background: stopped until it is back
    Paused = 3,
    /// the manifest (`location`) or the user refuses it, or the user took it back
    Denied = 4,
    /// the system's location switch is off
    Disabled = 5,
    /// no location service on this device or platform
    Unavailable = 6,
}

/// one reading of a sensor
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Reading {
    /// x, y, z (the rotation estimate: x, y, z, w of the quaternion; else 0)
    pub v: [f32; 4],
    /// how far to trust it: 0 unreliable (a magnetometer asking for the figure eight), 1 low,
    /// 2 medium, 3 high
    pub accuracy: u8,
    /// (rotation) its zero is north; false: only relative to where it started
    pub absolute: bool,
}

/// the device's position in the room, degrees
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attitude {
    /// -90 .. 90: the top edge above the bottom edge (upright: 90)
    pub pitch: f32,
    /// -180 .. 180: the right edge below the left edge
    pub roll: f32,
    /// -90 .. 90: the right edge above the left edge (a level's other axis beside pitch)
    pub side: f32,
    /// 0 .. 360 clockwise from north: where the device's top points, or its back when it is held
    /// upright; None: no rotation estimate and no magnetic field
    pub heading: Option<f32>,
    /// heading's zero is north (false: where the estimate started)
    pub absolute: bool,
    /// the direction away from the earth in the device's axes (a unit vector)
    pub up: [f32; 3],
}

/// where a fix comes from
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum FixSource {
    /// the host did not say
    Other = 0,
    /// satellites
    Gps = 1,
    /// mobile cells and wireless networks
    Network = 2,
}

/// one location fix
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fix {
    /// degrees, north positive
    pub latitude: f64,
    /// degrees, east positive
    pub longitude: f64,
    /// metres (68 % of the fixes lie this close), when the provider says
    pub accuracy: Option<f32>,
    /// metres above the WGS 84 ellipsoid, when the provider says
    pub altitude: Option<f64>,
    /// metres per second, when the provider says
    pub speed: Option<f32>,
    /// degrees clockwise from north, when the provider says
    pub bearing: Option<f32>,
    pub source: FixSource,
    age0_ms: u64,
    got_us: u64,
}

impl Fix {
    /// how old it is, ms (a fix of a script stays as old as the script says)
    pub fn age_ms(&self) -> u64 {
        if self.got_us == 0 {
            return self.age0_ms;
        }
        self.age0_ms + crate::task::now_us().saturating_sub(self.got_us) / 1000
    }
}

#[derive(Clone, Copy)]
struct Chan {
    state: SensorState,
    reading: Option<Reading>,
}

struct St {
    /// live guards per sensor
    watchers: [u32; COUNT],
    ch: [Chan; COUNT],
    /// what the host runs
    running: u32,
    suspended: bool,
    denied: bool,
    /// sensors with news since take()
    fresh: u32,
    /// (scripts) the sensors a test feed made exist
    scripted: u32,
    loc_watchers: u32,
    loc_running: bool,
    loc_state: LocationState,
    loc_fix: Option<Fix>,
    loc_denied: bool,
    loc_fresh: bool,
    /// (scripts) a test feed answers for the host
    loc_scripted: bool,
}

static ST: SpinLock<St> = SpinLock::new(St {
    watchers: [0; COUNT],
    ch: [Chan { state: SensorState::Off, reading: None }; COUNT],
    running: 0,
    suspended: false,
    denied: false,
    fresh: 0,
    scripted: 0,
    loc_watchers: 0,
    loc_running: false,
    loc_state: LocationState::Off,
    loc_fix: None,
    loc_denied: false,
    loc_fresh: false,
    loc_scripted: false,
});

fn index(bit: u32) -> Option<usize> {
    (bit.count_ones() == 1 && bit & SENSOR_ALL != 0).then(|| bit.trailing_zeros() as usize)
}

/// these sensors run while the guard lives; dropping it stops them
#[must_use = "the sensors stop when the guard is dropped"]
pub struct SensorWatch {
    mask: u32,
}

impl SensorWatch {
    /// the SENSOR_* bits this guard keeps running (0: refused by the manifest)
    pub fn mask(&self) -> u32 {
        self.mask
    }
}

impl Drop for SensorWatch {
    fn drop(&mut self) {
        if self.mask == 0 {
            return;
        }
        {
            let mut s = ST.lock();
            for k in 0..COUNT {
                if self.mask & (1 << k) != 0 {
                    s.watchers[k] = s.watchers[k].saturating_sub(1);
                }
            }
        }
        apply();
    }
}

/// start these sensors (SENSOR_* bits) until the guard is dropped; a sensor this device lacks is
/// not an error: its state() says Unavailable.  Without `sensors` in the manifest: nothing runs,
/// state() says Denied
pub fn watch(mask: u32) -> SensorWatch {
    let mask = mask & SENSOR_ALL;
    if mask == 0 {
        return SensorWatch { mask: 0 };
    }
    if !crate::policy::sensors_allowed() {
        let mut s = ST.lock();
        s.denied = true;
        s.fresh |= mask;
        drop(s);
        crate::wake();
        return SensorWatch { mask: 0 };
    }
    {
        let mut s = ST.lock();
        s.denied = false;
        for k in 0..COUNT {
            if mask & (1 << k) != 0 {
                s.watchers[k] += 1;
            }
        }
    }
    apply();
    SensorWatch { mask }
}

// the host runs what is wanted now (the lock is free for its answer: a host may report at once)
fn apply() {
    let (mask, scripted) = {
        let mut s = ST.lock();
        let suspended = s.suspended;
        let mut want = 0;
        for k in 0..COUNT {
            let bit = 1u32 << k;
            if s.watchers[k] == 0 {
                s.ch[k] = Chan { state: SensorState::Off, reading: None };
                continue;
            }
            want |= bit;
            let c = &mut s.ch[k];
            if suspended {
                c.state = SensorState::Paused;
            } else if matches!(c.state, SensorState::Off | SensorState::Paused) {
                c.state = SensorState::Waiting;
            }
        }
        let mask = if suspended { 0 } else { want };
        if mask == s.running {
            return;
        }
        s.fresh |= mask ^ s.running;
        s.running = mask;
        (mask, s.scripted)
    };
    if let Some(have) = native::sensors(mask) {
        let have = have | scripted;
        let mut s = ST.lock();
        for k in 0..COUNT {
            let bit = 1u32 << k;
            if mask & bit != 0 && have & bit == 0 {
                s.ch[k].state = SensorState::Unavailable;
                s.fresh |= bit;
            }
        }
    }
    crate::wake();
}

/// what this sensor (one SENSOR_* bit) does now
pub fn state(bit: u32) -> SensorState {
    let Some(k) = index(bit) else { return SensorState::Off };
    let s = ST.lock();
    if s.denied && s.watchers[k] == 0 {
        return SensorState::Denied;
    }
    s.ch[k].state
}

/// the last reading of this sensor (one SENSOR_* bit)
pub fn read(bit: u32) -> Option<Reading> {
    let k = index(bit)?;
    ST.lock().ch[k].reading
}

fn xyz(r: Option<Reading>) -> Option<[f32; 3]> {
    r.map(|r| [r.v[0], r.v[1], r.v[2]])
}

/// pitch, roll and heading from what runs now (see the module header)
pub fn attitude() -> Option<Attitude> {
    let (accel, gravity, rotation, magnet) = {
        let s = ST.lock();
        (s.ch[0].reading, s.ch[1].reading, s.ch[4].reading, s.ch[3].reading)
    };
    let g = xyz(gravity).or(if rotation.is_some() { None } else { xyz(accel) });
    attitude_of(g, rotation, xyz(magnet))
}

fn deg(rad: f64) -> f64 {
    rad * 180.0 / core::f64::consts::PI
}

// (atan2's degrees: no fmod here)
fn wrap360(mut d: f64) -> f32 {
    while d < 0.0 {
        d += 360.0;
    }
    while d >= 360.0 {
        d -= 360.0;
    }
    // what rounds to 360 is 0
    if d >= 359.9995 {
        d = 0.0;
    }
    d as f32
}

fn norm3(v: [f64; 3]) -> Option<[f64; 3]> {
    let n = rt::fx::sqrtd(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
    (n > 1e-6 && n.is_finite()).then(|| [v[0] / n, v[1] / n, v[2] / n])
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn to64(v: [f32; 3]) -> [f64; 3] {
    [v[0] as f64, v[1] as f64, v[2] as f64]
}

// the top edge's height, how far the right edge lies below, the right edge's height: from the up
// vector
fn tilt(u: [f64; 3]) -> (f32, f32, f32) {
    let flat = rt::fx::sqrtd(u[0] * u[0] + u[2] * u[2]);
    let across = rt::fx::sqrtd(u[1] * u[1] + u[2] * u[2]);
    (deg(rt::x87::atan2(u[1], flat)) as f32, deg(rt::x87::atan2(-u[0], u[2])) as f32, deg(rt::x87::atan2(u[0], across)) as f32)
}

// where the top points when it lies flat, its back when it stands upright; between them both
// point the same way.  (east, north) of those two axes -> degrees clockwise from north
fn heading_of(east: [f64; 2], north: [f64; 2]) -> Option<f32> {
    let (e, n) = (east[0] - east[1], north[0] - north[1]);
    (e * e + n * n > 1e-8).then(|| wrap360(deg(rt::x87::atan2(e, n))))
}

/// the attitude from a gravity vector (the accelerometer at rest: what points away from the
/// earth), a rotation quaternion and a magnetic field, each in the device's axes; pitch and roll
/// follow gravity when it is given, the rotation otherwise; heading follows the rotation, else
/// gravity and the field.  None: neither gravity nor rotation (or useless values)
pub fn attitude_of(gravity: Option<[f32; 3]>, rotation: Option<Reading>, magnet: Option<[f32; 3]>) -> Option<Attitude> {
    // rows of the matrix device -> (east, north, up)
    let m = rotation.and_then(|r| {
        let [x, y, z, w] = [r.v[0] as f64, r.v[1] as f64, r.v[2] as f64, r.v[3] as f64];
        let n = x * x + y * y + z * z + w * w;
        (n > 0.5 && n < 1.5).then(|| {
            let s = 2.0 / n;
            (
                [1.0 - s * (y * y + z * z), s * (x * y - z * w), s * (x * z + y * w)],
                [s * (x * y + z * w), 1.0 - s * (x * x + z * z), s * (y * z - x * w)],
                [s * (x * z - y * w), s * (y * z + x * w), 1.0 - s * (x * x + y * y)],
                r.absolute,
            )
        })
    });
    let up = match gravity.and_then(|g| norm3(to64(g))) {
        Some(u) => u,
        None => m?.2,
    };
    let (pitch, roll, side) = tilt(up);
    let (heading, absolute) = if let Some((e, n, _, abs)) = m.filter(|r| r.3 || magnet.is_none()) {
        (heading_of([e[1], e[2]], [n[1], n[2]]), abs)
    } else if let Some(field) = magnet {
        // east = field x up, north = up x east (as Android's getRotationMatrix)
        let east = norm3(cross(to64(field), up));
        let north = east.and_then(|e| norm3(cross(up, e)));
        match (east, north) {
            (Some(e), Some(n)) => (heading_of([e[1], e[2]], [n[1], n[2]]), true),
            _ => (None, false),
        }
    } else {
        (None, false)
    };
    Some(Attitude { pitch, roll, side, heading, absolute, up: [up[0] as f32, up[1] as f32, up[2] as f32] })
}

// ---------------------------------------------------------------- location

/// the location runs while the guard lives; dropping it stops it
#[must_use = "the location stops when the guard is dropped"]
pub struct LocationWatch {
    live: bool,
}

impl Drop for LocationWatch {
    fn drop(&mut self) {
        if !self.live {
            return;
        }
        {
            let mut s = ST.lock();
            s.loc_watchers = s.loc_watchers.saturating_sub(1);
        }
        apply_location(false);
    }
}

/// start the location until the guard is dropped; the first start may ask the user, so call it
/// from a user action.  Without `location` in the manifest: nothing runs, location_state() says
/// Denied (an `ask location` line: the UI's question, UIC_PERMISSION, then call it again)
pub fn location_watch() -> LocationWatch {
    if !crate::policy::location_allowed() {
        let mut s = ST.lock();
        s.loc_denied = true;
        s.loc_fresh = true;
        drop(s);
        crate::wake();
        return LocationWatch { live: false };
    }
    {
        let mut s = ST.lock();
        s.loc_denied = false;
        s.loc_watchers += 1;
    }
    apply_location(true);
    LocationWatch { live: true }
}

// (web_sensors.rs, Java) what the host is told: stop, start (may ask the user), start again after
// the background (asks nobody: a permission taken back shows as Denied)
pub(crate) const LOC_STOP: u32 = 0;
pub(crate) const LOC_START: u32 = 1;
pub(crate) const LOC_RESUME: u32 = 2;

fn apply_location(ask: bool) {
    let (op, scripted) = {
        let mut s = ST.lock();
        let run = s.loc_watchers > 0 && !s.suspended;
        if s.loc_watchers == 0 {
            s.loc_state = LocationState::Off;
            s.loc_fix = None;
        } else if s.suspended {
            s.loc_state = LocationState::Paused;
        } else if matches!(s.loc_state, LocationState::Off | LocationState::Paused) {
            s.loc_state = LocationState::Searching;
        }
        if run == s.loc_running {
            return;
        }
        s.loc_running = run;
        s.loc_fresh = true;
        (if !run { LOC_STOP } else if ask { LOC_START } else { LOC_RESUME }, s.loc_scripted)
    };
    let answer = native::location(op);
    if op != LOC_STOP && !scripted {
        if let Some(state) = answer {
            location_state_set(state);
        }
    }
    crate::wake();
}

/// what the location does now
pub fn location_state() -> LocationState {
    let s = ST.lock();
    if s.loc_denied && s.loc_watchers == 0 {
        return LocationState::Denied;
    }
    s.loc_state
}

/// the last fix (kept while Paused or Searching again; gone when stopped)
pub fn location_fix() -> Option<Fix> {
    ST.lock().loc_fix
}

fn location_state_set(state: u32) {
    let state = match state {
        1 => LocationState::Searching,
        2 => LocationState::Fix,
        4 => LocationState::Denied,
        5 => LocationState::Disabled,
        6 => LocationState::Unavailable,
        _ => return,
    };
    let mut s = ST.lock();
    if s.loc_running && s.loc_state != state {
        s.loc_state = state;
        s.loc_fresh = true;
    }
}

// ---------------------------------------------------------------- the host and the UI

/// (ui) what is new since the last call: the sensors with news (a reading or another state),
/// whether the location has news
pub fn take() -> (u32, bool) {
    let mut s = ST.lock();
    let r = (s.fresh, s.loc_fresh);
    s.fresh = 0;
    s.loc_fresh = false;
    r
}

/// (ui) the app went to the background or the system to sleep (true): everything stops, Paused;
/// back (false): it starts again, nobody is asked
pub fn suspend(on: bool) {
    {
        let mut s = ST.lock();
        if s.suspended == on {
            return;
        }
        s.suspended = on;
    }
    apply();
    apply_location(false);
}

// a reading of the host (a late one of a stopped sensor is dropped) -> taken
pub(crate) fn host_sensor(bit: u32, v: [f32; 4], accuracy: u8, absolute: bool) -> bool {
    let Some(k) = index(bit) else { return false };
    let mut s = ST.lock();
    if s.running & bit == 0 || v.iter().any(|x| !x.is_finite()) {
        return false;
    }
    s.ch[k] = Chan { state: SensorState::Running, reading: Some(Reading { v, accuracy, absolute }) };
    s.fresh |= bit;
    true
}

// the host says what these sensors do: Waiting, Denied or Unavailable (SensorState as a number)
pub(crate) fn host_sensor_state(mask: u32, state: u32) {
    let state = match state {
        1 => SensorState::Waiting,
        4 => SensorState::Denied,
        6 => SensorState::Unavailable,
        _ => return,
    };
    let mut s = ST.lock();
    for k in 0..COUNT {
        let bit = 1u32 << k;
        if mask & s.running & bit != 0 && s.ch[k].state != state {
            s.ch[k].state = state;
            if state != SensorState::Waiting {
                s.ch[k].reading = None; // what it said no longer counts
            }
            s.fresh |= bit;
        }
    }
}

// the host's location: a state (LocationState as a number) or, with coordinates, a fix (state Fix
// without them: the last fix counts again).  NaN: the provider does not say.  age_ms: how old the
// fix is now
#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(any(target_arch = "wasm32", target_os = "android")), allow(dead_code))]
pub(crate) fn host_location(state: u32, lat: f64, lon: f64, accuracy: f64, altitude: f64, speed: f64, bearing: f64, age_ms: f64, source: u32) {
    if state != LocationState::Fix as u32 || lat.is_nan() {
        location_state_set(state);
        return;
    }
    if !lat.is_finite() || !lon.is_finite() || !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return;
    }
    let f = |x: f64| x.is_finite().then_some(x as f32);
    let fix = Fix {
        latitude: lat,
        longitude: lon,
        accuracy: f(accuracy).filter(|a| *a >= 0.0),
        altitude: altitude.is_finite().then_some(altitude),
        speed: f(speed).filter(|a| *a >= 0.0),
        bearing: f(bearing).filter(|a| *a >= 0.0),
        source: match source {
            1 => FixSource::Gps,
            2 => FixSource::Network,
            _ => FixSource::Other,
        },
        age0_ms: if age_ms.is_finite() && age_ms > 0.0 { age_ms as u64 } else { 0 },
        got_us: crate::task::now_us().max(1),
    };
    let mut s = ST.lock();
    if s.loc_running {
        s.loc_fix = Some(fix);
        s.loc_state = LocationState::Fix;
        s.loc_fresh = true;
    }
}

/// (scripts: `sensor NAME ...`) this sensor exists now; its reading arrives if it runs (a stopped
/// sensor delivers nothing) -> it was taken
pub fn test_sensor(bit: u32, v: [f32; 4], accuracy: u8, absolute: bool) -> bool {
    {
        let mut s = ST.lock();
        s.scripted |= bit & SENSOR_ALL;
        // one the host called missing a moment ago
        if s.running & bit != 0 {
            if let Some(k) = index(bit) {
                if s.ch[k].state == SensorState::Unavailable {
                    s.ch[k].state = SensorState::Waiting;
                }
            }
        }
    }
    let taken = host_sensor(bit, v, accuracy, absolute);
    crate::wake();
    taken
}

/// (scripts: `sensor none NAME`) these sensors do not exist (any more)
pub fn test_sensor_none(mask: u32) {
    {
        let mut s = ST.lock();
        s.scripted &= !mask;
    }
    host_sensor_state(mask, SensorState::Unavailable as u32);
    crate::wake();
}

/// (scripts: `location ...`) the host's answer: a state (LocationState: Searching, Denied,
/// Disabled, Unavailable), or Fix with `fix` (a fix of a script keeps its age)
pub fn test_location(state: LocationState, fix: Option<(f64, f64, [f64; 4], u64)>) {
    {
        let mut s = ST.lock();
        s.loc_scripted = true;
        if s.loc_running {
            if let Some((lat, lon, m, age)) = fix {
                let f = |x: f64| x.is_finite().then_some(x as f32);
                s.loc_fix = Some(Fix {
                    latitude: lat,
                    longitude: lon,
                    accuracy: f(m[0]),
                    altitude: m[1].is_finite().then_some(m[1]),
                    speed: f(m[2]),
                    bearing: f(m[3]),
                    source: FixSource::Other,
                    age0_ms: age,
                    got_us: 0,
                });
            }
            s.loc_state = state;
            s.loc_fresh = true;
        }
    }
    crate::wake();
}

/// (scripts: `location age MS`) how old the script's fix is
pub fn test_location_age(age_ms: u64) {
    {
        let mut s = ST.lock();
        if let Some(f) = &mut s.loc_fix {
            f.age0_ms = age_ms;
            f.got_us = 0;
        }
        s.loc_fresh = true;
    }
    crate::wake();
}

#[cfg(any(target_arch = "wasm32", target_os = "android"))]
use crate::web::sensors_host as native;

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
mod native {
    // Windows and Linux: nothing yet (docs/ROADMAP.md)
    pub fn sensors(_mask: u32) -> Option<u32> {
        Some(0)
    }
    pub fn location(_op: u32) -> Option<u32> {
        Some(super::LocationState::Unavailable as u32)
    }
}
