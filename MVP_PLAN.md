# MVP_PLAN.md — Plan d'implémentation guidé

## Définition du MVP (le contrat)

Le MVP est atteint quand ce scénario fonctionne de bout en bout, sur Windows ET Linux :

> J'ouvre l'app, je crée 3 pistes (drums, bass, lead) avec des instruments
> intégrés, je pose des notes au piano roll (chaque note sonne quand je la pose),
> j'ajuste la chaîne d'effets d'une piste par drag & drop **pendant la lecture**
> sans interruption du son, je règle volumes/pans, j'exporte en WAV.
> Pendant ce temps, Claude Code modifie `song.json` et je vois/entends ses
> changements apparaître en direct.

**Dans le MVP** : moteur graphe + hot-swap, séquenceur, 3 instruments
(soustractif, FM, drums-sampler), 4 effets (filtre, delay, reverb, compresseur),
mixeur (gain/pan + master), timeline + piano roll canvas, panneau chaîne avec
drag & drop, VU-mètres + playhead fluide, export WAV, file watcher.

**Hors MVP (post-MVP, dans cet ordre)** : entrée MIDI live, éditeur de graphe
libre (Vue Flow) + modulations + macros, câbles animés, pré-écoute des presets,
CLAP. *Justification : le panneau chaîne couvre 90 % de l'usage et partage son
modèle de données avec le futur graphe libre — on ne jette rien.*

## Règles de conduite du plan

- Un jalon = une branche git = un objectif vérifiable. On ne passe au suivant
  que si TOUS les critères d'acceptation (CA) sont verts.
- Les CA du moteur sont des tests automatisés ; les CA d'UI sont des checklists
  manuelles chronométrées.
- Chaque jalon liste ses **pièges** : les lire avant de coder, pas après.

---

## M0 — Bootstrap (½ journée)

**Objectif** : squelette qui compile et se lance sur les deux OS.

**Étapes**
1. `npm create tauri-app@latest` → template Vue + TypeScript, Tauri v2.
2. Côté Rust : `cargo add cpal rtrb hound serde serde_json thiserror` et
   `cargo add notify` (file watcher), `cargo add midir` (utilisé en post-MVP,
   mais on fige la dépendance). Ne pas épingler de versions à la main :
   prendre les dernières stables.
3. Créer l'arborescence vide de CONTRIBUTING.md (`engine/`, `modules/`, `control/`,
   `telemetry/`) avec des `mod.rs` et un test bidon par dossier.
4. CI locale : script `scripts/check.sh` = `cargo fmt --check && cargo clippy -- -D
   warnings && cargo test && npm run build`.

**CA**
- [ ] `npm run tauri dev` ouvre une fenêtre sur Windows et Linux.
- [ ] `scripts/check.sh` passe.

---

## M1 — Le bip (1-2 jours)

**Objectif** : un thread audio temps réel qui sort un son contrôlé depuis l'UI.

**Fichiers**
- `engine/stream.rs` — ouverture du device cpal, callback audio.
- `engine/rt.rs` — état possédé par le callback (`RtState`).
- `control/commands.rs` — commandes Tauri `transport_play`, `transport_stop`.

**Étapes**
1. Ouvrir le device par défaut en f32, sample rate natif, buffer demandé
   256 frames. Stocker le sample rate réel obtenu (jamais le supposer).
2. Dans le callback : générer une sinusoïde 440 Hz par accumulateur de phase
   (`phase += freq / sr`). Gain master avec **rampe linéaire de 10 ms** sur
   play/stop (sinon clic).
3. Canal contrôle → audio : `rtrb::RingBuffer<ControlMsg>` (cap. 1024). Le
   callback draine TOUS les messages en début de bloc, puis remplit le buffer.
4. Bouton Play/Stop dans l'UI Vue → commande Tauri → push dans le ring buffer.
5. Compteur de xruns : cpal signale les erreurs de flux via le error callback ;
   incrémenter un `AtomicU32` lisible par l'UI.

