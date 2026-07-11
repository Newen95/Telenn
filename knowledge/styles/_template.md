# Style : <nom> (gabarit)

> Copier ce fichier en `styles/<nom-kebab>.md`, remplir **en grandeurs
> mesurables ou actionnables** (un adjectif seul ne guide pas une machine),
> ajouter la ligne d'INDEX.md dans le même commit.

## Identité

- Tempo : <plage BPM typique> ; signature : <4/4…> ; énergie : <sobre/dense…>
- Ce qui rend le style reconnaissable en une phrase.

## Instrumentation type

| Rôle | Son recherché (mesurable) | Recette locale |
|---|---|---|
| basse | ex. centroïde < 500 Hz, attaque < 15 ms | `sounds/<fiche>` ou « à créer » |
| lead | … | … |
| batterie | … | … |

## Patterns rythmiques

Placements en `bar.beat.tick` (PPQ 960, positions 1-indexées). Ex. :
- kick : `x.1.0` et `x.3.0` ; snare : `x.2.0` et `x.4.0` ; hats : croches.
- Particularités (swing, ghost notes, breaks toutes les N mesures…).

## Harmonie & mélodie

- Gammes/modes ; progressions récurrentes (degrés, pas des citations) ;
  registres typiques (pitches MIDI) ; portée mélodique (conjoint/sauts).

## Structure type

- Ex. intro 4 — couplet 8 — refrain 8 — pont 4… (en mesures).

## Cibles mix (mesurables par `render_probe --song`)

- Peak master ≤ <x> dBFS ; crest visé <x..y> dB ; équilibre par piste
  (RMS relatif) ; largeur stéréo (corrélation attendue).

## Sources consultées

- <liens/références écoutées ou lues, datées>

## Verdicts & historique

- `AAAA-MM-JJ` — création de la fiche (recherche seule). Statut : plausible.
