# red-table

`red-table` ist ein konsequent auf Performance ausgelegter Terminal-Bildbrowser
für große Bildsammlungen. Das durchsuchbare und scrollbare Thumbnail-Raster wird
vollständig per Tastatur bedient, sowohl mit den Pfeiltasten als auch mit den
Vim-Tasten `h`, `j`, `k` und `l`.

Die Anwendung ist in Rust implementiert und wird mit reproduzierbaren
Nix-Build-Metadaten ausgeliefert.

## Kurz-Demo

[Die 19-sekündige Terminal-Demo auf YouTube ansehen](https://youtu.be/rcg16ZPUYgA).

Die echte Kitty-Sitzung beginnt an einer neutralen Eingabeaufforderung, startet
`red-table`, öffnet das scharfe Raster mit acht Bildern, markiert ein Bild und
prüft es in der Großansicht. Danach kehrt sie exakt zur vorherigen Rasterposition
zurück und endet im zweigeteilten A/B-Vergleich. Die acht neutral benannten
Beispielbilder aus der Aufnahme liegen unter [`media/showcase`](media/showcase/)
und können direkt lokal ausprobiert werden.

## Entwicklungsumgebung

Nix und direnv sind die einzigen Bootstrap-Voraussetzungen. Rust, Cargo, Task,
Git, Formatter, Linter und MkDocs kommen vollständig aus der gesperrten Flake.

```sh
direnv allow
task check
```

Ohne direnv lässt sich dieselbe Umgebung explizit starten:

```sh
nix develop
task check
```

Der reproduzierbare Build und Programmstart erfolgen mit:

```sh
nix build
nix run . -- /pfad/zu/bildern
```

Eine bevorzugte Thumbnail-Größe kann beim Start angegeben werden:

```sh
nix run . -- --thumbnail-size 40x18 /pfad/zu/bildern
```

Die Startqualität lässt sich von `1` (schnell) bis `9` (maximale
Detailverstärkung) wählen:

```sh
nix run . -- --quality 9 /pfad/zu/bildern
```

Das Terminal-Grafikprotokoll wird automatisch erkannt. Zur Diagnose oder als
gezielte Vorgabe kann es explizit gewählt werden:

```sh
nix run . -- --graphics-protocol kitty /pfad/zu/bildern
```

Benutzervorgaben und Tastenbelegungen können über eine streng geprüfte
TOML-Datei unter `$XDG_CONFIG_HOME/red-table/config.toml` konfiguriert werden.
`--config PFAD` verlangt eine andere Datei, `--no-config` startet ausschließlich
mit den eingebauten Werten.

`task` zeigt alle verfügbaren Entwicklungsbefehle an.

## Bedienung

- Pfeiltasten oder `h`, `j`, `k`, `l`: Auswahl bewegen
- Enter: fokussiertes Bild groß anzeigen; Escape kehrt exakt zum Raster zurück
- `c`: fokussierte Referenz zweigeteilt mit Kandidaten vergleichen
- `s`: Fundreihenfolge / Pfadsortierung umschalten; Start in stabiler Fundreihenfolge
- Bild hoch, Bild runter, Pos1, Ende: größere Sprünge
- `+` / `-`: Thumbnails vergrößern oder verkleinern; `0` setzt auf `32x14` zurück
- `1` bis `9`: Thumbnail-Qualität sofort umschalten; Standard ist `7`
- `/`: Dateinamen und relative Pfade durchsuchen; Enter übernimmt, Escape bricht ab
- `?`: aus der aktiven Belegung erzeugte Hilfe anzeigen
- Leertaste: fokussiertes Bild markieren oder Markierung entfernen; sichtbar als `[x]` und Doppelrahmen
- `v`: Bereich ab dem letzten Leertasten-Anker einschließlich markieren
- `u` / `m`: Markierungen löschen / nur markierte Bilder anzeigen
- Strg+s: Pfade im Modus `--select` bestätigen
- `F12`: Renderer-, Qualitäts-, Queue- und Cache-Diagnose umschalten
- `q`: beenden; Escape schließt nur die aktive Unteransicht oder bricht sie ab

In der Großansicht wechseln `p` / `n` zum vorherigen / nächsten Treffer. `z`
schaltet zwischen Einpassen und exakten 100 Prozent um, `+` / `-` durchlaufen
die begrenzten Zoomstufen von 25 bis 800 Prozent. Pfeile beziehungsweise
`h`, `j`, `k`, `l` verschieben den Ausschnitt; `b` wechselt den Hintergrund für
Transparenz zwischen Schachbrett, dunkel und hell. EXIF-Ausrichtung wird vor der
Darstellung angewendet; Decoding und Ausschnittberechnung blockieren die Eingabe
nicht.

Im A/B-Vergleich bleibt die linke `REFERENZ` fest, während `p` / `n` den rechten
`KANDIDATEN` wechseln. Enter macht den Kandidaten zur Referenz und setzt den
Vergleich mit einem weiteren Bild fort. Tab wählt die aktive Seite; die Leertaste
markiert deren Bild. `s` schaltet zwischen gekoppeltem und unabhängigem Zoom und
Ausschnitt um. Gekoppelt bleiben normalisierte Mitte und Zoomstufe auch bei
unterschiedlichen Bildmaßen gleich. Escape führt zurück zum Raster.

Als visueller Dateifilter lässt sich red-table so verwenden:

```sh
nix run . -- --select --print0 /pfad/zu/bildern > auswahl.paths0
```

Die Oberfläche schreibt ausschließlich nach `/dev/tty`; nur bestätigte absolute
Pfade landen auf stdout. Die NUL-Trennung bewahrt beliebige Unix-Dateinamen und
eignet sich für `xargs -0`. Ohne `--print0` wird zur besseren Lesbarkeit ein Pfad
je Zeile ausgegeben. `q` bricht ohne Ausgabe mit Status 2 ab, Strg+s bestätigt
auch eine leere Auswahl erfolgreich. Eine NUL-getrennte
Dateiliste kann anstelle eines Ordners übergeben werden:

```sh
find ./bilder -type f -print0 |
  nix run . -- --select --print0 --files0-from=- > auswahl.paths0
```

Damit stdout sauber bleibt, überspringt `--select` die aktive, Escape-Sequenzen
sendende Protokollabfrage. Abgesicherte Terminal-Umgebungsmerkmale erkennen
direktes Kitty/iTerm2 weiterhin; bei Bedarf setzt `--graphics-protocol` den
Renderer ausdrücklich.

## Yazi-Plugin

Das installierbare funktionale Plugin [`red-table.yazi`](red-table.yazi/) öffnet
Yazis aktuelles echtes Verzeichnis mit `red-table --select --print0`, übergibt
red-table vorübergehend das Terminal und ersetzt Yazis Auswahl erst nach einer
erfolgreichen Bestätigung. Abbruchstatus 2 lässt Yazi unverändert. Die Brücke
benötigt Unix und Yazi ab Version 26.5.6; `red-table` muss in `PATH` liegen oder
in Yazis `init.lua` als absoluter Programmpfad konfiguriert sein.

Nach der Plugin-Installation lässt es sich beispielsweise so belegen:

```toml
[[mgr.prepend_keymap]]
on   = [ "g", "i" ]
run  = "plugin red-table"
desc = "Bilder mit red-table auswählen"
```

Die [Plugin-Anleitung](red-table.yazi/README.md) beschreibt Installation mit
`ya pkg`, lokale Checkouts, Home Manager und die Programmkonfiguration. Der
Flake stellt das Plugin zusätzlich separat als `packages.yazi-plugin` bereit.

Erlaubt sind Startgrößen zwischen `12x6` und `120x60` Terminalzellen. JPEG-,
PNG-, GIF-, WebP-, TIFF- und BMP-Dateien werden rekursiv gefunden. Das
Decoding läuft nur für den sichtbaren Bereich und eine Prefetch-Zeile in
begrenzten Hintergrund-Workern. Im Leerlauf zeichnet die Oberfläche nicht
fortlaufend neu.

Unter Linux speichert `red-table` protokollneutrale Vorschaubilder im gemeinsamen
freedesktop.org-Cache unter `$XDG_CACHE_HOME/thumbnails`, ersatzweise unter
`$HOME/.cache/thumbnails`. `D<Treffer>/<Misses>` in der Statuszeile zeigt
die abgeschlossenen Disk-Zugriffe der laufenden Sitzung. URI, Dateigröße,
Änderungszeit mit Sekunden und Nanosekunden sowie die Verarbeitungsversion
sichern die Gültigkeit ab. Terminalprotokoll, Zellgeometrie und Q1–Q9 bleiben im
begrenzten RAM-Cache.

Die Qualitätsstufen tauschen Rechenzeit gegen Kantenschärfe: `1` ist am
schnellsten, `3` entspricht dem bisherigen Catmull-Rom-Verfahren, und `4` bis
`9` verwenden Lanczos3 mit zunehmend stärkerer, begrenzter Detailverstärkung.
Die beste sichtbare Qualität liefern Terminals mit Kitty-, Sixel- oder
iTerm2-Grafik. Der portable Status `Half 1x2` bedeutet, dass pro Terminalzelle
physisch nur ein horizontaler und zwei vertikale Farbpunkte darstellbar sind. In
diesem Modus hilft `+`, die Thumbnails weiter zu vergrößern.
Innerhalb dieser Grenze steuern Q1–Q9 jetzt direkt die endgültige
Halfblocks-Abtastung und -Schärfung statt nur ein Zwischenbild.
Der Renderer-Status nennt auch die Quelle der Auswahl, beispielsweise
`Kitty/auto`, `Kitty/env`, `Kitty/forced` oder `Half 1x2/auto`. Ein erzwungenes
Protokoll muss vom verwendeten Terminal unterstützt werden.

Der Kitty-Pfad verwendet eine vollständige virtuelle Unicode-Platzierung mit
expliziter Zellfläche. Verlässt ein Bild den sichtbaren Bereich und kehrt zurück,
wird es erneut übertragen; verworfene Bild-IDs werden aus dem Terminalspeicher
entfernt. In tmux werden die Befehle anhand von `TMUX`, `TERM` oder
`TERM_PROGRAM` gekapselt. Das externe tmux-Passthrough muss trotzdem eingerichtet
sein. Falls ein Terminal weiterhin leere oder schwarze
Bildbereiche zeigt, dient `red-table --graphics-protocol halfblocks PFAD` als
sichere Ausweichlösung; bei einem Fehlerbericht sollte der Statusname angegeben
werden. Decode-Fehler verlassen `loading…` und erscheinen als begrenzte
Fehlerkacheln mit ihrem Grund.

Der RAM-Rendercache ist gleichzeitig auf 256 Einträge und 128 MiB angerechnete
Protokolldaten begrenzt. Die technische Statusanzeige nennt ihn als
`M<Einträge>/<MiB>`.
