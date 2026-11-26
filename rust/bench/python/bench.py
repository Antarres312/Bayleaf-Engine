"""Python counterpart of bench/cpp/bench.cpp (bench/README.md): the same scene and per-object work,
in plain Python with no libraries. Entities own their components; scripts are objects with an
update method. Timed for the script step alone and with every object's world matrix. Prints one
JSON line per part.

Run: python bench.py <objects>
"""

import json
import math
import platform
import statistics
import sys
import time as clock

DT = 1.0 / 60.0
WARMUP, MIN_FRAMES, MAX_FRAMES, MIN_SECONDS = 30, 100, 20000, 3.0


class Transform:
    __slots__ = ("position", "rotation", "scale")

    def __init__(self):
        self.position = [0.0, 0.0, 0.0]
        self.rotation = [0.0, 0.0, 0.0]
        self.scale = [1.0, 1.0, 1.0]


class Entity:
    __slots__ = ("name", "transform", "mesh", "collider", "light", "tags", "scripts", "world")

    def __init__(self, name):
        self.name = name
        self.transform = Transform()
        self.mesh = None
        self.collider = None
        self.light = None
        self.tags = []
        self.scripts = []
        self.world = [0.0] * 16


class Spin:
    __slots__ = ("speed",)

    def __init__(self, speed):
        self.speed = speed

    def update(self, entity, dt, time):
        r = entity.transform.rotation
        r[1] = math.fmod(r[1] + self.speed * dt, 360.0)


class Bob:
    __slots__ = ("speed", "height", "base")

    def __init__(self, speed, height, base):
        self.speed, self.height, self.base = speed, height, base

    def update(self, entity, dt, time):
        entity.transform.position[1] = self.base + self.height * math.sin(time * self.speed)


class Follow:
    __slots__ = ("target", "rate")

    def __init__(self, target, rate):
        self.target, self.rate = target, rate

    def update(self, entity, dt, time):
        k = min(self.rate * dt, 1.0)
        p, g = entity.transform.position, self.target.transform.position
        p[0] += (g[0] - p[0]) * k
        p[2] += (g[2] - p[2]) * k


def world_matrix(t, m):
    rad = math.pi / 180.0
    ax, ay, az = t.rotation[0] * rad, t.rotation[1] * rad, t.rotation[2] * rad
    cx, sx, cy, sy, cz, sz = math.cos(ax), math.sin(ax), math.cos(ay), math.sin(ay), math.cos(az), math.sin(az)
    s0, s1, s2 = t.scale
    m[0:4] = (cy * cz * s0, cy * sz * s0, -sy * s0, 0.0)
    m[4:8] = ((sx * sy * cz - cx * sz) * s1, (sx * sy * sz + cx * cz) * s1, sx * cy * s1, 0.0)
    m[8:12] = ((cx * sy * cz + sx * sz) * s2, (cx * sy * sz - sx * cz) * s2, cx * cy * s2, 0.0)
    m[12:16] = (t.position[0], t.position[1], t.position[2], 1.0)


def build_scene(n):
    target = Entity("Target")
    entities = [target]
    side = max(1, math.ceil(math.sqrt(n)))
    for i in range(n):
        e = Entity(f"Object {i}")
        e.transform.position[0] = (i % side) * 2.0 - side
        e.transform.position[2] = (i // side) * 2.0 - side
        e.mesh = {"shape": "cube", "color": [0.8, 0.8, 0.8, 1.0]}
        e.collider = {"shape": "box", "size": [1.0, 1.0, 1.0]}
        e.tags.append("bench")
        if i % 10 == 0:
            e.light = {"color": [1.0, 1.0, 1.0], "intensity": 1.0, "range": 10.0}
        # The same formulas as bench/src/main.rs.
        e.scripts.append(Spin(30.0 + (i % 7) * 10.0))
        e.scripts.append(Bob(1.0 + (i % 5) * 0.25, 0.25 + (i % 3) * 0.1, 0.5))
        e.scripts.append(Follow(target, 0.05 + (i % 4) * 0.05))
        entities.append(e)
    return entities


def measure(step):
    for _ in range(WARMUP):
        step()
    ms = []
    started = clock.perf_counter()
    while len(ms) < MAX_FRAMES and (len(ms) < MIN_FRAMES or clock.perf_counter() - started < MIN_SECONDS):
        t = clock.perf_counter()
        step()
        ms.append((clock.perf_counter() - t) * 1000.0)
    ms.sort()
    return ms


def run(n, with_matrices):
    entities = build_scene(n)
    state = {"time": 0.0}

    def step():
        state["time"] += DT
        now = state["time"]
        for e in entities:
            for s in e.scripts:
                s.update(e, DT, now)
        if with_matrices:
            for e in entities:
                world_matrix(e.transform, e.world)

    ms = measure(step)
    at = lambda q: ms[round((len(ms) - 1) * q)]
    check = sum(e.transform.position[0] + e.transform.position[1] + e.transform.rotation[1] + e.world[0] for e in entities)
    print(json.dumps({
        "mode": "python", "runtime": f"{platform.python_implementation()} {platform.python_version()}", "scripts": "python",
        "part": "step" if with_matrices else "scripts", "objects": n, "checksum": check,
        "stats": {"frames": len(ms), "meanMs": statistics.fmean(ms), "medianMs": at(0.5), "p5Ms": at(0.05), "p95Ms": at(0.95), "stdevMs": statistics.pstdev(ms)},
    }), flush=True)


if __name__ == "__main__":
    objects = int(sys.argv[1]) if len(sys.argv) > 1 else 5000
    run(objects, False)
    run(objects, True)
