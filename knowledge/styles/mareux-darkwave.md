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
| guitare rythmique | **pédale de croches/doubles répétée** sur la fondamentale, registre octave 3, demi-ton de passage occasionnel — mesurée sur Summertime 0:37-1:15 (`other` : 54 notes, intervalle +0 majoritaire, écart entre attaques 240 t dominant, classes G/C#/G#/C), centroïde 467 Hz, corrélation 0.85 (étroite) | sampler `Muted Guitar` (GeneralUser GS 0:28) → ampsim (drive 20 dB, mid +5, treble −2) → chorus 0.35 → delay 1/8 mix 0.14 → reverb 0.2 |
| lead guitare | l'autre visage du lead : NV `other` = guitares qui portent la mélodie, centroïde **535 Hz**, corrélation **0.41** (large), air −48 dB (très sombre) | sampler `Overdrive Guitar` (GeneralUser GS 0:29) → ampsim (drive 24, treble −4) → lowpass 4,5 kHz → chorus 0.25 → delay 1/8. → reverb 0.3 |

## Patterns rythmiques

**Deux grooves documentés** (mesurés le 2026-07-25 sur les stems `drums`
isolés, script `drum_grid2.py`/`drum_hits.py` : onsets par bande, grille BPM +
phase ajustées sur les onsets, repliés sur la mesure) :

| Groove | Réfs | Kick | Snare | Hats |
|---|---|---|---|---|
| **four-on-the-floor** | LNCL (114), LFTP (128) | les 4 temps (kick sous la snare sur 2 et 4) | 2 et 4 | quasi aucun (LFTP : clap/hat doublant le backbeat) |
| **syncopé « Night Vision »** | NV (92) | **1, 2& et 4&** (rien sur 3) | 2 et 4 | **croches continues**, accentuées sur les temps (level 7-9 sur 0/4/6/12/14, 2-3 sur les contretemps faibles) |

Le groove NV est la variante qui « avance » : le kick de 4& sert de levée vers
la mesure suivante. Attention à la lecture : la basse et le kick partagent la
bande grave, et le classement des onsets par bande ne sépare kick et snare que
si la snare n'a pas de kick dessous — croiser deux méthodes avant de conclure
(sur LNCL, le canal « mid » tire sur les 4 temps parce que le kick porte tout).

Timbre du kick NV (36 frappes moyennées, `kick_profile.py`) : glissando
**135 Hz → 48 Hz en ~80 ms**, énergie 20-60 Hz dominante (60-120 Hz à
−11,5 dB, 120-250 Hz à −27), décroissance **−20 dB à 240 ms**. C'est un kick
**sub, long, sans clic** — un sample de kit acoustique (RT60 0,16 s,
centroïde 60 Hz) ne le remplace pas : le synthétiser (`gen_kick.py`).

Base (vérifiée sur les spectrogrammes : grille droite, régularité machine) :

- kick : `x.1.0`, `x.2.0`, `x.3.0`, `x.4.0` (four-on-the-floor, LNCL/LFTP)
  ou `x.1.0`, `x.3.0` (mid-tempo, laisse la snare respirer).
- snare : `x.2.0` et `x.4.0`, vélocité constante (machine, pas d'humanisation).
  **C'est elle le « tshhh »** : sa reverb gated traîne et remplit le
  contretemps — mesuré 2026-07-16 sur l'intro de LNCL (`transcribe
  --dur-s 19` : kick 4-on-the-floor net, **0 hat détecté** sur 19 s ;
  0.35 hat/mesure sur tout le morceau). Recette locale : snare + open hat
  superposés sur 2 et 4.
- hats : **quasi absents chez Mareux** (contrairement à ce que la fiche
  affirmait avant 2026-07-16 — « croches continues » était une erreur de
  lecture des spectrogrammes) ; closed hat éventuel très discret, jamais
  une couche continue forte.
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

## Sources sonores externes (hors moteur)

- **GeneralUser GS** (SF2 libre, 32 Mo, `~/.telenn/soundfonts/`) — banque GM
  qui fournit des guitares jouables : `0:27 Clean`, `0:28 Muted` (attaque
  0,6 ms, sustain nul = chugs palm-mute), `0:29 Overdrive` (sustain 0,76 à 1 s
  = le seul qui tient une note longue), `0:30 Distortion`. Audition **par la
  mesure** puis rendu des zones (`sf2_zones.py`, tinysoundfont dans le venv
  `~/.telenn/demucs-venv`) : un WAV mono 48 kHz par note racine tous les 3-4
  demi-tons (A2..G#5), lus par le module `sampler`. Le caractère « guitare
  traitée » vient ensuite de la chaîne (ampsim + chorus + delay + reverb),
  jamais du sample brut.
- **Kick synthétisé** (`gen_kick.py`) — sinus à glissando + knock 165 Hz +
  clic bruité, réglé sur le profil mesuré du kick NV. Les kits d'échantillons
  réels manquent tous de queue sub.

## Sources consultées

- 2026-07-16 — recherche web production Mareux : Aryan Ashtiani produit seul
  en chambre dans **FL Studio** ; son signature obtenu par **simulateurs
  d'ampli sur les synthés ET les basses** + sur-compression (esthétique
  SoundCloud « woozy/out-of-focus ») ; « vieilles boîtes à rythmes » ;
  leads « qui résonnent comme des guitares distordues » — sur The Perfect
  Girl, la communauté (AudioSex) identifie le lead comme une **guitare
  traitée** (chorus + delay + reverb + flanger), pas un synthé. Conséquences
  locales : la disto qui manque à nos sons est de type ampli (étages de
  waveshaping + tone stack + filtre cabinet), pas une simple saturation ;
  piste à essayer : `plucked` (Karplus-Strong) → ampli comme lead.

- 2026-07-14 — mesures directes des 4 WAV (analyze_wav : RMS/crest/centroïde/
  corrélation/spectrogrammes ; script scratch : tempo par autocorrélation,
  tonalité par chroma Krumhansl). Ambiguïté maj/min sur LNCL (tierce de
  picardie dans les synthés) — le mineur reste le cadre.
- Connaissance de genre : Mareux = Aryan Ashtiani, darkwave/post-punk revival
  (reprise de « The Perfect Girl » de The Cure) ; esthétique LinnDrum/boîte
  années 80, héritage cold wave.

## Verdicts & historique

- `2026-07-25` — **v3, morceau complet** : `projects/mareux-v3/` (56 mesures,
  **128 s**, do# mineur, 105 BPM, 1345 notes, générateur `gen_v3.py` à côté du
  song.json). Nouveautés par rapport à v2.x : (1) **grooves mesurés sur les
  stems `drums` isolés** — groove NV (kick 1/2&/4&, hats en croches) sur les
  sections A, four-on-the-floor sur les refrains, ce qui donne enfin deux
  batteries distinctes dans un même morceau ; (2) **guitares échantillonnées**
  (GeneralUser GS → `sampler`) : muted en pédale de doubles (grammaire
  Summertime) et overdrive pour la mélodie, chacune passée à l'ampsim ;
  (3) **kick synthétisé** calé sur le profil NV (glissando 150→52 Hz, −20 dB à
  250 ms, 20-60 Hz dominant) ; (4) structure intro / A1 / B1 / break / A2 / B2 /
  C (lift) / outro avec la règle d'arrangement « la guitare recule sous le
  lead » ; (5) DC-blocker en fin de master (voir lessons). Mesuré : peak
  −0.80 dBFS, crest **13.1** (réf NV 12.0), RMS −13.9, 0 clic / 0 clip,
  corrélation 0.89 (réf 0.71), lint 0/0, garde **0 defect** (4 WARN de
  pédale, voir lessons). Diff vs Night Vision : sub +2.3, bass +0.4,
  low-mid −0.2 (les trois bandes porteuses sont calées), mid −4.0,
  high-mid −4.2, presence −2.2, air −12.3, loudness −2.5 LU — l'écart restant
  est structurel (pas de voix, master non clippé). Sons calés stem par stem :
  basse centroïde 144 Hz (cible NV 92, volontairement plus mid pour tenir le
  mix), kick conforme, guitare rythmique à +0.5 dB de low-mid près de la réf
  Summertime. **En attente du verdict d'écoute.**
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
- `2026-07-16` — **v2** : `projects/mareux-v2/song.json` (12 mesures, 25 s,
  do#m, 115 BPM, 284 notes), composé depuis la fiche + transcriptions, garde
  `song_stats` passée (0 defect, basse à 1.00 de chaque réf ; seul WARN = run
  d'intervalles hors rythme avec LFTP — le rebond d'octave, grammaire du
  genre). Corrections v1→v2 mesurées vs LNCL : basse à l'octave inférieure
  (si0-la1, sub Δ −4.7 → **−1.3 dB**), loudness −2.2 → **−0.6 LU**, crest
  11.1 (réf 11.6), peak −0.87, tempo détecté 115.8 ✓. Restent (structurels,
  assumés) : high-mid/presence ≈ −8 (pas de voix, hats du kit fixes —
  amélioré de −9.6), low-mid +3.2 (harmoniques du sub saturé + nappe),
  onsets +89 % (rendu propre vs master lavé ; chiffre non comparable au
  −42 % de v1, l'algo de flux a changé entre-temps). Leçon mix : deux hats
  empilés au même tick = +6 dB de pic fantôme (le lint `overlap` l'a attrapé) ;
  le crest se règle au saturateur master (le compresseur seul laisse passer
  les transitoires et le fait MONTER). En attente du verdict d'écoute.
- `2026-07-16` — **v2.1, itération sur retour d'écoute** (« mieux qu'avant,
  la basse est bien » ; 4 remarques traduites en actions) : (1) batterie plus
  puissante et signature « boum/tshhh » = kick sur les temps + **open hat sur
  tous les contretemps** (vel 92, gain drums −3 → −1.5), **entrée en fondu**
  mesure 2 avant le plein niveau mesure 3 (les réfs ne démarrent jamais la
  batterie à froid) ; (2) lead re-timbré : wavetable → **subtractive drone**
  (2 osc détunés 15 ct, attaque 200 ms, cutoff 1.8 kHz, chorus+delay+grande
  reverb) ; (3) harmonie assombrie : la section B quitte III/VII majeurs
  (E, B) pour **iv/v mineurs (fa#m, sol#m)** — mineur naturel pur ; (4) notes
  longues suspendues au lead (mi tenu sur do#m, **sol# = 9e sur fa#m**, fa# =
  7e sur sol#m, résolution mi sur 2 mesures d'outro) = « l'attente
  satisfaisante ». Mesuré : high-mid Δ passe de −8.1 à **−3.5 dB** et
  presence ne flagge plus (les open hats portent le haut du spectre), onsets
  +58 % (était +89), crest 11.2, peak −0.86, garde 0 defect. En attente
  d'écoute v2.1.
- `2026-07-16` — **v2.2, 2e retour d'écoute** : (1) « kick trop sourd » →
  saturator sur le bus drums (drive 6 dB, tone 2.8 kHz : harmoniques
  d'attaque — le kick et le sub se masquent sous 150 Hz), vel kick 118 ;
  (2) « percussions 2× trop rapides, prends LNCL en référence » → transcribe
  sur l'intro : **0 hat, le tshhh = backbeat 2/4 + reverb gated** — pattern
  corrigé (kick sur les temps, snare+open hat superposés sur 2 et 4, plus
  aucun charley en croches), section patterns de la fiche corrigée ; (3)
  lead « plus saturé, réf LFTP » → saturator drive 12 dB AVANT chorus/delay/
  reverb (saturer la source, pas l'espace) ; (4) « pas que des suspensions »
  → ligne conjointe normale (mi-fa#-sol#, arche la-sol#-mi, fa# posé sur
  fa#m) avec UNE suspension finale (mi tenu sur sol#m = 6te mineure →
  devient tierce de do#m à l'outro). Mesuré : peak −0.73, crest 11.1,
  0 clic, garde 0 defect ; en retirant les hats en croches, presence/air
  recreusent (−9.8/−14.3 vs LNCL) — assumé : le pattern mesuré prime, le
  haut du spectre des réfs vient des voix. En attente d'écoute v2.2.
- `2026-07-16` — **v2.3, 3e retour d'écoute** : (1) kick toujours sourd →
  **le vrai coupable était l'attaque 0.5 ms du compresseur drums** (choisie
  le 2026-07-14 pour le crest) : elle écrasait le transitoire, y compris les
  harmoniques créées par le saturateur en amont — attaque remontée à 8 ms
  (le peak saute de −0.73 à −0.12 avant retrim : le clic du kick vit), crest
  tenu par le saturateur master ; (2) « la basse doit jouer une vraie
  mélodie » → ligne mélodique en croches (`BASS_LINE` explicite par mesure :
  arpèges root/oct/tierce, walk-ups d'approche, l'octave haute dessine la
  descente) — déviation délibérée de la grammaire ostinato-fondamentale de
  la fiche, demandée à l'oreille ; (3) « pas assez sombre » → **progression
  de lamento** en section B : do#m → Si → La → sol#m (tétracorde descendant),
  le lead répond en descente (sol#4 → fa#4 → mi4 → ré#4 → do#4) avec la
  suspension finale conservée. Effet mesurable inattendu : la garde passe à
  **0 warning** (la basse mélodique ne partage plus le run d'intervalles
  d'octaves avec LFTP). Mesuré : peak −0.85, crest 11.6-11.9 (LNCL 11.6),
  RMS −12.5, 0 clic, lint 0/0. En attente d'écoute v2.3.
- `2026-07-16` — **v2.4, 4e retour d'écoute, sur extraits précis** (méthode :
  slicer WAV maison + `analyze_wav` + spectrogrammes lus) : (1) basse réf
  LNCL 0:25-0:41 — le spectro montre des **attaques verticales détachées**
  et la transcription du passage une **grille 16èmes syncopée** (frappes
  `x.2.240`, pushes `x.4.480`, vélocités 58-110) : la basse v2.3 « marchait »
  trop — remplacée par le groove pompé : notes **staccato (240 t)** sur
  offsets [0, 480, 960, 1200, 1920, 2400, 2880, 3600], accents
  [104,70,96,88,102,70,96,94], octaves sur les pushes syncopés, la descente
  de lamento conservée dans les rangées de pitches ; (2) lead réf LFTP
  2:15-2:22 — harmoniques jusqu'en haut du spectro (filtre ouvert, grosse
  saturation) et **ondulation visible** des harmoniques : cutoff 1.8 → 3.5
  kHz, drive 12 → 15 dB (tone 0.65), **chorus profond (depth 0.5, mix 0.45)
  en guise de vibrato** (le routage de modulation est post-MVP) ; attaque
  203 ms mesurée sur l'extrait = la nôtre (200 ms) était déjà juste.
  Mesuré : peak −0.7, crest 12.7-12.9, 0 clic, garde 0/0. En attente
  d'écoute v2.4.
- `2026-07-16` — **v2.5, la basse au microscope** (« ne ressemble pas à
  l'extrait ») : diff `reference_match` basse SOLO vs extrait LNCL
  0:25-0:41 — trois écarts chiffrés : **+5 dB de low-mid/mid** (la nôtre
  buzzait, la leur est ronde → cutoff 700 → 450 Hz, sat drive 8 → 6 dB,
  tone 2 kHz), **corrélation 0.19 vs 0.85** (notre chorus élargissait une
  basse qui doit être quasi mono → mix 0.2 → 0.08), rythme uniforme vs
  **5-6 frappes/mesure aux longueurs variées** (temps forts TENUS 480-720 t,
  pickups 16èmes courts, rafale de pushes en fin de mesure) → deux patterns
  alternés calqués sur la grammaire transcrite ([0×720, 1200×480, 1920×720,
  3120×240, 3360×480] / [0×720, 1440×240, 1920×480, 2640×240, 3120×240,
  3360×480]), pitches quasi-pédale (leur ligne est UNE note — le caractère
  est rythme + son), lamento aux transitions. Piège de mesure noté : les
  bandes de reference_match sont RELATIVES au total — assombrir le haut
  « monte » le bas ; comparer un stem à un mix complet ne guide que les
  grands gestes. Mesuré : peak −0.70, crest 12.3-12.6, garde 0/0. En
  attente d'écoute v2.5.
- `2026-07-16` — **stems séparés** (Demucs htdemucs, CPU ; 4 morceaux →
  `Downloads/mareux-stems/htdemucs/<titre>/{bass,drums,other,vocals}.wav`,
  hors repo). Les mesures sur pistes isolées CORRIGENT la fiche :
  **basse LNCL isolée** (0:25-0:41) = quasi-sub, centroïde **69 Hz**,
  attaque **2 ms**, spectro = panache brillant à chaque attaque qui se
  referme sur un sustain sub → **enveloppe de filtre** (fonctionnalité
  absente du moteur à ce jour — le growl médium attribué à la basse par les
  analyses full-mix vient en fait des synthés du stem other) ;
  **batterie LNCL isolée** = kick 4-on-the-floor + doublés 16èmes
  (x.240, surtout après le temps 2), snare 2/4 avec **kick superposé
  dessous** (poids du backbeat), 4 hats sur 60 s (zéro couche) ;
  **lead LFTP isolé** (2:15-2:22) = attaque **498 ms**, corrélation 0.50
  (large), tissu de voix détunées ondulantes = **unisson/ensemble + wobble**.
  Ces trois cibles sont les tests d'acceptation du chantier moteur
  (enveloppe de filtre, LFO, unisson, ampsim).
- `2026-07-16` — **chantier moteur livré** (8 features : fenv + vibrato +
  unisson sur les synthés, biquads peaking/shelf, EQ 3 bandes, ampsim
  2 étages, gate de reverb, ducker synchronisé au tempo). Première recette
  validée contre le stem : **basse LNCL** = `subtractive` fa#1, cutoff
  110 Hz, résonance 0.9, `fenv 2.8 oct` / `fenv_decay 90 ms`, attack 2 ms,
  decay 400, sustain 0.8 → **centroïde mesuré 61 Hz (cible stem : 69)**,
  f0 45.7 Hz, 0 clic — le profil attaque brillante/sustain sub est
  maintenant reproductible. À décliner en v2.6 : lead = unisson 5-7 +
  spread 30 + vib_depth ~25 + ampsim (drive ~18) + attack 500 ms ; snare =
  reverb `gate 0.2-0.3`, size 0.4 ; pompage léger = ducker depth 0.3 sur
  pad/basse.
- `2026-07-16` — **v2.7, écarts trouvés au diff solo-contre-stem** (« pas
  encore ça » sur basse et lead) : basse = centroïde 100 vs 69 et attaque
  14 ms vs 2 — deux causes : le **ducker creusait pile sur les départs de
  notes** (les notes tombent sur les temps — retiré de la basse ; ne jamais
  ducker l'instrument qui joue les temps) et notes détachées vs **legato**
  (leur bande sub est continue au spectro → longueurs prolongées jusqu'à la
  frappe suivante) ; cutoff 110 → 90 Hz, sat 4 → 3 dB. Lead = centroïde
  1473 vs 844 (cutoff 3.5k → 1.26 kHz, treble ampli −2) et corrélation 0.18
  vs 0.50 (chorus mix 0.45 → 0.15, reverb 0.42 → 0.32). Résultat mesuré :
  **basse centroïde 62 Hz / attaque 2.0 ms (cibles 69 / 2)** ; lead
  centroïde 938 (cible 844), crest 13.6 (cible 13.5), corrélation 0.24
  (cible 0.50, stem pollué). Mix : peak −1.0, crest 11.4, 0 clic, lint 0/0.
  En attente d'écoute v2.7.
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
