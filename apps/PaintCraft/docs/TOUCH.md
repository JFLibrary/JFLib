# PaintCraft mit den Fingern

Was die Finger in PaintCraft tun, warum, und was sich nur auf einem echten Gerät prüfen lässt. Die
Bibliothek dazu: [`lib/ui/touch.rs`](../../../lib/ui/touch.rs), [GUIDE §37](../../../docs/GUIDE.md).

## Was die Finger tun

| Geste | Wirkung |
| --- | --- |
| Zwei Finger | Zoomen stufenlos um den Punkt zwischen den Fingern und verschieben dabei die Ansicht in jede Richtung; der Bildpunkt zwischen den Fingern bleibt dort. Das Bild darf dabei den sonst üblichen Rahmen verlassen; ein Viertel der Fläche bleibt sichtbar |
| Ein Finger | Malt mit dem aktuellen Werkzeug, oder verschiebt die Ansicht (Einstellung, unten) |
| Tippen | Ein Klick an der Stelle, auch wenn der Finger dabei ein paar Pixel rollt |
| Finger liegt, ohne sich zu bewegen | Der Druck kommt nach 400 ms (ein Punkt des Pinsels), kein Kontextmenü |
| Werkzeugfenster: oben drücken und halten | In gut einer halben Sekunde läuft eine Linie unter der Titelleiste voll, dann folgt das Fenster dem Finger in jede Richtung. Ein schnelles Wischen dort tut nichts |
| Zoomregler, Zahnrad und andere kleine Knöpfe | Auch ein Finger knapp daneben trifft: Auf dem Hintergrund gewinnt das nächste kleine Bedienelement in 44 DIP Reichweite; mit der Maus ändert sich nichts |
| Kleine Knöpfe am Rand eines Werkzeugfensters („+“ und Stift im Ebenen-Fenster) | Sie gewinnen gegen den für Finger breiten Griff zum Größenziehen; daneben zieht der Griff wie gewohnt |
| Pinselgröße − und + | Tippen ist ein Schritt. Bleibt der Finger liegen oder hält man Stift oder Maus gedrückt, läuft die Größe mit fester Geschwindigkeit weiter (25 Schritte in der Sekunde) |
| Zweiter Finger kurz nach dem ersten (unter 200 ms) | Der angefangene Strich wird zurückgenommen, es zählt die Zwei-Finger-Geste. Später stört ein zweiter Finger den Strich nicht |

**Ein Finger und der Stift.** Einstellungen ▸ Stift & Tablett ▸ „Ein Finger auf der Leinwand“:

- *Automatisch* (Vorgabe): Der Finger malt, bis in dieser Sitzung ein Stift die Leinwand berührt hat, danach
  verschiebt er die Ansicht.
- *Malt mit dem aktuellen Werkzeug* / *Verschiebt die Ansicht*: immer so (`[UI] finger` = 1 / 2).

Warum so: Wer nur Finger hat (der Surface-Bericht), will damit malen. Wer einen Stift benutzt, malt damit, und
der Finger ist dann frei für die Ansicht, wie in den meisten Zeichen-Apps. Das Gerät allein entscheidet nicht:
Ein gekoppelter Stift, der gerade im Etui liegt, soll den Finger nicht abschalten.

**Der Stift geht vor.** Solange der Stift auf dem Bildschirm oder nah darüber ist und 500 ms danach zählen
Finger nicht: Der Handballen malt nicht und verschiebt nichts. Ein Stift, der aufsetzt, beendet Finger, die schon
liegen.

## Was der Surface-Bericht war

Gemessen mit echten Kontakten (`InjectTouchInput`, 60 px breit wie ein Finger, ohne Touchscreen), vor der
Änderung:

- **Zoomregler „triggert das Widget darüber“:** nachgestellt. Windows gibt einen breiten Kontakt dem Fensterrahmen,
  den er berührt. Der unsichtbare Rahmen des Ebenen-Fensters endet genau an der Statusleiste: Der Finger auf dem
  Regler kam als `WM_NCPOINTERDOWN HTBOTTOM` dort an und zog das Fenster größer.
