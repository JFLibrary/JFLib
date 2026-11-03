# Kurz erinnert: VLANs

Für die nächste Fehlersuche brauchen Sie VLANs. Ein eigener Kurs behandelt sie später ausführlich; hier das Nötigste.

Ein **VLAN** teilt einen Switch in mehrere logische Switches. Ports im selben VLAN können einander erreichen, Ports in verschiedenen VLANs nicht. Das gilt auch für Broadcasts: Eine ARP-Frage bleibt in ihrem VLAN. Für die Geräte sieht das aus, als hingen sie an getrennten Switches, und für den Verkehr zwischen zwei VLANs braucht es wie zwischen zwei Netzen einen Router.

![Ein Switch mit vier Ports; die zwei linken gehören zu VLAN 10, die zwei rechten zu VLAN 20](img/vlan.svg)

Man benutzt VLANs, um Gruppen zu trennen, ohne für jede einen eigenen Switch zu kaufen: etwa Büro und Technik, oder Gäste und Mitarbeitende. Normalerweise bekommt jedes VLAN auch ein eigenes IP-Netz; im Labor teilen sich die beiden Gruppen ausnahmsweise einen Adressbereich, damit die Trennung allein am VLAN hängt.

## Wie ein Port zu seinem VLAN kommt

Ein **Access-Port** gehört zu genau einem VLAN. Ohne Einstellung ist das VLAN 1. So kommt ein Port in ein anderes:

```console
SW1>enable
SW1#configure terminal
Configuration mode: one command per line; end or Ctrl+Z leaves it.
SW1(config)#interface GigabitEthernet1/0/6
SW1(config-if)#switchport mode access
SW1(config-if)#switchport access vlan 10
SW1(config-if)#end
*Mar  1 00:00:09.000: %SYS-5-CONFIG_I: configuration changed at the console
```

Das Host-Gerät merkt davon nichts. Es hat weder eine Einstellung dafür noch eine Fehlermeldung. Das macht diesen Fehler so unangenehm: Kabel, Adresse und Maske stimmen, alles leuchtet, und trotzdem antwortet niemand.

## Wie man es prüft

`show vlan brief` zeigt jedes VLAN mit seinen Ports:

```console
SW1#show vlan brief

VLAN Name                             Status    Ports
---- -------------------------------- --------- -------------------------------
1    default                          active    Gi1/0/7, Gi1/0/8, Gi1/0/9
                                                Gi1/0/10, Gi1/0/11, Gi1/0/12
                                                Gi1/0/13, Gi1/0/14, Gi1/0/15
                                                Gi1/0/16, Gi1/0/17, Gi1/0/18
                                                Gi1/0/19, Gi1/0/20, Gi1/0/21
                                                Gi1/0/22, Gi1/0/23, Gi1/0/24
                                                Te1/1/1, Te1/1/2, Te1/1/3
                                                Te1/1/4
10   Buero                            active    Gi1/0/1, Gi1/0/3, Gi1/0/6
20   Technik                          active    Gi1/0/2, Gi1/0/4, Gi1/0/5
1002 fddi-default                     act/unsup
1003 token-ring-default               act/unsup
1004 fddinet-default                  act/unsup
1005 trnet-default                    act/unsup
```

Die Zeilen darunter mit den Nummern 1002 bis 1005 sind altes Erbe und können Sie übergehen.

Ein typisches Bild für einen Port im falschen VLAN: Das Gerät kann manche anderen erreichen, nämlich die in seinem falschen VLAN, und ausgerechnet die, die es sucht, nicht. Es hilft, sich zu fragen: Mit wem kann dieses Gerät sprechen? Das Muster verrät das VLAN.

Weiter zur Fehlersuche: [Im falschen VLAN](step:vlan).
