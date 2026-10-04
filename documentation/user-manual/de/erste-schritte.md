# Erste Schritte

Ein Verzeichnis wird rekursiv geöffnet mit:

```sh
nix run . -- /pfad/zu/bildern
```

Die Standard-Zielgröße beträgt `32x14` Terminalzellen je Thumbnail. Sie lässt
sich beim Start überschreiben:

```sh
nix run . -- --thumbnail-size 40x18 /pfad/zu/bildern
```

Erlaubt sind `12x6` bis `120x60`. Es handelt sich um Zielwerte; das Raster
verteilt den verbleibenden Platz gleichmäßig.

Verzeichnis-Scans zeigen Bilder byteweise aufsteigend nach ihrem Pfad relativ
zum gescannten Verzeichnis, also in der Reihenfolge von `ls`. Bilder, die ein
laufender Scan später findet, werden in diese Reihenfolge einsortiert. Eine
Liste über `--files0-from` behält ihre Eingabereihenfolge.

Die Qualitätsstufe ist standardmäßig `7`. Beim Start kann sie von `1` (schnell)
bis `9` (maximale Detailverstärkung) gesetzt werden:

```sh
nix run . -- --quality 9 /pfad/zu/bildern
```

Das Grafikprotokoll wird standardmäßig automatisch ausgehandelt. Für Diagnose
oder Wiederherstellung kann `auto`, `kitty`, `sixel`, `iterm2` oder `halfblocks`
explizit vorgegeben werden:

```sh
nix run . -- --graphics-protocol kitty /pfad/zu/bildern
```

Ohne Pfad öffnet `red-table` das aktuelle Verzeichnis. JPEG-, PNG-, GIF-, WebP-,
TIFF- und BMP-Dateien erscheinen inkrementell, ohne dass ein großes Verzeichnis
zuerst vollständig gescannt werden muss.

## Konfiguration

Eine optionale, streng geprüfte TOML-Konfiguration wird unter
`$XDG_CONFIG_HOME/red-table/config.toml` gesucht, ersatzweise unter
`$HOME/.config/red-table/config.toml`. `--config PFAD` verlangt eine bestimmte
Datei, `--no-config` unterbindet die Suche. CLI-Angaben haben Vorrang. Die
[Konfigurationsreferenz](../../references/configuration.md) beschreibt das
vollständige Schema der Version 1 und alle Aktionsnamen.

## Tastatursteuerung

| Taste | Aktion |
| --- | --- |
| Pfeiltasten oder `h`, `j`, `k`, `l` | Auswahl bewegen |
| Bild hoch / Bild runter | eine Ansicht springen |
| Pos1 / Ende | ersten / letzten Treffer auswählen |
| Enter | fokussiertes Bild als Großansicht öffnen |
| `c` | fokussiertes Bild mit Kandidaten vergleichen |
| Leertaste | dauerhafte `[x]`-Markierung und Doppelrahmen umschalten |
| `v` | Bereich ab dem letzten Leertasten-Anker einschließlich markieren |
| `u` / `m` | alle Markierungen löschen / nur markierte Bilder zeigen |
| Strg+s | Markierungen in einer `--select`-Sitzung bestätigen |
| `+` / `-` | Thumbnail-Zellen vergrößern / verkleinern |
| `0` | Standardgröße `32x14` wiederherstellen |
| `1` bis `9` | Thumbnail-Qualität sofort umschalten |
| `/` | Suchfilter bearbeiten |
| `?` | kontextbezogene Hilfe aus der aktiven Belegung anzeigen |
| F12 | technische Renderer-, Qualitäts-, Queue- und Cache-Daten umschalten |
| Enter während der Suche | aktuellen Filter übernehmen |
| Escape während der Suche | vorherigen Filter wiederherstellen |
| `q` | beenden und Terminal wiederherstellen |

## Großansicht

Escape führt aus der Großansicht zur unveränderten fokussierten Kachel und
Scrollposition zurück. `p` und `n` wechseln innerhalb der aktuellen gefilterten
Reihenfolge zum vorherigen oder nächsten Bild. `z` schaltet direkt zwischen
vollständigem Einpassen und exakten 100 Prozent Renderer-Pixeln um; `+` und `-`
durchlaufen 25, 50, 100, 200, 400 und 800 Prozent. Bei festem Zoom verschieben
Pfeile oder `h`, `j`, `k`, `l` den Ausschnitt in begrenzten Schritten. Mit `b`
lässt sich Transparenz vor Schachbrett, dunklem oder hellem Hintergrund prüfen.

EXIF-Ausrichtung gilt einheitlich für Thumbnails und Großansicht. Zu große oder
beschädigte Quellen erscheinen als behebbare Fehleransicht. Decoding, Zoom und
Verschieben laufen in einer eigenen begrenzten Hintergrund-Pipeline; schnelle,
bereits überholte Eingaben werden verworfen und blockieren die Bedienung nicht.

## A/B-Vergleich

