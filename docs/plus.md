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

**Kick (Audio)** und **Snare (Audio)** hören direkt auf den Ton der Sequenz statt auf ein Tempo:
Organisches Wackeln mit „Kick (Audio)“ wackelt bei jeder Kick, Flacker-Stapel mit „Snare (Audio)“
zuckt bei jeder Snare – auch bei Tempowechseln, Breaks und Drops. Dafür ist kein Sync-Befehl nötig.

### Beat-Puls (Beat Pulse)

Der Klassiker für Musikvideos: Bei jedem Beat „springt“ das Bild und klingt bis zum nächsten
wieder ab. Vorgaben: **Bass-Punch** (Zoom + leichtes Wackeln), **Blitz**, **Glow-Treffer**,
**RGB-Kick**, **Zoom-Bounce**, **Wackel-Treffer**. Der erste Beat im Takt wird etwas stärker
betont. Ohne Sync hält der Effekt den vollen Puls – dann per Keyframes auf **Stärke** steuern.

### AVKO-Spektrum (Audio-Spektrum im AVKO-Look)

Das Audio-Spektrum für deine Beat-Videos, gebaut nach deinem Logo: Jede Frequenz ist ein
Plattenbau aus Fensterblöcken, die Antennen auf den Dächern zeigen die Spitzen (sie fahren hoch
und sinken wieder), darunter die Straße mit Mittelstreifen, alles mit Grunge-Körnung.

**So geht's (eigene Ebene, empfohlen):**
1. Beat auf eine Audiospur, Hintergrund (Video oder Bild) auf **V1**.
2. **Datei ▸ Neu ▸ Transparentes Video…** und auf **V2** über den Hintergrund legen, so lang wie der Song.
3. **FilmCraft Plus ▸ Musik ▸ AVKO-Spektrum** auf das transparente Video ziehen.

So bewegt sich nur das Spektrum, nicht dein ganzes Projekt.

- **Reagiert vor allem auf Sub und Kick, dann Snare, Melodien nur leicht** (**Reaktion auf
  Melodie**, Standard 35 %).
- **Weichheit** (Standard 70 %): Hochhäuser wachsen weich aus dem Dach statt zu springen,
  fallen langsamer, und Wackeln/Zucken laufen sanft ein und aus. Höher = noch ruhiger.
- **Bass-Bounce:** Das Spektrum „atmet“ mit dem Bass (sanftes Zoomen von der Straße aus) – der
  Trap-City-Look.
- **Kick:** Bass-Hochhäuser springen (**Kick-Sprung**), dazu ein weiches, fließendes Wackeln
  (**Kick-Wackeln**, Standard 35 %). Optional **Fensterlicht bei Kick**.
- **Snare:** sanftes **Snare-Zucken** (leichter Farbversatz, kleiner Versatz/Zoom oder Aufhellen).
- **Treffer-Länge** = wie lange es nachklingt, **Treffer-Empfindlichkeit** = wie leicht
  Kick/Snare erkannt werden.
- **Anordnung** (Hochhäuser, Stockwerke, Breite, Mitte, Straßenhöhe, Blockabstand), **Look**
  (Gebäudefarbe, Grunge, Leuchten, Straße, Straßentempo) und **Audioquelle** (Gesamtmix oder eine
  Spur, z. B. nur die Drums).

**Soll der Hintergrund sanft mitgehen** (wie bei Trap City), lege auf den Hintergrund-Clip:
- **Beat-Puls**, Vorgabe **Zoom-Bounce**, Sync **Kick (Audio)**, Stärke ca. 40–60 %, und/oder
- **Organisches Wackeln**, Vorgabe **Handkamera**, Sync **Kick (Audio)**, Intensität ca. 50 %.

Beide laufen bei Kick-Treffern weich ein und aus. Den ganzen Hintergrund direkt mit dem
AVKO-Spektrum zu bewegen geht weiterhin: Effekt auf den Hintergrund-Clip statt aufs
transparente Video legen.

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

## Reels & TikTok

### Animierte Untertitel (ein Klick)

**Sequenz ▸ Animierte Untertitel** und einen Look wählen. FilmCraft Plus hört sich an, was du im
Video sagst, schreibt es mit und legt daraus eine Untertitelspur an, bei der jedes Wort genau
dann erscheint, wenn du es sprichst. Kurze Zeilen, groß, mittig im unteren Drittel, passend für
Hochformat (im Querformat werden die Zeilen automatisch länger).

