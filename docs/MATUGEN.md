# Wallpaper palettes: Matugen, wallust and pywal

Wallpaper palette support is **opt-in**. Limusic never reads or watches the palette file unless
the **Wallpaper palette** switch under Settings ▸ Appearance is on.

## What it does

While enabled, Limusic injects the CSS in `matugen.css` as a stylesheet and adds the
`matugen-theme` class to `<html>`. The palette then owns the application's color tokens, accent
included: the accent picker and "Adapt colors to artwork" are switched off while it's on. When
your tool regenerates `matugen.css` after a wallpaper change, Limusic reloads it live.

Limusic doesn't care what wrote the file. Templates for three tools ship in this folder:

| Tool | Template | Light and dark |
|---|---|---|
| [Matugen](https://github.com/InioX/matugen) | [`matugen-template.css`](matugen-template.css) | Both schemes, follows Limusic's mode |
| [wallust](https://codeberg.org/explosion-mental/wallust) | [`wallust-template.css`](wallust-template.css) | One palette, used in both modes |
| [pywal16](https://github.com/eylles/pywal16) | [`pywal-template.css`](pywal-template.css) | One palette, used in both modes |

wallust and pywal make one palette, dark or light. Keep Limusic's own light/dark setting the same
as the palette's: a few controls (button hovers, inputs) take their tint from Limusic's mode, not
from the palette, and look slightly off when the two disagree.

## Expected CSS path

Limusic reads `limusic/matugen.css` from the system config directory:

- Linux: `~/.config/limusic/matugen.css`
- Windows: `%APPDATA%\limusic\matugen.css`
- macOS: `~/Library/Application Support/limusic/matugen.css`

The file keeps the name `matugen.css` whichever tool writes it.

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

## Configure wallust

1. Copy [`wallust-template.css`](wallust-template.css) into `~/.config/wallust/templates/` as
   `limusic.css`.
2. Add it to the `[templates]` table in `~/.config/wallust/wallust.toml`:

```toml
[templates]
limusic = { template = "limusic.css", target = "~/.config/limusic/matugen.css" }
```

3. Run wallust against an image: `wallust run /path/to/wallpaper.png`.

The template picks `color12` as the accent on dark palettes and `color4` on light ones. If the
accent still comes out too dark to read, turn on `check_contrast = true` in `wallust.toml`.

## Configure pywal

The template is written for [pywal16](https://github.com/eylles/pywal16), the maintained fork.

1. Copy [`pywal-template.css`](pywal-template.css) into `~/.config/wal/templates/` as
   `limusic.css`.
2. Run pywal once (`wal -i /path/to/wallpaper.png`), which writes `~/.cache/wal/limusic.css`.
3. pywal writes its templates into its cache directory, so link Limusic's path to that file:

```sh
mkdir -p ~/.config/limusic
ln -sf ~/.cache/wal/limusic.css ~/.config/limusic/matugen.css
```

Limusic follows the link, so every later `wal` run reloads the palette live. Create the link
before turning the switch on, or turn it off and on again afterwards.

pywal's light schemes (`wal -l`) keep the wallpaper's colors as they are, so on a pale wallpaper
the accent can be hard to read. That is the same palette your terminal gets; a darker wallpaper
or a dark scheme avoids it.

## Enable in Limusic

Open Settings ▸ Appearance and turn **Wallpaper palette** on. Limusic starts its file watcher and loads
the generated palette. With the switch off (the default), no watcher starts and the
`limusic/matugen.css` file is not read or created.

## Disable

Turn **Wallpaper palette** off. Limusic stops the watcher, removes the injected stylesheet and restores
the normal theme, including your own accent. Your template and generated CSS file are not
deleted.

## Troubleshooting

- **No colors change:** verify your tool wrote `matugen.css` at the expected path (for pywal,
  that the link points at `~/.cache/wal/limusic.css`), and that the file contains
  `html.matugen-theme` selectors.
- **Invalid template syntax:** run the tool by hand (`matugen image <file>`, `wallust run <file>`
  or `wal -i <file>`) and read its errors.
- **Watcher not updating:** make sure the tool writes (not just prints) the file, then turn
  **Wallpaper palette** off and on again.
