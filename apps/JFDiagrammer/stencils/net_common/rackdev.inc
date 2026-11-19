# Gestellmontierte Geräte (JD13, VL-52): 19-Zoll-Einschübe nach EIA-310 in Frontansicht, Maßstab
# 1:20 (der Master nennt scale: 1:20 und size:).  Eine Höheneinheit (HE, User.U) ist 1,75 Zoll =
# 44,45 mm, die Frontplatte 482,6 mm breit.
#
# Die Höhe folgt den Höheneinheiten (Prop.RackUnits): Zieht man das Shape höher oder niedriger,
# schickt SETATREF die gerundete Zahl der HE in die Shape-Daten, die Höhe rastet auf ganze HE ein.
# Der Verbindungspunkt unten links ist „nach außen“ (Type 1): Er rastet an den Punkten des
# Gestells ein, die auf jeder HE-Grenze liegen (in JFDiagrammer und in Visio).
# User.msvShapeCategories macht das Gerät zum Mitglied, das ein Gestell aufnimmt (Container).
[Shape]
Width = 482.6 mm
Width.us = 19 in
Height = SETATREF(Prop.RackUnits,MAX(1,MIN(52,ROUND(SETATREFEXPR()/User.U,0))))*User.U
LockWidth = TRUE
TxtWidth = 1200 mm
TxtWidth.us = 48 in
TxtHeight = Height
TxtPinX = Width+60 mm
TxtPinX.us = Width+2.4 in
TxtPinY = Height*0.5
TxtLocPinX = 0 mm
TxtLocPinY = TxtHeight*0.5
VerticalAlign = 1
[User]
U:            Value = 44.45 mm
              Value.us = 1.75 in
msvShapeCategories: Value = "RackEquipment"
[Property]
RackUnits:    Type = 2; Value = 1
              Label.de = Höheneinheiten (HE)
              Label.en = Rack units (U)
              Label.es = Unidades de rack (U)
              Label.it = Unità rack (U)
              Label.fr = Unités de baie (U)
[Connection]
Row  X = 0 mm; Y = 0 mm; DirX = 0; DirY = 0; Type = 1
[Character]
Row  Size = 6 pt
[Paragraph]
Row  HorzAlign = 0
