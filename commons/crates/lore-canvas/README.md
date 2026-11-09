# lore-canvas

畫布核心（設計稿 §2.2、§7.3、§7.8）：與 App 無關的投影片場景模型、樣式規則、可復原的操作、
幾何、漸進揭露產生器，以及 `lore-ooxml` ↔ 畫布的無損轉換。SlideComposer 用它；之後的平面設計 App 也能用。

| 模組 | 內容 |
|---|---|
| `model` | `Deck` → `Slide`（uid、版面配置、隱藏、備註、動畫、來源）→ `Object`（id、名稱、`Frame` EMU＋旋轉＋翻轉、placeholder）＋ `Kind`：`Text`/`Shape`（`SpObj`：幾何 prst/custom、`Paint`、`Stroke`、`TextBody`、equation）、`Connector`（起訖黏著、箭頭、虛線）、`Picture`（asset、裁切、透明度、外框、白色遮罩、效果）、`Group`（chOff/chExt）、`Table`、`Locked`。文字：`Para`（對齊含左右對齊、兩層、項目符號／編號、行距、段距）→ `Run`（`tag`＋`em`＋`look`；text/br/field/math，math 存 OMML＋LaTeX）。指紋 `object_ks` / `changed_fields` / `untouched` / `slide_k` |
| `style` | `StyleSystem`（包 `lore_ooxml::style::StylePack`）：`look(tag, em)`、`match_look`（匯入時把格式對回標籤）、`palette_hex`、`color("numbered-circle[2]")`、`check_change`（標籤必須已定義、新顏色必須在色票；匯入的原格式不受限直到被改動）；保留強調 `ghost` |
| `ops` | `Op`：`set_obj`（JSON pointer）、`insert_obj`、`delete_obj`、`move_obj`、`set_slide`、`insert_slide`、`delete_slide`、`move_slide`、巨集 `group`/`ungroup`；`Editor::apply`（驗證、原子、黏著連接線自動重排）、`undo`、`redo` |
| `geom` | `bounds`、群組座標、`sites`（連接點：矩形 4 個、橢圓 8 個，依 PowerPoint 順序）、`connector_points`、`reroute`、`snap_site`、`align`、`distribute` |
| `reveal` | `RevealSpec::Ghost { steps, future }`（每步一頁，其他步驟依變淡規則：圖片 alpha、文字灰、線條／填色 alpha、方程式灰）、`Agenda { obj, paras, em }`（每個第一層、有內容的段落一頁，略過空白行與第二層項目；`agenda_items` 列出這些段落；當前項加粗） |
| `components` | 風格包元件 → 物件：編號圓圈（色序）、模型梯形、標籤卡（群組）、重點框、語意遮罩；`arrow(style)` |
| `pptx_in` | `import(deck, style, id)`：每個物件保留原 XML＋逐欄位指紋；placeholder 位置與文字樣式依版面配置／母片繼承解析；`backdrops`（版面配置＋母片的非 placeholder 圖形，顯示用）；`cust_path`（custGeom → SVG） |
| `pptx_out` | `export` / `export_parts` / `export_bytes`：未改動的投影片保留原 part 位元組；改動的投影片逐物件重建（未改動物件＝原 XML，改動物件＝原 XML 上逐欄位修補，新物件從頭建）；新增／複製／刪除／重新排序投影片、備註頁建立、圖片關聯（同內容共用）、清掉沒人用的媒體 |

## 無損原則

- 匯入時每個物件記下原 XML 與每個欄位（frame、name、fill、line、text、crop…）的指紋；
  段落與 run 記下來源索引與格式指紋。匯出時只修補改過的欄位，其他屬性與未知子元素原封保留。
- 投影片內容指紋沒變 → 該投影片 part 與備註 part 原位元組輸出。
- 實測（4 份真實簡報，`examples/deck_roundtrip.rs`）：匯入→ JSON → 匯出，**所有 part 逐位元組相同**。

```sh
cargo test -p lore-canvas
LORE_DECKS="a.pptx:b.pptx" LORE_STYLE_DIR=styles/x cargo run --release -p lore-canvas --example deck_roundtrip
```

## 已知限制

- 群組內的子物件只能改內容（文字），不能單獨移動。
- 連接點用矩形 4 點／橢圓 8 點近似（其他 preset 的實際連接點未逐一實作）；匯入的連接線只在它黏著的物件移動時才重排。
- 方程式改動後，`mc:AlternateContent` 的 Fallback 圖片不更新。
- 表格儲存格框線只在新儲存格寫入；既有儲存格只修補底色。
