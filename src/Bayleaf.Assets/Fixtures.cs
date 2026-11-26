using System.Buffers.Binary;
using System.Text;

using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Assets;

/// <summary>
/// A small skinned model written in code, for tests and examples (our own work; see
/// ATTRIBUTION.md): a wall two units wide whose left edge follows the joint <c>Root</c> and whose
/// right edge follows <c>Arm</c>, with a <c>Hand</c> joint at that edge. Its animations, a second
/// long: <c>Stretch</c> moves <c>Arm</c> 1.5 to the right, <c>Raise</c> moves it 1 up, <c>Wave</c>
/// turns <c>Hand</c> a quarter turn about Z. The files are the same, byte for byte, as the Rust
/// engine's fixture writes.
/// </summary>
public static class Fixtures
{
    /// <summary>Writes <c>arm.gltf</c> and <c>arm.bin</c> into <paramref name="dir"/>; gives the path of <c>arm.gltf</c>.</summary>
    public static Result<string, string> SkinnedArm(string dir)
    {
        var (bin, gltf) = SkinnedArmFiles();
        var written = Files.Write(Path.Join(dir, "arm.bin"), bin);
        if (written.IsError)
        {
            return written.AsFailure();
        }
        var path = Path.Join(dir, "arm.gltf");
        written = Files.Write(path, Encoding.UTF8.GetBytes(gltf));
        return written.IsOk ? path : written.AsFailure();
    }

    /// <summary>The fixture's buffer and its glTF text.</summary>
    internal static (byte[] Bin, string Gltf) SkinnedArmFiles()
    {
        var bin = new List<byte>();
        var four = new byte[4];
        void F32s(params float[] values)
        {
            foreach (var v in values)
            {
                BinaryPrimitives.WriteSingleLittleEndian(four, v);
                bin.AddRange(four);
            }
        }
        // Positions 0..48: a wall from (-1, -1) to (1, 1), facing +Z.
        F32s(-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, 1, 0);
        // Indices 48..60.
        foreach (ushort i in new ushort[] { 0, 1, 2, 0, 2, 3 })
        {
            bin.Add((byte)i);
            bin.Add((byte)(i >> 8));
        }
        // Joints 60..76: the left edge follows joint 0 (Root), the right edge joint 1 (Arm).
        foreach (var joint in new byte[] { 0, 1, 1, 0 })
        {
            bin.AddRange([joint, 0, 0, 0]);
        }
        // Weights 76..140.
        for (var i = 0; i < 4; i++)
        {
            F32s(1, 0, 0, 0);
        }
        // Key times 140..148, then Stretch 148..172, Raise 172..196 and Wave 196..228.
        F32s(0, 1);
        F32s(0, 0, 0, 1.5f, 0, 0);
        F32s(0, 0, 0, 0, 1, 0);
        const float half = 0.70710678118654752f;
        F32s(0, 0, 0, 1, 0, 0, half, half);
        static JsonNumber N(long v) => JsonNumber.Of(v);
        static JsonNumber D(double v) => JsonNumber.Of(v);
        static JsonArray A(params JsonValue[] items) => new(items);
        static JsonObject View(long offset, long length) => new() { { "buffer", N(0) }, { "byteOffset", N(offset) }, { "byteLength", N(length) } };
        static JsonObject Channel(string name, long node, string path, long output) => new()
        {
            { "channels", A(new JsonObject { { "sampler", N(0) }, { "target", new JsonObject { { "node", N(node) }, { "path", new JsonString(path) } } } }) },
            { "samplers", A(new JsonObject { { "input", N(4) }, { "output", N(output) } }) },
            { "name", new JsonString(name) },
        };
        static JsonObject Accessor(long view, long type, long count, string kind) => new()
        {
            { "bufferView", N(view) }, { "componentType", N(type) }, { "count", N(count) }, { "type", new JsonString(kind) },
        };
        var positions = Accessor(0, 5126, 4, "VEC3");
        positions.Add("min", A(N(-1), N(-1), N(0)));
        positions.Add("max", A(N(1), N(1), N(0)));
        var times = Accessor(4, 5126, 2, "SCALAR");
        times.Add("min", A(N(0)));
        times.Add("max", A(N(1)));
        var root = new JsonObject
        {
            { "asset", new JsonObject { { "version", new JsonString("2.0") }, { "generator", new JsonString("Bayleaf Engine test fixture") } } },
            { "scene", N(0) },
            { "scenes", A(new JsonObject { { "nodes", A(N(0)) } }) },
            {
                "nodes", A(
                    new JsonObject { { "name", new JsonString("Root") }, { "mesh", N(0) }, { "skin", N(0) }, { "children", A(N(1)) } },
                    new JsonObject { { "name", new JsonString("Arm") }, { "children", A(N(2)) } },
                    new JsonObject { { "name", new JsonString("Hand") }, { "translation", A(D(1.0), D(0.0), D(0.0)) } })
            },
            {
                "meshes", A(new JsonObject
                {
                    {
                        "primitives", A(new JsonObject
                        {
                            { "attributes", new JsonObject { { "POSITION", N(0) }, { "JOINTS_0", N(2) }, { "WEIGHTS_0", N(3) } } }, { "indices", N(1) },
                        })
                    },
                })
            },
            { "skins", A(new JsonObject { { "joints", A(N(0), N(1), N(2)) } }) },
            { "animations", A(Channel("Stretch", 1, "translation", 5), Channel("Raise", 1, "translation", 6), Channel("Wave", 2, "rotation", 7)) },
            { "buffers", A(new JsonObject { { "uri", new JsonString("arm.bin") }, { "byteLength", N(bin.Count) } }) },
            { "bufferViews", A(View(0, 48), View(48, 12), View(60, 16), View(76, 64), View(140, 8), View(148, 24), View(172, 24), View(196, 32)) },
            {
                "accessors", A(positions, Accessor(1, 5123, 6, "SCALAR"), Accessor(2, 5121, 4, "VEC4"), Accessor(3, 5126, 4, "VEC4"), times,
                    Accessor(5, 5126, 2, "VEC3"), Accessor(6, 5126, 2, "VEC3"), Accessor(7, 5126, 2, "VEC4"))
            },
        };
        // The Rust engine's JSON values keep their keys sorted.
        return ([.. bin], root.Canonical().ToCompact());
    }
}
