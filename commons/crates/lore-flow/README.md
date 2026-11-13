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
  enc:  encoder "Enc" style=slabs finish=outline   # style = what is drawn, finish = how it is painted
  loss: loss "$\mathcal L_{seg}$"
  y:    image "$y$" role=gt col=5 lane=0   # col / lane pin a position (lane 0 = the trunk)
  x -> net -> loss
  y -> loss : supervision
  loss -> net : feedback "update"
end
var theta = "\theta_{seg}" "segmentation model"
```

Module kinds: box, op, loss, label, image, image-stack, feature-stack, vector-bar, matrix,
layer-bars, cuboid-seq, unet, encoder, decoder, autoencoder, nn, brace, plus the kinds of the library.
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
to the group). Connection sites are the frame's mid-sides. Every part is a native shape, text box
or line (arcs are freeform curves), so a module can be ungrouped and edited like anything else.

**Style = construction, finish = paint.** A style is a different drawing, recognisable as a
thumbnail; how it is painted is the finish, which every style accepts (`finish.rs`). The test
`styles_are_constructions_and_finish_is_paint` fails when two styles of a kind are the same
drawing once the paint is fixed, or when a finish changes what is drawn.

Built-in styles (`modules::generator_styles(kind)`, the first is the default):

| kind | styles |
|---|---|
| encoder (17) | trapezoid, bars, cubes (PlotNeuralNet blocks + sizes), slabs (SegNet), planes, lenet (NN-SVG), fc, resnet, dense, unet, transformer, rnn, vit, fpn, vae (μ, σ, sampling), graph, layers |
| decoder (16) | the same mirrored (transformer: masked + cross attention; rnn: outputs ŷ), without vae |
| autoencoder (17) | an encoder style, its bottleneck and the mirrored decoder in one drawing: SegNet pooling indices, U-Net skips, FPN laterals, seq2seq context, transformer memory, VAE sampling; Encoder / Latent space / Decoder under it (`names=off` for one model), `latent=vector\|neurons\|context\|none` |
| nn (15) | circles, labelled (x₁…xₙ, +1 bias, x̂, layer names), dropout, neuron (Σ, f), pills, fans, attention, graph, recurrent (loop = unrolled), lstm, conv, residual, heads (multi-head attention), moe, rbm |
| brace | curly, bracket, paren, angle, dimension (`ends=dash` marks a section, `heads=none`) |
| feature-stack | offset, 3d, cube (H, W, C), grid, row |
| image-stack | stack, film, mosaic, row |
| box | rect, trapezoid, parallelogram, hexagon, ellipse, can |
| loss | plain, box, circle · op: circle, box, plain · unet: bars, lineart |

Old names still draw: `3d`→cubes, `stacked`→planes, `square`→bracket, `span`→dimension with
dashed ends, box `round`/`pill` and loss `pill` = a radius finish.

**Finish** words (`finish=outline,translucent` on a node; `[node] finish` and
`[modules.<kind>] finish / line_w / radius` in the theme; the node wins): `solid` | `outline` |
`gradient` | `flat`, `translucent` | `opaque`, `typed` (colour by layer type or role, with the
colour legend) | `mono`, `round` | `square`, `thin` | `bold`; plus `fill=`, `line=`, `line_w=`,
`radius=`. A misspelt word is reported by `notation::check_graph` (`finish_word`).

**Colour legend.** Drawings coloured by type take `[layers.<type>]` (`color`, `label`) from the
theme: conv, pool, up, softmax, fc, attn, norm, ffn, embed, token, cls, cell, enc, latent, dec,
other (classic-bw: grey levels). `cuboid-seq classes=[…]` uses any type the theme defines;
layer bars colour a kind of layer the same in every module of the figure. When a figure has two
or more types, one legend row goes over it (`Layout::swatches`; `legend off` hides it).

- Library folder: `modules/<kind>/<style>.json` (`ModuleTemplate`: size, optional `port` rect,
  canvas objects with relative frames, colours `role:fill|line|text` / `theme:*` / `layer:<type>`,
  a `{{label}}` run). `Library::load_dir`, `Library::save` (never overwrites),
  `template_from_objects` (a selection → a module file), `sheet::contact_sheet`,
  `sheet::builtin_templates` (`export-modules`: encoder, decoder, autoencoder, nn, brace).
- A node's `style`, else the theme's `[modules.<kind>] style`, picks the drawing; a library style
  of a built-in kind wins over the built-in drawing of the same name.

## Themes

TOML over the built-in `classic-bw` (`themes/classic-bw.toml`): fonts (`family`, `title`,
`caption`, `math`, `ea`, sizes), role colours, per-kind colours, style and finish
(`[modules.unet]`), layer-type colours (`[layers.conv]`), panel frames, every line kind, 3-D
shading, spacing. Any 6-digit colour is allowed. `private-example/SlideComposer/flow/themes/`
also has `soft-colour.toml` (SegNet colours: blue conv, green pooling, red upsampling).

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

## The figure spec and the shared editor

`figure::FlowSpec` (`lore_flow: 1`) is what rides with a document: the L1 text (`dsl`) plus the
author's choices — `layout` (a template id), `dirs` (block → `right`/`down`), `theme`, `types`
(kinds of block with fill/outline), `blocks` / `nodes` (→ a type), `lines` (line kind → colour, dash,
width, meaning), `legend` (`show` auto|on|off, `pos` below|above, `rows` of `line:<kind>` /
`type:<id>` / `block:<id>` with words), `frame` (pt), `discussion` (the steps' answers). Unknown keys
are kept. `FlowSpec::prepare` turns it into graph + theme + `LayoutOptions` (template, directions,
panel fills, node `fill=`/`line=`, edge looks, `LegendChoice`); `figure::render` draws it,
`preview_svg` crops a plain SVG, `as_picture` wraps the objects as one group (`lock_aspect`, the spec
in `data`, connectors retargeted for PowerPoint), `linear` gives one line per block.

Layout templates (`layouts.json`, 20 neutral ones: rows, columns, grids, small/big splits, two small +
one big, wide top + two/three below …; `FlowLayout`: `arrange` row|column|grid or fraction `panels`,
`dir` / `dirs`) and themes are data: `Catalogue::load_all([private/shared/flow, <app>/flow])` adds
the author's `layouts/*.json` (or `templates/*.json`), `themes/*.toml`, `modules/`; a private id wins.

`web::router(Flow::new(home).with_llm(…).with_notation(…).with_app_dir(…))` mounts the editor like
lore-plot's `/plot/*` — `/flow/` (`commons/ui/flow/editor.js`, `LoreFlow.edit({ spec, doc, frame,
target, source, sources, save })`) and `/flow/api/{catalogue, notation, preview, layouts, discuss,
graph, assist, svg}` (table in `src/web.rs`). The author's own files in `private/shared/flow/`:
`layouts/`, `themes/`, `modules/`, `palette.json` (block colours `[[name, fill, line]]`),
`examples/*.json` (`{description, dsl}`, few-shot for the graph task), `assist.md` (habits, appended
to the discussion and assistant prompts). `examples/serve.rs` runs the editor with a scripted model.

LLM tasks (all through the app's `llm::Llm`, the same shape as lore-plot's): `flow-discuss` (class
`create`; one step of the discussion: `discuss::STEPS` start → outline → blocks → focus → detail →
vars → io, closed reply schema per step), `sc-flow-graph` (the last step: the decisions → L1),
`flow-assist` (class `tidy`; `assist::adjust` → spec-path changes, checked by drawing them;
`assist::alternatives` → 2–4 layouts of the same content with thumbnails). Prompts in `prompts/`.

## Command line

```sh
cargo run -p lore-flow -- draw fig.flow -o fig.pptx --png fig.png [--theme t.toml] [--modules DIR] [--template t.json] [--notation n.json]
cargo run -p lore-flow -- graph desc.md -o fig.flow [--image ref.png] [--engine claude:sonnet]
cargo run -p lore-flow -- check fig.flow --notation n.json
cargo run -p lore-flow -- sheet -o modules.pptx --png modules.png [--modules DIR]
cargo run -p lore-flow -- export-modules DIR
```

The PNG is the SVG preview through headless Chrome (arrowheads about 3 × the line width, as PowerPoint
draws them). **Not tested on real hardware**: opening the
exported files in PowerPoint itself (checked here: round trip through lore-canvas, the XML, macOS
Quick Look for geometry).
