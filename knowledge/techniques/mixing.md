# Mixage — cibles génériques et gain-staging

Graine de départ ; enrichir avec les verdicts d'écoute au fil des morceaux.

## Cibles génériques d'un rendu propre

Vérifiées automatiquement par `render_probe --song` / `analyze_wav`
(politique de défauts, sortie `== verdict ==`) :

- **Peak master ≤ −1 dBFS** (garde-fou : > −0,3 dBFS = avertissement,
  clipping = erreur dure).
- **DC ≈ 0** (|DC| > 0,005 = erreur).
- **Zéro clic** (discontinuités) ; fin de rendu propre (retour à ~0).
- Crest global : ~8–14 dB pour un mix dense, plus pour du clairsemé ;
  un crest qui s'effondre (< 6 dB) signale une compression excessive.

## Procédure de gain-staging

1. `cargo run --bin render_probe -- --song <song.json> --track <id> --out …`
   pour chaque piste (solo) : noter peak/RMS par piste.
2. Régler `mixer.gain_db` dans `song.json` (jamais dans le code) pour asseoir
   la hiérarchie voulue (ex. basse et kick dominants, pads en retrait).
3. Rendu complet + `--png` : vérifier l'équilibre spectral au spectrogramme
   (le bas ne doit pas noyer le haut, chaque famille a sa bande).
4. Boucler jusqu'à exit 0 **et** cibles de la fiche style atteintes.

## Largeur stéréo

Chorus et reverb décorrèlent gauche/droite nativement (le repli mono reste
exactement le wet mono d'antan : la somme mono d'un morceau existant ne bouge
pas, seule l'image s'ouvre). Leviers et mesures :

- `stereo_correlation` (analyze_wav) : 1.00 = mono ; un mix « ouvert » vit
  vers 0.6–0.9. La reverb est le plus gros ouvreur (mix ↑ = corrélation ↓),
  le chorus suit, le delay et les pans font le placement.
- `width` (reference_match, ratio side/mid) : 0 = mono, ~0.2–0.4 = mix
  moderne typique.
- Pas de widener : la largeur vient des effets et des pans, la compatibilité
  mono est garantie par construction (vérifiable : repli mono ≈ rendu mono).

## Bus master (`master_chain` dans song.json)

Chaîne d'effets appliquée au mix complet, avant le gain master (le fader
reste le trim final). Recette « glue » de départ, à ajuster à l'oreille :

```json
"master_chain": [
  { "id": "mst-comp", "module": "compressor",
    "params": { "threshold": 0.667, "makeup": 0.125 } },
  { "id": "mst-sat", "module": "saturator", "params": { "drive": 0.111 } }
]
```

(threshold −20 dB, ratio 4:1 par défaut, makeup +3 dB ; saturator drive
~4 dB.) Mesuré sur mareux-test : crest 14.6 → 13.0 dB, RMS +2.9 dB, zéro
défaut — le « glue » que la somme de pistes seule ne donne pas. Après ajout,
**retrimmer `master_gain_db`** (le makeup + la saturation remontent le
niveau : viser peak ≤ −1 dBFS).

## Se comparer à une référence (`reference_match`)

```
cargo run --bin reference_match -- <rendu.wav> <référence.wav> [--json p]
```

Diff chiffré : loudness intégrée (BS.1770, LUFS), crest, équilibre 7 bandes
(sub/bass/low-mid/mid/high-mid/presence/air, en dB relatifs au total —
insensible au volume), corrélation/largeur, tempo et densité d'attaques.
Verdict `OFF …` au-delà des seuils (±1 LU, ±3 dB/bande, ±0.3 corr., ±3 BPM,
±30 % densité) ; exit 0 = dans les clous. Les deux fichiers doivent partager
le sample rate. Lire les bandes comme un EQ : « high-mid −9 dB vs réf » =
le mix est plus sombre que la référence dans cette bande.

## Historique

- 2026-07-11 — création (aucun verdict d'écoute encore).
- 2026-07-14 — stéréo native (chorus/reverb décorrélés, repli mono prouvé à
  1 LSB près sur mareux-test), bus master (crest 14.6 → 13.0 dB mesuré),
  `reference_match` (auto-diff vs les 4 références Mareux cohérent avec les
  mesures Python du 2026-07-14 : tempo 114.8 vs 114.0 BPM détecté).
- 2026-07-14 — verdict d'écoute : la reverb stéréo est perçue comme de
  **meilleure qualité** (pas seulement plus large) — le repli mono des taps
  L/R créait des annulations de phase que la décorrélation supprime ; le
  tail est plus dense et enveloppant à son direct identique. VALIDÉ.
