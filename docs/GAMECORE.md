# GAMECORE -- der Spielkern

`lib/game` (`jflib-game`, in der Fassade `jflib::game`) ist der Kern der Spiel-Engine ([GAME.md](GAME.md), Stufe G7b,
Entscheidung E9): ein Weltmodell aus Entitäten und Komponenten, ein fester Takt, Eingabe-Aktionen und Kamera-Steuerungen. Die
Brücke zu [`render3d`](RENDER3D.md) ist ein eigenes kleines Crate, `lib/game_render3d` (`jflib-game-render3d`,
`jflib::game_render3d`). Beide ohne Betriebssystem (`#![no_std]`, `#![forbid(unsafe_code)]`, keine fremden Crates); `lib/game`
hängt an `rt`, `plat`, `base` und [`lib/physics`](PHYSICS.md), nicht an einer GPU: Der Server von Spiel 1 rechnet dieselben
Systeme ohne Fenster, unter Linux statisch ohne libc. Wie die Physik rechnet der Kern auf allen Zielen dieselben Bits.

Stand: G7b Teil 2. Teil 1 brachte Weltmodell, Takt, Eingabe, Kameras, Brücke, `apps/gametest` und die Demo `apps/gamedemo`;
seit G7a Teil 2 nimmt die Eingabe auch Gamepads ([Gamepads](#gamepads)). Teil 2 brachte die Messung der beiden Wege des
Spielfensters mit einem Vorschlag ([Spielfenster](#spielfenster-zwei-wege-gemessen)) und dabei die **Schleier** in `lib/ui`
(eine HUD über einer `gpu_view` wird nicht mehr in jedem Bild zurückgelesen), Assets über das VFS im Hintergrund mit Handles
([Assets](#assets)) und Raumklang über den Mischer von `plat::audio` ([Raumklang](#raumklang)).

| Teil | Wo |
| --- | --- |
| Entitäten, Welt, Bündel | [`world.rs`](../lib/game/src/world.rs) |
| Spalten (Sparse-Sets), Versionen | [`sparse.rs`](../lib/game/src/sparse.rs) |
| `Transform`, `Pose`, Interpolation | [`pose.rs`](../lib/game/src/pose.rs) |
| Takt, Systeme, `Game` | [`tick.rs`](../lib/game/src/tick.rs) |
| Eingabe-Aktionen, Maus- und Pad-Einstellungen, Pointer-Lock, Gamepads | [`input.rs`](../lib/game/src/input.rs) |
| Kameras | [`camera.rs`](../lib/game/src/camera.rs) |
| (Teil 2) Assets: Handles, Zustände, Laden im Hintergrund, Entladen | [`assets.rs`](../lib/game/src/assets.rs) |
| (Teil 2) Raumklang | [`audio.rs`](../lib/game/src/audio.rs) |
| Brücke zu render3d | [`lib/game_render3d/src/lib.rs`](../lib/game_render3d/src/lib.rs); (Teil 2) glTF-Modelle und Bilder als Assets, `ModelRef`, `sync_models`: [`models.rs`](../lib/game_render3d/src/models.rs) |
| Tests | [`apps/gametest`](../apps/gametest/README.md): [`world_t.rs`](../apps/gametest/src/world_t.rs), [`tick_t.rs`](../apps/gametest/src/tick_t.rs), [`input_t.rs`](../apps/gametest/src/input_t.rs), [`camera_t.rs`](../apps/gametest/src/camera_t.rs), [`bridge.rs`](../apps/gametest/src/bridge.rs), [`checksum.rs`](../apps/gametest/src/checksum.rs), (Teil 2) [`asset_t.rs`](../apps/gametest/src/asset_t.rs), [`audio_t.rs`](../apps/gametest/src/audio_t.rs), das Fenster der Skripte [`probe.rs`](../apps/gametest/src/probe.rs) |
| Demo | [`apps/gamedemo`](../apps/gamedemo/README.md); (Teil 2) die Messung des Spielfensters: [`bench.rs`](../apps/gamedemo/src/bench.rs) (Weg a), [`own.rs`](../apps/gamedemo/src/own.rs) (Weg b), [`probe.rs`](../apps/gamedemo/src/probe.rs) (Tastendrücke und Bildschirm) |

## Ein Beispiel

```rust
use jflib::game::{ActionKind, ActionMap, Bind, FirstPerson, Game, Input, Pose, Source, Step, Vec3, World};
use jflib::game_render3d::{Bridge, MeshRef};
use jflib::plat::{KC_S, KC_W};

struct Tempo(f64);                                       // eine Komponente: irgendein 'static-Typ
struct Res { input: Input, takt: jflib::game::TickInput, vor: jflib::game::ActionId }

fn gehen(welt: &mut World, r: &mut Res, s: &Step) {      // ein System: eine Funktion je Takt
    let (posen, tempi) = welt.pair_mut::<Pose, Tempo>();
    for (e, mut p) in posen.iter_mut() {
        if let Some(t) = tempi.get(e) {
            p.now.pos.z -= r.takt.value(r.vor) * t.0 * s.dt;
        }
    }
}

let mut karte = ActionMap::new();
let vor = karte.add("forward", ActionKind::Axis, &[Bind::key(KC_W), Bind::scaled(Source::Key(KC_S), -1.0)]);
let mut spiel = Game::new(Res { input: Input::new(karte), takt: Default::default(), vor });
spiel.systems.add("gehen", gehen);
let figur = spiel.world.spawn_with((Pose::at(Vec3::new(0.0, 0.0, 6.0)), Tempo(4.5), MeshRef { mesh, material }));

// je Bild der App (ui): Eingabe hinein, die fälligen Takte, die Szene
spiel.res.input.set_lock(ui.pointer_lock_state());       // sein Ende ist die Pause des Spiels
while let Some(k) = ui.key_event_take() { spiel.res.input.key(k.code, k.down); }
spiel.res.input.buttons(inp.pressed, inp.released);
let (dx, dy) = ui.pointer_motion();
spiel.res.input.motion(dx, dy);
if spiel.res.input.take_lock_request() { ui.pointer_lock(true); }   // der Klick dafür löst nichts aus
for _ in 0..spiel.clock.advance_to(plat::task::now_us() * 1000) {
    spiel.res.takt = spiel.res.input.tick();
    spiel.step();
}
spiel.res.input.end_frame();
bridge.sync(&spiel.world, &mut scene, spiel.alpha())?;   // nur Geändertes, bei alpha zwischen zwei Takten
```

## API-Überblick

- **Welt** (`World`): `spawn`, `spawn_with((a, b, ..))` (Tupel aus 1 bis 6 Komponenten, `Bundle`), `despawn` (nimmt alle
  Komponenten), `alive`, `len`, `entities()` (`Entities`: `iter` in Index-Reihenfolge, `at`, `slots`), `insert` (false für eine
  tote Entität), `remove`, `get`, `get_mut` (`Mut<T>`), `has`, `register`, `column::<T>()` (`Option<&SparseSet<T>>`),
  `column_mut::<T>()`, `pair_mut::<A, B>()` (zwei Spalten zum Schreiben), `columns()` (Name und Zahl je Spalte),
  `sort_columns()`, `begin_tick()`, `tick()`, `pose_mark()`. `Entity` (`index`, `generation`, `to_bits`, `from_bits`,
  `Entity::NONE`).
- **Spalte** (`SparseSet<T>`, auch allein nutzbar): `insert`, `remove`, `get`, `get_mut`, `contains`, `position`, `touch`,
  `clear`, `entities()`, `values()`, `versions()`, `iter()`, `iter_mut()`, `version()`, `changed_at`, `changed_since(v)`,
  `changed_mut_since(v)`, `removals()`, `sort()`, `index_bytes()`. `Mut<'a, T>`: `Deref`, `DerefMut` (stempelt), `untracked()`.
- **Lage** (`Transform { pos: Vec3, rot: Quat, scale: [f32; 3] }`, `IDENTITY`, `at`, `new`, `scaled`, `lerp`), `Pose { now,
  prev }` (`new`, `at`, `teleport`, `moving`, `interpolate(alpha)`), `nlerp`. `Vec3` und `Quat` sind die von `lib/physics`.
- **Takt**: `FixedTick` (`new(rate)`, `Default` 60 Hz, `with_max_steps`, `advance_to(now_ns)`, `advance_by(elapsed_ns)`,
  `alpha`, `dt`, `rate`, `max_steps`, `dropped_ns`, `reset`), `DEFAULT_RATE` (60), `DEFAULT_MAX_STEPS` (8), `Step { tick, dt }`,
  `System<R>` (`fn(&mut World, &mut R, &Step)`), `Systems<R>` (`add`, `add_before`, `remove`, `run`, `names`), `Game<R> { world,
  res, systems, clock }` (`new`, `frame(now_ns)`, `step`, `alpha`).
- **Eingabe** (`game::input`, die Typen auch an der Wurzel): `Source::{Key(KC_*), Mouse(b), WheelUp, WheelDown, MouseX, MouseY,
  Wheel, PadButton(plat::PAD_*), PadAxis(a), PadDir(a, positiv)}` (`#[non_exhaustive]`; `name`, `parse`, `label`, `label_for(PadStyle)`,
  `is_motion`, `is_pad`), `MOUSE_LEFT` bis `MOUSE_X2`, `Bind { source, scale }`
  (`new`, `scaled`, `key`, `text`, `parse`), `ActionKind::{Button, Axis, Motion}` (`takes`), `ActionId` (`index`), `ActionMap`
  (`add(name, kind, defaults)`, `find`, `name`, `kind`, `binds`, `defaults`, `bind`, `unbind`, `set_binds`, `reset`,
  `reset_all`, `users`, `ids`, `store`, `load`), `MouseSettings { sensitivity, invert_x, invert_y, factor }` (`DEFAULT`,
  `motion_factor`, `radians_per_unit`, `store`, `load`), `DEGREES_PER_COUNT` (0,022), `platform_motion_factor()`,
  `FACTOR_WINDOWS`, `FACTOR_LINUX`, `FACTOR_MACOS`, `FACTOR_BROWSER`, `FACTOR_OTHER`, `PadSettings { sensitivity, dead_zone, curve,
  invert_x, invert_y }` (`DEFAULT`, `stick`, `look`, `store`, `load`), `TURN_RATE` (180°/s), `PAD_DIR_PRESS` (0,5),
  `PAD_DIR_RELEASE` (0,4), `Device::{KeyboardMouse, Pad}`. `Input`: hinein `key`, `button`, `buttons`
  (die Masken von `UiInput`), `motion`, `wheel`, `set_lock`, `plat_event`, `release_all`, `pad_frame(pads, dt)`; je Bild `down`, `pressed`, `released`,
  `axis`, `motion_of`, `end_frame`; je Takt `tick() -> TickInput` (`get`, `down`, `pressed`, `released`, `value`, `states`;
  `ActionState { down, pressed, released, value }`); der Lock `playing`, `lock`, `take_lock_request`, `take_pause`, `pauses`,
  `set_require_lock`; das Pad `active_pad`, `pin_pad`, `pinned_pad`, `pad_playing`, `device`, `pad_held`, `pad_stick`,
  `pad_settings`, `set_pad_settings`; `start_capture`, `take_capture`, `capturing`; `edit_map`, `map`, `mouse`, `set_mouse`, `key_held`,
  `button_held`; `store`, `load`.
- **Kameras** (`game::camera`): `View { eye, forward, up }` (`look_at`, `right`), `FirstPerson { eye, yaw, pitch, pitch_limit }`
  (`look`, `forward`, `rotation`, `walk_dir`, `view`), `Follow { target, yaw, pitch, distance, min_distance, max_distance,
  pitch_min, pitch_max }` (`look`, `zoom`, `eye`, `view`, `view_in(&physics::World, &Filter, radius)`), `FreeFly { eye, yaw,
  pitch, pitch_limit, speed }` (`look`, `fly`, `view`), `basis`, `yaw_pitch_of`, `walk_dir`, `wrap_angle`, `PITCH_LIMIT` (89°).
- **Brücke** (`game_render3d`): `MeshRef { mesh, material }`, `Bridge` (`new`, `sync(&World, &mut Scene, alpha) -> SyncStats {
  added, removed, switched, moved, visited }`, `instance`, `len`, `mesh_users`, `material_users`, `reset`, `clear`,
  `retire_mesh`, `retire_material`, `collect(&mut Renderer)`, `retired`, `follow(&mut Scene, eye)`, `set_origin_step`,
  `origin_step`, (Teil 2) `sync_models(&World, &mut Scene, alpha, &Models)`, `model_entities`), `ORIGIN_STEP` (512 m),
  `camera(&View, fov_y, aspect, near, far)`, `transform(&Transform)`.
- **Assets** (Teil 2, `game::assets`): `Asset` (`decode(bytes, files) -> Result<Self, String>`), `Bytes`, `Assets<T>` (`new(Loading)`,
  `Default`, `load(path) -> Handle<T>`, `insert(name, value)`, `state`, `state_of`, `get`, `get_id`, `error`, `path`, `refs`,
  `refs_of`, `len`, `ids`, `stats -> AssetStats`, `reload`, `set_linger`, `update() -> Vec<Change>`, `wait_idle`, `mode`),
  `Loading::{Background, Frames { budget_us }}` (`default_for_target`), `State::{Loading, Ready, Failed}`, `Handle<T>` (`id`,
  Clone zählt), `AssetId { index, generation }`, `Change::{Ready, Failed, Unloaded}`, `sibling(path, rel)`. In
  `game_render3d`: `Model` (glTF), `Image` (PNG, JPEG; `IMAGE_PIXELS`), `ModelRef { model }`, `Models` (`update(&Assets<Model>,
  &[Change], &mut Renderer) -> ModelStats`, `parts`, `serial`, `made`, `version`, `reset`), `Part { mesh, material, local }`,
  `Textures` (`update`, `texture`, `reset`, `linear`, `nearest`).
- **Raumklang** (Teil 2, `game::audio`): `Attenuation { near, far, rolloff }` (`DEFAULT`, `linear`, `gain(d)`), `Rolloff::{Linear,
  Inverse(r)}`, `gains(&View, at, volume, &Attenuation) -> Gains { left, right, distance, pan, gain }`, `PAN_RADIUS`, `Emitter`
  (Komponente: `new(Sound)`, `with_volume`, `with_attenuation`, `looping`, `playing`, `finished`, `restart`, `stop`, `gains`),
  `SpatialAudio` (`new`, `set_listener(&View)`, `listener`, `master`, `update(&mut World, alpha, &mut dyn Mixer) -> AudioStats`,
  `play_at(sound, at, volume, att, mixer)`, `stop_all`), `Mixer` (`Speakers`, `plat::audio::MixTest`), `AudioStats`. In
  `plat::audio`: `Voice::set_gains(left, right)`, `play_gains(&Sound, left, right)`, `MixTest` (Tests).

Konventionen wie in `lib/physics` und `render3d`: Meter, rechtshändig, Y oben; Gier 0 und Nicken 0 blicken nach −Z, Gier dreht um
+Y (positiv nach links, `Quat::from_yaw_pitch`), Nicken um das gedrehte +X (positiv nach oben). Bewegung der Maus kommt nach
rechts und unten positiv an; `look(dx, dy)` dreht nach rechts und unten (Gier −= dx, Nicken −= dy).

## Entscheidungen

### Die Brücke als eigenes Crate

GAME.md verlangt, dass `lib/game` `render3d` nur in einem Brückenmodul kennt, damit ein Server ohne GPU läuft. Zwei Wege waren
offen: ein Cargo-Feature `render3d` in `lib/game`, das das Modul einschaltet, oder ein kleines eigenes Crate. **Gewählt ist das
eigene Crate**, aus drei Gründen:

- **Cargo vereinigt die Features eines Crates über alles, was ein `cargo build` baut.** `build_rs.py` baut eine App und ihre
  `BINS` in einem Lauf (unter Windows auch die statischen und die glibc-Programme gemeinsam), und Spiel 1 wird Client und Server
  im selben Workspace haben. Mit einem Feature bekäme der Server `lib/game` mit eingeschaltetem `render3d`, also `render3d`,
  `gpu` und deren Linker-Anweisungen (Metal-Frameworks auf Apple, die Vulkan-Teile) in sein Programm, auch wenn er die Brücke nie
  ruft. Ein Crate, das der Server nicht nennt, kommt nie in seinen Abhängigkeitsgraphen.
- **Die Bibliothek hat außer `std` und `unwind` keine Features**, und die Fassade lädt alles ohne Features
  ([CRATES.md](CRATES.md)). Ein eigenes Crate passt dazu, die Fassade nennt es einfach.
- **Die Grenze ist im Code sichtbar**: `lib/game` hat keine Zeile, die eine GPU kennt; was zeichnen will, nennt
  `game_render3d`.

Der Preis: ein Crate mehr auf crates.io und in der Fassade, und GAME.md spricht noch vom „Brückenmodul“ (Vorschlag für den Plan im
Bericht dieses Laufs).

### Weltmodell: Sparse-Sets mit Generationen

- **Entitäten** sind ein Index und eine Generation (je `u32`). `despawn` zählt die Generation des Platzes hoch und legt ihn auf
  eine Freiliste; `spawn` nimmt den zuletzt frei gewordenen (LIFO). Eine alte Id trifft deshalb nie die neue Entität desselben
  Platzes: `insert` gibt false, `get`, `get_mut`, `remove` finden nichts, `despawn` tut nichts. Ein Platz, dessen Generationen
  aufgebraucht sind (2³² − 1), wird nie wieder vergeben.
- **Komponenten** liegen je Typ in einer Spalte (`SparseSet<T>`): drei dichte Felder (Entität, Wert, Version) und ein
  dünner Index Entität → Platz, in Seiten zu 1024 Einträgen, die erst entstehen, wenn eine Entität der Seite einen Wert bekommt
  (eine Spalte mit einer Entität bei Index 999 999 kostet eine Seite, keine 4 MB). Einfügen hängt an, Entfernen setzt den letzten
  Eintrag in die Lücke: alles O(1). Der Index prüft die ganze Entität, eine fremde Generation findet nichts.
- **Kein Archetyp, kein Scheduler** (G7b: bis eine Messung sie verlangt). Ein System nimmt die Spalten, die es braucht; ein
  Verbund läuft über die kleinere Spalte und sucht in der anderen nach (`pair_mut` gibt zwei Spalten zum Schreiben). Die Welt
  findet eine Spalte über ihre `TypeId` in einer sortierten Liste; das ist nur ein Nachschlagen, iteriert wird dort nie.
- **Reihenfolge**: Entitäten iterieren nach Index, Werte in der Reihenfolge ihrer Spalte, und die folgt allein aus der Folge der
  Operationen (anhängen, Lücke mit dem letzten füllen), nie aus Adressen oder Hashes. Zwei Welten mit derselben Geschichte
  iterieren gleich, auf jedem Ziel. Eine Welt, die anders entstand (ein Client baut sie aus Schnappschüssen), bekommt mit
  `sort_columns` die kanonische Reihenfolge nach Index; der Test `canonical` zeigt es. Gegenweg wäre, die Spalten immer sortiert zu
  halten: Einfügen und Entfernen in der Mitte kosteten dann O(n).
- **Versionen**: Jede Spalte zählt ihre Änderungen; ein Einfügen, ein Schreiben durch `Mut` (nur `DerefMut`, Lesen nicht),
  `touch` und was `iter_mut` beschreibt, bekommen eine neue Version. Wer die Welt beobachtet (die Brücke, später die
  Schnappschüsse des Netzes in G7e), merkt sich die zuletzt gesehene Version und fragt `changed_since`; `removals()` zählt, was
  ging. Eine Version ist ein `u64`, ein Überlauf ist nicht zu erreichen.

### Takt und Interpolation

- **Exakt**: Der Akkumulator hält Nanosekunden mal Rate in einer 128-Bit-Zahl, ein Takt nimmt 10⁹ davon. 60 Hz verliert so
  nichts an den 16,666… ms eines Takts; nach einer Stunde in Schritten von 1 ms stimmt die Zahl der Takte genau (Test `tick`).
- **Kappe**: Höchstens 8 Takte je Aufruf (`with_max_steps`); was ein hängendes Bild darüber hinaus brächte, fällt weg
  (`dropped_ns`), der Rest eines Takts bleibt für das Alpha. `reset` vergisst die Uhr, damit eine Pause nicht nachgeholt wird.
- **Die Zeit kommt von außen**: `advance_to(now_ns)` mit `plat::task::now_us`, das in einem Lauf ohne Fenster der virtuellen
  Uhr des Skripts folgt. Tests geben eigene Zahlen. Eine Uhr, die zurückgeht, steht still.
- **Interpolation**: `Pose` hält `now` (was die Systeme schreiben) und `prev` (den Stand am Anfang des Takts). Ein Bild zeigt
  `prev.lerp(now, alpha)`: Positionen und Skalen gerade, Drehungen per `nlerp` auf dem kürzeren Weg. `World::begin_tick` setzt
  `prev = now` nur für die Posen, die im Takt davor geschrieben wurden (eine Version neuer als `pose_mark`): Eine Welt stehender
  Dinge kostet einen Durchlauf über ihre Versionen, kein Kopieren. `teleport` springt ohne Zwischenbilder.
- **Systeme** sind Funktionen `fn(&mut World, &mut R, &Step)` in der Reihenfolge, in der sie hinzukamen (`add_before` für eine
  andere). `R` ist alles, was keine Komponente ist (die Physik-Welt, die Eingabe des Takts, die Karte).

### Eingabe

- **Aktionen statt Tasten**: Ein Spiel fragt „vor“ oder „feuern“, nicht „W“. Drei Arten: `Button` (gedrückt, losgelassen,
  gehalten), `Axis` (die Skalen der gehaltenen Quellen, −1 bis 1: W +1, S −1) und `Motion` (rohe Bewegung, summiert).
- **Tasten nach Lage**: `Source::Key` hält die USB-HID-Usage (`KeyEvent::code`, `plat::KC_*`), nie das Zeichen. WASD liegt
  auf einer französischen Tastatur auf Z Q S D und bleibt dieselbe Bindung (Skript `input.uis`). In Menüs zeigt `label()` die
  Beschriftung der Belegung (`plat::key_label`: „Z“ für `KC_W` auf einer französischen Tastatur); wo das System nichts sagt
  (Browser, Telefone), der Name ohne „Key“/„Digit“. Mausquellen haben deutsche Texte, die eine App über ihren Katalog übersetzt.
- **Der Pointer-Lock** (Entscheidungen des Nutzers vom 18.11.2026): Solange das Spiel den Lock braucht (`require_lock`, Vorgabe)
  und nicht hat, löst keine Aktion aus. Ein Mausdruck fragt nach dem Lock (`take_lock_request`; die App ruft im selben Bild
  `ui.pointer_lock(true)`, der Browser gewährt ihn nur aus einem Druck heraus) und wird mitsamt seinem Loslassen verschluckt: Der
  Klick, der das Spiel beginnt, schießt nicht. Wartet der Lock (`Pending`, im Browser bis zum nächsten Druck), wird auch dieser
  Druck verschluckt. Esc gehört dem Lock und kommt beim Spiel nie an; das Spiel sieht nur, dass der Lock endet (`set_lock` mit
  `ui.pointer_lock_state()` oder `E_POINTER_LOCK` über `plat_event`), und nimmt das als Pause (`take_pause`): Alles Gehaltene wird
  losgelassen (die Aktionen melden es), Bewegung verfällt, auch bei Fokusverlust nichts klemmt. Tasten, die in der Pause gedrückt
  werden, zählen nach dem nächsten Klick als gehalten, ohne „gedrückt“ (wer beim Zurückklicken Feuer hält, schießt nicht). Ohne
  Lock auf dem Ziel (`Unsupported`, Telefone) spielt das Spiel ohne ihn; `set_require_lock(false)` für Spiele ohne Mausblick.
- **Bild und Takt**: Ereignisse kommen mit den Bildern, die Simulation läuft in Takten. Jede Aktion zählt Drücken, Loslassen und
  Bewegung zweimal: fürs Bild (`end_frame` leert, die Kamera dreht je Bild) und für den Takt (`tick()` nimmt). Ein Druck zwischen
  zwei Takten erreicht den nächsten, auch wenn das Bild keinen hatte; ein Tippen zwischen zwei Takten ergibt einen Takt mit
  gehalten, gedrückt und losgelassen. Eine Achse liest die Tasten, die im Moment der Frage gehalten sind.
- **Gamepads**: siehe [Gamepads](#gamepads).
- **Gespeichert** in einer Einstellungsdatei der Bibliothek: `Input::store(ini, "input")` und `load` schreiben in einen
  Abschnitt einer `base::Ini`, mit `base::Settings` in deren freien Teil (`settings.ini_mut()`, was auch `apps/prefstest`
  benutzt). Wie dort steht nur, was von der Vorgabe abweicht: `bind.NAME=KeyD -KeyA` (Name der Quelle wie `KeyboardEvent.code`,
  `-` für die Skala −1, `*0.5` für andere, `none` für keine; Pads `PadA`, `PadRT`, `PadLeftX`, `PadLeftY-`), `sensitivity`,
  `invert_x`, `invert_y`, `motion_factor`, für das Pad `pad_sensitivity`, `pad_dead_zone`, `pad_curve`, `pad_invert_x`,
  `pad_invert_y`. Eine neue Vorgabe erreicht so jeden, der sie nie geändert hat; Unlesbares wird ausgelassen und gezählt.

### Gamepads

G7a Teil 2. `plat` liefert die Pads in einer Belegung, der „standard“-Belegung der W3C-Gamepad-API ([GUIDE.md §39](GUIDE.md)): 17
Knöpfe mit analogen Werten (die Trigger `PAD_LT` / `PAD_RT`), zwei Sticks von −1 bis 1, rechts und unten positiv, ohne tote Zone,
dazu die Knöpfe, die seit dem letzten Lesen herunter- und hochgingen. `Input::pad_frame(ui.gamepads(), dt)` nimmt sie einmal je
Bild (ein Spiel mit eigenem Fenster: `plat::pads()`), `dt` ist die Zeit seit dem Bild davor in Sekunden.

- **Quellen**: `PadButton(b)` (gedrückt und losgelassen wie eine Taste; in einer Achse sein analoger Wert mal die Skala, ein Trigger
  als Gas), `PadAxis(a)` (die Achse eines Sticks nach der toten Zone: in einer `Axis` ihr Wert −1 bis 1, in einer `Motion` dreht
  sie), `PadDir(a, positiv)` (eine Richtung eines Sticks als Knopf: gedrückt ab 0,5, losgelassen unter 0,4, damit sie an der
  Schwelle nicht flattert). Ein `Button` nimmt Knöpfe und Richtungen, eine `Axis` alle drei, eine `Motion` die Achsen. Menüs zeigen
  `label_for(pad.style)`: Xbox A B X Y, LB RB LT RT, Ansicht, Menü (auch für Pads ohne bekannten Stil), PlayStation Kreuz, Kreis,
  Quadrat, Dreieck, L1 R1 L2 R2, Create, Options, Nintendo nach der Lage B A Y X, L R ZL ZR, Minus, Plus.
- **Tote Zone, radial**: Beide Achsen eines Sticks zusammen, nicht jede für sich (sonst rastet ein schräger Stick auf die Achsen
  ein): bis `dead_zone` (Vorgabe 0,15) vom Mittelpunkt ist der Stick in Ruhe, der Rest des Wegs wird auf 0 bis 1 gestreckt, in
  derselben Richtung; über 1 (die Ecken eines eckigen Sticks) bleibt er bei 1.
- **Blicken mit dem Stick**: eine `Motion` dreht um `TURN_RATE` (180°/s) × `sensitivity` bei vollem Ausschlag, der Ausschlag nach
  der toten Zone hoch `curve` (Vorgabe 2: langsam nahe der Mitte zum Zielen, schnell am Rand; 1 linear, bis 4), mal `dt` des Bilds
  (höchstens 0,25 s, ein hängendes Bild dreht nicht weit). Das Ergebnis ist in Radiant wie die Maus, beide zählen in derselben
  Aktion zusammen; `invert_x` / `invert_y` drehen das Vorzeichen. Laufen mit dem Stick bleibt linear.
- **Welches Pad**: Ein Pad treibt die Eingabe eines Spielers: das zuletzt benutzte. Ein Knopfdruck macht ein Pad zum aktiven (auch
  ein anderes als das bisherige; dessen gehaltene Knöpfe werden losgelassen, die gehaltenen des neuen zählen erst nach dem
  Loslassen), ein Stick außerhalb der toten Zone nur, wenn noch keins aktiv ist: Ein driftender Stick auf dem Tisch stiehlt nichts.
  Für geteilten Bildschirm hält `pin_pad(Some(i))` eine `Input` auf Pad i. `device()` sagt, was zuletzt benutzt wurde (Tastatur
  und Maus oder Pad), für die Hinweise im Spiel.
- **Spielen ohne Pointer-Lock** (Vorschlag dieses Laufs, siehe Fragen an den Nutzer): Ein Pad braucht keinen Lock. Drückt man einen
  Pad-Knopf, während das Spiel nicht spielt, spielt es mit dem Pad (`pad_playing`); dieser Druck löst wie der Klick des Locks
  nichts aus, sein Loslassen auch nicht. Die Maus fragt mit ihrem ersten Druck weiter nach dem Lock (verschluckt); kommt der Lock,
  endet das Spielen mit dem Pad, sein Ende pausiert wie bisher. Tasten wirken beim Spielen mit dem Pad (Esc kommt dann beim Spiel
  an, weil kein Lock es nimmt). Geht das aktive Pad beim Spielen mit dem Pad weg, ist das die Pause. Nach einer Pause zählt ein
  gehaltener Pad-Knopf erst wieder, wenn er losgelassen wurde.
- **Bild und Takt**: Die Knöpfe zählen Drücken und Loslassen wie Tasten; ein Tippen zwischen zwei Bildern (der Schnappschuss hat
  es in `down` und `up`, aber nicht in `pressed`) ist ein Drücken und ein Loslassen im selben Bild; die Drehung des Sticks geht
  in die Bewegung des Bilds und des Takts.
- **Fokus**: `plat` meldet nichts gedrückt, solange kein Fenster der App aktiv ist; die Aktionen lassen dann los.

### Einheiten der rohen Bewegung und die Faktoren je Plattform

`ui.pointer_motion` und `plat::win_raw_motion_take` liefern, was die Plattform hat (G7a Teil 1, [GUIDE.md §39](GUIDE.md)). Damit
eine Empfindlichkeit überall ähnlich dreht (Entscheidung vom 18.11.2026), rechnet `Input` jede Bewegung in **Zählwerte einer Maus
mit 1000 dpi** um, mal einem Faktor je Plattform (`platform_motion_factor`, überschreibbar mit `MouseSettings::factor`):

| Plattform | Einheit von `pointer_motion` | Faktor | Warum |
| --- | --- | ---: | --- |
| Windows | Zählwerte der Maus (Raw Input); absolute Geräte (Remote-Desktop, Tablett, manche VMs) in Pixeln | 1,0 | die Bezugseinheit |
| Linux, X11 | rohe Werte des Geräts (`XI_RawMotion`), absolute Geräte in Pixeln | 1,0 | dieselben Zählwerte |
| Linux, Wayland | unbeschleunigt (`dx_unaccel`), libinput normiert auf 1000 dpi | 1,0 | genau die Bezugseinheit |
| macOS | Punkte nach der Beschleunigung des Systems (`deltaX`) | 2,5 | geschätzt: macOS nimmt eine Maus als 400 dpi und bewegt den Zeiger bei Verstärkung 1 um 96 Punkte je Zoll, bei mittlerer Geschwindigkeit etwa 1,7-fach: rund 0,4 Punkte je Zählwert einer 1000-dpi-Maus |
| Browser | `movementX/Y`: roh mit `unadjustedMovement` (Chrome unter Windows und macOS), sonst CSS-Pixel nach der Beschleunigung | 1,0 | roh: Zählwerte; CSS-Pixel bei mittlerer Geschwindigkeit etwa einer je Zählwert |
| Android, iOS | kein Lock (`Unsupported`), keine Bewegung | 1,0 | – |

Die Faktoren außer macOS folgen aus den Einheiten; ob sich derselbe Wert auf allen Plattformen gleich anfühlt, prüft der Nutzer
von Hand (G7b Teil 2). Was ein Faktor nicht ausgleichen kann: die dpi der eigenen Maus unter Windows und X11 (eine 1600-dpi-Maus
dreht 1,6-mal so schnell wie unter Wayland, das sie auf 1000 normiert, wie in jedem Spiel unter Windows), die Beschleunigung unter
macOS und im Browser ohne `unadjustedMovement` (schnelle Bewegungen drehen weiter), ein `devicePixelRatio` ≠ 1 im Browser ohne
`unadjustedMovement`.

**Empfindlichkeit**: Der Wert einer Motion-Aktion ist Zählwerte × Faktor × `sensitivity` × 0,022° (`DEGREES_PER_COUNT`, das
`m_yaw` von Quake und Source) in Radiant; Empfindlichkeiten lesen sich also wie in diesen Spielen (1 000 Zählwerte bei 1,0 sind
22°). `invert_x` und `invert_y` drehen das Vorzeichen der Mausachsen. Das Mausrad als Motion zählt Rasten, ohne Empfindlichkeit.

### Kameras

`FirstPerson` (Auge, Gier, Nicken, Nicken bis ±89°), `Follow` (eine Umlaufbahn um ein Ziel im Abstand 0,5 bis 50 m, Zoom 10 %
je Raste, `view_in` holt das Auge vor eine Wand: eine Kugel von `radius`, vom Ziel zum Auge durch eine `physics::World`
geschoben) und `FreeFly` (fliegt, wohin es blickt, Geschwindigkeit in m/s). Alles in `f64`, Sinus und Kosinus aus
`physics::math` (die portablen von `rt`): dieselben Bits auf jedem Ziel, wie es die Simulation eines Servers braucht. `walk_dir`
gibt eine Richtung auf dem Boden aus zwei Achsen, höchstens 1 lang (schräg ist nicht schneller).

### Die Brücke zu render3d

- **Was gezeichnet wird**: eine Entität mit `MeshRef { mesh, material }`, an ihrer `Pose` (ohne Pose am Ursprung). Die Ids
  sind die des Renderers; nach einem verlorenen Gerät baut die App den Renderer neu (ein frischer Renderer zählt seine Meshes
  wieder 0, 1, 2, …), `reset` vergisst die Instanzen, der nächste `sync` legt alle neu an.
- **Nur Geändertes**: `sync` fragt die Versionen der Spalten `MeshRef` und `Pose`. Neue `MeshRef` werden `Scene::add`,
  geänderte `set_mesh` und `set_material`; zählt die Spalte `MeshRef` neue Entfernungen, geht die Brücke einmal über ihre eigenen
  Einträge und nimmt, was weg ist (`Scene::remove`). Posen, die seit dem letzten `sync` geschrieben wurden oder im letzten Takt
  sich bewegten (neuer als `pose_mark`, also zwischen zwei Ständen), werden bei `alpha` geschrieben, aber nur, wenn sich die
  Transformation gegenüber der zuletzt geschriebenen ändert. Bewegte sich im letzten Takt etwas, kostet eine stehende Pose je
  Bild einen Vergleich ihrer Version; hat sich keine Spalte geändert, gibt es keinen Durchlauf.
- **Der Ursprung folgt der Kamera**: `follow` setzt `Scene::set_origin` auf das Auge, gerundet auf 512 m (`ORIGIN_STEP`, eine
  Zweierpotenz hält die Positionen exakt), sobald das Auge auf einer Achse mehr als einen Schritt vom Ursprung entfernt ist.
  `f32` hat bei 1 km einen Schritt von 0,06 mm; ein neuer Ursprung schickt alle Instanzen einmal (render3d).
- **Entfernen**: Instanzen gehen mit `Scene::remove`. Meshes und Materialien, mit denen das Spiel fertig ist (eine Karte
  verlassen), gibt es mit `retire_mesh` / `retire_material` frei; `collect` ruft die neuen `Renderer::remove_mesh` und
  `remove_material` (G7f), sobald keine Instanz der Brücke sie mehr nutzt, nach dem `sync` und vor dem nächsten `prepare`. Was
  render3d verweigert (eine andere Szene nutzt es, ein Mesh nimmt es als Detailstufe), bleibt vorgemerkt.

### Die Demo

`apps/gamedemo`: ein Boden, Kisten im Ring, eine Mauer, Stufen, vier drehende Würfel über den Köpfen; `ui.gpu_view` mit
`AF_GPU_3D` über das Fenster, darunter eine Statuszeile. Ein Klick in die Ansicht fängt den Zeiger (und schießt nicht), WASD nach
Lage (Umschalt links rennt), die Maus schaut, Esc pausiert: Die Simulation steht, die Uhr holt nichts nach. Der Läufer ist eine
Kapsel, die ein Cast durch `lib/physics` an den Kisten stoppt und entlanggleiten lässt (der richtige Character-Controller ist
G7c Teil 2). Die Welt lebt im Inhalt der Ansicht; das Bild der App speist die Eingabe und lässt die Takte laufen, `draw` des
Inhalts gleicht die Welt bei `alpha` in die Szene ab. Weitere Bilder holt ein Timer (`ui.timer(1, 1)`, `UIC_TIMER` →
`ui.invalidate()`): ein `invalidate` in jedem `frame` hält die UI nach acht Bildern an (`need_after_passes`).

Seit Teil 2 liegt eine HUD über der Ansicht (Fadenkreuz, ein Balken mit Zahl, ein Feld mit Takt und Bildern je Sekunde, von
`lib/ui` gezeichnet: Schleier), vier Leuchtfeuer sind ein glTF-Modell (`assets/beacon.gltf`, `app://`, seine Textur ein PNG darin),
das `Assets` im Hintergrund lädt und die Entitäten mit seinem `ModelRef` zeigen, sobald es auf der GPU ist, und ein drehender
Würfel brummt (ein `Emitter` mit einem erzeugten Ton), zu hören über `SpatialAudio` von der Kamera aus, solange das Spiel läuft
(nicht in Skripten; `PERMISSIONS`: `audio.output`). `gamedemo --bench view|own` misst das Spielfenster (unten).

### Spielfenster: zwei Wege gemessen

GAME.md verlangt zwei Wege, gemessen mit Eingabe bis Bild und Bildabstand: **(a)** eine `gpu_view` über das ganze Fenster, die
UI für HUD und Menüs darüber (der Weg von Spiel 1 und der Demo), **(b)** eine eigene Swapchain mit der UI als Textur über dem Bild.
Gebaut in der Demo: `gamedemo --bench view` ist (a), `gamedemo --bench own` ist (b) (ein Fenster von `plat`, ein Gerät auf Stufe
Voll auf dem schnellsten Adapter, die Swapchain mit zwei Bildern in Arbeit, render3d direkt in ihr Bild, die HUD als Textur mit
vormultipliziertem Alpha in einem Durchgang darüber). Beide zeichnen dieselbe Welt mit einer HUD derselben Lage (Fadenkreuz,
Balken, Feld), die Kamera dreht sich gleichmäßig wie ein Blick mit der Maus.

**Was die HUD in Weg (a) kostete — und warum.** Spiel 1 fand: Mit einer HUD über der Ansicht wird im Fenster jedes Bild
zurückgelesen. Der Grund lag in `lib/ui` (`gpu_out.rs`, „Views“): Die Ausgabe zeichnet eine Ansicht nur in den Zellen, in denen sie
oben liegt; jede Zelle, in der etwas der Grundebene über ihr liegt (Text, ein durchscheinendes Feld), zeichnete die CPU über dem
Bild der Ansicht, und dafür las deren `Draw` jedes neue Bild zurück (die ganze Textur, gewartet: 1,3 ms bei 1200 × 700), dann
wurden die Zellen neu gezeichnet und hochgeladen, obwohl sich in ihnen nur die Ansicht geändert hatte. Vermeidbar: **Schleier**
([GPU.md, „G7b: Schleier“](GPU.md#3d-ansicht-g5e1)). Die CPU zeichnet, was über der Ansicht liegt, einmal über Schwarz (K) und
einmal über Weiß; die Ausgabe zeichnet die Zelle als K + T · Ansicht mit T = Weiß − Schwarz, das „over“ der CPU auf eine Stufe
genau. Ein neues Bild der Ansicht kostet diese Zellen nichts mehr; in Ruhe bekommt der Framebuffer einmal die wahren Pixel.
`uigputest --test viewplain`: 30 Bilder Drehung mit etwas darüber, 0 Rücklesungen statt einer je Bild, die Zellen einmal zweifach
gezeichnet statt je Bild, der Bildschirm gegen das Bild der CPU höchstens eine Stufe anders. Spiel 1 bekommt das ohne Änderung.

**Wie gemessen** (`apps/gamedemo/src/probe.rs`): ein Thread schickt F9 an das Fenster (`WM_KEYDOWN`/`WM_KEYUP`, wie Windows eine
Taste schickt; 25–45 ms Abstand, zufällig), ein zweiter liest den Bildschirm an einem Punkt des Himmels (`BitBlt` vom DC des
Bildschirms: was DWM zuletzt zusammengesetzt hat; ein Lesen dauert einen Bildwechsel, 6,06 ms, misst also auf etwa einen
Bildwechsel genau und nicht bis zum Licht). Der Himmel trägt im Rot die Zahl der genommenen Drücke (mod 8) und im Blau die Nummer
des Bildes (mod 16): „Eingabe bis Bild“ ist Druck → Bildschirm, dazu Druck → App nimmt ihn und Druck → ein Bild mit ihm übergeben;
„Bildabstand“ ist der Abstand zweier Bilder auf dem Bildschirm (ein Wechsel der Nummer), dazu Bilder, die gezeichnet und nie
gezeigt wurden, und die Zeit vom Zeichnen bis zum Bildschirm. Je Lauf ein Fenster, darin die Varianten nacheinander (150 Drücke,
3 s Bildabstand); Release-Build, Ryzen 5 3600 mit RTX 2070 SUPER (Treiber 610.88), Monitor 1080p mit 165 Hz, Windows 11, Fenster
1280 × 800 an derselben Stelle, ohne Aktivierung. **Der Rechner war geteilt und laut**: Last 11–88 % durch andere Sitzungen, und
alle paar Sekunden stand der Bildschirm für 0,5–0,8 s still (in jeder Variante gleich, auch ohne Drücke; das Lesen selbst dauerte
dabei nicht länger). Drücke, die erst nach 150 ms erschienen, sind deshalb als „im Stillstand“ gezählt und nicht in den Werten
(Runde 3; in Runde 2 noch mitgezählt, was dort die Mediane um weniger als einen Bildwechsel hebt); die Mediane sind über die Läufe
stabil, die p95 der Bildabstände nicht. Gezeigt: Mediane aus zwei Läufen (Runde 2 / Runde 3; im Vollbild
Runde 1 / Runde 3; Runde 1 lag im Fenster zum Teil unter fremden Fenstern).

| Weg, Present-Modus, Begrenzer | HUD | Druck → Bildschirm | Druck → Bild übergeben | Bildabstand p50 / p95 auf dem Bildschirm | gezeichnet, nie gezeigt |
| --- | --- | ---: | ---: | --- | ---: |
| (a) MAILBOX (Vorgabe der UI) | keine | 9,6 / 9,6 ms | 3,2 / 3,4 ms | 6,06 / 6,47 · 6,08 / 11,3 ms | 13–15 % |
| (a) MAILBOX | Schleier | 11,4 / 8,9 ms | 3,4 / 2,9 ms | 6,08 / 10,6 · 6,06 / 6,38 ms | 14–17 % |
| (a) MAILBOX | zurückgelesen (vorher) | 9,8 / 9,9 ms | 3,0 / 3,3 ms | 6,12 / 15,3 · 6,12 / 23,8 ms | 14–15 % |
| (a) FIFO (`--present fifo`) | Schleier | 37,9 / 37,2 ms | 3,6 / 3,9 ms | 6,06 / 6,29 · 6,07 / 7,20 ms | < 1 % |
| (a) FIFO | zurückgelesen | 24,0 / 23,8 ms | 3,0 / 3,3 ms | 6,11 / 18,6 · 6,07 / 7,60 ms | 1 % |
| (a) IMMEDIATE | Schleier | 10,1 / 10,3 ms | 3,3 / 3,6 ms | 6,06 / 6,38 · 6,06 / 6,36 ms | 14 % |
| (a) IMMEDIATE | zurückgelesen | 11,5 / 11,2 ms | 3,5 / 4,4 ms | 6,08 / 6,52 · 6,10 / 15,0 ms | 13–16 % |
| (a) MAILBOX, Begrenzer 160 Hz | Schleier | 7,9 / 8,0 ms | 3,1 / 3,5 ms | 6,09 / 12,2 · 6,18 / 17,1 ms | 11–18 % |
| (b) FIFO | HUD | 36,0 / 37,0 ms | 7,8 / 9,3 ms | 6,06 / 6,42 · 6,07 / 7,90 ms | < 1 % |
| (b) MAILBOX | HUD | 20,5 / 19,5 ms | 3,4 / 3,6 ms | 6,07 / 6,50 · 6,06 / 6,36 ms | 50 % |
| (b) IMMEDIATE | HUD | 6,4 / 5,9 ms | 1,1 / 1,3 ms | 6,07 / 6,39 · 6,05 / 6,43 ms | 84–87 % (über 1000 Bilder/s) |
| (b) FIFO, Begrenzer 160 Hz | HUD | 11,5 / 20,2 ms | 4,3 / 5,2 ms | 6,08 / 12,2 · 6,10 / 15,1 ms | 3–12 % |
| (b) MAILBOX, Begrenzer 160 Hz | HUD | 5,7 / 8,5 ms | 4,0 / 5,8 ms | 6,10 / 12,0 · 6,74 / 53,9 ms | 13–23 % |
| (a) MAILBOX, Vollbild 1920 × 1080 | Schleier | 9,4 / 10,3 ms | 3,3 / 3,9 ms | 6,06 / 6,40 · 6,07 / 6,52 ms | 13 % |
| (a) MAILBOX, Vollbild | zurückgelesen | 9,2 / 9,7 ms | 3,5 / 3,4 ms | 6,10 / 9,06 · 6,10 / 8,07 ms | 13–15 % |
| (b) FIFO, Vollbild | HUD | 39,7 / 40,4 ms | 11,0 / 9,1 ms | 6,08 / 9,19 · 6,06 / 6,34 ms | 0–7 % |
| (b) MAILBOX, Vollbild | HUD | 3,4 / 3,4 ms | 0,9 / 0,7 ms | 6,12 / 17,3 · 6,08 / 6,61 ms | 78–88 % (über 1500 Bilder/s) |

Ohne HUD sind die Werte von (b) wie mit (die HUD ist dort eine Textur, die nur bei einer Änderung hochgeht). Die App nimmt einen
Druck in Weg (a) nach 0,07–0,14 ms (die Fensterschleife wartet auf Nachrichten), in Weg (b) erst, wenn die Schleife wieder Ereignisse
holt (1–5 ms, hinter `acquire`, beim Begrenzer nach dem Warten); in (a) mit Rücklesen im p95 2–21 ms (der UI-Thread wartet auf die
GPU).

**Was das heißt:**

- **Die Schleier sind nötig, nicht die Latenz.** Mit dem Rücklesen bleibt der Median von Druck → Bildschirm fast gleich (9,8 gegen
  8,9–11,4 ms), aber die Bilder kommen unregelmäßig: p95 des Bildabstands 15–24 ms statt 6,4 ms (MAILBOX, zwei Runden), im
  Vollbild 9,1 statt 6,4 ms, und die App nimmt Eingaben im p95 bis 21 ms später. Mit FIFO „hilft“ das Rücklesen scheinbar
  (24 statt 37 ms), weil das Warten auf die GPU die Schlange der Swapchain kurz hält. Mit Schleiern ist (a) mit HUD so gut wie
  ohne HUD.
- **FIFO ohne Begrenzer ist für Spiele falsch**, auf beiden Wegen: 36–38 ms, die Bilder stauen sich (vom Zeichnen bis zum Bildschirm
  27–34 ms). Die UI nimmt MAILBOX als Vorgabe; dabei bleibt es.
- **Weg (a) mit MAILBOX** liefert rund 9–11 ms von der Taste bis zum Bildschirm bei 165 Hz, die Bilder im Takt des Bildschirms
  (p50 6,06 ms; p95 6,4 ms in den ruhigen Läufen), ohne dass die App etwas einstellt. Ein Begrenzer auf den Takt (160 Hz) bringt
  rund 1,5 ms weniger.
- **Weg (b) ist nur mit Feinarbeit schneller**: IMMEDIATE ohne Begrenzer 6 ms (zerreißt, mehr als 1000 Bilder je Sekunde, GPU und
  CPU voll), MAILBOX mit Begrenzer 6–8,5 ms im Fenster (genau diese Variante lief unter Last am unruhigsten), MAILBOX im Vollbild
  3,4 ms, aber mit über 1500 Bildern je Sekunde, von denen 80–90 % nie gezeigt werden; MAILBOX ohne Begrenzer im Fenster ist mit
  20 ms schlechter als (a). **Woher der Abstand kommt**: vom Druck bis zum übergebenen Bild braucht (a) 3–4 ms, (b) mit vielen
  Bildern unter 1 ms; vom Zeichnen bis zum Bildschirm sind beide gleich (je 5–6 ms im Vollbild). (a) zeichnet die Ansicht in
  einem Abstand von rund 5 ms: Die Ausgabe der UI stellt höchstens alle 4 ms dar (`MIN_PRESENT_US`), und ein neues Bild der
  Ansicht wartet, bis das letzte des Fensters dargestellt ist (G5e3). Ein Druck wartet also im Mittel 2,5–3 ms auf das nächste
  Bild. Das ist eine Stellschraube von `lib/ui`, kein Grund für einen zweiten Weg.

**Vorschlag (die Entscheidung trifft der Nutzer, GAME.md „Offen“): Die Spiele nehmen Weg (a)**, eine `gpu_view` über das ganze
Fenster mit HUD und Menüs von `lib/ui` darüber, mit MAILBOX. Gründe: (1) dieselbe Klasse der Latenz (rund 10 ms bei 165 Hz) und
Bilder im Takt des Bildschirms, seit den Schleiern auch mit HUD; (2) HUD, Menüs, Einstellungen, Text, Eingabe, Skalierung und
Barrierefreiheit kommen aus `lib/ui`; Weg (b) bräuchte für seine UI als Textur ein `lib/ui`, das in eine Textur mit Alpha zeichnet
(sein Framebuffer ist deckend, sein Text subpixel-geglättet), oder ein zweites UI-Werkzeug für Spiele (die HUD von (b) hier ist
eigens mit gfx gezeichnet, ohne Text); (3) ein Weg auf allen Zielen: der Browser (Canvas, `requestAnimationFrame`, die UI im
Worker), Android und iOS haben die Ansicht schon. Was an Weg (a) für die letzten Millisekunden zu stellen
wäre (nicht gebaut, nicht gemessen; Frage an den Nutzer): für Apps mit `AF_GPU_3D` eine kürzere `MIN_PRESENT_US` (heute höchstens
250 Bilder/s) und das Bild der Ansicht ohne Warten auf das letzte des Fensters (G5e3), das brächte die 2,5–3 ms bis zum nächsten
Bild auf unter 1 ms; dazu ein Begrenzer auf den Takt des Bildschirms in `lib/game` (gemessen: rund 1,5 ms weniger, und weniger
Strom). Weg (b) bleibt als Messwerkzeug in der Demo.

**Browser**: `apps/gamedemo/tests/bench_web.mjs` (Chrome 157 über Playwright, ohne Kopf und einmal im echten Fenster;
RTX 2070 SUPER über Dawn, Bündel `simd`): die Demo läuft (Ansicht, HUD mit Schleiern, das Leuchtfeuer über `Loading::Frames`
bereit, keine Fehler der Seite), danach die Schritte von Weg (a) je 3 s. Eine Seite kann ihren Bildschirm nicht lesen; gemessen ist
der Abstand der Bilder, die die App zeichnet. **Die Ansicht zeichnet im Browser nur rund 30 Bilder je Sekunde** (Abstand p50
31–34 ms, p95 37–47 ms, mit und ohne HUD, ohne Kopf wie im Fenster), wie schon G5e2 maß (GPU.md, „Der Takt kommt im Browser von
den Runden des Workers“). Die Ursache ist gefunden: Die Seite gibt dem Worker eine Runde je `requestAnimationFrame` nur, solange er
„beschäftigt“ meldet (`web/worker.js`: `jfgui_frame_delay() === 0`), sonst alle 33 ms (`web/loader.js`, `setInterval`); ein Spiel,
das sein nächstes Bild mit einem Timer von 1 ms holt (die Demo, Spiel 1), meldet 1 und bekommt 30 Bilder/s. Gegenprobe, nur im
gebauten Bündel (nicht eingecheckt): mit `<= 4` statt `=== 0` zeichnet die Ansicht im Takt des Bildschirms, Abstand 6,06 / 6,17 ms
(p50 / p95, 165 Hz), mit HUD ebenso. Das ist eine Zeile im Host des Browsers und eine Frage an den Nutzer (Vorschlag: so ändern;
Spiele im Browser brauchen 60 Bilder/s und mehr). Im Browser liest die Ansicht asynchron zurück (kein Warten), dort kosteten Schleier
und Rücklesen gleich viel Zeit; die Schleier zeigen aber in den Zellen unter der HUD das aktuelle Bild statt des zuletzt
zurückgekommenen.

**Nicht gemessen**: Iris Xe und HD 610 (Linux), der Mac (dort ist die GPU-Ausgabe der UI aus, `DEFAULT_ON_APPLE`: eine Ansicht wird
jedes Bild zurückgelesen, Schleier greifen erst mit `jfgui_GPU=1`), ein 60-Hz-Monitor, Linux (Wayland meldet über `wp_presentation`,
wann ein Bild auf dem Bildschirm war; der Weg dahin steht in `uigputest --bench latency`), Weg (b) unter Linux und im Browser.

### Assets

`game::assets` (Teil 2): Dateien über das VFS (`plat::read_file`: `app://`, `data://`, `mem://`, Pfade), im Hintergrund dekodiert,
hinter Handles.

- **Arten sind Typen**: `impl Asset for T` mit `decode(bytes, files)`; `files` liest Dateien neben der Datei über einen relativen
  Namen (die Puffer und Bilder einer `.gltf`; `sibling` verweigert `..`, absolute Pfade und Schemata). `lib/game` selbst hat nur
  `Bytes`; glTF-Modelle (`Model`, über `lib/gltf`) und Bilder (`Image`, PNG und JPEG über `img_png` und `img_jpeg`) liegen in
  `game_render3d`, weil `lib/game` keine Grafik kennt. Ein Server lädt mit derselben Maschinerie, was er braucht (`Bytes`).
- **Ein Speicher je Art**: `Assets<T>`. `load(path)` gibt für denselben Pfad dieselbe Eintragung (ein weiterer Handle), sonst eine
  neue im Zustand `Loading`. `update()` einmal je Bild: fertige Dekodierungen werden `Ready` oder `Failed`, Eintragungen ohne Handle
  werden entladen (der Wert geht, der Pfad wird vergessen, die `AssetId` findet nichts mehr); es gibt zurück, was geschah
  (`Change::Ready`, `Failed`, `Unloaded`), für alle, die je Asset etwas halten (die Kopien auf der GPU).
- **Im Hintergrund**: `Loading::Background` nimmt den Blocking-Pool von `plat::task` (`spawn_blocking`: Threads, wo es welche gibt;
  wo keine starten können, führt der Executor die Aufträge zwischen den Bildern aus, einen je Lauf, `lib/ui` tut das).
  `Loading::Frames { budget_us }` dekodiert in `update()`, eins nach dem anderen, bis das Budget verbraucht ist, mindestens eins:
  der Browser ohne Threads (`default_for_target` nimmt es dort) und Tests, die auf jedem Ziel dieselben Bilder brauchen (die
  Skripte der Demo). Ein Handle, der weggeht, solange sein Asset lädt, bricht den Auftrag ab (er startet nicht mehr) bzw. lässt ihn
  im Budget aus.
- **Referenzen**: Ein `Handle<T>` ist ein `Rc`; Clone zählt einen mehr, Drop einen weniger, `refs` sagt die Zahl. Ohne Handle wird
  ein Asset beim nächsten `update` entladen, mit `set_linger(n)` erst nach n weiteren; ein `load` desselben Pfads davor nimmt es
  zurück, ohne es neu zu lesen. `insert(name, value)` legt ein Asset aus Code ab (sofort `Ready`), `reload` liest neu (der alte Wert
  bleibt bis zum neuen).
- **Auf der GPU** (`game_render3d`): `Models::update(&assets, &changes, &mut renderer)` legt, was `Ready` wurde, auf den Renderer
  (Meshes, Texturen, Materialien wie `render3d::gltf::upload`, aber mit den Ids der Texturen, damit sie wieder gehen können) und
  nimmt herunter, was entladen wurde (`remove_mesh`, `remove_material`, `remove_texture`); der erste Aufruf auf einem Renderer (und
  der erste nach `reset`, einem verlorenen Gerät) legt alles hin, was `Ready` ist. `Textures` tut dasselbe für `Image`.
- **Entitäten bekommen ihre Meshes, sobald sie bereit sind**: Die Komponente `ModelRef { model: Handle<Model> }`; `Bridge::sync_models`
  gibt einer Entität je Teil der Szene des Modells eine Instanz, an ihrer `Pose` mit dem Ort des Teils verknüpft (Verschiebung,
  Drehung, Skala je Achse: genau für gleichmäßige Skalen). Solange das Modell lädt oder fehlschlug, zeigt sie nichts. Nur Geändertes
  wird geschrieben wie bei `sync`; ein neues Hochladen (`Models::serial`) baut ihre Instanzen neu.
- **Was nicht**: Lichter und Kameras eines Modells (dafür `render3d::gltf::upload` für eine ganze Szene), Detailstufen, Streaming
  (G7j), ein Cache auf der Platte, das Freigeben der CPU-Daten eines Modells, das auf der GPU ist (nach einem verlorenen Gerät wird
  es von dort neu hochgeladen).

### Raumklang

`game::audio` (Teil 2) über den gemeinsamen Mischer von `plat::audio`:

- **Hörer** ist die Kamera (`set_listener(&View)`); **Quellen** sind `Emitter`-Komponenten (an der `Pose` der Entität, interpoliert
  bei `alpha`) und Klänge an einem Punkt (`play_at`: ein Schuss, ein Treffer).
- **Lautstärke nach Entfernung** (`Attenuation { near, far, rolloff }`): bis `near` 1; `Rolloff::Linear` fällt bis `far` auf 0,
  `Rolloff::Inverse(r)` ist OpenALs inverse Entfernung mit Klemme: near / (near + r (d − near)), bis `far`, danach bleibt sie. Die
  Vorgabe ist near 1 m, far 50 m, invers mit 1 (die Hälfte bei 2 m, ein Fünfzigstel bei 50 m und weiter).
- **Panorama nach Richtung**: der Sinus des Winkels zur Rechten des Hörers, gleiche Leistung (links cos, rechts sin von
  (pan + 1) π/4: in der Mitte je −3 dB); vorn und hinten klingen gleich (Stereo). Näher als `PAN_RADIUS` (0,5 m) wandert eine Quelle
  zur Mitte, damit sie nicht durch den Kopf von links nach rechts springt.
- **Kein Doppler**: Der Mischer spielt eine Stimme mit ihrer Rate; eine Tonhöhe je Stimme hieße seinen Resampler steuern.
- **Der Mischer**: `plat::audio` hatte je Stimme nur eine Lautstärke. Neu sind `Voice::set_gains(left, right)` und `play_gains(&Sound,
  left, right)` (eine ferne Quelle beginnt leise, nicht einen Puffer lang in voller Lautstärke) und `MixTest`, derselbe Mischer
  ohne Gerät, Puffer für Puffer. Die Verstärkungen wechseln mit dem nächsten Puffer des Mischers (rund 20 ms), ohne Rampe.
  `Mixer` ist, wohin Stimmen gehen: `Speakers` (`play_gains`) oder `MixTest`.
- **Schleifen** starten neu, wenn der Mischer das Ende meldet (eine Lücke von einem Puffer); eine Entität, die geht, nimmt ihren
  Klang mit (`Drop` hält die Stimme an).
- **Dieselben Bits** auf jedem Ziel: nur `+ − × ÷` und die Mathematik von `lib/physics` (Wurzel, Sinus, Kosinus von `rt`).

## Tests

`python build_rs.py test gametest` führt `apps/gametest/tests/*.uis` aus: `check.uis` (`--check`, die fünf Gruppen in der
Konsole), `input.uis` und `tick.uis` (das Fenster `probe.rs` ohne Fenster: der echte Weg der Eingabe durch `ui`). Die Referenzen
teilen keinen Code mit den Crates: die Welt gegen ein Modell aus einfachen Feldern (welche Entität mit welcher Generation lebt,
jede Spalte mit ihrer Reihenfolge nach der Regel anhängen / Lücke mit dem letzten füllen, ihren Versionen), der Takt gegen
⌊t · Rate / 10⁹⌋ in Ganzzahlen, die Eingabe Fall für Fall gegen die Regeln oben, die Kameras gegen `Quat::from_yaw_pitch` und
geschlossene Formen, die Brücke gegen eine render3d-Szene, die für jedes Bild neu aus der Welt gebaut wird (ohne Versionen, ohne
Abgleich).

| Gruppe | Prüft | Ergebnis |
| --- | --- | --- |
| `world` | Entitäten: die ersten Ids, LIFO der freien Plätze, die Generation zählt je Platz, eine alte Id wird überall verweigert (`insert`, `get`, `get_mut`, `remove`, `despawn`) und trifft nie die neue Entität, `to_bits`/`from_bits`, `Entity::NONE`, ein Platz 100 000-mal neu vergeben, `despawn` leert jede Spalte, `spawn_with`. Spalten: anhängen, Lücke mit dem letzten füllen, Versionen (Lesen durch `Mut` stempelt nicht, Schreiben schon, `iter_mut` stempelt nur Beschriebenes, `untracked`, `touch`, `clear`, `removals`), `pair_mut` in beiden Reihenfolgen, der Index in Seiten (eine Entität bei Index 999 999: unter 32 KB), `sort`. 200 000 Zufallsschritte (`spawn`, `despawn`, `insert` zweier Typen, `remove`, Schreiben durch `get_mut`, Nachschlagen; sieben von zehn auf einer der 256 neuesten Ids, sonst irgendeiner je vergebenen, oft toten) gegen das Modell: jede Antwort, alle 997 Schritte beide Spalten ganz (Entitäten, Werte, Versionen, Version, Entfernungen) und die lebenden Entitäten; dieselben Schritte in einer zweiten Welt iterieren gleich. 1 000 000 Entitäten mit `Pose`, ein Viertel mit einer zweiten Komponente: Summe, Verbund, ein Drittel weg, neu (Generation 1, die alten Ids bleiben tot). Zwei Geschichten, nach `sort_columns` dieselbe Reihenfolge | 649 Prüfungen, 0 falsch; am Ende 19 508 lebend auf 19 512 Plätzen, 8190 und 6406 Werte |
| `tick` | 625 Bilder zu 16 ms: 600 Takte, das Alpha nach jedem Bild der exakte Bruch; je 100 000 zufällige Bilder von 0 bis 50 ms bei 1, 24, 30, 60, 120 und 144 Hz: die Summe der Takte ⌊t · Rate / 10⁹⌋ und das Alpha exakt nach jedem Bild; eine Stunde in Schritten von 1 ms bei 60 und 10 000 Hz; die Kappe (1,008 s: 8 Takte, 866 666 666 ns verworfen, Alpha 0,48, danach weiter), der erste Aufruf, eine Uhr rückwärts, `reset`; Systeme in ihrer Reihenfolge (`add_before`, `remove`), `Game::frame`; 10 000 Posen, 10 davon bewegt: Interpolation in Achteln exakt, nur die 10 neuer als `pose_mark`, nach einem Takt ohne Bewegung `prev = now`, `teleport`, die Enden von `lerp`, `nlerp` halb zu 90° ist 45° (10⁻¹⁵), der kurze Weg für −q | 33 Prüfungen, 0 falsch |
| `input` | (G7a Teil 2, 84 Prüfungen dazu) Pad-Quellen: Namen hin und zurück, Bindungstexte, Beschriftungen je Stil, welche Art welche Quelle nimmt; die radiale tote Zone (innen Ruhe, beide Achsen zusammen, 0,575 → 0,5, Länge 1 an der Ecke, NaN), das Blicken (voller Ausschlag `TURN_RATE`, Kurve 1, 2, 3, Empfindlichkeit, Invertieren); Spielen mit dem Pad ohne Lock (der Druck, der beginnt, und sein Loslassen lösen nichts aus), Tippen zwischen zwei Bildern, hoch und wieder herunter, der Klick der Maus fragt beim Spielen mit dem Pad nach dem Lock, der Lock beendet es ohne Pause, ein gehaltener Knopf nach der Pause wartet aufs Loslassen, Tasten beim Spielen mit dem Pad, das Pad weg ist die Pause, Abfangen von Knopf und Stickrichtung; analoge Achsen mit Tasten zusammen, Trigger als Gas unter und über der Schwelle, Stickrichtung mit Hysterese, Blicken über zwei Bilder und im Takt, Maus und Stick in einer Aktion, `dt` begrenzt und NaN, ohne Spielen nichts; welches Pad (gedrückt, driftender Stick stiehlt nicht, Knopf übernimmt, zurück, angeheftet, weg); Pad-Einstellungen in der `Ini` (nur Abweichungen, durch den Text zurück, Grenzen, neben Bindungen und Maus). Dazu wie bisher: Namen aller Tasten aus plats Tabelle hin und zurück, Maus, Bewegung, unbekannte Namen; Bindungstexte mit Skalen; die Karte (was verweigert wird, `users`, `reset`); der Abschnitt einer `Ini` (nur Abweichungen, `none`, Unlesbares gezählt, durch den Text der Datei und zurück), Maus-Einstellungen (Grenzen, unlesbarer Faktor), dasselbe durch `base::Settings` in `mem://` und zurück; der Lock: Tasten vorher treiben nichts, der Klick, der fragt, löst nichts aus und sein Loslassen auch nicht, gehaltenes W zählt danach ohne „gedrückt“, ein Klick im Lock feuert, die Bewegung ist Zählwerte × Faktor × 0,022°, das Ende des Locks pausiert einmal und lässt alles los, in der Pause keine Bewegung und keine Aktion, `Pending` verschluckt weiter, `Refused`, `Unsupported` spielt ohne Lock, `set_require_lock(false)`; Bild gegen Takt (ein Tippen ergibt einen Takt mit gehalten, gedrückt und losgelassen, gehalten über drei Takte, Wiederholung ändert nichts, Bewegung zweier Bilder in einem Takt), Achsen (Gegenrichtungen, Skala 0,5, Grenze 1), eine Bindung, die im Halten geht, Empfindlichkeit 2 und Invertieren; Mausrad (eine Raste, zwei halbe, nach unten), die Masken von `UiInput`, das Abfangen für eine Einstellungsseite; `plat_event` (Tasten über den Scancode dieser Plattform, Knöpfe, Rad, Lock 1 und 0, Fokusverlust); der Faktor dieser Plattform, ein eigener, 1 000 Zählwerte = 22° | 190 Prüfungen, 0 falsch |
| `camera` | 20 000 zufällige Gier- und Nickwinkel: die Basis gegen `Quat::from_yaw_pitch` (höchstens 10⁻¹⁵), orthonormal, rechtshändig, die Winkel zurück; Grenzen, `wrap_angle`, `look_at`; `FirstPerson` (rechts dreht nach +X, unten blickt nach unten, ±89°, NaN wird übergangen), `walk_dir`; `Follow` über 2000 Drehungen (Abstand auf 10⁻¹², blickt aufs Ziel), Zoom und seine Grenzen, vor einer Wand (das Auge bei z 2,68 vor der Fläche bei 2,9 mit einer Kugel von 0,2, die Kugel berührt nichts), frei ohne Wand; `FreeFly` | 34 Prüfungen, 0 falsch |
| `bridge` | auf Voll und auf einem Gerät auf Basis begrenzt: der erste Abgleich (301 Instanzen, auch eine Entität ohne Pose), ein Abgleich ohne Änderung schreibt nichts und das Bild lädt 0 Instanzen hoch; ein Takt bewegt fünf (5 geschrieben, 5 hochgeladen), bei Alpha 0,25, dasselbe Alpha noch einmal (nichts geschrieben), der Takt danach (einmal `now`, dann nichts); entfernt (`despawn`, `MeshRef` weg; eine entfernte Pose lässt die Instanz stehen), Meshes und Materialien gewechselt, ein Platz zwischen zwei Abgleichen neu vergeben; `retire_mesh`/`collect` (bleibt, solange benutzt, geht danach: `mesh_info` leer, die Zahl der Meshes kleiner); `reset` mit einer neuen Szene, `clear`; der Ursprung (Schritte, Runden, eine Weite unter 1 verweigert) und dieselbe Welt bei 10⁷ gegen 0; 60 Zufallsbilder (Entitäten kommen, gehen, bewegen sich, wechseln das Material; nach jedem Abgleich jede Transformation gegen die interpolierte der Welt, alle 15 Bilder das Bild); 100 000 Entitäten für die Kosten | 0 Pixel anders in jedem Vergleich mit der neu gebauten Szene (je Gerät 8 und 4 in den Zufallsbildern), bei 10⁷ mit folgendem Ursprung 0 Pixel anders, ohne über 1000; Zufallsbilder: 3370 Posen, 1390 Transformationen geschrieben, 5096 Posen angesehen; 79 Prüfungen, 0 falsch |
| `assets` (Teil 2) | die Regeln von `sibling` (9 Fälle, auch `..`, absolute Pfade, Schemata); `Loading::Frames` Schritt für Schritt mit Budget 0: `Loading` vor dem ersten `update`, dann `Ready` mit den Bytes der Datei in `mem://`, derselbe Pfad dieselbe Eintragung (zwei Handles, kein zweites Lesen), je `update` genau eine Dekodierung in der Reihenfolge der Anfragen, eine fehlende Datei `Failed` mit ihrem Pfad, der letzte Handle weg: `Unloaded` beim nächsten `update`, die Id findet nichts mehr, neu geladen eine neue Eintragung; `set_linger(2)`: zwei `update` ohne Entladen, ein `load` holt es ohne Lesen zurück, danach beim dritten entladen; ein Handle, der beim Laden weggeht, wird nie dekodiert; `insert`; `reload` einer geänderten Datei; `stats`. `Loading::Background`: 60 Dateien auf Threads, alle `Ready`, jede Id einmal, die Bytes; während des Ladens fallen gelassene Assets gehen und kommen nie als `Ready` zurück. Die Arten: ein `.glb` aus `gltf::write_document` (zwei Würfel, zwei Knoten) zurückgelesen, eine `.gltf` mit ihrem Puffer daneben, ein abgeschnittenes `.glb` und eine `.gltf`, deren Puffer außerhalb ihres Ordners läge, `Failed`; ein PNG (7 × 5, `img_png`) Pixel für Pixel, Müll `Failed`. Auf der GPU (Hintergrund): vor dem Hochladen keine Instanz, danach drei Entitäten mit je zwei Teilen (6 Instanzen) und eine mit einem fehlenden Modell ohne; das Bild gegen eine Szene, deren Teile aus den Matrizen (Entität × Weltmatrix des Knotens, `gltf::decompose`) gebaut sind; ein Abgleich ohne Änderung schreibt nichts; eine Pose bewegt (2 Teile geschrieben), ein anderes Modell (2 weg, 1 dazu), `despawn` (2 weg); das Modell ohne Handle entladen: seine Meshes verlassen den Renderer; ein neuer Renderer (verlorenes Gerät) bekommt alles wieder; `clear`; ein Bild als Textur hoch und beim Entladen wieder weg | 75 Prüfungen, 0 falsch; alle Bilder höchstens eine Stufe anders (0 Pixel über 1) |
| `audio` (Teil 2) | die Dämpfung gegen geschlossene Formen (invers mit Rolloff 1 und 0,5, linear, Grenzen, NaN, monoton über 2000 Entfernungen); das Panorama: vorn, hinten, rechts, links, schräg (3, 0, −4: pan 0,6, cos und sin von 0,4 π), oben, nah am Kopf (gezogen zur Mitte), L² + R² = Verstärkung², umgedreht vertauscht, nur relativ zum Auge; durch den Mischer von `plat::audio` ohne Gerät (`MixTest`), Probe für Probe gegen Signal × Verstärkung: eine Rampe an einer Quelle, die Quelle auf die andere Seite (die nächsten 480 Proben mit den neuen Verstärkungen), der Hörer umgedreht, hinter `far` still, zwei Stimmen mit Master 0,5 (in der Reihenfolge des Mischers summiert), eine Entität geht (ihre Stimme auch), ein Stereo-Klang behält seine Kanäle, 24 kHz durch den Resampler (Pegel nach Goertzel); einmal gespielt und beendet, Schleife von vorn, `restart`, `stop`, `play_at` folgt dem Hörer und endet; derselbe Lauf zweimal dieselben Proben | 44 Prüfungen, 0 falsch |
| Skript `input.uis` | der Weg durch `ui`: vor dem Lock „Pause“, der Klick fängt den Zeiger und zählt nicht, Tasten einer französischen Tastatur nach Lage (Z Q S D), 1000 und −500 Zählwerte sind 22° und −11°, ein Klick im Lock zählt, die rechte Taste springt, Esc pausiert (Space losgelassen, die Taste kommt nicht an), in der Pause nichts, ein Klick spielt weiter und zählt nicht | alle Erwartungen erfüllt |
| Skript `tick.uis` | ein `FixedTick` auf `plat::task::now_us`, der virtuellen Uhr: 1000 ms sind 60 Takte, W 500 ms gehalten 30 Takte (96 nach weiteren 100 ms), ein Tippen von Space und ein Klick im selben Bild je ein Takt, in der Pause 1000 ms ohne Takt | 60, 96 / 30, 102, 102 |
| `gamedemo` `shot.uis` | Klick, W 500 ms (30 Takte, 4,5 m/s: von z 6 auf 3,75), 400 Zählwerte nach links und 120 nach unten (8,8° und −2,6°; auf macOS 22,0° und −6,6°), Esc, ein Bild; seit Teil 2 mit HUD und den Leuchtfeuern (im Skript `Loading::Frames`: im ersten Bild geladen) | erfüllt, das Bild zeigt die Szene, die HUD und die Leuchtfeuer („Modell bereit“) |

| Ziel | `--check` | Skripte | GPU der Brücke |
| --- | --- | --- | --- |
| Windows x86-64 (Ryzen 5 3600), Rust 1.100 | 901 von 901 | `input`, `tick`, `shot` bestanden | RTX 2070 SUPER, Voll und Basis |
| Linux x86-64 (glibc) in WSL Ubuntu 24.04 | 901 von 901 | `input`, `tick`, `shot` bestanden (das Bild über llvmpipe) | llvmpipe (LLVM 20.1.2), Voll und Basis |
| macOS ARM64 (Mac mini M2, macOS 27.2), Rust 1.98.1 | 901 von 901 | `input`, `tick`, `shot` bestanden (das Bild über Metal) | Apple M2, Voll und Basis |

Die Zahlen sind die von G7b Teil 1. Seit G7a Teil 2 hat die Gruppe `input` 190 Prüfungen (84 für die Pads); sie bestehen auf
Windows (Rust 1.100), Linux in WSL, macOS (M2) und als WebAssembly in Node (`tests/web.mjs`), die Skripte `input.uis` und
`tick.uis` ebenso, die Prüfsumme ist überall unverändert.

Die Zahlen der Prüfungen zählen Fälle grob: eine Schleife über 200 000 Schritte ist eine Prüfung „0 Antworten anders“. Gemessen
im Entwicklungs-Build (Ryzen 5 3600 / M2): eine Million Entitäten mit Komponenten anlegen 211 / 54 ms, Summe und Verbund 11 /
10 ms, ein Drittel entfernen und neu anlegen 74 / 23 ms (in WSL 420, 21 und 85 ms; der Rechner wird von anderen Sitzungen
mitbenutzt). Die Brücke mit 100 000 Entitäten (Ryzen 5 3600 mit RTX 2070 SUPER, ruhigster von vier Läufen, die Läufe schwankten bis zum
Siebenfachen / M2): der erste Abgleich 26 / 5 ms (100 000 `Scene::add`), ein Bild nach einem Takt, in dem 1 % sich bewegte,
0,8 / 0,26 ms (1950 Transformationen: die neu bewegten und die im Takt davor bewegten, die jetzt stehen), dasselbe Alpha noch
einmal 0,5 / 0,22 ms (der Durchlauf über die Versionen der 100 000 Posen), ein Bild ohne jede Änderung unter 1 µs (die Versionen
der Spalten haben sich nicht bewegt: kein Durchlauf).

## Prüfsumme

`gametest --checksum` faltet die Bits eines festen Laufs in eine Zahl (FNV-1a über `f64::to_bits`): 900 Bilder mit 7 bis 25 ms,
400 und mehr Körper mit Schwerkraft und Boden, drehende, je Takt sterbende und neue Entitäten (die Reihenfolge der Spalten ändert
sich), eine `Input` mit denselben Ereignissen (Tasten, Klicks, Bewegung, Lock und sein Ende), drei Kameras (auch `view_in` gegen
eine Physik-Welt), ein Läufer mit Kapsel-Cast, interpolierte Posen. Die Maus mit dem Faktor 1, damit die Ziele vergleichbar sind.

| Ziel | Build | Prüfsumme |
| --- | --- | --- |
| Windows x86-64 (Ryzen 5 3600), Rust 1.100 | Entwicklung | `8edfe993e4ffcb65` (303 378 Werte) |
| Linux x86-64 (glibc) in WSL Ubuntu 24.04 | Entwicklung | `8edfe993e4ffcb65` |
| macOS ARM64 (Mac mini M2), Rust 1.98.1 | Entwicklung | `8edfe993e4ffcb65` |

Im Browser (WebAssembly in Node 22.17, einfach, SIMD und Threads, `apps/gametest/tests/web.mjs`, seit G7e) ebenfalls
`8edfe993e4ffcb65`. Nicht gelaufen: Android, ARM64 unter Linux und Windows, iOS. Die Physik ist dort bitgleich gezeigt
([PHYSICS.md](PHYSICS.md#prüfsummen)), der Kern nimmt nur `+ − × ÷` und deren Mathematik dazu.

## Was nicht getestet ist

- **Echte Fenster und echte Eingabe**: Die Demo lief in Teil 2 in echten Fenstern unter Windows (die Messungen, eingespeiste
  Tasten über `WM_KEYDOWN`, keine echte Maus) und im Browser (Chrome ohne Kopf); ob sich Lock, Klick, Esc und die
  Empfindlichkeit von Hand richtig anfühlen, prüft der Nutzer auf Windows, Linux (Wayland, X11), macOS und im Browser.
- **Assets und Raumklang**: die Gruppen `assets` und `audio` liefen unter Windows und Linux (WSL); macOS, Android und der Browser
  nicht. Der Raumklang über echte Lautsprecher (`Speakers`) lief nur in der Demo von Hand nicht (Skripte spielen nichts); geprüft
  ist er Probe für Probe über den Mischer ohne Gerät. Ein Asset im Browser lädt mit `Loading::Frames` (die Demo dort: das
  Leuchtfeuer bereit), Threads im Browser-Bündel mit geteiltem Speicher nicht.
- **Gamepads**: `Input` nur mit Schnappschüssen aus den Tests; echte Pads auf Windows, Linux, macOS und im Browser prüft der Nutzer
  von Hand (`apps/inputtest`); `plat`s Wege dorthin sind mit eingespeisten Werten geprüft (`plattest --test pad`).
- **Die Faktoren**: nur gerechnet. Der Wert für macOS (2,5) ist geschätzt, der Browser ohne `unadjustedMovement` und ein
  `devicePixelRatio` ≠ 1 sind nicht ausgeglichen.
- **Browser und Android**: `lib/game` und `lib/game_render3d` bauen für `wasm32-unknown-unknown` und `aarch64-linux-android`
  (`cargo check -p jflib`), die Demo baut für den Browser; gelaufen ist dort nichts, auch die Prüfsumme nicht. iOS:
  `cargo check` der beiden Crates auf dem Mac, nicht gestartet.
- **Statisches Linux**: `cargo check -p jflib` ohne `std` deckt den Kern für das statische Ziel ab; ein Server-Programm, das ihn
  ohne glibc baut und laufen lässt, gibt es noch nicht (gametest braucht glibc für die GPU der Brücke).
- **Was die Tests nicht erreichen**: die 2³² − 1 Generationen eines Platzes (geprüft sind 100 000), `plat_event` mit
  `E_RAW_MOTION` (braucht einen echten Lock), `win_pointer_lock_state` eines echten Fensters, die Brücke auf anderen GPUs
  als RTX 2070 SUPER, llvmpipe und M2 (die Intel-GPUs, die HD 610), ein verlorenes Gerät mitten in der Demo (geprüft ist `reset`
  mit einer neuen Szene).
- **Mehrere Threads**: Die Welt ist nicht `Sync`-geprüft und für einen Thread gedacht; Systeme laufen nacheinander.

## Was nicht gebaut ist

- Ein Begrenzer der Bildrate in `lib/game` (die Demo begrenzt mit `--limit` selbst), die kürzere `MIN_PRESENT_US` für Spiele
  ([Spielfenster](#spielfenster-zwei-wege-gemessen)); Vollbild und Present-Modus gibt es schon (`ui.window_fullscreen`,
  `jfgui_GPU_PRESENT`).
- Ein Spiel mit eigenem Fenster bekommt `Input::plat_event`; ein Weg, die Ereignisse eines Fensters ohne `ui` zu holen, ist in
  `lib/game` nicht gebaut (Weg b in der Demo holt sie selbst aus `plat::evq_pop`).
- Assets: Streaming und Detailstufen (G7j), ein Cache auf der Platte, Abhängigkeiten zwischen Assets (ein Material, das eine
  Textur-Asset nennt), die CPU-Daten eines Modells nach dem Hochladen freigeben. Raumklang: Doppler, Hall, Verdeckung,
  HRTF, Rampen beim Wechsel der Verstärkung.
- Entitäten mit vorgegebener Id (`spawn` mit der Id des Servers): Der Netcode (G7e Teil 1, [NETCODE.md](NETCODE.md)) gibt
  jeder replizierten Entität eine eigene Netz-Id (u16 mit Generation); ein Client, der eine `World` aus Schnappschüssen baut,
  bildet sie auf eigene Entitäten ab.
- Abfragen über mehr als zwei Spalten in einem Aufruf, Ereignisse beim Entfernen von Komponenten, Hierarchien (Eltern und
  Kinder), Ressourcen in der Welt: was davon nötig ist, zeigt Spiel 1.
- Die Brücke nimmt keine Lichter, keine Detailstufen und keine Materialwechsel je Instanz außerhalb von `MeshRef`; eine Entität
  ohne `Pose` steht am Ursprung, eine entfernte `Pose` lässt die Instanz, wo sie war.
- Touch-Steuerung (später), Tastenkombinationen als Bindung (Umschalt+W), Rumpeln der Pads (G7a Teil 2: auf macOS nur über Core
  Haptics, nicht billig; offen), eine Belegung je Pad-Modell über die Standard-Belegung hinaus.

## Offen nach Teil 2

- **Der Weg des Spielfensters**: Vorschlag Weg (a) ([Spielfenster](#spielfenster-zwei-wege-gemessen)); die Entscheidung trifft der
  Nutzer. Danach gegebenenfalls die Stellschrauben von Weg (a) für Spiele (`MIN_PRESENT_US`, das Warten der Ansicht) und ein
  Begrenzer in `lib/game`, gemessen wie hier.
- **Die Faktoren** von Hand prüfen (Windows, Wayland, X11, macOS, Chrome, Firefox): dieselbe Empfindlichkeit, dieselbe
  Mausbewegung, derselbe Winkel? Besonders macOS (2,5 ist geschätzt) und der Browser ohne `unadjustedMovement`.
- **Die Demo auf allen Zielen laufen lassen**: In Teil 2 lief sie unter Windows im echten Fenster und im Browser (Chrome ohne
  Kopf); Linux und macOS im echten Fenster und die Messungen auf Iris Xe, HD 610 und dem Mac fehlen.
- **Bewegte Plattformen und der Controller** (G7c Teil 2) setzen auf `Pose` und `World::begin_tick` auf: der Controller als
  System, seine Kapsel aus der Pose.
- **Netcode**: gebaut in G7e Teil 1 als `game::net` ([NETCODE.md](NETCODE.md)). Er nimmt die Versionen der Spalten nicht: Ein
  Schnappschuss vergleicht die quantisierten Zustände mit dem, was der Client hat (je Client verschieden), das ist billig
  genug (rund 0,2 ms je Tick für 16 Spieler und 300 Requisiten). Die Eingabe eines Takts geht als `net::NetInput` (aus
  `TickInput` gebaut) über die Leitung.
