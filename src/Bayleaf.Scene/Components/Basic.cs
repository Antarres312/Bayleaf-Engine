using System.Numerics;

using Bayleaf.Core;
using Bayleaf.Core.Geometry;
using Bayleaf.Core.Json;

namespace Bayleaf.Scene.Components;

// Built-in scene components. Axis convention: right-handed, +Y up. Cameras and lights look along
// their local −Z axis.

/// <summary>Display name of an object.</summary>
public sealed record Name(string Value) : ISceneComponent<Name>
{
    public static string Key => "name";

    public static string Label => "Name";

    public static bool Removable => false;

    public static Name Initial => new("Object");

    public static Name Read(JsonValue value) => new(JsonRead.Str(value));

    public JsonValue Write() => J.S(Value);
}

/// <summary>
/// Position, rotation (Euler angles in degrees, applied X then Y then Z) and scale relative to the
/// parent. The position is double precision, so objects far from the origin keep millimetre accuracy.
/// </summary>
public readonly record struct Transform(Double3 Position, Vector3 Rotation, Vector3 Scale) : ISceneComponent<Transform>
{
    public static string Key => "transform";

    public static string Label => "Transform";

    public static bool Removable => false;

    public static Transform Initial => new(Double3.Zero, Vector3.Zero, Vector3.One);

    public static Transform Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Transform", true, "position", "rotation", "scale");
        return new(f.Required("position", J.ReadD3), f.Required("rotation", J.ReadV3), f.Required("scale", J.ReadV3));
    }

    public JsonValue Write() => new JsonObject { { "position", J.D3(Position) }, { "rotation", J.V3(Rotation) }, { "scale", J.V3(Scale) } };

    private static double Radians(float degrees) => degrees * (Math.PI / 180.0);

    /// <summary>Local-to-parent matrix, in double precision.</summary>
    public DMat4 Matrix() =>
        DMat4.FromScaleRotationTranslation(new Double3(Scale.X, Scale.Y, Scale.Z), DQuat.FromEulerXyz(Radians(Rotation.X), Radians(Rotation.Y), Radians(Rotation.Z)), Position);

    /// <summary>True when every value is a finite number.</summary>
    public bool IsFinite => Position.IsFinite && float.IsFinite(Rotation.X) && float.IsFinite(Rotation.Y) && float.IsFinite(Rotation.Z)
        && float.IsFinite(Scale.X) && float.IsFinite(Scale.Y) && float.IsFinite(Scale.Z);

    /// <summary>The transform a local-to-parent matrix stands for, with -0 made 0.</summary>
    public static Transform FromMatrix(DMat4 matrix)
    {
        var (scale, rotation, position) = matrix.ToScaleRotationTranslation();
        var (rx, ry, rz) = rotation.ToEulerXyz();
        static double Zero(double v) => v == 0 ? 0 : v;
        static float Degrees(double radians) => (float)Zero(radians * (180.0 / Math.PI));
        return new(position.Map(Zero), new Vector3(Degrees(rx), Degrees(ry), Degrees(rz)), new Vector3((float)Zero(scale.X), (float)Zero(scale.Y), (float)Zero(scale.Z)));
    }

    public Result<Unit, string> Validate()
    {
        if (!Position.IsFinite)
        {
            return Checks.Fail("transform values must be finite numbers");
        }
        return Checks.Finite("transform", [.. J.Of(Rotation), .. J.Of(Scale)]);
    }
}

/// <summary>Free-form labels for finding and grouping objects.</summary>
public sealed record Tags(IReadOnlyList<string> Values) : ISceneComponent<Tags>
{
    public static string Key => "tags";

    public static string Label => "Tags";

    public static bool Removable => true;

    public static Tags Initial => new([]);

    public static Tags Read(JsonValue value) => new(JsonRead.List(value, JsonRead.Str));

    public JsonValue Write() => J.Strings(Values);
}

/// <summary>Built-in mesh shapes, all one unit across and centred on the origin.</summary>
public enum Shape
{
    Cube,
    Sphere,

