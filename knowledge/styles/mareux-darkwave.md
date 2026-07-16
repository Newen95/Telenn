# Style : Mareux (darkwave / post-punk synthétique)

Fiche construite le 2026-07-14 à partir de **mesures sur 4 morceaux de
référence** (Laugh Now Cry Later, Night Vision, Lovers From The Past,
Summertime — WAV fournis par l'utilisateur, analyse `analyze_wav` +
autocorrélation d'attaques + chroma) et de connaissances de genre.

## Identité

- Tempo mesuré : 75 / 92 / 115 / 128 BPM — cœur du style **~110-128**,
  ballades à 75-92. Signature 4/4, droit (pas de swing mesurable).
- Tonalités mesurées : do# mineur ×2, ré# mineur, la# mineur — **mineur,
  tonalités en dièses**, basses toniques très graves (f0 mesuré 60 Hz sur
  Summertime, ≈ la#1).
- En une phrase : une boîte à rythmes sèche années 80 et une **basse synthé
  omniprésente qui porte le morceau**, des nappes froides très réverbérées,
  peu de notes, beaucoup d'espace nocturne — The Cure passé au filtre
  synthwave lo-fi.

## Mesures de référence (moyennes des 4 morceaux)

- RMS master −11 à −13 dBFS, crest **10.5-13.3 dB** (master très compressé).
- Centroïde global **80-468 Hz** — l'énergie vit dans le grave ; même le
  morceau le plus « brillant » (Night Vision) reste sous 500 Hz.
- Corrélation stéréo 0.64-0.86 (image plutôt étroite, lo-fi).
- Attaque du master 13-482 ms : les intros s'installent, pas d'impact
  d'entrée façon EDM.
- Spectrogrammes : kick très régulier (four-on-the-floor ou 1-3), hats en
  croches continues, sections nettes (les aigus tombent d'un bloc dans les
  breaks), treillis harmonique médium = accords tenus/arpèges répétitifs.

## Instrumentation type

| Rôle | Son recherché (mesurable) | Recette locale (mesurée 2026-07-14, `projects/mareux-test`) |
|---|---|---|
| basse | LA voix principale : ostinato en croches, saw/pulse doublé détuné, lowpass 500-900 Hz, centroïde < 400 Hz, attaque < 15 ms, ligne registre **23-38 (si0-ré2)** — transcription LNCL 2026-07-15 : tonique tenue à F#1/B0/C#1, plus grave que supposé | subtractive (saw, osc2 1.0, detune 10 ct, cutoff 700 Hz, att 3 ms) → saturator (drive 8 dB, tone 2 kHz, level −3) → chorus (mix 0.2) — mesuré : centroïde 128 Hz, attaque 6 ms, crest solo 10.6 dB |
| batterie | drum machine sèche : kick rond, snare claquante + reverb courte dense (gated 80s), hats fins en croches | drum_sampler → compressor (**attack 0.5 ms**, threshold −22, ratio 4:1, makeup +7 — voir leçon 2026-07-14) → reverb (size 0.25, damp 0.7, mix 0.12) |
| nappe | froide, attaque lente (200-500 ms), spectre médium qui bouge peu, très réverbérée, accords tenus 1-2 mesures | wavetable (position 0.35, attack 350 ms, release 500 ms) → chorus (0.35) → reverb (size 0.7, mix 0.4) — mesuré : attaque 441 ms, centroïde 246 Hz à G#3 |
| lead | rare et simple : mélodie conjointe mineure, 1 note à la fois, delay en croche pointée, jamais virtuose | wavetable (position 0.55, pos_env −0.25, att 8 ms) → delay (sync 1/8., feedback 0.4, mix 0.28, tone 3.5 kHz) → reverb (0.3) |

## Patterns rythmiques

PPQ 960, positions 1-indexées. Base (vérifiée sur les spectrogrammes :
grille droite, régularité machine) :

- kick : `x.1.0`, `x.2.0`, `x.3.0`, `x.4.0` (four-on-the-floor, LNCL/LFTP)
  ou `x.1.0`, `x.3.0` (mid-tempo, laisse la snare respirer).
- snare : `x.2.0` et `x.4.0`, vélocité constante (machine, pas d'humanisation).
- closed hat : croches continues `x.y.0`/`x.y.480`, vélocité légèrement
  plus basse sur les contretemps ; open hat ponctuel sur `x.4.480`.
- Pas de fill spectaculaire : les transitions se font par ajout/retrait de
  couches (mute d'un bloc), pas par roulements.

## Harmonie & mélodie

- Mineur naturel ; progressions typiques du post-punk : **i-VI-III-VII**,
  i-VII-VI, ou pédale de tonique quasi permanente à la basse avec accords
  qui bougent au-dessus.
- Basse : ostinato en croches sur la fondamentale de l'accord, avec
  ouvertures ponctuelles à l'octave ou à la quinte en fin de mesure —
  c'est le motif signature, il ne s'arrête jamais.
- Lead : registre 62-75 (ré4-mi♭5), portée étroite, mouvements conjoints,
  phrases courtes séparées par du silence (le delay remplit).
- Nappes : triades serrées registre 50-65, une par accord, tenues.

## Structure type (ramenée à 20-30 s : 12 mesures à ~115 BPM ≈ 25 s)

Structure des originaux : intro (basse+drums seuls) → couplet (+ nappe) →
refrain (+ lead) → break (retrait d'un bloc d'aigus) → refrain. Les couches
s'empilent et se retirent en blocs de 4/8 mesures.

Miniature 12 mesures : intro 2 (drums+basse) — A 4 (+nappe) — B 4 (+lead)
— outro 2 (retrait lead+hats, la basse finit).

## Cibles mix (mesurables par `render_probe --song`)

- Peak master ≤ −0.7 dBFS (standard maison), **pas** de clipping (les
  références clippent : artefact de mastering commercial, ne pas imiter).
- Crest visé **11-13 dB** (compressor/saturator sur le master si le rendu
  brut dépasse 15).
- Centroïde global **< 500 Hz** ; la basse est la piste la plus forte
  (RMS solo ≥ RMS drums, nappe/lead nettement derrière).
- Image : mono-compatible (chorus léger, pas de widener) — corrélation des
  références 0.64-0.86.

## Sources consultées

- 2026-07-14 — mesures directes des 4 WAV (analyze_wav : RMS/crest/centroïde/
  corrélation/spectrogrammes ; script scratch : tempo par autocorrélation,
  tonalité par chroma Krumhansl). Ambiguïté maj/min sur LNCL (tierce de
  picardie dans les synthés) — le mineur reste le cadre.
- Connaissance de genre : Mareux = Aryan Ashtiani, darkwave/post-punk revival
  (reprise de « The Perfect Girl » de The Cure) ; esthétique LinnDrum/boîte
  années 80, héritage cold wave.

## Verdicts & historique

- `2026-07-14` — création de la fiche (mesures + genre). Statut : plausible,
  en attente d'un premier morceau validé.
- `2026-07-14` — premier morceau : `projects/mareux-test/song.json`
  (12 mesures, 25 s, do# mineur, 115 BPM, i-VI-III-VII, 270 notes).
  Mesuré : peak −1.0 dBFS, RMS −15.6, crest 14.6 dB, centroïde 348 Hz,
  f0 dominant 55 Hz, 0 clic/clip. Note : sur une miniature clairsemée le
  crest plancher est ~14-15 dB sans bus master — les 11-13 dB des
  références viennent d'un mastering pleine densité (qui clippe).
  En attente du verdict d'écoute.
- `2026-07-15` — première transcription (`transcribe` sur LNCL) : tempo
  détecté 113.9 vs 114.0 ✓ ; basse = ostinato en croches confirmé, clartés
  0.85-0.98, mais **registre plus grave que la fiche** (F#1/B0/C#1, soit
  34-46 Hz — recoupe le déficit de sub mesuré par reference_match) ;
  accords = pédale C#/C#m sur le couplet, l'intro assise sur F# (iv) ;
  four-on-the-floor lisible sur l'intro exposée (mesures 3-4), canal kick
  sur-détecte dès que l'ostinato entre (kick et basse partagent 40-150 Hz —
  transcrire la batterie sur segments exposés). Ambiguïté maj/min persistante
  (Krumhansl donne C# majeur 0.84 sur le couplet — tierce de picardie).
- `2026-07-15` — espace de liberté mesuré (`song_stats` sur les 4 basses
  transcrites) : distances inter-références 0.81-1.00 (paire la plus proche
  LFTP↔Summertime 0.81) — même chez l'artiste, deux morceaux ne partagent
  presque aucun n-gramme (intervalle, rythme) ; c'est la grammaire (ostinato
  de croches, pédale, ouvertures d'octave) qui est commune, pas les lignes.
  Plancher de nouveauté pour composer : ≥ 0.81. `mareux-test` v1 : basse à
  0.85 de LNCL (au-dessus), pad/lead à 1.00, 0 defect — garde passée.
- `2026-07-14` — stéréo native + bus master : le même morceau passe à
  corrélation 0.94 (les reverbs/chorus décorrèlent ; références 0.64-0.86)
  et un master `compressor (thr −20, makeup +3) → saturator (drive 4 dB)`
  amène le crest à **13.0 dB — dans la cible** (RMS +2.9 dB ; retrimmer
  `master_gain_db` après ajout). Premier `reference_match` vs Laugh Now Cry
  Later : tempo 114.8 vs 114.0 ✓, crest 13.0 vs 11.6 ✓ ; écarts restants
  chiffrés — −2.2 LU de densité, sub −4.7 dB (leur basse descend sous
  60 Hz), high-mid/presence −9.6 dB (mix plus sombre : pas de voix, hats
  discrets), densité d'attaques −42 %. Pistes : octaver la basse vers C#1,
  éclaircir lead/hats, épaissir l'arrangement.
