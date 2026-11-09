# App Icons

`icon-source.png` is the master: 1024×1024 RGBA, the rounded tile inset to roughly the macOS
icon grid with a transparent margin. Every other file in this directory (PNGs, `icon.icns`,
`icon.ico`, and the `ios/` and `android/` sets) is generated from it.

## Regenerating

```bash
npm run tauri -- icon src-tauri/icons/icon-source.png
```

The bundle uses `32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.icns` and `icon.ico`
(see `bundle.icon` in `src-tauri/tauri.conf.json`).
