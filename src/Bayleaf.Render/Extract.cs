using System.Numerics;

using Bayleaf.Assets;
using Bayleaf.Core;
using Bayleaf.Core.Geometry;
using Bayleaf.Scene;

using Bayleaf.Scene.Components;

using EnvironmentComponent = Bayleaf.Scene.Components.Environment;
using SceneModel = Bayleaf.Scene.Scene;
using AnimationComponent = Bayleaf.Scene.Components.ModelAnimation;
using Transform = Bayleaf.Scene.Components.Transform;

namespace Bayleaf.Render;

/// <summary>
/// Where a frame gets the files its scene names: materials, images and models, by the path written in
/// the scene. Null draws the object without it.
/// </summary>
public interface IAssetSource
{
    CompiledMaterial? Material(string path);

    Image? Image(string path);

    MeshData? Model(string path);

    /// <summary>A lighting asset (<c>.blight</c>) the scene's settings name.</summary>
    Lighting? Lighting(string path) => null;

    /// <summary>A post-processing asset (<c>.bpost</c>) the scene's settings or a camera name.</summary>
    PostProcessing? Post(string path) => null;

    /// <summary>
    /// Where the nodes of <paramref name="id"/>'s model are now (each its matrix in the model, before
    /// the import scale and axes), when something animates it; null leaves it to the object's Model
    /// Animation at the frame's time.
    /// </summary>
    IReadOnlyList<Matrix4x4>? Pose(StableId id) => null;
}

/// <summary>Draws every object without its files.</summary>
public sealed class NoAssets : IAssetSource
{
    public static readonly NoAssets Instance = new();

    public CompiledMaterial? Material(string path) => null;

    public Image? Image(string path) => null;

    public MeshData? Model(string path) => null;
}

/// <summary>
/// Reads materials, images and models from files, relative to <see cref="Base"/>, through caches that
/// notice when a file changes. Every path passes <see cref="Allow"/> first (the automation folders).
/// Problems are kept in <see cref="Warnings"/>, once each.
/// </summary>
public sealed class FileAssets(string root, MaterialLibrary materials, AssetCache cache, Func<string, Result<string, string>> allow) : IAssetSource
{
    public string Base { get; } = root;

    public MaterialLibrary Materials { get; } = materials;

    public AssetCache Cache { get; } = cache;

    public Func<string, Result<string, string>> Allow { get; } = allow;

    public List<string> Warnings { get; } = [];

    /// <summary>Files read from where they are, with no folders refused.</summary>
    public static FileAssets Unconfined(string root) => new(root, new MaterialLibrary(), new AssetCache(), p => p);

    private T? Load<T>(string path, Func<string, Result<T, string>> load)
        where T : class
    {
        var allowed = Allow(Path.Combine(Base, path));
        var result = allowed.IsOk ? load(allowed.Value) : Result.Fail(allowed.Error);
        if (result.TryGet(out var value, out var error))
        {
            return value;
        }
        var message = $"{path}: {error}";
        if (!Warnings.Contains(message))
        {
            Warnings.Add(message);
        }
        return null;
    }

    public CompiledMaterial? Material(string path) => Load(path, Materials.Get);

    public Image? Image(string path) => Load(path, Cache.Image);

    public MeshData? Model(string path) => Load(path, Cache.Model);

    public Lighting? Lighting(string path) => Load(path, Materials.Lighting);

    public PostProcessing? Post(string path) => Load(path, Materials.Post);
}

/// <summary>Assets as another source gives them, with each object's pose from <paramref name="poses"/> (the engine's animation players).</summary>
public sealed class PosedAssets(IAssetSource assets, IReadOnlyDictionary<StableId, Matrix4x4[]> poses) : IAssetSource
{
    public CompiledMaterial? Material(string path) => assets.Material(path);

    public Image? Image(string path) => assets.Image(path);

    public MeshData? Model(string path) => assets.Model(path);

    public Lighting? Lighting(string path) => assets.Lighting(path);

    public PostProcessing? Post(string path) => assets.Post(path);

