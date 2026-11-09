// web_sensors -- (sensors.rs) the sensors and the location in the browser, on Android and on iOS
//
// Browser (web/sensors.js): REQUEST_SENSORS, u32 SENSOR_* union (0: stop) and REQUEST_LOCATION,
// u32 LOC_STOP / LOC_START / LOC_RESUME; no completion.  The page answers with jfgui_sensor,
// jfgui_sensor_state and jfgui_location; while it has not, a sensor is Waiting and the location
// Searching.  Android (android/GallerySensors.java): CALL_SENSORS, u32 union -> u32 the bits this
// device has; CALL_LOCATION, u32 op -> u32 LocationState; it asks ACCESS_FINE_LOCATION /
// ACCESS_COARSE_LOCATION on LOC_START and answers later through the same exports.  iOS
// (apple_ios_motion.rs): Core Motion and Core Location in this process, the same answers:
//
//   jfgui_sensor(bit, x, y, z, w, flags) -> 1   a reading (flags: accuracy 0..3, 4 absolute)
//   jfgui_sensor_state(mask, state) -> 1        Waiting 1, Denied 4, Unavailable 6
//   jfgui_location(state, lat, lon, accuracy, altitude, speed, bearing, age_ms, source) -> 1
//                                               a state (LocationState as a number) or with
//                                               state 2 a fix; NaN: the provider does not say;
//                                               source 1 GPS, 2 network
//
// A reading is stored, plat::wake() brings E_WAKE at the next jfgui_input / jfgui_tick (Android:
// the host's nativeEvent(E_WAKE)), the UI's UIC_SENSOR / UIC_LOCATION follow.

/// (browser) the sensors that shall run (u32 SENSOR_* union; no completion)
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const REQUEST_SENSORS: u32 = 26;
/// (browser) the location (u32 LOC_*; no completion)
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const REQUEST_LOCATION: u32 = 27;
// (Android) the host calls of GallerySensors (android_host.rs: CALL_*)
#[cfg(target_os = "android")]
const CALL_SENSORS: u32 = 53;
#[cfg(target_os = "android")]
const CALL_LOCATION: u32 = 54;

/// the sensors of `mask` run, the others stop -> Some(the bits this device has), None: the host
/// answers later
pub(crate) fn sensors(mask: u32) -> Option<u32> {
    #[cfg(target_arch = "wasm32")]
    {
        crate::web::notice(REQUEST_SENSORS, &mask.to_le_bytes());
        None
    }
    #[cfg(target_os = "android")]
    {
        let have = crate::web::android_host::call(CALL_SENSORS, &mask.to_le_bytes());
        Some(have.and_then(|r| Some(u32::from_le_bytes(r.get(..4)?.try_into().ok()?))).unwrap_or(0))
    }
    #[cfg(target_os = "ios")]
    {
        Some(crate::apple::ios::motion::sensors(mask))
    }
}

/// tell the host LOC_* -> Some(LocationState as a number), None: it answers later
pub(crate) fn location(op: u32) -> Option<u32> {
    #[cfg(target_arch = "wasm32")]
    {
        crate::web::notice(REQUEST_LOCATION, &op.to_le_bytes());
        None
    }
    #[cfg(target_os = "android")]
    {
        let answer = crate::web::android_host::call(CALL_LOCATION, &op.to_le_bytes());
        Some(answer.and_then(|r| Some(u32::from_le_bytes(r.get(..4)?.try_into().ok()?))).unwrap_or(6))
    }
    #[cfg(target_os = "ios")]
    {
        Some(crate::apple::ios::motion::location(op))
    }
}

/// A reading of the host: bit SENSOR_*, the values, flags: accuracy 0..3, 4 absolute -> 1 taken
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_sensor(bit: u32, x: f32, y: f32, z: f32, w: f32, flags: u32) -> u32 {
    let taken = crate::sensors::host_sensor(bit, [x, y, z, w], (flags & 3) as u8, flags & 4 != 0);
    if taken {
        crate::web::wake();
    }
    taken as u32
}

/// What the host says of these sensors: 1 Waiting, 4 Denied, 6 Unavailable -> 1
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_sensor_state(mask: u32, state: u32) -> u32 {
    crate::sensors::host_sensor_state(mask, state);
    crate::web::wake();
    1
}

/// The host's location: a state (LocationState as a number) or with state 2 a fix; NaN: the
/// provider does not say; age_ms: how old the fix is now; source: 1 GPS, 2 network -> 1
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn jfgui_location(state: u32, lat: f64, lon: f64, accuracy: f64, altitude: f64, speed: f64, bearing: f64, age_ms: f64, source: u32) -> u32 {
    crate::sensors::host_location(state, lat, lon, accuracy, altitude, speed, bearing, age_ms, source);
    crate::web::wake();
    1
}
