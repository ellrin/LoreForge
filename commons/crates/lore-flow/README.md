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

## Filling a page (the base draft)

`pack::pack(graph, Format, Measure, arrangement, Air)` draws a figure's blocks as large as a page of
a fixed shape allows, without lines: the areas in the rows the arrangement names (`a / b c`), in
each area its blocks in the order written, in rows that break wherever that keeps things largest
(a region is one item with rows of its own). Three searches, each on one number: the largest text
(the smallest page that holds everything at its natural size), then every area's pictures enlarged
together, then the areas that still have room. `Air` states the air in text sizes (`gap` between
blocks, `pad` inside a frame, `between` areas) and `text_max_pt`, a limit on the text that hands the
rest of the page to the pictures. `Packed` carries the layout, the graph with each block's `grow=`,
the text size on the page, the page's cover and each area's rows.
`cargo run -p lore-flow --example flow_pack -- <spec> <width_pt> <w:h> <out.svg> <flow dir> [arrangement] [gap] [pad] [largest text pt]`.
Alignment, all of it rules: the pictures of a row stand on one middle line (`Body.axis`, the
middle of a picture's large pieces, so a loop arrow over one block and under the next does not
shift them); the words under them start on one line under the row's tallest picture (`label_drop=`
on the lower ones; a frame in the row does not push them down); the rows of a frame share columns
set by its longest rows, so blocks lie under each other (a shorter row joins when its items are at
most 1.3 columns wide, centred on columns when that lands on them, else it is centred by itself);
the rows are centred in their frame. A block that is words only on lines none of which carries the
flow is a note (`pack::is_note`: a weight fed in from the side, the name of a comparison): it takes
no room, does not move the objects, and is written in free air inside its own frame, beside or
under the block it is tied to; when there is none its frame gets a band above and below its rows
(`Packed.notes`, `Packed.crowded`). A label of one line wider than its picture and ten text sizes
is written on lines of about one length (`modules::label_lines`); a saved drawing's arrow made of
a line and a separate small triangle becomes one line with its own arrowhead.
Paths: when every object of an area states its column and row (`col=`, `lane=`: where it stands
along the flow and which of the parallel paths it is on), the area stands on that grid: a row is a
path, kept whole, the steps of different paths stand under each other, a region frames the cells
of its blocks, so flow lines are straight. The fill is the same computation on the grid's one
shape. An area keeps its grid as long as the page's text keeps `Air.paths` (0.75) of what rows
broken freely give; past that the area that gives most back goes free (`AreaPack.grid`). Lines
enter on a block's middle line (`Body.cy` = `Body.axis`, the `port` child evenly about it).
`LINES=1` on the example draws the router's lines and counts bends, lines over pictures, lines off
the page and where each flow line's ends lie; `PATHS=<share>` sets `Air.paths`. Tests in
`tests/pack.rs` and `modules::align_tests`. Not yet: room for lines in the air, lines beneath
blocks, the lines of notes.

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

A style can be layers: a structure (weights, dashes), a palette, a corner scheme, each its own
file (`part = "palette"` / `"corners"` marks a layer). `Catalogue::theme("lore+macaron+graded")`
lays the named files over one another in that order (`Theme::from_layers`); an unknown layer
gives the built-in theme.

`[corners]` is a corner scheme: one radius per level in points (`panel`, `region`, `inner` = a
region in a region, `data` = blocks of role input/data/output/gt, `step` = other rectangles,
`loss`; `0` = square, negative = pill). With one, the theme decides the corners of panel
frames, region frames, boxes and losses, over the figure's own `round` words; a block's own
`radius=` still wins. Without one nothing changes. A saved library block follows the style when drawn: its rounded
rectangles take the `step` radius (a pill stays a pill), and with `[node] levels = [0.6, 1.1, 1.6]`
each of its strokes is drawn at the nearest listed weight. The saved file is not changed.

`[tints.<name>] fill / mid / line` are the palette's named colours: what `tint=blue`, `fill=blue`
or a line's `color=blue` means in this theme (a name not listed keeps the built-in pair).
`[region] line_w / inner_w / inner_dash / soft / title_tint` draws region frames: `soft` outlines
a region in its tint's `mid` and fills a region inside a region with it.

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
`preview_svg` crops a plain SVG, `svg_mm(fig, frame)` is the print-size file (with the frame, as wide
as the frame, e.g. a paper's column, so `\includegraphics[width=\linewidth]` keeps the designed size;
only the empty space above and below is cropped), `as_picture` wraps the objects as one group (`lock_aspect`, the spec
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