    public IReadOnlyList<Matrix4x4>? Pose(StableId id) => poses.TryGetValue(id, out var pose) ? pose : null;
}

/// <summary>The CPU side of a frame: what to draw, from where, and which object a screen point hits.</summary>
public static class Extraction
{
    /// <summary>Lights beyond this count are ignored.</summary>
    public const int MaxLights = 8;

    /// <summary>Ambient light and background for a scene without an environment.</summary>
    private static readonly Vector3 DefaultAmbient = new(0.22f, 0.24f, 0.28f);

    private static readonly Vector4 DefaultClear = new(0.035f, 0.04f, 0.05f, 1f);

    /// <summary>Ambient light from an environment's colours at strength 1.</summary>
    private const float EnvironmentAmbient = 0.5f;

    /// <summary>Light used when the scene has none, so a new scene is not black.</summary>
    private static readonly LightItem FallbackLight = LightItem.Directional(new Vector3(-0.4f, -1f, -0.6f), new Vector3(2.8f), false);

    private static T? Get<T>(SceneModel scene, StableId id)
        where T : class => scene.TryGet<T>(id, out var value) ? value : null;

    /// <summary>Where object <paramref name="id"/>'s model's nodes are now: as the source says it is posed, else as its Model Animation poses it at <paramref name="time"/>, else at rest.</summary>
    private static IReadOnlyList<Matrix4x4> ModelPose(SceneModel scene, StableId id, Skeleton skeleton, float time, IAssetSource assets)
    {
        if (assets.Pose(id) is { } posed && posed.Count == skeleton.Nodes.Count)
        {
            return posed;
        }
        var animation = Get<AnimationComponent>(scene, id) is { Playing: true } a ? a : null;
        var index = animation is null ? null : skeleton.Animation(animation.Animation);
        var at = animation is null ? 0f : skeleton.ClipTime(index, time * animation.Speed, animation.Looping);
        return skeleton.PoseAt(index, at);
    }

    private static Skin? SkinOf(SceneModel scene, StableId id, MeshData model, float time, IAssetSource assets) =>
        model.Skeleton is { } skeleton ? SkinFor(skeleton, ModelPose(scene, id, skeleton, time, assets)) : null;

    private static Skeleton? SkeletonOf(SceneModel scene, StableId id, IAssetSource assets)
    {
        if (Get<Mesh>(scene, id) is not { } mesh || mesh.Model.Length == 0)
        {
            return null;
        }
        return assets.Model(mesh.Model)?.Skeleton;
    }

    /// <summary>The matrix of joint <paramref name="joint"/> of object <paramref name="id"/>'s model in the object's own space (with the import scale and axes), as the model stands now.</summary>
    public static Matrix4x4? JointMatrix(SceneModel scene, StableId id, string joint, float time, IAssetSource assets)
    {
        if (SkeletonOf(scene, id, assets) is not { } skeleton || skeleton.NodeNamed(joint) is not { } node)
        {
            return null;
        }
        var world = ModelPose(scene, id, skeleton, time, assets);
        return node < world.Count ? world[node] * skeleton.Root : null;
    }

    /// <summary>Lines from each joint of object <paramref name="id"/>'s model to its parent joint, in the object's own space, as the model stands now; null without a skeleton.</summary>
    public static IReadOnlyList<(Vector3 From, Vector3 To)>? SkeletonLines(SceneModel scene, StableId id, float time, IAssetSource assets)
    {
        if (SkeletonOf(scene, id, assets) is not { } skeleton)
        {
            return null;
        }
        var world = ModelPose(scene, id, skeleton, time, assets);
        Vector3? At(int node) => node >= 0 && node < world.Count ? (world[node] * skeleton.Root).Translation : null;
        var lines = new List<(Vector3, Vector3)>();
        for (var i = 0; i < skeleton.Nodes.Count; i++)
        {
            if (skeleton.Nodes[i].Parent is { } parent && At(parent) is { } from && At(i) is { } to)
            {
                lines.Add((from, to));
            }
        }
        return lines.Count > 0 ? lines : null;
    }

