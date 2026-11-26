using System.Numerics;

using Bayleaf.Core;

using AssetLoader = Bayleaf.Assets.Loader;

namespace Bayleaf.Assets.Gltf;

/// <summary>Reads glTF 2.0 models (<c>.glb</c>, or <c>.gltf</c> with its buffers beside it) into the engine's mesh data.</summary>
internal static class Loader
{
    private sealed class Refused(string message) : Exception(message);

    /// <summary>What a model's vertices are bound to, gathered as its parts are read.</summary>
    private sealed class Rig
    {
        public List<Binding> Bindings { get; } = [];

        /// <summary>Binding index by skin and joint, or (no skin, -1) by node.</summary>
        public Dictionary<(int Skin, int Joint), ushort> BindingOf { get; } = [];

        /// <summary>Each skin's inverse bind matrices.</summary>
        public Dictionary<int, Matrix4x4[]> Skins { get; } = [];

        public List<BindingSet> VertexBindings { get; } = [];

        public List<Vector4> VertexWeights { get; } = [];

        public List<Vector3> Positions { get; } = [];

        public List<Vector3> Normals { get; } = [];

        public ushort Bind((int Skin, int Joint) key, Func<Binding> binding)
        {
            if (BindingOf.TryGetValue(key, out var index))
            {
                return index;
            }
            if (Bindings.Count > ushort.MaxValue)
            {
                throw new Refused("the model has more than 65535 joints and parts");
            }
            index = (ushort)Bindings.Count;
            Bindings.Add(binding());
            BindingOf[key] = index;
            return index;
        }
    }

    /// <summary>Where a model's parts are gathered.</summary>
    private sealed class Gathered
    {
        public required Document Document { get; init; }

        public required Accessors Accessors { get; init; }

        public required Matrix4x4 Root { get; init; }

        public List<Vector3> Positions { get; } = [];

        public List<Vector3> Normals { get; } = [];

        public List<Vector2> Uvs { get; } = [];

        public List<uint> Indices { get; } = [];

        /// <summary>Runs of indices and the glTF material each uses.</summary>
        public List<(uint First, uint Count, int? Material)> Runs { get; } = [];

        public Rig Rig { get; } = new();
    }