**CA**
- [ ] Test automatisé : le callback ne contient aucun appel à `alloc` (test de
      non-régression : exécuter le rendu d'un bloc sous un allocateur de garde
      qui panique — voir `engine/tests/rt_safety.rs`).
- [ ] Play/Stop sans clic audible (rampe vérifiée par test : aucun delta
      inter-échantillon > 0,05 sur la sortie capturée).
- [ ] 10 minutes de lecture à buffer 256 sans xrun, sur les deux OS.

**Pièges**
- WASAPI peut imposer un sample rate différent de 48 kHz : toujours lire la
  config réelle du stream.
- Ne JAMAIS logger (`println!`) dans le callback — c'est une syscall.
- Le drop du stream cpal doit se faire hors du callback (évident, mais
  fréquent dans les handlers de fermeture Tauri).

---

## M2 — Le graphe v1 + premier synthé (3-5 jours)

**Objectif** : le trait `AudioNode`, un graphe linéaire compilé, un synthé
soustractif jouable par messages, des paramètres lissés.

**Fichiers**
- `engine/node.rs` — trait `AudioNode`, `ParamSpec`, `PortSpec` (voir
  ARCHITECTURE.md §Spécifications).
- `engine/graph.rs` — `GraphDesc` (description sérialisable) et
  `CompiledGraph` (ordre topo à plat + buffers pré-alloués).
- `engine/params.rs` — lisseur : rampe linéaire 10 ms pour les gains, filtre
  1 pôle (coef ~0,995 à control rate) pour le reste.
- `modules/_template.rs` — module de référence ultra-commenté.
- `modules/subtractive.rs` — saw polyBLEP → biquad LP (cookbook RBJ) → ADSR.
  8 voix de polyphonie, vol de voix simple (plus ancienne).
- `modules/registry.rs` — `fn create(module_id: &str) -> Box<dyn AudioNode>`.

**Étapes**
1. Définir `ControlMsg` (enum complet en ARCHITECTURE.md) et router les
   `NoteOn/NoteOff/SetParam` vers les nœuds via index plat.
2. `CompiledGraph::process_block()` : itérer l'ordre topo, copier/mixer les
   buffers selon les connexions. Dans M2 le graphe est une chaîne, mais
   l'implémentation est déjà générique (vec de connexions).
3. Le lissage de paramètres vit dans le nœud (chaque param a sa valeur cible
   et sa valeur courante) ; `set_param` ne fait que poser la cible.
4. Bouton de test dans l'UI : un mini-clavier d'une octave (boutons) qui envoie
   NoteOn/NoteOff. Slider cutoff branché sur `SetParam`.

**CA**
- [ ] Test : balayer le cutoff de 200 Hz à 8 kHz pendant une note tenue,
      capturer la sortie, vérifier l'absence de discontinuité (pas de
      zipper noise : delta inter-échantillon borné).
- [ ] Test : 8 NoteOn simultanés → 8 voix actives, le 9e vole la plus ancienne.
- [ ] Test rt_safety (M1) passe toujours avec le graphe complet.
- [ ] À l'oreille : l'attaque/release de l'ADSR ne clique pas (release ≥ 5 ms
      même à 0).

**Pièges**
- Le polyBLEP est obligatoire pour la saw : une saw naïve aliase atrocement
  dès 1 kHz. Ne pas « faire simple d'abord » ici, c'est la qualité sonore.
- Dénormaux : ajouter un DC minuscule (1e-15) dans les feedbacks de filtres ou
  activer flush-to-zero, sinon le CPU explose sur les queues de release.
- Les buffers du graphe sont allous dans `prepare()`/compilation, indexés par
  des `usize` précalculés. Aucune `HashMap` dans le chemin chaud.

---

## M3 — Séquenceur + song.json + export WAV (3-4 jours)

**Objectif** : lire un projet texte, le jouer, l'exporter. La source de vérité
devient le fichier.