    /// <summary>
    /// Every object's world matrix in hierarchy order, with each object that follows a joint (Follow
    /// Joint) and the objects under it placed on that joint of its parent's model as the model stands now.
    /// </summary>
    public static IReadOnlyList<(StableId Id, DMat4 Matrix)> FollowedWorlds(SceneModel scene, float time, IAssetSource assets)
    {
        var worlds = scene.WorldMatrices();
        if (!worlds.Any(w => scene.TryGet<FollowJoint>(w.Id, out _)))
        {
            return worlds;
        }
        var placed = new Dictionary<StableId, DMat4>(worlds.Count);
        var output = new List<(StableId, DMat4)>(worlds.Count);
        foreach (var (id, world) in worlds)
        {
            var parent = scene.Parent(id);
            var local = scene.TryGet<Transform>(id, out var transform) ? transform.Matrix() : DMat4.Identity;
            var placedParent = parent is { } p && placed.TryGetValue(p, out var pw) ? pw : (DMat4?)null;
            var matrix = placedParent is { } parentWorld ? parentWorld * local : world;
            if (Get<FollowJoint>(scene, id) is { } follow && parent is { } owner && placedParent is { } ownerWorld
                && JointMatrix(scene, owner, follow.Joint, time, assets) is { } joint)
            {
                matrix = ownerWorld * FMat.ToDouble(joint) * local;
            }
            placed[id] = matrix;
            output.Add((id, matrix));
        }
        return output;
    }

    /// <summary>The skin of <paramref name="skeleton"/> with its nodes at <paramref name="world"/>.</summary>
    public static Skin SkinFor(Skeleton skeleton, IReadOnlyList<Matrix4x4> world)
    {
        var matrices = skeleton.SkinMatrices(world);
        var (min, max) = skeleton.SkinnedBounds(matrices) ?? (Vector3.Zero, Vector3.Zero);
        return new Skin(matrices, min, max);
    }

    /// <summary>What a post-processing asset asks for, as far as <paramref name="allowed"/> lets it; its lookup image is for the caller to load.</summary>
    public static PostSettings PostSettingsOf(PostProcessing post, EffectsAllowed allowed)
    {
        var s = new PostSettings();
        if (post.ToneMapping.Enabled && allowed.ToneMapping)
        {
            s = s with { ToneMapping = true, Exposure = post.ToneMapping.Exposure };
        }
        if (post.Bloom.Enabled && allowed.Bloom)
        {
            s = s with { Bloom = post.Bloom.Strength, BloomThreshold = post.Bloom.Threshold };
        }
        if (post.Color.Enabled && allowed.Color)
        {
            s = s with { Contrast = post.Color.Contrast, Saturation = post.Color.Saturation, LookupStrength = post.Color.LookupStrength };
        }
        if (post.Vignette.Enabled && allowed.Vignette)
        {
            s = s with { Vignette = post.Vignette.Strength, VignetteSmoothness = post.Vignette.Smoothness };
        }
        if (post.DepthFog.Enabled && allowed.DepthFog)
        {
            s = s with { Fog = (post.DepthFog.Color, post.DepthFog.Start, post.DepthFog.End) };
        }
        if (post.Occlusion.Enabled && allowed.Occlusion)
        {
            s = s with { Occlusion = post.Occlusion.Strength, OcclusionRadius = post.Occlusion.Radius };
        }
        return s with { Antialiasing = allowed.Antialiasing ? post.Antialiasing : Antialiasing.Off };
    }

    /// <summary>The post-processing a frame gets: the camera's own asset, else the scene's, as the options allow.</summary>
    private static PostSettings FramePost(SceneModel scene, FrameOptions options, IAssetSource assets)
    {
        if (!options.Effects.Show)
        {
            return PostSettings.Default;
        }
        var path = options.Effects.Camera is { } id && Get<Camera>(scene, id) is { PostProcessing.Length: > 0 } camera ? camera.PostProcessing : null;
        path ??= scene.Settings.PostProcessing.Length > 0 ? scene.Settings.PostProcessing : null;
        if (path is null || assets.Post(path) is not { } post)
        {
            return PostSettings.Default;
        }
        var settings = PostSettingsOf(post, options.Effects.Allowed);
        return post.Color.Enabled && options.Effects.Allowed.Color ? settings with { Lookup = LookupImage(post, assets) } : settings;
    }

