# UX_SPEC.md — Vues, gestes & feedback

Cible : débutants/intermédiaires. Principes : divulgation progressive, feedback
audio/visuel immédiat partout, gestes cohérents entre toutes les vues, aucune
action ne fait peur (undo universel, le son ne s'arrête jamais).

Deux règles transverses gouvernent tout le document :

- **Le feedback réagit aux changements d'état, pas aux gestes** (détaillé plus
  bas) — l'humain et l'agent qui édite `song.json` sont deux éditeurs de rang
  égal, traités à l'identique.
- **Chaque propriété visuelle a un sens et un seul** : une couleur, une ombre,
  une animation encodent toujours la même chose, partout (voir « Code couleur
  & lumière » et « Profondeur »).

## Identité visuelle « ardoise & cuivre »

Telenn = harpe en breton. Ardoise sombre bleutée (granit breton) pour tout le
décor, **cuivre chaud pour tout ce qui se touche et tout ce qui sonne
maintenant** (les cordes de la harpe). Tous les tokens (couleurs, rayons,
ombres, transitions, polices) vivent dans `src/styles/theme.ts` — source de
vérité unique, consommée par les composants Vue (CSS custom properties) et par
le renderer canvas. Aucune valeur codée en dur ailleurs.

- **Typo** : Space Grotesk (titres de zones, marque, onglets), Inter (courant),
  JetBrains Mono en `tabular-nums` pour **toute** valeur numérique (BPM, dB,
  ms, compteurs) — un chiffre qui change ne fait jamais trembler son voisin
  (réserver la largeur du pire cas si besoin).
- **Signature « cordes »** : les VU-mètres sont de fines cordes verticales
  teal qui vibrent avec le signal (RMS + tick de peak, rouge au clip) ; le
  master est une paire de cordes L/R ; le playhead est une corde cuivre
  tendue sur les deux canvas. C'est LA touche identitaire — le reste reste
  sobre.
- **UI en français par défaut, bascule EN à tout instant** (menu Affichage,
  persistée). Aucun texte en dur : tout passe par `fr`/`en`, toujours ajoutés
  dans le même commit.

## Layout en zones

Zones **fixes mais vivantes** : pas de docking libre ; chaque zone se
redimensionne (splitters) et se plie/replie (chevron + raccourci), la
disposition est **persistée** entre sessions (localStorage, clé versionnée
`telenn.ui.v1`, tailles clampées au chargement).

```
┌──────────────────────────────────────────────────────────────┐
│ Barre unique : menus texte · transport centré · santé        │
├────────┬─────────────────────────────────────────────────────┤
│Biblio- │  TIMELINE (gutter noms de pistes + clips)           │
│thèque  │═════════════ splitter ══════════════════════════════│
│(pliable│  PIANO ROLL (piste active)                          │
│en rail)│═════════════ splitter ══════════════════════════════│
│        │  [ Chaîne | Mixeur ]                       ▾ plier  │
└────────┴─────────────────────────────────────────────────────┘
```

- **Barre unique en haut** (modèle FL Studio/Bitwig) : menus texte à gauche
  (Fichier : Exporter WAV, Recharger · Édition : Annuler/Rétablir ·
  Affichage : zones, console dev, langue), transport au centre (play, stop,
  métronome + volume, BPM mono) — il ne bouge jamais, quoi que fassent les
  zones —, master meter + CPU/xruns à droite.
- **Timeline et piano roll empilés au centre**, partageant **un seul axe
  temporel** : le même instant est au même X dans les deux vues, playhead en
  tête. Le gutter gauche a la même largeur partout (constante partagée), donc
  le bord du canvas est aligné du haut en bas. Le piano roll est **visible en
  split permanent** — divergence assumée avec la première version de cette
  spec (« onglet contextuel ») : l'axe partagé est la pépite, on ne la cache
  pas.
- **Panneau bas à onglets Chaîne / Mixeur** ; la barre d'onglets reste visible
  replié, cliquer un onglet rouvre dessus.
- **Bibliothèque à gauche**, repliable en rail vertical (le label rouvre).
- **Console dev** : hors du layout, overlay togglable.
- Raccourcis zones : `Alt+B` bibliothèque, `Alt+E` panneau bas (chaîne),
  `Alt+M` panneau bas (mixeur), `Ctrl+Alt+D` console dev. Inactifs dans les
  contextes de saisie.
