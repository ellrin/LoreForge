# Icons for figures

Two open icon sets, whole, for the pictures a flow figure or a timeline puts beside its text.
Each is one JSON file in Iconify's format (`icons.<name>.body` is the inside of an `<svg>`,
`width` / `height` the grid), read locally: nothing is fetched when a figure is drawn.

| File | Set | Icons | Licence |
|---|---|---|---|
| `tabler.json` | Tabler Icons (`@iconify-json/tabler` 1.2.47) | 6199 | MIT, `LICENSE-tabler.txt` |
| `healthicons.json` | Health Icons (`@iconify-json/healthicons` 1.2.18) | 2797 | icons CC0 1.0, `LICENSE-healthicons.txt` |

- Tabler: thin lines; general, networking (router, server, firewall, topologies), research.
  Its 413 `brand-*` icons (company logos) are left out: the licence covers the drawing, not the trademark.
- Health Icons: people and care (doctor, nurse, patient, pharmacy, prescription, x-ray, organs).
  Each icon comes as `<name>` (filled), `<name>-outline` and `<name>-negative`.
- Both may be used in a published paper's figures. The licence files stay beside the sets
  because this folder redistributes them.

To update: download `icons.json` of the package's new version, drop Tabler's `brand-*`, write
it with sorted keys, and change the versions above.