    /// <summary>The colour lookup image a post-processing asset names, when it loads and is one (N squares of N by N side by side).</summary>
    public static Image? LookupImage(PostProcessing post, IAssetSource assets)
    {
        if (post.Color.Lookup.Length == 0)
        {
            return null;
        }
        return assets.Image(post.Color.Lookup) is { } image && ColorAdjustment.IsLookupSize((uint)image.Width, (uint)image.Height) ? image : null;
    }

    /// <summary>The draw items and lights of <paramref name="scene"/> as seen by <paramref name="camera"/>, with <paramref name="assets"/> giving the files it names (none when null).</summary>
    public static Frame Extract(SceneModel scene, CameraView camera, FrameOptions options, IAssetSource? assets = null)
    {
        assets ??= NoAssets.Instance;
        var items = new List<DrawItem>();
        var lights = new List<LightItem>();
        // The scene's lighting asset, else (in scenes not yet moved to one) its first Environment.
        var lighting = scene.Settings.Lighting.Length > 0 ? assets.Lighting(scene.Settings.Lighting) : null;
        var environment = lighting?.Environment();
        var hasSun = false;
        foreach (var (id, worldMatrix) in FollowedWorlds(scene, options.Time, assets))
        {
            var world = camera.Relative(worldMatrix);
            environment ??= Get<EnvironmentComponent>(scene, id);
            hasSun |= scene.TryGet<DirectionalLight>(id, out _);
            if (Get<Mesh>(scene, id) is { } mesh)
            {
                var material = mesh.Material.Length == 0 ? null : assets.Material(mesh.Material);
                var model = mesh.Model.Length == 0 ? null : assets.Model(mesh.Model);
                var skin = model is null ? null : SkinOf(scene, id, model, options.Time, assets);
                var item = new DrawItem
                {
                    ObjectId = id,
                    Shape = mesh.Shape,
                    Model = world,
                    Color = mesh.Color,
                    Metallic = mesh.Metallic,
                    Roughness = mesh.Roughness,
                    Textures = material is null ? [] : [.. material.Textures.Select(assets.Image)],
                    Material = material,
                    Mesh = model,
                    Skin = skin,
                };
                // A model's own materials tint the mesh colour and set metallic and roughness, unless a
                // material graph is chosen; a model with several draws one item per part.
                if (model is not null && material is null && model.Parts.Count > 1)
                {
                    foreach (var part in model.Parts)
                    {
                        var piece = item with { Part = (part.First, part.Count) };
                        items.Add(part.Material is { } own ? WithModelMaterial(piece, own) : piece);
                    }
                }
                else if (model is not null && material is null)
                {
                    items.Add(model.Material is { } own ? WithModelMaterial(item, own) : item);
                }
                else
                {
                    items.Add(item);
                }
            }
            if (Get<Sprite>(scene, id) is { } sprite)
            {
                items.Add(new DrawItem
                {
                    ObjectId = id,
                    Shape = Shape.Quad,
                    Model = Matrix4x4.CreateScale(sprite.Size.X, sprite.Size.Y, 1f) * world,
                    Color = sprite.Color,
                    Metallic = 0,
                    Roughness = 1,
                    Unlit = true,
                    Textures = sprite.Image.Length == 0 ? [] : [assets.Image(sprite.Image)],
                });
            }
            var direction = FMat.NormalizeOr(FMat.TransformVector(world, -Vector3.UnitZ), -Vector3.UnitY);
            var position = world.Translation;
            if (Get<DirectionalLight>(scene, id) is { } sun)
            {
                lights.Add(LightItem.Directional(direction, sun.Color * sun.Intensity, sun.Shadows));
            }
            if (Get<PointLight>(scene, id) is { } point)
            {
                lights.Add(new LightItem(LightKind.Point, position, direction, point.Color * point.Intensity, point.Range, -1, -1, point.Shadows));
            }
            if (Get<SpotLight>(scene, id) is { } spot)
            {
                var half = CameraView.Radians(spot.Angle * 0.5f);
                lights.Add(new LightItem(LightKind.Spot, position, direction, spot.Color * spot.Intensity, spot.Range, MathF.Cos(half), MathF.Cos(half * (1f - spot.Softness)), spot.Shadows));
            }
        }
        // The lighting asset's sun lights a scene that has no Directional Light of its own.
        if (lighting?.Sun is { Enabled: true } assetSun && !hasSun)
        {
            var turn = FMat.FromDouble((Transform.Initial with { Rotation = assetSun.Rotation }).Matrix());
            var direction = FMat.NormalizeOr(FMat.TransformVector(turn, -Vector3.UnitZ), -Vector3.UnitY);
            lights.Add(LightItem.Directional(direction, assetSun.Color * assetSun.Intensity, assetSun.Shadows));
        }
        // Sun-like lights first, then the lights nearest the camera, as many as the shader takes.
        float Rank(LightItem l) => l.Kind == LightKind.Directional ? 0f : 1f + Vector3.DistanceSquared(l.Position, camera.Position);
        lights = [.. lights.Select((l, i) => (l, i)).OrderBy(x => x, Comparer<(LightItem L, int I)>.Create((a, b) =>
        {
            var c = FMat.TotalOrder(Rank(a.L), Rank(b.L));
            return c != 0 ? c : a.I.CompareTo(b.I);
        })).Select(x => x.l).Take(MaxLights)];
        // A scene without lights gets a soft default light, so a new scene is not black.
        if (lights.Count == 0)
        {
            lights.Add(FallbackLight);
        }
        var (ambient, ambientGround, clear, sky) = environment is not null ? EnvironmentLight(environment) : (DefaultAmbient, DefaultAmbient * 0.35f, DefaultClear, null);
        var post = FramePost(scene, options, assets);
        // Half of what lies fogDistance away is hidden: 1 - exp(-density * distance) = 0.5.
        var fog = environment is { FogDistance: > 0 } env ? (env.HorizonColor, MathF.Log(2f) / env.FogDistance) : (Vector3.Zero, 0f);
        return new Frame
        {
            Camera = camera,
            Items = items,
            Lights = lights,
            Ambient = ambient,
            AmbientGround = ambientGround,
            ClearColor = clear,
            Sky = sky,
            Background = true,
            Fog = fog,
            ShowGrid = options.ShowGrid,
            Shadows = options.Shadows,
            Post = post,
            Selected = options.Selected,
            Time = options.Time,
        };
    }

