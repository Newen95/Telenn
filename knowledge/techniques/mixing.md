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

## Historique

- 2026-07-11 — création (aucun verdict d'écoute encore).
