// t_sensors -- the sensors and the location without hardware (lib/plat/sensors.rs)
//
//   plattest --test sensors         the attitude from gravity, a field and a rotation; a computer
//                                   says Unavailable; guards counted per sensor, started and stopped
//                                   only with them; the background; a test feed like the host's
//   plattest --test sensors-policy  a manifest without `location` and `sensors`: Denied, PERMDENY
//                                   (not in "all"); the rights parse, also with ask
//   plattest --test sensors-ask     ask lines: refused while nobody answers, running after a yes,
//                                   the decision recorded (not in "all")

use plat::sensors::{self as ps, LocationState, Reading, SensorState, SENSOR_ACCEL, SENSOR_ALL, SENSOR_GRAVITY, SENSOR_GYRO, SENSOR_MAGNET, SENSOR_ROTATION};

use crate::T;

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

fn rot(x: f32, y: f32, z: f32, w: f32, absolute: bool) -> Option<Reading> {
    Some(Reading { v: [x, y, z, w], accuracy: 3, absolute })
}

fn attitude(t: &mut T) {
    let flat = Some([0.0, 0.0, 9.81]);
    let a = ps::attitude_of(flat, None, None).unwrap();
    t.check(near(a.pitch, 0.0) && near(a.roll, 0.0) && near(a.side, 0.0) && a.heading.is_none(), "attitude: flat, no field: no heading");
    t.check(near(a.up[2], 1.0), "attitude: up is +z when flat");
    let a = ps::attitude_of(Some([0.0, 9.81, 0.0]), None, None).unwrap();
    t.check(near(a.pitch, 90.0), "attitude: upright: pitch 90");
    let a = ps::attitude_of(Some([-4.905, 0.0, 8.4957]), None, None).unwrap();
    t.check(near(a.roll, 30.0) && near(a.side, -30.0) && near(a.pitch, 0.0), "attitude: the right edge down: roll 30, side -30");
    t.check(ps::attitude_of(None, None, None).is_none(), "attitude: nothing known");
    t.check(ps::attitude_of(Some([0.0, 0.0, 0.0]), None, None).is_none(), "attitude: free fall gives nothing");

    // the field's horizontal part points north: heading from it
    for (field, want) in [([0.0, 20.0, -40.0], 0.0), ([20.0, 0.0, -40.0], 270.0), ([-20.0, 0.0, -40.0], 90.0), ([0.0, -20.0, -40.0], 180.0), ([14.0, 14.0, -40.0], 315.0)] {
        let a = ps::attitude_of(flat, None, Some(field)).unwrap();
        t.check(a.absolute && a.heading.is_some_and(|h| near(h, want)), "attitude: heading from gravity and the field");
    }
    // upright, the back to the north: still north
    let a = ps::attitude_of(Some([0.0, 9.81, 0.0]), None, Some([0.0, -40.0, -20.0])).unwrap();
    t.check(a.heading.is_some_and(|h| near(h, 0.0)), "attitude: upright: the back's direction");
    // a field along gravity says nothing
    t.check(ps::attitude_of(flat, None, Some([0.0, 0.0, -40.0])).unwrap().heading.is_none(), "attitude: a field along gravity: no heading");

    // the rotation: a quarter turn around z to the left: the top to the west
    let s = core::f32::consts::FRAC_1_SQRT_2;
    let a = ps::attitude_of(None, rot(0.0, 0.0, s, s, true), None).unwrap();
    t.check(a.absolute && near(a.pitch, 0.0) && a.heading.is_some_and(|h| near(h, 270.0)), "attitude: rotation alone: heading 270");
    // around x by 90 degrees: upright, the back to the north
    let a = ps::attitude_of(None, rot(s, 0.0, 0.0, s, true), None).unwrap();
    t.check(near(a.pitch, 90.0) && a.heading.is_some_and(|h| near(h, 0.0)), "attitude: rotation upright");
    // relative: says so; a field gives north instead
    let a = ps::attitude_of(flat, rot(0.0, 0.0, 0.0, 1.0, false), None).unwrap();
    t.check(!a.absolute && a.heading.is_some(), "attitude: a relative rotation is no north");
    let a = ps::attitude_of(flat, rot(0.0, 0.0, 0.0, 1.0, false), Some([20.0, 0.0, -40.0])).unwrap();
    t.check(a.absolute && a.heading.is_some_and(|h| near(h, 270.0)), "attitude: the field wins over a relative rotation");
    t.check(ps::attitude_of(None, rot(0.0, 0.0, 0.0, 0.0, true), None).is_none(), "attitude: a broken quaternion gives nothing");
}