    public static Result<MeshData, string> Load(string path)
    {
        var settings = ImportSettings.ReadFor(path);
        if (settings.IsError)
        {
            return settings.AsFailure();
        }
        var model = settings.Value?.Model ?? ModelImport.Default;
        var read = AssetLoader.ReadLimited(path);
        if (read.IsError)
        {
            return read.AsFailure();
        }
        var parsed = Document.Parse(read.Value.Bytes);
        if (parsed.IsError)
        {
            return Result.Fail($"{path}: not a glTF 2.0 model: {parsed.Error}");
        }
        var document = parsed.Value;
        var folder = Paths.Folder(path);
        var buffers = new List<byte[]>();
        for (var i = 0; i < document.Buffers.Count; i++)
        {
            var buffer = document.Buffers[i];
            byte[] data;
            if (buffer.Uri is null)
            {
                if (document.Blob is null)
                {
                    return Result.Fail($"{path}: the binary chunk is missing");
                }
                data = document.Blob;
            }
            else if (buffer.Uri.StartsWith("data:", StringComparison.Ordinal))
            {
                return Result.Fail($"{path}: buffers inside the file as text are not supported yet; save the model as .glb");
            }
            else
            {
                if (AssetLoader.BufferPath(folder, buffer.Uri) is not { } file)
                {
                    return Result.Fail($"{path}: buffer {buffer.Uri} must be a file next to the model");
                }
                var bytes = AssetLoader.ReadLimited(file);
                if (bytes.IsError)
                {
                    return bytes.AsFailure();
                }
                data = bytes.Value.Bytes;
            }
            if ((ulong)data.Length < buffer.ByteLength)
            {
                return Result.Fail($"{path}: buffer {i} is shorter than it says");
            }
            buffers.Add(data);
        }
        var id = AssetLoader.FileId(path, read.Value.Modified);
        int scene;
        if (document.Scene is { } s)
        {
            scene = s;
        }
        else if (document.Scenes.Count > 0)
        {
            scene = 0;
        }
        else
        {
            return Result.Fail($"{path}: the model has no scene");
        }
        // Z up, Y forward becomes Y up, Z toward the viewer.
        var up = model.UpAxis == UpAxis.Z ? Matrix4x4.CreateRotationX(-MathF.PI / 2) : Matrix4x4.Identity;
        var root = up * Matrix4x4.CreateScale(model.Scale);
        var gathered = new Gathered { Document = document, Accessors = new Accessors(document, buffers), Root = root };
        var warnings = new List<string>();
        Skeleton? skeleton = null;
        try
        {
            foreach (var node in document.Scenes[scene])
            {
                AddNode(node, Matrix4x4.Identity, gathered, 0);
            }
            if (document.Animations.Count > 0 || document.Skins.Count > 0)
            {
                skeleton = RigSkeleton(document, gathered, scene, warnings);
            }
        }
        catch (Refused e)
        {
            return Result.Fail($"{path}: {e.Message}");
        }
        var mesh = new MeshData
        {
            Id = id,
            Positions = [.. gathered.Positions],
            Normals = [.. gathered.Normals],
            Uvs = [.. gathered.Uvs],
            Indices = [.. gathered.Indices],
            Warnings = warnings,
        };
        if (skeleton is not null)
        {
            // Skinned parts stand as their joints hold them.
            var rest = skeleton.Posed(mesh, null, 0f, false, mesh.Id, 0);
            mesh = mesh with { Positions = rest.Positions, Normals = rest.Normals, Skeleton = skeleton };
        }
        var materials = new Dictionary<int, ModelMaterial>();
        var parts = new List<MeshPart>();
        foreach (var (first, count, index) in gathered.Runs)
        {
            ModelMaterial? material = null;
            if (index is { } i && i < document.Materials.Count)
            {
                if (!materials.TryGetValue(i, out material))
                {
                    var made = MaterialOf(document.Materials[i], document, buffers, folder, mesh.Id);
                    if (made.IsError)
                    {
                        return Result.Fail($"{path}: {made.Error}");
                    }
                    material = made.Value;
                    materials[i] = material;
                }
            }
            parts.Add(new(first, count, material));
        }
        if (mesh.Indices.Count == 0)
        {
            return Result.Fail($"{path}: the model has no triangles");
        }
        var (min, max) = MeshData.Bounds(mesh.Positions);
        return mesh with { Parts = parts, Material = parts.Select(p => p.Material).FirstOrDefault(m => m is not null), Min = min, Max = max };
    }

    /// <summary>A node's own matrix: the one it gives, or its translation, rotation and scale put together.</summary>
    private static Matrix4x4 NodeMatrix(NodeDef node)
    {
        if (node.Matrix is { } m)
        {
            return FMath.FromColumnMajor(m);
        }
        var (t, r, s) = (node.Translation, node.Rotation, node.Scale);
        return FMath.FromScaleRotationTranslation(new(s[0], s[1], s[2]), new(r[0], r[1], r[2], r[3]), new(t[0], t[1], t[2]));
    }

