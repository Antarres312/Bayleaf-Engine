using System.Buffers.Binary;

using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Assets.Gltf;

internal sealed record BufferDef(string? Uri, ulong ByteLength);

internal sealed record ViewDef(int Buffer, ulong Offset, ulong Length, int? Stride);

internal sealed record SparseDef(int Count, int IndicesView, ulong IndicesOffset, int IndicesType, int ValuesView, ulong ValuesOffset);

/// <summary>An accessor: <see cref="Type"/> is the number of components of an element (1 for SCALAR, 16 for MAT4).</summary>
internal sealed record AccessorDef(int? View, ulong Offset, int ComponentType, int Count, int Type, SparseDef? Sparse, float[]? Min, float[]? Max);

internal sealed record PrimitiveDef(IReadOnlyDictionary<string, int> Attributes, int? Indices, int? Material, int Mode);

internal sealed record MeshDef(IReadOnlyList<PrimitiveDef> Primitives);

internal sealed record NodeDef(string? Name, IReadOnlyList<int> Children, int? Mesh, int? Skin, float[]? Matrix, float[] Translation, float[] Rotation, float[] Scale);

internal sealed record SkinDef(IReadOnlyList<int> Joints, int? InverseBindMatrices);

internal enum TargetPath
{
    Translation,
    Rotation,
    Scale,
    Weights,
}

internal enum Interpolation
{
    Linear,
    Step,
    CubicSpline,
}

internal sealed record ChannelDef(int Sampler, int Node, TargetPath Path);

internal sealed record SamplerDef(int Input, int Output, Interpolation Interpolation);

internal sealed record AnimationDef(string? Name, IReadOnlyList<ChannelDef> Channels, IReadOnlyList<SamplerDef> Samplers);

internal sealed record TextureInfo(int Index, int TexCoord);

internal sealed record MaterialDef(float[] BaseColorFactor, float Metallic, float Roughness, TextureInfo? BaseColorTexture, TextureInfo? MetallicRoughnessTexture, TextureInfo? NormalTexture);

internal sealed record ImageDef(string? Uri, int? View);

/// <summary>
/// A glTF 2.0 document (Khronos glTF 2.0 specification): the JSON of a <c>.gltf</c> file or of a
/// <c>.glb</c> file's first chunk, with the <c>.glb</c>'s binary chunk. Every index is checked to
/// name something that is there and every required property to be present, so readers of the
/// document need not check them again.
/// </summary>
internal sealed class Document
{
    public List<BufferDef> Buffers { get; } = [];

    public List<ViewDef> Views { get; } = [];

    public List<AccessorDef> Accessors { get; } = [];

    public List<MeshDef> Meshes { get; } = [];

    public List<NodeDef> Nodes { get; } = [];

    public List<IReadOnlyList<int>> Scenes { get; } = [];

    public int? Scene { get; private set; }

    public List<SkinDef> Skins { get; } = [];

    public List<AnimationDef> Animations { get; } = [];

    public List<MaterialDef> Materials { get; } = [];

    /// <summary>Each texture's image.</summary>
    public List<int> Textures { get; } = [];

    public List<ImageDef> Images { get; } = [];

    /// <summary>A <c>.glb</c>'s binary chunk.</summary>
    public byte[]? Blob { get; private set; }

    private sealed class Invalid(string message) : Exception(message);

    private const uint JsonChunk = 0x4E4F534A;
    private const uint BinChunk = 0x004E4942;

    /// <summary>Components of an element of each accessor type.</summary>
    private static readonly Dictionary<string, int> TypeSizes = new(StringComparer.Ordinal)
    {
        ["SCALAR"] = 1, ["VEC2"] = 2, ["VEC3"] = 3, ["VEC4"] = 4, ["MAT2"] = 4, ["MAT3"] = 9, ["MAT4"] = 16,
    };

