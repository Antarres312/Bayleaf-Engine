using System.Numerics;
using System.Runtime.InteropServices;

using Bayleaf.Assets;
using Bayleaf.Native;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;

namespace Bayleaf.Render;

/// <summary>What a frame drew and what the renderer keeps on the GPU.</summary>
/// <param name="GpuMs">The GPU's own time for the newest frame it has measured, in milliseconds.</param>
/// <param name="GpuTimed">Whether the device measures GPU time at all.</param>
public readonly record struct RenderStats(uint DrawCalls, ulong Triangles, ulong MeshBytes, ulong ImageBytes, float? GpuMs, bool GpuTimed);

/// <summary>
/// Draws <see cref="Frame"/>s into <see cref="RenderTarget"/>s: the shadow maps, the scene (sky,
/// opaque shapes grouped into instanced draws, material outlines, the grid, then transparent shapes
/// back to front), post-processing, and the selection outline.
/// </summary>
/// <remarks>
/// The GPU's own time per frame is not measured: the wgpu-native this engine loads does not say how
/// long a timestamp tick is, so <see cref="RenderStats.GpuTimed"/> stays false.
/// </remarks>
public sealed partial class Renderer : IDisposable
{
    private readonly GpuDevice device;

    /// <summary>Samples per pixel the pipelines draw with; a target of another count makes them again.</summary>
    private uint samples;

    private GpuBuffer globals = null!;
    private GpuBindGroupLayout globalsLayout = null!;
    private GpuBindGroup globalsGroup = null!;
    private GpuPipelineLayout layout = null!;

    /// <summary>Opaque and transparent pipelines by material key; 0 is the built-in surface, null a material whose shader the GPU refused.</summary>
    private readonly Dictionary<ulong, (GpuPipeline Opaque, GpuPipeline Transparent)?> pipelines = [];

    /// <summary>Outline pipelines of the materials that have an outline, by material key.</summary>
    private readonly Dictionary<ulong, GpuPipeline> outlines = [];

    /// <summary>Additive pipelines by material key (0 the built-in surface).</summary>
    private readonly Dictionary<ulong, GpuPipeline> additive = [];

    private readonly List<GpuObject> kept = [];
    private Post post = null!;
    private GpuPipeline grid = null!;
    private GpuPipeline sky = null!;
    private readonly Dictionary<Shape, GpuMesh> meshes = [];
    private GpuBuffer instances = null!;
    private int instanceCapacity;

    /// <summary>The shadow map, its layers to draw into, and its size per side.</summary>
    private (GpuTexture Texture, GpuTextureView Whole, List<GpuTextureView> Layers, GpuBindGroup Group) shadowMap;
    private uint shadowSize;
    private GpuBindGroupLayout shadowLayout = null!;
    private GpuSampler shadowSampler = null!;
    private GpuBuffer shadowFaces = null!;
    private GpuBindGroup shadowFaceGroup = null!;
    private GpuPipeline shadowPipeline = null!;
    private GpuBindGroupLayout textureLayout = null!;
    private GpuPipeline maskPipeline = null!;
    private GpuPipeline outlinePipeline = null!;
    private GpuBindGroupLayout outlineLayout = null!;
    private GpuSampler textureSampler = null!;

    /// <summary>Uploaded images by id; 0 is a white pixel.</summary>
    private readonly Dictionary<ulong, GpuImage> textures = [];

    /// <summary>Texture bind groups by the images in their four slots and which slots read plain numbers.</summary>
    private readonly Dictionary<TextureKey, GpuBindGroup> textureGroups = [];

    /// <summary>Uploaded models by id.</summary>
    private readonly Dictionary<ulong, GpuMesh> models = [];

    /// <summary>Every skinned item's matrices this frame, four texels a matrix, <see cref="JointsPerRow"/> to a row.</summary>
    private GpuTexture joints = null!;
    private uint jointRows;

    /// <summary>A version for each frame's CPU-posed models.</summary>
    private ulong posedVersion;

    /// <summary>Draw calls and triangles of the last frame's scene.</summary>
    private (uint Calls, ulong Triangles) drawn;

    /// <summary>A renderer for targets of <paramref name="samples"/> per pixel; <see cref="Render"/> makes it again for a target of another count.</summary>
    public Renderer(GpuDevice device, uint samples = RenderTarget.DefaultSamples)
    {
        this.device = device;
        Build(samples);
        // Sixteen vertex inputs: the vertex's five (with its joints and weights) and the item's eleven.
        CpuSkinning = device.MaxVertexAttributes < 16;
    }

    /// <summary>Why the GPU refused the last material shader it was given, if it refused one.</summary>
    public string? MaterialError { get; private set; }

    /// <summary>
    /// Moves skinned models' vertices on the CPU and uploads them per item, instead of on the GPU: for
    /// devices without the vertex inputs GPU skinning needs (on from the start there), and for tests.
    /// </summary>
    public bool CpuSkinning { get; set; }

    private sealed class GpuMesh(GpuBuffer vertices, GpuBuffer indices, uint indexCount, ulong version) : IDisposable
    {
        public GpuBuffer Vertices { get; } = vertices;

        public GpuBuffer Indices { get; } = indices;

        public uint IndexCount { get; } = indexCount;

        /// <summary>The model's version its vertices are of.</summary>
        public ulong Version { get; set; } = version;

        public void Dispose()
        {
            Vertices.Dispose();
            Indices.Dispose();
        }
    }

    /// <summary>An image on the GPU: one texture, read through two views, as colour and as plain numbers.</summary>
    private sealed record GpuImage(GpuTexture Texture, GpuTextureView Color, GpuTextureView Raw, bool Owned = true) : IDisposable
    {
        public void Dispose()
        {
            if (Owned)
            {
                Color.Dispose();
                Raw.Dispose();
                Texture.Dispose();
            }
        }
    }

