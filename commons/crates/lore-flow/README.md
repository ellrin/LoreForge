# lore-flow

Flow charts and architecture figures from a data-flow graph. Design: ecosystem design §7.6.

```
L1  spec / dsl / llm     what is in the figure: nodes, edges, panels, variables (no coordinates, no colours)
L2  layout / route       where it sits and how the lines run
L3  modules / build      lore-canvas objects: native shapes, text boxes, attached connectors, groups
```

```rust
use lore_flow::*;
let g = dsl::parse("fig demo\nx: image \"$x$\"\nf: unet \"$\\theta$\"\nx -> f\n")?;
let fig = draw(&g, &Drawing::default())?;                 // Figure { layout, routes, built }
let ops = insert_ops(&slide_uid, slide.objects.len(), fig.objects());
editor.apply(ops, "流程圖")?;                              // one undo step
```

## The text form

```text
fig demo
arrange a / b c                      # rows of panels; or: template rows | cols | <template id>
panel a "a. Training"                # dir=down for top-to-bottom, bare for no frame
  x:    image "$x$" src=frames/1.png # a real picture (path relative to the file); look=photo|mask|heatmap|plot otherwise
  net:  unet "@theta" depth=3        # @id = a variable of the notation table / vars_new
  enc:  encoder "Enc" style=3d       # art style: built-in, or a saved drawing of the library
  loss: loss "$\mathcal L_{seg}$"
  y:    image "$y$" role=gt col=5 lane=0   # col / lane pin a position (lane 0 = the trunk)
  x -> net -> loss
  y -> loss : supervision
  loss -> net : feedback "update"
end
var theta = "\theta_{seg}" "segmentation model"
```

Module kinds: box, op, loss, label, image, image-stack, feature-stack, vector-bar, matrix,
layer-bars, cuboid-seq, unet, encoder, decoder, nn, brace, plus the kinds of the library.
Line kinds: data, supervision, feedback, loss-out, compare, optional, shared, skip, concat,
down, up, cross, stage, repeat, zoom. `\\` breaks a line inside a label.

## Layout and lines

Panels follow `arrange` / the template (`PanelTemplate`: fraction rects) or the arrangement that
fits the frame best. Inside a panel: flow edges give the layers, the longest chain is the straight
trunk, other chains take the nearest free lane; cells are centred, gaps equal. Lines are routed at
right angles on a sparse grid: never through a module or a panel title, feedback under the row,
lines between panels in the gaps, unrelated lines never on one track. Up to three corners = a
straight/bent connector attached to its modules; more = an open polyline.

## Modules, styles, the library

A module of one shape is that shape; anything else is a group whose first child `port` is an
invisible rectangle over the frame (PowerPoint attaches connectors to shapes, not groups:
`retarget_for_pptx` points connector ends at it before export; inside lore-canvas they are attached
to the group). Connection sites are the frame's mid-sides.

- Built-in styles: `modules::generator_styles(kind)` (encoder/decoder 7, nn 5, brace 5, …).
- Library folder: `modules/<kind>/<style>.json` (`ModuleTemplate`: size, optional `port` rect,
  canvas objects with relative frames, colours `role:fill|line|text` / `theme:*`, a `{{label}}` run).
  `Library::load_dir`, `Library::save` (never overwrites), `template_from_objects` (a selection →
  a module file), `sheet::contact_sheet`, `sheet::builtin_templates`.
- A node's `style`, else the theme's `[modules.<kind>] style`, picks the drawing; a library style
  of a built-in kind wins over the built-in drawing of the same name.

## Themes

TOML over the built-in `classic-bw` (`themes/classic-bw.toml`): fonts (`family`, `title`,
`caption`, `math`, `ea`, sizes), role colours, per-kind colours and style (`[modules.unet]`),
panel frames, every line kind, 3-D shading, spacing. Any 6-digit colour is allowed.

## Variables

Labels refer to the notation registry by id (`@id`, `var=`); registered macros are expanded
before conversion. `notation::check_graph` reconciles `vars_new` with the registry (existing
LaTeX → the registry's id; taken id → a free one), lints figure conventions and runs
lore-notation's checks on the figure's text; `notation::register_new` adds the new ones to the
*document's* table only (tag `flow-draft`). Math is Office Math when a `MathSource` converts it
(`PandocMath`), else text runs with Unicode symbols and sub/superscripts.

## LLM task

`llm::task_for(has_image)` = `sc-flow-graph` / `sc-flow-graph-vision` (`llm::task_table()` to merge into the app's table):
`llm::propose(provider, &FlowPrompt { description, image, notation, library, examples })` →
`Proposal { graph, dsl, notation, … }`. Closed lists in the JSON schema; a graph that does not
hold together is sent back once.

## Command line

```sh
cargo run -p lore-flow -- draw fig.flow -o fig.pptx --png fig.png [--theme t.toml] [--modules DIR] [--template t.json] [--notation n.json]
cargo run -p lore-flow -- graph desc.md -o fig.flow [--image ref.png] [--engine claude:sonnet]
cargo run -p lore-flow -- check fig.flow --notation n.json
cargo run -p lore-flow -- sheet -o modules.pptx --png modules.png [--modules DIR]
cargo run -p lore-flow -- export-modules DIR
```

The PNG is the SVG preview through headless Chrome. **Not tested on real hardware**: opening the
exported files in PowerPoint itself (checked here: round trip through lore-canvas, the XML, macOS
Quick Look for geometry).
