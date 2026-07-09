import { createApp } from "vue";
import { createPinia } from "pinia";

import "@fontsource-variable/inter";
import "@fontsource-variable/space-grotesk";
import "@fontsource-variable/jetbrains-mono";
import "./styles/base.css";

import App from "./App.vue";
import { i18n } from "./i18n";
import { applyTheme } from "./styles/theme";

applyTheme();
createApp(App).use(createPinia()).use(i18n).mount("#app");
