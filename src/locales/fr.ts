export default {
  app: { title: "Telenn" },
  transport: { play: "Lecture", stop: "Stop", metronome: "Métronome" },
  view: {
    timeline: "Timeline",
    pianoRoll: "Piano roll",
    chain: "Chaîne",
    mixer: "Mixeur",
    browser: "Bibliothèque",
  },
  ui: { collapse: "Replier", expand: "Déplier" },
  menu: {
    file: "Fichier",
    edit: "Édition",
    view: "Affichage",
    exportWav: "Exporter en WAV",
    reloadProject: "Recharger le projet",
    undo: "Annuler",
    redo: "Rétablir",
    devOverlay: "Console dev",
  },
  meter: { master: "Master", cpu: "CPU", xruns: "xruns" },
  edit: {
    hints: "Double-clic : créer · glisser : déplacer · bords : redimensionner · clavier : préécouter · Suppr : supprimer · Ctrl+Z : annuler",
  },
  browser: {
    instruments: "Instruments",
    modules: "Effets",
    search: "Rechercher…",
    dropToSave: "Glissez un effet ou un instrument ici pour l'enregistrer",
  },
  prompt: { presetName: "Nom du preset" },
  chain: {
    prevTrack: "Piste précédente",
    nextTrack: "Piste suivante",
    replaceInstrument: "Changer d'instrument",
  },
  timeline: { addTrack: "Ajouter une piste" },
  empty: {
    noTracks: "Aucune piste — cliquez sur + ou déposez un instrument ici",
    pianoRoll: "Créez ou sélectionnez une piste pour éditer ses notes",
    chain: "Créez une piste dans la timeline pour afficher sa chaîne",
  },
  toast: {
    exported: "Export WAV terminé",
    actionFailed: "L'action a échoué",
    projectError: "song.json invalide — le projet n'a pas été rechargé",
  },
};