    /// <summary>A node's translation, rotation and scale: as it gives them, or taken from its matrix.</summary>
    private static (Vector3 Translation, Quaternion Rotation, Vector3 Scale) Decomposed(NodeDef node)
    {
        if (node.Matrix is not { } m)
        {
            var (t, r, s) = (node.Translation, node.Rotation, node.Scale);
            return (new(t[0], t[1], t[2]), new(r[0], r[1], r[2], r[3]), new(s[0], s[1], s[2]));
        }
        var (x, y, z) = (new Vector3(m[0], m[1], m[2]), new Vector3(m[4], m[5], m[6]), new Vector3(m[8], m[9], m[10]));
        var matrix = FMath.FromColumnMajor(m);
        var (sx, sy, sz) = (x.Length(), y.Length(), MathF.Sign(FMath.Determinant3(matrix)) * z.Length());
        var (cx, cy, cz) = (x / sx, y / sy, z / sz);
        // A rotation matrix's quaternion (Shepperd's method), from its columns.
        float m00 = cx.X, m10 = cx.Y, m20 = cx.Z, m01 = cy.X, m11 = cy.Y, m21 = cy.Z, m02 = cz.X, m12 = cz.Y, m22 = cz.Z;
        var trace = m00 + m11 + m22;
        Quaternion q;
        if (trace > 0)
        {
            var w = MathF.Sqrt(1 + trace) * 2;
            q = new((m21 - m12) / w, (m02 - m20) / w, (m10 - m01) / w, w / 4);
        }
        else if (m00 > m11 && m00 > m22)
        {
            var w = MathF.Sqrt(1 + m00 - m11 - m22) * 2;
            q = new(w / 4, (m01 + m10) / w, (m02 + m20) / w, (m21 - m12) / w);
        }
        else if (m11 > m22)
        {
            var w = MathF.Sqrt(1 + m11 - m00 - m22) * 2;
            q = new((m01 + m10) / w, w / 4, (m12 + m21) / w, (m02 - m20) / w);
        }
        else
        {
            var w = MathF.Sqrt(1 + m22 - m00 - m11) * 2;
            q = new((m02 + m20) / w, (m12 + m21) / w, w / 4, (m10 - m01) / w);
        }
        return (new(m[12], m[13], m[14]), q, new(sx, sy, sz));
    }

    private static void AddNode(int index, Matrix4x4 parent, Gathered gathered, int depth)
    {
        if (depth > 64)
        {
            throw new Refused("the node tree is too deep");
        }
        var (document, accessors) = (gathered.Document, gathered.Accessors);
        var node = document.Nodes[index];
        var local = NodeMatrix(node) * parent;
        var world = local * gathered.Root;
        var normalMatrix = FMath.NormalMatrix(world);
        var localNormals = FMath.NormalMatrix(local);
        if (node.Mesh is { } meshIndex)
        {
            foreach (var primitive in document.Meshes[meshIndex].Primitives)
            {
                if (primitive.Mode != 4)
                {
                    continue;
                }
                if (!primitive.Attributes.TryGetValue("POSITION", out var positionAccessor) || accessors.Vec3(positionAccessor) is not { } positions)
                {
                    continue;
                }
                if (gathered.Positions.Count + positions.Length > AssetLoader.MaxVertices)
                {
                    throw new Refused($"models may have at most {AssetLoader.MaxVertices} vertices");
                }
                var vertexBase = (uint)gathered.Positions.Count;
                var count = positions.Length;
                var indices = primitive.Indices is { } ia && accessors.Indices(ia) is { } read ? read : [.. Enumerable.Range(0, count).Select(i => (uint)i)];
                if (indices.Any(i => i >= count))
                {
                    throw new Refused("a triangle uses a vertex that does not exist");
                }
                var normals = primitive.Attributes.TryGetValue("NORMAL", out var na) && accessors.Vec3(na) is { } n ? n : AssetLoader.FlatNormals(positions, indices);
                var uvs = primitive.Attributes.TryGetValue("TEXCOORD_0", out var ta) && accessors.TexCoords(ta) is { } t ? t : new Vector2[count];
                if (normals.Length != count || uvs.Length != count)
                {
                    throw new Refused("a mesh part has a different number of normals or texture coordinates than positions");
                }
                gathered.Positions.AddRange(positions.Select(p => Vector3.Transform(p, world)));
                gathered.Normals.AddRange(normals.Select(v => FMath.NormalizeOrZero(Vector3.TransformNormal(v, normalMatrix))));
                gathered.Uvs.AddRange(uvs);
                var (first, indexCount) = ((uint)gathered.Indices.Count, (uint)indices.Length);
                gathered.Indices.AddRange(indices.Select(i => vertexBase + i));
                // Neighbouring parts with the same material draw as one.
                var runs = gathered.Runs;
                if (runs.Count > 0 && runs[^1].Material == primitive.Material && runs[^1].First + runs[^1].Count == first)
                {
                    runs[^1] = (runs[^1].First, runs[^1].Count + indexCount, runs[^1].Material);
                }
                else
                {
                    runs.Add((first, indexCount, primitive.Material));
                }
                (SkinDef Skin, int SkinIndex, ushort[][] Joints, Vector4[] Weights)? skinned = null;
                if (node.Skin is { } skinIndex && primitive.Attributes.TryGetValue("JOINTS_0", out var ja) && accessors.Joints(ja) is { } joints
                    && primitive.Attributes.TryGetValue("WEIGHTS_0", out var wa) && accessors.Weights(wa) is { } weights)
                {
                    skinned = (document.Skins[skinIndex], skinIndex, joints, weights);
                }
                BindPart(index, skinned, positions, normals, local, localNormals, gathered);
            }
        }
        foreach (var child in node.Children)
        {
            AddNode(child, local, gathered, depth + 1);
        }
    }

