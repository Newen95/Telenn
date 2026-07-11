---
name: compose-style
description: Composer ou arranger un morceau Telenn, notamment dans le style d'un artiste, d'un album ou d'un genre (« à la manière de X », « fais-moi un morceau synthwave »…). Utiliser dès qu'on demande de composer, écrire ou arranger de la musique.
---

Suivre la procédure de `knowledge/workflows/compose-style.md` — la lire
d'abord, elle fait foi (fiche de style, analyse d'écart, composition,
boucle de mix).

Points non négociables :
- Consulter `knowledge/INDEX.md` : fiche de style existante à enrichir plutôt
  qu'à recréer ; sons existants dans `knowledge/sounds/` avant d'en créer
  (les manquants passent par la skill sound-design).
- Composer dans `projects/<p>/song.json` (positions 1-indexées, durées
  0-indexées, PPQ 960, ids stables, params normalisés) puis
  `cargo run --bin song_lint -- <song.json>` jusqu'à exit 0.
- Boucle de mix : `cargo run --bin render_probe -- --song <song.json>
  [--track <id>] [--png p] [--json p]` contre les cibles de la fiche.
- S'inspirer d'un style, jamais recopier une œuvre (patterns, pas citations).
- Verdict humain daté dans la fiche style ; leçons dans
  `knowledge/lessons.md`.
