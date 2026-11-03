# Eine Änderung, ein Test, eine Notiz

Eine gute Vermutung nützt wenig, wenn Sie danach nicht mehr wissen, was Sie schon getan haben. Darum gehören drei Gewohnheiten zur Methode.

## Eine Änderung auf einmal

Ändern Sie immer nur eine Sache, und testen Sie sofort danach. Wer drei Dinge gleichzeitig ändert und es geht danach, weiß nicht, welches davon geholfen hat, und lässt womöglich zwei unnötige Änderungen im Netz zurück. Geht es danach nicht, weiß er noch weniger.

Das gilt auch dann, wenn Sie sicher sind. Ein Test kostet Sekunden, eine falsche Fährte eine Stunde.

## Immer denselben Test

Legen Sie vorab fest, woran Sie den Erfolg messen, und verwenden Sie immer genau diesen Test: dieselbe Quelle, dasselbe Ziel. Ein Ping vom Router sagt etwas anderes als ein Ping vom PC, wie die vorige Lektion gezeigt hat. Wer zwischendurch einen anderen Test nimmt, vergleicht Äpfel mit Birnen.

Und achten Sie dabei auf Zustände, die nachwirken. Eine ARP-Tabelle merkt sich auch einen falschen Eintrag noch eine Weile, ein Client mit einer laufenden DHCP-Suche holt sich eine neue Adresse nicht sofort. Nach einer Reparatur kann es also einen Moment oder einen neuen Anstoß brauchen. `ip neigh flush all` leert die Tabelle, `dhclient eth0` fragt von neuem.

## Aufschreiben

Notieren Sie, was Sie geprüft und was Sie geändert haben, auch die Vermutungen, die sich als falsch erwiesen haben. Das hilft Ihnen, wenn Sie unterbrochen werden. Es hilft der Kollegin, die übernimmt. Und es hilft Ihnen beim Rückbau: Was keine Wirkung hatte, nehmen Sie wieder zurück.

Das muss nichts Großes sein. Ein paar Zeilen reichen:

```text
10:05  PC1 -> SRV1: Destination Host Unreachable
10:07  ip -br link auf SRV1: eth0 DOWN
10:09  Kabel SW1-SRV1 ersetzt
10:10  PC1 -> SRV1: Antwort. PC2 -> SRV1: Antwort.
```

Am Ende steht dort auch schon die Antwort auf die Frage, was die Ursache war. Das ist nicht dasselbe wie das, was geholfen hat: Ein Neustart hilft oft, ohne dass man die Ursache kennt, und dann kommt der Fehler wieder.

## So läuft eine Fehlersuche in diesem Kurs

Jede Fehlersuche öffnet ein kaputtes Netz. Der Bereich Probleme, die Fehlerliste unter Eigenschaften und die Antwort sind ausgeblendet, denn Sie sollen wie im echten Netz suchen.

- Oben stehen die Klage und was Sie tun sollen. Die Hinweise öffnen sich nacheinander; sie kosten nichts.
- Sie reparieren, wie in echt. Ein gezogenes oder schlechtes Kabel löschen Sie und verbinden die Enden neu. Eine Einstellung ändern Sie in der Konsole.
- Unter **Diagnose** nennen Sie das Gerät oder Kabel, das schuld war, aus der Liste oder mit einem Klick auf die Zeichenfläche. Das zählt erst beim Abgeben.
- **Abgeben** bewertet den Zustand in diesem Moment. Danach sehen Sie die Punkte und eine Erklärung, was los war und wie man es hätte finden können.
- **Neu beginnen** auf der Leiste lädt das Netz wieder im kaputten Zustand. Probieren ist ausdrücklich erlaubt.

Reparieren allein reicht zum Bestehen nicht: Die Diagnose gehört dazu. Eine Reparatur, die man nicht versteht, ist nur Glück.

Beginnen wir mit etwas Einfachem. Das Quiz fragt noch einmal die Methode ab, dann geht es los: [Der Dateiserver ist weg](step:kabel).
