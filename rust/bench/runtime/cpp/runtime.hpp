// A small game runtime for the comparison in bench/README.md ("The same runtime in C++ and Rust"):
// the design a fast Bayleaf runtime would have, written the same way in both languages
// (bench/runtime/rust/src/runtime.rs mirrors this file).
//
// - Components in packed arrays, an object is an index with a generation.
// - Objects are found by a 128-bit id or a name through hash maps, at start, not per step.
// - Writing a transform marks the object moved, once per step; world matrices are worked out for
//   the moved objects only, and the list of them is what a renderer or an editor would read.
// - Game scripts are plain structs kept in one array per script type; the runtime makes one indirect
//   call per type per step ("typed"). For comparison, the usual heap object per script behind a
//   virtual update per object ("virtual") runs on the same world.

#pragma once

#include <cmath>
#include <cstdint>
#include <memory>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

namespace mini {

struct Id {
    uint64_t hi = 0, lo = 0;
    bool operator==(const Id& o) const { return hi == o.hi && lo == o.lo; }
};
struct IdHash {
    size_t operator()(const Id& id) const { return static_cast<size_t>(id.hi ^ (id.lo * 0x9e3779b97f4a7c15ull)); }
};

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

// An object: its place in the arrays, and the generation that place had when the handle was made.
struct Entity {
    uint32_t index = 0, generation = 0;
};

struct Frame {
    float dt;
    double time;
};

// Translation * rotation (X, then Y, then Z, degrees) * scale, column-major (as bench/cpp/bench.cpp).
inline void world_matrix(const Transform& t, double* m) {
    constexpr double kRad = 3.14159265358979323846 / 180.0;
    const double ax = t.rotation[0] * kRad, ay = t.rotation[1] * kRad, az = t.rotation[2] * kRad;
    const double cx = std::cos(ax), sx = std::sin(ax), cy = std::cos(ay), sy = std::sin(ay), cz = std::cos(az), sz = std::sin(az);
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

class World {
public:
    Entity create(Id id, std::string name) {
        const auto index = static_cast<uint32_t>(transforms_.size());
        by_id_.emplace(id, index);
        by_name_.emplace(name, index);
        ids_.push_back(id);
        names_.push_back(std::move(name));
        transforms_.emplace_back();
        world_.emplace_back();
        generation_.push_back(1);
        moved_flag_.push_back(0);
        mark(index);
        return {index, 1};
    }

    size_t size() const { return transforms_.size(); }
    bool alive(Entity e) const { return e.index < generation_.size() && generation_[e.index] == e.generation; }
    // The first object of that name; generation 0 (never alive) when there is none.
    Entity find(std::string_view name) const {
        const auto found = by_name_.find(std::string(name));
        return found == by_name_.end() ? Entity{} : Entity{found->second, generation_[found->second]};
    }
    Entity find(Id id) const {
        const auto found = by_id_.find(id);
        return found == by_id_.end() ? Entity{} : Entity{found->second, generation_[found->second]};
    }

    const Transform& transform(Entity e) const { return transforms_[e.index]; }
    // For writing: the object counts as moved this step.
    Transform& transform_mut(Entity e) {
        mark(e.index);
        return transforms_[e.index];
    }
    const double* world_of(Entity e) const { return world_[e.index].m; }

    void add(Entity e, Mesh c) { meshes_.push_back({e.index, c}); }
    void add(Entity e, Collider c) { colliders_.push_back({e.index, c}); }
    void add(Entity e, PointLight c) { lights_.push_back({e.index, c}); }
    void tag(Entity e, std::string tag) { tags_.push_back({e.index, std::move(tag)}); }

    // World matrices of the objects moved since the last call. Returns what a consumer of the moved
    // list (a renderer, an editor's viewport) would take: here, the sum of their world x positions.
    double finish_step() {
        double taken = 0;
        for (const uint32_t index : moved_) {
            world_matrix(transforms_[index], world_[index].m);
            taken += world_[index].m[12];
            moved_flag_[index] = 0;
        }
        moved_.clear();
        return taken;
    }
    // Only forgets what moved (for timing the scripts alone).
    void forget_moved() {
        for (const uint32_t index : moved_) moved_flag_[index] = 0;
        moved_.clear();
    }

    // A sum over every component that only holds data, and the target found by its id: equal across
    // languages when the scenes are the same.
    double data_sum() const {
        double sum = 0;
        for (const auto& m : meshes_) sum += m.value.shape + static_cast<double>(m.value.color[0] + m.value.color[1] + m.value.color[2] + m.value.color[3] + m.value.metallic + m.value.roughness);
        for (const auto& c : colliders_)
            sum += c.value.shape + static_cast<double>(c.value.size[0] + c.value.size[1] + c.value.size[2] + c.value.friction + c.value.restitution) + (c.value.trigger ? 1.0 : 0.0);
        for (const auto& l : lights_) sum += static_cast<double>(l.value.color[0] + l.value.color[1] + l.value.color[2] + l.value.intensity + l.value.range);
        for (const auto& t : tags_) sum += static_cast<double>(t.value.size());
        return sum + world_of(find(Id{0, 1}))[15];
    }

    double checksum() const {
        double sum = 0;
        for (size_t i = 0; i < transforms_.size(); ++i) {
            const Transform& t = transforms_[i];
            sum += t.position[0] + t.position[1] + t.position[2] + t.rotation[1] + world_[i].m[0];
        }
        return sum;
    }

private:
    struct Matrix {
        double m[16] = {};
    };
    template <class C>
    struct Held {
        uint32_t entity;
        C value;
    };

    void mark(uint32_t index) {
        if (!moved_flag_[index]) {
            moved_flag_[index] = 1;
            moved_.push_back(index);
        }
    }

    std::vector<Id> ids_;
    std::vector<std::string> names_;
    std::vector<Transform> transforms_;
    std::vector<Matrix> world_;
    std::vector<uint32_t> generation_;
    std::vector<uint8_t> moved_flag_;
    std::vector<uint32_t> moved_;
    std::vector<Held<Mesh>> meshes_;
    std::vector<Held<Collider>> colliders_;
    std::vector<Held<PointLight>> lights_;
    std::vector<Held<std::string>> tags_;
    std::unordered_map<Id, uint32_t, IdHash> by_id_;
    std::unordered_map<std::string, uint32_t> by_name_;
};

// What a script works with: the world and its own object.
struct Self {
    World& world;
    Entity entity;
    Transform& transform_mut() { return world.transform_mut(entity); }
};

// "virtual": a heap object per script, a virtual call per object per step.
struct Script {
    virtual ~Script() = default;
    virtual void start(Self&) {}
    virtual void update(Self&, const Frame&) = 0;
};

// "typed": every script of one type in one array; one indirect call per type per step.
struct Store {
    virtual ~Store() = default;
    virtual void start_all(World&) = 0;
    virtual void update_all(World&, const Frame&) = 0;
};
template <class T>
struct Typed final : Store {
    std::vector<Entity> owners;
    std::vector<T> items;
    void start_all(World& world) override {
        for (size_t i = 0; i < items.size(); ++i) {
            Self self{world, owners[i]};
            items[i].start(self);
        }
    }
    void update_all(World& world, const Frame& frame) override {
        for (size_t i = 0; i < items.size(); ++i) {
            Self self{world, owners[i]};
            items[i].update(self, frame);
        }
    }
};

class Runtime {
public:
    World world;

    // Typed: `T` needs `void start(Self&)` and `void update(Self&, const Frame&)`.
    template <class T>
    void add(Entity owner, T script) {
        static const size_t slot = next_slot()++;
        if (slot >= stores_.size()) stores_.resize(slot + 1);
        if (!stores_[slot]) stores_[slot] = std::make_unique<Typed<T>>();
        auto& store = static_cast<Typed<T>&>(*stores_[slot]);
        store.owners.push_back(owner);
        store.items.push_back(std::move(script));
    }
    // Virtual: the object owns its scripts.
    void add_virtual(Entity owner, std::unique_ptr<Script> script) {
        if (owner.index >= owned_.size()) owned_.resize(owner.index + 1);
        owned_[owner.index].push_back(std::move(script));
    }

    void start() {
        for (auto& store : stores_)
            if (store) store->start_all(world);
        for (uint32_t i = 0; i < owned_.size(); ++i) {
            Self self{world, Entity{i, 1}};
            for (auto& script : owned_[i]) script->start(self);
        }
    }
    void update(const Frame& frame) {
        for (auto& store : stores_)
            if (store) store->update_all(world, frame);
        for (uint32_t i = 0; i < owned_.size(); ++i) {
            Self self{world, Entity{i, 1}};
            for (auto& script : owned_[i]) script->update(self, frame);
        }
    }

private:
    static size_t& next_slot() {
        static size_t next = 0;
        return next;
    }
    std::vector<std::unique_ptr<Store>> stores_;
    std::vector<std::vector<std::unique_ptr<Script>>> owned_;
};

}  // namespace mini
