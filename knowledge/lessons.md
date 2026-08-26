# Leçons transverses

Journal daté, du plus récent au plus ancien. Format : `## AAAA-MM-JJ — titre`
puis contexte, mesure (commande exacte), conclusion. Les impasses se
consignent au même titre que les réussites.

## 2026-07-25 — Saturer du grave asymétrique fabrique du DC (mettre un HP derrière)

Mareux v3 : le mix sortait avec `dc +0.0078` (DEFECT) alors qu'aucune source
n'en avait. Autopsie : le `subtractive` à nu est propre (dc −0.00018) ; dès
qu'un saturateur est derrière, l'offset apparaît et **croît avec le drive**
(drive 6 → +0.0009, 8 → +0.0034, 10 → +0.0061, 12 → +0.0175). Le shaper est
pourtant un `tanh` symétrique : c'est le **signal** qui est asymétrique (une
scie filtrée bas avec résonance n'a pas la même excursion des deux côtés), et
un shaper symétrique compresse davantage la plus grande excursion → moyenne
non nulle. Le coupable final n'était pas la basse (son HP 25 Hz local marchait)
mais le **saturateur du master**, qui refabrique du DC sur le mix entier.
Règle : après **chaque** étage de saturation traversé par du grave, un
passe-haut 20-25 Hz (le condensateur de liaison de l'analogique). Coût
mesuré : nul (dc −0.000003, aucune perte de sub audible à 34 Hz).

## 2026-07-25 — Une ligne en pédale ne peut pas passer le plancher de nouveauté

`song_stats` sur mareux-v3 : la basse sort à 0.88 d'une basse de référence
quand le plancher inter-références vaut 0.97, et la garde signale « 21
intervalles partagés en ignorant le rythme ». Ces 21 intervalles sont… des
`+0` : les deux lignes tiennent une pédale en croches. Le détecteur de runs
compte une suite de notes répétées comme un motif partagé, donc **toute
basse-pédale du genre échouera contre toute autre basse-pédale**. Deux
conséquences : (1) lire un WARN de run en regardant les `steps` du rapport —
une suite de `+0` est de la grammaire, pas une citation ; (2) ce qui fait
vraiment baisser la distance, ce sont les **figures** (levée à l'octave, 5te,
b7, approche chromatique) et un **motif rythmique par section** — ajoutés en
v3, ils ont fait tomber le DEFECT de la guitare doublée (0.50 → 1.00) sans
changer le caractère. Piste outil : ignorer les runs dont tous les intervalles
sont nuls, ou les pondérer par l'entropie du motif.

## 2026-07-25 — Compresser le master efface la structure du morceau

Le morceau semblait « plat » : mesuré section par section (RMS sur 4 mesures),
intro −13.7, refrain −13.1, break −14.7 — 0,6 dB entre l'intro dépouillée
(basse + hats) et le refrain complet. Cause : le compresseur master à
threshold −25 / makeup +7 (réglé pour la densité) ramenait tout au même
niveau. Réglé à −14 / ratio 2.5 / makeup +1.5, le même arrangement respire :
intro −13.5, refrains −12.0, break −15.4, C −11.9. Leçon : le crest cible de
la fiche (11-13 dB) se règle **sans** écraser la dynamique de forme — vérifier
toute compression master par un RMS par section, pas seulement par le crest
global (le crest, lui, ne bougeait presque pas : 12.2 → 13.1).

## 2026-07-16 — Scie sub à nu : le premier front lit comme un clic

Mareux v2.1, basse saw do#1 (34,6 Hz, période 29 ms) seule sur la mesure 1 :
`render_probe --song` flagge « 1 clic à 0.029 s, jump 0.12 ». Ce n'est pas
l'enveloppe (attaque 3 → 8 ms : inchangé) : c'est le **premier front de
scie**, une période après le départ — les deux fenêtres avant du détecteur
(2/25 ms) sont plus courtes que la période, donc « rien ne suit » = glitch.
Seul le premier front flagge (les suivants baignent dans le signal). En v2
le kick à t=0 le masquait ; à do#2 (14,5 ms de période) le cas n'existe pas.
Correctif musical, pas de détecteur : ne jamais démarrer un morceau sur une
scie sub à plein niveau depuis le silence — fondu de vélocité sur la
première mesure (56 → 91), cohérent avec les intros du genre qui
s'installent. Clic éteint, son inchangé passé la mesure 1.

## 2026-07-14 — Compresser un bus de drums : l'attaque décide si le crest monte ou descend

Mix mareux-test, cible crest 11-13 dB. Compressor sur
`drum_sampler` avec attack 5 ms + makeup +5 dB : le crest du mix est passé
de 19.6 à **23.6 dB** — le transitoire du kick (< 1 ms) traverse la fenêtre
d'attaque intact et le makeup le remonte, pendant que le corps se fait
comprimer. Avec attack 0.5 ms le même étage fait 19.6 → 14.6. Deuxième
piège : une fois le bus compressé, baisser la vélocité d'un élément (snare
104 → 98) ne change quasi rien au pic — le makeup réabsorbe l'écart. Régler
l'équilibre AVANT d'insérer la compression, et toujours re-mesurer le crest
après tout changement de makeup/attack (`render_probe --song`).

