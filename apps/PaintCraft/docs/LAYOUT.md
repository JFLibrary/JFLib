# Werkzeugfenster-Layout und Icons

## Layout der Werkzeugfenster (`src/wnd_dock.s`)

`dock_frame` läuft als erstes Frame-Modul und setzt jedes Frame die Position der vier
Werkzeugfenster (Werkzeuge, Verlauf, Ebenen, Farben) im Arbeitsbereich (Canvas-Fläche unter den
Symbolleisten, über der Statusleiste, 8 DIP Rand):

| Modus | Bedeutung |
|---|---|
| automatisch (`DM_AUTO`) | Werkzeuge oben links, Farben unten links (rechts neben den Werkzeugen, wenn sie sich überlappen würden), Verlauf oben rechts, Ebenen unten rechts |
| verankert (`DM_ANCHOR`) | vom Benutzer innerhalb des Hauptfensters verschoben: Abstand zur nächsten waagrechten und senkrechten Kante bleibt bei jeder Größenänderung |
| frei (`DM_FREE`) | aus dem Hauptfenster herausgezogen (Mitte außerhalb): bleibt am Bildschirm stehen; unter Windows im Arbeitsbereich seines Monitors gehalten (z. B. Farben mit „Mehr >>“ am rechten Rand) |

Angedockte Fenster (automatisch / verankert) ragen nie über das Hauptfenster hinaus. Platzmangel:
der Verlauf wird ausgeblendet, wenn er nicht über die Ebenen passt; ein sehr kleines Fenster
(Arbeitsbereich < 640 × 360 DIP) zeigt nur Werkzeuge (links) und Farben (rechts); ist die Höhe
zu klein für die zweispaltigen Werkzeuge, liegen sie in zwei Zeilen zu je zehn Knöpfen
(quadratische 32-DIP-Knöpfe). Ein geschlossenes Fenster erscheint beim Öffnen wieder an seinem
Platz. „Fensteranordnung zurücksetzen“ und die Werkseinstellungen stellen auf automatisch.
Gespeichert wird je Fenster `dock` (0 / 1 / 2), bei 1 `ax dx ay dy`, bei 2 `x y`; Dateien ohne
`dock` (ältere Versionen) bekommen das automatische Layout. Test: `tests/wnd_dock.uis`.

## Icons (`src/pn_icons.s`, `src/icons_gen.s`)

PaintCraft zeichnet 38 Icons (Symbolleiste, Werkzeugfenster, Fensterschalter, Menüs) farbig als
Vektor-Schichten auf einem 16er-Raster statt der einfarbigen Glyphen des Stils. Die Daten erzeugt
`python tools/gen_pn_icons.py` (Formen, Farben; Farbe 0 = Textfarbe, damit Umrisse in hell und
dunkel passen). Gezeichnet wird über den Bibliotheks-Haken `ui_icon_hook` zur Renderzeit, nur in
den normalen Textfarben: deaktivierte oder weiße Icons bleiben Glyphen des Stils. Der Stil
selbst wird nicht verändert.
