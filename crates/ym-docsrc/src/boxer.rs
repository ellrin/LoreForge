//! Semantic crop hook (design §7.4-3): given an architecture figure and its
//! caption, return labelled sub-module boxes ("CNN encoder", "knowledge base
//! M", "decoder"). The real implementation is a ym-llm vision request with a
//! JSON schema under task `sc-crop-modules`; [`MockBoxer`] is for tests.
//! Boxes are in pixels of the image passed in; [`crate::crop::add_module_regions`]
//! turns them into region assets on the page.

use crate::crop::Renderer;
use crate::error::{Error, Result};
use crate::model::AssetCatalog;
use async_trait::async_trait;
use image::GenericImageView;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::{Arc, Mutex};
use ym_llm::{Image, LlmProvider, LlmRequest, Route};

/// ym-llm task id (tasks.toml) for module boxes.
pub const TASK_ID: &str = "sc-crop-modules";

/// One labelled sub-module of a figure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleBox {
    /// Name as the figure / caption calls it, e.g. "knowledge base M".
    pub label: String,
    /// Kind of block: `model`, `data`, `loss`, `output`, `group` … (free text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// `[x0, y0, x1, y1]` in pixels of the image, origin top-left.
    pub bbox: [f64; 4],
}

/// A figure image (PNG bytes and size).
#[derive(Debug, Clone)]
pub struct FigureImage {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl FigureImage {
    pub fn from_png_bytes(png: Vec<u8>) -> Result<FigureImage> {
        let img = image::load_from_memory(&png)?;
        let (width, height) = img.dimensions();
        Ok(FigureImage { png, width, height })
    }
    pub fn from_file(path: &Path) -> Result<FigureImage> {
        FigureImage::from_png_bytes(std::fs::read(path)?)
    }
}

#[async_trait]
pub trait ModuleBoxer: Send + Sync {
    async fn module_boxes(&self, img: &FigureImage, caption: &str) -> Result<Vec<ModuleBox>>;
}

/// Fixed answer; records the captions it was asked about.
#[derive(Debug, Default)]
pub struct MockBoxer {
    pub boxes: Vec<ModuleBox>,
    pub seen: Mutex<Vec<String>>,
}

impl MockBoxer {
    pub fn new(boxes: Vec<ModuleBox>) -> MockBoxer {
        MockBoxer {
            boxes,
            seen: Mutex::new(vec![]),
        }
    }
}

#[async_trait]
impl ModuleBoxer for MockBoxer {
    async fn module_boxes(&self, _img: &FigureImage, caption: &str) -> Result<Vec<ModuleBox>> {
        self.seen.lock().unwrap().push(caption.to_string());
        Ok(self.boxes.clone())
    }
}

const SYSTEM: &str = "You locate the building blocks of a scientific architecture/pipeline figure. \
Return every distinct module (networks, encoders/decoders, memory or knowledge bases, losses, inputs, outputs, \
and dashed groups that the caption names) with a tight bounding box in pixel coordinates of the given image \
(origin top-left, x to the right, y down). Use the names printed in the figure or caption. \
Do not invent modules that are not drawn. Reply with JSON only.";

/// The JSON schema of the reply.
pub fn reply_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "modules": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "label": {"type": "string"},
                        "role": {"type": "string"},
                        "box": {"type": "array", "items": {"type": "number"}}
                    },
                    "required": ["label", "box"]
                }
            }
        },
        "required": ["modules"]
    })
}

/// Vision-model boxer through ym-llm.
pub struct LlmModuleBoxer {
    pub provider: Arc<dyn LlmProvider>,
    pub route: Option<Route>,
    /// Longest image edge sent to the model (larger figures are downscaled;
    /// boxes are scaled back).
    pub max_edge: u32,
}

impl LlmModuleBoxer {
    pub fn new(provider: Arc<dyn LlmProvider>) -> LlmModuleBoxer {
        LlmModuleBoxer {
            provider,
            route: None,
            max_edge: 1568,
        }
    }

