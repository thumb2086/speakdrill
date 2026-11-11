#!/usr/bin/env python3
"""
kokoro_tts.py — 給 myet-practice 用的 TTS 轉接器

用法：
    python kokoro_tts.py <文字檔> <輸出.wav>

環境變數：
    KOKORO_MODEL   模型路徑（預設找常見位置）
    KOKORO_VOICES  聲音檔路徑
    KOKORO_VOICE   音色 ID（預設 af_sol）
    KOKORO_SPEED   語速（預設 1.0）

安裝：
    pip install kokoro-onnx soundfile
    # 模型：https://github.com/thewh1teagle/kokoro-onnx/releases
"""
import os
import sys
import wave

def find_model():
    env = os.environ.get("KOKORO_MODEL")
    if env and os.path.isfile(env):
        return env
    cands = [
        os.path.join(os.path.dirname(__file__), "..", "models", "kokoro-v1.0.onnx"),
        os.path.expanduser("~/.cache/kokoro/kokoro-v1.0.onnx"),
        r"C:\Users\CPXru\Desktop\thumb\Projects\ai_vtuber\models\kokoro\kokoro-v1.1-zh.onnx",
    ]
    for c in cands:
        if os.path.isfile(c):
            return os.path.abspath(c)
    return None

def find_voices():
    env = os.environ.get("KOKORO_VOICES")
    if env and os.path.isfile(env):
        return env
    cands = [
        os.path.join(os.path.dirname(__file__), "..", "models", "voices-v1.0.bin"),
        os.path.expanduser("~/.cache/kokoro/voices-v1.0.bin"),
        r"C:\Users\CPXru\Desktop\thumb\Projects\ai_vtuber\models\kokoro\voices-v1.1-zh.bin",
    ]
    for c in cands:
        if os.path.isfile(c):
            return os.path.abspath(c)
    return None

def main():
    if len(sys.argv) < 3:
        print("usage: kokoro_tts.py <textfile> <out.wav>", file=sys.stderr)
        return 2

    textfile, out = sys.argv[1], sys.argv[2]
    with open(textfile, encoding="utf-8") as f:
        text = f.read().strip()
    if not text:
        print("empty text", file=sys.stderr)
        return 2

    # 清掉會讓 TTS 拉長的寫法（MyET 錄音視窗只有 ~2.7 秒）
    import re
    text = text.replace("...", ", ").replace("…", ", ")
    text = re.sub(r"\s+", " ", text).strip()

    try:
        import numpy as np
        import soundfile as sf
        from kokoro_onnx import Kokoro
    except ImportError as e:
        print(f"缺少套件：{e}\n請執行： pip install kokoro-onnx soundfile", file=sys.stderr)
        return 3

    model, voices = find_model(), find_voices()
    if not model or not voices:
        print("找不到 Kokoro 模型檔；請設定 KOKORO_MODEL / KOKORO_VOICES", file=sys.stderr)
        return 3

    voice = os.environ.get("KOKORO_VOICE", "af_sol")
    speed = float(os.environ.get("KOKORO_SPEED", "1.0"))

    k = Kokoro(model, voices)
    samples, sr = k.create(text, voice=voice, speed=speed, lang="en-us")
    a = np.asarray(samples, dtype="float32")
    peak = float(np.max(np.abs(a))) if a.size else 0.0
    if peak > 0:
        a = a * (0.95 / peak)
    sf.write(out, a, sr)
    return 0

if __name__ == "__main__":
    sys.exit(main())