    /// <summary>The images in a bind group's four slots (0 for an empty slot), and a bit for each slot read as plain numbers.</summary>
    private readonly record struct TextureKey(ulong A, ulong B, ulong C, ulong D, byte Raw) : IComparable<TextureKey>
    {
        public ulong this[int slot] => slot switch
        {
            0 => A,
            1 => B,
            2 => C,
            _ => D,
        };

        public bool Contains(ulong id) => A == id || B == id || C == id || D == id;

        public int CompareTo(TextureKey other)
        {
            var c = A.CompareTo(other.A);
            c = c != 0 ? c : B.CompareTo(other.B);
            c = c != 0 ? c : C.CompareTo(other.C);
            c = c != 0 ? c : D.CompareTo(other.D);
            return c != 0 ? c : Raw.CompareTo(other.Raw);
        }
    }

    /// <summary>Texture id of the one-pixel flat normal map (image ids from files are odd, 0 is white).</summary>
    private const ulong FlatNormal = 2;

    private const GpuTextureFormat ShadowFormat = GpuTextureFormat.Depth32Float;

    /// <summary>Faces of a point light's shadow: six around it.</summary>
    private const int ShadowFacesPerPoint = 6;

    /// <summary>Most lights that cast shadows at once, and most shadow-map layers they use together.</summary>
    private const int MaxShadowLights = 4;

    private const int MaxShadowLayers = MaxShadowLights * ShadowFacesPerPoint;

    /// <summary>Bytes between face matrices in the face buffer (the uniform offset alignment).</summary>
    private const ulong FaceStride = 256;

    /// <summary>Matrices in each row of the joint texture (four texels each).</summary>
    private const uint JointsPerRow = 256;

    /// <summary>Most rows the joint texture grows to: 2048, the smallest texture side every device takes.</summary>
    private const uint MaxJointRows = 2048;

    /// <summary>Floats of the per-frame values (common.wgsl's Globals).</summary>
    private const int GlobalsFloats = 652;

    /// <summary>Floats of one item's instance data.</summary>
    private const int InstanceFloats = 44;

    private static readonly GpuVertexBuffer VertexLayout = new(56, false,
    [
        new(GpuVertexFormat.Float32x3, 0, 0),
        new(GpuVertexFormat.Float32x3, 12, 1),
        new(GpuVertexFormat.Float32x2, 24, 11),
        new(GpuVertexFormat.Uint16x4, 32, 13),
        new(GpuVertexFormat.Float32x4, 40, 14),
    ]);

    private static readonly GpuVertexBuffer InstanceLayout = new(InstanceFloats * sizeof(float), true,
    [
        new(GpuVertexFormat.Float32x4, 0, 2), new(GpuVertexFormat.Float32x4, 16, 3), new(GpuVertexFormat.Float32x4, 32, 4), new(GpuVertexFormat.Float32x4, 48, 5),
        new(GpuVertexFormat.Float32x4, 64, 6), new(GpuVertexFormat.Float32x4, 80, 7), new(GpuVertexFormat.Float32x4, 96, 8),
        new(GpuVertexFormat.Float32x4, 112, 9), new(GpuVertexFormat.Float32x4, 128, 10), new(GpuVertexFormat.Float32x4, 144, 12), new(GpuVertexFormat.Float32x4, 160, 15),
    ]);

    private T Keep<T>(T value)
        where T : GpuObject
    {
        kept.Add(value);
        return value;
    }

    /// <summary>Makes everything the renderer keeps for targets of <paramref name="count"/> samples per pixel.</summary>
    private void Build(uint count)
    {
        samples = count <= 1 ? 1u : 4u;
        globals = Keep(device.CreateBuffer("bayleaf globals", GlobalsFloats * sizeof(float), GpuBufferUsage.Uniform | GpuBufferUsage.CopyDestination));
        globalsLayout = Keep(device.CreateBindGroupLayout("bayleaf globals layout",
        [
            new GpuUniformLayout(0, GpuStages.Vertex | GpuStages.Fragment),
            new GpuTextureLayout(1, GpuStages.Vertex, GpuSampleType.Unfilterable),
        ]));
        jointRows = 1;
        joints = JointTexture(1);
        globalsGroup = GlobalsGroup();
        // Each layer's matrix, at its own offset, for the shadow pass.
        shadowFaces = Keep(device.CreateBuffer("bayleaf shadow faces", FaceStride * MaxShadowLayers, GpuBufferUsage.Uniform | GpuBufferUsage.CopyDestination));
        var faceLayout = Keep(device.CreateBindGroupLayout("bayleaf shadow face layout", [new GpuUniformLayout(0, GpuStages.Vertex, true, 64)]));
        shadowFaceGroup = Keep(device.CreateBindGroup("bayleaf shadow face group", faceLayout, [new GpuBufferBinding(0, shadowFaces, 0, 64)]));
        shadowSampler = Keep(device.CreateSampler(new GpuSamplerDesc("bayleaf shadow sampler") { Linear = true, Compare = true }));
        shadowLayout = Keep(device.CreateBindGroupLayout("bayleaf shadow layout",
        [
            new GpuTextureLayout(0, GpuStages.Fragment, GpuSampleType.Depth, GpuViewDimension.D2Array),
            new GpuSamplerLayout(1, GpuStages.Fragment, GpuSamplerType.Comparison),
        ]));
        // A small map for now; the first frame makes it the size the frame's settings ask for.
        shadowMap = ShadowMap(1, 1);
        shadowSize = 1;
        textureLayout = Keep(device.CreateBindGroupLayout("bayleaf texture layout",
        [
            new GpuTextureLayout(0, GpuStages.Fragment, GpuSampleType.Filterable),
            new GpuTextureLayout(1, GpuStages.Fragment, GpuSampleType.Filterable),
            new GpuTextureLayout(2, GpuStages.Fragment, GpuSampleType.Filterable),
            new GpuTextureLayout(3, GpuStages.Fragment, GpuSampleType.Filterable),
            new GpuSamplerLayout(4, GpuStages.Fragment, GpuSamplerType.Filtering),
        ]));
        textureSampler = Keep(device.CreateSampler(new GpuSamplerDesc("bayleaf texture sampler") { Repeat = true, Linear = true, LinearMipmaps = true, Anisotropy = 8 }));
        layout = Keep(device.CreatePipelineLayout("bayleaf pipeline layout", [globalsLayout, shadowLayout, textureLayout]));
        var shadowPassLayout = Keep(device.CreatePipelineLayout("bayleaf shadow pass layout", [globalsLayout, faceLayout]));
        var shadowShader = Keep(device.CreateShader("bayleaf shadow shader", Shaders.Shadow));
        // Depth only. Back faces are kept: a closed shape's far side is what keeps light from leaking under it.
        shadowPipeline = Keep(device.CreatePipeline(new GpuPipelineDesc("bayleaf shadow", shadowPassLayout, shadowShader, "vs_main", [VertexLayout, InstanceLayout])
        {
            Depth = new GpuDepthState(ShadowFormat, true, GpuCompare.LessEqual, 2, 2f),
        }));
        var gridShader = Keep(device.CreateShader("bayleaf grid shader", Shaders.Grid));
        var meshShader = Keep(device.CreateShader("bayleaf mesh shader", Shaders.Mesh(Shaders.DefaultSurface)));
        pipelines[0] = (Keep(MeshPipeline(meshShader, Kind.Opaque)), Keep(MeshPipeline(meshShader, Kind.Transparent)));
        // Pushed back a little, so a surface lying on the ground plane always covers the grid instead of flickering with it.
        grid = Keep(device.CreatePipeline(new GpuPipelineDesc("bayleaf grid", layout, gridShader, "vs_main", [])
        {
            FragmentEntry = "fs_main",
            Target = new GpuColorTarget(RenderTarget.HdrFormat, GpuBlend.Alpha),
            Depth = new GpuDepthState(RenderTarget.DepthFormat, false, GpuCompare.LessEqual, 64, 2f),
            Samples = samples,
        }));
        var skyShader = Keep(device.CreateShader("bayleaf sky shader", Shaders.Sky));
        // The sky: drawn first, behind everything, without touching the depth buffer.
        sky = Keep(device.CreatePipeline(new GpuPipelineDesc("bayleaf sky", layout, skyShader, "vs_main", [])
        {
            FragmentEntry = "fs_main",
            Target = new GpuColorTarget(RenderTarget.HdrFormat, GpuBlend.Replace),
            Depth = new GpuDepthState(RenderTarget.DepthFormat, false, GpuCompare.Always),
            Samples = samples,
        }));
        OutlinePipelines();
        post = new Post(device, globalsLayout, samples);
        additive[0] = Keep(MeshPipeline(meshShader, Kind.Additive));
        foreach (var shape in Enum.GetValues<Shape>())
        {
            var (vertices, indices) = Shapes.Build(shape);
            meshes[shape] = new GpuMesh(
                device.CreateBuffer("bayleaf vertices", MemoryMarshal.AsBytes(vertices.AsSpan()), GpuBufferUsage.Vertex),
                device.CreateBuffer("bayleaf indices", MemoryMarshal.AsBytes(indices.AsSpan()), GpuBufferUsage.Index),
                (uint)indices.Length,
                0);
        }
        instanceCapacity = 256;
        instances = InstanceBuffer(instanceCapacity);
    }