- **Pinch „eher müßig“:** nachgestellt. Windows macht daraus Strg + Mausrad, also eine Voreinstellung pro Stufe.
- **Canvas ziehen „geht gar nicht“:** nachgestellt. Zwei Finger, die sich gemeinsam bewegen, ergaben kein einziges
  Ereignis. Dazu blieb ein Bild, das in die Fläche passt, immer mittig.
- **Werkzeugfenster „nur horizontal“:** teilweise nachgestellt. Ein senkrechtes Ziehen oben traf den oberen Rahmen und
  änderte die Größe; ob auf dem Surface noch mehr dahintersteckt, zeigt nur das Gerät.
- **Zahnrad „gar nichts“:** hier *nicht* nachgestellt. Ein sauberer Tipp öffnete die Einstellungen (normal und
  maximiert, 100 %), einmal unter rund 20 Versuchen kam gar kein Ereignis an. Ein Finger, der beim Abheben den Knopf
  verlässt, war vorher kein Klick; das ist jetzt behoben, aber ob das die Ursache war, zeigt nur das Gerät.

## Was der zweite Surface-Test war

- **Zoomregler „triggert das Fenster darüber“, und die Leinwand verschiebt sich mit:** teilweise nachgestellt. Mit dem Finger
  wird die Titelleiste höher, und das Ebenen-Fenster wuchs in die Statusleiste: Sein unsichtbarer Rahmen lag über dem ganzen
  Regler. Die automatische Anordnung rechnet jetzt mit dieser Höhe. Die Leinwand: Windows verschiebt die gemeldete Position
  eines Kontakts auf den Rahmen, dem es ihn gibt, und lässt sie dabei (gemessen: 17 Pixel; `ptPixelLocationRaw` ebenso), und
  das lag vor der neuen Anordnung über der Leinwand; nach dem ersten Stiftstrich verschiebt ein Finger dort die Ansicht. Jetzt
  zählt für so einen Kontakt die Mitte des Kontaktrechtecks.
  Außerdem geändert: Der Rahmen eines Werkzeugfensters gibt einen Finger nie mehr ans System
  (dafür ist das Band im Fenster da), verborgene Fenster anderer Programme in der Z-Reihenfolge zählen nicht mehr, ein
  Kontakt, dessen Mitte nicht im Fenster liegt, das ihn bekam, geht an das Fenster unter seiner Mitte, und knapp über der
  Statusleiste meint ein Finger den Regler, nicht die Leinwand.
- **Wie es weitergeht, wenn es noch hakt:** `set jfgui_TOUCH_LOG=1`, dann `build\win\PaintCraft.exe 2> finger.txt`. In der
  Datei steht je Finger, wo er aufkam, wem er gehört (`ht` ist der Bereich des Fensterrahmens, 0 das Fensterinnere) und
  was Windows meldete: Position, Rohposition, Kontaktrechteck, rohes Kontaktrechteck.
- **Weiße Fenster:** nachgestellt, die GPU-Ausgabe (`docs/GPU.md`, „Aus- und Einblenden“).
- **Dicker Punkt beim Absetzen des Stifts:** Ein Punkt ohne Druck zeichnete in voller Breite; ein Strich mit Druck behält
  jetzt den letzten.

## Tests

| Was | Wo | Was es beweist |
| --- | --- | --- |
| `tests/touch.uis` (Windows, Linux) | Skript auf der virtuellen Uhr | Die Entscheidungen des Erkenners und von PaintCraft: alle Gesten oben, Handballen, Einstellung |
| `apps/plattest/tests/touch.uis` | Attrappen der Zeiger-API (Windows), eingespeiste Ereignisse (`wl_touch`, XInput 2.2) | Der Weg vom System zum Finger: Ids, Verlauf, Abbruch, keine Doppelereignisse |
| `python tools/touchcheck.py` (Windows) | Echtes Fenster, echte Kontakte über `InjectTouchInput`, ohne Touchscreen | Was Windows selbst entscheidet: Zahnrad, Regler unter dem Rahmen, Halten und Ziehen, kein Strg + Rad. Überspringt mit Meldung, wenn der Bildschirm gesperrt oder das Fenster verdeckt ist |

