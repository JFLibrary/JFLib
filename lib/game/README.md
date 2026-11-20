# jflib-game

The core of a game engine, part of [JFLib](https://jflib.org): a world of entities and components, a
fixed simulation rate, input actions, camera controllers and the netcode of a multiplayer game. It needs no GPU and no operating system
service beyond the input events an app hands it, so a game's server runs the same code headless. It
is plain Rust, without the standard library and without a line of `unsafe` code.

## How it is used

```rust
use jflib::game::{ActionKind, ActionMap, Bind, FirstPerson, Game, Input, Pose, Source, Step, Vec3, World};
use jflib::plat::{KC_S, KC_W};

// the world: entities with a generation, components in dense columns per type
struct Speed(f64);
fn walk(world: &mut World, _res: &mut (), step: &Step) {
    let (poses, speeds) = world.pair_mut::<Pose, Speed>();
    for (e, mut pose) in poses.iter_mut() {
        if let Some(s) = speeds.get(e) {
            pose.now.pos.x += s.0 * step.dt;
        }
    }
}
let mut game = Game::new(());
game.systems.add("walk", walk);                     // systems run in the order added
let runner = game.world.spawn_with((Pose::at(Vec3::ZERO), Speed(4.0)));

// every frame: the clock comes from outside, the ticks due run at 60 Hz
game.frame(now_ns);
let shown = game.world.get::<Pose>(runner).unwrap().interpolate(game.alpha());

// input: actions bound to where the keys sit, not to what they print
let mut map = ActionMap::new();
let forward = map.add("forward", ActionKind::Axis, &[Bind::key(KC_W), Bind::scaled(Source::Key(KC_S), -1.0)]);
let look_x = map.add("look_x", ActionKind::Motion, &[Bind::new(Source::MouseX)]);
let mut input = Input::new(map);
input.set_lock(ui.pointer_lock_state());            // its end is the game's pause
input.key(key_event.code, key_event.down);
input.motion(ui.pointer_motion().0, ui.pointer_motion().1);
if input.take_lock_request() { ui.pointer_lock(true); }   // the click that locks fires nothing
input.pad_frame(ui.gamepads(), dt);                  // game pads: the one used last drives
let mut camera = FirstPerson::new(Vec3::new(0.0, 1.7, 0.0));
camera.look(input.motion_of(look_x), 0.0);
```

## What is in it

- **World**: entities carry a generation, so the id of a despawned entity never reaches the entity
  that gets its slot next. Components live in dense arrays per type (sparse sets) with a version per
  change, so a consumer such as a renderer or a network snapshot sees what changed. Systems are plain
  functions run in a fixed order. No archetypes, no scheduler, no hash maps: iteration follows the
  sequence of operations, the same on every platform.
- **Fixed tick**: 60 Hz unless set, an exact accumulator over time that comes from outside (tests run
  on a virtual clock), a cap on catch-up steps, and the alpha a frame interpolates with between the
  last two states of every `Pose`.
- **Input actions**: buttons, axes and motion bound to keys by their place on the keyboard, mouse
  buttons, the wheel and the raw motion of the locked pointer; sensitivity, inversion and a factor
  per platform that evens out the different units of raw motion; the click that locks the pointer
  fires nothing, the end of the lock pauses. Game pads too: their buttons, sticks with a radial dead
  zone as analog axes, looking with a stick at a turn rate with a response curve; the pad used last
  drives a player's input, and a pad plays without the pointer lock (`Input::pad_frame` with
  `ui.gamepads()`). Bindings and settings go into a settings file.
- **Cameras**: first person, an orbit that follows a target and comes in front of walls (through
  [jflib-physics](https://crates.io/crates/jflib-physics)), and free flight, all in `f64`.
- **Netcode** (`game::net`, over [jflib-netchan](https://crates.io/crates/jflib-netchan): UDP and
  WebSocket): the server decides at a fixed rate; clients send their inputs with tick numbers, several
  in every packet, predict their own figure and reconcile it bit for bit with the server's state; the
  others are shown 100 ms behind the newest snapshot. Snapshots are deltas against what each client
  holds, quantised and cut by priority to fit one 1200-byte datagram; a join's full state that does not
  fit comes over a WebSocket. Hits are lag-compensated (the others rewound to what the shooter saw).
  Rooms by a code or a link, a name, no account; bots and a simulated network for tests.
- **Assets** (`game::assets`): files through JFLib's virtual file system, decoded in the background on
  threads (or within a budget per frame where there are none), behind handles with a state (loading,
  ready, failed), counted references and unloading; the kinds are types (glTF models and images are in
  the bridge crate).
- **Spatial audio** (`game::audio`): a listener (the camera), sounds on entities and at points, the
  volume by distance (near, far, a rolloff) and the pan by direction, over the shared mixer of
  JFLib's sound output.

The bridge that draws entities through JFLib's 3D renderer is the crate
[jflib-game-render3d](https://crates.io/crates/jflib-game-render3d): a server never links a renderer.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::game`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.1

This crate is new and not part of 0.2.1: it comes with a later release. The
[changelog](https://docs.rs/crate/jflib-game/latest/source/CHANGELOG.md) will say so.

## Documentation

- [JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html): how an app is written
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#game): every public
  item of `game`, also on [docs.rs](https://docs.rs/jflib-game)
- [GAMECORE.md](https://github.com/JFLibrary/JFLib/blob/main/docs/GAMECORE.md) (in German): the
  design and its reasons, the units of the raw motion per platform, the tests
- [NETCODE.md](https://github.com/JFLibrary/JFLib/blob/main/docs/NETCODE.md) (in German): the
  netcode's design, the measurements with 16 bots, the tests

## License

MIT or Apache-2.0, at your choice.