    /// <summary>Lets go of everything <see cref="Build"/> and the frames made.</summary>
    private void ReleaseAll()
    {
        post.Dispose();
        foreach (var image in textures.Values)
        {
            image.Dispose();
        }
        textures.Clear();
        foreach (var group in textureGroups.Values)
        {
            group.Dispose();
        }
        textureGroups.Clear();
        foreach (var mesh in models.Values.Concat(meshes.Values))
        {
            mesh.Dispose();
        }
        models.Clear();
        meshes.Clear();
        foreach (var pair in pipelines.Values)
        {
            if (pair is { } p && !kept.Contains(p.Opaque))
            {
                p.Opaque.Dispose();
                p.Transparent.Dispose();
            }
        }
        pipelines.Clear();
        foreach (var p in outlines.Values.Concat(additive.Values).Where(p => !kept.Contains(p)))
        {
            p.Dispose();
        }
        outlines.Clear();
        additive.Clear();
        ReleaseShadowMap();
        instances.Dispose();
        joints.Dispose();
        globalsGroup.Dispose();
        foreach (var o in Enumerable.Reverse(kept))
        {
            o.Dispose();
        }
        kept.Clear();
    }

    public void Dispose() => ReleaseAll();

    private GpuBuffer InstanceBuffer(int capacity) => device.CreateBuffer("bayleaf instances", (ulong)(capacity * InstanceFloats * sizeof(float)), GpuBufferUsage.Vertex | GpuBufferUsage.CopyDestination);

    private GpuTexture JointTexture(uint rows) => device.CreateTexture(new GpuTextureDesc("bayleaf joints", JointsPerRow * 4, Math.Max(rows, 1), GpuTextureFormat.Rgba32Float, GpuTextureUsage.Binding | GpuTextureUsage.CopyDestination));

    /// <summary>The per-frame group: the globals and the joint texture.</summary>
    private GpuBindGroup GlobalsGroup()
    {
        using var view = joints.CreateView();
        return device.CreateBindGroup("bayleaf globals group", globalsLayout, [new GpuBufferBinding(0, globals), new GpuViewBinding(1, view)]);
    }

    /// <summary>A shadow map of <paramref name="layers"/> layers, <paramref name="size"/> texels a side, read with the comparison sampler, and a view of each layer to draw into.</summary>
    private (GpuTexture, GpuTextureView, List<GpuTextureView>, GpuBindGroup) ShadowMap(uint size, uint layers)
    {
        layers = Math.Max(layers, 1);
        var texture = device.CreateTexture(new GpuTextureDesc("bayleaf shadow map", size, size, ShadowFormat, GpuTextureUsage.RenderTarget | GpuTextureUsage.Binding) { Layers = layers });
        var whole = texture.CreateView(new GpuViewDesc(Dimension: GpuViewDimension.D2Array));
        var views = Enumerable.Range(0, (int)layers).Select(layer => texture.CreateView(new GpuViewDesc(Dimension: GpuViewDimension.D2, BaseLayer: (uint)layer, Layers: 1))).ToList();
        var group = device.CreateBindGroup("bayleaf shadow group", shadowLayout, [new GpuViewBinding(0, whole), new GpuSamplerBinding(1, shadowSampler)]);
        return (texture, whole, views, group);
    }

