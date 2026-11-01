// sensors -- the sensors and the location for the app and the test scripts (plat::sensors)
//
//   sensors_poll()                 (E_WAKE) App::command(UIC_SENSOR, SENSOR_* bits) for sensors
//                                  with a new reading or another state, App::command(UIC_LOCATION,
//                                  LocationState) for a new fix or another state
//   sc_sensor / sc_location        the script commands (auto.rs): `sensor ...`, `location ...`
//
// The app asks plat directly: plat::sensors::watch, read, state, attitude, location_watch,
// location_state, location_fix.  The lifecycle stops them (runtime.rs: E_POWER -> suspend).

use alloc::string::String;
use alloc::vec::Vec;

use plat::sensors::{self as ps, LocationState, SensorState, SENSOR_ACCEL, SENSOR_ALL, SENSOR_GRAVITY, SENSOR_GYRO, SENSOR_MAGNET, SENSOR_ROTATION};

use super::consts::*;
use super::{App, Ui};

fn sensor_bit(name: &str) -> Option<u32> {
    Some(match name {
        "accel" => SENSOR_ACCEL,
        "gravity" => SENSOR_GRAVITY,
        "gyro" => SENSOR_GYRO,
        "magnet" => SENSOR_MAGNET,
        "rot" => SENSOR_ROTATION,
        _ => return None,
    })
}

fn sensor_state_name(s: SensorState) -> &'static str {
    match s {
        SensorState::Off => "off",
        SensorState::Waiting => "waiting",
        SensorState::Running => "running",
        SensorState::Paused => "paused",
        SensorState::Denied => "denied",
        SensorState::Unavailable => "unavailable",
    }
}

fn location_state_name(s: LocationState) -> &'static str {
    match s {
        LocationState::Off => "off",
        LocationState::Searching => "searching",
        LocationState::Fix => "fix",
        LocationState::Paused => "paused",
        LocationState::Denied => "denied",
        LocationState::Disabled => "disabled",
        LocationState::Unavailable => "unavailable",
    }
}

impl Ui {
    /// E_WAKE: what the sensors and the location have new -> App::command
    pub(crate) fn sensors_poll(&mut self) {
        let (bits, location) = ps::take();
        if bits != 0 {
            self.cmd_run(UIC_SENSOR, bits);
        }
        if location {
            self.cmd_run(UIC_LOCATION, ps::location_state() as u32);
        }
    }

    // the host would wake the app now: the command, then a frame
    fn sc_sensor_wake(&mut self, app: &mut dyn App) {
        let ev = plat::Event { typ: plat::E_WAKE, ..plat::Event::default() };
        plat::evq_push(ev);
        self.process_events(app);
        self.run_passes(app);
    }

    // sensor accel|gravity|gyro|magnet X Y Z [ACCURACY] | rot X Y Z W [abs|rel] | none NAME|all |
    // expect NAME STATE
    pub(crate) fn sc_sensor(&mut self, app: &mut dyn App, a: &[u8]) {
        const USE: &[u8] = b"sensor accel|gravity|gyro|magnet X Y Z [ACCURACY], rot X Y Z W [abs|rel], none NAME|all or expect NAME STATE";
        let line = String::from_utf8_lossy(a).into_owned();
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.first().copied() {
            Some("expect") => {
                let (Some(bit), Some(want)) = (w.get(1).and_then(|n| sensor_bit(n)), w.get(2)) else {
                    self.fail(USE);
                    return;
                };
                let now = sensor_state_name(ps::state(bit));
                if now != *want {
                    let m = alloc::format!("sensor {}: {now}", w[1]);
                    self.fail(m.as_bytes());
                }
            }
            Some("none") => {
                let mask = match w.get(1).copied() {
                    Some("all") => SENSOR_ALL,
                    Some(n) => match sensor_bit(n) {
                        Some(b) => b,
                        None => {
                            self.fail(USE);
                            return;
                        }
                    },
                    None => {
                        self.fail(USE);
                        return;
                    }
                };
                ps::test_sensor_none(mask);
                self.sc_sensor_wake(app);
            }
            Some(name) => {
                let Some(bit) = sensor_bit(name) else {
                    self.fail(USE);
                    return;
                };
                let n = if bit == SENSOR_ROTATION { 4 } else { 3 };
                let mut v = [0f32; 4];
                for (k, x) in v.iter_mut().enumerate().take(n) {
                    let Some(f) = w.get(k + 1).and_then(|s| s.parse::<f32>().ok()) else {
                        self.fail(USE);
                        return;
                    };
                    *x = f;
                }
                let extra = w.get(n + 1).copied();
                let (accuracy, absolute) = if bit == SENSOR_ROTATION { (3, extra != Some("rel")) } else { (extra.and_then(|s| s.parse::<u8>().ok()).unwrap_or(3), false) };
                ps::test_sensor(bit, v, accuracy.min(3), absolute);
                self.sc_sensor_wake(app);
            }
            None => self.fail(USE),
        }
    }

    // location LAT LON [ACC ALT SPEED BEARING] | age MS | searching|denied|off|none | expect STATE
    pub(crate) fn sc_location(&mut self, app: &mut dyn App, a: &[u8]) {
        const USE: &[u8] = b"location LAT LON [ACC ALT SPEED BEARING], age MS, searching|denied|off|none or expect STATE";
        let line = String::from_utf8_lossy(a).into_owned();
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.first().copied() {
            Some("expect") => {
                let Some(want) = w.get(1) else {
                    self.fail(USE);
                    return;
                };
                let now = location_state_name(ps::location_state());
                if now != *want {
                    let m = alloc::format!("location: {now}");
                    self.fail(m.as_bytes());
                }
                return;
            }
            Some("age") => {
                let Some(ms) = w.get(1).and_then(|s| s.parse::<u64>().ok()) else {
                    self.fail(USE);
                    return;
                };
                ps::test_location_age(ms);
            }
            Some("searching") => ps::test_location(LocationState::Searching, None),
            Some("denied") => ps::test_location(LocationState::Denied, None),
            Some("off") => ps::test_location(LocationState::Disabled, None),
            Some("none") => ps::test_location(LocationState::Unavailable, None),
            Some(lat) => {
                let num = |s: Option<&&str>| s.and_then(|s| s.parse::<f64>().ok()).unwrap_or(f64::NAN);
                let (la, lo) = (lat.parse::<f64>().unwrap_or(f64::NAN), num(w.get(1)));
                if !(-90.0..=90.0).contains(&la) || !(-180.0..=180.0).contains(&lo) {
                    self.fail(USE);
                    return;
                }
                let m = [num(w.get(2)), num(w.get(3)), num(w.get(4)), num(w.get(5))];
                ps::test_location(LocationState::Fix, Some((la, lo, m, 0)));
            }
            None => {
                self.fail(USE);
                return;
            }
        }
        self.sc_sensor_wake(app);
    }
}
