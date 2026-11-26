using System.Numerics;

using Bayleaf.Assets;
using Bayleaf.Core;
using Bayleaf.Core.Geometry;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;

namespace Bayleaf.Render;

/// <summary>
/// Where the scene is seen from. A frame is drawn in coordinates measured from <see cref="Origin"/>,
/// a world point at or near the camera, so the GPU's single precision stays exact however far from
/// the world's centre the camera is.
/// </summary>
public readonly record struct CameraView(Matrix4x4 View, Matrix4x4 Proj, Vector3 Position, Double3 Origin)
{
    /// <summary>Perspective camera at <paramref name="eye"/> looking at <paramref name="target"/>, +Y up, measured from the world's centre.</summary>
    public static CameraView LookAt(Vector3 eye, Vector3 target, float fovDegrees, float aspect)
    {
        var up = Vector3.Cross(FMat.NormalizeOrZero(target - eye), Vector3.UnitY).LengthSquared() < 1e-6f ? Vector3.UnitZ : Vector3.UnitY;
        return new(FMat.LookAtRh(eye, target, up), FMat.PerspectiveRh(Radians(fovDegrees), Math.Max(aspect, 1e-3f), 0.05f, 5000f), eye, Double3.Zero);
    }

    /// <summary>Like <see cref="LookAt"/> for world points in double precision: the frame is measured from the eye.</summary>
    public static CameraView LookAtWorld(Double3 eye, Double3 target, float fovDegrees, float aspect) =>
        LookAt(Vector3.Zero, Single(target - eye), fovDegrees, aspect) with { Origin = eye };

    /// <summary>The view through a camera component placed by <paramref name="world"/>, measured from the camera.</summary>
    public static CameraView FromCamera(DMat4 world, Camera camera, float aspect)
    {
        aspect = Math.Max(aspect, 1e-3f);
        // Scale on a camera object does not stretch the view.
        var (_, rotation, position) = world.ToScaleRotationTranslation();
        var view = FMat.Inverse(Matrix4x4.CreateFromQuaternion(new Quaternion((float)rotation.X, (float)rotation.Y, (float)rotation.Z, (float)rotation.W)));
        var proj = camera.Projection switch
        {
            Projection.Orthographic => FMat.OrthographicRh(-camera.Size * aspect, camera.Size * aspect, -camera.Size, camera.Size, camera.Near, camera.Far),
            _ => FMat.PerspectiveRh(Radians(camera.Fov), aspect, camera.Near, camera.Far),
        };
        return new(view, proj, Vector3.Zero, position);
    }

    /// <summary>A perspective view that shows a sphere of <paramref name="radius"/> around <paramref name="center"/> from a three-quarter angle.</summary>
    public static CameraView Framing(Double3 center, float radius, float aspect)
    {
        var distance = (double)Math.Max(radius, 0.5f) * 2.6 + 1.0;
        var eye = center + Normalized(new Double3(0.6, 0.45, 0.65)) * distance;
        return LookAtWorld(eye, center, 50f, aspect);
    }

    internal static Double3 Normalized(Double3 v) => v * (1 / v.Length);

    /// <summary>A world matrix in double precision, measured from this view's origin.</summary>
    public Matrix4x4 Relative(DMat4 world)
    {
        var w = world.WAxis;
        return FMat.FromDouble(world with { WAxis = new Double4(w.X - Origin.X, w.Y - Origin.Y, w.Z - Origin.Z, w.W) });
    }

    /// <summary>A world point measured from this view's origin.</summary>
    public Vector3 RelativePoint(Double3 point) => Single(point - Origin);

    /// <summary>The eye in world space.</summary>
    public Double3 Eye => Origin + new Double3(Position.X, Position.Y, Position.Z);

    /// <summary>The view, then the projection.</summary>
    public Matrix4x4 ViewProj => View * Proj;

    /// <summary>The ray through a point in normalized device coordinates (-1 to 1, +Y up), measured from the origin.</summary>
    public Ray Ray(Vector2 ndc)
    {
        var inverse = FMat.Inverse(ViewProj);
        var near = FMat.ProjectPoint(inverse, new Vector3(ndc, 0));
        var far = FMat.ProjectPoint(inverse, new Vector3(ndc, 1));
        return new Ray(near, far - near);
    }

    internal static float Radians(float degrees) => degrees * (MathF.PI / 180f);

    internal static Vector3 Single(Double3 v) => new((float)v.X, (float)v.Y, (float)v.Z);
}

