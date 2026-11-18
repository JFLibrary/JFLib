# DXF-Testdateien von ezdxf

Vier kleine Zeichnungen aus dem Projekt [ezdxf](https://github.com/mozman/ezdxf) von Manfred
Moitzi, unter der MIT-Lizenz (`LICENSE-ezdxf.txt`, unverändert aus dem Repository von ezdxf).
`render2dtest --test dxf` bettet sie ein (`dxftest.rs`, `include_bytes!`) und prüft an ihnen,
was die selbst geschriebenen Zeichnungen des Tests nicht zeigen. Die Dateien sind
Byte für Byte die der Quelle; `.gitattributes` schließt die Umwandlung der Zeilenenden aus.

| Datei | Größe | Quelle | Git-Blob | Was der Test prüft |
| --- | --- | --- | --- | --- |
| `clockwise_arcs_hatch.dxf` | 19 257 | ezdxf `examples_dxf/` @ `a5a2eb642f8b` | `116a94f4d5c4` | Bögen im Uhrzeigersinn im Rand einer Schraffur: Die Kanten jedes Pfads schließen sich nur mit den gespiegelt gespeicherten Winkeln zu einem Ring |
| `caret_encoding.dxf` | 21 825 | ezdxf `examples_dxf/` @ `a5a2eb642f8b` | `72a06ad9131b` | Caret-Codes in MTEXT: `^J` neuer Absatz, `^I` Tabulator, `^ ` ein Caret |
| `mtext_complex_inline_codes.dxf` | 166 396 | ezdxf `examples_dxf/` @ `a5a2eb642f8b` | `32f4a239e141` | MTEXT mit vielen Formatcodes und vier Spiegelungen: alles gezeichnet, GPU gegen CPU-Weg |
| `bin_dxf_r12.dxf` | 3 761 | GDAL `autotest/ogr/data/dxf/` @ `1fe1fe6b8a48`, dort laut README aus ezdxfs `integration_tests/data` | `d30f0b6430ec` | ein binäres DXF: `Error::Binary` |

Die Commits sind die vollen Hashes `a5a2eb642f8b2bac31414f9a06bed398d276883b` (ezdxf) und
`1fe1fe6b8a48f1213573b7e6c6d954e4b6fb031f` (GDAL), geladen am 16.11.2026. Die übrigen Dateien des
Testkorpus (GDAL, ezdxf, dxf-parser, QCAD, LibreCAD, die amtliche Vermessung von Zürich) liegen
außerhalb des Repositorys; `--test dxfcorpus` liest sie, wenn es sie gibt (docs/RENDER2D.md,
Abschnitt DXF).
