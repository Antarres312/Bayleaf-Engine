// Times the small runtime of runtime.hpp running the game of game.cpp (bench/README.md, "The same
// runtime in C++ and Rust"). Prints one JSON line per style (typed, virtual) and part (scripts:
// every script's update; step: that, then world matrices of what moved and reading the moved list).
//
// Build (from a VS x64 developer prompt): cl /nologo /O2 /EHsc /std:c++17 /Fe:runtime.exe main.cpp game.cpp
// Run: runtime.exe <objects>

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <functional>
#include <vector>

#include "game.hpp"

namespace {

constexpr float kDt = 1.0f / 60.0f;
constexpr int kWarmup = 30;
constexpr size_t kMinFrames = 100;
constexpr double kMinSeconds = 3.0;
constexpr size_t kMaxFrames = 20000;
// Steps of the fixed run whose checksum the two languages must agree on.
constexpr int kCheckSteps = 100;

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

// The checksum after a fixed number of whole steps: equal across languages and styles when they do
// the same work.
double check(size_t n, bool typed) {
    mini::Runtime runtime;
    game::build(runtime, n, typed);
    runtime.start();
    double time = 0;
    for (int i = 0; i < kCheckSteps; ++i) {
        time += kDt;
        runtime.update(mini::Frame{kDt, time});
        runtime.world.finish_step();
    }
    return runtime.world.checksum();
}

void run(size_t n, bool typed, bool whole_step) {
    mini::Runtime runtime;
    game::build(runtime, n, typed);
    runtime.start();
    runtime.world.finish_step();
    double time = 0;
    volatile double taken = 0;
    const Stats s = measure([&] {
        time += kDt;
        runtime.update(mini::Frame{kDt, time});
        if (whole_step)
            taken = taken + runtime.world.finish_step();
        else
            runtime.world.forget_moved();
    });
    std::printf(
        "{\"mode\":\"cpp-runtime-%s\",\"scripts\":\"cpp\",\"part\":\"%s\",\"objects\":%zu,\"check\":%.9e,\"world\":%zu,\"data\":%.9e,"
        "\"stats\":{\"frames\":%zu,\"meanMs\":%.6f,\"medianMs\":%.6f,\"p5Ms\":%.6f,\"p95Ms\":%.6f,\"stdevMs\":%.6f}}\n",
        typed ? "typed" : "virtual", whole_step ? "step" : "scripts", n, check(n, typed), runtime.world.size(), runtime.world.data_sum(), s.frames, s.mean, s.median, s.p5, s.p95,
        s.stdev);
    std::fflush(stdout);
}

}  // namespace

int main(int argc, char** argv) {
    const size_t n = argc > 1 ? static_cast<size_t>(std::strtoull(argv[1], nullptr, 10)) : 5000;
    for (const bool typed : {true, false})
        for (const bool whole_step : {false, true}) run(n, typed, whole_step);
    return 0;
}