    /// <summary>What a part's vertices follow when the model moves: its skin's joints by their weights, or else its own node.</summary>
    private static void BindPart(int node, (SkinDef Skin, int SkinIndex, ushort[][] Joints, Vector4[] Weights)? skinned, Vector3[] positions, Vector3[] normals,
        Matrix4x4 local, Matrix4x4 localNormals, Gathered gathered)
    {
        var rig = gathered.Rig;
        var count = positions.Length;
        if (skinned is var (skin, skinIndex, joints, weights))
        {
            if (joints.Length != count || weights.Length != count)
            {
                throw new Refused("a skinned part has a different number of joints or weights than positions");
            }
            if (!rig.Skins.TryGetValue(skinIndex, out var inverses))
            {
                inverses = skin.InverseBindMatrices is { } ibm && gathered.Accessors.Matrices(ibm) is { } matrices ? [.. matrices.Select(m => FMath.FromColumnMajor(m))] : [];
                rig.Skins[skinIndex] = inverses;
            }
            for (var v = 0; v < count; v++)
            {
                var bound = new ushort[4];
                for (var slot = 0; slot < 4; slot++)
                {
                    int joint = joints[v][slot];
                    var inverse = joint < inverses.Length ? inverses[joint] : Matrix4x4.Identity;
                    if (joint >= skin.Joints.Count)
                    {
                        throw new Refused("a vertex names a joint its skin does not have");
                    }
                    var jointNode = skin.Joints[joint];
                    bound[slot] = rig.Bind((skinIndex, joint), () => new Binding(jointNode, inverse));
                }
                var set = weights[v];
                var total = 0f;
                for (var k = 0; k < 4; k++)
                {
                    if (float.IsFinite(set[k]) && set[k] > 0)
                    {
                        total += set[k];
                    }
                }
                static float Part(float w, float total) => float.IsFinite(w) && w > 0 ? w / total : 0f;
                rig.VertexBindings.Add(new(bound[0], bound[1], bound[2], bound[3]));
                rig.VertexWeights.Add(total > 0 ? new(Part(set.X, total), Part(set.Y, total), Part(set.Z, total), Part(set.W, total)) : Vector4.UnitX);
            }
            // A skinned part's vertices are in its skin's space: its own node does not move them.
            rig.Positions.AddRange(positions);
            rig.Normals.AddRange(normals);
        }
        else
        {
            var inverse = FMath.Inverse(local);
            var bound = rig.Bind((-1, node), () => new Binding(node, inverse));
            for (var v = 0; v < count; v++)
            {
                rig.VertexBindings.Add(new(bound, 0, 0, 0));
                rig.VertexWeights.Add(Vector4.UnitX);
            }
            rig.Positions.AddRange(positions.Select(p => Vector3.Transform(p, local)));
            rig.Normals.AddRange(normals.Select(n => FMath.NormalizeOrZero(Vector3.TransformNormal(n, localNormals))));
        }
    }

