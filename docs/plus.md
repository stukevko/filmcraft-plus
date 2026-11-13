# FilmCraft Plus

Eine angepasste Version von [FilmCraft](https://github.com/storytold/filmcraft) mit drei neuen
Effekten und einem Build für **Intel Core Ultra 7 270K Plus + AMD Radeon RX 5700** unter Windows 11
(NVIDIA- und Intel-Grafik funktionieren genauso).

## Installieren

1. Auf GitHub unter **Releases ▸ plus-latest** die Datei `FilmCraft-Plus-windows-x64.zip` laden.
2. Entpacken, `FilmCraft Plus.exe` starten. Windows SmartScreen fragt beim ersten Start nach
   („Weitere Informationen ▸ Trotzdem ausführen“), weil der Build nicht signiert ist.
3. Sprache: **Bearbeiten ▸ Sprache ▸ Deutsch** (falls nicht automatisch).

### macOS (gleicher PC, Intel + RX 5700)

1. Unter **Releases ▸ plus-latest** die Datei `FilmCraft-Plus-macos-intel.dmg` laden.
2. DMG öffnen, **FilmCraft** in **Programme** ziehen.
3. Die App ist nicht von Apple notarisiert. Beim ersten Start meldet macOS das: dann
   **Systemeinstellungen ▸ Datenschutz & Sicherheit ▸ „Dennoch öffnen“** (ältere macOS-Versionen:
   Rechtsklick auf die App ▸ Öffnen). Meldet macOS „beschädigt“, einmal im Terminal:
   `xattr -dr com.apple.quarantine /Applications/FilmCraft.app`

Alle Plus-Effekte, Beat-Marker, Musik-Sync, YouTube-Vorgaben und Auto-Untertitel gibt es auch
auf dem Mac. Vorschau und Effekte laufen über **Metal** auf der RX 5700, der Export über Apples
**VideoToolbox** (siehe „Optimierungen“ unten).

## Die neuen Effekte

Alle drei liegen im Effekte-Fenster ganz oben im Ordner **FilmCraft Plus**. Effekt auf einen
Clip ziehen (oder Clip markieren und doppelklicken), dann in den **Effekteinstellungen** eine
**Vorgabe** wählen. Fertig – mehr ist nicht nötig. Ein Regler (**Intensität / Stärke**) macht den
Effekt schwächer oder stärker; alle Regler lassen sich wie gewohnt mit Keyframes animieren.

### Organisches Wackeln (Organic Shake)

Natürliches Kamerawackeln mit Bewegungsunschärfe, Z-Wackeln (Zoom) und Drehung. Die Ränder
werden automatisch verdeckt („Ränder verbergen“), damit nie schwarze Kanten zu sehen sind.

| Vorgabe | Wofür |
|---|---|
| Handkamera | ruhige Handheld-Optik, Interviews, Vlogs |
| Gehen / Laufen | Kamera läuft mit |
| Autofahrt | feines, schnelles Rütteln |
| Erdbeben | heftiges Dauerbeben |
| Einschlag | harter Stoß, der in 1–2 s abklingt – an den Moment des Treffers legen |
| Drohnen-Drift | langsames Schweben |
| Herzschlag | pulsierender Zoom im Takt |

„Eigene“ schaltet die Regler unter **Eigenes Wackeln** frei. **Geschwindigkeit** macht jede
Vorgabe schneller/langsamer. **Abklingzeit** > 0 lässt das Wackeln ab Clip-Anfang ausklingen.

### Flacker-Stapel (Flicker Stack)

Rhythmische Zuckungen von Unschärfe, Farbe, Licht, Skalierung, Verschieben, Farbversatz und
Zeit – ideal für Musikvideos, Trailer, Übergänge und Glitch-Momente.

| Vorgabe | Wofür |
|---|---|
| Dezent | leichtes Leben im Bild |
| Musikvideo | alles an, im Beat-Tempo |
| Glitch-Treffer | schnelle digitale Störungen mit Zeitsprüngen |
| Horror | Lichtblitze, Ruckler, Zeitsprünge |
| Stroboskop | reines Lichtflackern |
| Verträumt | weiche Blur- und Licht-Pulse |

Tipp: **Stärke** auf 0 % stellen und nur auf den Beat Keyframes auf 100 % setzen – so zuckt
das Bild genau dort. **Flackern pro Sekunde** = BPM ÷ 60 (z. B. 120 BPM → 2 oder 4).

### Look-Baukasten (Look Builder)

Fertige Kino-Looks per Klick, aufgebaut wie beim Coloristen: **Motiv** (Hautglättung) ▸
**Farbe** (Belichtung, Weißabgleich, Kontrast, Sättigung, Schatten-/Lichtertönung) ▸ **Matt**
(Fade, Lichter-Abrollen) ▸ **Objektiv** (Diffusion, Halation, Vignette) ▸ **Film** (Korn).

Looks: Petrol & Orange, Warmer Film, Kalter Thriller, Bleach Bypass, Vintage 70er,
Pastelltraum, Noir, Neonnacht, Goldene Stunde, Grüner Code, Verblasst matt, Kino-Kontrast.

**Stärke** blendet den Look stufenlos ein. Für einen eigenen Look „Eigene“ wählen und die
Regler der fünf Gruppen nutzen. Hautglättung wirkt auch zusätzlich zu einem fertigen Look.

## Musikvideos: alles im Takt

### Beat-Marker setzen

Musik-Clip in der Zeitleiste auswählen ▸ **Marker ▸ Beat-Marker aus Musik setzen**.
FilmCraft erkennt Tempo und Beats und setzt auf jeden Beat einen Marker: **rosa** = erster Beat
im Takt, **gelb** = die übrigen. Mit **Sequenz ▸ In der Timeline einrasten** (S) springen Schnitte und Clips
genau auf die Beats. Nochmals ausführen ersetzt die alten Beat-Marker.

### Effekte mit der Musik synchronisieren

1. Organisches Wackeln, Flacker-Stapel oder Beat-Puls auf deine Video-Clips legen.
2. Musik-Clip auswählen ▸ **Marker ▸ Plus-Effekte mit Musik synchronisieren**.

Alle Plus-Effekte der Sequenz bekommen Tempo und ersten Beat aus der Musik und schalten
**Mit Musik synchronisieren** ein. Danach sitzen die Stöße, Zuckungen und Pulse genau auf den
Beats. In der Gruppe **Musik-Sync** wählst du: Jeder Beat, Alle 2 Beats, Jeder Takt, Halbe Beats.

### Beat-Puls (Beat Pulse)

Der Klassiker für Musikvideos: Bei jedem Beat „springt“ das Bild und klingt bis zum nächsten
wieder ab. Vorgaben: **Bass-Punch** (Zoom + leichtes Wackeln), **Blitz**, **Glow-Treffer**,
**RGB-Kick**, **Zoom-Bounce**, **Wackel-Treffer**. Der erste Beat im Takt wird etwas stärker
betont. Ohne Sync hält der Effekt den vollen Puls – dann per Keyframes auf **Stärke** steuern.

## YouTube

### Fertige Export-Vorgaben

Im Export unter **Vorgaben ▸ FilmCraft Plus**:

- **YouTube 1080p – Plus (GPU, −14 LUFS)**: H.264, 24 Mbit/s
- **YouTube 4K – Plus (GPU HEVC, −14 LUFS)**: H.265 über die RX 5700, 60 Mbit/s
- **YouTube Shorts – Plus (GPU, −14 LUFS)**: 1080×1920 Hochformat

Alle laufen über den Hardware-Encoder der Grafikkarte und normalisieren den Ton auf **−14 LUFS** (true peak −1 dBTP). Das ist
der Pegel, auf den YouTube regelt – dein Video wird also nicht leiser gedreht.

### Automatische Untertitel

Spracherkennung (Whisper) ist in diesem Build eingeschaltet und läuft komplett lokal auf deinem
PC. **Sequenz ▸ Sequenz transkribieren…** – beim ersten Mal wird das Sprachmodell einmalig
heruntergeladen. Danach **Sequenz ▸ Transkript ▸ Untertitel aus Transkript erstellen…**, Text
korrigieren und über **Datei ▸ Exportieren ▸ Untertitel…** als SRT für YouTube speichern. Über das Transkript kannst du
auch Text-basiert schneiden, Pausen und Füllwörter („äh“, „ähm“) entfernen.

## Optimierungen für deinen PC

- **CPU:** Der Build ist für Arrow Lake-S kompiliert (`target-cpu=arrowlake-s`: AVX2, FMA,
  AVX-VNNI) und nutzt alle Kerne (P- und E-Kerne) für Effekte und Decoding.
  Hinweis: Diese .exe läuft nur auf ähnlich modernen Intel/AMD-CPUs.
- **GPU:** FilmCraft zeichnet Vorschau und Effekte auf der **RX 5700** (DirectX 12 / Vulkan)
  statt auf der Intel-Grafik der CPU (anders wählen: Umgebungsvariable `WGPU_POWER_PREF=low`).
- **Decoding:** H.264/HEVC laufen über die Hardware der RX 5700 (Media Foundation / DXVA).
- **Export:** „Hardware-Encoder verwenden“ ist standardmäßig an. Auf der RX 5700 exportiert
  FilmCraft Plus H.264 und H.265 über **AMD VCN** (Media Foundation), auf NVIDIA-Karten über
  NVENC, ohne Grafikkarte über Intel Quick Sync. Passt eine Einstellung nicht (z. B. zwei
  Durchgänge, CRF, HDR), nimmt FilmCraft für H.264 automatisch den Software-Encoder.
  - Gibt es zwei Encoder (RX 5700 **und** Intel-Grafik), wird AMD bevorzugt. Lieber Intel Quick
    Sync? Umgebungsvariable `FILMCRAFT_HW_ENCODER=Intel` setzen.
  - Die RX 5700 kann kein AV1-Encoding und kein 10-Bit/HDR beim Export; HDR wird für H.265
    automatisch in SDR umgewandelt.
- **macOS:** Der Mac-Build nutzt AVX2/FMA (`x86-64-v3`) statt des exakten CPU-Modells, weil
  OpenCore die CPU oft als anderes Modell meldet. Der Export läuft über VideoToolbox auf der
  RX 5700. Dafür muss macOS die Karte als Hardware-Encoder freigeben (üblich mit dem
  SMBIOS-Modell **iMacPro1,1** oder **MacPro7,1**). Ob das klappt, siehst du daran, dass im Export
  **H.265** angeboten wird. Fehlt H.265, exportiert H.264 über die CPU (langsamer, aber
  funktioniert). Intel Quick Sync gibt es unter macOS mit diesem Prozessor nicht.
- In den Wiedergabe-Einstellungen des Programmmonitors bei 4K ruhig **Voll** wählen; bei sehr
  vielen Effekten **1/2** für flüssige Vorschau, der Export rendert immer in voller Qualität.

## Rechtliches

Die Effekte, Vorgaben und Looks sind eigene Arbeit dieses Forks. Sie sind von der *Art* von
Werkzeugen bekannter Plugins inspiriert, enthalten aber keinen fremden Code, keine fremden
Presets oder LUTs und tragen bewusst eigene Namen. Lizenz wie FilmCraft: MIT oder Apache-2.0.