    private void ReleaseShadowMap()
    {
        shadowMap.Group?.Dispose();
        foreach (var view in shadowMap.Layers ?? [])
        {
            view.Dispose();
        }
        shadowMap.Whole?.Dispose();
        shadowMap.Texture?.Dispose();
    }

    private enum Kind
    {
        Opaque,
        Transparent,
        Outline,
        Additive,
    }

    private GpuPipeline MeshPipeline(GpuShader shader, Kind kind)
    {
        var transparent = kind is Kind.Transparent or Kind.Additive;
        var outline = kind == Kind.Outline;
        return device.CreatePipeline(new GpuPipelineDesc(outline ? "bayleaf material outline" : transparent ? "bayleaf transparent" : "bayleaf opaque", layout, shader, outline ? "vs_outline" : "vs_main",
            [VertexLayout, InstanceLayout])
        {
            FragmentEntry = outline ? "fs_outline" : "fs_main",
            // No culling: quads and planes are visible from both sides; the shader flips back-face
            // normals. An outline draws only the back faces.
            Cull = outline ? GpuCull.Front : GpuCull.None,
            Depth = new GpuDepthState(RenderTarget.DepthFormat, !transparent, GpuCompare.LessEqual),
            Samples = samples,
            Target = new GpuColorTarget(RenderTarget.HdrFormat, kind switch
            {
                Kind.Additive => GpuBlend.Additive,
                Kind.Transparent => GpuBlend.Alpha,
                _ => GpuBlend.Replace,
            }),
        });
    }

    /// <summary>The selection outline's two pipelines (the selected shapes into the mask; the band around it) and the layout of the mask's bind group.</summary>
    private void OutlinePipelines()
    {
        var shader = Keep(device.CreateShader("bayleaf outline shader", Shaders.Outline));
        outlineLayout = Keep(device.CreateBindGroupLayout("bayleaf selection mask layout", [new GpuTextureLayout(0, GpuStages.Fragment, GpuSampleType.Unfilterable)]));
        var maskPassLayout = Keep(device.CreatePipelineLayout("bayleaf selection mask pass layout", [globalsLayout]));
        var outlinePassLayout = Keep(device.CreatePipelineLayout("bayleaf outline pass layout", [globalsLayout, outlineLayout]));
        maskPipeline = Keep(device.CreatePipeline(new GpuPipelineDesc("bayleaf selection mask", maskPassLayout, shader, "vs_mask", [VertexLayout, InstanceLayout])
        {
            FragmentEntry = "fs_mask",
            Target = new GpuColorTarget(RenderTarget.MaskFormat, GpuBlend.Replace),
        }));
        outlinePipeline = Keep(device.CreatePipeline(new GpuPipelineDesc("bayleaf outline", outlinePassLayout, shader, "vs_screen", [])
        {
            FragmentEntry = "fs_outline",
            Target = new GpuColorTarget(RenderTarget.ColorFormat, GpuBlend.Alpha),
        }));
    }

    /// <summary>What the last frame drew and what the renderer keeps on the GPU, for the profiler.</summary>
    public RenderStats Stats()
    {
        var meshBytes = meshes.Values.Concat(models.Values).Aggregate(0UL, (sum, m) => sum + m.Vertices.Size + m.Indices.Size);
        var imageBytes = textures.Values.Aggregate(0UL, (sum, image) =>
        {
            var desc = image.Texture.Desc;
            // The first level's bytes (a quarter or an eighth of four a pixel when compressed), and a third more for the smaller copies.
            var (side, bytes) = desc.Format switch
            {
                GpuTextureFormat.Bc1RgbaUnorm or GpuTextureFormat.Bc1RgbaUnormSrgb => (4u, 8u),
                GpuTextureFormat.Bc3RgbaUnorm or GpuTextureFormat.Bc3RgbaUnormSrgb => (4u, 16u),
                _ => (1u, 4u),
            };
            var first = (ulong)((desc.Width + side - 1) / side) * ((desc.Height + side - 1) / side) * bytes;
            return sum + (desc.MipLevels > 1 ? first * 4 / 3 : first);
        });
        return new RenderStats(drawn.Calls, drawn.Triangles, meshBytes, imageBytes, null, false);
    }

    /// <summary>Whether the GPU refused <paramref name="material"/>'s shader, so its objects show the built-in surface.</summary>
    public bool Refused(CompiledMaterial material) => pipelines.TryGetValue(material.Key, out var pair) && pair is null;

    /// <summary>
    /// Builds the pipelines of a material the first time it is drawn. A shader the GPU refuses is
    /// remembered as refused, so it is tried once, and its objects use the built-in surface.
    /// </summary>
    private void Prepare(CompiledMaterial material)
    {
        if (pipelines.ContainsKey(material.Key))
        {
            return;
        }
        device.PushErrorScope();
        using var shader = device.CreateShader("bayleaf material shader", Shaders.Mesh(material.Surface));
        var pair = (MeshPipeline(shader, Kind.Opaque), MeshPipeline(shader, Kind.Transparent));
        var outline = material.Outline is not null ? MeshPipeline(shader, Kind.Outline) : null;
        var add = MeshPipeline(shader, Kind.Additive);
        // wgpu-native answers before the call returns, so the material is settled now.
        if (device.PopErrorScope() is { } refused)
        {
            pair.Item1.Dispose();
            pair.Item2.Dispose();
            outline?.Dispose();
            add.Dispose();
            pipelines[material.Key] = null;
            MaterialError = refused;
            return;
        }
        pipelines[material.Key] = pair;
        if (outline is not null)
        {
            outlines[material.Key] = outline;
        }
        additive[material.Key] = add;
    }

    /// <summary>
    /// Draws the texture <paramref name="id"/> for items whose image has that id, instead of uploading
    /// their image: a picture drawn on the GPU each frame. Image groups made with an earlier texture of
    /// that id are made again. The texture stays the caller's.
    /// </summary>
    public void SetTexture(ulong id, GpuTexture texture, GpuTextureView view)
    {
        if (textures.Remove(id, out var old))
        {
            old.Dispose();
        }
        textures[id] = new GpuImage(texture, view, view, false);
        foreach (var key in textureGroups.Keys.Where(k => k.Contains(id)).ToList())
        {
            textureGroups.Remove(key, out var group);
            group?.Dispose();
        }
    }

