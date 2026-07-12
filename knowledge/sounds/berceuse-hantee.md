# Son : berceuse-hantee

## Intention

Brief (2026-07-11) : « un son de berceuse, utilisable comme son d'ambiance
dans un jeu/film d'horreur ». Précisé après écoute : **le mobile musical
au-dessus du lit d'un bébé** — la boîte à musique, pas la cloche. Référence
fournie par l'utilisateur : `freesound_community-music-box-34179.mp3`
(vraie boîte à musique, analysée note isolée à F#6).
Rôle conseillé : lead lent / ambiance, mélodies clairsemées.
Registre utile : **F#5–F#6 (78–90)** validé ; au-dessus de C7 le timbre
KS devient rêche (H2 fort).

## Empreinte de la référence (note isolée, F#6)

| Mesure | Référence réelle | v3 (plucked) |
|---|---|---|
| H2 / H3 / H4+ | −26 / −44 / ≤ −44 dB | −19 / −26 / −34 dB (plus riche, même forme) |
| attaque | 1 ms | 0 ms ✓ |
| RT60 | ≥ 2 s | 3.2 s (avec reverb) ✓ |
| caractère | plink brillant → tenue quasi pure | idem (physique KS) ✓ |

## Recette (v3 — lame pincée)

- Chaîne : `plucked -> reverb`
- Presets : [`berceuse-hantee-plucked.preset.json`](./berceuse-hantee-plucked.preset.json)
  + [`berceuse-hantee-reverb.preset.json`](./berceuse-hantee-reverb.preset.json)

| Param | Valeur réelle | Normalisé |
|---|---|---|
| plucked.brightness | 0.35 (**≥ 0.3 obligatoire**, voir quirk DC) | 0.350 |
| plucked.decay | 0.98 (la lame résonne) | 0.980 |
| plucked.release | 1000 ms (**libère la résonance** — un release court coupe la lame) | 1.000 |
| plucked.level | −8 dB | 0.867 |
| reverb.size | 0.55 | 0.550 |
| reverb.mix | 0.55 | 0.550 |
| reverb.predelay | 20 ms | 0.200 |

Ré-étalonnage 2026-07-11 : la reverb est devenue une **plate Dattorro** (queue
plus dense/lisse, +predelay). L'ancienne recette (size 0.82 / mix 0.35 sur le
Freeverb) donnait RT60 3.2 s ; sur la plate elle sonnait 6.9 s. Les valeurs
ci-dessus retrouvent RT60 3.4 s ; le halo est plus doux qu'avant (wet plus
maigre sur source clairsemée) — monter `mix` si on veut plus de brume.

## Mesures de référence (v3)

```
cargo run --bin render_probe -- --module plucked --set brightness=0.35 --set decay=0.98 --set release=1000 --set level=-8 --fx reverb --set reverb.size=0.82 --set reverb.mix=0.35 --pitches 78,84,90 --note-beats 2 --tail 5
```

f0 exact aux trois registres (clarity 0.91–0.99), zéro défaut, RT60 3.2 s.
Limite connue : H2 reste ~7 dB au-dessus de la référence (pas d'effet EQ au
catalogue pour amincir davantage — manque noté dans lessons.md).

## Historique des recettes

- v1 `fm` ratio 3.53 / index 1.2 — **rejetée à l'oreille** : « cloche/
  sonnette, métallique » (ratio non entier = partiels désaccordés).
- v2 `fm` ratio 3 / index 0.8 — « un peu mieux » mais pas l'effet boîte à
  musique : le spectre FM est figé, or la vraie lame a un plink brillant qui
  s'éteint en tenue pure.
- v3 `plucked` — changement de physique (Karplus-Strong = lame pincée),
  calé sur l'empreinte mesurée de la référence.

## Verdicts & historique

- 2026-07-11 — v1 créée (mesures conformes au brief initial « cloche »).
- 2026-07-11 — **écoute v1 : rejetée** (« vraiment une cloche/sonnette,
  métallique ; je pensais au mobile au-dessus du lit »).
- 2026-07-11 — **écoute v2 : « un peu mieux » mais pas boîte à musique** →
  l'utilisateur fournit un extrait de référence ; empreinte extraite
  (note F#6 isolée) → v3 sur `plucked`.
- 2026-07-11 — **écoute v3 en comparaison directe avec la référence
  (même mélodie, Berceuse de Brahms transcrite depuis l'enregistrement) :
  « l'instrument en lui-même est très proche, un succès ». Statut :
  VALIDÉ (oreille humaine)**. Mélodie de démo :
  [`berceuse-hantee-demo.song.json`](./berceuse-hantee-demo.song.json)
  (3/4, 87 bpm, 24 notes — transcription mesurée, « quelques notes à
  côté » de l'original, octaves médianes ambiguës dans l'extraction).
- 2026-07-11 — **reverb remplacée par la plate Dattorro** : recette
  ré-étalonnée (voir table : size 0.55 / mix 0.55 / predelay 20 ms, RT60
  re-mesuré 3.4 s ≈ l'ancien 3.2 s), **re-validée à l'oreille le
  2026-07-11. Statut : VALIDÉ**.
