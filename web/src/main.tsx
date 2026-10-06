/* @refresh reload */
import { render } from "solid-js/web";
import "@/styles/tokens.css";
import "@/styles/base.css";
import "@/styles/components.css";
import "@/styles/layout.css";
import { applyLocale, locale } from "@/i18n";
import { initPrefs } from "@/lib/prefs";
import { App } from "./App";

initPrefs();
applyLocale(locale());

const root = document.getElementById("root");
if (!root) throw new Error("#root missing");
render(() => <App />, root);