fn lifecycle(t: &mut T) {
    let _ = ps::take();
    t.check(ps::state(SENSOR_ACCEL) == SensorState::Off, "sensors: off without a guard");
    t.check(ps::state(3) == SensorState::Off && ps::read(3).is_none(), "sensors: two bits are no sensor");
    // a reading without a guard goes nowhere
    t.check(!ps::test_sensor(SENSOR_GYRO, [1.0, 2.0, 3.0, 0.0], 3, false), "sensors: a stopped sensor delivers nothing");
    ps::test_sensor_none(SENSOR_ALL);
    {
        let a = ps::watch(SENSOR_ACCEL | SENSOR_GRAVITY);
        t.expect_i(a.mask() as i64, (SENSOR_ACCEL | SENSOR_GRAVITY) as i64, "sensors: the guard's bits");
        t.check(ps::state(SENSOR_ACCEL) == SensorState::Unavailable, "sensors: a computer has none");
        t.check(ps::state(SENSOR_MAGNET) == SensorState::Off, "sensors: not asked for: off");
        let (bits, _) = ps::take();
        t.check(bits & SENSOR_ACCEL != 0, "sensors: the state is news");
        // the feed: it exists from now on
        t.check(ps::test_sensor(SENSOR_ACCEL, [0.0, 0.0, 9.81, 0.0], 2, false), "sensors: a reading arrives");
        t.check(ps::state(SENSOR_ACCEL) == SensorState::Running, "sensors: running");
        t.check(ps::read(SENSOR_ACCEL) == Some(Reading { v: [0.0, 0.0, 9.81, 0.0], accuracy: 2, absolute: false }), "sensors: the reading");
        let (bits, _) = ps::take();
        t.expect_i(bits as i64, SENSOR_ACCEL as i64, "sensors: the news");
        t.expect_i(ps::take().0 as i64, 0, "sensors: taken once");
        t.check(!ps::test_sensor(SENSOR_ACCEL, [f32::NAN, 0.0, 0.0, 0.0], 3, false), "sensors: a broken value is dropped");
        // a second guard keeps it running
        {
            let b = ps::watch(SENSOR_ACCEL);
            t.check(ps::state(SENSOR_ACCEL) == SensorState::Running, "sensors: a second guard changes nothing");
            drop(b);
        }
        t.check(ps::state(SENSOR_ACCEL) == SensorState::Running, "sensors: counted per sensor");
        t.check(ps::attitude().is_some_and(|a| near(a.pitch, 0.0)), "sensors: the attitude of what runs");
        // the background
        ps::suspend(true);
        t.check(ps::state(SENSOR_ACCEL) == SensorState::Paused, "sensors: paused");
        t.check(ps::read(SENSOR_ACCEL).is_some(), "sensors: the last reading stays");
        t.check(!ps::test_sensor(SENSOR_ACCEL, [1.0, 1.0, 1.0, 0.0], 3, false), "sensors: nothing arrives while paused");
        ps::suspend(false);
        t.check(ps::state(SENSOR_ACCEL) == SensorState::Waiting, "sensors: it exists: waiting again");
        t.check(ps::state(SENSOR_GRAVITY) == SensorState::Unavailable, "sensors: still none of this");
        ps::test_sensor_none(SENSOR_ACCEL);
        t.check(ps::state(SENSOR_ACCEL) == SensorState::Unavailable && ps::read(SENSOR_ACCEL).is_none(), "sensors: gone, its reading too");
    }
    t.check(ps::state(SENSOR_ACCEL) == SensorState::Off && ps::read(SENSOR_ROTATION).is_none(), "sensors: the last guard stops them");
    t.check(ps::watch(0).mask() == 0, "sensors: no bits");

    // the location
    t.check(ps::location_state() == LocationState::Off && ps::location_fix().is_none(), "location: off");
    ps::test_location(LocationState::Fix, Some((1.0, 2.0, [f64::NAN; 4], 0)));
    t.check(ps::location_fix().is_none(), "location: a stopped location delivers nothing");
    let _ = ps::take();
    {
        let w = ps::location_watch();
        t.check(ps::location_state() == LocationState::Searching, "location: the test feed answers for the host");
        t.check(ps::take().1, "location: news");
        ps::test_location(LocationState::Fix, Some((48.1, -11.5, [5.0, 519.0, f64::NAN, 45.0], 0)));
        t.check(ps::location_state() == LocationState::Fix, "location: a fix");
        let f = ps::location_fix().unwrap();
        t.check(f.latitude == 48.1 && f.longitude == -11.5 && f.accuracy == Some(5.0) && f.altitude == Some(519.0) && f.speed.is_none() && f.bearing == Some(45.0), "location: its values");
        ps::test_location_age(7000);
        t.expect_i(ps::location_fix().unwrap().age_ms() as i64, 7000, "location: its age");
        ps::suspend(true);
        t.check(ps::location_state() == LocationState::Paused, "location: paused");
        ps::suspend(false);
        t.check(ps::location_state() == LocationState::Searching && ps::location_fix().is_some(), "location: back: searching, the last fix stays");
        ps::test_location(LocationState::Denied, None);
        t.check(ps::location_state() == LocationState::Denied, "location: refused");
        ps::test_location(LocationState::Disabled, None);
        t.check(ps::location_state() == LocationState::Disabled, "location: the system's switch");
        drop(w);
    }
    t.check(ps::location_state() == LocationState::Off && ps::location_fix().is_none(), "location: stopped with the guard");
}

