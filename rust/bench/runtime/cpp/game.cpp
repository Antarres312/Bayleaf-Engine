// The game: the benchmark's three scripts and its scene, written against runtime.hpp the way game
// code would be (bench/runtime/rust/src/game.rs mirrors this file). Each script exists twice with
// the same body: as a plain struct (typed) and as a virtual class.

#include <algorithm>
#include <cmath>
#include <string>

#include "game.hpp"

namespace game {
using namespace mini;

namespace {

// Rotation Y += speed x dt.
struct Spin {
    float speed = 90.0f;
    void start(Self&) {}
    void update(Self& self, const Frame& f) {
        float& y = self.transform_mut().rotation[1];
        y = std::fmod(y + speed * f.dt, 360.0f);
    }
};

// Position Y = base + height x sin(time x speed).
struct Bob {
    float speed = 1.0f, height = 0.25f, base = 0.5f;
    void start(Self&) {}
    void update(Self& self, const Frame& f) {
        // Positions are kept in double, set from single precision as Bayleaf's scripts set them.
        self.transform_mut().position[1] = base + height * std::sin(static_cast<float>(f.time) * speed);
    }
};

// Position X and Z ease towards the object named Target, found at start.
struct Follow {
    float rate = 0.5f;
    Entity target{};
    void start(Self& self) { target = self.world.find("Target"); }
    void update(Self& self, const Frame& f) {
        if (!self.world.alive(target)) return;
        const float k = std::min(rate * f.dt, 1.0f);
        const double* goal = self.world.transform(target).position;
        double* p = self.transform_mut().position;
        for (int axis : {0, 2}) {
            const float here = static_cast<float>(p[axis]);
            p[axis] = here + (static_cast<float>(goal[axis]) - here) * k;
        }
    }
};

// The same scripts as heap objects behind a virtual update.
template <class T>
struct Virtual final : Script {
    T inner;
    explicit Virtual(T t) : inner(t) {}
    void start(Self& self) override { inner.start(self); }
    void update(Self& self, const Frame& f) override { inner.update(self, f); }
};

// Per-object property values: the same formulas as bench/src/main.rs.
float spin_speed(size_t i) { return 30.0f + static_cast<float>(i % 7) * 10.0f; }
float bob_speed(size_t i) { return 1.0f + static_cast<float>(i % 5) * 0.25f; }
float bob_height(size_t i) { return 0.25f + static_cast<float>(i % 3) * 0.1f; }
float follow_rate(size_t i) { return 0.05f + static_cast<float>(i % 4) * 0.05f; }

template <class T>
void attach(Runtime& runtime, Entity e, T script, bool typed) {
    if (typed)
        runtime.add(e, script);
    else
        runtime.add_virtual(e, std::make_unique<Virtual<T>>(script));
}

}  // namespace

// The scene of bench/src/main.rs: the target first, then a grid of objects, each with a mesh, a
// collider, a tag, a light on every tenth, and the three scripts.
void build(Runtime& runtime, size_t n, bool typed) {
    World& world = runtime.world;
    world.create(Id{0, 1}, "Target");
    const size_t side = std::max<size_t>(1, static_cast<size_t>(std::ceil(std::sqrt(static_cast<double>(n)))));
    for (size_t i = 0; i < n; ++i) {
        const Entity e = world.create(Id{0x9e3779b97f4a7c15ull * (i + 1), i + 2}, "Object " + std::to_string(i));
        Transform& t = world.transform_mut(e);
        t.position[0] = static_cast<double>(i % side) * 2.0 - static_cast<double>(side);
        t.position[2] = static_cast<double>(i / side) * 2.0 - static_cast<double>(side);
        world.add(e, Mesh{});
        world.add(e, Collider{});
        world.tag(e, "bench");
        if (i % 10 == 0) world.add(e, PointLight{});
        attach(runtime, e, Spin{spin_speed(i)}, typed);
        attach(runtime, e, Bob{bob_speed(i), bob_height(i), 0.5f}, typed);
        attach(runtime, e, Follow{follow_rate(i)}, typed);
    }
}

}  // namespace game