    private static readonly Image White = new(0, 1, 1, [255, 255, 255, 255]);

    /// <summary>A normal straight out of the surface: (0.5, 0.5, 1) read as plain numbers.</summary>
    private static readonly Image Flat = new(FlatNormal, 1, 1, [128, 128, 255, 255]) { Data = true };

    /// <summary>Puts the frame's images and models on the GPU the first time they are drawn, and makes the image groups its items need.</summary>
    private void UploadAssets(Frame frame)
    {
        if (!textures.ContainsKey(0))
        {
            textures[0] = UploadImage(White);
        }
        if (!textures.ContainsKey(FlatNormal))
        {
            textures[FlatNormal] = UploadImage(Flat);
        }
        var keys = new List<TextureKey> { default };
        foreach (var item in frame.Items)
        {
            foreach (var image in item.Textures)
            {
                if (image is not null && !textures.ContainsKey(image.Id))
                {
                    textures[image.Id] = UploadImage(image);
                }
            }
            if (item.Mesh is { } mesh)
            {
                if (models.TryGetValue(mesh.Id, out var gpu) && gpu.Version == mesh.Version)
                {
                }
                else if (gpu is not null && gpu.Vertices.Size == (ulong)(mesh.Positions.Count * 56))
                {
                    // A posed model: the same vertices moved, written over the old ones.
                    device.WriteBuffer(gpu.Vertices, 0, MemoryMarshal.AsBytes(ModelVertices(mesh).AsSpan()));
                    gpu.Version = mesh.Version;
                }
                else
                {
                    gpu?.Dispose();
                    models[mesh.Id] = UploadModel(mesh);
                }
            }
            keys.Add(KeyOf(item));
        }
        foreach (var key in keys)
        {
            if (textureGroups.ContainsKey(key))
            {
                continue;
            }
            GpuTextureView View(int slot)
            {
                var image = textures.TryGetValue(key[slot], out var found) ? found : textures[0];
                return (key.Raw & (1 << slot)) == 0 ? image.Color : image.Raw;
            }
            textureGroups[key] = device.CreateBindGroup("bayleaf images", textureLayout,
            [
                new GpuViewBinding(0, View(0)),
                new GpuViewBinding(1, View(1)),
                new GpuViewBinding(2, View(2)),
                new GpuViewBinding(3, View(3)),
                new GpuSamplerBinding(4, textureSampler),
            ]);
        }
    }

    /// <summary>
    /// An image on the GPU, with its smaller copies so it does not shimmer far away (unless its import
    /// settings turn them off): its 4 x 4 blocks when it has them and the device reads them, else its
    /// pixels. Its colour view reads the stored numbers as sRGB colour, its raw view as they are.
    /// </summary>
    private GpuImage UploadImage(Image image)
    {
        GpuTextureFormat raw, color;
        uint levels;
        byte[] data;
        if (image.Blocks is { } blocks && device.BlockCompression)
        {
            (raw, color) = blocks.Compression == Compression.Bc3 ? (GpuTextureFormat.Bc3RgbaUnorm, GpuTextureFormat.Bc3RgbaUnormSrgb) : (GpuTextureFormat.Bc1RgbaUnorm, GpuTextureFormat.Bc1RgbaUnormSrgb);
            levels = (uint)blocks.Levels.Count;
            data = [.. blocks.Levels.SelectMany(l => l)];
        }
        else
        {
            var mips = image.Mipmaps ? Loader.MipLevels(image) : [];
            (raw, color) = (GpuTextureFormat.Rgba8Unorm, GpuTextureFormat.Rgba8UnormSrgb);
            levels = (uint)mips.Count + 1;
            data = [.. image.Rgba, .. mips.SelectMany(m => m.Rgba)];
        }
        var texture = device.CreateTexture(new GpuTextureDesc("bayleaf image", (uint)Math.Max(image.Width, 1), (uint)Math.Max(image.Height, 1), raw, GpuTextureUsage.Binding)
        {
            MipLevels = Math.Max(levels, 1),
            ViewFormat = color,
        }, data);
        return new GpuImage(texture, texture.CreateView(new GpuViewDesc(color)), texture.CreateView(new GpuViewDesc(raw)));
    }

    /// <summary>A model on the GPU.</summary>
    private GpuMesh UploadModel(MeshData mesh)
    {
        // Written again when an animation poses the model.
        var vertices = device.CreateBuffer("bayleaf model vertices", MemoryMarshal.AsBytes(ModelVertices(mesh).AsSpan()), GpuBufferUsage.Vertex | GpuBufferUsage.CopyDestination);
        var indices = device.CreateBuffer("bayleaf model indices", MemoryMarshal.AsBytes(mesh.Indices.ToArray().AsSpan()), GpuBufferUsage.Index);
        return new GpuMesh(vertices, indices, (uint)mesh.Indices.Count, mesh.Version);
    }

    /// <summary>A model's vertices: for one skinned on the GPU, in the bound pose with their joints and weights; else as the model holds them.</summary>
    private static Vertex[] ModelVertices(MeshData mesh)
    {
        Vector2 Uv(int i) => i < mesh.Uvs.Count ? mesh.Uvs[i] : Vector2.Zero;
        var vertices = new Vertex[mesh.Positions.Count];
        if (GpuSkeleton(mesh) is { } skeleton)
        {
            for (var i = 0; i < vertices.Length; i++)
            {
                var bound = i < skeleton.VertexBindings.Count ? skeleton.VertexBindings[i] : default;
                vertices[i] = new Vertex
                {
                    Position = skeleton.Positions[i],
                    Normal = i < skeleton.Normals.Count ? skeleton.Normals[i] : Vector3.UnitY,
                    Uv = Uv(i),
                    J0 = bound.B0,
                    J1 = bound.B1,
                    J2 = bound.B2,
                    J3 = bound.B3,
                    Weights = i < skeleton.VertexWeights.Count ? skeleton.VertexWeights[i] : Vector4.UnitX,
                };
            }
            return vertices;
        }
        for (var i = 0; i < vertices.Length; i++)
        {
            vertices[i] = Vertex.Plain(mesh.Positions[i], i < mesh.Normals.Count ? mesh.Normals[i] : Vector3.UnitY, Uv(i));
        }
        return vertices;
    }

