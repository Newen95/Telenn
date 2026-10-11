# Contribuer à Telenn

Règles de développement du DAW : moteur, modules, interface. Les documents de
référence sont `ARCHITECTURE.md` (moteur, graphe, formats) et `UX_SPEC.md`
(vues, gestes, feedback) ; les lire avant toute feature significative.
`MVP_PLAN.md` décrit les jalons.

## Règles d'or (non négociables)

1. **Le thread audio est sacré.** Aucune allocation, aucun lock, aucune I/O, aucun
   appel système bloquant dans le callback audio. Communication uniquement par
   ring buffers lock-free (`rtrb`). Toute PR qui viole ça est refusée.
2. **Toute modification du graphe passe par le hot-swap.** On construit la nouvelle
   version côté contrôle, swap atomique côté audio avec crossfade ~10-20 ms.
   Jamais de mutation directe du graphe actif.
3. **Tout l'état projet est du texte.** `song.json` pour les séquences, presets en
   JSON, modules en Rust. Pas de format binaire. Tout doit être diffable par git.
4. **Un module = un fichier + un enregistrement.** Chaque instrument/effet implémente
   le trait `AudioNode` (voir ARCHITECTURE.md) et s'enregistre dans le registre de
   modules. Ajouter/retirer un module ne touche jamais au cœur du moteur.
5. **Flux de télémétrie unidirectionnel.** Le thread audio écrit (peaks, RMS,
   valeurs de modulation), l'UI lit à 60 fps. L'UI ne « demande » jamais rien au
   thread audio.
6. **Ports typés.** Trois types de signaux : `Notes` (événements MIDI), `Audio`
   (buffers), `Modulation` (control rate). Une connexion entre types incompatibles
   est une erreur de compilation ou de validation, jamais un comportement silencieux.

## Messages de commit

Chaque sujet commence par un tag de type, en anglais, sujet impératif minuscule
(le tag remplace le verbe) : `[ADD] audio graph engine`, pas
`[ADD] Add audio graph engine`.

| Tag | Quand |
|------------|-------|
| `[ADD]` | nouveau code/fichier/fonctionnalité |
| `[UPD]` | évolution d'un comportement existant |
| `[FIX]` | correction de bug |
| `[DEL]` | suppression |
| `[REFACTO]` | restructuration **sans changement de comportement** |
| `[DOC]` | documentation seule (`*.md`, commentaires, doc) |

`[UPD]` = le comportement change ; `[REFACTO]` = strictement iso-comportement.
Éviter les commits mixtes ; si inévitable, prendre le **tag dominant** (intention
principale) et détailler le reste dans le corps. Corps optionnel mais bienvenu
pour les commits conséquents (quoi + pourquoi).

## Structure du repo

```
/src-tauri/           # Rust
  /src/engine/        # thread audio : graphe, scheduler, hot-swap
  /src/modules/       # instruments & effets (1 fichier par module)
  /src/control/       # séquenceur, projet, MIDI in, export WAV, file watcher
  /src/telemetry/     # ring buffer audio -> UI
  /src/analysis/      # extraction de features offline (jamais sur le thread audio)
  /tools/             # CLIs de vérification sonore (rendu, mesure, lint)
/src/                 # Vue 3 + TS
  /views/timeline/    # canvas timeline + piano roll
  /views/graph/       # éditeur de nœuds (Vue Flow)
  /views/mixer/
  /stores/            # Pinia, miroir de l'état projet
/knowledge/           # base de connaissances : sons, styles, workflows
/projects/            # projets utilisateur (song.json, presets/) — gitignoré
```

## Commandes