    /// From `router.provider_for(TASK_ID)`; the task's effort/max_tokens apply.
    pub fn from_route(route: Route) -> LlmModuleBoxer {
        LlmModuleBoxer {
            provider: route.provider.clone(),
            route: Some(route),
            max_edge: 1568,
        }
    }

    /// The request, and the factor from sent-image pixels back to original pixels.
    pub fn request(
        &self,
        img: &FigureImage,
        caption: &str,
    ) -> Result<(LlmRequest, f64, (u32, u32))> {
        let (mut png, mut w, mut h) = (img.png.clone(), img.width, img.height);
        let mut back = 1.0;
        let long = w.max(h);
        if long > self.max_edge {
            let f = self.max_edge as f64 / long as f64;
            let (nw, nh) = (
                ((w as f64) * f).round().max(1.0) as u32,
                ((h as f64) * f).round().max(1.0) as u32,
            );
            let small = image::load_from_memory(&img.png)?.resize_exact(
                nw,
                nh,
                image::imageops::FilterType::Triangle,
            );
            let mut buf = std::io::Cursor::new(vec![]);
            small.write_to(&mut buf, image::ImageFormat::Png)?;
            png = buf.into_inner();
            back = long as f64 / self.max_edge as f64;
            (w, h) = (nw, nh);
        }
        let user = format!(
            "The image is {w}×{h} pixels.\nCaption: {}\n\nList the modules as {{\"modules\": [{{\"label\": …, \"role\": …, \"box\": [x0, y0, x1, y1]}}]}}.",
            caption.trim()
        );
        let mut req = LlmRequest::prompt(SYSTEM, user)
            .json(reply_schema())
            .max_tokens(4000)
            .image(Image::from_bytes("image/png", &png));
        if let Some(r) = &self.route {
            req = r.apply(req);
        }
        Ok((req, back, (w, h)))
    }
}

/// Check and convert a reply: 4 numbers per box, fractions (all ≤ 1) scaled
/// to pixels, clamped to the image, degenerate boxes dropped.
pub fn parse_reply(
    v: &Value,
    sent: (u32, u32),
    back: f64,
    original: (u32, u32),
) -> Result<Vec<ModuleBox>> {
    let items = v["modules"]
        .as_array()
        .ok_or_else(|| Error::BadReply("沒有 modules".into()))?;
    let mut out = vec![];
    for it in items {
        let label = it["label"].as_str().unwrap_or("").trim().to_string();
        let nums: Vec<f64> = it["box"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_f64).collect())
            .unwrap_or_default();
        if label.is_empty() || nums.len() != 4 {
            continue;
        }
        let fractions = nums.iter().all(|n| (0.0..=1.0).contains(n));
        let (sx, sy) = if fractions {
            (sent.0 as f64, sent.1 as f64)
        } else {
            (1.0, 1.0)
        };
        let (ow, oh) = (original.0 as f64, original.1 as f64);
        let x0 = (nums[0].min(nums[2]) * sx * back).clamp(0.0, ow);
        let x1 = (nums[0].max(nums[2]) * sx * back).clamp(0.0, ow);
        let y0 = (nums[1].min(nums[3]) * sy * back).clamp(0.0, oh);
        let y1 = (nums[1].max(nums[3]) * sy * back).clamp(0.0, oh);
        if x1 - x0 < 2.0 || y1 - y0 < 2.0 {
            continue;
        }
        let role = it["role"]
            .as_str()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_string);
        out.push(ModuleBox {
            label,
            role,
            bbox: [x0.round(), y0.round(), x1.round(), y1.round()],
        });
    }
    if out.is_empty() && !items.is_empty() {
        return Err(Error::BadReply("所有框都無效".into()));
    }
    Ok(out)
}