## 2026-07-12 — Sons à longue résonance : mesurer les pitches UN PAR UN

`render_probe --pitches 78,84,90` sur la berceuse (release 1 s, decay
0.98) : la fenêtre d'analyse de chaque note contient encore la résonance
des précédentes. Concret : le « H2 de F#6 » lisait −18 dB alors qu'en
mono-pitch il vaut −26 — c'était le H4 de F#5 (2960 Hz, la même case) qui
sonnait encore. Règle : dès que la queue d'une note dépasse l'espacement
des notes de la probe, lancer UNE probe PAR pitch (ou espacer avec
--note-beats). Les chiffres multi-pitch de sons résonants sont pollués.

## 2026-07-12 — Cloche modale : le f0 mesuré ment (pitch virtuel)

`render_probe --module modal --set material=2` sur A4 : YIN lit 48,8 Hz
(clarté 0,89 !) alors que la cloche est frappée à 440 Hz — le cluster
inharmonique (hum 0.56·f, prime, tierce 1.19·f…) n'a pas de fondamental
physique et le tracker trouve un sous-harmonique commun. C'est le
comportement des vraies cloches (pitch virtuel), pas un bug. Pour juger un
son de cloche : mesurer les PARTIELS attendus (Goertzel à 0.56/1.19/1.56·f),
jamais le f0 rapporté.

## 2026-07-12 — Recette MVP : le pop du kick était dans le sample, pas dans le moteur

La probe d'export du démo (`render_probe --song assets/demo_song.json`)
signalait 3 clics/boucle sur la piste drums. Autopsie au niveau échantillon
(dump du WAV) : **les samples générés démarrent à chaud** (kick[0] = 0,36,
open_hat[0] = −0,88) — marche instantanée à chaque frappe. Deux fixes dans
`drum_sampler` (chacun testé) : micro-rampe d'attaque 1 ms par hit, et fondu
de declick 3 ms au retrigger d'un slot encore résonnant. Export du démo :
zéro défaut.

