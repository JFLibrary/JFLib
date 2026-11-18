# GAMECORE -- der Spielkern

`lib/game` (`jflib-game`, in der Fassade `jflib::game`) ist der Kern der Spiel-Engine ([GAME.md](GAME.md), Stufe G7b,
Entscheidung E9): ein Weltmodell aus Entitäten und Komponenten, ein fester Takt, Eingabe-Aktionen und Kamera-Steuerungen. Die
Brücke zu [`render3d`](RENDER3D.md) ist ein eigenes kleines Crate, `lib/game_render3d` (`jflib-game-render3d`,
`jflib::game_render3d`). Beide ohne Betriebssystem (`#![no_std]`, `#![forbid(unsafe_code)]`, keine fremden Crates); `lib/game`
hängt an `rt`, `plat`, `base` und [`lib/physics`](PHYSICS.md), nicht an einer GPU: Der Server von Spiel 1 rechnet dieselben
Systeme ohne Fenster, unter Linux statisch ohne libc. Wie die Physik rechnet der Kern auf allen Zielen dieselben Bits.

Stand: G7b Teil 1 (dieser Lauf): Weltmodell, Takt, Eingabe, Kameras, Brücke, `apps/gametest`, die Demo `apps/gamedemo`. Teil 2
fehlt noch ([Für Teil 2](#für-teil-2)): die Messung der beiden Wege des Spielfensters, Vollbild, Bildrate-Begrenzer und
Present-Modus, Assets über das VFS im Hintergrund, Raumklang, die Demo auf allen Zielen.

| Teil | Wo |
| --- | --- |
| Entitäten, Welt, Bündel | [`world.rs`](../lib/game/src/world.rs) |
| Spalten (Sparse-Sets), Versionen | [`sparse.rs`](../lib/game/src/sparse.rs) |
| `Transform`, `Pose`, Interpolation | [`pose.rs`](../lib/game/src/pose.rs) |
| Takt, Systeme, `Game` | [`tick.rs`](../lib/game/src/tick.rs) |
| Eingabe-Aktionen, Maus-Einstellungen, Pointer-Lock | [`input.rs`](../lib/game/src/input.rs) |
| Kameras | [`camera.rs`](../lib/game/src/camera.rs) |
| Brücke zu render3d | [`lib/game_render3d/src/lib.rs`](../lib/game_render3d/src/lib.rs) |
| Tests | [`apps/gametest`](../apps/gametest/README.md): [`world_t.rs`](../apps/gametest/src/world_t.rs), [`tick_t.rs`](../apps/gametest/src/tick_t.rs), [`input_t.rs`](../apps/gametest/src/input_t.rs), [`camera_t.rs`](../apps/gametest/src/camera_t.rs), [`bridge.rs`](../apps/gametest/src/bridge.rs), [`checksum.rs`](../apps/gametest/src/checksum.rs), das Fenster der Skripte [`probe.rs`](../apps/gametest/src/probe.rs) |
| Demo | [`apps/gamedemo`](../apps/gamedemo/README.md) |

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
  Wheel}` (`#[non_exhaustive]`; `name`, `parse`, `label`, `is_motion`), `MOUSE_LEFT` bis `MOUSE_X2`, `Bind { source, scale }`
  (`new`, `scaled`, `key`, `text`, `parse`), `ActionKind::{Button, Axis, Motion}` (`takes`), `ActionId` (`index`), `ActionMap`
  (`add(name, kind, defaults)`, `find`, `name`, `kind`, `binds`, `defaults`, `bind`, `unbind`, `set_binds`, `reset`,
  `reset_all`, `users`, `ids`, `store`, `load`), `MouseSettings { sensitivity, invert_x, invert_y, factor }` (`DEFAULT`,
  `motion_factor`, `radians_per_unit`, `store`, `load`), `DEGREES_PER_COUNT` (0,022), `platform_motion_factor()`,
  `FACTOR_WINDOWS`, `FACTOR_LINUX`, `FACTOR_MACOS`, `FACTOR_BROWSER`, `FACTOR_OTHER`. `Input`: hinein `key`, `button`, `buttons`
  (die Masken von `UiInput`), `motion`, `wheel`, `set_lock`, `plat_event`, `release_all`; je Bild `down`, `pressed`, `released`,
  `axis`, `motion_of`, `end_frame`; je Takt `tick() -> TickInput` (`get`, `down`, `pressed`, `released`, `value`, `states`;
  `ActionState { down, pressed, released, value }`); der Lock `playing`, `lock`, `take_lock_request`, `take_pause`, `pauses`,
  `set_require_lock`; `start_capture`, `take_capture`, `capturing`; `edit_map`, `map`, `mouse`, `set_mouse`, `key_held`,
  `button_held`; `store`, `load`.
- **Kameras** (`game::camera`): `View { eye, forward, up }` (`look_at`, `right`), `FirstPerson { eye, yaw, pitch, pitch_limit }`
  (`look`, `forward`, `rotation`, `walk_dir`, `view`), `Follow { target, yaw, pitch, distance, min_distance, max_distance,
  pitch_min, pitch_max }` (`look`, `zoom`, `eye`, `view`, `view_in(&physics::World, &Filter, radius)`), `FreeFly { eye, yaw,
  pitch, pitch_limit, speed }` (`look`, `fly`, `view`), `basis`, `yaw_pitch_of`, `walk_dir`, `wrap_angle`, `PITCH_LIMIT` (89°).
- **Brücke** (`game_render3d`): `MeshRef { mesh, material }`, `Bridge` (`new`, `sync(&World, &mut Scene, alpha) -> SyncStats {
  added, removed, switched, moved, visited }`, `instance`, `len`, `mesh_users`, `material_users`, `reset`, `clear`,
  `retire_mesh`, `retire_material`, `collect(&mut Renderer)`, `retired`, `follow(&mut Scene, eye)`, `set_origin_step`,
  `origin_step`), `ORIGIN_STEP` (512 m), `camera(&View, fov_y, aspect, near, far)`, `transform(&Transform)`.

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
- **Platz für Gamepads** (G7a Teil 2): `Source` ist `#[non_exhaustive]`, Knöpfe und Sticks kommen als weitere Quellen; eine
  Achse nimmt analoge Werte wie die Skalen der Tasten; ein Pad bekommt eigene Einstellungen neben `MouseSettings`.
- **Gespeichert** in einer Einstellungsdatei der Bibliothek: `Input::store(ini, "input")` und `load` schreiben in einen
  Abschnitt einer `base::Ini`, mit `base::Settings` in deren freien Teil (`settings.ini_mut()`, was auch `apps/prefstest`
  benutzt). Wie dort steht nur, was von der Vorgabe abweicht: `bind.NAME=KeyD -KeyA` (Name der Quelle wie `KeyboardEvent.code`,
  `-` für die Skala −1, `*0.5` für andere, `none` für keine), `sensitivity`, `invert_x`, `invert_y`, `motion_factor`. Eine neue
  Vorgabe erreicht so jeden, der sie nie geändert hat; Unlesbares wird ausgelassen und gezählt.

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
| `input` | Namen aller Tasten aus plats Tabelle hin und zurück, Maus, Bewegung, unbekannte Namen; Bindungstexte mit Skalen; die Karte (was verweigert wird, `users`, `reset`); der Abschnitt einer `Ini` (nur Abweichungen, `none`, Unlesbares gezählt, durch den Text der Datei und zurück), Maus-Einstellungen (Grenzen, unlesbarer Faktor), dasselbe durch `base::Settings` in `mem://` und zurück; der Lock: Tasten vorher treiben nichts, der Klick, der fragt, löst nichts aus und sein Loslassen auch nicht, gehaltenes W zählt danach ohne „gedrückt“, ein Klick im Lock feuert, die Bewegung ist Zählwerte × Faktor × 0,022°, das Ende des Locks pausiert einmal und lässt alles los, in der Pause keine Bewegung und keine Aktion, `Pending` verschluckt weiter, `Refused`, `Unsupported` spielt ohne Lock, `set_require_lock(false)`; Bild gegen Takt (ein Tippen ergibt einen Takt mit gehalten, gedrückt und losgelassen, gehalten über drei Takte, Wiederholung ändert nichts, Bewegung zweier Bilder in einem Takt), Achsen (Gegenrichtungen, Skala 0,5, Grenze 1), eine Bindung, die im Halten geht, Empfindlichkeit 2 und Invertieren; Mausrad (eine Raste, zwei halbe, nach unten), die Masken von `UiInput`, das Abfangen für eine Einstellungsseite; `plat_event` (Tasten über den Scancode dieser Plattform, Knöpfe, Rad, Lock 1 und 0, Fokusverlust); der Faktor dieser Plattform, ein eigener, 1 000 Zählwerte = 22° | 106 Prüfungen, 0 falsch |
| `camera` | 20 000 zufällige Gier- und Nickwinkel: die Basis gegen `Quat::from_yaw_pitch` (höchstens 10⁻¹⁵), orthonormal, rechtshändig, die Winkel zurück; Grenzen, `wrap_angle`, `look_at`; `FirstPerson` (rechts dreht nach +X, unten blickt nach unten, ±89°, NaN wird übergangen), `walk_dir`; `Follow` über 2000 Drehungen (Abstand auf 10⁻¹², blickt aufs Ziel), Zoom und seine Grenzen, vor einer Wand (das Auge bei z 2,68 vor der Fläche bei 2,9 mit einer Kugel von 0,2, die Kugel berührt nichts), frei ohne Wand; `FreeFly` | 34 Prüfungen, 0 falsch |
| `bridge` | auf Voll und auf einem Gerät auf Basis begrenzt: der erste Abgleich (301 Instanzen, auch eine Entität ohne Pose), ein Abgleich ohne Änderung schreibt nichts und das Bild lädt 0 Instanzen hoch; ein Takt bewegt fünf (5 geschrieben, 5 hochgeladen), bei Alpha 0,25, dasselbe Alpha noch einmal (nichts geschrieben), der Takt danach (einmal `now`, dann nichts); entfernt (`despawn`, `MeshRef` weg; eine entfernte Pose lässt die Instanz stehen), Meshes und Materialien gewechselt, ein Platz zwischen zwei Abgleichen neu vergeben; `retire_mesh`/`collect` (bleibt, solange benutzt, geht danach: `mesh_info` leer, die Zahl der Meshes kleiner); `reset` mit einer neuen Szene, `clear`; der Ursprung (Schritte, Runden, eine Weite unter 1 verweigert) und dieselbe Welt bei 10⁷ gegen 0; 60 Zufallsbilder (Entitäten kommen, gehen, bewegen sich, wechseln das Material; nach jedem Abgleich jede Transformation gegen die interpolierte der Welt, alle 15 Bilder das Bild); 100 000 Entitäten für die Kosten | 0 Pixel anders in jedem Vergleich mit der neu gebauten Szene (je Gerät 8 und 4 in den Zufallsbildern), bei 10⁷ mit folgendem Ursprung 0 Pixel anders, ohne über 1000; Zufallsbilder: 3370 Posen, 1390 Transformationen geschrieben, 5096 Posen angesehen; 79 Prüfungen, 0 falsch |
| Skript `input.uis` | der Weg durch `ui`: vor dem Lock „Pause“, der Klick fängt den Zeiger und zählt nicht, Tasten einer französischen Tastatur nach Lage (Z Q S D), 1000 und −500 Zählwerte sind 22° und −11°, ein Klick im Lock zählt, die rechte Taste springt, Esc pausiert (Space losgelassen, die Taste kommt nicht an), in der Pause nichts, ein Klick spielt weiter und zählt nicht | alle Erwartungen erfüllt |
| Skript `tick.uis` | ein `FixedTick` auf `plat::task::now_us`, der virtuellen Uhr: 1000 ms sind 60 Takte, W 500 ms gehalten 30 Takte (96 nach weiteren 100 ms), ein Tippen von Space und ein Klick im selben Bild je ein Takt, in der Pause 1000 ms ohne Takt | 60, 96 / 30, 102, 102 |
| `gamedemo` `shot.uis` | Klick, W 500 ms (30 Takte, 4,5 m/s: von z 6 auf 3,75), 400 Zählwerte nach links und 120 nach unten (8,8° und −2,6°; auf macOS 22,0° und −6,6°), Esc, ein Bild | erfüllt, das Bild zeigt die Szene |

| Ziel | `--check` | Skripte | GPU der Brücke |
| --- | --- | --- | --- |
| Windows x86-64 (Ryzen 5 3600), Rust 1.100 | 901 von 901 | `input`, `tick`, `shot` bestanden | RTX 2070 SUPER, Voll und Basis |
| Linux x86-64 (glibc) in WSL Ubuntu 24.04 | 901 von 901 | `input`, `tick`, `shot` bestanden (das Bild über llvmpipe) | llvmpipe (LLVM 20.1.2), Voll und Basis |
| macOS ARM64 (Mac mini M2, macOS 27.2), Rust 1.98.1 | 901 von 901 | `input`, `tick`, `shot` bestanden (das Bild über Metal) | Apple M2, Voll und Basis |

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

- **Echte Fenster und echte Eingabe**: Die Demo lief nur ohne Fenster (Windows, Linux in WSL, macOS); ob sich Lock, Klick,
  Esc und die Empfindlichkeit von Hand richtig anfühlen, prüft der Nutzer in Teil 2 auf Windows, Linux (Wayland, X11), macOS und
  im Browser.
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

- Was zu Teil 2 gehört: Spielfenster (die Messung `gpu_view` über das ganze Fenster gegen eine eigene Swapchain mit der UI als
  Textur, Eingabe bis Bild und Bildabstand), Vollbild, Bildrate-Begrenzer, Present-Modus, Assets über das VFS im Hintergrund mit
  Handles, Raumklang über `plat::audio`.
- Ein Spiel mit eigenem Fenster bekommt `Input::plat_event`; ein Weg, die Ereignisse eines Fensters ohne `ui` zu holen, ist in
  `lib/game` nicht gebaut (das gehört zur Messung des eigenen Swapchain-Wegs).
- Entitäten mit vorgegebener Id (`spawn` mit der Id des Servers): Der Netcode (G7e Teil 1, [NETCODE.md](NETCODE.md)) gibt
  jeder replizierten Entität eine eigene Netz-Id (u16 mit Generation); ein Client, der eine `World` aus Schnappschüssen baut,
  bildet sie auf eigene Entitäten ab.
- Abfragen über mehr als zwei Spalten in einem Aufruf, Ereignisse beim Entfernen von Komponenten, Hierarchien (Eltern und
  Kinder), Ressourcen in der Welt: was davon nötig ist, zeigt Spiel 1.
- Die Brücke nimmt keine Lichter, keine Detailstufen und keine Materialwechsel je Instanz außerhalb von `MeshRef`; eine Entität
  ohne `Pose` steht am Ursprung, eine entfernte `Pose` lässt die Instanz, wo sie war.
- Gamepads (G7a Teil 2), Touch-Steuerung (später), Tastenkombinationen als Bindung (Umschalt+W).

## Für Teil 2

- **Die Messung der beiden Wege**: Die Demo ist der Ausgangspunkt. Heute holt ein Timer jedes Bild (`ui.timer(1, 1)`); ob
  das an die Bildwiederholung des Bildschirms gebunden ist, wie viel Eingabe bis Bild es kostet und wie der Abstand der Bilder
  schwankt, ist zu messen, ebenso eine eigene Swapchain mit `Input::plat_event`.
- **Die Faktoren** von Hand prüfen (Windows, Wayland, X11, macOS, Chrome, Firefox): dieselbe Empfindlichkeit, dieselbe
  Mausbewegung, derselbe Winkel? Besonders macOS (2,5 ist geschätzt) und der Browser ohne `unadjustedMovement`.
- **Die Demo auf allen Zielen laufen lassen**: Sie baut für Windows, Linux, macOS und den Browser; gelaufen ist sie in diesem Lauf
  nur ohne Fenster (das Skript `shot.uis` unter Windows, in WSL und auf dem M2), im Browser gar nicht.
- **Bewegte Plattformen und der Controller** (G7c Teil 2) setzen auf `Pose` und `World::begin_tick` auf: der Controller als
  System, seine Kapsel aus der Pose.
- **Netcode**: gebaut in G7e Teil 1 als `game::net` ([NETCODE.md](NETCODE.md)). Er nimmt die Versionen der Spalten nicht: Ein
  Schnappschuss vergleicht die quantisierten Zustände mit dem, was der Client hat (je Client verschieden), das ist billig
  genug (rund 0,2 ms je Tick für 16 Spieler und 300 Requisiten). Die Eingabe eines Takts geht als `net::NetInput` (aus
  `TickInput` gebaut) über die Leitung.
