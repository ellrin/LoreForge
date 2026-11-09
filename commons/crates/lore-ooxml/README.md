# lore-ooxml

PPTX 無損讀寫（設計稿 §7.2、§7.3、§7.8、§10）。自寫 zip ＋ quick-xml 薄層：每個 part 以原始位元組保留，
投影片／備註頁由型別化模型重新產生；未改動的投影片寫回時與原檔**逐位元組相同**。

| 模組 | 內容 |
|---|---|
| `package` | `Package`：zip 內每個 part 原樣保留（順序、stored/deflate）；`Relationships`、`ContentTypes`、`resolve_target`/`relative_target`；`save_new` 不覆蓋既有檔案 |
| `xml` | 小型 DOM（前綴原樣保留、命名空間宣告當屬性）、`canonical`/`canonical_diff`/`canonical_eq_bytes`（屬性排序、忽略格式化空白） |
| `model` | `SlideDoc`（sld / notes / sldLayout / sldMaster 共用）→ `Group`（spTree、巢狀 grpSp 含 chOff/chExt）→ `Shape`：`Sp`（prstGeom/custGeom、填色、外框、`TextBody`）、`Pic`（blip rel、`srcRect` 裁切、`alphaModFix`、外框、a14 去背原樣保留）、`Cxn`（stCxn/endCxn、箭頭、虛線）、`Frame`（`Table`：格線、列、儲存格、合併）、`Alt`（`mc:AlternateContent`：Choice 內型別化、`a14:m` 方程式原樣、Fallback 圖片）、`Locked`（圖表/SmartArt/OLE/contentPart…原封輸出） |
| `model`（文字） | `Paragraph`（`ParaProps`：lvl、algn、marL/indent、行距/段前段後、項目符號 `Bullet::{None,Char,AutoNum,Blip}`）、`Run::{Text,Break,Field,Math,Raw}`、`RunProps`（sz/b/i、填色、latin/ea/cs/sym） |
| `deck` | `Deck`：投影片（依 sldIdLst 順序）、版面配置、母片、佈景主題、備註、隱藏頁；`effective_xfrm`、`resolve_run`（投影片 → 版面配置 → 母片 → txStyles/defaultTextStyle → 佈景主題字型與色彩，含 lumMod/lumOff/shade/tint）；`template_package`（只留母片/版面配置/佈景主題/媒體，0 張投影片，清掉 docProps 與 sections）；`add_slide(版面配置名稱)`、`import_slide`（跨簡報複製投影片＋媒體，同內容共用、原檔名優先）、`add_image`、`set_placeholder_text` |
| `anim` | 5 種進場動畫（出現 1、飛入 2、淡入 10、擦去 22、縮放 53）→ `p:timing`（mainSeq、click/with/after、`bldP`）；`read_timing` 讀回 |
| `inventory` | 每頁物件清單、鎖定物件、原樣保留的特性（custGeom、gradFill、effectLst、a14 去背、OMML…）；`fully_editable()` |
| `shapes` | 建立新物件：`text_box`、`preset_shape`、`picture`、`connector`、`group`、`styled_run`；`absolute_rects`（解巢狀群組座標） |
| `style` | 風格包格式 `StylePack`（`style.toml`：pack、layouts、tags、palette、emphasis、components、arrows、ghost）＋ `extract_draft`（統計標籤/色票/元件草稿）＋ `StylePack::run(tag, emphasis, text)` |

## 無損原則

- 每個型別化結構保留**所有屬性**（`Attrs`，型別化存取器讀寫它），不認得的子元素放在 `Extras`（記住原位置；`extLst` 永遠最後）。
- 整個物件無法編輯 → `Shape::Locked`，原封輸出並列入 inventory。
- 只有投影片與備註頁重新產生；母片、版面配置、佈景主題、媒體、rels 等都是原位元組。

## 範例程式（路徑一律用環境變數，真實簡報不進 repo）

```sh
# 4 份簡報的往返測試：清單、鎖定物件、canonical 比對、重建（從風格範本＋匯入每頁）、視覺比對
LORE_DECKS="a.pptx:b.pptx" LORE_SCRATCH=/tmp/rt [LORE_RENDERER=none|quicklook] \
  cargo run --release -p lore-ooxml --example roundtrip
# 從簡報抽風格包草稿（template.pptx、style.draft.toml、extract-stats.json；不覆蓋既有檔案）
LORE_DECKS="a.pptx:b.pptx" LORE_STYLE_DIR=styles/x LORE_STYLE_ID=x cargo run -p lore-ooxml --example extract_style
# 用風格包產生示範簡報與預覽 PNG
LORE_STYLE_DIR=styles/x LORE_OUT=/tmp/sample.pptx LORE_PREVIEW_DIR=styles/x/preview \
  cargo run -p lore-ooxml --example style_preview
```

視覺比對的渲染器優先序：PowerPoint → Keynote（osascript 匯出 PDF，**not tested on real hardware**）→
QuickLook（macOS OfficeImport 產生 HTML，形狀 PDF 片段用 `pdftocairo` 轉 PNG，headless Chrome 列印成 PDF；
近似渲染：字型替換、`alphaModFix` 與 OMML 不畫）→ `pdftoppm -gray` → SSIM。

## 已知限制 / TODO

- 動畫：preset id 依 PowerPoint 內建表；Wipe 的 `wipe(down)` 方向、Zoom 的 subtype 16 與行為組合**未在 PowerPoint 上驗證**（程式內 TODO）。
- `custGeom`、`gradFill`、效果、3D、`a14` 影像效果、超連結：可保留、可整體移動縮放，但不逐項編輯。
- 表格儲存格屬性（`a:tcPr`）、`p:style`、項目符號字型/顏色/大小目前以原始 XML 保存。
- `import_slide` 不複製備註頁。

```sh
cargo test -p lore-ooxml
```
