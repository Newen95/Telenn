# Son : <nom> (gabarit)

> Copier en `sounds/<nom-kebab>.md` + `sounds/<nom-kebab>.preset.json`
> (format preset de l'app : `{"module": "...", "params": {…normalisés…}}`,
> déposable tel quel dans `projects/<p>/presets/`). Ligne d'INDEX.md dans le
> même commit.

## Intention

- Brief d'origine (« basse ronde et chaude pour … ») et rôle conseillé
  (bass/lead/pad/perc) ; registre utile (pitches MIDI où le son tient).

## Recette

- Chaîne : `<instrument> -> <fx> -> …`
- Preset : [`<nom>.preset.json`](./<nom>.preset.json)

| Param | Valeur réelle | Normalisé |
|---|---|---|
| ex. `cutoff` | 800 Hz | 0.534 |

## Mesures de référence

Commande exacte (reproductible) :

```
cargo run --bin render_probe -- --module <id> --set <…> --pitches <grave,médium,aigu>
```

| Mesure | Valeur | Cible du brief |
|---|---|---|
| centroïde | | |
| attaque | | |
| RT60 | | |
| flatness / ratio harmonique | | |

## Verdicts & historique

- `AAAA-MM-JJ` — créé (mesures conformes au brief). Statut : plausible.
- `AAAA-MM-JJ` — écoute humaine : <retour exact>. Statut : validé / à revoir.
