// What the program knows of the game: how to fill a runtime with its scene and scripts.

#pragma once

#include <cstddef>

#include "runtime.hpp"

namespace game {

// `typed`: scripts as plain structs in one array per type; else heap objects with a virtual update.
void build(mini::Runtime& runtime, size_t n, bool typed);

}  // namespace game