    private static DrawItem WithModelMaterial(DrawItem item, ModelMaterial own) => item with
    {
        Color = item.Color * own.BaseColor,
        Metallic = own.Metallic,
        Roughness = own.Roughness,
        Textures = [own.Image, own.MetalRoughness, own.Normal],
    };

    /// <summary>Ambient light from above and below, the background colour and the sky of an environment.</summary>
    private static (Vector3 Ambient, Vector3 Ground, Vector4 Clear, Sky? Sky) EnvironmentLight(EnvironmentComponent env)
    {
        var strength = env.Ambient * EnvironmentAmbient;
        Sky? sky = env.ShowSky ? new Sky(env.SkyColor, env.HorizonColor, env.GroundColor) : null;
        return ((env.SkyColor + env.HorizonColor) * 0.5f * strength, env.GroundColor * strength, new Vector4(env.HorizonColor, 1f), sky);
    }

    /// <summary>
    /// A frame that shows <paramref name="material"/> on a sphere under the default sky and a sun from
    /// the upper left, with no background (the previews of the Graph and Project panels and the Inspector).
    /// </summary>
    public static Frame MaterialPreview(CompiledMaterial material, IReadOnlyList<Image?> textures, float aspect, float time)
    {
        var (ambient, ground, clear, sky) = EnvironmentLight(EnvironmentComponent.Initial);
        return new Frame
        {
            Camera = CameraView.LookAt(new Vector3(0, 0.3f, 1.9f), Vector3.Zero, 36f, aspect),
            Items = [new DrawItem { ObjectId = default, Shape = Shape.Sphere, Model = Matrix4x4.Identity, Color = Vector4.One, Metallic = 0, Roughness = 0.5f, Material = material, Textures = textures }],
            Lights = [LightItem.Directional(FMat.Normalize(new Vector3(-0.5f, -0.7f, -0.6f)), new Vector3(3f), false)],
            Ambient = ambient,
            AmbientGround = ground,
            ClearColor = clear,
            Sky = sky,
            Background = false,
            // Nothing casts a shadow here, so the smallest map.
            Shadows = new ShadowSettings(256, 1, 0, 1),
            Time = time,
        };
    }