    public static Result<Document, string> Parse(byte[] bytes)
    {
        try
        {
            var document = new Document();
            ReadOnlySpan<byte> json = bytes;
            if (bytes.AsSpan().StartsWith("glTF"u8))
            {
                json = document.ReadGlb(bytes);
            }
            var text = JsonReader.DecodeUtf8(json);
            if (text.IsError)
            {
                throw new Invalid(text.Error);
            }
            var parsed = JsonReader.Parse(text.Value);
            if (parsed.IsError)
            {
                throw new Invalid(parsed.Error);
            }
            document.Read(parsed.Value);
            return document;
        }
        catch (Invalid e)
        {
            return Result.Fail(e.Message);
        }
        catch (JsonShapeException e)
        {
            return Result.Fail(e.Message);
        }
    }

    /// <summary>The binary file's JSON chunk; its binary chunk kept as the blob.</summary>
    private ReadOnlySpan<byte> ReadGlb(byte[] bytes)
    {
        if (bytes.Length < 12)
        {
            throw new Invalid("the binary file is shorter than its header");
        }
        var version = BinaryPrimitives.ReadUInt32LittleEndian(bytes.AsSpan(4));
        var length = BinaryPrimitives.ReadUInt32LittleEndian(bytes.AsSpan(8));
        if (version != 2)
        {
            throw new Invalid($"binary glTF version {version} is not 2");
        }
        if (length > bytes.Length || length < 12)
        {
            throw new Invalid("the binary file is shorter than it says");
        }
        ReadOnlySpan<byte> json = default;
        var at = 12;
        var index = 0;
        while (at + 8 <= length)
        {
            var size = BinaryPrimitives.ReadUInt32LittleEndian(bytes.AsSpan(at));
            var type = BinaryPrimitives.ReadUInt32LittleEndian(bytes.AsSpan(at + 4));
            if (size > length - at - 8)
            {
                throw new Invalid("a chunk runs past the end of the binary file");
            }
            var data = bytes.AsSpan(at + 8, (int)size);
            if (index == 0)
            {
                if (type != JsonChunk)
                {
                    throw new Invalid("the binary file's first chunk is not JSON");
                }
                json = data;
            }
            else if (index == 1 && type == BinChunk)
            {
                Blob = data.ToArray();
            }
            at += 8 + (int)size;
            index++;
        }
        if (index == 0)
        {
            throw new Invalid("the binary file has no JSON chunk");
        }
        return json;
    }

    private static JsonValue? Field(JsonValue o, string name) => o.Get(name) is { IsNull: false } v ? v : null;

    private static List<JsonValue> Array(JsonValue root, string name)
    {
        var v = Field(root, name);
        if (v is null)
        {
            return [];
        }
        return v is JsonArray a ? [.. a] : throw new Invalid($"{name} must be an array");
    }

    private static JsonValue Object(JsonValue v, string what) => v is JsonObject ? v : throw new Invalid($"{what} must be an object");

    private static ulong Unsigned(JsonValue v, string what) =>
        v is JsonNumber n && n.AsUInt64() is { } u ? u : throw new Invalid($"{what} must be a whole number of 0 or more");

    private static int Index(JsonValue v, string what) => Unsigned(v, what) is var u && u <= int.MaxValue ? (int)u : throw new Invalid($"{what} is too large");

    private static int? OptionalIndex(JsonValue o, string name, string what) => Field(o, name) is { } v ? Index(v, what) : null;

    private static int RequiredIndex(JsonValue o, string name, string what) => Field(o, name) is { } v ? Index(v, what) : throw new Invalid($"{what} is missing");

    private static string? OptionalString(JsonValue o, string name) => Field(o, name) is { } v ? JsonRead.Str(v) : null;

    private static float Number(JsonValue v, string what) => v is JsonNumber n ? n.ToSingle() : throw new Invalid($"{what} must be a number");

    private static float[] Numbers(JsonValue v, int count, string what)
    {
        if (v is not JsonArray a || a.Count != count)
        {
            throw new Invalid($"{what} must be an array of {count} numbers");
        }
        return [.. a.Select(x => Number(x, what))];
    }

    private static void Check(int? index, int count, string what)
    {
        if (index is { } i && i >= count)
        {
            throw new Invalid($"{what} {i} does not exist");
        }
    }