public readonly record struct Ray(Vector3 Origin, Vector3 Direction);

/// <summary>Where a skinned model's vertices go: the matrix each of its bindings moves them by, and the box round the posed model.</summary>
public sealed record Skin(IReadOnlyList<Matrix4x4> Matrices, Vector3 Min, Vector3 Max);

/// <summary>One drawn shape.</summary>
public sealed record DrawItem
{
    /// <summary>The whole image, as most items show it.</summary>
    public static readonly Vector4 WholeImage = new(0, 0, 1, 1);

    public required StableId ObjectId { get; init; }

    public Shape Shape { get; init; }

    public Matrix4x4 Model { get; init; } = Matrix4x4.Identity;

    public Vector4 Color { get; init; } = Vector4.One;

    public float Metallic { get; init; }

    public float Roughness { get; init; } = 0.5f;

    /// <summary>Sprites ignore lighting.</summary>
    public bool Unlit { get; init; }

    /// <summary>A compiled material graph; null uses colour, metallic and roughness.</summary>
    public CompiledMaterial? Material { get; init; }

    /// <summary>A loaded model, drawn instead of <see cref="Shape"/>.</summary>
    public MeshData? Mesh { get; init; }

    /// <summary>Images by texture slot: a sprite's image, or a material's Texture nodes in order. A missing image reads as white.</summary>
    public IReadOnlyList<Image?> Textures { get; init; } = [];

    /// <summary>The model's triangles this item draws (first index, count); null for all of them.</summary>
    public (uint First, uint Count)? Part { get; init; }

    /// <summary>Adds its light to what is behind instead of covering it (fire, sparks).</summary>
    public bool Additive { get; init; }

    /// <summary>The part of its images it shows: offset x and y, then width and height, in texture coordinates.</summary>
    public Vector4 UvRect { get; init; } = WholeImage;

    /// <summary>A model with a skeleton: where its vertices go now. Null draws such a model at rest.</summary>
    public Skin? Skin { get; init; }

    /// <summary>The box around the item before its model matrix: the model's, or the unit shape's.</summary>
    public (Vector3 Min, Vector3 Max) LocalBounds()
    {
        if (Skin is not null)
        {
            return (Skin.Min, Skin.Max);
        }
        if (Mesh is not null)
        {
            return (Mesh.Min, Mesh.Max);
        }
        return Shape switch
        {
            Shape.Plane => (new Vector3(-0.5f, -0.001f, -0.5f), new Vector3(0.5f, 0.001f, 0.5f)),
            Shape.Quad => (new Vector3(-0.5f, -0.5f, -0.001f), new Vector3(0.5f, 0.5f, 0.001f)),
            _ => (new Vector3(-0.5f), new Vector3(0.5f)),
        };
    }

    public bool IsTransparent => Unlit || (Material is not null ? Material.Transparent : Color.W < 1f);
}

/// <summary>What kind of source a light is.</summary>
public enum LightKind
{
    Directional,
    Point,
    Spot,
}

public readonly record struct LightItem(LightKind Kind, Vector3 Position, Vector3 Direction, Vector3 Radiance, float Range, float CosOuter, float CosInner, bool Shadows)
{
    public static LightItem Directional(Vector3 direction, Vector3 radiance, bool shadows) => new(LightKind.Directional, Vector3.Zero, direction, radiance, 0, -1, -1, shadows);
}

/// <summary>A sky: colours straight up, at the horizon and below it, in linear RGB.</summary>
public readonly record struct Sky(Vector3 Zenith, Vector3 Horizon, Vector3 Ground);

