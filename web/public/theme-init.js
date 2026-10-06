// Applies the stored theme, colour convention and language before first paint (no flash).
// A separate file so the Content-Security-Policy can forbid inline scripts.
(function () {
  try {
    var root = document.documentElement;
    var pref = localStorage.getItem("hone-quant.theme") || "auto";
    var dark = pref === "dark" || (pref === "auto" && window.matchMedia("(prefers-color-scheme: dark)").matches);
    root.dataset.theme = dark ? "dark" : "light";
    root.dataset.updown = localStorage.getItem("hone-quant.updown") || "green-up";
    if (localStorage.getItem("hone-quant.locale") === "en") root.lang = "en";
  } catch (e) {
    /* storage unavailable: defaults apply */
  }
})();
