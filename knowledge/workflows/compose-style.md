# Workflow : compose-style (composer dans un style)

Procédure agnostique de l'agent pour « fais-moi un morceau à la manière de
X ». S'appuie sur `workflows/sound-design.md` pour les sons manquants.

## 1. Fiche de style d'abord

- Chercher `knowledge/styles/<style>.md` via `INDEX.md`. Si absente : la
  créer d'après `styles/_template.md` (recherche web + connaissances
  propres), **tout en grandeurs actionnables** (BPM, placements
  `bar.beat.tick`, degrés harmoniques, cibles mix chiffrées). Si présente :
  la relire et l'enrichir si la recherche apporte du neuf.
- Garde-fou : une fiche décrit des **patterns** (règles du style), jamais la
  mélodie d'un morceau existant — s'inspirer d'un style oui, recopier une
  œuvre non.

## 2. Analyse d'écart (sons)

Lister les rôles requis par la fiche (basse, lead, batterie…) et regarder
dans `knowledge/sounds/` (via INDEX) ce qui existe. Chaque son manquant passe
par le workflow sound-design **avant** de composer ; les presets validés se
copient dans `projects/<p>/presets/`.

## 3. Composer dans `song.json`

Éditer `projects/<p>/song.json` (l'app recharge à chaud). Rappels bloc :
- positions `"bar.beat.tick"` **1-indexées**, durées **0-indexées**, PPQ 960 ;
- ids stables : ne jamais réutiliser/renuméroter un id existant, en créer
  seulement pour les objets neufs (sinon laisser vide, l'app assigne) ;
- params de chaîne **normalisés 0..1** (plages réelles : `module_specs`) ;
- drum map : 36 kick, 38 snare, 39 clap, 42 ch, 45 lt, 46 oh, 49 crash,
  50 ht — tout autre pitch est muet.

Puis valider **avant de considérer l'édition terminée** :

```
cargo run --bin song_lint -- <chemin>/song.json
```

Boucler jusqu'à exit 0.

## 4. Boucle de mix

```
cargo run --bin render_probe -- --song <chemin>/song.json --png <scratch>/mix.png --json <scratch>/mix.json
```

- Comparer aux cibles mix de la fiche style (peak, crest, équilibre par piste
  — soloter avec `--track <id>`).
- Ajuster `mixer.gain_db`/`pan` dans `song.json` (voir
  `techniques/mixing.md`), jamais le code.
- Arrêt : exit 0 **et** cibles de la fiche atteintes.

## 5. Verdict humain

Faire écouter. Dater le retour dans la fiche style (section Verdicts) ; toute
leçon générale va dans `lessons.md`. La fiche enrichie est le vrai livrable
de long terme : la prochaine demande « à la X » repart de là.