    /// <summary>Flat square in the XZ plane, facing +Y.</summary>
    Plane,

    /// <summary>Flat square in the XY plane, facing +Z.</summary>
    Quad,
}

public static class Shapes
{
    public static readonly IReadOnlyList<string> Names = ["cube", "sphere", "plane", "quad"];

    public static string Code(this Shape shape) => Names[(int)shape];

    public static Shape Read(JsonValue value) => (Shape)JsonRead.Variant(value, [.. Names]);
}

/// <summary>A lit shape with a simple physically based material.</summary>
public sealed record Mesh(Shape Shape, Vector4 Color, float Metallic, float Roughness, string Material, string Model) : ISceneComponent<Mesh>
{
    public static string Key => "mesh";

    public static string Label => "Mesh";

    public static string Menu => "Rendering/Mesh";

    public static bool Removable => true;

    public static Mesh Initial => new(Shape.Cube, new(0.8f, 0.8f, 0.8f, 1f), 0f, 0.5f, "", "");

    public static Mesh Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Mesh", true, "shape", "color", "metallic", "roughness", "material", "model");
        return new(f.Required("shape", Shapes.Read), f.Required("color", J.ReadV4), f.Required("metallic", JsonRead.F32), f.Required("roughness", JsonRead.F32),
            f.Optional("material", JsonRead.Str, ""), f.Optional("model", JsonRead.Str, ""));
    }

    public JsonValue Write()
    {
        var o = new JsonObject { { "shape", J.S(Shape.Code()) }, { "color", J.V4(Color) }, { "metallic", J.F(Metallic) }, { "roughness", J.F(Roughness) } };
        if (Material.Length > 0)
        {
            o.Add("material", J.S(Material));
        }
        if (Model.Length > 0)
        {
            o.Add("model", J.S(Model));
        }
        return o;
    }

    public Result<Unit, string> Validate() => Checks.All(
        () => Checks.UnitRange("color", J.Of(Color)),
        () => Checks.UnitRange("metallic", [Metallic]),
        () => Checks.UnitRange("roughness", [Roughness]),
        () => Checks.FileField("material", Material, "bmaterial"),
        () => Checks.FileField("model", Model, "glb", "gltf", "obj"));

    public static IReadOnlyList<string> Options(string field) => field == "shape" ? Shapes.Names : [];
}

public enum Projection
{
    Perspective,
    Orthographic,
}

/// <summary>A viewpoint the scene can be rendered from. It looks along its local −Z axis.</summary>
public sealed record Camera(Projection Projection, float Fov, float Size, float Near, float Far, string PostProcessing) : ISceneComponent<Camera>
{
    private static readonly string[] Projections = ["perspective", "orthographic"];

    public static string Key => "camera";

    public static string Label => "Camera";

    public static string Menu => "Rendering/Camera";

    public static bool Removable => true;

    public static Camera Initial => new(Projection.Perspective, 60f, 5f, 0.1f, 1000f, "");

    public static Camera Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Camera", true, "projection", "fov", "size", "near", "far", "postProcessing");
        return new((Projection)f.Required("projection", v => JsonRead.Variant(v, Projections)), f.Required("fov", JsonRead.F32), f.Required("size", JsonRead.F32),
            f.Required("near", JsonRead.F32), f.Required("far", JsonRead.F32), f.Optional("postProcessing", JsonRead.Str, ""));
    }

    public JsonValue Write()
    {
        var o = new JsonObject
        {
            { "projection", J.S(Projections[(int)Projection]) },
            { "fov", J.F(Fov) },
            { "size", J.F(Size) },
            { "near", J.F(Near) },
            { "far", J.F(Far) },
        };
        if (PostProcessing.Length > 0)
        {
            o.Add("postProcessing", J.S(PostProcessing));
        }
        return o;
    }

    public Result<Unit, string> Validate()
    {
        var finite = Checks.Finite("camera", [Fov, Size, Near, Far]);
        if (finite.IsError)
        {
            return finite;
        }
        if (!J.Within(Fov, 1f, 179f))
        {
            return Checks.Fail("fov must be between 1 and 179 degrees");
        }
        if (Size <= 0f)
        {
            return Checks.Fail("size must be greater than 0");
        }
        if (Near <= 0f || Far <= Near)
        {
            return Checks.Fail("near must be greater than 0 and far greater than near");
        }
        return SceneSettings.FilePath("postProcessing", PostProcessing, Bayleaf.Scene.PostProcessing.Extension);
    }

    public static IReadOnlyList<string> Options(string field) => field == "projection" ? Projections : [];
}