    private void Read(JsonValue root)
    {
        Object(root, "the document");
        var asset = Field(root, "asset") ?? throw new Invalid("asset is missing");
        if (Field(Object(asset, "asset"), "version") is not JsonString)
        {
            throw new Invalid("asset.version is missing");
        }
        if (Array(root, "extensionsRequired").Count > 0)
        {
            throw new Invalid($"the model needs the extension {Array(root, "extensionsRequired")[0]}, which this engine does not read");
        }
        foreach (var b in Array(root, "buffers"))
        {
            Object(b, "a buffer");
            Buffers.Add(new(OptionalString(b, "uri"), Unsigned(Field(b, "byteLength") ?? throw new Invalid("buffer.byteLength is missing"), "buffer.byteLength")));
        }
        foreach (var v in Array(root, "bufferViews"))
        {
            Object(v, "a buffer view");
            var stride = OptionalIndex(v, "byteStride", "bufferView.byteStride");
            if (stride is < 4 or > 252)
            {
                throw new Invalid("bufferView.byteStride must be from 4 to 252");
            }
            Views.Add(new(RequiredIndex(v, "buffer", "bufferView.buffer"), Field(v, "byteOffset") is { } o ? Unsigned(o, "bufferView.byteOffset") : 0,
                Unsigned(Field(v, "byteLength") ?? throw new Invalid("bufferView.byteLength is missing"), "bufferView.byteLength"), stride));
        }
        foreach (var a in Array(root, "accessors"))
        {
            Accessors.Add(ReadAccessor(Object(a, "an accessor")));
        }
        foreach (var m in Array(root, "meshes"))
        {
            Object(m, "a mesh");
            var primitives = new List<PrimitiveDef>();
            foreach (var p in Array(m, "primitives"))
            {
                Object(p, "a primitive");
                var attributes = new Dictionary<string, int>(StringComparer.Ordinal);
                if (Field(p, "attributes") is not JsonObject o)
                {
                    throw new Invalid("primitive.attributes is missing");
                }
                foreach (var (key, value) in o.Entries)
                {
                    attributes[key] = Index(value, $"attribute {key}");
                }
                var mode = OptionalIndex(p, "mode", "primitive.mode") ?? 4;
                if (mode > 6)
                {
                    throw new Invalid($"primitive.mode {mode} is not a mode");
                }
                primitives.Add(new(attributes, OptionalIndex(p, "indices", "primitive.indices"), OptionalIndex(p, "material", "primitive.material"), mode));
            }
            Meshes.Add(new(primitives));
        }
        foreach (var n in Array(root, "nodes"))
        {
            Object(n, "a node");
            Nodes.Add(new(OptionalString(n, "name"), [.. Array(n, "children").Select(c => Index(c, "node.children"))], OptionalIndex(n, "mesh", "node.mesh"),
                OptionalIndex(n, "skin", "node.skin"), Field(n, "matrix") is { } mx ? Numbers(mx, 16, "node.matrix") : null,
                Field(n, "translation") is { } t ? Numbers(t, 3, "node.translation") : [0, 0, 0], Field(n, "rotation") is { } r ? Numbers(r, 4, "node.rotation") : [0, 0, 0, 1],
                Field(n, "scale") is { } s ? Numbers(s, 3, "node.scale") : [1, 1, 1]));
        }
        foreach (var s in Array(root, "scenes"))
        {
            Object(s, "a scene");
            Scenes.Add([.. Array(s, "nodes").Select(x => Index(x, "scene.nodes"))]);
        }
        Scene = OptionalIndex(root, "scene", "scene");
        foreach (var s in Array(root, "skins"))
        {
            Object(s, "a skin");
            var joints = Field(s, "joints") is JsonArray j ? j.Select(x => Index(x, "skin.joints")).ToList() : throw new Invalid("skin.joints is missing");
            Skins.Add(new(joints, OptionalIndex(s, "inverseBindMatrices", "skin.inverseBindMatrices")));
        }
        foreach (var a in Array(root, "animations"))
        {
            Animations.Add(ReadAnimation(Object(a, "an animation")));
        }
        foreach (var m in Array(root, "materials"))
        {
            Materials.Add(ReadMaterial(Object(m, "a material")));
        }
        foreach (var t in Array(root, "textures"))
        {
            Object(t, "a texture");
            Textures.Add(RequiredIndex(t, "source", "texture.source"));
        }
        foreach (var i in Array(root, "images"))
        {
            Object(i, "an image");
            var (uri, view) = (OptionalString(i, "uri"), OptionalIndex(i, "bufferView", "image.bufferView"));
            if (uri is null && view is null)
            {
                throw new Invalid("an image has neither a uri nor a buffer view");
            }
            Images.Add(new(uri, view));
        }
        Validate();
    }