    /// <summary>The skeleton a model is skinned by on the GPU: one whose bound-pose vertices match the model's.</summary>
    private static Skeleton? GpuSkeleton(MeshData mesh) =>
        mesh.Skeleton is { } s && s.Positions.Count == mesh.Positions.Count && s.Bindings.Count > 0 ? s : null;

    /// <summary><paramref name="frame"/> with each skinned model posed on the CPU: a mesh of its own per object, drawn as it is.</summary>
    private Frame PoseOnCpu(Frame frame)
    {
        posedVersion++;
        var posed = new Dictionary<ulong, MeshData>();
        var items = new List<DrawItem>(frame.Items.Count);
        foreach (var item in frame.Items)
        {
            if (item.Mesh is not { Skeleton: { } skeleton } mesh)
            {
                items.Add(item);
                continue;
            }
            // Its own id, so each object's pose is its own mesh; its parts share it.
            var id = Mix(mesh.Id, item.ObjectId.GetHashCode()) | 1;
            if (!posed.TryGetValue(id, out var own))
            {
                var matrices = item.Skin?.Matrices ?? skeleton.SkinMatrices(skeleton.PoseAt(null, 0));
                own = skeleton.SkinOnCpu(mesh, matrices, id, posedVersion) with { Skeleton = null };
                posed[id] = own;
            }
            items.Add(item with { Mesh = own, Skin = null });
        }
        return frame with { Items = items };
    }

    private static ulong Mix(ulong a, int b)
    {
        var x = a ^ ((ulong)(uint)b * 0x9E3779B97F4A7C15UL);
        x ^= x >> 33;
        x *= 0xFF51AFD7ED558CCDUL;
        return x ^ (x >> 33);
    }

    /// <summary>
    /// Writes the matrices of every skinned item into the joint texture (each skin once, however many
    /// items share it), growing it as needed; gives each item the first of its matrices there, or null
    /// for an item not skinned on the GPU. A model without a skin of its own stands at rest.
    /// </summary>
    private List<uint?> WriteJoints(List<DrawItem> items)
    {
        var data = new List<float>();
        var written = new Dictionary<Skin, uint>(ReferenceEqualityComparer.Instance);
        var atRest = new Dictionary<ulong, uint>();
        var capacity = (int)(MaxJointRows * JointsPerRow);
        var firsts = new List<uint?>(items.Count);
        foreach (var item in items)
        {
            if (item.Mesh is not { } mesh || GpuSkeleton(mesh) is not { } skeleton)
            {
                firsts.Add(null);
                continue;
            }
            var count = skeleton.Bindings.Count;
            var first = (uint)(data.Count / 16);
            uint? known = item.Skin is { } skin ? (written.TryGetValue(skin, out var k) ? k : null) : (atRest.TryGetValue(mesh.Id, out var r) ? r : null);
            if (known is not null)
            {
                firsts.Add(known);
                continue;
            }
            if (data.Count / 16 + count > capacity)
            {
                firsts.Add(null);
                continue;
            }
            IReadOnlyList<Matrix4x4> matrices;
            if (item.Skin is { } own)
            {
                written[own] = first;
                matrices = own.Matrices;
            }
            else
            {
                atRest[mesh.Id] = first;
                matrices = skeleton.SkinMatrices(skeleton.PoseAt(null, 0));
            }
            // Exactly one matrix per binding, so no vertex reads another skin's.
            var m = new float[16];
            for (var i = 0; i < count; i++)
            {
                FMat.Write(m, i < matrices.Count ? matrices[i] : skeleton.Root);
                foreach (var v in m)
                {
                    data.Add(v);
                }
            }
            firsts.Add(first);
        }
        if (data.Count == 0)
        {
            return firsts;
        }
        var rows = (uint)((data.Count / 16 + (int)JointsPerRow - 1) / (int)JointsPerRow);
        if (rows > jointRows)
        {
            jointRows = Math.Min(System.Numerics.BitOperations.RoundUpToPowerOf2(rows), MaxJointRows);
            joints.Dispose();
            joints = JointTexture(jointRows);
            globalsGroup.Dispose();
            globalsGroup = GlobalsGroup();
        }
        var floats = new float[rows * JointsPerRow * 16];
        data.CopyTo(floats);
        device.WriteTexture(joints, 0, MemoryMarshal.AsBytes(floats.AsSpan()), JointsPerRow * 4 * 16, rows, JointsPerRow * 4, rows);
        return firsts;
    }

    /// <summary>The GPU mesh an item draws.</summary>
    private GpuMesh? MeshOf(DrawItem item) => item.Mesh is { } mesh ? models.GetValueOrDefault(mesh.Id) : meshes.GetValueOrDefault(item.Shape);

    private GpuPipeline? Pipeline(ulong key, bool transparent)
    {
        var pair = pipelines.TryGetValue(key, out var found) && found is { } p ? p : pipelines.TryGetValue(0, out var builtIn) ? builtIn : null;
        return pair is { } chosen ? (transparent ? chosen.Transparent : chosen.Opaque) : null;
    }

    private static ulong MaterialKey(DrawItem item) => item.Material?.Key ?? 0;

    /// <summary>Which mesh an item draws: a built-in shape (its index) or a model (its id, with the top bit set).</summary>
    private static ulong MeshKey(DrawItem item) => item.Mesh is { } mesh ? mesh.Id | (1UL << 63) : (ulong)item.Shape;

    /// <summary>The images in an item's four texture slots; 0 for an empty or missing slot.</summary>
    private static TextureKey KeyOf(DrawItem item)
    {
        // The built-in surface reads a normal map in slot 2, so an empty one there is flat, not white;
        // its slots 1 and 2 (metallic and roughness, normals) hold plain numbers whatever the image says.
        var builtIn = item.Material is null;
        Span<ulong> ids = builtIn ? [0, 0, FlatNormal, 0] : [0, 0, 0, 0];
        var raw = builtIn ? 0b0110 : 0;
        for (var slot = 0; slot < Math.Min(4, item.Textures.Count); slot++)
        {
            if (item.Textures[slot] is { } image)
            {
                ids[slot] = image.Id;
                if (image.Data)
                {
                    raw |= 1 << slot;
                }
            }
        }
        return new TextureKey(ids[0], ids[1], ids[2], ids[3], (byte)raw);
    }

