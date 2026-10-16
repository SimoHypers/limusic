# Matugen integration

Matugen support is **opt-in**. Limusic never reads or watches a Matugen file unless the
**Matugen** switch under Settings ▸ Appearance is on.

## What it does

While enabled, Limusic injects the generated CSS from your Matugen output file as a
stylesheet and adds the `matugen-theme` class to `<html>`. The generated palette then owns
the application's color tokens, accent included: the accent picker and "Adapt colors to
artwork" are switched off while it's on. When Matugen regenerates `matugen.css` after a
wallpaper change, Limusic reloads it live.

## Expected CSS path

Limusic reads `limusic/matugen.css` from the system config directory:

- Linux: `~/.config/limusic/matugen.css`
- Windows: `%APPDATA%\limusic\matugen.css`
- macOS: `~/Library/Application Support/limusic/matugen.css`

## Configure Matugen

1. Copy [`matugen-template.css`](matugen-template.css) into your Matugen templates directory
   as `limusic.css`.
2. Add a template entry to your Matugen config, for example:

```toml
[templates.limusic]
input_path = "~/.config/matugen/templates/limusic.css"
output_path = "~/.config/limusic/matugen.css"
```

Adjust `input_path` to match your setup; `output_path` must be the path above for your system.

3. Run Matugen against an image:

```sh
matugen image /path/to/wallpaper.png
```

Or otherwise regenerate after changing wallpaper.

## Enable in Limusic

Open Settings ▸ Appearance and turn **Matugen** on. Limusic starts its file watcher and loads
the generated palette. With the switch off (the default), no watcher starts and the
`limusic/matugen.css` file is not read or created.

## Disable

Turn **Matugen** off. Limusic stops the watcher, removes the injected stylesheet and restores
the normal theme, including your own accent. Your template and generated CSS file are not
deleted.

## Troubleshooting

- **No colors change:** verify Matugen wrote `matugen.css` at the expected path and that the
  template is configured with that `output_path`.
- **Invalid template syntax:** validate by running `matugen image <file>`; check that the
  generated file contains `html.matugen-theme` and `html.matugen-theme.dark` selectors.
- **Watcher not updating:** make sure Matugen writes (not just prints) the file, then turn
  Matugen off and on again.