pub fn t_sensors(t: &mut T) {
    attitude(t);
    lifecycle(t);
}

/// --test sensors-policy: a manifest without `location` and `sensors`
pub fn t_sensors_policy(t: &mut T) {
    t.check(plat::policy::install("fs.user\n").is_ok(), "policy: a manifest installed");
    let w = ps::watch(SENSOR_ALL);
    t.expect_i(w.mask() as i64, 0, "policy: no sensors: nothing runs");
    t.check(ps::state(SENSOR_ACCEL) == SensorState::Denied, "policy: Denied");
    let l = ps::location_watch();
    t.check(ps::location_state() == LocationState::Denied, "policy: no location: Denied");
    drop(w);
    drop(l);
    let d = plat::policy::denials();
    t.check(d.iter().any(|x| x.kind == "sensors") && d.iter().any(|x| x.kind == "location"), "policy: both refusals recorded");
    t.check(plat::policy::check("location\nsensors\nask location \"Wo bin ich?\"\nask sensors\n").is_ok(), "policy: the rights parse (also asked)");
    t.check(plat::policy::check("location x\n").is_err(), "policy: location takes no arguments");
    t.check(plat::policy::check("sensors x\n").is_err(), "policy: sensors takes no arguments");
}

/// --test sensors-ask: the same rights behind ask lines
pub fn t_sensors_ask(t: &mut T) {
    let Ok(key) = plat::policy::install("ask location \"Wo bin ich?\"\nask sensors\n") else {
        t.check(false, "ask: the manifest installs");
        return;
    };
    // nobody asks (no UI): refused
    let w = ps::watch(SENSOR_ACCEL);
    t.expect_i(w.mask() as i64, 0, "ask: nobody answers: nothing runs");
    t.check(ps::state(SENSOR_ACCEL) == SensorState::Denied, "ask: Denied");
    let l = ps::location_watch();
    t.check(ps::location_state() == LocationState::Denied, "ask: the location Denied");
    drop(w);
    drop(l);
    // a yes: both run (a computer has neither: Unavailable, no longer Denied)
    key.set_answer(Some(plat::policy::Answer::Once));
    let w = ps::watch(SENSOR_ACCEL);
    t.expect_i(w.mask() as i64, SENSOR_ACCEL as i64, "ask: allowed: the sensor starts");
    t.check(ps::state(SENSOR_ACCEL) == SensorState::Unavailable, "ask: allowed: no longer Denied");
    let l = ps::location_watch();
    t.check(ps::location_state() == LocationState::Unavailable, "ask: allowed: the location starts");
    drop(w);
    drop(l);
    let d = plat::policy::decisions();
    t.check(d.iter().any(|x| x.rule == "location") && d.iter().any(|x| x.rule == "sensors"), "ask: both decisions recorded");
    t.check(d.iter().any(|x| x.reason == "Wo bin ich?"), "ask: the reason kept");
}
