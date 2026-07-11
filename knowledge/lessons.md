# Leçons transverses

Journal daté, du plus récent au plus ancien. Format : `## AAAA-MM-JJ — titre`
puis contexte, mesure (commande exacte), conclusion. Les impasses se
consignent au même titre que les réussites.

## 2026-07-11 — Empreintes par défaut des instruments (référence de départ)

Contexte : première passe de `render_probe` sur les 8 modules aux paramètres
par défaut (`cargo run --bin render_probe -- --module <id>`), 120 bpm, note
2 temps, vel 100.

Mesures :
- `plucked` : attaque ~0-1 ms (excitation instantanée), RT60 ≈ 0,09 s aux
  défauts — le son meurt vite ; pour un pluck qui « chante », allonger via
  `decay` ou un effet (delay/reverb).
- `subtractive` : attaque ≈ 7 ms, centroïde ≈ 1,6 kHz à cutoff 2000 Hz par
  défaut ; `--set cutoff=800` → centroïde ≈ 345 Hz (le centroïde suit bien le
  cutoff).
- `fm` : attaque ≈ 3 ms, f0 exact, clarity 1.00 aux défauts (ratio 1 = ton
  harmonique pur).
- `drum_sampler` : f0 = none (normal, non tonal), attaque ≈ 1 ms.
- Effets derrière `subtractive` : sec RT60 ≈ 0,17 s ; `+delay` ≈ 2,4 s ;
  `+reverb` ≈ 1,05 s (défauts). Le RT60 mesuré est le bon indicateur
  d'« ampleur ».

Conclusion : ces valeurs servent de point de départ aux boucles de
sound-design ; mesures seules (aucune écoute humaine) — statut plausible.