**Fichiers**
- `control/project.rs` — structs serde miroir de `song.json` (schéma en
  ARCHITECTURE.md), validation (refs de modules existants, positions valides).
- `control/sequencer.rs` — conversion ticks→samples, événements triés,
  curseur de lecture, boucle.
- `control/render.rs` — rendu offline vers `hound` (WAV f32 ou i16 au choix).
- `projects/demo/song.json` — projet de démo : 2 pistes, 8 mesures.

**Étapes**
1. Horloge : le **compteur d'échantillons est maître**. Ticks PPQ 960 ;
   `samples_per_tick = sr * 60 / (bpm * 960)`. Le séquenceur précalcule une
   liste d'événements `(sample_pos, msg)` triée, consommée bloc par bloc.
2. Les événements dont la position tombe dans le bloc courant sont émis avec
   leur **offset intra-bloc** (le trait `process` reçoit les événements datés ;
   pour M3, arrondir au début de bloc est acceptable, l'offset précis est
   un raffinement noté TODO).
3. Le transport (play/stop/seek/loop) vit côté contrôle ; le thread audio ne
   connaît que « voici les événements de ce bloc ».
4. Export : instancier un second `CompiledGraph` identique, le dérouler hors
   temps réel, écrire le WAV. Aucune logique dupliquée avec le temps réel.
5. Commandes Tauri : `load_project(path)`, `export_wav(path)`.

**CA**
- [ ] Test déterminisme : rendre 2 fois le projet démo offline → fichiers
      identiques bit à bit.
- [ ] Test équivalence : rendu offline ≈ capture du temps réel (corrélation
      > 0,999 ; les horloges différentes interdisent le bit-à-bit).
- [ ] Le projet démo joue en boucle sans dérive (position après 100 boucles
      = position théorique, à l'échantillon près).
- [ ] `song.json` modifié à la main (note ajoutée) + `load_project` → on
      entend la note.

**Pièges**
- Ne jamais accumuler le temps en flottant (dérive) : tout en u64 d'échantillons,
  conversions à la demande.
- Le BPM est global et fixe dans le MVP. Noter dans le code l'endroit unique
  où une automation de tempo s'insérerait plus tard.

---

## M4 — Hot-swap (3-4 jours) — *le jalon le plus important*

**Objectif** : modifier le graphe pendant la lecture sans aucune interruption
audible. Toute l'UX modulaire repose dessus.

**Fichiers**
- `engine/swap.rs` — double graphe + crossfade.
- `control/graph_edit.rs` — opérations d'édition (`add_node`, `remove_node`,
  `move_node`, `reconnect`) qui produisent un nouveau `GraphDesc`.
- `control/undo.rs` — pile d'undo = pile de `GraphDesc` + diffs de projet.

**Étapes**
1. Côté contrôle : cloner `GraphDesc`, appliquer l'édition, compiler
   (`CompiledGraph` complet : prepare(), buffers, ordre topo).
2. **Réutilisation d'état** : pour chaque nœud du nouveau graphe présent dans
   l'ancien (même `node_id`), déplacer (`std::mem::swap`) l'instance existante
   au lieu d'en créer une neuve — préserve queues de reverb, phases, voix.
   Cette étape se fait côté contrôle, le thread audio étant notifié ensuite.
   ⚠️ Pour le MVP, simplification sûre : geler les éditions du nœud en cours
   de déplacement pendant 1 bloc ; l'état des AUTRES nœuds est transféré via
   le mécanisme suivant.
3. Envoi du nouveau graphe par `ControlMsg::SwapGraph(Box<CompiledGraph>)`
   (transfert de propriété, zéro copie en RT).
4. Dans le callback : pendant N = 0,015 s × sr échantillons, exécuter ancien
   ET nouveau graphes, sortie = equal-power crossfade
   (`out = cos(t·π/2)·old + sin(t·π/2)·new`). Puis pousser l'ancien graphe
   dans un ring buffer « poubelle » audio→contrôle, où il sera droppé hors RT.