    /// <summary>The first camera object in hierarchy order, if the scene has one: the one the game shows.</summary>
    public static StableId? CameraObject(SceneModel scene)
    {
        foreach (var (id, _) in scene.WorldMatrices())
        {
            if (scene.TryGet<Camera>(id, out _))
            {
                return id;
            }
        }
        return null;
    }

    /// <summary>The view of the first camera object in hierarchy order, if the scene has one.</summary>
    public static CameraView? SceneCamera(SceneModel scene, float aspect)
    {
        foreach (var (id, world) in scene.WorldMatrices())
        {
            if (Get<Camera>(scene, id) is { } camera)
            {
                return CameraView.FromCamera(world, camera, aspect);
            }
        }
        return null;
    }

    /// <summary>What the game shows: the first camera's view, or for a scene without a camera a view of everything in it.</summary>
    public static CameraView GameCamera(SceneModel scene, float aspect)
    {
        if (SceneCamera(scene, aspect) is { } view)
        {
            return view;
        }
        var (center, radius) = Bounds(scene) ?? (Double3.Zero, 2f);
        var eye = center + CameraView.Normalized(new Double3(0.6, 0.45, 1.0)) * ((double)Math.Max(radius, 0.5f) * 2.6 + 1.0);
        return CameraView.LookAtWorld(eye, center, 60f, aspect);
    }

    /// <summary>The view of camera object <paramref name="id"/>, if it has a camera component.</summary>
    public static CameraView? ObjectCamera(SceneModel scene, StableId id, float aspect) =>
        Get<Camera>(scene, id) is { } camera && scene.WorldMatrix(id) is { } world ? CameraView.FromCamera(world, camera, aspect) : null;

    /// <summary>Centre and radius of a sphere around every drawn shape, or null for a scene with nothing to draw.</summary>
    public static (Double3 Center, float Radius)? Bounds(SceneModel scene) => WorldBounds(scene, null);

    /// <summary>Bounds of one object's shapes.</summary>
    public static (Double3 Center, float Radius)? ObjectBounds(SceneModel scene, StableId id) =>
        WorldBounds(scene, id) ?? (scene.WorldMatrix(id) is { } m ? (m.WAxis.Xyz, 0.5f) : null);

    /// <summary>Bounds in world space, worked out near the shapes (measured from the first object) so far-away scenes keep their precision.</summary>
    private static (Double3 Center, float Radius)? WorldBounds(SceneModel scene, StableId? only)
    {
        var worlds = scene.WorldMatrices();
        var origin = worlds.Count > 0 ? worlds[0].Matrix.WAxis.Xyz : Double3.Zero;
        var camera = CameraView.LookAt(Vector3.One, Vector3.Zero, 60f, 1f) with { Origin = origin };
        var frame = Extract(scene, camera, FrameOptions.Default);
        if (ItemBounds(frame.Items.Where(i => only is null || i.ObjectId == only)) is not { } bounds)
        {
            return null;
        }
        return (origin + new Double3(bounds.Center.X, bounds.Center.Y, bounds.Center.Z), bounds.Radius);
    }

