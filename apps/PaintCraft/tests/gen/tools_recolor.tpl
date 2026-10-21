# tools_recolor.uis -- Neu färben (agent T6): left = pixels similar to the secondary colour take
# the primary colour (shifted by primary - secondary, shading kept), right = roles swapped,
# "Quelle: erster Klick" (colour under the press point), Toleranz (0 = exact), Toleranz-
# Alpha-Modus Vormultipliziert / Gerade, one step per stroke, undo / redo, Esc, test clip.
# Mapping (1280 x 800, zoom 100 %): image pixel (x, y) at (240 + x, 148 + y).  Probes: 901
# layer / 900 composite pixel (x | y << 16) -> #9900; 902 / 903 / 904 test fills (no step).
# Records (t_clone.s): #532739 "sampling,alpha" (value stroke running), #532740 "OLD,NEW".
# Hooks: 847 sampling (0 einmalig / 1 Sekundärfarbe), 848 alpha mode (0 Vormultipliziert /
# 1 Gerade), 930 width * 100, 932 antialiasing, 935 tolerance, 941 / 942 test clip, 943 / 944
# primary / secondary test colour (3 blue, 5 white, 8 black 50 %).
# Expected pixels (Python reference build/t6/ref_recolor.py = rc_px): match = flood.s
# CheckColor(cmp, px) with cmp / px premultiplied in "Vormultipliziert": sum over R, G, B of
# floor((1 + d^2) * cmp.A / 256) + (cmp.A - px.A)^2 <= tol^2 * 4, tol = round(pct * 2.55);
# a match becomes clamp(px.c + new.c - old.c) per channel, alpha kept.  Aliased width 9 dabs
# cover rows y - 4 .. y + 4 at the pointer column (full coverage: result = recoloured pixel).
size 1280 800
theme light
key r
expect #9931 value 15
expect text:"Pinselgröße:" exists
expect text:"Toleranz:" exists
expect #532739 text "1,0"
expect #532739 value 0
expect #9935 value 1

# ------------------------------------------------------------ row 1 (y 20..39): red, grey, yellow,
# black squares of 20 px from x = 20 on white
cmd 902 1
cmd 903 {P 20 20}
cmd 904 {P 40 40}
cmd 902 7
cmd 903 {P 40 20}
cmd 904 {P 60 40}
cmd 902 9
cmd 903 {P 60 20}
cmd 904 {P 80 40}
cmd 902 6
cmd 903 {P 80 20}
cmd 904 {P 100 40}

# ------------------------------------------------------------ left, Sekundärfarbe (white) -> blue, 50 %
cmd 943 3
cmd 932 0
cmd 930 900
drag {W 10 30} {W 110 30} 20
expect #9930 value 0
expect #9935 value 2
expect #532740 text "FFFFFFFF,FF0000FF"
cmd 901 {P 15 30}
expect #9900 text "FF0000FF"
cmd 901 {P 30 30}
expect #9900 text "FFFF0000"
cmd 901 {P 50 30}
expect #9900 text "FF000080"
cmd 901 {P 70 30}
expect #9900 text "FF000000"
cmd 901 {P 90 30}
expect #9900 text "FF000000"
cmd 901 {P 105 34}
expect #9900 text "FF0000FF"
cmd 901 {P 105 35}
expect #9900 text "FFFFFFFF"
cmd 901 {P 50 35}
expect #9900 text "FF808080"

# ------------------------------------------------------------ undo / redo
key ctrl+z
cmd 901 {P 15 30}
expect #9900 text "FFFFFFFF"
cmd 901 {P 50 30}
expect #9900 text "FF808080"
key ctrl+y
cmd 901 {P 15 30}
expect #9900 text "FF0000FF"

# ------------------------------------------------------------ right button: blue -> white
drag {W 10 30} {W 110 30} 20 right
expect #9935 value 3
expect #532740 text "FF0000FF,FFFFFFFF"
cmd 901 {P 15 30}
expect #9900 text "FFFFFFFF"
cmd 901 {P 30 30}
expect #9900 text "FFFF0000"
cmd 901 {P 50 30}
expect #9900 text "FFFFFF80"
cmd 901 {P 70 30}
expect #9900 text "FFFFFF00"
cmd 901 {P 105 30}
expect #9900 text "FFFFFFFF"

