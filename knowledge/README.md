# Base de connaissances — conventions

Mémoire **agnostique de l'outil** : tout agent (Claude Code, Codex…) et tout
humain lit et entretient ce dossier. Tout est markdown/JSON versionné ; rien
d'important ne vit ailleurs que dans le repo.

## Organisation

| Dossier / fichier | Contenu |
|---|---|
| `INDEX.md` | une ligne par fiche — **point d'entrée obligatoire** |
| `styles/` | fiches de style (artiste, genre, album) |
| `sounds/` | recettes de sons validées : paire `<nom>.md` + `<nom>.preset.json` |
| `techniques/` | techniques de production transversales (mix, gain-staging…) |
| `workflows/` | procédures agent : `sound-design.md`, `compose-style.md` |
| `lessons.md` | journal daté des leçons transverses (réussites ET impasses) |

## Règles d'écriture

1. **Consulter avant de créer.** Chercher dans `INDEX.md` si une fiche couvre
   déjà le sujet ; si oui, **l'enrichir** (section Historique datée) au lieu
   d'en créer une deuxième. Une fiche = un fichier, nommage kebab-case.
2. **Verdicts datés.** Chaque fiche porte son état :
   `Validé le AAAA-MM-JJ (oreille humaine)` ou `Plausible le AAAA-MM-JJ
   (mesures seules)`. Une mesure conforme sans écoute humaine = « plausible »,
   jamais « validé ».
3. **Consigner les échecs.** Une impasse documentée (« l'approche X ne donne
   pas Y, mesuré le… ») vaut une réussite : elle évite de refaire le chemin.
   Les leçons transverses vont dans `lessons.md`, les leçons propres à un
   son/style dans la fiche concernée.
4. **Presets normalisés, valeurs réelles dans la fiche.** Les
   `<nom>.preset.json` suivent le format preset de l'app (`module` + `params`
   **normalisés 0..1**) et sont déposables tels quels dans
   `projects/<p>/presets/`. Les valeurs réelles (Hz, ms, dB) se lisent dans la
   table de la fiche `.md`, jamais dans le JSON.
5. **INDEX.md dans le même commit.** Toute fiche créée, renommée ou dont le
   verdict change ⇒ sa ligne d'`INDEX.md` est mise à jour dans le même commit.
6. **Reproductibilité.** Chaque mesure citée donne la commande exacte qui l'a
   produite (`render_probe …`), pour pouvoir la rejouer après un changement de
   moteur.

## Outils de mesure (depuis `src-tauri/`)

| Commande | Rôle |
|---|---|
| `cargo run --bin module_specs` | catalogue des modules : params, plages réelles, quirks |
| `cargo run --bin render_probe -- --module <id> …` | rendre une note/gamme et la mesurer (boucle de sound-design) |
| `cargo run --bin render_probe -- --song <song.json> …` | rendre et mesurer un morceau (boucle de mix) |
| `cargo run --bin analyze_wav -- <f.wav>` | mesurer un WAV existant (+ spectrogramme `--png`) |
| `cargo run --bin song_lint -- <song.json>` | valider un song.json avant/après édition |
