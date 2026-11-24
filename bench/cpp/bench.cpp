// Plain C++ counterpart of the Bayleaf benchmark (bench/README.md): the same scene and the same
// per-object work, with no engine. Two layouts:
//   virtual - each entity owns its components; scripts are heap objects behind a virtual update,
//             the way a straightforward entity/component program is usually written.
//   packed  - the same data in flat arrays, one loop per script kind (a lower bound).
// Each layout is timed for the script step alone, and for the step plus every object's world matrix
// (Bayleaf's play step computes those too). Prints one JSON line per layout and part.
//
// Build (from a VS x64 developer prompt): cl /nologo /O2 /EHsc /std:c++17 /Fe:bench.exe bench.cpp
// Run: bench.exe <objects>

#include <algorithm>
#include <array>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <functional>
#include <memory>
#include <string>
#include <vector>

namespace {

constexpr float kDt = 1.0f / 60.0f;
constexpr int kWarmup = 30;
constexpr size_t kMinFrames = 100;
constexpr double kMinSeconds = 3.0;
constexpr size_t kMaxFrames = 20000;

struct Transform {
    double position[3] = {0, 0, 0};
    float rotation[3] = {0, 0, 0};
    float scale[3] = {1, 1, 1};
};
struct Mesh {
    int shape = 0;
    float color[4] = {0.8f, 0.8f, 0.8f, 1.0f};
    float metallic = 0.0f, roughness = 0.5f;
};
struct Collider {
    int shape = 0;
    float size[3] = {1, 1, 1};
    float friction = 0.5f, restitution = 0.0f;
    bool trigger = false;
};
struct PointLight {
    float color[3] = {1, 1, 1};
    float intensity = 1.0f, range = 10.0f;
};

struct Entity;
struct Frame {
    float dt;
    double time;
};

struct Script {
    virtual ~Script() = default;
    virtual void update(Entity& self, const Frame& frame) = 0;
};

struct Entity {
    std::string name;
    Transform transform;
    std::unique_ptr<Mesh> mesh;
    std::unique_ptr<Collider> collider;
    std::unique_ptr<PointLight> light;
    std::vector<std::string> tags;
    std::vector<std::unique_ptr<Script>> scripts;
    double world[16] = {};
};

struct Spin final : Script {
    float speed;
    explicit Spin(float s) : speed(s) {}
    void update(Entity& self, const Frame& f) override {
        float& y = self.transform.rotation[1];
        y = std::fmod(y + speed * f.dt, 360.0f);
    }
};

struct Bob final : Script {
    float speed, height, base;
    Bob(float s, float h, float b) : speed(s), height(h), base(b) {}
    void update(Entity& self, const Frame& f) override {
        // Positions are kept in double, set from single precision as Bayleaf's scripts set them.
        self.transform.position[1] = base + height * std::sin(static_cast<float>(f.time) * speed);
    }
};

struct Follow final : Script {
    const Entity* target;
    float rate;
    Follow(const Entity* t, float r) : target(t), rate(r) {}
    void update(Entity& self, const Frame& f) override {
        const float k = std::min(rate * f.dt, 1.0f);
        double* p = self.transform.position;
        for (int axis : {0, 2}) {
            const float here = static_cast<float>(p[axis]);
            const float goal = static_cast<float>(target->transform.position[axis]);
            p[axis] = here + (goal - here) * k;
        }
    }
};

// Per-object property values: the same formulas as bench/src/main.rs.
float spin_speed(size_t i) { return 30.0f + static_cast<float>(i % 7) * 10.0f; }
float bob_speed(size_t i) { return 1.0f + static_cast<float>(i % 5) * 0.25f; }
float bob_height(size_t i) { return 0.25f + static_cast<float>(i % 3) * 0.1f; }
float follow_rate(size_t i) { return 0.05f + static_cast<float>(i % 4) * 0.05f; }

// Translation * rotation (X, then Y, then Z, degrees) * scale, column-major.
void world_matrix(const Transform& t, double* m) {
    constexpr double kRad = 3.14159265358979323846 / 180.0;
    const double ax = t.rotation[0] * kRad, ay = t.rotation[1] * kRad, az = t.rotation[2] * kRad;
    const double cx = std::cos(ax), sx = std::sin(ax), cy = std::cos(ay), sy = std::sin(ay), cz = std::cos(az), sz = std::sin(az);
    // R = Rz * Ry * Rx
    const double r[9] = {cy * cz, cy * sz, -sy, sx * sy * cz - cx * sz, sx * sy * sz + cx * cz, sx * cy, cx * sy * cz + sx * sz, cx * sy * sz - sx * cz, cx * cy};
    for (int c = 0; c < 3; ++c) {
        for (int row = 0; row < 3; ++row) m[c * 4 + row] = r[c * 3 + row] * t.scale[c];
        m[c * 4 + 3] = 0.0;
    }
    m[12] = t.position[0];
    m[13] = t.position[1];
    m[14] = t.position[2];
    m[15] = 1.0;
}

struct Stats {
    size_t frames;
    double mean, median, p5, p95, stdev;
};

Stats measure(const std::function<void()>& step) {
    using clock = std::chrono::steady_clock;
    for (int i = 0; i < kWarmup; ++i) step();
    std::vector<double> ms;
    const auto started = clock::now();
    while (ms.size() < kMaxFrames &&
           (ms.size() < kMinFrames || std::chrono::duration<double>(clock::now() - started).count() < kMinSeconds)) {
        const auto t = clock::now();
        step();
        ms.push_back(std::chrono::duration<double, std::milli>(clock::now() - t).count());
    }
    std::sort(ms.begin(), ms.end());
    double sum = 0;
    for (double v : ms) sum += v;
    const double mean = sum / static_cast<double>(ms.size());
    double var = 0;
    for (double v : ms) var += (v - mean) * (v - mean);
    auto at = [&](double q) { return ms[static_cast<size_t>(std::lround((static_cast<double>(ms.size()) - 1.0) * q))]; };
    return {ms.size(), mean, at(0.5), at(0.05), at(0.95), std::sqrt(var / static_cast<double>(ms.size()))};
}

void print(const char* layout, const char* part, size_t objects, const Stats& s, double check) {
    std::printf(
        "{\"mode\":\"cpp-%s\",\"scripts\":\"cpp\",\"part\":\"%s\",\"objects\":%zu,\"checksum\":%.6g,"
        "\"stats\":{\"frames\":%zu,\"meanMs\":%.6f,\"medianMs\":%.6f,\"p5Ms\":%.6f,\"p95Ms\":%.6f,\"stdevMs\":%.6f}}\n",
        layout, part, objects, check, s.frames, s.mean, s.median, s.p5, s.p95, s.stdev);
    std::fflush(stdout);
}

// Builds the same scene as bench/src/main.rs: the target first, then a grid of objects.
std::vector<Entity> build_scene(size_t n) {
    std::vector<Entity> entities(n + 1);
    entities[0].name = "Target";
    const size_t side = std::max<size_t>(1, static_cast<size_t>(std::ceil(std::sqrt(static_cast<double>(n)))));
    for (size_t i = 0; i < n; ++i) {
        Entity& e = entities[i + 1];
        e.name = "Object " + std::to_string(i);
        e.transform.position[0] = static_cast<double>(i % side) * 2.0 - static_cast<double>(side);
        e.transform.position[2] = static_cast<double>(i / side) * 2.0 - static_cast<double>(side);
        e.mesh = std::make_unique<Mesh>();
        e.collider = std::make_unique<Collider>();
        e.tags.push_back("bench");
        if (i % 10 == 0) e.light = std::make_unique<PointLight>();
        e.scripts.push_back(std::make_unique<Spin>(spin_speed(i)));
        e.scripts.push_back(std::make_unique<Bob>(bob_speed(i), bob_height(i), 0.5f));
        e.scripts.push_back(std::make_unique<Follow>(&entities[0], follow_rate(i)));
    }
    return entities;
}

double checksum(const std::vector<Entity>& entities) {
    double sum = 0;
    for (const Entity& e : entities) sum += e.transform.position[0] + e.transform.position[1] + e.transform.position[2] + e.transform.rotation[1] + e.world[0];
    return sum;
}

void run_virtual(size_t n, bool with_matrices) {
    std::vector<Entity> entities = build_scene(n);
    double time = 0;
    const Stats s = measure([&] {
        time += kDt;
        const Frame frame{kDt, time};
        for (Entity& e : entities)
            for (auto& script : e.scripts) script->update(e, frame);
        if (with_matrices)
            for (Entity& e : entities) world_matrix(e.transform, e.world);
    });
    print("virtual", with_matrices ? "step" : "scripts", n, s, checksum(entities));
}

// The same data in flat arrays, one loop per script kind.
void run_packed(size_t n, bool with_matrices) {
    std::vector<Transform> transforms(n + 1);
    std::vector<std::array<double, 16>> world(n + 1);
    struct SpinData { size_t entity; float speed; };
    struct BobData { size_t entity; float speed, height, base; };
    struct FollowData { size_t entity; size_t target; float rate; };
    std::vector<SpinData> spins;
    std::vector<BobData> bobs;
    std::vector<FollowData> follows;
    const size_t side = std::max<size_t>(1, static_cast<size_t>(std::ceil(std::sqrt(static_cast<double>(n)))));
    for (size_t i = 0; i < n; ++i) {
        Transform& t = transforms[i + 1];
        t.position[0] = static_cast<double>(i % side) * 2.0 - static_cast<double>(side);
        t.position[2] = static_cast<double>(i / side) * 2.0 - static_cast<double>(side);
        spins.push_back({i + 1, spin_speed(i)});
        bobs.push_back({i + 1, bob_speed(i), bob_height(i), 0.5f});
        follows.push_back({i + 1, 0, follow_rate(i)});
    }
    double time = 0;
    const Stats s = measure([&] {
        time += kDt;
        for (const SpinData& d : spins) {
            float& y = transforms[d.entity].rotation[1];
            y = std::fmod(y + d.speed * kDt, 360.0f);
        }
        for (const BobData& d : bobs) transforms[d.entity].position[1] = d.base + d.height * std::sin(static_cast<float>(time) * d.speed);
        for (const FollowData& d : follows) {
            const float k = std::min(d.rate * kDt, 1.0f);
            double* p = transforms[d.entity].position;
            const double* goal = transforms[d.target].position;
            for (int axis : {0, 2}) {
                const float here = static_cast<float>(p[axis]);
                p[axis] = here + (static_cast<float>(goal[axis]) - here) * k;
            }
        }
        if (with_matrices)
            for (size_t i = 0; i < transforms.size(); ++i) world_matrix(transforms[i], world[i].data());
    });
    double check = 0;
    for (size_t i = 0; i < transforms.size(); ++i) check += transforms[i].position[0] + transforms[i].position[1] + transforms[i].rotation[1] + world[i][0];
    print("packed", with_matrices ? "step" : "scripts", n, s, check);
}

}  // namespace

int main(int argc, char** argv) {
    const size_t n = argc > 1 ? static_cast<size_t>(std::strtoull(argv[1], nullptr, 10)) : 5000;
    run_virtual(n, false);
    run_virtual(n, true);
    run_packed(n, false);
    run_packed(n, true);
    return 0;
}