    private static AccessorDef ReadAccessor(JsonValue a)
    {
        var componentType = RequiredIndex(a, "componentType", "accessor.componentType");
        if (componentType is not (5120 or 5121 or 5122 or 5123 or 5125 or 5126))
        {
            throw new Invalid($"accessor.componentType {componentType} is not a component type");
        }
        var typeName = OptionalString(a, "type") ?? throw new Invalid("accessor.type is missing");
        if (!TypeSizes.TryGetValue(typeName, out var type))
        {
            throw new Invalid($"accessor.type {typeName} is not a type");
        }
        var count = RequiredIndex(a, "count", "accessor.count");
        SparseDef? sparse = null;
        if (Field(a, "sparse") is { } s)
        {
            Object(s, "accessor.sparse");
            var indices = Object(Field(s, "indices") ?? throw new Invalid("sparse.indices is missing"), "sparse.indices");
            var values = Object(Field(s, "values") ?? throw new Invalid("sparse.values is missing"), "sparse.values");
            var indexType = RequiredIndex(indices, "componentType", "sparse.indices.componentType");
            if (indexType is not (5121 or 5123 or 5125))
            {
                throw new Invalid("sparse.indices.componentType must be an unsigned type");
            }
            sparse = new(RequiredIndex(s, "count", "sparse.count"), RequiredIndex(indices, "bufferView", "sparse.indices.bufferView"),
                Field(indices, "byteOffset") is { } io ? Unsigned(io, "sparse.indices.byteOffset") : 0, indexType,
                RequiredIndex(values, "bufferView", "sparse.values.bufferView"), Field(values, "byteOffset") is { } vo ? Unsigned(vo, "sparse.values.byteOffset") : 0);
        }
        float[]? Bounds(string name) => Field(a, name) is JsonArray b ? [.. b.Select(x => x is JsonNumber n ? n.ToSingle() : float.NaN)] : null;
        return new(OptionalIndex(a, "bufferView", "accessor.bufferView"), Field(a, "byteOffset") is { } o ? Unsigned(o, "accessor.byteOffset") : 0, componentType, count, type,
            sparse, Bounds("min"), Bounds("max"));
    }

    private static AnimationDef ReadAnimation(JsonValue a)
    {
        var channels = new List<ChannelDef>();
        foreach (var c in Array(a, "channels"))
        {
            Object(c, "an animation channel");
            var target = Object(Field(c, "target") ?? throw new Invalid("channel.target is missing"), "channel.target");
            var path = OptionalString(target, "path") switch
            {
                "translation" => TargetPath.Translation,
                "rotation" => TargetPath.Rotation,
                "scale" => TargetPath.Scale,
                "weights" => TargetPath.Weights,
                var other => throw new Invalid($"channel.target.path {other ?? "(missing)"} is not a path"),
            };
            channels.Add(new(RequiredIndex(c, "sampler", "channel.sampler"), RequiredIndex(target, "node", "channel.target.node"), path));
        }
        var samplers = new List<SamplerDef>();
        foreach (var s in Array(a, "samplers"))
        {
            Object(s, "an animation sampler");
            var interpolation = OptionalString(s, "interpolation") switch
            {
                null or "LINEAR" => Interpolation.Linear,
                "STEP" => Interpolation.Step,
                "CUBICSPLINE" => Interpolation.CubicSpline,
                var other => throw new Invalid($"sampler.interpolation {other} is not an interpolation"),
            };
            samplers.Add(new(RequiredIndex(s, "input", "sampler.input"), RequiredIndex(s, "output", "sampler.output"), interpolation));
        }
        return new(OptionalString(a, "name"), channels, samplers);
    }

