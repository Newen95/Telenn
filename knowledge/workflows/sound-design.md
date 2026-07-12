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
| soufflé, airy, bruité | flatness > −20 dB, ratio harmonique < 0,5 |
| sec | RT60 < 0,3 s |
| ample, spacieux | RT60 > 1,5 s (delay/reverb dans la chaîne) |

Poser 2 à 4 cibles chiffrées ; le reste est libre.

## 2. Choisir le point de départ

Arbre court :
- son tenu, filtré, classique → `subtractive`
- cloche, métal, électrique, basse FM → `fm` (ratio entier = harmonique,
  non entier = inharmonique)
- corde pincée, harpe, guitare → `plucked`
- percussion → `drum_sampler` (+ effets)

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
| centroïde ↑/↓ | `cutoff` (subtractive), `index` (fm), `brightness` (plucked), `tone` (delay) |
| attaque | `attack` (ms, subtractive/fm) — plucked/drums sont instantanés par nature |
| RT60 | `release`, `decay` (plucked), `size` (reverb), `time`+`feedback` (delay) |
| flatness ↑ (plus de souffle) | `index` (fm) ↑, résonance ↑ |
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