Nicht geprüft werden kann hier: ein echter Finger (Kontaktform, Zittern, die Handballen-Erkennung des Geräts), das
Tablet-Verhalten von Windows und die Bildschirmtastatur.

## Prüfliste für das Surface

Bauen und starten, im Ordner des Repositories:

```
python build_rs.py run PaintCraft
```

Mit dem Finger, ohne Stift:

1. **Zahnrad** oben rechts antippen: Einstellungen gehen auf. Auch ein paarmal unsauber, knapp daneben, mit etwas
   Rollen. Mit dem X oder Esc wieder schließen.
2. **Zoomregler** unten rechts: nach links und rechts ziehen. Die Zoomzahl ändert sich, das Ebenen-Fenster darüber bleibt, wie
   es ist. Auch einmal auf den Regler tippen.
3. **Ein Werkzeugfenster verschieben:** oben auf die Titelleiste drücken und liegen lassen. Eine blaue Linie läuft in
   gut einer halben Sekunde voll, danach folgt das Fenster dem Finger, nach oben, unten, links, rechts. Ist die Zeit
   angenehm, zu lang oder zu kurz (`HOLD_LONG_MS` in `lib/ui/touch.rs`)? Schnell darüberwischen darf nichts bewegen.
4. **Knöpfe in einem Werkzeugfenster** antippen (Pinsel, Radierer): Das Werkzeug wechselt, das Fenster bleibt liegen.
   Auch „+“ und den Stift unten im Ebenen-Fenster: Sie legen eine Ebene an und öffnen die Eigenschaften, das Fenster ändert
   dabei nicht seine Größe.
5. **Zoomen:** zwei Finger auf die Leinwand und auseinanderziehen. Läuft es flüssig, ohne Stufen? Der Punkt zwischen den
   Fingern bleibt unter den Fingern. Auch mit einem großen Bild (etwa 4000 × 3000) probieren.
6. **Verschieben:** zwei Finger gemeinsam ziehen, nach rechts, links, oben, unten, auch ohne vorher zu zoomen.
7. **Malen:** ein Finger zieht einen Strich, Tippen macht einen Punkt. Mit zwei Fingern zoomen darf keinen Strich hinterlassen.
8. **Rollleisten** auf der Leinwand: Bei einem großen Bild sind sie beim Finger weg, und die Maus oder das Touchpad bringt
   sie zurück. Das Mausrad setzt die Ansicht in die üblichen Grenzen zurück.
9. **Listen** im Ebenen- und Verlaufsfenster mit dem Finger rollen; Menüs und die Auswahllisten (Zoom in der
   Statusleiste) öffnen und tippen.

Mit dem Stift, falls einer da ist:

10. Mit dem Stift zeichnen und die Hand dabei auflegen: Nichts verschiebt sich, nichts malt außer dem Stift. Der Andruck
    wirkt wie bisher.
11. Nach dem ersten Stiftstrich: Ein Finger verschiebt die Ansicht (Einstellung „Automatisch“).

Im Tablet-Betrieb (Tastatur ab):

12. Ein Textfeld antippen, etwa „Pinselgröße“ oder den Dateinamen im Speichern-Dialog: Die Bildschirmtastatur geht auf, und
    Tippen landet im Feld. Ein anderes Bedienelement antippen: Sie geht wieder zu.
13. Vom Rand nach innen wischen (Windows-Gesten): Eine angefangene Geste bricht sauber ab, kein Strich bleibt hängen.

Was nicht passt, bitte mit der Nummer melden. Dazu hilft `build\win\PaintCraft.exe --log-events > ereignisse.txt`: In der
Datei steht jedes Ereignis; bei einem Finger steht in `data` an der fünften Stelle von rechts eine 3 (`data=30000`).
