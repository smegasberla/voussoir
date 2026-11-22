// Deterministic type: the two faces the promo uses, loaded at registration
// time so every frame renders with the same glyphs.
import { loadFont as loadInter } from "@remotion/google-fonts/Inter";
import { loadFont as loadMono } from "@remotion/google-fonts/JetBrainsMono";

export const { fontFamily: interFamily } = loadInter("normal", {
  weights: ["400", "500", "600", "800"],
});
export const { fontFamily: monoFamily } = loadMono("normal", {
  weights: ["400", "500"],
});
