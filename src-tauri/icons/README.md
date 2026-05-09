# App Icons

This directory should contain the following icon files for the macOS app bundle:
- 32x32.png
- 128x128.png
- 128x128@2x.png
- icon.icns
- icon.ico

## Generating Icons

Run on macOS with Rust toolchain installed:

```bash
npm run tauri -- icon assets/icon.svg
```

This will generate all required icon sizes from the source SVG.

## Placeholder Mode

For development without icons, the app will still build but show a default icon.
