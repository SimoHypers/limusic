// Pure SPA for Tauri (no Node server). context/18. With multiple routes incl. a dynamic
// /playlist/[id], the app is fully client-rendered from a single `index.html` fallback shell
// (adapter-static `fallback` in vite.config.ts) — prerender is off since ssr=false makes
// prerendered pages empty shells anyway.
export const ssr = false;
export const prerender = false;
