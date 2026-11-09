"""Build the synthetic Office fixtures (a .docx paper and a .pptx deck).

Everything here is fictional. Run from anywhere:

    python3 make.py            # needs pandoc and Pillow

Outputs next to this script: fixture-paper.docx, fixture-slides.pptx and the
images they use (fig-*.png). The files exercise what PaperComposer and
SlideComposer must survive when opening Office documents: native equations
(OMML) with fractions, sums, matrices and aligned rows; an equation that is
only a picture (as MathType leaves it); figures with captions; a table;
a reviewer comment and tracked changes (Word); speaker notes (slides).
"""
import os, subprocess, tempfile
from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
os.chdir(HERE)

def font(size):
    for f in ["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/System/Library/Fonts/Supplemental/Arial.ttf"]:
        if os.path.exists(f):
            return ImageFont.truetype(f, size)
    return ImageFont.load_default()

def bar_chart(path):
    w, h = 900, 520
    im = Image.new("RGB", (w, h), "white"); d = ImageDraw.Draw(im)
    vals = [("All", 0.690), ("Discard", 0.695), ("Linear", 0.704), ("Fuzzy", 0.720)]
    d.text((30, 15), "Dice by frame-handling strategy (synthetic)", fill="black", font=font(26))
    base, top = 460, 80
    for i, (k, v) in enumerate(vals):
        x = 120 + i * 190
        y = base - (v - 0.6) / 0.15 * (base - top)
        d.rectangle([x, y, x + 110, base], fill=(70, 110 + i * 30, 200))
        d.text((x + 10, y - 32), f"{v:.3f}", fill="black", font=font(22))
        d.text((x + 10, base + 12), k, fill="black", font=font(22))
    d.line([90, base, w - 40, base], fill="black", width=2)
    im.save(path)

def pipeline(path):
    w, h = 1000, 300
    im = Image.new("RGB", (w, h), "white"); d = ImageDraw.Draw(im)
    boxes = ["Frame x_b", "Quality q_b", "Area a_b", "Fuzzy rules", "Weight w_b"]
    for i, t in enumerate(boxes):
        x = 20 + i * 195
        d.rounded_rectangle([x, 110, x + 165, 190], radius=14, outline=(40, 40, 40), width=3, fill=(235, 242, 255))
        d.text((x + 14, 135), t, fill="black", font=font(22))
        if i < len(boxes) - 1:
            d.line([x + 165, 150, x + 195, 150], fill="black", width=3)
    im.save(path)

def equation_picture(path):
    # What a MathType object leaves in a .docx: a picture, no math inside.
    im = Image.new("RGB", (520, 90), "white"); d = ImageDraw.Draw(im)
    d.text((14, 22), "L = (1/B) Σ w_b · ℓ(ŷ_b, y_b)", fill="black", font=font(34))
    im.save(path)

bar_chart("fig-dice.png"); pipeline("fig-pipeline.png"); equation_picture("fig-equation-picture.png")

PAPER = r"""---
title: "Down-weighting unreliable frames in synthetic endoscopy segmentation"
author: "Ellrin (fixture)"
---

# Introduction

Drug-induced sleep endoscopy (DISE) frames vary in quality. Discarding hard frames is common, but it removes
the cases that matter most. We instead give every frame $x_b$ a weight $w_b \in [0, 1]$.

# Methods

Let $\tilde{q}_b$ denote the predicted quality of frame $b$ and $a_b = \frac{|y_b|}{|I_b|}$ its airway area
ratio. The linear baseline mixes the two cues,

$$ w_b^{\mathrm{lin}} = \lambda \tilde{q}_b + (1-\lambda)\, a_b, \qquad \lambda \in [0,1]. $$

The fuzzy weight uses rule activations $\alpha_{mn}$ and consequents $\rho_{mn}$:

$$ w_b = \frac{\sum_{m,n} \alpha_{mn}\, \rho_{mn}}{\sum_{m,n} \alpha_{mn} + \epsilon}. $$

The batch loss is shown below as a picture, the way an old equation editor leaves it:

![Equation as a picture (no math inside).](fig-equation-picture.png){width=60%}

Rule prototypes form a matrix

$$ \mathbf{V} = \begin{pmatrix} v_{11} & v_{12} \\ v_{21} & v_{22} \end{pmatrix}, \qquad
\begin{aligned} \mu_{\mathrm{Low}}(q) &= 1 - \sigma(k(q - c_1)) \\ \mu_{\mathrm{High}}(q) &= \sigma(k(q - c_2)) \end{aligned} $$

![Pipeline from frame to weight.](fig-pipeline.png){width=100%}

# Results

| Strategy | Dice | IoU |
|---|---|---|
| All frames | 0.690 | 0.588 |
| Discard levels 1–2 | 0.695 | 0.591 |
| Linear ($\lambda=0.5$) | 0.704 | 0.602 |
| Fuzzy | 0.720 | 0.619 |

: Segmentation performance of frame-handling strategies (5×5-fold CV).

![Dice by strategy.](fig-dice.png){width=80%}

[The fuzzy weight gives the highest Dice.]{.comment-start id="0" author="Reviewer 2" date="2026-11-01T09:00:00Z"}
Is the gain larger than one standard deviation?[]{.comment-end id="0"}
The gain over linear weighting is [small but consistent]{.insertion author="Ellrin" date="2026-11-02T10:00:00Z"}[large]{.deletion author="Ellrin" date="2026-11-02T10:00:00Z"}.

# Conclusion

Down-weighting keeps the clinically important frames while limiting their influence.
"""

SLIDES = r"""---
title: "Fuzzy frame weights (fixture deck)"
---

# Why not discard?

- Hard frames hold the obstructed airway
- Discarding gains only 0.005 Dice

::: notes
Say that discarding looks harmless but removes the cases that matter.
:::

# The weight

$$ w_b = \frac{\sum_{m,n} \alpha_{mn}\, \rho_{mn}}{\sum_{m,n} \alpha_{mn} + \epsilon} $$

- $\tilde{q}_b$: quality, $a_b$: area ratio

::: notes
Point at alpha and rho; the denominator only normalises.
:::

# Pipeline

![Pipeline from frame to weight.](fig-pipeline.png)

# Results

![Dice by strategy.](fig-dice.png)

::: notes
The gap to linear is within one SD: say consistent, not large.
:::
"""

with tempfile.TemporaryDirectory() as t:
    for name, text, out in [("paper.md", PAPER, "fixture-paper.docx"), ("slides.md", SLIDES, "fixture-slides.pptx")]:
        src = os.path.join(t, name)
        open(src, "w").write(text)
        subprocess.run(["pandoc", src, "-o", out, "--resource-path", HERE], check=True)
print("wrote fixture-paper.docx, fixture-slides.pptx")
