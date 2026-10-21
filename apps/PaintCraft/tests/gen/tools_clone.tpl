# tools_clone.uis -- Klonen (Stempel) (agent T6): Strg+click sets the anchor (pixel + layer),
# the first stroke fixes the offset, strokes copy the source through the tool layer (one step
# each), the source is read before the stroke (no self-copy), another layer as the source
# (DRAW-017), Füllmethode Unterschied / Überschreiben (DRAW-013 / 014), Esc, undo / redo, the
# test clip rectangle, the crosshair, the error text without an anchor.
# Mapping (1280 x 800, zoom 100 %): image pixel (x, y) at (240 + x, 148 + y).  Probes: 901
# layer / 900 composite pixel (x | y << 16) -> #9900; 902 / 903 / 904 test fills (no step).
# Records (t_clone.s): #532736 anchor "x,y,layer" (value set), #532737 offset "dx,dy" (value
# fixed), #532738 crosshair "sx,sy" (window px, value shown).  Hooks 930 width * 100, 931
# hardness, 932 antialiasing, 933 blend mode, 941 / 942 test clip, 846 0 = clear the anchor.
# Expected pixels: aliased dabs of width w are w x w discs (d < w / 2 around the pixel under
# the pointer); dest (x, y) = source (x + dx, y + dy); Unterschied |lhs - rhs|, Überschreiben
# = the source pixel (all four channels) at full coverage.
size 1280 800
theme light
key l
expect #9931 value 14
expect text:"Pinselgröße:" exists
expect text:"Härte:" exists
expect text:"Abstand:" exists
expect #532736 value 0
expect #532737 value 0
expect #532738 value 0
expect #9935 value 1

# ------------------------------------------------------------ pattern: red [20,40)^2, blue
# [40,60) x [20,40), green [20,40) x [40,60)
cmd 902 1
cmd 903 {P 20 20}
cmd 904 {P 40 40}
cmd 902 3
cmd 903 {P 40 20}
cmd 904 {P 60 40}
cmd 902 2
cmd 903 {P 20 40}
cmd 904 {P 40 60}

# ------------------------------------------------------------ Strg+click = anchor (no step)
drag {W 30 30} {W 30 30} 1 ctrl
expect #532736 text "30,30,0"
expect #532736 value 1
expect #532737 value 0
expect #532738 text "{W 30 30}"
expect #532738 value 1
expect #9935 value 1
expect #9930 value 0

# ------------------------------------------------------------ aliased width 5: one click copies
# the 5 x 5 disc around the anchor; the offset is fixed now
cmd 932 0
cmd 930 500
click {W 130 130}
expect #9935 value 2
expect #9930 value 0
expect #532737 text "-100,-100"
expect #532737 value 1
cmd 901 {P 130 130}
expect #9900 text "FFFF0000"
cmd 901 {P 132 130}
expect #9900 text "FFFF0000"
cmd 901 {P 133 130}
expect #9900 text "FFFFFFFF"
cmd 901 {P 132 132}
expect #9900 text "FFFFFFFF"
cmd 900 {P 131 131}
expect #9900 text "FFFF0000"

# ------------------------------------------------------------ the offset stays: (150, 130) <- (50, 30)
click {W 150 130}
expect #9935 value 3
expect #532737 text "-100,-100"
cmd 901 {P 150 130}
expect #9900 text "FF0000FF"
expect #532738 text "{W 50 30}"

# ------------------------------------------------------------ a drag: one step, dest = source + offset
drag {W 120 140} {W 160 140} 8
expect #9935 value 4
cmd 901 {P 125 140}
expect #9900 text "FF00FF00"
cmd 901 {P 125 138}
expect #9900 text "FFFF0000"
cmd 901 {P 145 138}
expect #9900 text "FF0000FF"
cmd 901 {P 150 140}
expect #9900 text "FFFFFFFF"
cmd 901 {P 125 143}
expect #9900 text "FFFFFFFF"
# the crosshair follows the pointer (+ offset)
move {W 200 200}
expect #532738 text "{W 100 100}"
expect #532738 value 1

# ------------------------------------------------------------ the source is the layer before the
# stroke: red [195,205) x [295,305), anchor (200, 300), a stroke from (210, 300) to the right
# copies the red square once (a self-copying stroke would drag it along)
cmd 902 1
cmd 903 {P 195 295}
cmd 904 {P 205 305}
drag {W 200 300} {W 200 300} 1 ctrl
expect #532736 text "200,300,0"
expect #532737 value 0
drag {W 210 300} {W 260 300} 10
expect #9935 value 5
expect #532737 text "-10,0"
cmd 901 {P 210 300}
expect #9900 text "FFFF0000"
cmd 901 {P 214 300}
expect #9900 text "FFFF0000"
cmd 901 {P 216 300}
expect #9900 text "FFFFFFFF"
cmd 901 {P 220 300}
expect #9900 text "FFFFFFFF"
cmd 901 {P 240 300}
expect #9900 text "FFFFFFFF"