/// <summary>How shadows are drawn: the same for every scene of a project.</summary>
public readonly record struct ShadowSettings(uint Resolution, uint Cascades, float Reach, uint Lights)
{
    public static readonly ShadowSettings Default = new(2048, 3, 0, 1);

    /// <summary>The same settings within the ranges the renderer takes.</summary>
    public ShadowSettings Clamped() => new(
        Math.Min(System.Numerics.BitOperations.RoundUpToPowerOf2(Math.Clamp(Resolution, 256u, 4096u)), 4096u),
        Math.Clamp(Cascades, 1u, 4u),
        float.IsFinite(Reach) ? Math.Max(Reach, 0f) : 0f,
        Math.Clamp(Lights, 1u, 4u));
}

/// <summary>The effects a project lets its scenes use (Project Settings > Graphics).</summary>
public readonly record struct EffectsAllowed(bool ToneMapping, bool Bloom, bool Color, bool Vignette, bool DepthFog, bool Occlusion, bool Antialiasing)
{
    public static readonly EffectsAllowed All = new(true, true, true, true, true, true, true);
}

/// <summary>Which post-processing a view shows: whether at all, which effects the project allows, and the camera object whose own asset comes before the scene's.</summary>
public readonly record struct Effects(bool Show, EffectsAllowed Allowed, StableId? Camera)
{
    public static readonly Effects Default = new(true, EffectsAllowed.All, null);
}

/// <summary>Options for <see cref="Extraction.Extract"/>.</summary>
public readonly record struct FrameOptions(bool ShowGrid, StableId? Selected, float Time, ShadowSettings Shadows, Effects Effects)
{
    public static readonly FrameOptions Default = new(false, null, 0, ShadowSettings.Default, Effects.Default);
}

/// <summary>What post-processing a frame gets. The default changes nothing.</summary>
public sealed record PostSettings
{
    /// <summary>Brightness before tone mapping; 1 as drawn.</summary>
    public float Exposure { get; init; } = 1;

    /// <summary>A filmic curve from high-range colour to the screen; off, colours above 1 clip.</summary>
    public bool ToneMapping { get; init; }

    /// <summary>How strongly bright parts glow; 0 for no bloom.</summary>
    public float Bloom { get; init; }

    public float BloomThreshold { get; init; } = 1;

    public float Contrast { get; init; } = 1;

    public float Saturation { get; init; } = 1;

    /// <summary>A colour lookup image, after contrast and saturation; one of another size is not used.</summary>
    public Image? Lookup { get; init; }

    public float LookupStrength { get; init; } = 1;

    public float Vignette { get; init; }

    public float VignetteSmoothness { get; init; } = 0.5f;

    /// <summary>Fog by distance from the camera: linear RGB colour, where it starts and where it hides everything.</summary>
    public (Vector3 Color, float Start, float End)? Fog { get; init; }

    public float Occlusion { get; init; }

    public float OcclusionRadius { get; init; } = 0.5f;

    public Antialiasing Antialiasing { get; init; } = Antialiasing.Off;

    public static readonly PostSettings Default = new();
}

/// <summary>Everything one frame needs, independent of the GPU.</summary>
public sealed record Frame
{
    public required CameraView Camera { get; init; }

    public List<DrawItem> Items { get; init; } = [];

    public List<LightItem> Lights { get; init; } = [];

    /// <summary>Light from above (the sky) that every shape gets.</summary>
    public Vector3 Ambient { get; init; }

    /// <summary>Light from below (the ground).</summary>
    public Vector3 AmbientGround { get; init; }

    public Vector4 ClearColor { get; init; }

    /// <summary>The sky drawn behind the scene, if the scene has one.</summary>
    public Sky? Sky { get; init; }

    /// <summary>Draws the sky (or the clear colour) behind the shapes; off, the background stays transparent.</summary>
    public bool Background { get; init; } = true;

    /// <summary>Fog colour (linear RGB) and density per world unit; density 0 is no fog.</summary>
    public (Vector3 Color, float Density) Fog { get; init; }

    public bool ShowGrid { get; init; }

    public StableId? Selected { get; init; }

    public ShadowSettings Shadows { get; init; } = ShadowSettings.Default;

    public PostSettings Post { get; init; } = PostSettings.Default;

    /// <summary>More selected objects, highlighted like <see cref="Selected"/>.</summary>
    public List<StableId> Highlighted { get; init; } = [];

    /// <summary>Seconds, for animated materials.</summary>
    public float Time { get; init; }
}