5. Budget CPU : pendant le crossfade la charge double. Mesurer ; si un swap
   provoque des xruns à buffer 256, réduire la fenêtre à 8 ms avant d'optimiser.

**CA**
- [ ] Test continuité : pendant un rendu, déclencher add/remove/move d'un effet
      toutes les 0,5 s ; sur la capture, aucun delta inter-échantillon > 0,1 et
      aucun bloc de silence inattendu.
- [ ] Test préservation d'état : note tenue + queue de reverb → déplacer la
      reverb d'une position dans la chaîne → la queue ne s'interrompt pas
      (énergie de sortie continue).
- [ ] Test poubelle : aucun `drop` de graphe sur le thread audio (compteur
      d'allocations/désallocations de garde).
- [ ] Undo/redo de 50 éditions de graphe consécutives → état final identique
      à l'initial (comparaison des `GraphDesc`).

**Pièges**
- Le crossfade equal-power, pas linéaire (creux de -3 dB au milieu sinon).
- Si deux swaps arrivent pendant un crossfade : le second remplace la cible,
  on ne crossfade jamais plus de 2 graphes (le cas « rafale de drag & drop »).
- L'ancien graphe peut contenir des nœuds lourds : le ring poubelle doit être
  dimensionné (cap. 8) et drainé en continu côté contrôle.

---

## M5 — Timeline + piano roll canvas (5-8 jours)

**Objectif** : les trois interactions reines à 60 fps, plus le premier slice
vertical de télémétrie (faire passer un chiffre du thread audio jusqu'au pixel,
de bout en bout — on prouve le pipeline tôt, sur une donnée, plutôt que de le
découvrir en entier à M7).

**Fichiers**
- `views/timeline/TimelineCanvas.vue` + `renderer.ts` — un seul canvas,
  rendu immédiat (pas de DOM par clip/note), viewport {scrollX, zoomX} en
  ticks/pixel.
- `views/timeline/interactions.ts` — machine à états des gestes (idle →
  hover → drag{move|resize|create} → commit/cancel).
- `views/pianoroll/` — même architecture, axe Y = pitch (hauteur de rangée
  fixe, 128 rangées virtualisées).
- `stores/project.ts` — miroir Pinia de `song.json` ; toute mutation passe
  par des actions qui appellent les commandes Tauri (source de vérité côté
  Rust, l'UI est optimiste puis confirmée).
- `telemetry/frame.rs` — `TelemetryFrame` minimal `{playhead_samples,
  master_peak}`, écrit chaque bloc par le thread audio dans un `rtrb` dédié
  (jamais d'allocation, écrase si plein). // RT-SAFE
- `control/telemetry_pump.rs` — draine le ring, agrège (max des peaks), pousse
  un event Tauri `telemetry` à ~60 Hz maximum.
- `views/transport/MasterMeter.vue` — VU master (peak, ballistique : attaque
  instantanée, release 300 ms), alimenté par l'event `telemetry`.

**Étapes**
1. Renderer : boucle `requestAnimationFrame` qui ne redessine que si
   (état dirty || playhead bouge). Culling : ne dessiner que les clips/notes
   dans le viewport. Hit-testing par recherche dans une structure triée par
   position (les notes d'un clip sont triées par start).
2. Gestes (cf. UX_SPEC tableau) : implémenter dans l'ordre — création de note
   au double-clic (avec écho audio : envoyer NoteOn/Off de 150 ms), drag de
   note (snap adaptatif au zoom, Alt désactive), resize par les bords (zone
   de 6 px), drag de clip, resize de clip.
3. Chaque geste committé = une commande Tauri + une entrée d'undo. Pendant le
   drag, l'UI affiche la position fantôme localement ; le commit ne se fait
   qu'au relâchement.
4. **Slice vertical de télémétrie minimal** — faire passer une seule donnée du
   thread audio jusqu'au pixel, de bout en bout. Le thread audio écrit chaque
   bloc un `TelemetryFrame {playhead_samples, master_peak}` dans le ring `rtrb`
   (jamais d'allocation, écrase si plein) ; le pump le draine et émet l'event
   Tauri `telemetry` à ~60 Hz ; l'UI interpole le playhead entre deux frames
   (`pos + vitesse × dt`) pour un déplacement parfaitement fluide et anime le VU
   master sur `master_peak`. Rien de plus : c'est le pipeline complet prouvé sur
   une donnée, pas encore le DAW vivant (qui l'élargit à M7).

**CA (manuels chronométrés + un test perf)**
- [ ] Poser une note : double-clic → la note apparaît ET sonne en < 30 ms
      perçus ; snap correct à 3 niveaux de zoom différents.
- [ ] Déplacer une note/clip : aucun lag visible, Échap annule, Ctrl+Z rétablit.
- [ ] Test perf automatisé : scène de 5 000 notes, scroll + zoom scriptés →
      frame time p95 < 16 ms (mesuré via `performance.now()` dans le renderer).
- [ ] Resize d'un clip ne déplace jamais les notes qu'il contient (positions
      absolues conservées).
- [ ] Le playhead se déplace de façon fluide et le VU master réagit au signal,
      alimentés de bout en bout par le pipeline de télémétrie (audio → ring →
      pump → event → UI), sans allocation côté audio.

**Pièges**
- Un composant Vue par note = mort à 500 notes. Canvas immédiat obligatoire.
- `devicePixelRatio` : dimensionner le canvas en pixels physiques sinon flou.
- Ne pas brancher le store Pinia en réactif profond sur le renderer (coût
  énorme) : le renderer lit des structures plates, invalidées par événement.
- **i18n dès le premier composant.** Installer `vue-i18n` à ce jalon (première
  UI réelle). Aucun texte visible codé en dur : toujours `t('transport.play')`,
  jamais `"Lecture"`. Fournir le français maintenant ET sa traduction anglaise
  dans le même commit ; structurer les clés pour ajouter `en` (et d'autres
  langues plus tard) sans refactor. Rétrofitter l'i18n sur une UI déjà écrite en
  dur est pénible ; le faire d'emblée ne coûte presque rien.

---

## M6 — Panneau chaîne + drag & drop + presets (4-6 jours)

**Objectif** : l'UX modulaire — réordonner/déplacer des nœuds entre pistes
pendant la lecture.

**Fichiers**
- `views/chain/ChainPanel.vue` — chaîne de la piste sélectionnée : cartes de
  nœuds (nom, type, bypass, VU placeholder), rendue en DOM (peu d'éléments,
  pas besoin de canvas ici).
- `views/chain/dragdrop.ts` — machine à états du drag global (source, cibles
  valides, interstice survolé, annulation), avec un overlay plein écran
  pendant le drag pour capturer les events.
- `views/browser/PresetBrowser.vue` — liste des modules du registre + presets
  JSON du projet.
- `control/commands.rs` — `move_node{from_track,to_track,index}`,
  `save_preset`, `load_preset`.

**Étapes**
1. Le registre Rust expose `list_modules()` (id, nom, catégorie, params) ;
   le browser l'affiche. Glisser un module sur une piste = `add_node` (en fin
   de chaîne) ; sur un interstice = insertion à l'index.
2. Pendant le drag : toutes les cibles valides s'illuminent (classe CSS),
   l'interstice survolé s'écarte (transition 120 ms sur margin).
3. Drop entre pistes = UNE édition de graphe (move), donc UN hot-swap, donc
   un seul crossfade. Alt au drop = copie (nouveau node_id, mêmes params).
4. Échap ou drop hors cible = animation de retour (translate vers l'origine,
   150 ms) + aucune commande émise.
5. `save_preset` : sérialise `{module, params}` dans `projects/<p>/presets/`.

**CA**
- [ ] Déplacer une reverb de la piste A à la piste B **pendant la lecture** :
      le son ne s'interrompt pas, la queue de reverb suit (cf. M4).
- [ ] Réordonner 2 effets dans une chaîne : un seul crossfade audible… c'est
      à dire aucun artefact.
- [ ] Drag annulé (Échap) : retour animé, état strictement inchangé (vérifier
      le GraphDesc).
- [ ] Un preset sauvé puis rechargé reproduit le son (params identiques).

**Pièges**
- HTML5 drag & drop natif est inutilisable pour ça (ghost image, events
  incohérents) : pointer events + overlay maison.
- Pendant un drag, désactiver le hover du canvas timeline (sinon conflits de
  curseur).

---

## M7 — Le DAW vivant : télémétrie, VU, file watcher (3-4 jours)

**Objectif** : feedback visuel temps réel complet + boucle Claude Code en direct.
**Élargissement** du pipeline de télémétrie déjà éprouvé en M5 : le fil vertical
`audio → ring → pump → event → UI` existe et fait transiter un
`{playhead_samples, master_peak}`. Ici on l'étend au `TelemetryFrame` plein et à
tous les consommateurs visuels — on ne crée pas le pipeline, on le densifie.

**Fichiers**
- `telemetry/frame.rs` — **étend** le `TelemetryFrame` minimal de M5 à sa capacité
  fixe pleine (cf. ARCHITECTURE.md) : peaks/RMS par nœud, valeurs de modulation.
  Le ring `rtrb` dédié et son écriture par le thread audio existent déjà.
- `control/telemetry_pump.rs` — **existe depuis M5** ; on l'étend pour agréger le
  frame plein (max des peaks par nœud) en gardant l'event `telemetry` à ~60 Hz.
- `views/chain/NodeMeter.vue` — VU par nœud (peak + RMS, ballistique :
  attaque instantanée, release 300 ms).
- `control/watcher.rs` — `notify` sur `projects/<p>/` avec debounce 100 ms →
  re-parse → diff superficiel (pistes/clips/notes ajoutés-supprimés-modifiés)
  → event `project_changed{diff}` → l'UI recharge et surligne 1,5 s.

**CA**
- [ ] VU actifs sur chaque nœud de la chaîne, playhead fluide, charge CPU et
      xruns affichés.
- [ ] La télémétrie ne provoque AUCUNE allocation côté audio (test de garde)
      et < 2 % de CPU côté UI à 60 Hz.
- [ ] Modifier `song.json` dans un éditeur externe → l'UI reflète en < 300 ms,
      les notes ajoutées clignotent ; si la lecture tourne, les nouvelles
      notes jouent au prochain passage.
- [ ] Édition UI + édition fichier quasi simultanées → pas de crash, règle
      « le fichier gagne, undo disponible » respectée.

**Pièges**
- Si le ring de télémétrie est plein, le thread audio ÉCRASE l'ancien (jamais
  bloquer, jamais attendre).
- Le watcher reçoit des rafales d'événements par fichier sauvé : debounce
  obligatoire, et ignorer les écritures émises par l'app elle-même (flag).

---

## M8 — Contenu sonore + mixeur + polish = MVP (4-6 jours)

**Objectif** : de quoi faire un vrai morceau.

**Prérequis (à traiter AVANT les effets de M8) — débordement de la zone chaîne.**
En M6 les cartes d'effet sont posées en ligne dans la zone chaîne ; dès deux ou
trois effets la chaîne dépasse la largeur disponible et les dernières cartes sont
coupées à droite. M8 ajoute delay/reverb/compresseur (chaînes longues + cartes
plus hautes à cause des paramètres multiples), donc ce layout doit être cadré
avant, sinon on teste M8 sur une UI qui cache la moitié de la chaîne. Définir et
implémenter le comportement de débordement — au choix ou en combinaison :
scroll horizontal de la zone chaîne ; cartes compactes qui ne déplient le détail
de leurs paramètres qu'à la sélection/au survol ; repli par piste. Non bloquant
pour les CA de M6 ; bloquant pour commencer l'intégration visuelle de M8.

**Étapes**
1. Instruments : `modules/fm.rs` (2 opérateurs, ratio + index modulables,
   c'est suffisant pour cloches/keys/basses) ; `modules/drum_sampler.rs`
   (lecture d'échantillons WAV mappés par pitch, 8 slots, embarquer un kit
   libre de droits dans `assets/`).
2. Effets : `modules/delay.rs` (synchronisé au tempo, feedback filtré),
   `modules/reverb.rs` (Freeverb/FDN simple — qualité honnête, pas un Lexicon),
   `modules/compressor.rs` (attack/release/ratio/makeup, détection RMS). La reverb
   est un **effet d'insert** dans la chaîne d'une piste (comme le delay) : pas de
   bus de reverb partagé au MVP.
3. Mixeur : gain/pan par piste (pan loi -3 dB) + master. Pas de bus d'envoi : les
   **sends/bus = post-MVP #1**, déclenchés par le besoin concret d'une reverb
   partagée (cf. vision modulaire de CONTRIBUTING.md). `views/mixer/MixerPanel.vue` :
   faders verticaux + VU.
4. Métronome (clic généré, pas un sample), volume dédié.
5. Projet de démo complet 16 mesures qui sonne bien — c'est aussi le test
   d'intégration vivant et la démo pour Claude Code.

**CA — recette finale du MVP**
- [ ] Le scénario du contrat (en tête de ce document) déroulé en entier sur
      Windows et Linux, filmé une fois pour référence.
- [ ] Export WAV du projet démo : aucun clip > 0 dBFS, pas de DC offset.
- [ ] 8 pistes + 16 effets à buffer 256 : charge audio < 40 %, zéro xrun
      sur 10 minutes.
- [ ] `scripts/check.sh` vert, tous les tests des jalons précédents verts.

---

## Post-MVP (ordre recommandé)

1. **MIDI live** (`midir` + petits buffers + monitoring de latence) — l'archi
   est prête, c'est surtout du réglage de buffers par OS.
2. **Éditeur de graphe libre** (Vue Flow) + **modulations** (LFO, enveloppes
   assignables, anneaux sur les knobs) + **macros** — le `GraphDesc` les
   supporte déjà, c'est principalement de l'UI.
3. **Câbles animés + pré-écoute des presets + scrub.**
4. **CLAP host** (crate `clack`) comme simple module de plus.

## Polish visuel global (passe unique, au moment de fixer palette + logo)

Cosmétique réversible, à traiter **en une seule passe** quand l'identité visuelle
sera arrêtée — pas isolément. À ne pas confondre avec un bug fonctionnel.

- **Label de paramètre redondant.** Sur un effet à paramètre unique dont le
  paramètre porte le même nom que l'effet (ex. `gain`), le label du slider répète
  le nom du module et n'apporte rien. À masquer **dans ce seul cas** (paramètre
  unique homonyme) ; pour un effet multi-paramètres (compresseur :
  attack/release/ratio/threshold) les labels restent indispensables.
- **Couleur des sliders.** Les `input[type=range]` des cartes d'effet utilisent le
  style natif (bleu vif) qui détonne avec la palette teal/anthracite. À
  harmoniser avec la couleur d'accent.

## Stratégie de test (transverse)

- `engine/` : tests de rendu offline déterministes (golden files WAV courts,
  tolérance epsilon) + tests de propriétés (continuité, RT-safety, absence
  d'allocations via allocateur de garde).
- `control/` : tests unitaires purs (séquenceur, parsing, undo, diff watcher).
- UI : le renderer canvas a un harnais de perf scripté ; le reste est couvert
  par les checklists de CA, pas de tests E2E fragiles dans le MVP.
- Tout bug audio corrigé = un test de non-régression ajouté, sans exception.

## Conduite des jalons

- Ne jamais laisser passer un jalon avec des CA rouges « qu'on corrigera
  après » : chaque jalon est la fondation du suivant.