# ------------------------------------------------------------ preview = composite only; Esc drops it
drag {W 30 30} {W 30 30} 1 ctrl
down {W 300 400}
move {W 310 400}
move {W 320 400}
expect #9930 value 1
expect #9941 value 1
cmd 900 {P 305 400}
expect #9900 text "FFFF0000"
cmd 900 {P 315 400}
expect #9900 text "FF0000FF"
cmd 901 {P 305 400}
expect #9900 text "FFFFFFFF"
expect #532738 text "{W 50 30}"
key esc
up {W 320 400}
expect #9930 value 0
expect #9941 value 0
cmd 900 {P 305 400}
expect #9900 text "FFFFFFFF"
expect #9935 value 5

# ------------------------------------------------------------ another layer as the source (DRAW-017):
# anchor on "Hintergrund", the stroke on the new transparent layer
cmd 320
expect #9903 value 2
expect #9904 value 1
expect #9935 value 6
cmd 339 0
expect #9904 value 0
drag {W 30 30} {W 30 30} 1 ctrl
expect #532736 text "30,30,0"
cmd 339 1
expect #9904 value 1
expect #532736 text "30,30,0"
click {W 300 100}
expect #9935 value 7
cmd 901 {P 300 100}
expect #9900 text "FFFF0000"
cmd 901 {P 303 100}
expect #9900 text "00000000"
cmd 900 {P 300 100}
expect #9900 text "FFFF0000"
cmd 339 0
cmd 901 {P 300 100}
expect #9900 text "FFFFFFFF"
# the anchor remembers its layer (index 1 now)
cmd 339 1
drag {W 500 500} {W 500 500} 1 ctrl
expect #532736 text "500,500,1"

# ------------------------------------------------------------ Überschreiben (DRAW-014): transparent
# source pixels of layer 1 replace the white of "Hintergrund"; Normal leaves it white
cmd 339 0
cmd 933 14
click {W 450 100}
expect #532737 text "50,400"
cmd 901 {P 450 100}
expect #9900 text "00000000"
cmd 901 {P 453 100}
expect #9900 text "FFFFFFFF"
cmd 933 0
click {W 470 100}
cmd 901 {P 470 100}
expect #9900 text "FFFFFFFF"
# nothing changed: no history step
expect #9935 value 8

# ------------------------------------------------------------ Füllmethode Unterschied (DRAW-013):
# |white - blue| = yellow
cmd 933 8
drag {W 50 30} {W 50 30} 1 ctrl
expect #532736 text "50,30,0"
click {W 400 100}
cmd 901 {P 400 100}
expect #9900 text "FFFFFF00"
expect #9935 value 9
cmd 933 0

# ------------------------------------------------------------ undo / redo: one step per stroke
key ctrl+z
cmd 901 {P 400 100}
expect #9900 text "FFFFFFFF"
key ctrl+y
cmd 901 {P 400 100}
expect #9900 text "FFFFFF00"

# ------------------------------------------------------------ the test clip rectangle [600,610) x [200,210)
cmd 941 {P 600 200}
cmd 942 {P 610 210}
drag {W 30 30} {W 30 30} 1 ctrl
drag {W 595 205} {W 615 205} 10
expect #9935 value 10
cmd 901 {P 602 205}
expect #9900 text "FFFF0000"
cmd 901 {P 608 205}
expect #9900 text "FF0000FF"
cmd 901 {P 597 205}
expect #9900 text "FFFFFFFF"
cmd 901 {P 612 205}
expect #9900 text "FFFFFFFF"
cmd 942 0

# ------------------------------------------------------------ antialiased, width 9, hardness 100:
# the centre is copied exactly, nothing beyond the radius
cmd 932 1
cmd 931 100
cmd 930 900
drag {W 30 30} {W 30 30} 1 ctrl
click {W 700 300}
expect #9935 value 11
cmd 901 {P 700 300}
expect #9900 text "FFFF0000"
cmd 901 {P 702 302}
expect #9900 text "FFFF0000"
cmd 901 {P 706 300}
expect #9900 text "FFFFFFFF"

# ------------------------------------------------------------ another tool hides the crosshair
key b
expect #9931 value 10
key l
expect #532738 value 1

# ------------------------------------------------------------ no anchor: error text, no stroke
cmd 846 0
expect #532736 value 0
expect #532738 value 0
click {W 300 500}
expect #9930 value 0
expect #9935 value 11
expect #9918 text "Zum Klonen fehlt noch die Quelle. Legen Sie sie zuerst mit Strg+Klick fest."
cmd 901 {P 300 500}
expect #9900 text "FFFFFFFF"
