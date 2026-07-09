import { createI18n } from "vue-i18n";

import en from "./locales/en";
import fr from "./locales/fr";

// French is the default; English is provided alongside it (every key in both).
// New locales drop in here without touching components. The last language
// picked in the View menu is remembered across sessions.
const stored = localStorage.getItem("telenn.locale");

export const i18n = createI18n({
  legacy: false,
  globalInjection: true, // make `$t` available in templates
  locale: stored === "en" ? "en" : "fr",
  fallbackLocale: "en",
  messages: { fr, en },
});
