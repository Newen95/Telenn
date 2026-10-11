# Telenn — guide pour l'agent musical

Telenn est un DAW pilotable par une IA (Claude Code, Codex ou autre) : tout le
projet musical est du texte, l'agent compose, crée des sons et mixe en éditant
des fichiers pendant que le musicien voit et entend le résultat en direct.

## Composer et mixer

- Un morceau = `projects/<nom>/song.json` (format décrit dans `ARCHITECTURE.md`).
  L'app recharge le fichier à chaud dès qu'il change et affiche le diff.
- **Identifiants stables.** Chaque note et chaque clip porte un `id` : toujours
  **préserver** les `id` existants, n'en créer que pour les objets neufs, ne
  jamais les renuméroter. Adresser les éditions par `id`, jamais par position.
- Mixer = éditer gains, pans et sends dans `song.json`, pas le code.
- Valider chaque édition avec `song_lint` (voir plus bas).

## Base de connaissances : `knowledge/`

Mémoire versionnée, indépendante de l'IA utilisée : recettes de sons
(`sounds/`, preset + fiche), fiches de style (`styles/`), techniques, leçons.

- Lire `knowledge/README.md` (conventions) puis `knowledge/INDEX.md` **avant
  toute création sonore ou stylistique** — une fiche existe peut-être déjà.
- Après le travail, écrire ce qui a été appris (fiche + INDEX dans le même
  commit, échecs compris).

## Workflows

- « Crée-moi un son » → `knowledge/workflows/sound-design.md`.
- « Compose dans le style de… » → `knowledge/workflows/compose-style.md`.

Les deux imposent la boucle rendre → mesurer → ajuster et l'enregistrement dans
`knowledge/`. Les skills de `.claude/skills/` y renvoient.

## Les « oreilles » de l'agent : jamais de rendu non mesuré

À lancer depuis `src-tauri/` :

| Commande | Rôle |
|---|---|
| `cargo run --bin module_specs` | catalogue des modules (params, plages réelles, quirks) |
| `cargo run --bin render_probe -- --module <id> …` | rendre une note/gamme et la mesurer |
| `cargo run --bin render_probe -- --song <song.json> …` | rendre et mesurer un morceau |
| `cargo run --bin analyze_wav -- <f.wav> [--png p]` | mesurer un WAV, spectrogramme lisible |
| `cargo run --bin reference_match -- <rendu.wav> <réf.wav>` | diff chiffré rendu vs référence (loudness, bandes, stéréo, rythme) |
| `cargo run --bin transcribe -- <réf.wav> […]` | transcription partielle d'une référence (basse, batterie, accords) |
| `cargo run --bin song_lint -- <song.json>` | valider un song.json après édition |
| `cargo run --bin song_stats -- <song.json> [--vs <transcription.json>]…` | métriques de composition + garde anti-plagiat contre des références transcrites |

## Nouvel instrument ou effet

Si aucun module existant ne permet le son voulu, un nouveau module se code en
Rust : suivre `CONTRIBUTING.md` (section Conventions, « Nouveau module audio »)
et ses règles temps réel.
