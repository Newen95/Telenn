# UX_SPEC.md — Vues, gestes & feedback

Cible : débutants/intermédiaires. Principes : divulgation progressive, feedback
audio/visuel immédiat partout, gestes cohérents entre toutes les vues, aucune
action ne fait peur (undo universel, le son ne s'arrête jamais).

## Vues & navigation

Trois niveaux de zoom conceptuels :

1. **Timeline (vue principale)** — pistes, clips, transport. Un débutant peut y
   vivre à 100 % sans jamais voir un nœud.
2. **Panneau chaîne (bas d'écran)** — la chaîne de la piste sélectionnée :
   `Notes -> Instrument -> Effets -> Mixeur`. C'est le graphe rendu linéairement
   tant qu'il est simple ; il bascule en graphe libre (Vue Flow) dès qu'un routage
   non linéaire apparaît (send, modulation croisée, sortie multiple). **Une seule
   source de vérité, deux rendus.** La bascule est automatique et **sans retour
   forcé** : une fois la chaîne non linéaire, on ne la replie jamais en strip. Une
   vue linéaire qui cacherait un send ferait croire la chaîne plus simple qu'elle
   n'est — le genre de mensonge qui empêche un débutant de comprendre son propre
   son. Asymétrie volontaire : on peut toujours *ouvrir* le graphe d'une chaîne
   simple par curiosité, jamais voir moins que la réalité. Quand une action est sur
   le point de rendre la chaîne non linéaire (on tire un send depuis la vue
   linéaire), un indice visuel annonce la transition vers le graphe — pas de pop-up
   de confirmation, juste de quoi éviter l'effet « ma chaîne a sauté en mode bizarre
   sans prévenir ».
3. **Intérieur d'un module** — double-clic sur un nœud = on descend dans son
   patch (oscillateurs, filtres, LFO). **Breadcrumb** pour remonter :
   `Master ▸ Lead ▸ Piano FM`. Modèle mental : sous-graphes Blueprints.

Le piano roll s'ouvre par double-clic sur un clip, dans le même panneau bas
(onglet contextuel), jamais dans une fenêtre flottante.

## Code couleur des signaux (partout identique)

| Signal       | Couleur | Usage                                   |
|--------------|---------|-----------------------------------------|
| Notes (MIDI) | violet  | ports, câbles, clips de notes           |
| Audio        | teal    | ports, câbles, clips audio, VU          |
| Modulation   | ambre   | ports, câbles, anneaux de modulation    |

Pendant un drag de câble, seuls les ports compatibles restent colorés ; les
autres se grisent. Aucune connexion invalide n'est possible, aucun message
d'erreur n'est nécessaire.

## Gestes (cohérents dans timeline, piano roll ET graphe)

### Navigation (souris, trackpad, clavier)

**Une seule grammaire de navigation partout**, qui sert proprement souris *et*
trackpad et ne présuppose aucun périphérique précis. Choix structurant : on suit
la **convention DAW**, pas la convention éditeur-de-nœuds — la molette/le
défilement **navigue**, un **modificateur zoome**. Raison : le trackpad est le
périphérique principal (portable) et son geste deux-doigts natif veut dire
« défiler » ; si la molette seule zoomait, le deux-doigts zoomerait à contretemps.
La convention DAW couvre les deux périphériques avec une seule logique.

**Souris**

| Geste                       | Effet                                       |
|-----------------------------|---------------------------------------------|
| Molette                     | défilement vertical                         |
| Shift + molette             | défilement horizontal                       |
| Ctrl + molette              | zoom, centré sur le curseur                 |
| Glisser au bouton du milieu | pan                                         |

**Trackpad** (à câbler explicitement — rien n'est gratuit dans la webview)

| Geste                       | Effet                                       |
|-----------------------------|---------------------------------------------|
| Défilement deux-doigts      | pan sur les deux axes à la fois             |
| `Ctrl` + défilement deux-doigts | zoom, centré sur le curseur             |
| Pincement deux-doigts       | zoom *(visé, mais indisponible sous WebView2/Windows — voir limites connues)* |
| Un doigt                    | curseur / clic (créer, déplacer, brancher)  |

**Alias clavier** (indispensables, pas optionnels)

| Geste                       | Effet                                       |
|-----------------------------|---------------------------------------------|
| Espace + glisser            | pan (outil main) — le pan du trackpad, qui n'a pas de bouton du milieu |
| `+` / `−`                   | zoom                                        |
| `F` (ou double-clic dans le vide, graphe seul) | tout cadrer / recentrer la vue |

> Détail technique : un `wheel` avec **`ctrlKey` actif** signifie zoom (`Ctrl` +
> molette/deux-doigts), sinon défilement → pan. Toujours `preventDefault()` pour
> empêcher le navigateur de zoomer/défiler la page entière (le piège « le zoom
> agrandit toute l'UI au lieu de la timeline »). En théorie l'OS traduit aussi le
> **pincement** en `wheel`+`ctrlKey` (vrai dans Chrome) — mais **pas sous WebView2**,
> qui l'avale avant le DOM (voir limites connues), d'où le repli sur `Ctrl`+deux-doigts.

> Recentrer la vue : le déclencheur universel est **`F`** (aucune collision). Le
> double-clic dans le vide ne sert de « tout cadrer » **que dans le graphe** ; en
> timeline et piano roll il est déjà réservé à la **création** (voir gestes
> d'édition). Une commande de recentrage existe dans le graphe et la timeline pour
> se retrouver quand on s'est perdu.

Contraintes transverses :

- Aucune action de navigation ne dépend **exclusivement** du bouton du milieu
  (absent au trackpad) ni du survol : toujours un équivalent clic ou clavier.
- La machine à états des gestes s'appuie sur les **Pointer Events**
  (souris/tactile/stylet unifiés), pas sur `mousedown`/`mousemove`. Le tactile
  n'est pas pour maintenant (post-MVP), mais raisonner en pointeur dès aujourd'hui
  évite une réécriture — décision gratuite, même esprit que les clés i18n.
- Le **zoom est toujours centré sur le curseur** (souris) ou le **point pincé**
  (trackpad), jamais sur le coin de la vue.
- Le **même renderer et la même machine de gestes** servent timeline et piano roll
  (déjà acté) ; cette convention de navigation y est mutualisée, seule la couche de
  mapping diffère. Le graphe (M6) suit la même grammaire.

> Correction d'une note antérieure : la version précédente disait « zoom molette
> centré sur le curseur ». Le « centré sur le curseur » reste juste, mais le
> **déclencheur change** — le zoom passe derrière **Ctrl** (souris) / **`Ctrl` +
> deux-doigts** (trackpad), la molette/le défilement seul **défile**. Le pincement
> nu était visé pour le trackpad mais n'est pas exploitable sous WebView2 (voir
> limites connues).

Échéances : la navigation concerne **M5** pour la timeline et le piano roll,
**M6** pour le graphe.

#### Limites connues & raffinements post-MVP

- **Pincement trackpad sous Windows : indisponible (limite plateforme).** Sous
  WebView2 (le moteur web de Tauri sur Windows), le pincement nu est consommé par
  le compositeur natif et **n'atteint jamais le DOM** ; il n'existe pas de chemin
  d'interception fiable (pas d'event `wheel`, pas de `gesture*` — réservé à Safari).
  Désactiver le zoom de page natif (`IsPinchZoomEnabled`) ne le fait pas non plus
  retomber en `ctrl+wheel`. **Solution officielle de zoom au trackpad : `Ctrl` +
  défilement deux-doigts, ou `+`/`−`.** À **revérifier sous WebKitGTK (Linux)** lors
  de la validation Linux : moteur différent, comportement possiblement différent ;
  si le pincement y émet bien un event DOM, on documentera la limite comme
  spécifique à Windows.
- **Sens du zoom configurable : post-MVP.** Par défaut, geste vers le haut = zoom
  avant (convention dominante). Le « natural scrolling » système et le type de
  périphérique (souris vs trackpad) influent sur le signe du delta ; une option
  d'inversion viendra plus tard si l'usage le réclame.
- **Zoom vertical (hauteur des rangées) : post-MVP.** Le piano roll ne zoome que
  l'axe temps (largeur) ; les rangées de pitch ont une hauteur fixe (une rangée =
  un demi-ton, reconnaissable d'un coup d'œil). Un zoom vertical, sur un **geste
  distinct** (ex. `Ctrl+Shift`+molette ou contrôles dédiés), resserrera/agrandira
  les rangées pour les morceaux à large tessiture ou la visée au trackpad.

### Gestes d'édition

| Geste                  | Effet                                            |
|------------------------|--------------------------------------------------|
| Drag simple            | déplacer (snap actif)                            |
| Alt + drag             | déplacer sans snap / dupliquer (sur un nœud)     |
| Double-clic sur vide   | créer (clip dans timeline, note dans piano roll) |
| Double-clic sur objet  | descendre dedans (clip->piano roll, nœud->patch) |
| Tab ou clic droit (graphe) | palette de recherche de nœuds (taper "rev" -> Reverb sous le curseur) |
| Ctrl+Z / Ctrl+Shift+Z  | undo/redo universel (y compris graphe et knobs ; un geste continu de knob = une entrée d'undo) |
| Suppr                  | supprimer (avec reconnexion automatique de la chaîne) |

Le snap s'adapte au zoom : zoomé = double-croche, dézoomé = mesure.
Knobs : drag vertical, Shift = précision fine, double-clic = valeur par défaut,
Ctrl+clic = saisie numérique.

## Drag & drop universel

Règle : **tout objet est saisissable ; dès le début du drag, toutes les cibles
valides s'illuminent.**

- Nœud d'effet glissé : interstices des chaînes (toutes pistes + bus + master)
  s'éclairent ; au survol d'un interstice, les nœuds s'écartent (animation
  ~120 ms) pour montrer l'insertion.
- Déposer sur une autre piste = déplacement (avec son état interne préservé).
  Alt = copie.
- Lâcher hors de toute cible = retour élastique animé (annulation).
- Glisser un nœud vers le navigateur = création d'un preset.
- Glisser un preset du navigateur sur un câble = insertion dans la chaîne ;
  sur une piste = ajout en fin de chaîne ; sur le vide de la timeline =
  nouvelle piste.
- Toute opération de drop passe par le hot-swap moteur : **le son ne
  s'interrompt jamais**, y compris pendant la lecture.

## Feedback visuel (le DAW « vivant »)

**Principe directeur : le feedback réagit aux changements d'état, pas aux gestes
de l'utilisateur.** Le renderer pense « l'état a changé, je l'anime », jamais
« l'utilisateur a agi, je réagis ». L'UI a deux sources d'édition de rang égal —
l'humain (souris/clavier) et l'agent Claude Code (`song.json` + macros) — et le
feedback les traite à l'identique : qu'un knob soit bougé à la souris ou par une
édition de fichier, l'anneau s'anime et la valeur transite en douceur, jamais de
saut sec. Pensé ainsi, le feedback des éditions agent vient gratuitement. Corollaire :
l'UI ne ment jamais sur le signal — ce qu'on voit reflète ce que le moteur fait.

Alimenté par la télémétrie (audio -> UI, 60 fps, unidirectionnel) :

- **Câbles animés** : opacité/épaisseur selon l'amplitude du signal qui les
  traverse (cf. fils Blueprints en debug). Câble éteint = rien ne passe.
- **Mini VU-mètre sur chaque nœud** ; mini-oscilloscope optionnel (toggle).
- **Anneaux de modulation** autour des knobs modulés : plage + valeur
  instantanée animée.
- **Timeline vivante** : playhead fluide (interpolé entre frames de télémétrie),
  notes du piano roll qui s'allument quand elles jouent, waveforms dans les
  clips audio.
- **Indicateur de santé** : compteur de xruns + charge CPU du thread audio,
  discret mais toujours visible.

## Feedback audio

- Pré-écoute : survol/clic d'un preset joue une note de démo ; clic sur une
  note du piano roll la joue ; **poser une note la fait sonner immédiatement**.
- Scrub : drag du playhead joue l'audio.
- Métronome + count-in pour l'enregistrement MIDI.
- Optimistic UI : l'effet visuel est immédiat, le moteur suit. Jamais de spinner.

## Débutants : macros & pédagogie

- Chaque instrument expose **4-8 macros** en façade (gros knobs : Brightness,
  Punch, Space...), câblées vers les paramètres internes. Le capot (graphe
  interne) est optionnel. Les macros sont aussi l'interface privilégiée de
  Claude Code (« plus brillant » = bouger une macro).
- Tooltips pédagogiques sur chaque paramètre (1 phrase, ex. « Cutoff :
  fréquence au-delà de laquelle le filtre atténue le son »).
- Empty states actifs : piste vide -> « Glisse un instrument ici ou appuie sur
  Tab » ; projet vide -> proposer un template.

## Collaboration live avec Claude Code

File watcher sur le projet : quand `song.json` change sur disque, l'UI se met à
jour en direct avec **surbrillance temporaire des éléments modifiés** (notes
ajoutées, params changés). Si l'utilisateur a des modifications non sauvegardées
en conflit, proposer un merge simple (le fichier gagne par défaut, undo
disponible).

## Priorités d'implémentation UX

Les **trois interactions reines**, à rendre impeccables avant tout le reste :

1. Poser/étirer/déplacer une note au piano roll (avec écho audio).
2. Déplacer/redimensionner un clip sur la timeline.
3. Brancher un câble dans le graphe (avec typage visuel des ports).

Snap intelligent + undo + feedback sonore sur ces trois gestes = le DAW
« paraît » fini. Ensuite : drag & drop universel -> câbles animés/VU ->
file watcher.