    private static int ComparePart((uint First, uint Count)? a, (uint First, uint Count)? b) => (a, b) switch
    {
        (null, null) => 0,
        (null, _) => -1,
        (_, null) => 1,
        ({ } x, { } y) => x.First != y.First ? x.First.CompareTo(y.First) : x.Count.CompareTo(y.Count),
    };

    /// <summary>The indices an item draws: its part of the mesh, or all of it.</summary>
    private static (uint First, uint Count) Indices(DrawItem item, GpuMesh mesh)
    {
        if (item.Part is not { } part)
        {
            return (0, mesh.IndexCount);
        }
        var first = Math.Min(part.First, mesh.IndexCount);
        var end = (uint)Math.Min((ulong)part.First + part.Count, mesh.IndexCount);
        return (first, end > first ? end - first : 0);
    }

    /// <summary>Records and submits the commands that draw <paramref name="frame"/> into <paramref name="target"/>.</summary>
    public void Render(RenderTarget target, Frame frame)
    {
        // Pipelines are made for one sample count: a target of another (multisampling turned on or
        // off) needs them made again, and the images and models are uploaded again with them.
        if (target.Samples != samples)
        {
            ReleaseAll();
            Build(target.Samples);
        }
        if (CpuSkinning)
        {
            frame = PoseOnCpu(frame);
        }
        // The shadow map as large as the settings ask, with a layer for each face the frame's
        // shadow-casting lights use (a sun one per cascade, a point light six).
        var settings = frame.Shadows.Clamped();
        var shadows = ShadowViews(frame, settings);
        var layers = Math.Clamp(shadows.Sum(v => v.Faces.Count), 1, MaxShadowLayers);
        if (shadowSize != settings.Resolution || shadowMap.Layers.Count != layers)
        {
            ReleaseShadowMap();
            shadowMap = ShadowMap(settings.Resolution, (uint)layers);
            shadowSize = settings.Resolution;
        }
        device.WriteBuffer(globals, 0, MemoryMarshal.AsBytes(GlobalsFor(frame, shadows, settings.Resolution).AsSpan()));
        foreach (var item in frame.Items)
        {
            if (item.Material is { } material)
            {
                Prepare(material);
            }
        }
        UploadAssets(frame);

        // Opaque items grouped by material, mesh and images, then transparent items back to front.
        var opaque = frame.Items.Where(i => !i.IsTransparent).OrderBy(i => i, Comparer<DrawItem>.Create((a, b) =>
        {
            var c = MaterialKey(a).CompareTo(MaterialKey(b));
            c = c != 0 ? c : MeshKey(a).CompareTo(MeshKey(b));
            c = c != 0 ? c : ComparePart(a.Part, b.Part);
            return c != 0 ? c : KeyOf(a).CompareTo(KeyOf(b));
        })).ToList();
        var eye = frame.Camera.Position;
        var transparent = frame.Items.Where(i => i.IsTransparent)
            .OrderBy(i => i, Comparer<DrawItem>.Create((a, b) => FMat.TotalOrder(Vector3.DistanceSquared(b.Model.Translation, eye), Vector3.DistanceSquared(a.Model.Translation, eye))))
            .ToList();
        var ordered = opaque.Concat(transparent).ToList();
        var skins = WriteJoints(ordered);
        var raw = new float[ordered.Count * InstanceFloats];
        for (var i = 0; i < ordered.Count; i++)
        {
            var item = ordered[i];
            var selected = frame.Selected == item.ObjectId || frame.Highlighted.Contains(item.ObjectId);
            WriteInstance(raw.AsSpan(i * InstanceFloats, InstanceFloats), item, selected, skins[i]);
        }
        if (ordered.Count > instanceCapacity)
        {
            instanceCapacity = (int)System.Numerics.BitOperations.RoundUpToPowerOf2((uint)ordered.Count);
            instances.Dispose();
            instances = InstanceBuffer(instanceCapacity);
        }
        if (raw.Length > 0)
        {
            device.WriteBuffer(instances, 0, MemoryMarshal.AsBytes(raw.AsSpan()));
        }

        var made = new List<GpuObject>();
        using var encoder = device.CreateEncoder("bayleaf frame");
        (uint Calls, ulong Triangles) count = (0, 0);
        var faces = shadows.SelectMany(v => v.Faces).ToList();
        Span<float> face = stackalloc float[16];
        for (var layer = 0; layer < faces.Count; layer++)
        {
            FMat.Write(face, faces[layer]);
            device.WriteBuffer(shadowFaces, (ulong)layer * FaceStride, MemoryMarshal.AsBytes(face));
        }
        // The opaque shapes, as seen from each light that casts shadows, into its layers.
        for (var layer = 0; layer < faces.Count; layer++)
        {
            using var pass = encoder.BeginRenderPass("bayleaf shadow pass", null, new GpuDepthAttachment(shadowMap.Layers[layer]));
            pass.SetPipeline(shadowPipeline);
            pass.SetBindGroup(0, globalsGroup);
            pass.SetBindGroup(1, shadowFaceGroup, (uint)((ulong)layer * FaceStride));
            pass.SetVertexBuffer(1, instances);
            for (var start = 0; start < opaque.Count;)
            {
                var first = opaque[start];
                var run = 1;
                while (start + run < opaque.Count && MeshKey(opaque[start + run]) == MeshKey(first) && opaque[start + run].Part == first.Part)
                {
                    run++;
                }
                if (MeshOf(first) is { } mesh)
                {
                    pass.SetVertexBuffer(0, mesh.Vertices);
                    pass.SetIndexBuffer(mesh.Indices);
                    var (from, n) = Indices(first, mesh);
                    pass.DrawIndexed(from, n, (uint)start, (uint)run);
                }
                start += run;
            }
        }
        {
            var c = frame.Background ? frame.ClearColor : Vector4.Zero;
            using var pass = encoder.BeginRenderPass("bayleaf scene pass",
                // The multisampled colour is resolved into the scene colour and then not needed;
                // without multisampling the pass draws into the scene colour itself, which stays.
                new GpuColorAttachment(target.MsaaView ?? target.HdrView, target.MsaaView is null ? null : target.HdrView, c, target.MsaaView is null),
                // Kept for post-processing (depth fog, ambient occlusion).
                new GpuDepthAttachment(target.DepthView));
            pass.SetBindGroup(0, globalsGroup);
            pass.SetBindGroup(1, shadowMap.Group);
            pass.SetVertexBuffer(1, instances);
            // Every pipeline takes the image group; start with white images.
            if (textureGroups.TryGetValue(default, out var white))
            {
                pass.SetBindGroup(2, white);
            }
            if (frame.Sky is not null && frame.Background)
            {
                pass.SetPipeline(sky);
                pass.Draw(3);
            }
            // Runs of opaque items that share a material, a mesh and images are drawn as one instanced
            // call; then the outlines of the opaque shapes whose material has one: their back faces, grown.
            foreach (var outlined in new[] { false, true })
            {
                for (var start = 0; start < opaque.Count;)
                {
                    var first = opaque[start];
                    var key = (MaterialKey(first), MeshKey(first), KeyOf(first), first.Part);
                    var run = 1;
                    while (start + run < opaque.Count && (MaterialKey(opaque[start + run]), MeshKey(opaque[start + run]), KeyOf(opaque[start + run]), opaque[start + run].Part) == key)
                    {
                        run++;
                    }
                    var pipeline = outlined ? outlines.GetValueOrDefault(key.Item1) : Pipeline(key.Item1, false);
                    if (pipeline is not null && MeshOf(first) is { } mesh && textureGroups.TryGetValue(key.Item3, out var group))
                    {
                        pass.SetPipeline(pipeline);
                        pass.SetBindGroup(2, group);
                        pass.SetVertexBuffer(0, mesh.Vertices);
                        pass.SetIndexBuffer(mesh.Indices);
                        var (from, n) = Indices(first, mesh);
                        pass.DrawIndexed(from, n, (uint)start, (uint)run);
                        count.Calls++;
                        count.Triangles += (ulong)(n / 3) * (ulong)run;
                    }
                    start += run;
                }
            }
            if (frame.ShowGrid)
            {
                pass.SetPipeline(grid);
                pass.Draw(6);
            }
            for (var offset = 0; offset < transparent.Count; offset++)
            {
                var item = transparent[offset];
                var pipeline = item.Additive ? additive.GetValueOrDefault(MaterialKey(item)) ?? additive.GetValueOrDefault(0UL) : Pipeline(MaterialKey(item), true);
                if (pipeline is not null && MeshOf(item) is { } mesh && textureGroups.TryGetValue(KeyOf(item), out var group))
                {
                    var index = (uint)(opaque.Count + offset);
                    pass.SetPipeline(pipeline);
                    pass.SetBindGroup(2, group);
                    pass.SetVertexBuffer(0, mesh.Vertices);
                    pass.SetIndexBuffer(mesh.Indices);
                    var (from, n) = Indices(item, mesh);
                    pass.DrawIndexed(from, n, index, 1);
                    count.Calls++;
                    count.Triangles += n / 3;
                }
            }
        }
        var chosen = ordered.Select((item, index) => (Index: (uint)index, Item: item))
            .Where(x => frame.Selected == x.Item.ObjectId || frame.Highlighted.Contains(x.Item.ObjectId)).ToList();
        drawn = count;
        // The displayed picture, from the scene's colour.
        post.Run(encoder, target, globalsGroup, frame.Post, made);
        if (chosen.Count > 0)
        {
            Outline(encoder, target, chosen, made);
        }
        device.Submit(encoder.Finish());
        foreach (var o in made)
        {
            o.Dispose();
        }
    }

