import json, base64, os, re
S = os.path.dirname(os.path.abspath(__file__)); T = S + "/thumbs"
idx = json.load(open(T + "/index.json"))
def uri(f): return "data:image/svg+xml;base64," + base64.b64encode(open(T + "/" + f, "rb").read()).decode()
byid = {x["id"]: x for x in idx}
pick = ["fig101", "fig102", "unet.bars", "unet.lineart", "encoder.slabs", "encoder.resnet", "encoder.rnn", "encoder.graph", "decoder.cubes", "decoder.fc", "decoder.transformer", "autoencoder.vae", "autoencoder.cubes", "nn.lstm", "nn.fans", "nn.moe", "feature-stack.cube", "image-stack.mosaic", "matrix.default", "cuboid-seq.default", "layer-bars.default", "vector-bar.default", "box.rect", "box.ellipse", "op.plain", "op.circle", "loss.box", "loss.plain", "brace.paren", "label.default", "image.default", "box.trapezoid", "box.can", "box.hexagon"]
pick = [p for p in pick if p in byid]
for x in idx:
    if len(pick) >= 44: break
    if x["id"] not in pick: pick.append(x["id"])
ITEMS = [{"id": p, "t": byid[p]["title"], "lv": byid[p]["level"], "ln": byid[p]["line"], "src": uri(byid[p]["file"])} for p in pick]
html = open(S + "/draft.tpl.html").read().replace("/*ITEMS*/[]", json.dumps(ITEMS, ensure_ascii=False)).replace("__N__", str(len(idx)))
lv = {}
for x in idx: lv[x["level"]] = lv.get(x["level"], 0) + 1
html = html.replace("/*LV*/{}", json.dumps(lv))
open(os.environ["OUT"], "w").write(html); print(len(html), lv)