Découverte connexe, **limite connue à ce jour** : tous les instruments
consomment leurs événements de note AVANT de rendre le bloc (`ev.frame`
ignoré) → les notes partent au début du bloc qui les contient, jusqu'à
~10 ms tôt selon la taille de bloc. Inaudible aux tempos usuels, masqué par
les attaques des synthés ; le vrai fix (découper les blocs aux offsets
d'événements) est un chantier moteur post-recette.

## 2026-07-12 — Détecteur de clics : les fronts d'onde périodiques ne sont pas des glitches

Contexte : chantier palette. Une carrée brute (`subtractive wave=1
cutoff=20000`) déclenchait 467 « clics », une FM feedback/index élevés
pareil — le détecteur ne comparait chaque saut qu'à la MOYENNE des 25 ms
suivantes, que le front raide d'une onde brillante dépasse toujours de ×12.
Correctif dans `analysis/levels.rs` : un candidat doit aussi être un outlier
(> 4×) du PLUS GRAND saut de chaque côté (fenêtres de 25 ms) — un front qui a
des jumeaux est la forme d'onde, un glitch n'en a nulle part. Conséquences
assumées : un train de glitches identiques espacés < 25 ms n'est plus signalé
(ça se lit dans les autres métriques). Leçon générale (deuxième fois — cf.
granularité du scan spectral) : **quand une mesure contredit la théorie,
suspecter l'instrument de mesure** ; et pour tester du matériau brillant
continu, la borne de delta adjacent est le bon outil, pas `detect_clicks`.

## 2026-07-12 — Battement de deux oscillateurs : mesurer sur le fondamental isolé

`subtractive osc2=1 detune=30ct` : le battement RMS pleine bande ne varie
que de ×1,6 (chaque harmonique bat à sa propre vitesse n·Δf, l'ensemble ne
creuse jamais ensemble). Filtrer juste au-dessus du fondamental
(cutoff 300 Hz sur A3) → annulation quasi totale, ratio > ×2 net. Pour
entendre/mesurer un détune : viser le fondamental.

## 2026-07-11 — Reverb plate Dattorro : nouveaux repères

La reverb est passée de Freeverb à une plate Dattorro (modulée, +predelay).
Repères mesurés (mix=1, source subtractive) : size 0.3 → RT60 1.5 s ;
0.5 → 2.3 s ; 0.9 → 6.8 s (l'ancienne : 0.8/1.1/4.8). À réglages égaux la
plate sonne PLUS LONGTEMPS et son wet est plus maigre sur les sources
clairsemées — recettes existantes à ré-étalonner (voir berceuse-hantee :
size .82/mix .35 → size .55/mix .55/predelay 20). La queue d'une note tenue
garde sa structure harmonique (flatness −31.5 dB vs −12.9 pour l'ancienne =
moins de bouillie métallique).

## 2026-07-11 — Lissage OnePole f32 : cible en dizaines de milliers = erreur ~5

Un OnePole f32 qui glisse vers une grande cible (delay en échantillons :
33 000) cale à ~τ·ulp(cible)/2 ≈ 5 échantillons de la cible (l'incrément
par pas passe sous le quantum float). Inaudible (0,014 %), mais les tests
d'exactitude doivent tolérer ±8, pas ±2.

## 2026-07-11 — Boîte à musique = lame pincée : c'est `plucked`, pas `fm`

Contexte : v2 de `berceuse-hantee` (fm ratio entier) toujours « pas boîte à
musique » à l'oreille. L'utilisateur a fourni un enregistrement réel ;
empreinte d'une note isolée (F#6) : fondamentale écrasante (H2 −26 dB,
H3 ≤ −44), attaque 1 ms, résonance libre ≥ 2 s, et surtout **un plink
brillant qui s'éteint en tenue quasi pure** — un spectre qui ÉVOLUE dans la
note. La FM a un spectre figé ; Karplus-Strong (`plucked`) a exactement
cette physique (le filtre de boucle éteint les aigus à chaque tour).
Leçons :
- Vocabulaire : « boîte à musique / mobile / lame, tige, peigne » →
  `plucked` registre aigu (F#5–F#6), pas un module FM.
- **Quirk `plucked` : brightness < ~0.3 accumule un offset DC** dans la
  boucle (mesuré −0.013 à 0.1, défaut dur) et fausse le centroïde. Rester
  ≥ 0.3.
- **Quirk `plucked` : `release` coupe la résonance** — pour une lame qui
  sonne librement, release au max (1000 ms) et c'est `decay` qui gouverne.
- Manque au catalogue noté : **aucun effet filtre/EQ** — impossible
  d'amincir H2 de 7 dB pour coller parfaitement à la référence. Premier
  besoin concret d'un module `filter` (effet).
- Méthode : découper la référence en tranches de 2-3 s (VLC) et analyser la
  note la plus isolée (clarity ≈ 1) vaut mieux que mesurer la mélodie
  entière (mesures brouillées par le mécanisme/les recouvrements).

## 2026-07-11 — « Cloche » vs « boîte à musique » : le ratio FM entier fait tout

Contexte : v1 de `sounds/berceuse-hantee.md` rejetée à l'oreille (« vraiment
une cloche/sonnette, métallique ») alors que le brief voulait un mobile
musical pour bébé. La différence tient à UN paramètre : **ratio FM non
entier (3.53) = partiels désaccordés = métallique ; ratio entier (3) +
index < 1 = harmonique = doux/jouet**. Vocabulaire à retenir : « boîte à
musique / mobile / celesta » → harmonique (ratio entier, ratio harmonique
mesuré > 0.9) ; « cloche / gong / métallique » → inharmonique (ratio non
entier). La table du workflow classait « berceuse » trop vite côté cloche.

## 2026-07-11 — Granularité du scan de partiels (outil, corrigé)

Le scan Goertzel ±3 % en 9 pas quantifiait les déviations à ~13 cents :
inharmonicité et ratio harmonique étaient faux sur les sons aigus (ratio
harmonique 0.12 sur un son PARFAITEMENT harmonique). Corrigé dans
`analysis/spectral.rs` (scan grossier puis fin, ~1,6 cents ; harmonic_ratio
±3 bins). Leçon : quand une mesure contredit la théorie (ratio entier ⇒
harmonique), suspecter l'instrument de mesure avant le son.

## 2026-07-11 — Mesurer les cloches FM : trois pièges

Contexte : création de `sounds/berceuse-hantee.md` (fm ratio 3.53 + reverb).

1. **Index FM élevé = porteuse effondrée.** À index 3, la fondamentale
   disparaît quasiment (Bessel J0(3) ≈ −0.26) : f0 illisible, centroïde
   7.7 kHz strident. Pour une cloche *douce*, rester vers index 1–2 ;
   au-delà, c'est du métal agressif.
2. **f0 d'une cloche inharmonique lit souvent l'octave du dessous** (la
   quasi-période s'allonge). Vérifier la justesse via la clarity et le
   registre, pas via la valeur brute du f0.
3. **L'attaque mesurée en chaîne inclut la montée de la reverb** (~120 ms à
   mix 0.45). Pour vérifier le caractère percussif de l'instrument, mesurer
   À SEC (sans --fx) ; la version en chaîne mesure l'ambiance.
4. Bonus : l'inharmonicité « en cents » ne voit que les partiels proches de
   la grille k·f0 (scan ±3 %) ; pour une cloche franche, le **ratio
   harmonique bas** (< 0.6) est le vrai indicateur de non-harmonicité.

## 2026-07-11 — Empreintes par défaut des instruments (référence de départ)

Contexte : première passe de `render_probe` sur les 8 modules aux paramètres
par défaut (`cargo run --bin render_probe -- --module <id>`), 120 bpm, note
2 temps, vel 100.

Mesures :
- `plucked` : attaque ~0-1 ms (excitation instantanée), RT60 ≈ 0,09 s aux
  défauts — le son meurt vite ; pour un pluck qui « chante », allonger via
  `decay` ou un effet (delay/reverb).
- `subtractive` : attaque ≈ 7 ms, centroïde ≈ 1,6 kHz à cutoff 2000 Hz par
  défaut ; `--set cutoff=800` → centroïde ≈ 345 Hz (le centroïde suit bien le
  cutoff).
- `fm` : attaque ≈ 3 ms, f0 exact, clarity 1.00 aux défauts (ratio 1 = ton
  harmonique pur).
- `drum_sampler` : f0 = none (normal, non tonal), attaque ≈ 1 ms.
- Effets derrière `subtractive` : sec RT60 ≈ 0,17 s ; `+delay` ≈ 2,4 s ;
  `+reverb` ≈ 1,05 s (défauts). Le RT60 mesuré est le bon indicateur
  d'« ampleur ».

Conclusion : ces valeurs servent de point de départ aux boucles de
sound-design ; mesures seules (aucune écoute humaine) — statut plausible.
