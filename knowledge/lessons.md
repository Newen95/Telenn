# Leçons transverses

Journal daté, du plus récent au plus ancien. Format : `## AAAA-MM-JJ — titre`
puis contexte, mesure (commande exacte), conclusion. Les impasses se
consignent au même titre que les réussites.

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