`c` öffnet bei mindestens zwei sichtbaren Bildern den Vergleich. Das fokussierte
Bild wird zur festen linken `REFERENZ`; rechts beginnt der `KANDIDAT` beim nächsten
sichtbaren Bild, am Listenende beim vorherigen. `p` und `n` durchlaufen die
Kandidaten und überspringen stets die Referenz. Enter macht den Kandidaten zur
Referenz und setzt den Vergleich mit einem gültigen Kandidaten fort. Bei genau
zwei Bildern wird die bisherige Referenz zum neuen Kandidaten.

Der rote Rahmen kennzeichnet zusätzlich zur ausgeschriebenen Rolle die aktive
Seite. Tab wechselt die aktive Seite, die Leertaste deren `[x]`-Markierung. `s`
schaltet zwischen gekoppelter und unabhängiger Prüfung um. Gekoppelt werden
Einpassen, Zoom und normalisierter Ausschnitt auf beide Seiten übertragen; beim
Einschalten wird der Zustand der aktiven Seite übernommen. Unabhängig verändern
`z`, `+`, `-` und die Verschiebetasten nur die aktive Seite. Beide Bilder laden
und scheitern unabhängig, teilen aber dasselbe konfigurierte Decode-Speicherlimit.
Escape kehrt mit dem Kandidaten als Fokus zum Raster zurück.

## Visuelle Dateiauswahl

Eine Ergebnissitzung startet mit `--select`. Der Fokus bleibt der rote, fette
Rahmen; die Auswahl erscheint als gelber Doppelrahmen und zusätzlich immer als
`[x]`. Eine fokussierte Markierung hat somit roten, fetten Doppelrahmen und
`[x]`. Beide Zustände sind auch ohne Farberkennung eindeutig. Markierungen
bleiben bei Suche, der Ansicht „nur markierte“ und der Navigation in der
Großansicht erhalten. Die Leertaste setzt zugleich den Anker für `v`; neue Pfade
behalten diese Interaktionsreihenfolge.

Strg+s stellt das Terminal wieder her und schreibt die bestätigten vollständigen
Pfade nach stdout. `q` bricht ohne Ausgabe mit Exitstatus 2 ab; Escape hat im
Raster keine Wirkung. Eine Bestätigung ohne Markierungen ist eine erfolgreiche
leere Ausgabe. Für beliebige Unix-Namen einschließlich Zeilenumbrüchen und
Nicht-UTF-8-Bytes dient `--print0`:

```sh
red-table --select --print0 ./bilder > auswahl.paths0
```

Die Oberfläche verwendet in diesem Modus `/dev/tty`, niemals stdout. Mit
`--files0-from=-` kann vor dem Öffnen eine NUL-getrennte Liste von stdin gelesen
werden. Relative Pfade beziehen sich auf das aktuelle Verzeichnis, kanonische
Duplikate erscheinen einmal. Die Eingabe ist bewusst auf 1 MiB je Datensatz,
64 MiB insgesamt und 1.000.000 Datensätze begrenzt.

Der Auswahlmodus überspringt bewusst die aktive Protokollabfrage, weil die
aktuelle Renderer-Bibliothek deren Escape-Sequenzen fest nach stdout schreibt.
Abgesicherte Umgebungserkennung und eine ausdrückliche Wahl mit
`--graphics-protocol` bleiben verfügbar; normales Browsen behält die vollständige
aktive Aushandlung.

## Yazi-Integration

Das Repository enthält das installierbare funktionale Plugin `red-table.yazi`
für Unix und Yazi ab Version 26.5.6. Es öffnet Yazis aktuelles echtes
Verzeichnis, übergibt red-table vorübergehend das Terminal und erfasst nur die
bestätigten NUL-getrennten Pfade. Eine Bestätigung ersetzt Yazis Auswahl und
fokussiert das erste Ergebnis; eine bestätigte leere Ausgabe leert die Auswahl.
Abbruch mit `q` und jeder Fehler lassen die bestehende Auswahl unverändert.

Nach der Plugin-Installation wird beispielsweise folgende Taste ergänzt:

```toml
[[mgr.prepend_keymap]]
on   = [ "g", "i" ]
run  = "plugin red-table"
desc = "Bilder mit red-table auswählen"
```

Standardmäßig wird `red-table` in `PATH` verwendet. Ein absoluter Pfad,
einschließlich eines Nix-Store-Pfades, lässt sich in
`~/.config/yazi/init.lua` konfigurieren:

```lua
require("red-table"):setup({
  command = "/absoluter/pfad/zu/red-table",
})
```

Die README des Plugin-Pakets beschreibt die Installation mit `ya pkg`, aus
einem lokalen Checkout und mit Home Manager. Virtuelle Yazi-Verzeichnisse
werden bewusst abgewiesen; vor dem Aufruf muss ein echtes Verzeichnis geöffnet
sein.

Mit Ausnahme des festen Notabbruchs `Ctrl+c` können alle genannten Belegungen in
TOML ersetzt werden. Standardmäßig bleibt die Statuszeile knapp; technische
Felder erscheinen nur in der Debugansicht.

