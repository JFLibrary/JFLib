# Shape-Daten jedes Geräts (JD13): Inventar und Standort.  Die Zeilennamen sind Visios Namen
# (Kompatibilität mit Berichten und Legenden), Bezeichnungen und Werte sind eigene.
# ShapeClass/ShapeType/SubShapeType sind verborgen und ordnen das Gerät für Berichte ein; der
# Master setzt ShapeType und SubShapeType selbst.
[User]
# Visios Kennung für Shapes, die eine Legende zählt (JD13: die Legende in Netzwerk und Peripheriegeräte)
visLegendShape: Value = 2
[Property]
AssetNumber:  Type = 0; Value = ""
              Label.de = Inventarnummer
              Label.en = Asset number
              Label.es = Número de inventario
              Label.it = Numero di inventario
              Label.fr = Numéro d'inventaire
SerialNumber: Type = 0; Value = ""
              Label.de = Seriennummer
              Label.en = Serial number
              Label.es = Número de serie
              Label.it = Numero di serie
              Label.fr = Numéro de série
Location:     Type = 0; Value = ""
              Label.de = Standort
              Label.en = Location
              Label.es = Ubicación
              Label.it = Sede
              Label.fr = Site
Building:     Type = 0; Value = ""
              Label.de = Gebäude
              Label.en = Building
              Label.es = Edificio
              Label.it = Edificio
              Label.fr = Bâtiment
Room:         Type = 0; Value = ""
              Label.de = Raum
              Label.en = Room
              Label.es = Sala
              Label.it = Stanza
              Label.fr = Salle
Manufacturer: Type = 0; Value = ""
              Label.de = Hersteller
              Label.en = Manufacturer
              Label.es = Fabricante
              Label.it = Produttore
              Label.fr = Fabricant
ProductNumber: Type = 0; Value = ""
              Label.de = Produktnummer
              Label.en = Product number
              Label.es = Número de producto
              Label.it = Codice prodotto
              Label.fr = Référence produit
PartNumber:   Type = 0; Value = ""
              Label.de = Teilenummer
              Label.en = Part number
              Label.es = Número de pieza
              Label.it = Codice componente
              Label.fr = Référence pièce
ProductDescription: Type = 0; Value = ""
              Label.de = Produktbeschreibung
              Label.en = Product description
              Label.es = Descripción del producto
              Label.it = Descrizione prodotto
              Label.fr = Description du produit
ShapeClass:   Type = 0; Invisible = TRUE; Label = "ShapeClass"
              Value.de = Geräte
              Value.en = Equipment
              Value.es = Equipos
              Value.it = Apparecchiature
              Value.fr = Équipements
