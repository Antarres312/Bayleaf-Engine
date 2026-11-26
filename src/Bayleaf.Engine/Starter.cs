namespace Bayleaf.Engine;

/// <summary>The files <c>project.new</c> writes, the same as the Rust engine's starting point.</summary>
internal static class Starter
{
    /// <summary>The starting scene: a camera, the sun, a checkered ground and a spinning cube, lit by <c>scenes/daylight.blight</c>.</summary>
    public const string MainScene = """"
{
  "format": "bayleaf.scene",
  "version": 17,
  "settings": { "lighting": "scenes/daylight.blight" },
  "objects": [
    {
      "id": "0b1f6e3a8c2d4e5f9a7b6c5d4e3f2a10",
      "parent": null,
      "components": {
        "name": "Camera",
        "camera": { "projection": "perspective", "fov": 60.0, "size": 5.0, "near": 0.1, "far": 500.0 },
        "transform": { "position": [0.0, 3.0, 8.0], "rotation": [-18.0, 0.0, 0.0], "scale": [1.0, 1.0, 1.0] }
      }
    },
    {
      "id": "0b1f6e3a8c2d4e5f9a7b6c5d4e3f2a11",
      "parent": null,
      "components": {
        "name": "Sun",
        "directionalLight": { "color": [1.0, 0.96, 0.9], "intensity": 3.0 },
        "transform": { "position": [0.0, 5.0, 0.0], "rotation": [-50.0, 30.0, 0.0], "scale": [1.0, 1.0, 1.0] }
      }
    },
    {
      "id": "0b1f6e3a8c2d4e5f9a7b6c5d4e3f2a12",
      "parent": null,
      "components": {
        "name": "Ground",
        "mesh": { "shape": "plane", "color": [0.3, 0.36, 0.32, 1.0], "metallic": 0.0, "roughness": 0.9, "material": "materials/ground.bmaterial" },
        "transform": { "position": [0.0, 0.0, 0.0], "rotation": [0.0, 0.0, 0.0], "scale": [20.0, 1.0, 20.0] }
      }
    },
    {
      "id": "0b1f6e3a8c2d4e5f9a7b6c5d4e3f2a13",
      "parent": null,
      "components": {
        "name": "Cube",
        "mesh": { "shape": "cube", "color": [0.85, 0.45, 0.25, 1.0], "metallic": 0.0, "roughness": 0.45 },
        "scripts": [ { "module": "scripts/spin.bscript" } ],
        "transform": { "position": [0.0, 0.5, 0.0], "rotation": [0.0, 0.0, 0.0], "scale": [1.0, 1.0, 1.0] }
      }
    }
  ]
}
"""" + "\n";

    /// <summary>A checkered ground in two greens.</summary>
    public const string GroundMaterial = """"
{
  "format": "bayleaf.graph",
  "version": 1,
  "domain": "material",
  "nodes": [
    { "id": 1, "kind": "output", "position": [560.0, 40.0], "values": { "roughness": 0.9 } },
    { "id": 2, "kind": "checker", "position": [0.0, 40.0] },
    { "id": 3, "kind": "blend", "position": [280.0, 40.0], "values": { "a": [0.07, 0.085, 0.08], "b": [0.13, 0.16, 0.14] } }
  ],
  "links": [
    { "from": { "node": 2, "pin": "value" }, "to": { "node": 3, "pin": "factor" } },
    { "from": { "node": 3, "pin": "result" }, "to": { "node": 1, "pin": "baseColor" } }
  ]
}
"""" + "\n";

    /// <summary>Turns its object around Y by its Speed property, in degrees per second.</summary>
    public const string SpinScript = """"
{
  "format": "bayleaf.graph",
  "version": 1,
  "domain": "script",
  "nodes": [
    { "id": 1, "kind": "update", "position": [40.0, 40.0] },
    { "id": 2, "kind": "property", "position": [40.0, 200.0], "values": { "name": "speed", "default": 45.0 } },
    { "id": 3, "kind": "multiply", "position": [300.0, 200.0] },
    { "id": 4, "kind": "vector", "position": [540.0, 200.0] },
    { "id": 5, "kind": "rotateBy", "position": [780.0, 40.0] }
  ],
  "links": [
    { "from": { "node": 2, "pin": "value" }, "to": { "node": 3, "pin": "a" } },
    { "from": { "node": 1, "pin": "dt" }, "to": { "node": 3, "pin": "b" } },
    { "from": { "node": 3, "pin": "result" }, "to": { "node": 4, "pin": "y" } },
    { "from": { "node": 1, "pin": "then" }, "to": { "node": 5, "pin": "run" } },
    { "from": { "node": 4, "pin": "vector" }, "to": { "node": 5, "pin": "degrees" } }
  ]
}
"""" + "\n";
}