Die Suche ignoriert Groß-/Kleinschreibung und berücksichtigt Dateinamen sowie
Pfade relativ zum geöffneten Verzeichnis. Nicht dekodierbare oder beschädigte
Bilder verlassen den Ladezustand und zeigen eine Fehlerkachel mit dem Grund; sie
beenden das Programm nicht.

Je nach Terminal verwendet `red-table` Kitty, Sixel oder iTerm2. Wenn kein
Grafikprotokoll verfügbar ist, werden farbige Unicode-Halbblöcke verwendet. Die
Stufen `1` bis `9` tauschen Aufbereitungszeit gegen Lanczos-Skalierung und
zunehmend stärkere Kantenschärfung. Die Statuszeile zeigt die aktuelle Stufe als
`Qn` an.

Der Protokollname zeigt zusätzlich die Auswahlquelle: `/auto` bezeichnet die
aktive Aushandlung, `/env` den abgesicherten direkten Kitty-Ersatzpfad und
`/forced` eine explizite CLI-Vorgabe. Ein erzwungenes, vom Terminal nicht
unterstütztes Protokoll kann wirkungslose Escape-Sequenzen ausgeben. Innerhalb
von tmux bleibt der Umgebungs-Fallback bewusst deaktiviert; dort darf die
explizite Vorgabe erst nach Einrichtung des Passthroughs verwendet werden.
Bei erzwungenem Kitty kapselt red-table die Grafikbefehle, sobald `TMUX` gesetzt
ist, `TERM` mit `tmux` beginnt oder `TERM_PROGRAM=tmux` gilt. Dadurch wird das
tmux-Passthrough selbst nicht aktiviert.

Das Kitty-Backend überträgt RGBA-Pixel in eine virtuelle Unicode-Platzierung mit
expliziter Zeilen- und Spaltenzahl. Die 297 normativen Platzhalterzeilen werden
unterstützt; größere Bildflächen ergeben einen erklärenden Fehler statt einer
stillen Beschneidung. Bilddaten werden als kurzlebig markiert, nach dem Verlassen
und erneuten Öffnen einer Ansicht erneut übertragen und beim Verwerfen ihres
Cache-Objekts ausdrücklich aus dem Terminal entfernt. Falls dieser Pfad in einem inkompatiblen
Terminal leere oder schwarze Bereiche erzeugt, kann red-table mit
`red-table --graphics-protocol halfblocks PFAD` neu gestartet werden. Zeigt auch
Halfblocks eine Fehlerkachel, verweist der dort genannte Decode-Grund auf die
Quelldatei oder das Format statt auf den Terminal-Renderer. Für einen
Fehlerbericht sollte der ursprüngliche Protokollname mit angegeben werden.

Der Status `Half 1x2/auto` weist auf die physische Auflösungsgrenze dieses
Fallbacks hin: Eine Terminalzelle kann nur einen horizontalen und zwei vertikale
Farbpunkte darstellen. Das ist kein Fehler des Quellbilds oder Caches. Für mehr
Details können die Thumbnails mit `+` vergrößert oder ein Terminal mit Kitty-,
Sixel- oder iTerm2-Unterstützung verwendet werden. Die Qualitätsstufen verändern
Skalierung und Kantenschärfung direkt auf diesem endgültigen Farbraster, können
aber keine zusätzlichen Farbpunkte erzeugen.

Beim Qualitätswechsel bleibt ein bereits berechnetes Thumbnail nur sichtbar,
solange seine neue Variante noch berechnet wird. Ein fehlgeschlagener Versuch zeigt
anschließend eine Fehlerkachel. Der Zähler `load` zeigt laufende Ersetzungen.

## Dauerhafte Thumbnails unter Linux

Protokollneutrale PNG-Vorschaubilder liegen im gemeinsamen freedesktop.org-Cache
unter `$XDG_CACHE_HOME/thumbnails`, ersatzweise unter
`$HOME/.cache/thumbnails`. `D<Treffer>/<Misses>` in der Statuszeile zählt
die abgeschlossenen persistenten Cache-Ergebnisse der laufenden Sitzung. Ein
Treffer vermeidet das Decoding des Originals; terminal- und qualitätsspezifische
Varianten bleiben in einem auf 256 Einträge und 128 MiB angerechnete Daten
begrenzten RAM-Cache. Die technische Statuszeile zeigt ihn als
`M<Einträge>/<MiB>`. Ein Thumbnail-Ziel oberhalb von 64 MiB roher RGBA-Daten wird
vor der Speicherbelegung abgelehnt.

URI, Dateigröße, Änderungszeit und die red-table-Verarbeitungsversion prüfen die
Gültigkeit. Geänderte oder beschädigte Einträge werden automatisch neu erzeugt.
Der Cache wird mit anderen Desktopprogrammen geteilt: Seine Größenverzeichnisse
können gefahrlos gelöscht werden, dadurch verschwinden aber auch deren jederzeit
neu erzeugbare Vorschaubilder.