| Look | Wirkung |
|---|---|
| **Punch (gelber Pop)** | Wörter springen einzeln rein, das aktuelle Wort gelb und etwas größer |
| **Spotlight (gleitende Box)** | Farbige Box gleitet von Wort zu Wort |
| **Ein Wort** | Immer nur ein Wort, sehr groß, mit Pop |
| **Karaoke (Farbfüllung)** | Zeile steht, die Farbe läuft beim Sprechen durch |
| **Rise (minimal)** | Ruhig: Wörter steigen weich ein, ohne Kontur |
| **Kacheln** | Jedes Wort auf einer eigenen hellen Kachel |

- Beim **ersten Mal** wird das Sprachmodell einmalig heruntergeladen (Internet nötig, dauert
  einen Moment); die Erkennung selbst blockiert die App kurz, je nach Länge des Videos.
- Anderen Look ausprobieren: einfach einen anderen Look wählen. Deine Textkorrekturen bleiben.
- Falsch verstandenes Wort: den Untertitel wie jeden anderen bearbeiten und korrigieren.
- Schnitt geändert? **Aus Transkript neu erstellen** baut die Spur passend zum neuen Schnitt.
- Farbe, Schrift, Größe und Position änderst du wie bei jeder Untertitelspur.

### Übergänge (Effekte ▸ FilmCraft Plus ▸ Übergänge)

Auf den Schnitt zwischen zwei Clips ziehen, Länge wie gewohnt am Rand ziehen.

- **Block-Aufbau:** Das neue Bild baut sich Block für Block auf, wie die Hochhäuser im
  AVKO-Logo. Reihenfolge wählbar (Türme aufwärts/abwärts, von links/rechts, zufällig).
- **Reißschwenk:** Schneller Kameraschwenk mit Schliere, leichter Neigung und Farbsaum.
- **Durchschlag-Zoom:** Zoom durch das Bild mit Drehung, Farbversatz und kurzem Blitz.
- **Glitch-Schnitt:** Das Bild reißt in Streifen und springt streifenweise zum neuen Clip.

Tipp für Musikvideos: Übergang auf 8 bis 12 Bilder kürzen und genau auf einen Beat-Marker legen.

### Studiolicht (Effekte ▸ FilmCraft Plus ▸ Porträt)

Auf den Clip ziehen, fertig. Der Effekt findet dein Gesicht, belichtet darauf, setzt ein weiches
Führungslicht, hellt Schatten auf, nimmt den Hintergrund etwas zurück und glättet die Haut.

- **Licht-Setup:** Weiches Führungslicht, Beauty, Dramatisch, Hell & klar, Warmer Abend oder
  Eigenes (dann gelten die Regler unter „Eigenes Licht“).
- **Stärke** regelt alles zusammen. **Automatische Belichtung (Gesicht)** gleicht zu dunkle oder
  zu helle Aufnahmen aus.
- **Licht anzeigen** zeigt in Gelb, wohin das Licht fällt.

### Autofokus (Effekte ▸ FilmCraft Plus ▸ Porträt)

Hält dich scharf und zeichnet den Rest weich (gaußscher Weichzeichner), der Fokus folgt deinem
Gesicht von selbst.

- **Hintergrund-Unschärfe:** wie stark. **Fokusbereich:** wie groß der scharfe Bereich ist.
  **Übergang:** wie weich es von scharf nach unscharf geht.
- **Körper scharf lassen:** an = Kopf, Schultern und Oberkörper; aus = nur der Kopf.
- **Fokusbereich anzeigen** färbt alles rot, was unscharf wird (zum Einstellen).
- Wird kurz kein Gesicht gefunden (wegdrehen, Hand davor), bleibt der Fokus stehen und kehrt
  weich zurück. Ganz ohne Gesicht gilt **Manueller Punkt**.

Gut zu wissen: Gefunden werden Gesichter, die ungefähr zur Kamera schauen. Der scharfe Bereich
ist eine weiche Kopf-und-Schultern-Form, kein pixelgenaues Freistellen. Bei sehr starker
Unschärfe sieht man das an Haaren und Händen, die aus der Form ragen; dann den Fokusbereich
größer oder den Übergang weicher stellen. Beide Effekte am besten direkt auf den Clip legen
(nicht auf eine Einstellungsebene), dann läuft die Verfolgung ruhig.

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

Die Gesichtserkennung nutzt die frei lizenzierten Trainingsdaten des OpenCV-Projekts
(`assets/models/lbpcascade_frontalface.xml`, Apache-2.0, siehe `ATTRIBUTION.md`); ausgewertet werden
sie von eigenem Code.

Die Effekte, Vorgaben und Looks sind eigene Arbeit dieses Forks. Sie sind von der *Art* von
Werkzeugen bekannter Plugins inspiriert, enthalten aber keinen fremden Code, keine fremden
Presets oder LUTs und tragen bewusst eigene Namen. Lizenz wie FilmCraft: MIT oder Apache-2.0.
