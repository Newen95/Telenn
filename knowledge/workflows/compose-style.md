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

## 1 bis. Transcrire les patterns réels (si références audio fournies)

```
cargo run --bin transcribe -- <réf.wav> [--start-s a --dur-s n] [--json p]
```

Sort la ligne de basse note à note, le pattern de batterie et la grille
d'accords par mesure, positions `bar.beat.tick` sur une grille inférée
(tempo/phase/downbeat ; `--bpm`/`--offset-s` pour corriger une inférence
ratée). Usage :

- **Nourrir la fiche en patterns** : registre et densité de la basse,
  grammaire des enchaînements (degrés, pas des intervalles), squelette
  rythmique — toujours résumés en règles dans la fiche, **jamais** les
  lignes transcrites recopiées telles quelles dans une composition (le
  garde-fou ci-dessus s'applique à plus forte raison à une transcription).
- **Limites connues** : couches accessibles seulement (basse dominante,
  batterie, accords — pas les voix ni les arpèges noyés) ; sur un mix à
  ostinato de basse continu, le canal kick sur-détecte (basse et kick
  partagent la bande 40-150 Hz) → transcrire la batterie sur un **segment
  exposé** (intro/break, `--start-s`/`--dur-s`) ; grille rigide (tempo
  constant), `raw_s`/`quant_error_ticks` exposent la dérive.

## 1 ter. Séparer les stems des références (préprocessing externe)

Les outils maison comparent mal un stem à un mix complet (bandes relatives,
transcription batterie polluée par la basse). Solution : séparer les
références en stems UNE fois, hors app — même statut que la conversion VLC
d'un MP3, jamais une dépendance du cœur. Demucs vit dans un venv dédié :

```
%USERPROFILE%\.telenn\demucs-venv\Scripts\demucs.exe -o <dossier> <réf.wav> […]
```

Sortie : `<dossier>/htdemucs/<titre>/{bass,drums,other,vocals}.wav`
(CPU ~1.5× temps réel). Ensuite : `analyze_wav`/`transcribe` sur le stem
`bass` (f0, enveloppe, timbre réels), `transcribe` sur `drums` (pattern sans
la limite kick/basse), `reference_match` stem contre piste solo du rendu
(diff de timbre enfin comparable), extraits cibles propres pour
l'appariement de sons. Les stems restent hors repo (fichiers dérivés
d'œuvres protégées : analyse privée uniquement, ne jamais les committer ni
les échantillonner dans une composition).

Limites constatées à l'oreille (2026-07-16, stems Mareux) : `other` est le
meilleur stem — mélodie principale/lead bien isolée avec quelques sons
parasites (ex. le lead de LFTP net sur `other` 2:15-3:00) ; les **voix sont
la plupart du temps perdues** (pas captées du tout) et, quand elles le sont,
elles atterrissent bien dans `vocals` — cas piège : sur Summertime la
mélodie principale est partie dans `vocals` (le séparateur l'a prise pour
une voix — vérifier les deux stems pour trouver le lead) ; `drums` est bien
isolé ; le stem `bass` sonne dégradé (artefacts « sous-marins ») — bon pour
la **structure** (enveloppe, grille, registre, répartition d'énergie), pas
comme cible de timbre fine sans recouper avec la bande grave du mix.

## 2. Analyse d'écart (sons)

Lister les rôles requis par la fiche (basse, lead, batterie…) et regarder
dans `knowledge/sounds/` (via INDEX) ce qui existe. Chaque son manquant passe
par le workflow sound-design **avant** de composer ; les presets validés se
copient dans `projects/<p>/presets/`.

## 2 bis. Sources échantillonnées (quand le moteur ne suffit pas)

Un timbre acoustique (guitare, piano, cuivre) coûte cher à synthétiser et
n'apporte rien au moteur. Le module `sampler` le couvre : une banque SF2
libre (ex. **GeneralUser GS**, `~/.telenn/soundfonts/`) rendue en zones WAV,
une par note racine tous les 3-4 demi-tons, dans `projects/<p>/samples/<nom>/`
(`sf2_zones.py` : audition **par la mesure** — attaque, sustain à 1 s,
centroïde — puis rendu). Le module `drum_sampler` prend de la même façon un
répertoire `kit` d'one-shots. Règles :

- la banque et les samples restent **hors dépôt** (`projects/` est gitignoré) ;
  la fiche de style note quel preset a été retenu et pourquoi, pas le binaire ;
- le caractère vient de la **chaîne** (ampsim, chorus, delay, reverb), pas du
  sample brut — un preset GM nu sonne GM ;
- un one-shot qui manque de corps se **synthétise** plutôt que se remplacer
  (cf. le kick sub de la fiche Mareux) ; on le règle sur un profil mesuré
  (glissando, décroissance, parts de bandes), jamais à l'oreille seule ;
- ne jamais échantillonner une référence protégée — les stems servent de
  **cible de mesure**, pas de matière première.

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

## 3 bis. Garde anti-plagiat (si références transcrites)

```
cargo run --bin song_stats -- <chemin>/song.json --vs <transcription.json> […]
```

**Exit 0 requis** avant de passer au mix. Deux niveaux de signal :

- `DEFECT` (run d'intervalles+rythme ≥ 8 partagé avec une référence, ou
  similarité ≥ 0.85) = **preuve dure** : réécrire la ligne concernée, le
  rapport donne la position du passage des deux côtés ;
- `WARN` = à juger : le **plancher de nouveauté** (avec ≥ 2 références)
  vérifie qu'une ligne composée reste au moins aussi loin de chaque
  référence que les références le sont entre elles — plus proche = on
  imite un morceau, pas le style. `--strict` promeut les WARN en exit 2.

Sans `--vs`, l'outil sort les métriques seules (densité, registre,
intervalles, grille rythmique, auto-répétition) — utile pour comparer sa
composition aux grandeurs de la fiche. Le garde-fou de l'étape 1
(« s'inspirer oui, recopier non ») devient ainsi vérifiable chiffré ; il
trouve des preuves, il ne prouve pas l'absence de copie — l'écoute humaine
reste le dernier juge.

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
