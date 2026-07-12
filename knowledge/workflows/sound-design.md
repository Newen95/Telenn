# Workflow : sound-design (créer/ajuster un son)

Procédure agnostique de l'agent. Objectif : ne jamais livrer un son « au
jugé » — chaque itération est **rendue puis mesurée** (`render_probe`), et
l'oreille humaine n'intervient qu'à la fin, sur un candidat déjà propre.

## 0. Consulter l'existant

Lire `knowledge/INDEX.md` : un son proche existe peut-être dans `sounds/`
(à réutiliser ou à dériver). Lire la fiche et repartir de son preset le cas
échéant.

## 1. Traduire le brief en cibles mesurables

| Adjectif du brief | Cible mesurable |
|---|---|
| brillant, cristallin | centroïde > 2 kHz |
| sombre, chaud, rond | centroïde < 800 Hz, H2–H3 dominants |
| percussif, claquant | attaque < 10 ms |
| nappe, doux, qui monte | attaque > 200 ms, sustain élevé |
| métallique, cloche, gong | inharmonicité > 10 cents (fm ratio NON entier) |
| boîte à musique, mobile, celesta, jouet | ratio harmonique > 0.9 (fm ratio ENTIER, index < 1), attaque < 10 ms |
| boisé, pincé | attaque < 20 ms, RT60 0,3–1 s, ratio harmonique > 0,8 |
| marimba, xylophone, boisé frappé | modal `material=0` (mode 3,9 mesurable), decay court |
| vibraphone, métallique tenu | modal `material=1`, decay 2–4 s |
| cloche réaliste, carillon | modal `material=2` — juger aux partiels (0.56/1.19·f), le f0 mesuré ment (pitch virtuel) |
| cristallin, verre | modal `material=3` |
| nappe qui s'ouvre/se referme | wavetable `pos_env` ±0.5–1, `pos_decay` 300–1500 ms (centroïde qui voyage) |
| soufflé, airy, bruité | flatness > −20 dB, ratio harmonique < 0,5 (subtractive `noise` + cutoff) |
| creux, clarinette, chiptune, jeu vidéo | harmoniques impaires seules : H2 ≤ H3 − 20 dB (subtractive `wave=1`, width 0.5) |
| nasillard, pincé du bout | H2 fort (subtractive `wave=1`, width 0.15–0.3) |
| doux, flûte, rond pur | harmoniques en 1/n² (subtractive `wave=2`) |
| épais, large, nappe qui vit | battement de niveau lent (subtractive `osc2=1`, detune 5–20 ct) |
| EP, cloche naturelle, pluck FM | centroïde qui DESCEND dans la note (fm `index_env` 0.6–1, `index_decay` 100–500 ms) |
| gras, riche, cuivré FM | fm `feedback` 0.2–0.6 (spectre vers la saw) |
| sec | RT60 < 0,3 s |
| ample, spacieux | RT60 > 1,5 s (delay/reverb dans la chaîne) |
| chaud, sale, saturé, agressif | crest qui baisse + harmoniques impaires (fx `saturator`, drive 6–30 dB) |
| qui bouge, animé, flottant | niveau/timbre ondulant lent (fx `chorus`, mix 0.2–0.5) |
| aminci, isolé, téléphone | bande sculptée (fx `filter` : type 1 hp / 2 bp / 3 notch) |

Poser 2 à 4 cibles chiffrées ; le reste est libre.

## 2. Choisir le point de départ

Arbre court :
- son tenu, filtré, classique (+ carrée/triangle/bruit/nappes détunées) →
  `subtractive`
- électrique, basse FM, EP, cloche synthétique → `fm` (ratio entier =
  harmonique, non entier = inharmonique ; `index_env` = spectre vivant)
- corde pincée, harpe, guitare, boîte à musique → `plucked`
- **barre frappée : marimba, vibraphone, cloche réaliste, verre → `modal`**
  (le complément frappé de plucked ; `material` choisit la physique)
- **nappe qui voyage, basse digitale, spectre morphé → `wavetable`**
  (`position` = sine→triangle→saw→carrée→vox, `pos_env` = le voyage)
- percussion batterie → `drum_sampler` (+ effets)

Puis `cargo run --bin module_specs` pour les plages réelles exactes et les
quirks (drum map, threshold du compresseur…).

## 3. Boucle itérative (max 8 tours)

```
cargo run --bin render_probe -- --module <id> --set <param>=<réel> [--fx <id> --set <fx>.<param>=<réel>] --json <scratch>/probe.json
```

À chaque tour : comparer chaque mesure à sa cible, ajuster UN levier à la
fois. Leviers principaux :

| Pour bouger… | Agir sur… |
|---|---|
| centroïde ↑/↓ | `cutoff` (subtractive/filter), `index` (fm), `brightness` (plucked/modal), `position` (wavetable), `tone` (delay/saturator) |
| attaque | `attack` (ms, subtractive/fm) — plucked/drums sont instantanés par nature |
| RT60 | `release`, `decay` (plucked), `size` (reverb), `time`+`feedback` (delay) |
| flatness ↑ (plus de souffle) | `noise` (subtractive), `index` (fm) ↑, résonance ↑ |
| harmoniques paires/impaires | `wave`+`width` (subtractive) — carrée = impaires seules |
| évolution du timbre dans la note | `index_env`+`index_decay` (fm), `pos_env`+`pos_decay` (wavetable) — plucked/modal l'ont par physique |
| dureté de l'attaque frappée | `strike` (modal — le chiff du maillet) |
| densité/saleté | `feedback` (fm), `drive` (saturator) |
| épaisseur/mouvement | `osc2`+`detune` (subtractive), `rate`+`depth` (chorus) |
| niveau | `level` (dB) — viser un peak de probe entre −12 et −6 dBFS |

**Critères d'arrêt** (tous requis) :
1. chaque cible atteinte — tolérances : centroïde ±20 %, attaque ±30 %,
   RT60 ±25 %, flatness ±5 dB ;
2. exit code 0 (zéro défaut : ni clip, ni clic, ni DC, ni silence) ;
3. ≤ 8 itérations. Au-delà : consigner l'impasse dans `lessons.md` avec la
   meilleure config atteinte, et demander un arbitrage humain.

## 4. Valider sur tout le registre utile

```
cargo run --bin render_probe -- --module <id> --set <…> --pitches <grave>,<médium>,<aigu>
```

f0 juste (±10 cents) et zéro défaut aux trois registres ; si le son s'écroule
dans un registre, le noter dans la fiche (registre utile).

## 5. Enregistrer

1. Créer la paire `knowledge/sounds/<nom>.md` + `<nom>.preset.json` d'après
   `sounds/_template.md` (params **normalisés** dans le JSON, table réel ⇄
   normalisé dans la fiche, commande de mesure exacte).
2. Mettre à jour `knowledge/INDEX.md` (même commit).
3. Statut « plausible » tant que l'utilisateur n'a pas écouté ; son verdict
   (validé / retouches) se date dans la fiche.
4. Pour rendre le son disponible dans l'app (navigateur → Instruments, menu
   « + piste »), écrire un **preset de chaîne** dans
   `projects/<projet>/presets/chains/<nom>.json` :
   `{ "nodes": [ { "module": …, "params": {…} }, … ] }` — `nodes[0]` =
   instrument, la suite = effets, params normalisés. Données projet, non
   versionnées.