    private static TextureInfo? ReadTexture(JsonValue? t, string what)
    {
        if (t is null)
        {
            return null;
        }
        Object(t, what);
        return new(RequiredIndex(t, "index", $"{what}.index"), OptionalIndex(t, "texCoord", $"{what}.texCoord") ?? 0);
    }

    private static MaterialDef ReadMaterial(JsonValue m)
    {
        var pbr = Field(m, "pbrMetallicRoughness");
        if (pbr is not null)
        {
            Object(pbr, "material.pbrMetallicRoughness");
        }
        float Factor(string name) => pbr is not null && Field(pbr, name) is { } v ? Number(v, name) : 1f;
        return new(pbr is not null && Field(pbr, "baseColorFactor") is { } c ? Numbers(c, 4, "baseColorFactor") : [1, 1, 1, 1], Factor("metallicFactor"), Factor("roughnessFactor"),
            ReadTexture(pbr is null ? null : Field(pbr, "baseColorTexture"), "baseColorTexture"),
            ReadTexture(pbr is null ? null : Field(pbr, "metallicRoughnessTexture"), "metallicRoughnessTexture"), ReadTexture(Field(m, "normalTexture"), "normalTexture"));
    }

    /// <summary>Every index names something that is there.</summary>
    private void Validate()
    {
        foreach (var v in Views)
        {
            Check(v.Buffer, Buffers.Count, "buffer");
        }
        foreach (var a in Accessors)
        {
            Check(a.View, Views.Count, "buffer view");
            if (a.Sparse is { } s)
            {
                Check(s.IndicesView, Views.Count, "buffer view");
                Check(s.ValuesView, Views.Count, "buffer view");
            }
        }
        foreach (var p in Meshes.SelectMany(m => m.Primitives))
        {
            foreach (var index in p.Attributes.Values)
            {
                Check(index, Accessors.Count, "accessor");
            }
            Check(p.Indices, Accessors.Count, "accessor");
            Check(p.Material, Materials.Count, "material");
            // The specification asks for the bounds of every position accessor.
            if (p.Attributes.TryGetValue("POSITION", out var position) && (Accessors[position].Min is not { Length: 3 } || Accessors[position].Max is not { Length: 3 }))
            {
                throw new Invalid("a POSITION accessor has no min and max");
            }
        }
        foreach (var n in Nodes)
        {
            foreach (var c in n.Children)
            {
                Check(c, Nodes.Count, "node");
            }
            Check(n.Mesh, Meshes.Count, "mesh");
            Check(n.Skin, Skins.Count, "skin");
        }
        foreach (var s in Scenes.SelectMany(s => s))
        {
            Check(s, Nodes.Count, "node");
        }
        Check(Scene, Scenes.Count, "scene");
        foreach (var s in Skins)
        {
            foreach (var j in s.Joints)
            {
                Check(j, Nodes.Count, "node");
            }
            Check(s.InverseBindMatrices, Accessors.Count, "accessor");
        }
        foreach (var a in Animations)
        {
            foreach (var c in a.Channels)
            {
                Check(c.Sampler, a.Samplers.Count, "animation sampler");
                Check(c.Node, Nodes.Count, "node");
            }
            foreach (var s in a.Samplers)
            {
                Check(s.Input, Accessors.Count, "accessor");
                Check(s.Output, Accessors.Count, "accessor");
            }
        }
        foreach (var m in Materials)
        {
            foreach (var t in new[] { m.BaseColorTexture, m.MetallicRoughnessTexture, m.NormalTexture })
            {
                Check(t?.Index, Textures.Count, "texture");
            }
        }
        foreach (var t in Textures)
        {
            Check(t, Images.Count, "image");
        }
        foreach (var i in Images)
        {
            Check(i.View, Views.Count, "buffer view");
        }
    }
}
