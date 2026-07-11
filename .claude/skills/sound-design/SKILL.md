---
name: sound-design
description: Créer ou ajuster un instrument, un preset ou un timbre Telenn à partir d'un brief sonore (« une basse ronde », « un lead brillant », « un son de cloche »…). Utiliser dès qu'on demande de créer, modifier ou améliorer un son.
---

Suivre la procédure de `knowledge/workflows/sound-design.md` — la lire
d'abord, elle fait foi (cibles mesurables, boucle rendre→mesurer, critères
d'arrêt).

Points non négociables :
- Consulter `knowledge/INDEX.md` avant de créer quoi que ce soit (un son
  proche existe peut-être dans `knowledge/sounds/`).
- Outils depuis `src-tauri/` : `cargo run --bin module_specs` (plages
  réelles + quirks), `cargo run --bin render_probe -- --module <id>
  --set <param>=<valeur réelle> [--fx <id>] [--json p] [--png p]`.
- Jamais de son livré sans mesure (exit 0 = zéro défaut) ; l'oreille de
  l'utilisateur reste le verdict final.
- Résultat validé → paire fiche + preset dans `knowledge/sounds/` et
  `INDEX.md` mis à jour dans le même commit.
