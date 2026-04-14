# Bundle icons

Place the Tauri-style icon assets in this directory **before the first
`pnpm tauri dev` run or any `pnpm tauri build`**.

Generate them from a single 1024×1024 (or larger) PNG source:

```powershell
pnpm tauri icon path\to\source.png
```

The command fills this directory with:

- `32x32.png`
- `128x128.png`
- `128x128@2x.png`
- `icon.ico`       (Windows — required for the MSI bundle)
- `icon.icns`      (macOS — unused here but always generated)
- `icon.png`       (tray icon; referenced by `app.trayIcon.iconPath`)
- platform-specific store / lockscreen assets

Until this step runs, `pnpm tauri dev` will print a warning about
missing icons and fall back to the Tauri default logo; `pnpm tauri
build` will fail with a clearer error pointing at the missing paths.
