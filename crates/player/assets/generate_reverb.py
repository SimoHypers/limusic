"""Generate original diffuse stereo reverb presets (GPL-3.0-or-later).

No samples or third-party recordings. Fixed seed makes the bundled asset reproducible.
These impulse responses synthesize six room and plate reverb characters.
"""
import math
from pathlib import Path
import random
import struct

RATE = 48000
# Name, nominal RT60 (seconds), first reflection (seconds), initial HF damping.
PRESETS = [
    ("small-room", 0.4, 0.008, 0.55),
    ("medium-room", 0.7, 0.014, 0.6),
    ("large-room", 1.1, 0.022, 0.65),
    ("medium-hall", 1.8, 0.03, 0.7),
    ("large-hall", 3.2, 0.045, 0.75),
    ("plate", 1.8, 0.006, 0.9),
]
for preset, (name, decay, predelay, damping) in enumerate(PRESETS):
    frames = int(RATE * (predelay + decay * 1.5))
    channels = []
    for channel in range(2):
        rng = random.Random(742 + preset * 2 + channel)
        samples = [0.0] * frames
        low = 0.0
        for i in range(int(RATE * predelay), frames):
            time = i / RATE - predelay
            alpha = damping * math.exp(-time / decay)
            low += alpha * (rng.uniform(-1, 1) - low)
            samples[i] = low * math.exp(-math.log(1000) * time / decay)
        # Plate has dense, bright reflections rather than discrete room-wall bounces.
        if name != "plate":
            for multiple, gain in [(1, 0.7), (1.6, -0.5), (2.4, 0.35), (3.7, -0.25)]:
                samples[int(RATE * (predelay * multiple + channel * 0.001))] += gain
        energy = math.sqrt(sum(x * x for x in samples))
        channels.append([x / energy for x in samples])

    pcm = b"".join(struct.pack("<hh", *(round(c[i] * 32767) for c in channels))
                   for i in range(frames))
    # Quantization leaves silent padding; keep every nonzero stereo frame.
    end = len(pcm)
    while end >= 4 and pcm[end - 4:end] == b"\0" * 4:
        end -= 4
    assert end > 0 and not any(pcm[end:])
    # WAVE_FORMAT_EXTENSIBLE declares FL/FR explicitly; plain PCM left the layout unspecified.
    fmt = struct.pack("<HHIIHHHHI16s", 0xfffe, 2, RATE, RATE * 4, 4, 16,
                      22, 16, 3, bytes.fromhex("0100000000001000800000aa00389b71"))
    header = (b"RIFF" + struct.pack("<I", 4 + 8 + len(fmt) + 8 + end) + b"WAVEfmt "
              + struct.pack("<I", len(fmt)) + fmt + b"data" + struct.pack("<I", end))
    Path(__file__).with_name(f"{name}.wav").write_bytes(header + pcm[:end])