    internal static (Vector3 Center, float Radius)? ItemBounds(IEnumerable<DrawItem> items)
    {
        var min = new Vector3(float.PositiveInfinity);
        var max = new Vector3(float.NegativeInfinity);
        var any = false;
        foreach (var item in items)
        {
            var (lo, hi) = item.LocalBounds();
            for (var i = 0; i < 8; i++)
            {
                // The unit box's corners moved onto the item's own box.
                var corner = new Vector3((i & 1) == 0 ? -0.5f : 0.5f, (i & 2) == 0 ? -0.5f : 0.5f, (i & 4) == 0 ? -0.5f : 0.5f);
                var p = FMat.TransformPoint(item.Model, lo + (corner + new Vector3(0.5f)) * (hi - lo));
                min = Min(min, p);
                max = Max(max, p);
                any = true;
            }
        }
        return any ? ((min + max) * 0.5f, ((max - min) * 0.5f).Length()) : null;
    }

    private static float Min(float a, float b) => float.IsNaN(a) ? b : float.IsNaN(b) ? a : a < b ? a : b;

    private static float Max(float a, float b) => float.IsNaN(a) ? b : float.IsNaN(b) ? a : a > b ? a : b;

    private static Vector3 Min(Vector3 a, Vector3 b) => new(Min(a.X, b.X), Min(a.Y, b.Y), Min(a.Z, b.Z));

    private static Vector3 Max(Vector3 a, Vector3 b) => new(Max(a.X, b.X), Max(a.Y, b.Y), Max(a.Z, b.Z));

    /// <summary>The nearest drawn object under a point in normalized device coordinates.</summary>
    public static StableId? Pick(Frame frame, Vector2 ndc) => NearestHit(frame.Items, frame.Camera.Ray(ndc))?.Id;

    /// <summary>The nearest item <paramref name="ray"/> (measured from the view's origin) meets, and how far along it, in lengths of its direction.</summary>
    public static (StableId Id, float Distance)? NearestHit(IEnumerable<DrawItem> items, Ray ray)
    {
        (float T, StableId Id)? best = null;
        foreach (var item in items)
        {
            var inverse = FMat.Inverse(item.Model);
            if (!FMat.IsFinite(inverse))
            {
                continue;
            }
            var local = new Ray(FMat.TransformPoint(inverse, ray.Origin), FMat.TransformVector(inverse, ray.Direction));
            // `t` is shared between the world and local rays because the transform is affine.
            float? hit;
            if (item.Mesh is null && item.Shape == Shape.Sphere)
            {
                hit = HitSphere(local, 0.5f);
            }
            else
            {
                // Models are picked by the box around them.
                var (min, max) = item.LocalBounds();
                var center = (min + max) * 0.5f;
                hit = HitBox(new Ray(local.Origin - center, local.Direction), (max - min) * 0.5f);
            }
            if (hit is { } t && (best is null || t < best.Value.T))
            {
                best = (t, item.ObjectId);
            }
        }
        return best is { } b ? (b.Id, b.T) : null;
    }

    private static float? HitSphere(Ray ray, float radius)
    {
        var a = ray.Direction.LengthSquared();
        var b = Vector3.Dot(ray.Origin, ray.Direction);
        var c = ray.Origin.LengthSquared() - radius * radius;
        var disc = b * b - a * c;
        if (a <= 0 || disc < 0)
        {
            return null;
        }
        var sq = MathF.Sqrt(disc);
        var near = (-b - sq) / a;
        var far = (-b + sq) / a;
        return near >= 0 ? near : far >= 0 ? far : null;
    }

    private static float? HitBox(Ray ray, Vector3 half)
    {
        var tMin = 0f;
        var tMax = float.PositiveInfinity;
        for (var axis = 0; axis < 3; axis++)
        {
            var (o, d, h) = (ray.Origin[axis], ray.Direction[axis], half[axis]);
            if (MathF.Abs(d) < 1e-12f)
            {
                if (MathF.Abs(o) > h)
                {
                    return null;
                }
                continue;
            }
            var (t0, t1) = ((-h - o) / d, (h - o) / d);
            tMin = Max(tMin, Min(t0, t1));
            tMax = Min(tMax, Max(t0, t1));
            if (tMin > tMax)
            {
                return null;
            }
        }
        return tMin;
    }
}