/// <summary>Light from a distant source, such as the sun. It shines along the object's local −Z axis.</summary>
public sealed record DirectionalLight(Vector3 Color, float Intensity, bool Shadows) : ISceneComponent<DirectionalLight>
{
    public static string Key => "directionalLight";

    public static string Label => "Directional Light";

    public static string Menu => "Light/Directional Light";

    public static bool Removable => true;

    public static DirectionalLight Initial => new(Vector3.One, 3f, true);

    public static DirectionalLight Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct DirectionalLight", true, "color", "intensity", "shadows");
        return new(f.Required("color", J.ReadV3), f.Required("intensity", JsonRead.F32), f.Optional("shadows", JsonRead.Bool, true));
    }

    public JsonValue Write() => new JsonObject { { "color", J.V3(Color) }, { "intensity", J.F(Intensity) }, { "shadows", J.B(Shadows) } };

    public Result<Unit, string> Validate()
    {
        var color = Checks.UnitRange("color", J.Of(Color));
        if (color.IsError)
        {
            return color;
        }
        return float.IsFinite(Intensity) && Intensity >= 0f ? Checks.Ok : Checks.Fail("intensity must be 0 or more");
    }
}

/// <summary>
/// The world around the scene: a sky that fades from the horizon up to the top and down to the
/// ground, and the soft light that sky and ground give to every shape. The first object with one, in
/// hierarchy order, sets it for the scene (older scenes; a lighting asset holds this now).
/// </summary>
public sealed record Environment(Vector3 SkyColor, Vector3 HorizonColor, Vector3 GroundColor, float Ambient, bool ShowSky, float FogDistance) : ISceneComponent<Environment>
{
    public static string Key => "environment";

    public static string Label => "Environment";

    public static string Menu => "Rendering/Environment";

    public static bool Removable => true;

    public static bool Listed => false;

    public static Environment Initial => new(new(0.18f, 0.36f, 0.72f), new(0.62f, 0.72f, 0.82f), new(0.22f, 0.2f, 0.18f), 1f, true, 0f);

    public static Environment Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Environment", true, "skyColor", "horizonColor", "groundColor", "ambient", "showSky", "fogDistance");
        return new(f.Required("skyColor", J.ReadV3), f.Required("horizonColor", J.ReadV3), f.Required("groundColor", J.ReadV3), f.Required("ambient", JsonRead.F32),
            f.Required("showSky", JsonRead.Bool), f.Optional("fogDistance", JsonRead.F32, 0f));
    }

    public JsonValue Write() => new JsonObject
    {
        { "skyColor", J.V3(SkyColor) },
        { "horizonColor", J.V3(HorizonColor) },
        { "groundColor", J.V3(GroundColor) },
        { "ambient", J.F(Ambient) },
        { "showSky", J.B(ShowSky) },
        { "fogDistance", J.F(FogDistance) },
    };

    public Result<Unit, string> Validate()
    {
        var colors = Checks.All(
            () => Checks.UnitRange("skyColor", J.Of(SkyColor)),
            () => Checks.UnitRange("horizonColor", J.Of(HorizonColor)),
            () => Checks.UnitRange("groundColor", J.Of(GroundColor)));
        if (colors.IsError)
        {
            return colors;
        }
        if (!J.Within(Ambient, 0f, 4f))
        {
            return Checks.Fail("ambient must be between 0 and 4");
        }
        return FogDistance == 0f || J.Within(FogDistance, 0.1f, 100_000f) ? Checks.Ok : Checks.Fail("fogDistance must be 0 (no fog) or between 0.1 and 100000");
    }
}

