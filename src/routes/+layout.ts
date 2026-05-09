// Disable SSR for the entire app.
// Required for Tauri: load functions must run in the WebView (not at build time)
// because Tauri APIs are only available at runtime.
export const ssr = false;