    /// <summary>The skeleton of a glTF model: its nodes at rest, the bindings its parts gathered, and its animations.</summary>
    private static Skeleton RigSkeleton(Document document, Gathered gathered, int scene, List<string> warnings)
    {
        var count = document.Nodes.Count;
        var parents = new int?[count];
        for (var i = 0; i < count; i++)
        {
            foreach (var child in document.Nodes[i].Children)
            {
                parents[child] = i;
            }
        }
        var nodes = new SkeletonNode[count];
        for (var i = 0; i < count; i++)
        {
            var (t, r, s) = Decomposed(document.Nodes[i]);
            nodes[i] = new(parents[i], t, FMath.Normalize(r), s);
        }
        // Parents before children: down from the scene's roots, then any node left over.
        var order = new List<int>(count);
        var seen = new bool[count];
        var stack = new List<int>(document.Scenes[scene]);
        stack.AddRange(Enumerable.Range(0, count).Where(i => nodes[i].Parent is null));
        stack.Reverse();
        while (stack.Count > 0)
        {
            var i = stack[^1];
            stack.RemoveAt(stack.Count - 1);
            if (i >= count || seen[i])
            {
                continue;
            }
            seen[i] = true;
            order.Add(i);
            stack.AddRange(document.Nodes[i].Children.Reverse());
        }
        var accessors = gathered.Accessors;
        var animations = new List<ModelAnimation>();
        var keys = 0L;
        for (var a = 0; a < document.Animations.Count; a++)
        {
            var animation = document.Animations[a];
            var name = animation.Name ?? $"Animation {a + 1}";
            var channels = new List<AnimationChannel>();
            var duration = 0f;
            foreach (var channel in animation.Channels)
            {
                var sampler = animation.Samplers[channel.Sampler];
                ChannelProperty property;
                switch (channel.Path)
                {
                    case TargetPath.Translation:
                        property = ChannelProperty.Translation;
                        break;
                    case TargetPath.Rotation:
                        property = ChannelProperty.Rotation;
                        break;
                    case TargetPath.Scale:
                        property = ChannelProperty.Scale;
                        break;
                    default:
                        warnings.Add($"{name}: shape blending (morph targets) is left out");
                        continue;
                }
                var times = accessors.Scalars(sampler.Input) ?? throw new Refused($"{name}: a channel has no key times");
                var read = property == ChannelProperty.Rotation
                    ? accessors.NormalizedVec4(sampler.Output)
                    : accessors.Vec3(sampler.Output)?.Select(v => new Vector4(v, 0)).ToArray();
                var values = read ?? throw new Refused($"{name}: a channel has no key values");
                if (sampler.Interpolation == Interpolation.CubicSpline)
                {
                    // In-tangent, value, out-tangent for each key: the values.
                    values = [.. values.Chunk(3).Where(c => c.Length > 1).Select(c => c[1])];
                }
                keys += times.Length;
                if (keys > Skeleton.MaxKeys)
                {
                    throw new Refused($"the animations have more than {Skeleton.MaxKeys} keys");
                }
                var ordered = true;
                for (var i = 1; i < times.Length; i++)
                {
                    ordered &= !(times[i - 1] > times[i]);
                }
                if (values.Length != times.Length || !ordered || times.Any(t => !float.IsFinite(t)))
                {
                    warnings.Add($"{name}: a channel with keys out of order or unmatched is left out");
                    continue;
                }
                duration = FMath.Max(duration, times.Length > 0 ? times[^1] : 0f);
                channels.Add(new(channel.Node, property, sampler.Interpolation == Interpolation.Step, times, values));
            }
            animations.Add(new(name, duration, channels));
        }
        var rig = gathered.Rig;
        return new Skeleton
        {
            Nodes = nodes,
            Order = order,
            Bindings = [.. rig.Bindings],
            VertexBindings = [.. rig.VertexBindings],
            VertexWeights = [.. rig.VertexWeights],
            Positions = [.. rig.Positions],
            Normals = [.. rig.Normals],
            Root = gathered.Root,
            Animations = animations,
            Names = [.. document.Nodes.Select((n, i) => n.Name ?? $"Node {i + 1}")],
        }.WithBounds();
    }

