# lore-voice: the Qwen3-TTS worker (mlx-audio). Written next to its Python environment by
# `qwen::Install` and run as `<venv>/bin/python -I lore_tts.py`; JSON lines on stdin/stdout:
#
#   {"req":1,"cmd":"synth","model":"<dir>","text":"…","lang":"chinese","ref_audio":"<wav>","ref_text":"…","out":"<wav>"}
#   {"req":2,"cmd":"design","model":"<dir>","text":"…","lang":"chinese","instruct":"…","out":"<wav>"}
#   {"req":3,"cmd":"unload"}
# answers {"req":n,"ok":true,"seconds":<audio length>,"took":<wall seconds>} or {"req":n,"ok":false,"error":"…"}.
#
# One model in memory at a time (a different `model` unloads the one before: speaking after
# designing frees the design model). Exits when stdin closes.
import json
import sys
import time
import wave
from pathlib import Path

out = sys.stdout
sys.stdout = sys.stderr  # library prints never reach the protocol

state = {"path": None, "model": None}


def answer(v):
    out.write(json.dumps(v, ensure_ascii=False) + "\n")
    out.flush()


def model_at(path):
    if state["path"] != path:
        unload()
        from mlx_audio.tts.utils import load_model

        state["model"] = load_model(Path(path))
        state["path"] = path
    return state["model"]


def unload():
    if state["model"] is not None:
        state["model"] = None
        state["path"] = None
        import gc

        gc.collect()
        try:
            import mlx.core as mx

            mx.clear_cache()
        except Exception:
            pass


def write_wav(path, chunks, rate):
    import numpy as np

    audio = np.concatenate([np.asarray(c, dtype=np.float32).reshape(-1) for c in chunks]) if chunks else np.zeros(0, np.float32)
    pcm = (np.clip(audio, -1.0, 1.0) * 32767).astype("<i2")
    tmp = path + ".part"
    with wave.open(tmp, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(pcm.tobytes())
    Path(tmp).replace(path)
    return len(pcm) / float(rate)


def run(c):
    start = time.time()
    m = model_at(c["model"])
    if c.get("seed") is not None:
        import mlx.core as mx

        mx.random.seed(int(c["seed"]))
    kw ={"text": c["text"], "lang_code": c.get("lang") or "auto", "verbose": False}
    if c["cmd"] == "design":
        kw["instruct"] = c["instruct"]
    else:
        kw["ref_audio"] = c["ref_audio"]
        kw["ref_text"] = c["ref_text"]
    chunks = [r.audio for r in m.generate(**kw)]
    seconds = write_wav(c["out"], chunks, int(getattr(m, "sample_rate", 24000)))
    return {"seconds": seconds, "took": time.time() - start}


answer({"ready": True})
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        c = json.loads(line)
    except ValueError:
        continue
    req = c.get("req")
    try:
        if c.get("cmd") == "unload":
            unload()
            answer({"req": req, "ok": True})
        elif c.get("cmd") in ("synth", "design"):
            answer({"req": req, "ok": True, **run(c)})
        else:
            answer({"req": req, "ok": False, "error": "unknown command"})
    except Exception as e:  # one failed request does not end the worker
        answer({"req": req, "ok": False, "error": f"{type(e).__name__}: {e}"})