- La page ne défile jamais (l'app est une coquille plein écran) ; les
  scrollbars internes sont stylées ardoise (fines, thumb discret, cuivre au
  survol).

## Piste active

Une seule notion de sélection de piste, globale, suivie par toutes les vues :

- **Se choisit partout** : clic dans le gutter de la timeline, strip du
  mixeur, chevrons ‹ › de l'en-tête de chaîne. Une piste créée devient active.
- **Se voit partout** : lane éclairée cuivre dans la timeline (voile
  `accent` 8 % + deux liserés 1.5px en haut/bas — jamais un éclaircissement
  neutre qui casserait la chaleur de la palette), strip bordé cuivre au
  mixeur, nom dans l'en-tête du piano roll et de la chaîne.
- Toujours valide : si la piste active disparaît (suppression, reload
  externe), repli sur la première.

## Code couleur & lumière (partout identique)

| Couleur | Sens | Usage |
|---------|------|-------|
| **Cuivre** (accent) | l'interactif & le présent | playhead, notes en train de sonner, sélection, piste active, aperçu de drop, poignées, hover |
| **Teal** (signal Audio) | l'audio qui circule | VU-cordes, fils de la chaîne |
| **Magenta** (signal Notes) | les données de notes | échine de la carte instrument, items instruments et presets de chaîne de la bibliothèque, (post-MVP : ports/câbles Notes) |
| **Ambre** (signal Modulation) | la modulation | réservé pour les anneaux/câbles de modulation (post-MVP) |
| **Teintes de piste** (palette de 8, sourdes) | l'identité d'une piste | voir règle ci-dessous |

- **Cuivre vs teal, la nuance** : le teal dit « du signal circule ici » (les
  fils, les cordes des VU) ; le cuivre dit « c'est ça que le playhead touche
  maintenant » (les notes qui s'allument). La lumière cuivre (`glow`) est la
  version lumineuse de l'accent : halo serré (≈6px au piano roll, ≈3px sur
  les petites marques de la timeline — la taille du halo suit la taille de
  l'objet, jamais l'inverse).
- **Règle des teintes de piste** : partout où une piste est **nommée**
  (en-tête du piano roll, en-tête de chaîne, strip du mixeur), sa teinte est
  un **souligné de 3px aux bouts arrondis** sous le nom. Partout où une piste
  est une **rangée** (gutter de la timeline), sa teinte est une **barre de
  bord gauche**. Jamais de pastille, jamais de fond teinté.
- Les teintes sont attribuées **par id** (stables au réordonnancement),
  sans collision jusqu'à épuisement de la palette ; une couleur explicite
  dans `song.json` prime.
- Pendant un drag, seules les cibles valides réagissent ; les connexions
  invalides sont impossibles par construction (pas de message d'erreur).

## Profondeur

Une seule échelle d'élévation, sémantique : **plus une chose est transitoire,
plus elle flotte haut**. La lumière vient d'en haut. Tokens dans
`theme.shadow`.

| Niveau | Ombre | Qui |
|--------|-------|-----|
| Le puits (creusé) | `well` (inset, dominante en haut) | les deux canvas de notes — LA matière, un seul plan en retrait vu par deux ouvertures ; les gutters (clavier, noms de pistes) n'en font PAS partie : ils surplombent |
| Le plateau (plat) | aucune | les panneaux et barres fixes — ils se séparent par bordures, jamais par ombres entre eux |
| Posé (saisissable) | `raised` (ombre courte dessous + filet de lumière sur la face haute) | cartes de la chaîne, items de la bibliothèque, strips du mixeur — tout ce qui porte cette élévation se saisit à la souris |
| Chrome | `chrome` | la barre du haut, qui projette sur les zones |
| Flottant ancré | `overlay` | menus déroulants, pickers, toasts, console dev |
| En main | `held` | le ghost de drag — la chose la plus haute de l'app |

## Vues

### Timeline

- **Gutter gauche** : nom de chaque piste + barre de teinte ; clic =
  sélection (active partout) ; glisser verticalement = réordonner ; bouton
  « + » sous la dernière piste = menu d'instruments (nouvelle piste, qui
  devient active et prend le focus clavier — Suppr fonctionne immédiatement).
- **Clips avec aperçu miniature des notes** ; pendant la lecture, les
  marques de notes s'allument cuivre au passage du playhead.
- Empty state invitant quand le projet n'a aucune piste.
- À venir : clic sur la règle = seek (la commande moteur existe déjà).

### Piano roll

- Affiche la **piste active** ; en-tête : titre de section + nom de piste
  souligné à sa teinte + rappel des gestes.
- Gutter clavier : touches crème/ardoise, étiquettes d'octave (C3, C4…) sur
  les do ; presser une touche la fait sonner (glissando au drag) — audition
  non destructive.
- Notes à la teinte de la piste ; **la note qui sonne s'allume cuivre**
  (fill lumineux + halo serré) pendant la lecture.
- Les trois gestes reines (poser/étirer/déplacer avec écho audio) restent la
  référence de qualité.

### Chaîne (panneau bas)

La chaîne de la **piste active seule** (la vue d'ensemble est le rôle du
mixeur). En-tête : chevrons ‹ › pour naviguer, nom souligné à la teinte,
compteur n/N.

- **La chaîne se lit comme un chemin de signal, pas comme une rangée de
  boîtes** : des cartes reliées par des **fils teal** dont la luminosité et
  le halo suivent le niveau réel du nœud amont (télémétrie par nœud ; à fort
  niveau le fil vire vers le blanc chaud). Fil éteint = rien ne passe. Pas de
  fil après la dernière carte : le chemin s'arrête là.
- **La carte instrument est une autre espèce** : échine magenta (signal
  Notes), flanc gauche arrondi. Elle se sélectionne et déplie ses paramètres
  comme un effet ; seul son **chevron** ouvre le picker de remplacement
  (moteur = remplace la tête, preset de chaîne = remplace toute la chaîne).
  Elle ne se déplace jamais dans la chaîne ; la glisser vers la bibliothèque
  sauve la chaîne entière en preset. Les popovers ancrés dans des conteneurs
  scrollables sont `position: fixed`, calés sur le rect au moment du clic
  (sinon ils seraient rognés).
- Clic sur une carte = sélection (cadre cuivre) + **les paramètres se
  déplient sous l'en-tête** (nom — slider — valeur réelle), la carte grandit
  vers le bas sans bouger ses voisines. Suppr supprime (reconnexion
  automatique), Ctrl+D duplique — inertes sur la carte instrument. La
  sélection suit le focus.
- **Un preset de chaîne qui survole la tête estompe les effets condamnés**
  (opacité réduite) : la portée du remplacement se lit avant de lâcher.
- Chaque carte porte son mini VU-corde (peak + RMS, télémétrie).

### Mixeur (panneau bas)

Un strip par piste + master : nom souligné à la teinte, fader vertical
(linéaire en dB) avec sa VU-corde, pan, valeurs mono. Un geste de fader
streame en live (retargeting lissé côté moteur) et persiste au relâché — un
geste = une entrée d'undo. Double-clic = valeur par défaut. Clic sur un strip
= piste active.

### Bibliothèque

**Deux familles miroir des types de signaux**, dépliables, avec compteurs +
champ de recherche filtrant tout : **Instruments** (échine magenta — moteurs
d'usine + presets de chaîne) et **Effets** (échine teal — modules d'usine +
presets d'effet). Trait plein = usine, **pointillé = preset utilisateur** ;
au survol d'un preset de chaîne, l'infobulle donne sa composition
(`plucked → reverb`). Les items sont des objets saisissables (élévation
`raised`, curseur grab). La bibliothèque entière est la **cible de drop de
sauvegarde** : y déposer une carte d'effet ou la carte instrument (= chaîne
entière) ouvre un champ de nommage ; pendant le drag, la famille de
destination s'allume à sa couleur de signal.

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
> d'édition).

Contraintes transverses :

- Aucune action de navigation ne dépend **exclusivement** du bouton du milieu
  (absent au trackpad) ni du survol : toujours un équivalent clic ou clavier.
- La machine à états des gestes s'appuie sur les **Pointer Events**
  (souris/tactile/stylet unifiés), pas sur `mousedown`/`mousemove`.
- Le **zoom est toujours centré sur le curseur** (souris) ou le **point pincé**
  (trackpad), jamais sur le coin de la vue.
- Le **même renderer et la même machine de gestes** servent timeline et piano
  roll ; seule la couche de mapping diffère. Le futur graphe suivra la même
  grammaire.

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
  avant (convention dominante).
- **Zoom vertical (hauteur des rangées) : post-MVP.** Le piano roll ne zoome que
  l'axe temps ; les rangées de pitch ont une hauteur fixe (une rangée = un
  demi-ton, reconnaissable d'un coup d'œil).
