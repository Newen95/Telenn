# ARCHITECTURE.md — Moteur audio & formats

## Les trois couches

```
┌─────────────────────────────────────────────────────┐
│ UI (Vue 3 dans Tauri)                                │
│ timeline · piano roll · graphe (Vue Flow) · mixeur   │
└───────────────┬─────────────────────▲────────────────┘
        commandes Tauri        events + télémétrie 60fps
┌───────────────▼─────────────────────┴────────────────┐
│ Couche contrôle (Rust, threads normaux)              │
│ séquenceur · projet/undo · MIDI in · file watcher ·  │
│ construction du graphe · export WAV (offline)        │
└───────────────┬─────────────────────▲────────────────┘
     ring buffers lock-free (rtrb)  télémétrie (rtrb)
┌───────────────▼─────────────────────┴────────────────┐
│ Thread audio (temps réel, RT-SAFE)                   │
│ exécution du graphe · hot-swap · sortie cpal         │
└──────────────────────────────────────────────────────┘
```

Un seul processus. La modularité est dans le **graphe**, pas dans des services.

## Le graphe audio

Graphe orienté acyclique de nœuds. Trois types de signaux entre ports :

| Type         | Contenu                          | Cadence            |
|--------------|----------------------------------|--------------------|
| `Notes`      | événements note on/off, vélocité | événementiel       |
| `Audio`      | buffers f32, stéréo par défaut   | sample rate        |
| `Modulation` | valeurs de contrôle              | control rate (~1/64 du SR) |

### Trait central

```rust
pub trait AudioNode: Send {
    /// RT-SAFE. Remplit `out` pour `frames` échantillons.
    fn process(&mut self, ctx: &ProcessCtx, io: &mut NodeIo);

    /// Déclaration des ports (types, noms). Appelé hors temps réel.
    fn ports(&self) -> PortSpec;

    /// Paramètres exposés (id, nom, plage, unité, courbe, défaut).
    fn params(&self) -> &[ParamSpec];

    /// RT-SAFE. Changement de paramètre (déjà lissé côté appelant si besoin).
    fn set_param(&mut self, id: ParamId, value: f32);

    /// Préparation hors temps réel (allocations ici, jamais dans process).
    fn prepare(&mut self, sample_rate: f32, max_block: usize);
}
```

Le registre de modules associe un identifiant stable (`"fm_piano"`, `"reverb"`)
à un constructeur. `song.json` référence ces identifiants.

### Exécution