- `npm run tauri dev` — lancer l'app en dev
- `./scripts/check.sh` — CI locale : fmt + clippy `-D warnings` + tests + build front
- `cargo test` (dans `src-tauri/`) — tests moteur
- `cargo bench` — benchs DSP (vérifier qu'un module tient le budget temps réel)
- `npm run test:unit` — tests frontend

Les CLIs de vérification sonore (`module_specs`, `render_probe`, `analyze_wav`,
`reference_match`, `transcribe`, `song_lint`, `song_stats`) sont décrits dans
`AGENTS.md`.

## Conventions

- **Langue du code : anglais.** Tout le code, les identifiants, les noms de
  fichiers, les commentaires, les messages de log et de commit sont en anglais.
  Seuls les textes destinés à l'utilisateur final (UI) suivent la règle i18n
  ci-dessous.
- **i18n : UI en français, traductions anglaises systématiques.** L'interface
  s'affiche en français par défaut, mais l'app doit pouvoir basculer en anglais à
  tout instant. Aucun texte visible n'est codé en dur : tout passe par les fichiers
  de traduction (`fr` et `en`). Chaque fois qu'on ajoute une chaîne française, on
  ajoute sa traduction anglaise dans le même commit — jamais l'une sans l'autre.
- **Factorisation.** Réutiliser le code existant quand c'est pertinent avant d'en
  écrire du nouveau ; extraire les motifs répétés. Pas de copier-coller qui dérive.
- **Code propre.** Architecture et code restent lisibles et cohérents avec
  l'existant. Refuser la dette gratuite ; préférer une structure claire à une
  optimisation prématurée.
- **Commentaires : utiles, pas verbeux.** Commenter ce que le code ne peut pas
  dire lui-même (invariants, raisons d'un choix non évident, contraintes RT). Ne
  pas paraphraser le code. Annotation `// RT-SAFE` obligatoire dans le chemin audio.
- Rust : `cargo fmt` + `clippy` sans warning. Pas de `unwrap()` hors tests.
- Le code du callback audio est annoté `// RT-SAFE` ; toute fonction appelée depuis
  le callback doit l'être aussi.
- Nouveau module audio : copier `src/modules/_template.rs`, implémenter `AudioNode`,
  exposer 4-8 macros bien nommées, ajouter au registre, écrire un test de
  non-régression sonore (rendu offline comparé à un WAV de référence ou à des
  propriétés spectrales).
- Les paramètres exposés à l'UI et aux macros sont déclarés dans le module
  (nom, plage, unité, courbe) — jamais codés en dur côté frontend.
- Messages UI -> moteur : commandes Tauri typées. Moteur -> UI : events Tauri +
  télémétrie. Pas d'état dupliqué : `song.json` est la source de vérité.
- **Identifiants de notes/clips.** Chaque note et chaque clip de `song.json` porte
  un `id` stable. Toute édition (UI, éditeur externe ou agent) **préserve** les
  `id` existants et n'en crée que pour les objets neufs ; ne jamais réutiliser,
  renuméroter ni recycler un `id`. Une note/clip chargée **sans** `id` s'en voit
  attribuer un, réécrit dans le fichier (`control/ids.rs`). Les éditions
  s'adressent toujours par `id`, jamais par position dans le tableau.

## Vision modulaire & ordre de construction

Cadre de référence pour **toute proposition d'évolution**. But : ne jamais ajouter
une couche de modularité avant d'en rencontrer le **besoin concret**.

**Fondation (en place, ne pas la changer).** Tout est `AudioNode`. Instruments et
effets ne sont pas deux catégories distinctes : ce sont des boîtes à **ports
typés** qui se câblent entre elles à chaque niveau. Un instrument a une entrée
`Notes` + une sortie `Audio` ; un effet une entrée `Audio` + une sortie `Audio`.
C'est la seule différence. Cette unification permet **un seul système de graphe
pour tout**.

**Deux granularités, ajoutées dans un ordre strict — ne jamais sauter d'étape :**

1. **MVP (maintenant) — modules monolithiques en Rust, rien d'autre.** Instruments
   et effets sont des boîtes Rust complètes (`subtractive.rs`, `plucked.rs`, etc.),
   posées en série dans le panneau chaîne. Pas de graphe à câbles, pas de patch
   JSON, pas de briques décomposées. On finit **M6, M7, M8** sur cette base. Pas
   de graphe modulaire, de sends, de modulations ni de patch visuel tant que le
   MVP (scénario-contrat) n'est pas atteint, même si l'occasion semble se
   présenter. Le panneau chaîne couvre 90 % de l'usage ; le graphe attend.
2. **Post-MVP #1 — graphe de routage entre modules (niveau piste uniquement).**
   Déclencheur concret : le **bus de reverb partagé** (sends de plusieurs pistes
   vers une même reverb) — premier cas où la chaîne linéaire ne suffit plus. On
   introduit Vue Flow et les câbles entre boîtes (modules restant fermés). La
   bascule chaîne → graphe est automatique dès qu'un routage non linéaire existe
   (déjà spécifié dans `UX_SPEC.md`).
3. **Ensuite — modulations.** LFO/enveloppes assignables pilotant des paramètres,
   câble de type `Modulation` (ambre), anneaux autour des knobs. Suppose le graphe
   de routage déjà en place.
4. **En dernier, optionnel — instrument-patch en JSON.** L'intérieur d'un
   instrument devient un graphe de données éditable visuellement (modèle « The
   Grid »). Le plus ambitieux (le moteur doit exécuter un graphe défini en données
   dans un nœud) et le moins nécessaire pour faire de la musique. Projet en soi, à
   n'envisager que si Telenn est solide et qu'on veut la lutherie visuelle.

**Règles transverses :**

- N'ajouter une couche de modularité que face à un **besoin concret rencontré**,
  jamais « parce que ce serait puissant ». La justification d'un ajout est un
  **blocage réel** que la couche débloque (reverb partagée → sends ; son qui bouge
  → modulation ; fabriquer un instrument visuellement → patch).
- Ne **pas décomposer** les instruments Rust en briques séparées tant qu'il n'y a
  pas d'usage réel du patch JSON. Un `subtractive.rs` monolithique n'est pas du
  travail perdu : le jour venu, on extraira oscillateur/filtre/enveloppe de ce code
  existant. Décomposer trop tôt complique sans bénéfice.
- **Catalogue petit et orthogonal.** Avant d'écrire une nouvelle brique primitive,
  vérifier qu'elle est vraiment primitive et pas une combinaison que les briques
  existantes expriment déjà. Chaque brique fait une chose et se combine bien.
- **Limite réelle = budget temps réel** (le graphe complet doit tenir dans le temps
  d'un buffer). Ajouter des briques au catalogue est libre ; en faire tourner trop
  **simultanément** ne l'est pas — à garder en tête pour les décisions de perf.

## Jalons

Le plan détaillé (jalons M0 à M8, fichiers à créer, critères d'acceptation,
pièges connus) est dans **MVP_PLAN.md** — c'est lui qui fait foi.

- Travailler **un jalon à la fois**, sur sa branche. Ne jamais entamer le
  suivant avec des critères d'acceptation rouges.
- Tout bug audio corrigé = un test de non-régression ajouté, sans exception.
- Toute modification du moteur est vérifiée par `scripts/check.sh`, les tests et
  les CLIs de `src-tauri/tools/` — jamais de rendu non mesuré.