/// Render an asset's crop, ask `boxer` for its modules, and add them to the
/// catalog as `Region` assets (`<id>.m1`, `<id>.m2` …, label = module name).
pub async fn box_modules(
    cat: &mut AssetCatalog,
    id: &str,
    renderer: &Renderer,
    boxer: &dyn ModuleBoxer,
    dpi: u32,
) -> Result<Vec<String>> {
    let a = cat.require(id)?.clone();
    let png = renderer.render_asset(&a, dpi)?;
    let img = FigureImage::from_file(&png)?;
    let boxes = boxer
        .module_boxes(&img, a.caption.as_deref().unwrap_or(""))
        .await?;
    crate::crop::add_module_regions(cat, id, &boxes, img.width, img.height)
}

#[async_trait]
impl ModuleBoxer for LlmModuleBoxer {
    async fn module_boxes(&self, img: &FigureImage, caption: &str) -> Result<Vec<ModuleBox>> {
        let (req, back, sent) = self.request(img, caption)?;
        let v = ym_llm::complete_json(self.provider.as_ref(), &req).await?;
        parse_reply(&v, sent, back, (img.width, img.height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ym_llm::MockProvider;

    fn png(w: u32, h: u32) -> FigureImage {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([255, 255, 255, 255]));
        let mut buf = std::io::Cursor::new(vec![]);
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        FigureImage::from_png_bytes(buf.into_inner()).unwrap()
    }

    #[tokio::test]
    async fn llm_boxer_sends_image_and_schema_and_parses() {
        let reply = r#"{"modules":[{"label":"CNN encoder","role":"model","box":[10,20,110,80]},{"label":"knowledge base M","box":[0.5,0.5,0.9,0.9]},{"label":"bad","box":[1,2,3]}]}"#;
        let mock = Arc::new(MockProvider::new(vec![reply.into()]));
        let b = LlmModuleBoxer::new(mock.clone());
        let boxes = b
            .module_boxes(&png(400, 200), "Fig. 1. The architecture.")
            .await
            .unwrap();
        assert_eq!(boxes.len(), 2);
        assert_eq!(
            boxes[0],
            ModuleBox {
                label: "CNN encoder".into(),
                role: Some("model".into()),
                bbox: [10.0, 20.0, 110.0, 80.0]
            }
        );
        assert_eq!(boxes[1].bbox, [200.0, 100.0, 360.0, 180.0]);
        let seen = mock.seen.lock().unwrap();
        assert!(seen[0].has_images());
        assert!(seen[0].json_schema.is_some());
        assert!(seen[0].messages[0].text().contains("400×200"));
    }

    #[tokio::test]
    async fn large_images_are_downscaled_and_boxes_scaled_back() {
        let mock = Arc::new(MockProvider::new(vec![
            r#"{"modules":[{"label":"decoder","box":[0,0,784,392]}]}"#.into(),
        ]));
        let b = LlmModuleBoxer::new(mock.clone());
        let img = png(3136, 1568);
        let (req, back, sent) = b.request(&img, "x").unwrap();
        assert_eq!(sent, (1568, 784));
        assert_eq!(back, 2.0);
        assert!(req.messages[0].text().contains("1568×784"));
        let boxes = b.module_boxes(&img, "x").await.unwrap();
        assert_eq!(boxes[0].bbox, [0.0, 0.0, 1568.0, 784.0]);
    }

    #[tokio::test]
    async fn mock_boxer() {
        let m = MockBoxer::new(vec![ModuleBox {
            label: "decoder".into(),
            role: None,
            bbox: [0.0, 0.0, 5.0, 5.0],
        }]);
        let got = m.module_boxes(&png(10, 10), "cap").await.unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(m.seen.lock().unwrap()[0], "cap");
    }

    #[test]
    fn all_invalid_is_an_error() {
        let v = json!({"modules":[{"label":"x","box":[5,5,5,5]}]});
        assert!(parse_reply(&v, (10, 10), 1.0, (10, 10)).is_err());
        assert_eq!(
            parse_reply(&json!({"modules":[]}), (10, 10), 1.0, (10, 10)).unwrap(),
            vec![]
        );
    }
}
