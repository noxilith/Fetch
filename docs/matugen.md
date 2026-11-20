# Theming fetch with matugen

[matugen](https://github.com/InioX/matugen) builds a Material You color palette
from your wallpaper (or any color) and writes it into config files through
templates. With the template in this repo, fetch's labels and logo follow your
wallpaper: change the wallpaper, run matugen, and the next `fetch` uses the new
colors.

Out of the box fetch colors the logo and labels per distro. Once you set this up,
the matugen palette replaces the distro colors. You can switch back at any time
(see [Going back](#going-back)).

## What you need

- fetch built from this repository (it reads `#rrggbb` colors and a `colors` file)
- matugen 2.x or newer (checked with 4.2.0)
- a terminal with true color (kitty, Alacritty, WezTerm, foot, Ghostty, GNOME
  Terminal and most others)

## Set it up

**1. Copy the template** from this repository into matugen's template folder:

```bash
mkdir -p ~/.config/matugen/templates
cp matugen/fetch-colors ~/.config/matugen/templates/fetch-colors
```

The template is four lines:

```
label_color={{colors.primary.default.hex}}
logo_outer={{colors.primary.default.hex}}
logo_inner={{colors.tertiary.default.hex}}
```

**2. Register it.** Add this block to `~/.config/matugen/config.toml` (create the
file if it does not exist, and keep any other templates you already have):

```toml
[config]

[templates.fetch]
input_path = '~/.config/matugen/templates/fetch-colors'
output_path = '~/.config/fetch/colors'
```

`matugen/config.toml` in this repo has the same block to copy from.

**3. Generate colors.** Point matugen at a wallpaper or a color:

```bash
matugen image ~/Pictures/wallpaper.png
# or from a single color:
matugen color hex '#7c6cff'
```

matugen writes `~/.config/fetch/colors`:

```
label_color=#ffb784
logo_outer=#ffb784
logo_inner=#c8ca94
```

**4. Run fetch.** There is nothing to configure: fetch reads
`~/.config/fetch/colors` on every start.

```bash
fetch
```

## Keep it in sync with your wallpaper

fetch reads the colors when it starts, so it always shows the palette from the
last matugen run. To update both together, run matugen from whatever sets your
wallpaper. For example, in a script:

```bash
#!/bin/sh
wallpaper="$1"
swww img "$wallpaper"            # or hyprpaper, feh, your compositor's tool
matugen image "$wallpaper"       # regenerates every template, fetch included
```

Light and dark palettes: `matugen -m light image ...` or `-m dark`. Other
palette types: `matugen -t scheme-vibrant image ...` (see `matugen --help`).

## Choose different colors

`logo_outer` colors the extruded sides of the logo, `logo_inner` the front and
back faces, and `label_color` the `CPU:`, `Memory:` labels. Edit the template to
use any matugen color role, then run matugen again. Some combinations that work
well on a dark terminal:

| Look | `logo_outer` | `logo_inner` | `label_color` |
|------|--------------|--------------|----------------|
| Default of this repo | `primary` | `tertiary` | `primary` |
| One accent | `primary` | `primary` | `primary` |
| Soft | `secondary` | `on_surface` | `secondary` |
| Bright faces | `primary` | `on_surface` | `tertiary` |

A role is written like `{{colors.secondary.default.hex}}`. The full list of
roles is in the [matugen wiki](https://github.com/InioX/matugen/wiki).

Logos that ship their own colors (fastfetch logos) keep them: `logo_outer` and
`logo_inner` only color logos without their own. `label_color` always applies.

## How fetch combines the files

1. built-in defaults: per-distro logo colors, labels in the logo's color
2. `~/.config/fetch/colors` (matugen): only `label_color`, `logo_outer` and
   `logo_inner` are read from it; it never changes which fields are shown
3. `~/.config/fetch/config`: wins over the colors file for anything it sets

So a line like `label_color=white` in your own config overrides matugen for the
labels while the logo keeps following the wallpaper.

## Going back

- **Per-distro colors again:** delete `~/.config/fetch/colors` (and remove the
  `[templates.fetch]` block so matugen doesn't recreate it), or put these lines
  in `~/.config/fetch/config`, which override the colors file:

  ```
  label_color=auto
  logo_outer=auto
  logo_inner=auto
  ```

## Troubleshooting

- **Nothing changed:** check that matugen wrote the file with
  `cat ~/.config/fetch/colors`. If the output path in `config.toml` is wrong,
  matugen writes somewhere else.
- **Colors look wrong or washed out:** the terminal needs true color. In a
  terminal without it, use ANSI colors instead, for example `logo_outer=cyan` or
  `logo_outer=38;5;117`.
- **A value is ignored:** colors must be `#rrggbb` (six hex digits), a color
  name, or raw escape parameters like `38;5;216`. Comments go on their own line,
  never after a value.
- **Only the logo or only the labels changed:** your `config` may set
  `label_color` or `logo_outer` itself, and that wins. Remove those lines.