    /// <summary>The base colour, metallic, roughness and images of a glTF material.</summary>
    private static Result<ModelMaterial, string> MaterialOf(MaterialDef material, Document document, List<byte[]> buffers, string folder, ulong modelId)
    {
        Result<Image?, string> Read(TextureInfo? texture, string what)
        {
            // Images read by a second set of texture coordinates are left out.
            if (texture is not { TexCoord: 0 } t)
            {
                return (Image?)null;
            }
            var image = TextureImage(document.Textures[t.Index], document, buffers, folder, modelId);
            return image.IsOk ? image.Value : Result.Fail($"{what} image: {image.Error}");
        }
        var image = Read(material.BaseColorTexture, "base colour");
        if (image.IsError)
        {
            return image.AsFailure();
        }
        var metalRoughness = Read(material.MetallicRoughnessTexture, "metallic and roughness");
        if (metalRoughness.IsError)
        {
            return metalRoughness.AsFailure();
        }
        var normal = Read(material.NormalTexture, "normal");
        if (normal.IsError)
        {
            return normal.AsFailure();
        }
        static float Finite(float v, float fallback) => float.IsFinite(v) ? Math.Clamp(v, 0f, 1f) : fallback;
        var c = material.BaseColorFactor;
        return new ModelMaterial(new(Finite(c[0], 1f), Finite(c[1], 1f), Finite(c[2], 1f), Finite(c[3], 1f)), Finite(material.Metallic, 1f), Finite(material.Roughness, 1f),
            image.Value, metalRoughness.Value is { } mr ? AssetLoader.AsData(mr) : null, normal.Value is { } n ? AssetLoader.AsData(n) : null);
    }

    /// <summary>The image a glTF texture shows: inside the binary buffer, or a file beside the model.</summary>
    private static Result<Image, string> TextureImage(int source, Document document, List<byte[]> buffers, string folder, ulong modelId)
    {
        var image = document.Images[source];
        byte[] bytes;
        if (image.View is { } viewIndex)
        {
            var view = document.Views[viewIndex];
            if (view.Buffer >= buffers.Count)
            {
                return Result.Fail("its buffer is missing");
            }
            var buffer = buffers[view.Buffer];
            var end = view.Offset + view.Length;
            if (end < view.Offset || end > (ulong)buffer.Length)
            {
                return Result.Fail("it lies outside its buffer");
            }
            bytes = buffer.AsSpan((int)view.Offset, (int)view.Length).ToArray();
        }
        else if (image.Uri!.StartsWith("data:", StringComparison.Ordinal))
        {
            return Result.Fail("images inside the file as text are not supported yet; save the model as .glb");
        }
        else
        {
            if (AssetLoader.BufferPath(folder, image.Uri) is not { } file)
            {
                return Result.Fail($"{image.Uri} must be a file next to the model");
            }
            var read = AssetLoader.ReadLimited(file);
            if (read.IsError)
            {
                return read.AsFailure();
            }
            bytes = read.Value.Bytes;
        }
        var decoded = AssetLoader.DecodeImage(bytes);
        // Distinct from the model's id and from its other images.
        return decoded.IsOk ? decoded.Value with { Id = AssetLoader.ImageId(modelId, source) } : decoded;
    }
}
