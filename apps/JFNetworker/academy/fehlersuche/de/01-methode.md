# Erst denken, dann tippen

Wer ein kaputtes Netz vor sich hat, möchte sofort etwas ändern: ein Kabel tauschen, einen Router neu starten, eine Einstellung probieren. Manchmal hilft das sogar. Aber dann weiß man nicht, warum, und beim nächsten Mal steht man wieder am Anfang. Wenn es nicht hilft, hat man womöglich noch einen zweiten Fehler eingebaut.

Dieser Kurs übt das Gegenteil: eine Methode, die auch dann funktioniert, wenn Sie das Netz nicht kennen.

## Was genau geht nicht?

„Das Netz geht nicht“ ist noch keine Beschreibung. Bevor Sie etwas anfassen, klären Sie vier Fragen:

- **Was** geht nicht? Eine bestimmte Seite, alle Seiten, nur der Name oder auch die Adresse?
- **Wer** ist betroffen? Ein Gerät, alle in einem Raum, alle im Haus?
- **Seit wann?** Und was hat sich seitdem geändert? Änderungen sind die häufigste Ursache überhaupt.
- **Was geht noch?** Das wird oft vergessen, und es ist die wertvollste Frage. Was noch geht, schließt ganze Teile des Netzes als Ursache aus.

Prüfen Sie dabei, was man Ihnen sagt. „Das Kabel ist in Ordnung“ heißt oft nur, dass jemand hingesehen hat. Ein Ping dauert Sekunden.

## Vermuten, prüfen, schließen

Die Methode ist eine Schleife:

1. **Vermutung aufstellen:** Was würde dieses Verhalten erklären? Zum Beispiel: PC2 hat ein falsches Gateway.
2. **Vorhersage machen:** Wenn das stimmt, was müsste ich dann sehen? Zum Beispiel: `ip route` auf PC2 zeigt eine andere Adresse als auf PC1.
3. **Prüfen**, und zwar erst lesend, ohne etwas zu ändern.
4. **Schließen:** Stimmt die Vorhersage, ist die Vermutung gestützt. Stimmt sie nicht, ist die Vermutung falsch, und auch das bringt Sie weiter.

Der zweite Schritt ist der wichtigste. Wer vorher überlegt, was er erwartet, merkt, wenn etwas nicht passt. Ohne Vorhersage liest man in jede Ausgabe das hinein, was man ohnehin glaubt.

## Eine Vermutung, die nicht passt, ist kein Fehlschlag

Bleibt die Fehlermeldung nach einer Änderung dieselbe, war die Vermutung falsch, oder es gibt einen zweiten Fehler. Machen Sie die Änderung dann wieder rückgängig, bevor Sie weitersuchen. Sonst vergrößern Sie das Durcheinander, das Sie gerade ordnen wollen.

Ändert sich die Fehlermeldung, ist das eine gute Nachricht, auch wenn es noch nicht geht. Ein `Network is unreachable`, das zu einer Stille wird, heißt: Der Host hat jetzt einen Weg, und das Problem liegt weiter hinten.

Womit fängt man an, wenn alles in Frage kommt? Darum geht es in der [nächsten Lektion](step:schichten).