# ------------------------------------------------------------ Quelle: erster Klick -- the stroke
# starts on red: red (and what is close to it) -> blue; row 2 (y 60..79) like row 1
cmd 902 1
cmd 903 {P 20 60}
cmd 904 {P 40 80}
cmd 902 7
cmd 903 {P 40 60}
cmd 904 {P 60 80}
cmd 902 9
cmd 903 {P 60 60}
cmd 904 {P 80 80}
cmd 902 6
cmd 903 {P 80 60}
cmd 904 {P 100 80}
cmd 847 0
expect #532739 text "0,0"
drag {W 30 70} {W 110 70} 20
expect #9935 value 4
expect #532740 text "FFFF0000,FF0000FF"
cmd 901 {P 32 70}
expect #9900 text "FF0000FF"
cmd 901 {P 50 70}
expect #9900 text "FF0080FF"
cmd 901 {P 70 70}
expect #9900 text "FF00FFFF"
cmd 901 {P 90 70}
expect #9900 text "FF0000FF"
cmd 901 {P 105 70}
expect #9900 text "FFFFFFFF"
cmd 901 {P 22 70}
expect #9900 text "FFFF0000"

# ------------------------------------------------------------ Toleranz 0: only the exact colour
cmd 847 1
cmd 935 0
cmd 902 7
cmd 903 {P 40 100}
cmd 904 {P 60 120}
drag {W 10 110} {W 70 110} 12
expect #9935 value 5
cmd 901 {P 15 110}
expect #9900 text "FF0000FF"
cmd 901 {P 50 110}
expect #9900 text "FF808080"
cmd 901 {P 65 110}
expect #9900 text "FF0000FF"

# ------------------------------------------------------------ Alpha-Modus der Toleranz: red 50 % vs the
# secondary colour black 50 % at 30 %: premultiplied they match, straight they do not
cmd 935 30
cmd 944 8
cmd 902 4
cmd 903 {P 20 140}
cmd 904 {P 60 160}
drag {W 25 150} {W 35 150} 4
expect #9935 value 6
expect #532740 text "80000000,FF0000FF"
cmd 901 {P 30 150}
expect #9900 text "80FF00FF"
cmd 848 1
expect #532739 text "1,1"
drag {W 45 150} {W 55 150} 4
cmd 901 {P 50 150}
expect #9900 text "80FF0000"
expect #9935 value 6
cmd 848 0

# ------------------------------------------------------------ the test clip rectangle [200,210) x [195,205)
cmd 944 5
cmd 935 50
cmd 941 {P 200 195}
cmd 942 {P 210 205}
drag {W 190 200} {W 220 200} 10
expect #9935 value 7
cmd 901 {P 205 200}
expect #9900 text "FF0000FF"
cmd 901 {P 195 200}
expect #9900 text "FFFFFFFF"
cmd 901 {P 215 200}
expect #9900 text "FFFFFFFF"
cmd 942 0

# ------------------------------------------------------------ Esc during a stroke: no step
down {W 300 300}
move {W 320 300}
expect #9930 value 1
expect #532739 value 1
cmd 900 {P 310 300}
expect #9900 text "FF0000FF"
cmd 901 {P 310 300}
expect #9900 text "FFFFFFFF"
key esc
up {W 320 300}
expect #9930 value 0
cmd 900 {P 310 300}
expect #9900 text "FFFFFFFF"
expect #9935 value 7

# ------------------------------------------------------------ antialiased width 9: full coverage in the
# centre, nothing beyond the radius
cmd 932 1
cmd 931 100
click {W 400 300}
expect #9935 value 8
cmd 901 {P 400 300}
expect #9900 text "FF0000FF"
cmd 901 {P 406 300}
expect #9900 text "FFFFFFFF"