    /// <summary>Outlines the selected shapes: draws them into the target's mask, then paints a band around the mask over the finished picture.</summary>
    private void Outline(GpuEncoder encoder, RenderTarget target, List<(uint Index, DrawItem Item)> selected, List<GpuObject> made)
    {
        using (var pass = encoder.BeginRenderPass("bayleaf selection mask pass", new GpuColorAttachment(target.MaskView, null, Vector4.Zero), null))
        {
            pass.SetPipeline(maskPipeline);
            pass.SetBindGroup(0, globalsGroup);
            pass.SetVertexBuffer(1, instances);
            foreach (var (index, item) in selected)
            {
                if (MeshOf(item) is { } mesh)
                {
                    pass.SetVertexBuffer(0, mesh.Vertices);
                    pass.SetIndexBuffer(mesh.Indices);
                    var (from, n) = Indices(item, mesh);
                    pass.DrawIndexed(from, n, index, 1);
                }
            }
        }
        var mask = device.CreateBindGroup("bayleaf selection mask group", outlineLayout, [new GpuViewBinding(0, target.MaskView)]);
        made.Add(mask);
        using var band = encoder.BeginRenderPass("bayleaf outline pass", new GpuColorAttachment(target.ColorView, null, null), null);
        band.SetPipeline(outlinePipeline);
        band.SetBindGroup(0, globalsGroup);
        band.SetBindGroup(1, mask);
        band.Draw(3);
    }

    private static void WriteInstance(Span<float> into, DrawItem item, bool selected, uint? skin)
    {
        FMat.Write(into, item.Model);
        var n = FMat.NormalMatrix(item.Model);
        ReadOnlySpan<float> normal = [n.M11, n.M12, n.M13, 0, n.M21, n.M22, n.M23, 0, n.M31, n.M32, n.M33, 0];
        normal.CopyTo(into[16..]);
        ReadOnlySpan<float> rest =
        [
            item.Color.X, item.Color.Y, item.Color.Z, item.Color.W,
            item.Metallic, item.Roughness, selected ? 1 : 0, item.Unlit ? 1 : 0,
            item.UvRect.X, item.UvRect.Y, item.UvRect.Z, item.UvRect.W,
            skin ?? 0, skin is null ? 0 : 1, 0, 0,
        ];
        rest.CopyTo(into[28..]);
    }
}