/// <summary>Light from a point that shines in every direction and fades out by its range, like a bulb.</summary>
public sealed record PointLight(Vector3 Color, float Intensity, float Range, bool Shadows) : ISceneComponent<PointLight>
{
    public static string Key => "pointLight";

    public static string Label => "Point Light";

    public static string Menu => "Light/Point Light";

    public static bool Removable => true;

    public static PointLight Initial => new(new(1f, 0.95f, 0.85f), 8f, 8f, false);

    public static PointLight Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct PointLight", true, "color", "intensity", "range", "shadows");
        return new(f.Required("color", J.ReadV3), f.Required("intensity", JsonRead.F32), f.Required("range", JsonRead.F32), f.Optional("shadows", JsonRead.Bool, false));
    }

    public JsonValue Write() => new JsonObject { { "color", J.V3(Color) }, { "intensity", J.F(Intensity) }, { "range", J.F(Range) }, { "shadows", J.B(Shadows) } };

    public Result<Unit, string> Validate() => Checks.All(() => Checks.UnitRange("color", J.Of(Color)), () => Checks.LightAmounts(Intensity, Range));
}

/// <summary>Light from a point that shines in a cone along the object's local −Z axis, like a torch.</summary>
public sealed record SpotLight(Vector3 Color, float Intensity, float Range, float Angle, float Softness, bool Shadows) : ISceneComponent<SpotLight>
{
    public static string Key => "spotLight";

    public static string Label => "Spot Light";

    public static string Menu => "Light/Spot Light";

    public static bool Removable => true;

    public static SpotLight Initial => new(Vector3.One, 12f, 12f, 45f, 0.3f, false);

    public static SpotLight Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct SpotLight", true, "color", "intensity", "range", "angle", "softness", "shadows");
        return new(f.Required("color", J.ReadV3), f.Required("intensity", JsonRead.F32), f.Required("range", JsonRead.F32), f.Required("angle", JsonRead.F32),
            f.Required("softness", JsonRead.F32), f.Optional("shadows", JsonRead.Bool, false));
    }

    public JsonValue Write() => new JsonObject
    {
        { "color", J.V3(Color) },
        { "intensity", J.F(Intensity) },
        { "range", J.F(Range) },
        { "angle", J.F(Angle) },
        { "softness", J.F(Softness) },
        { "shadows", J.B(Shadows) },
    };

    public Result<Unit, string> Validate() => Checks.All(
        () => Checks.UnitRange("color", J.Of(Color)),
        () => Checks.UnitRange("softness", [Softness]),
        () => Checks.LightAmounts(Intensity, Range),
        () => float.IsFinite(Angle) && J.Within(Angle, 1f, 179f) ? Checks.Ok : Checks.Fail("angle must be between 1 and 179 degrees"));
}

/// <summary>An unlit flat rectangle in the object's XY plane, for 2D games and markers.</summary>
public sealed record Sprite(Vector4 Color, Vector2 Size, string Image) : ISceneComponent<Sprite>
{
    public static string Key => "sprite";

    public static string Label => "Sprite";

    public static string Menu => "Rendering/Sprite";

    public static bool Removable => true;

    public static Sprite Initial => new(Vector4.One, Vector2.One, "");

    public static Sprite Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Sprite", true, "color", "size", "image");
        return new(f.Required("color", J.ReadV4), f.Required("size", J.ReadV2), f.Optional("image", JsonRead.Str, ""));
    }

    public JsonValue Write()
    {
        var o = new JsonObject { { "color", J.V4(Color) }, { "size", J.V2(Size) } };
        if (Image.Length > 0)
        {
            o.Add("image", J.S(Image));
        }
        return o;
    }

    public Result<Unit, string> Validate() => Checks.All(
        () => Checks.UnitRange("color", J.Of(Color)),
        () => Checks.FileField("image", Image, "png", "jpg", "jpeg"),
        () => float.IsFinite(Size.X) && Size.X > 0f && float.IsFinite(Size.Y) && Size.Y > 0f ? Checks.Ok : Checks.Fail("size must be greater than 0"));
}