- **Espace = lecture/stop : différé** — conflit avec Espace+glisser = pan ; à
  trancher plus tard (tap vs hold).

### Gestes d'édition

| Geste                  | Effet                                            |
|------------------------|--------------------------------------------------|
| Drag simple            | déplacer (snap actif)                            |
| Alt + drag             | déplacer sans snap / dupliquer (sur une carte de chaîne) |
| Double-clic sur vide   | créer (clip dans timeline, note dans piano roll) |
| Ctrl+Z / Ctrl+Shift+Z  | undo/redo universel (notes, graphe, mix ; un geste continu = une entrée d'undo) |
| Suppr                  | supprimer (piste au gutter/canvas, note, carte d'effet — avec reconnexion automatique) |
| Échap                  | annule le drag en cours                          |

Le snap s'adapte au zoom : zoomé = double-croche, dézoomé = mesure.
Sliders de paramètres : drag = valeur live, relâché = persistance,
double-clic = valeur par défaut.

## Drag & drop universel

Règle : **tout objet est saisissable ; ce qu'on tient devient l'aperçu déjà
posé là où il atterrira.**

- Un press **arme** le drag ; il ne démarre qu'après un seuil de quelques
  pixels (un clic court reste un clic/une sélection). Le curseur « grab »
  n'apparaît qu'après une courte tenue.
- Pendant le drag, un **ghost** suit le pointeur (élévation `held`, la plus
  haute). Au-dessus d'un emplacement valide de la chaîne, le ghost flottant
  disparaît au profit d'une **prévisualisation en place** : la carte, en
  style cuivre pointillé, déjà insérée à sa position finale et **déjà câblée
  des deux côtés**, à la largeur exacte de la carte réelle. La carte
  d'origine d'un déplacement quitte entièrement la rangée (la chaîne se lit
  « sans elle »).
- **Relâcher = exactement ce qu'on voit.** L'UI applique l'édition
  localement dans la même frame (optimistic UI, le moteur confirme ensuite) et
  aucune animation ne rejoue le placement : ça se pose, figé. Le ciblage est
  indulgent : toute la rangée est cible, l'emplacement retenu est le plus
  proche en X.
- Déposer sur une autre piste = déplacement (état interne préservé) ; Alt =
  copie ; même emplacement = no-op strict (le graphe ne change pas).
- Un **instrument** dragué ne voit que les têtes de chaîne (remplacement,
  pointillé cuivre) et la zone nouvelle piste ; il ne peut pas se poser parmi
  les effets — et réciproquement.
- Glisser une carte d'effet vers la section presets de la bibliothèque =
  sauvegarde en preset (surbrillance ambre).
- Lâcher hors de toute cible = rien (le ghost disparaît, aucun changement).
- Toute opération de drop passe par le hot-swap moteur : **le son ne
  s'interrompt jamais**, y compris pendant la lecture.

## Feedback visuel (le DAW « vivant »)

**Principe directeur : le feedback réagit aux changements d'état, pas aux gestes
de l'utilisateur.** Le renderer pense « l'état a changé, je l'anime », jamais
« l'utilisateur a agi, je réagis ». L'UI a deux sources d'édition de rang égal —
l'humain (souris/clavier) et l'agent (`song.json` + macros) — et le feedback les
traite à l'identique : qu'un paramètre soit bougé à la souris ou par une édition
de fichier, la valeur transite en douceur, jamais de saut sec. Corollaire :
l'UI ne ment jamais sur le signal — ce qu'on voit reflète ce que le moteur fait.

Alimenté par la télémétrie (audio → UI, 60 fps, unidirectionnel) :

- **Fils de chaîne animés** : luminosité/halo selon le niveau réel du nœud
  amont. Fil éteint = rien ne passe.
- **VU-corde sur chaque carte de chaîne**, chaque strip du mixeur, le master
  (balistique commune : montée rapide, retombée lissée).
- **Notes qui s'allument** au passage du playhead (piano roll + marques des
  clips timeline), en lumière cuivre.
- **Playhead fluide** (interpolé entre frames de télémétrie).
- **Indicateur de santé** : charge CPU + xruns dans la barre, mono à largeur
  réservée (aucun tressautement), rouge au-delà de 80 % du budget temps réel.
- **Toasts** en bas à droite, discrets : succès (~3.5 s, liseré vert) et
  erreurs (~8 s, liseré rouge, détail en mono), clic = fermer. Une action de
  menu ne réussit ni n'échoue jamais en silence.
- **Empty states invitants** (projet sans piste, piano roll sans piste,
  chaîne vide, presets vides) — jamais un panneau muet.
- Transitions de pli/dépli des zones (~200 ms), hover/actif sur tout
  l'interactif ; les chevrons tournent avec leur zone.
- Flash ambre des éléments modifiés par une édition externe (voir
  Collaboration).
- **Anneaux de modulation** autour des knobs modulés : post-MVP (avec les
  modulations).

## Feedback audio

- Pré-écoute : presser une touche du gutter clavier joue la hauteur
  (glissando au drag) ; **poser une note la fait sonner immédiatement**.
- Métronome togglable avec son volume (état de session, jamais exporté).
  Count-in : post-MVP (avec l'enregistrement MIDI).
- Scrub du playhead : post-MVP.
- Optimistic UI : l'effet visuel est immédiat, le moteur suit. Jamais de
  spinner.

## Débutants : macros & pédagogie

- Chaque instrument expose **4-8 macros** en façade, câblées vers les
  paramètres internes ; ce sont aussi l'interface privilégiée de l'agent
  (« plus brillant » = bouger une macro). Les paramètres sont déclarés dans le
  module (nom, plage, unité, courbe), jamais côté frontend.
- Tooltips pédagogiques par paramètre : post-MVP.
- Empty states actifs (voir feedback visuel).

## Collaboration live avec l'agent

File watcher sur le projet : quand `song.json` change sur disque, l'UI se met à
jour en direct avec **surbrillance ambre temporaire des éléments modifiés**.
Une édition externe qui ne parse pas n'est jamais silencieuse : toast d'erreur
avec le détail (l'app continue de jouer la dernière version valide). En cas de
conflit avec des modifications non sauvegardées : le fichier gagne par défaut,
undo disponible.

## Graphe libre (post-MVP)

La chaîne linéaire est le graphe **rendu linéairement** tant qu'il est simple.
Le jour où un routage non linéaire apparaît (premier déclencheur concret : le
bus de reverb partagé), la vue bascule en graphe libre (Vue Flow). **Une seule
source de vérité, deux rendus.** La bascule est automatique et **sans retour
forcé** : une vue linéaire qui cacherait un send ferait croire la chaîne plus
simple qu'elle n'est. Asymétrie volontaire : on peut toujours *ouvrir* le
graphe d'une chaîne simple par curiosité, jamais voir moins que la réalité.
Quand une action est sur le point de rendre la chaîne non linéaire, un indice
visuel annonce la transition — pas de pop-up. Le graphe héritera des acquis :
tokens, code couleur des signaux, ports typés (aucune connexion invalide
possible), grammaire de navigation, palette de recherche de nœuds (Tab).
Plus tard encore : descendre dans l'intérieur d'un module (breadcrumb
`Master ▸ Lead ▸ Piano FM`, modèle sous-graphes Blueprints).

## Priorités d'implémentation UX

Les **trois interactions reines** (impeccables avant tout le reste) :

1. Poser/étirer/déplacer une note au piano roll (avec écho audio). ✔
2. Déplacer/redimensionner un clip sur la timeline. ✔
3. Brancher un câble dans le graphe (avec typage visuel des ports) — post-MVP,
   son équivalent MVP est le drag & drop de cartes dans la chaîne. ✔

Ensuite, dans l'ordre réalisé : drag & drop universel ✔ → fils animés/VU ✔ →
file watcher ✔. Reste ouvert (post-refonte) : seek à la règle, boucle
partielle, count-in, tooltips pédagogiques, sélecteur de couleur de piste.