- Ordre topologique précalculé hors temps réel, stocké à plat (vec d'indices).
- Buffers pré-alloués dans `prepare()`. Le callback ne fait que traverser et
  appeler `process`.
- Lissage des paramètres (anti zipper-noise) : rampe sur ~5-20 ms pour les gains,
  filtres 1 pôle pour les modulations.

## Hot-swap (modification du graphe sans interruption)

1. La couche contrôle clone la description du graphe, applique la modification,
   précalcule le nouvel ordre, alloue les nouveaux buffers, `prepare()` les
   nouveaux nœuds.
2. Elle envoie le nouveau graphe au thread audio via ring buffer (transfert de
   propriété, pas de copie en RT).
3. Le thread audio exécute ancien et nouveau en parallèle pendant 10-20 ms avec
   crossfade équi-puissance, puis renvoie l'ancien graphe à la couche contrôle
   pour destruction (jamais de `drop` en RT).

Les nœuds inchangés sont **réutilisés** (déplacés dans le nouveau graphe) pour
préserver leur état interne (queues de reverb, phases d'oscillateurs).

## Communication

- **UI -> contrôle** : commandes Tauri typées (`add_node`, `move_node`,
  `set_param`, `add_note`, `transport_play`...).
- **Contrôle -> audio** : ring buffer de messages compacts (enum sans allocation) :
  changements de paramètres, événements de notes, swap de graphe, transport.
- **Audio -> UI (télémétrie)** : ring buffer écrit par le thread audio à chaque
  bloc : peaks/RMS par nœud et par câble, position de lecture, valeurs de
  modulation, compteur de xruns. La couche contrôle agrège et pousse un event
  Tauri par frame d'animation (~60 fps). Flux strictement unidirectionnel.

## Entrée MIDI & latence

- `midir` reçoit sur son propre thread, timestampe, pousse dans le ring buffer.
- Cibles : Linux PipeWire 64-128 samples (~3-6 ms touche->son) ;
  Windows WASAPI exclusif (~10 ms), option ASIO via feature `cpal/asio` plus tard.
- Tout xrun est compté et affiché (indicateur UI), jamais silencieux.

## Rendu offline (export WAV)

Le **même graphe** est exécuté hors temps réel, bloc par bloc, plus vite que le
temps réel, vers `hound`. Aucune logique dupliquée : seul l'horloge et la
destination changent.

## Format projet — `song.json` (source de vérité)

```json
{
  "version": 1,
  "bpm": 120,
  "time_signature": [4, 4],
  "tracks": [
    {
      "id": "lead",
      "name": "Lead",
      "chain": [
        { "id": "n1", "module": "fm_piano", "preset": "presets/soft_keys.json",
          "params": { "brightness": 0.6 } },
        { "id": "n2", "module": "delay", "params": { "time_sync": "1/8.", "feedback": 0.35 } }
      ],
      "mixer": { "gain_db": -3.0, "pan": 0.1, "sends": { "reverb_bus": -12.0 } },
      "clips": [
        { "start": "1.1.1", "length": "4.0.0",
          "notes": [ { "pitch": 67, "start": "1.1.1", "length": "0.1.0", "vel": 96 } ] }
      ]
    }
  ],
  "buses": [
    { "id": "reverb_bus", "chain": [ { "id": "rv", "module": "reverb",
      "params": { "size": 0.8, "mix": 1.0 } } ] }
  ],
  "modulations": [
    { "source": { "node": "lfo1", "port": "out" },
      "target": { "node": "n1", "param": "cutoff" }, "amount": 0.3 }
  ]
}
```

- Positions en `mesure.temps.tick` (lisible et diffable).
- Un **file watcher** surveille le projet ; toute modification externe (ex. par
  Claude Code) est rechargée à chaud avec diff visuel dans l'UI.
- Les presets d'instruments sont des JSON séparés référencés par chemin.
- `master_chain` (optionnel, omis quand vide) : chaîne d'effets du bus master,
  mêmes `NodeSpec` que les chaînes de piste mais **sans instrument de tête**
  (effets seulement, validé par le compilateur de graphe et `song_lint`).
  Adressée dans les commandes d'édition par l'id réservé `_master`.

## Étage mixer & bus master (stéréo)

Le graphe traite `CHANNELS = 2` canaux par port (layout channel-major par
port, offsets calés sur les `frames` du bloc courant). Les instruments
écrivent le canal 0 puis le dupliquent (`spread_output`) ; les effets tiennent
un état de signal par canal ; chorus et reverb décorrèlent gauche/droite avec
un repli mono égal au wet mono d'origine (compatibilité mono par
construction). Le compresseur détecte sur `max(|L|,|R|)` (liaison stéréo).

Chemin du signal, identique en temps réel et à l'export :

```
pistes → mix_tracks (gains/pans par piste, bus planaire [L,R], SANS gain master)
      → [RT] add_faded (queues de l'ancien graphe pendant un hot-swap)
      → process_master (la master_chain traite le bus, nœuds ordinaires du
        graphe exclus de la marche topologique — adoptés par id au swap)
      → apply_master (gain master = trim final, entrelacement)
      → [RT] métronome (jamais pompé par un limiteur master) → declick
```

## Spécifications précises

### Horloge & positions

- Résolution : **PPQ 960** (ticks par noire). Le compteur d'échantillons (u64)
  est l'horloge maîtresse ; les ticks sont dérivés :
  `samples_per_tick = sample_rate * 60 / (bpm * 960)`.
- Positions textuelles `"mesure.temps.tick"` : mesure et temps 1-indexés,
  tick 0-959 (ex. `"3.2.480"` = mesure 3, 2e temps, croche pointée du temps).
  Les durées utilisent la même notation mais 0-indexée (`"0.1.0"` = un temps).
- Interdiction d'accumuler du temps en flottant : u64 partout, conversions à
  la demande.

### Messages contrôle → audio

```rust
pub enum ControlMsg {
    Transport(TransportCmd),                  // Play, Stop, Seek{sample}, SetLoop{start,end}
    NoteOn  { node: NodeIdx, pitch: u8, vel: u8 },
    NoteOff { node: NodeIdx, pitch: u8 },
    SetParam { node: NodeIdx, param: ParamId, value: f32 }, // valeur normalisée 0..1
    SwapGraph(Box<CompiledGraph>),            // transfert de propriété
}
```

- `NodeIdx` est l'index plat dans le graphe compilé (résolu côté contrôle à
  partir des `node_id` textuels de song.json).
- Ring `rtrb` capacité 1024 ; le callback draine tout en début de bloc.
- **Canal poubelle** audio → contrôle (`rtrb`, cap. 8) : les anciens
  `CompiledGraph` y sont poussés après crossfade et droppés hors temps réel.

### Télémétrie (audio → UI)

```rust
pub const MAX_METERS: usize = 64;

#[derive(Clone, Copy)]
pub struct TelemetryFrame {
    pub playhead_samples: u64,
    pub playing: bool,
    pub audio_load: f32,        // temps de calcul du bloc / durée du bloc
    pub xruns: u32,             // cumulatif
    pub meter_count: u8,
    pub meters: [Meter; MAX_METERS],  // par nœud : peak, rms (post-process)
}
```

- Capacité fixe, `Copy`, zéro allocation. Écrit chaque bloc dans un `rtrb`
  (cap. 64) ; si plein, on écrase (jamais bloquer).
- La couche contrôle draine, garde le max des peaks entre deux frames UI, et
  émet un event Tauri `telemetry` à 60 Hz maximum.

### Paramètres

- Déclarés par le module : `ParamSpec { id, name, unit, min, max, default,
  curve }` avec `curve ∈ {Linear, Log, Db}`. `min`/`max`/`default` sont des
  valeurs **réelles** (Hz, ms, dB) ; `song.json`, les presets et les messages
  moteur transportent des valeurs **normalisées 0..1**. La conversion se fait
  via la curve (`normalize`/`denormalize`) : l'UI et les outils affichent le
  réel, le disque et le fil restent normalisés. Cas particulier `Db` :
  `min`/`max` sont en dB mais `denormalize` renvoie un **gain linéaire**.
- Lissage dans le nœud : rampe linéaire 10 ms pour les gains, filtre 1 pôle
  pour le reste. `set_param` pose une cible, jamais la valeur courante.



- **Plugins CLAP** : un nœud `ClapHost` (crate `clack`) — juste un `AudioNode`
  de plus, le moteur n'en sait rien.
- **Pont VST3** : si un jour nécessaire, processus hôte séparé (type Carla)
  relié par mémoire partagée, encapsulé lui aussi dans un nœud.
- **DSP en Faust** : modules compilés en Rust/C via le compilateur Faust,
  enveloppés dans `AudioNode`.
