using System.Globalization;
using System.Numerics;

using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Scene.Components;

/// <summary>
/// Marks an object as a placed copy of a prefab file. The object and its children came from the
/// file; Apply writes them back to it, Revert reads them again.
/// </summary>
public sealed record Prefab(string Source, IReadOnlyList<string> Overrides) : ISceneComponent<Prefab>
{
    /// <summary>Most overrides one prefab copy keeps.</summary>
    public const int MaxOverrides = 1000;

    public Prefab(string source)
        : this(source, [])
    {
    }

    public static string Key => "prefab";

    public static string Label => "Prefab";

    public static bool Removable => true;

    public static bool Listed => false;

    public static Prefab Initial => new("", []);

    public bool Equals(Prefab? other) => other is not null && Source == other.Source && Overrides.SequenceEqual(other.Overrides);

    public override int GetHashCode() => StringComparer.Ordinal.GetHashCode(Source);

    public static Prefab Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Prefab", true, "source", "overrides");
        return new(f.Required("source", JsonRead.Str), f.Optional("overrides", v => (IReadOnlyList<string>)JsonRead.List(v, JsonRead.Str), []));
    }

    public JsonValue Write()
    {
        var o = new JsonObject { { "source", J.S(Source) } };
        if (Overrides.Count > 0)
        {
            o.Add("overrides", J.Strings(Overrides));
        }
        return o;
    }

    /// <summary>The object index and component name of each override (<c>"&lt;index&gt;/&lt;component&gt;"</c>).</summary>
    public IEnumerable<(int Index, string Component)> ParsedOverrides() =>
        Overrides.Select(o => o.Split('/', 2)).Where(p => p.Length == 2 && int.TryParse(p[0], NumberStyles.None, CultureInfo.InvariantCulture, out _))
            .Select(p => (int.Parse(p[0], NumberStyles.None, CultureInfo.InvariantCulture), p[1]));

    public Result<Unit, string> Validate()
    {
        if (Source.Length == 0 || Text.Bytes(Source) > 512 || !Source.EndsWith(".bprefab", StringComparison.Ordinal))
        {
            return Checks.Fail("source must be the path of a .bprefab file (at most 512 characters)");
        }
        static bool WellFormed(string o) => Text.Bytes(o) <= 80 && o.Split('/', 2) is [var i, var c]
            && ulong.TryParse(i, NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out _) && !i.StartsWith('-') && c.Length > 0 && c.All(char.IsAsciiLetterOrDigit);
        return Overrides.Count <= MaxOverrides && Overrides.All(WellFormed) ? Checks.Ok : Checks.Fail($"overrides must be at most {MaxOverrides} entries like \"2/mesh\"");
    }
}

/// <summary>How a body moves in play mode.</summary>
public enum BodyKind
{
    /// <summary>Moved by gravity, collisions and impulses.</summary>
    Dynamic,

    /// <summary>Moved only by its transform (scripts, animation); pushes dynamic bodies.</summary>
    Kinematic,

    /// <summary>Never moves.</summary>
    Static,
}

/// <summary>Makes the object a physics body in play mode. Its Collider gives it a shape.</summary>
public sealed record RigidBody(BodyKind Kind, float Mass, float GravityScale, bool LockRotation) : ISceneComponent<RigidBody>
{
    private static readonly string[] Kinds = ["dynamic", "kinematic", "static"];

    public static string Key => "rigidBody";

    public static string Label => "Rigid Body";

    public static string Menu => "Physics/Rigid Body";

    public static bool Removable => true;

    public static RigidBody Initial => new(BodyKind.Dynamic, 1f, 1f, false);

    public static RigidBody Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct RigidBody", true, "kind", "mass", "gravityScale", "lockRotation");
        return new((BodyKind)f.Required("kind", v => JsonRead.Variant(v, Kinds)), f.Required("mass", JsonRead.F32), f.Required("gravityScale", JsonRead.F32), f.Required("lockRotation", JsonRead.Bool));
    }

    public JsonValue Write() => new JsonObject { { "kind", J.S(Kinds[(int)Kind]) }, { "mass", J.F(Mass) }, { "gravityScale", J.F(GravityScale) }, { "lockRotation", J.B(LockRotation) } };

    public Result<Unit, string> Validate()
    {
        if (!(float.IsFinite(Mass) && Mass > 0f && Mass <= 1.0e6f))
        {
            return Checks.Fail("mass must be more than 0 and at most 1000000");
        }
        return float.IsFinite(GravityScale) && J.Within(GravityScale, -100f, 100f) ? Checks.Ok : Checks.Fail("gravityScale must be between -100 and 100");
    }

    public static IReadOnlyList<string> Options(string field) => field == "kind" ? Kinds : [];
}

/// <summary>A collider's shape, in the object's own space before its scale.</summary>
public enum ColliderShape
{
    /// <summary>The size is the box's width, height and depth.</summary>
    Box,

    /// <summary>The size's first number is the diameter.</summary>
    Sphere,

    /// <summary>Upright: the first number is the diameter, the second the whole height.</summary>
    Capsule,
}

/// <summary>
/// The shape physics uses for the object: what it collides with, or, as a trigger, what it notices
/// entering and leaving it. Without a Rigid Body the object is a static obstacle.
/// </summary>
public sealed record Collider(ColliderShape Shape, Vector3 Size, float Friction, float Restitution, bool Trigger) : ISceneComponent<Collider>
{
    private static readonly string[] Shapes = ["box", "sphere", "capsule"];

    public static string Key => "collider";

    public static string Label => "Collider";

    public static string Menu => "Physics/Collider";

    public static bool Removable => true;

    public static Collider Initial => new(ColliderShape.Box, Vector3.One, 0.5f, 0f, false);

    public static Collider Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Collider", true, "shape", "size", "friction", "restitution", "trigger");
        return new((ColliderShape)f.Required("shape", v => JsonRead.Variant(v, Shapes)), f.Required("size", J.ReadV3), f.Required("friction", JsonRead.F32),
            f.Required("restitution", JsonRead.F32), f.Required("trigger", JsonRead.Bool));
    }

    public JsonValue Write() => new JsonObject
    {
        { "shape", J.S(Shapes[(int)Shape]) },
        { "size", J.V3(Size) },
        { "friction", J.F(Friction) },
        { "restitution", J.F(Restitution) },
        { "trigger", J.B(Trigger) },
    };

    public Result<Unit, string> Validate()
    {
        if (!J.Of(Size).All(s => float.IsFinite(s) && s > 0f && s <= 10_000f))
        {
            return Checks.Fail("size values must be more than 0 and at most 10000");
        }
        if (!(float.IsFinite(Friction) && J.Within(Friction, 0f, 10f)))
        {
            return Checks.Fail("friction must be between 0 and 10");
        }
        return Checks.UnitRange("restitution", [Restitution]);
    }

    public static IReadOnlyList<string> Options(string field) => field == "shape" ? Shapes : [];
}

/// <summary>A moment of a clip that becomes a script event (a footstep).</summary>
public sealed record ClipMark(string Clip, float Time, string Event)
{
    public static ClipMark Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct ClipMark", true, "clip", "time", "event");
        return new(f.Required("clip", JsonRead.Str), f.Required("time", JsonRead.F32), f.Required("event", JsonRead.Str));
    }

    public JsonValue Write() => new JsonObject { { "clip", J.S(Clip) }, { "time", J.F(Time) }, { "event", J.S(Event) } };
}

/// <summary>
/// Plays one of the animations of the object's model (its Mesh's glTF model): its parts and skin move
/// as the animation says, from when the view's clock starts.
/// </summary>
public sealed record ModelAnimation(string Animation, bool Playing, bool Looping, float Speed, string States, bool RootMotion, string RootJoint, IReadOnlyList<ClipMark> Events)
    : ISceneComponent<ModelAnimation>
{
    /// <summary>Most marked moments one Model Animation holds.</summary>
    public const int MaxClipMarks = 256;

    public static string Key => "modelAnimation";

    public static string Label => "Model Animation";

    public static string Menu => "Rendering/Model Animation";

    public static bool Removable => true;

    public static ModelAnimation Initial => new("", true, true, 1f, "", false, "", []);

    public bool Equals(ModelAnimation? other) => other is not null && Animation == other.Animation && Playing == other.Playing && Looping == other.Looping
        && Speed.Equals(other.Speed) && States == other.States && RootMotion == other.RootMotion && RootJoint == other.RootJoint && Events.SequenceEqual(other.Events);

    public override int GetHashCode() => StringComparer.Ordinal.GetHashCode(Animation);

    public static ModelAnimation Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct ModelAnimation", true, "animation", "playing", "looping", "speed", "states", "rootMotion", "rootJoint", "events");
        return new(f.Required("animation", JsonRead.Str), f.Required("playing", JsonRead.Bool), f.Required("looping", JsonRead.Bool), f.Required("speed", JsonRead.F32),
            f.Optional("states", JsonRead.Str, ""), f.Optional("rootMotion", JsonRead.Bool, false), f.Optional("rootJoint", JsonRead.Str, ""),
            f.Optional("events", v => (IReadOnlyList<ClipMark>)JsonRead.List(v, ClipMark.Read), []));
    }

    public JsonValue Write()
    {
        var o = new JsonObject { { "animation", J.S(Animation) }, { "playing", J.B(Playing) }, { "looping", J.B(Looping) }, { "speed", J.F(Speed) } };
        if (States.Length > 0)
        {
            o.Add("states", J.S(States));
        }
        o.Add("rootMotion", J.B(RootMotion));
        o.Add("rootJoint", J.S(RootJoint));
        o.Add("events", new JsonArray(Events.Select(e => e.Write())));
        return o;
    }

    public Result<Unit, string> Validate()
    {
        if (Text.Bytes(Animation) > 256)
        {
            return Checks.Fail("animation is longer than 256 bytes");
        }
        if (Text.Bytes(States) > 1024 || !(States.Length == 0 || States.EndsWith(".banim", StringComparison.Ordinal)))
        {
            return Checks.Fail("states must be a .banim file of at most 1024 bytes, or empty");
        }
        if (Text.Bytes(RootJoint) > 256)
        {
            return Checks.Fail("rootJoint is longer than 256 bytes");
        }
        if (Events.Count > MaxClipMarks)
        {
            return Checks.Fail($"at most {MaxClipMarks} events");
        }
        foreach (var mark in Events)
        {
            if (Text.Bytes(mark.Clip) > 256 || mark.Event.Length == 0 || Text.Bytes(mark.Event) > 64 || Text.HasControl(mark.Event))
            {
                return Checks.Fail("each event needs a clip name of at most 256 bytes and an event name of 1 to 64 bytes");
            }
            if (!(float.IsFinite(mark.Time) && J.Within(mark.Time, 0f, 100_000f)))
            {
                return Checks.Fail("each event's time must be from 0 to 100000 seconds");
            }
        }
        return float.IsFinite(Speed) && J.Within(Speed, 0f, 10f) ? Checks.Ok : Checks.Fail("speed must be between 0 and 10");
    }
}

/// <summary>
/// Plays a keyframe animation (<c>.btimeline</c>) on the parts of the prefab this object is the root
/// of, in play mode: from the start, or when a script says.
/// </summary>
public sealed record KeyframeAnimation(string Animation, bool PlayOnStart, float Speed) : ISceneComponent<KeyframeAnimation>
{
    public static string Key => "keyframeAnimation";

    public static string Label => "Keyframe Animation";

    public static string Menu => "Rendering/Keyframe Animation";

    public static bool Removable => true;

    public static KeyframeAnimation Initial => new("", true, 1f);

    public static KeyframeAnimation Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct KeyframeAnimation", true, "animation", "playOnStart", "speed");
        return new(f.Required("animation", JsonRead.Str), f.Required("playOnStart", JsonRead.Bool), f.Required("speed", JsonRead.F32));
    }

    public JsonValue Write() => new JsonObject { { "animation", J.S(Animation) }, { "playOnStart", J.B(PlayOnStart) }, { "speed", J.F(Speed) } };

    public Result<Unit, string> Validate()
    {
        if (Text.Bytes(Animation) > 1024 || Text.HasNul(Animation))
        {
            return Checks.Fail("animation is a path of at most 1024 bytes, without NUL");
        }
        if (!(Animation.Length == 0 || Animation.EndsWith($".{Timeline.Extension}", StringComparison.Ordinal)))
        {
            return Checks.Fail($"animation is a .{Timeline.Extension} file");
        }
        return float.IsFinite(Speed) && J.Within(Speed, 0f, 10f) ? Checks.Ok : Checks.Fail("speed must be between 0 and 10");
    }
}

/// <summary>
/// Follows a joint of its parent's model (a sword in the hand): the object stands where the joint is,
/// as the parent's animation moves it, and its own Transform is measured from the joint.
/// </summary>
public sealed record FollowJoint(string Joint) : ISceneComponent<FollowJoint>
{
    public static string Key => "followJoint";

    public static string Label => "Follow Joint";

    public static string Menu => "Rendering/Follow Joint";

    public static bool Removable => true;

    public static FollowJoint Initial => new("");

    public static FollowJoint Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct FollowJoint", true, "joint");
        return new(f.Required("joint", JsonRead.Str));
    }

    public JsonValue Write() => new JsonObject { { "joint", J.S(Joint) } };

    public Result<Unit, string> Validate() => Text.Bytes(Joint) > 256 ? Checks.Fail("joint is longer than 256 bytes") : Checks.Ok;
}

/// <summary>
/// A sound the object plays in play mode: from its own place when spatial, heard from the first camera
/// (quieter with distance, from the side it is on), or the same everywhere.
/// </summary>
public sealed record AudioSource(string Clip, float Volume, bool Looping, bool PlayOnStart, bool Spatial, float Range) : ISceneComponent<AudioSource>
{
    public static string Key => "audioSource";

    public static string Label => "Audio Source";

    public static string Menu => "Audio/Audio Source";

    public static bool Removable => true;

    public static AudioSource Initial => new("", 1f, false, true, true, 30f);

    public static AudioSource Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct AudioSource", true, "clip", "volume", "looping", "playOnStart", "spatial", "range");
        return new(f.Required("clip", JsonRead.Str), f.Required("volume", JsonRead.F32), f.Required("looping", JsonRead.Bool), f.Required("playOnStart", JsonRead.Bool),
            f.Required("spatial", JsonRead.Bool), f.Required("range", JsonRead.F32));
    }

    public JsonValue Write() => new JsonObject
    {
        { "clip", J.S(Clip) },
        { "volume", J.F(Volume) },
        { "looping", J.B(Looping) },
        { "playOnStart", J.B(PlayOnStart) },
        { "spatial", J.B(Spatial) },
        { "range", J.F(Range) },
    };

    public Result<Unit, string> Validate()
    {
        var clip = Checks.FileField("clip", Clip, [.. PropertyTypes.SoundExtensions]);
        if (clip.IsError)
        {
            return clip;
        }
        if (!(float.IsFinite(Volume) && J.Within(Volume, 0f, 4f)))
        {
            return Checks.Fail("volume must be between 0 and 4");
        }
        return float.IsFinite(Range) && Range > 0f && Range <= 100_000f ? Checks.Ok : Checks.Fail("range must be more than 0 and at most 100000");
    }
}

/// <summary>
/// A game interface over the game: a layout file (<c>.bui</c>) of panels, text, buttons and other
/// elements, whose controls send the object's scripts messages.
/// </summary>
public sealed record GameInterface(string Layout, bool Visible, int Order, string Space, Vector2 Size, float PointsPerUnit, bool FaceCamera) : ISceneComponent<GameInterface>
{
    /// <summary>Largest side of a world panel's picture, in points.</summary>
    public const float MaxPanelPoints = 2048f;

    private static readonly string[] Spaces = ["screen", "world"];

    public static string Key => "interface";

    public static string Label => "Interface";

    public static string Menu => "Interface/Interface";

    public static bool Removable => true;

    public static GameInterface Initial => new("", true, 0, "screen", new(2f, 1f), 200f, false);

    public static GameInterface Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Interface", true, "layout", "visible", "order", "space", "size", "pointsPerUnit", "faceCamera");
        return new(f.Required("layout", JsonRead.Str), f.Required("visible", JsonRead.Bool), f.Required("order", JsonRead.I32), f.Optional("space", JsonRead.Str, "screen"),
            f.Optional("size", J.ReadV2, new Vector2(2f, 1f)), f.Optional("pointsPerUnit", JsonRead.F32, 200f), f.Optional("faceCamera", JsonRead.Bool, false));
    }

    public JsonValue Write() => new JsonObject
    {
        { "layout", J.S(Layout) },
        { "visible", J.B(Visible) },
        { "order", J.I(Order) },
        { "space", J.S(Space) },
        { "size", J.V2(Size) },
        { "pointsPerUnit", J.F(PointsPerUnit) },
        { "faceCamera", J.B(FaceCamera) },
    };

    public Result<Unit, string> Validate()
    {
        var layout = Checks.FileField("layout", Layout, "bui");
        if (layout.IsError)
        {
            return layout;
        }
        if (Order is < -1000 or > 1000)
        {
            return Checks.Fail("order must be between -1000 and 1000");
        }
        if (!Spaces.Contains(Space, StringComparer.Ordinal))
        {
            return Checks.Fail("space is screen or world");
        }
        if (!J.Of(Size).All(v => float.IsFinite(v) && J.Within(v, 0.01f, 10_000f)))
        {
            return Checks.Fail("size is two numbers from 0.01 to 10000 world units");
        }
        return float.IsFinite(PointsPerUnit) && J.Within(PointsPerUnit, 1f, 10_000f) ? Checks.Ok : Checks.Fail("pointsPerUnit is from 1 to 10000");
    }

    public static IReadOnlyList<string> Options(string field) => field == "space" ? Spaces : [];
}

/// <summary>
/// A part of the world kept in its own scene file and added under the object while playing, in the
/// background, when a camera comes within Load Distance; it goes again once every camera is further
/// than Load Distance and Keep Margin.
/// </summary>
public sealed record StreamedArea(string Scene, double LoadDistance, double KeepMargin) : ISceneComponent<StreamedArea>
{
    public static string Key => "streamedArea";

    public static string Label => "Streamed Area";

    public static string Menu => "World/Streamed Area";

    public static bool Removable => true;

    public static StreamedArea Initial => new("", 100.0, 20.0);

    public static StreamedArea Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct StreamedArea", true, "scene", "loadDistance", "keepMargin");
        return new(f.Required("scene", JsonRead.Str), f.Required("loadDistance", JsonRead.F64), f.Required("keepMargin", JsonRead.F64));
    }

    public JsonValue Write() => new JsonObject { { "scene", J.S(Scene) }, { "loadDistance", J.D(LoadDistance) }, { "keepMargin", J.D(KeepMargin) } };

    public Result<Unit, string> Validate()
    {
        var scene = Checks.FileField("scene", Scene, "bscene");
        if (scene.IsError)
        {
            return scene;
        }
        if (!(double.IsFinite(LoadDistance) && LoadDistance > 0 && LoadDistance <= 1.0e7))
        {
            return Checks.Fail("load distance must be more than 0 and at most 10000000");
        }
        return double.IsFinite(KeepMargin) && J.Within(KeepMargin, 0.0, 1.0e7) ? Checks.Ok : Checks.Fail("keep margin must be between 0 and 10000000");
    }
}

/// <summary>How a joint lets its object move against what it holds on to.</summary>
public enum JointKind
{
    /// <summary>Held fast, as if glued.</summary>
    Fixed,

    /// <summary>Turns about the axis only, like a door.</summary>
    Hinge,

    /// <summary>Turns every way about the anchor, like a shoulder.</summary>
    Ball,

    /// <summary>Slides along the axis only, like a drawer.</summary>
    Slider,
}

/// <summary>Holds the object's physics body to another object's, or to the world, at the anchor.</summary>
public sealed record Joint(JointKind Kind, string Connected, Vector3 Anchor, Vector3 Axis, bool Limited, float Lower, float Upper, float MotorSpeed, float MotorForce)
    : ISceneComponent<Joint>
{
    private static readonly string[] Kinds = ["fixed", "hinge", "ball", "slider"];

    public static string Key => "joint";

    public static string Label => "Joint";

    public static string Menu => "Physics/Joint";

    public static bool Removable => true;

    public static Joint Initial => new(JointKind.Hinge, "", Vector3.Zero, new(0f, 1f, 0f), false, -45f, 45f, 0f, 0f);

    public static Joint Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct Joint", true, "kind", "connected", "anchor", "axis", "limited", "lower", "upper", "motorSpeed", "motorForce");
        return new((JointKind)f.Required("kind", v => JsonRead.Variant(v, Kinds)), f.Required("connected", JsonRead.Str), f.Required("anchor", J.ReadV3), f.Required("axis", J.ReadV3),
            f.Required("limited", JsonRead.Bool), f.Required("lower", JsonRead.F32), f.Required("upper", JsonRead.F32), f.Required("motorSpeed", JsonRead.F32), f.Required("motorForce", JsonRead.F32));
    }

    public JsonValue Write() => new JsonObject
    {
        { "kind", J.S(Kinds[(int)Kind]) },
        { "connected", J.S(Connected) },
        { "anchor", J.V3(Anchor) },
        { "axis", J.V3(Axis) },
        { "limited", J.B(Limited) },
        { "lower", J.F(Lower) },
        { "upper", J.F(Upper) },
        { "motorSpeed", J.F(MotorSpeed) },
        { "motorForce", J.F(MotorForce) },
    };

    public Result<Unit, string> Validate()
    {
        static bool Finite(IEnumerable<float> v) => v.All(x => float.IsFinite(x) && Math.Abs(x) <= 1.0e6f);
        if (!Finite(J.Of(Anchor)) || !Finite(J.Of(Axis)) || !Finite([Lower, Upper, MotorSpeed, MotorForce]))
        {
            return Checks.Fail("joint values must be finite numbers of at most a million");
        }
        if (Axis.X * Axis.X + Axis.Y * Axis.Y + Axis.Z * Axis.Z < 1.0e-6f)
        {
            return Checks.Fail("the axis must not be zero");
        }
        if (Lower > Upper)
        {
            return Checks.Fail("lower must not be more than upper");
        }
        if (MotorForce < 0f)
        {
            return Checks.Fail("motorForce must be 0 or more");
        }
        return Text.Chars(Connected) > 256 ? Checks.Fail("connected must be a name of at most 256 characters") : Checks.Ok;
    }

    public static IReadOnlyList<string> Options(string field) => field == "kind" ? Kinds : [];
}

/// <summary>
/// A walking character for play mode: an upright capsule that scripts steer and jump, which climbs
/// steps and gentle slopes, slides along walls and falls under gravity.
/// </summary>
public sealed record CharacterBody(float Height, float Radius, float MaxSlope, float StepHeight) : ISceneComponent<CharacterBody>
{
    public static string Key => "characterBody";

    public static string Label => "Character";

    public static string Menu => "Physics/Character";

    public static bool Removable => true;

    public static CharacterBody Initial => new(1.8f, 0.4f, 45f, 0.3f);

    public static CharacterBody Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct CharacterBody", true, "height", "radius", "maxSlope", "stepHeight");
        return new(f.Required("height", JsonRead.F32), f.Required("radius", JsonRead.F32), f.Required("maxSlope", JsonRead.F32), f.Required("stepHeight", JsonRead.F32));
    }

    public JsonValue Write() => new JsonObject { { "height", J.F(Height) }, { "radius", J.F(Radius) }, { "maxSlope", J.F(MaxSlope) }, { "stepHeight", J.F(StepHeight) } };

    public Result<Unit, string> Validate()
    {
        if (!(float.IsFinite(Radius) && Radius > 0f && Radius <= 100f && float.IsFinite(Height) && Height >= Radius * 2f && Height <= 1000f))
        {
            return Checks.Fail("radius must be more than 0 (at most 100) and height at least twice the radius (at most 1000)");
        }
        if (!(float.IsFinite(MaxSlope) && J.Within(MaxSlope, 0f, 89f)))
        {
            return Checks.Fail("maxSlope must be between 0 and 89 degrees");
        }
        return float.IsFinite(StepHeight) && StepHeight >= 0f && StepHeight <= Height ? Checks.Ok : Checks.Fail("stepHeight must be between 0 and the height");
    }
}

/// <summary>How a Player Control places the camera.</summary>
public enum CameraFollow
{
    /// <summary>The camera stays where the scene puts it.</summary>
    None,

    /// <summary>The scene's first camera trails the object at a distance and height, looking at it.</summary>
    Follow,
}

/// <summary>Lets the player steer this object's Character with the project's input actions, relative to where the camera looks.</summary>
public sealed record PlayerControl(float Speed, float JumpSpeed, bool TurnToMove, CameraFollow Camera, float CameraDistance, float CameraHeight) : ISceneComponent<PlayerControl>
{
    private static readonly string[] Follows = ["none", "follow"];

    public static string Key => "playerControl";

    public static string Label => "Player Control";

    public static string Menu => "Gameplay/Player Control";

    public static bool Removable => true;

    public static PlayerControl Initial => new(5f, 5f, true, CameraFollow.Follow, 6f, 3f);

    public static PlayerControl Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct PlayerControl", true, "speed", "jumpSpeed", "turnToMove", "camera", "cameraDistance", "cameraHeight");
        return new(f.Required("speed", JsonRead.F32), f.Required("jumpSpeed", JsonRead.F32), f.Required("turnToMove", JsonRead.Bool),
            (CameraFollow)f.Required("camera", v => JsonRead.Variant(v, Follows)), f.Required("cameraDistance", JsonRead.F32), f.Required("cameraHeight", JsonRead.F32));
    }

    public JsonValue Write() => new JsonObject
    {
        { "speed", J.F(Speed) },
        { "jumpSpeed", J.F(JumpSpeed) },
        { "turnToMove", J.B(TurnToMove) },
        { "camera", J.S(Follows[(int)Camera]) },
        { "cameraDistance", J.F(CameraDistance) },
        { "cameraHeight", J.F(CameraHeight) },
    };

    public Result<Unit, string> Validate()
    {
        static bool Ranged(float v, float max) => float.IsFinite(v) && J.Within(v, 0f, max);
        if (!(Ranged(Speed, 1000f) && Ranged(JumpSpeed, 1000f)))
        {
            return Checks.Fail("speed and jumpSpeed must be between 0 and 1000");
        }
        return Ranged(CameraDistance, 10_000f) && float.IsFinite(CameraHeight) && Math.Abs(CameraHeight) <= 10_000f
            ? Checks.Ok
            : Checks.Fail("cameraDistance must be between 0 and 10000 and cameraHeight at most 10000 either way");
    }

    public static IReadOnlyList<string> Options(string field) => field == "camera" ? Follows : [];
}

/// <summary>What happens when the time limit runs out.</summary>
public enum TimeUp
{
    /// <summary>The game is lost.</summary>
    Lose,

    /// <summary>The game is won (survive until the time is up).</summary>
    Win,
}

/// <summary>The rules of the game in this scene: who the player is, where it starts, when it starts over, and how long the game lasts.</summary>
public sealed record GameRules(string PlayerPrefab, bool Respawn, float RespawnHeight, float TimeLimit, TimeUp TimeUp) : ISceneComponent<GameRules>
{
    private static readonly string[] Ends = ["lose", "win"];

    public static string Key => "gameRules";

    public static string Label => "Game Rules";

    public static string Menu => "Gameplay/Game Rules";

    public static bool Removable => true;

    public static GameRules Initial => new("", true, -20f, 0f, TimeUp.Lose);

    public static GameRules Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct GameRules", true, "playerPrefab", "respawn", "respawnHeight", "timeLimit", "timeUp");
        return new(f.Required("playerPrefab", JsonRead.Str), f.Required("respawn", JsonRead.Bool), f.Required("respawnHeight", JsonRead.F32), f.Required("timeLimit", JsonRead.F32),
            (TimeUp)f.Required("timeUp", v => JsonRead.Variant(v, Ends)));
    }

    public JsonValue Write() => new JsonObject
    {
        { "playerPrefab", J.S(PlayerPrefab) },
        { "respawn", J.B(Respawn) },
        { "respawnHeight", J.F(RespawnHeight) },
        { "timeLimit", J.F(TimeLimit) },
        { "timeUp", J.S(Ends[(int)TimeUp]) },
    };

    public Result<Unit, string> Validate()
    {
        var prefab = Checks.FileField("playerPrefab", PlayerPrefab, "bprefab");
        if (prefab.IsError)
        {
            return prefab;
        }
        if (!float.IsFinite(RespawnHeight))
        {
            return Checks.Fail("respawnHeight must be a finite number");
        }
        return float.IsFinite(TimeLimit) && J.Within(TimeLimit, 0f, 1.0e7f) ? Checks.Ok : Checks.Fail("timeLimit must be 0 (none) or a number of seconds up to 10000000");
    }

    public static IReadOnlyList<string> Options(string field) => field == "timeUp" ? Ends : [];
}

/// <summary>Where the player appears when play begins and when it starts over: this object's place and turn.</summary>
public sealed record PlayerStart : ISceneComponent<PlayerStart>
{
    public static string Key => "playerStart";

    public static string Label => "Player Start";

    public static string Menu => "Gameplay/Player Start";

    public static bool Removable => true;

    public static PlayerStart Initial => new();

    public static PlayerStart Read(JsonValue value)
    {
        JsonFields.Of(value, "struct PlayerStart", true);
        return new();
    }

    public JsonValue Write() => new JsonObject();
}

/// <summary>This object's own Input Map: its scripts and Player Control read their actions from this asset instead of the project's.</summary>
public sealed record InputMap(string Asset, string Start) : ISceneComponent<InputMap>
{
    public static string Key => "inputMap";

    public static string Label => "Input Map";

    public static string Menu => "Input/Input Map";

    public static bool Removable => true;

    public static InputMap Initial => new("", "");

    public static InputMap Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct InputMap", true, "asset", "start");
        return new(f.Required("asset", JsonRead.Str), f.Optional("start", JsonRead.Str, ""));
    }

    public JsonValue Write() => new JsonObject { { "asset", J.S(Asset) }, { "start", J.S(Start) } };

    public Result<Unit, string> Validate()
    {
        var asset = Checks.FileField("asset", Asset, "binput");
        if (asset.IsError)
        {
            return asset;
        }
        return Text.Bytes(Start) > 64 || !Start.All(c => char.IsAsciiLetterOrDigit(c) || c == '_') ? Checks.Fail("start must be a map name: up to 64 letters, digits or _") : Checks.Ok;
    }
}

/// <summary>A point of a particle's size over its life.</summary>
public readonly record struct SizeKey(float At, float Size)
{
    public static SizeKey Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct SizeKey", true, "at", "size");
        return new(f.Required("at", JsonRead.F32), f.Required("size", JsonRead.F32));
    }

    public JsonValue Write() => new JsonObject { { "at", J.F(At) }, { "size", J.F(Size) } };
}

/// <summary>A point of a particle's colour over its life.</summary>
public readonly record struct ColorKey(float At, Vector4 Color)
{
    public static ColorKey Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct ColorKey", true, "at", "color");
        return new(f.Required("at", JsonRead.F32), f.Required("color", J.ReadV4));
    }

    public JsonValue Write() => new JsonObject { { "at", J.F(At) }, { "color", J.V4(Color) } };
}

/// <summary>
/// Particles: many small images (or shapes) that start from the object, move, change size and colour
/// over their life, and go.
/// </summary>
public sealed record ParticleEmitter : ISceneComponent<ParticleEmitter>
{
    /// <summary>Most points in a size or colour curve.</summary>
    public const int MaxKeys = 16;

    /// <summary>Most particles one emitter may keep alive.</summary>
    public const uint MaxParticles = 10_000;

    public bool Playing { get; init; } = true;

    public bool Looping { get; init; } = true;

    public float Duration { get; init; } = 5f;

    public float Rate { get; init; } = 10f;

    public uint Burst { get; init; }

    public uint MaxParticlesAlive { get; init; } = 500;

    public string Shape { get; init; } = "cone";

    public float Radius { get; init; } = 0.2f;

    public float Angle { get; init; } = 25f;

    public Vector3 BoxSize { get; init; } = Vector3.One;

    public Vector2 Lifetime { get; init; } = new(1.5f, 2.5f);

    public Vector2 Speed { get; init; } = new(1f, 2f);

    public float Gravity { get; init; }

    public float Drag { get; init; }

    public float StartSize { get; init; } = 0.2f;

    public float MiddleSize { get; init; } = 0.25f;

    public float EndSize { get; init; } = 0.1f;

    public Vector4 StartColor { get; init; } = Vector4.One;

    public Vector4 MiddleColor { get; init; } = new(1f, 1f, 1f, 0.8f);

    public Vector4 EndColor { get; init; } = new(1f, 1f, 1f, 0f);

    public IReadOnlyList<SizeKey> SizeKeys { get; init; } = [];

    public IReadOnlyList<ColorKey> ColorKeys { get; init; } = [];

    public Vector2 Spin { get; init; } = Vector2.Zero;

    public string Render { get; init; } = "sprite";

    public string Image { get; init; } = "";

    public uint FlipbookColumns { get; init; } = 1;

    public uint FlipbookRows { get; init; } = 1;

    public Shape MeshShape { get; init; } = Components.Shape.Cube;

    public string Material { get; init; } = "";

    public string Blend { get; init; } = "alpha";

    public string Space { get; init; } = "world";

    public static string Key => "particleEmitter";

    public static string Label => "Particle Emitter";

    public static string Menu => "Effects/Particle Emitter";

    public static bool Removable => true;

    public static ParticleEmitter Initial => new();

    public bool Equals(ParticleEmitter? other) => other is not null && Write().Equals(other.Write());

    public override int GetHashCode() => Write().GetHashCode();

    private static readonly string[] Fields =
    [
        "playing", "looping", "duration", "rate", "burst", "maxParticles", "shape", "radius", "angle", "boxSize", "lifetime", "speed", "gravity", "drag",
        "startSize", "middleSize", "endSize", "startColor", "middleColor", "endColor", "sizeKeys", "colorKeys", "spin", "render", "image",
        "flipbookColumns", "flipbookRows", "meshShape", "material", "blend", "space",
    ];

    public static ParticleEmitter Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct ParticleEmitter", true, Fields);
        return new()
        {
            Playing = f.Required("playing", JsonRead.Bool),
            Looping = f.Required("looping", JsonRead.Bool),
            Duration = f.Required("duration", JsonRead.F32),
            Rate = f.Required("rate", JsonRead.F32),
            Burst = f.Required("burst", JsonRead.U32),
            MaxParticlesAlive = f.Required("maxParticles", JsonRead.U32),
            Shape = f.Required("shape", JsonRead.Str),
            Radius = f.Required("radius", JsonRead.F32),
            Angle = f.Required("angle", JsonRead.F32),
            BoxSize = f.Required("boxSize", J.ReadV3),
            Lifetime = f.Required("lifetime", J.ReadV2),
            Speed = f.Required("speed", J.ReadV2),
            Gravity = f.Required("gravity", JsonRead.F32),
            Drag = f.Required("drag", JsonRead.F32),
            StartSize = f.Required("startSize", JsonRead.F32),
            MiddleSize = f.Required("middleSize", JsonRead.F32),
            EndSize = f.Required("endSize", JsonRead.F32),
            StartColor = f.Required("startColor", J.ReadV4),
            MiddleColor = f.Required("middleColor", J.ReadV4),
            EndColor = f.Required("endColor", J.ReadV4),
            SizeKeys = f.Optional("sizeKeys", v => (IReadOnlyList<SizeKey>)JsonRead.List(v, SizeKey.Read), []),
            ColorKeys = f.Optional("colorKeys", v => (IReadOnlyList<ColorKey>)JsonRead.List(v, ColorKey.Read), []),
            Spin = f.Required("spin", J.ReadV2),
            Render = f.Required("render", JsonRead.Str),
            Image = f.Required("image", JsonRead.Str),
            FlipbookColumns = f.Required("flipbookColumns", JsonRead.U32),
            FlipbookRows = f.Required("flipbookRows", JsonRead.U32),
            MeshShape = f.Required("meshShape", Shapes.Read),
            Material = f.Required("material", JsonRead.Str),
            Blend = f.Required("blend", JsonRead.Str),
            Space = f.Required("space", JsonRead.Str),
        };
    }

    public JsonValue Write() => new JsonObject
    {
        { "playing", J.B(Playing) },
        { "looping", J.B(Looping) },
        { "duration", J.F(Duration) },
        { "rate", J.F(Rate) },
        { "burst", J.U(Burst) },
        { "maxParticles", J.U(MaxParticlesAlive) },
        { "shape", J.S(Shape) },
        { "radius", J.F(Radius) },
        { "angle", J.F(Angle) },
        { "boxSize", J.V3(BoxSize) },
        { "lifetime", J.V2(Lifetime) },
        { "speed", J.V2(Speed) },
        { "gravity", J.F(Gravity) },
        { "drag", J.F(Drag) },
        { "startSize", J.F(StartSize) },
        { "middleSize", J.F(MiddleSize) },
        { "endSize", J.F(EndSize) },
        { "startColor", J.V4(StartColor) },
        { "middleColor", J.V4(MiddleColor) },
        { "endColor", J.V4(EndColor) },
        { "sizeKeys", new JsonArray(SizeKeys.Select(k => k.Write())) },
        { "colorKeys", new JsonArray(ColorKeys.Select(k => k.Write())) },
        { "spin", J.V2(Spin) },
        { "render", J.S(Render) },
        { "image", J.S(Image) },
        { "flipbookColumns", J.U(FlipbookColumns) },
        { "flipbookRows", J.U(FlipbookRows) },
        { "meshShape", J.S(MeshShape.Code()) },
        { "material", J.S(Material) },
        { "blend", J.S(Blend) },
        { "space", J.S(Space) },
    };

    private static Result<Unit, string> Range(string name, float v, float low, float high) =>
        float.IsFinite(v) && J.Within(v, low, high) ? Checks.Ok : Checks.Fail($"{name} must be between {Text.Number(low)} and {Text.Number(high)}");

    public Result<Unit, string> Validate()
    {
        var problems = new List<Func<Result<Unit, string>>>
        {
            () => Range("duration", Duration, 0.01f, 3600f),
            () => Range("rate", Rate, 0f, 10_000f),
            () => Burst > MaxParticles || MaxParticlesAlive > MaxParticles ? Checks.Fail($"burst and maxParticles are at most {MaxParticles}") : Checks.Ok,
            () => Range("radius", Radius, 0f, 10_000f),
            () => Range("angle", Angle, 0f, 180f),
        };
        problems.AddRange(J.Of(BoxSize).Select(v => (Func<Result<Unit, string>>)(() => Range("boxSize", v, 0f, 10_000f))));
        foreach (var (name, pair, top) in new[] { ("lifetime", Lifetime, 3600f), ("speed", Speed, 10_000f), ("spin", Spin, 100_000f) })
        {
            var bottom = name == "spin" ? -top : 0f;
            problems.Add(() => Range(name, pair.X, bottom, top));
            problems.Add(() => Range(name, pair.Y, bottom, top));
            problems.Add(() => pair.Y < pair.X ? Checks.Fail($"{name}: the second number is the larger") : Checks.Ok);
        }
        problems.Add(() => Lifetime.Y <= 0f ? Checks.Fail("lifetime must be more than 0") : Checks.Ok);
        problems.Add(() => Range("gravity", Gravity, -100f, 100f));
        problems.Add(() => Range("drag", Drag, 0f, 100f));
        problems.AddRange(new[] { StartSize, MiddleSize, EndSize }.Select(v => (Func<Result<Unit, string>>)(() => Range("size", v, 0f, 10_000f))));
        problems.AddRange(new[] { StartColor, MiddleColor, EndColor }.SelectMany(J.Of).Select(v => (Func<Result<Unit, string>>)(() => Range("color", v, 0f, 1f))));
        problems.Add(() => SizeKeys.Count > MaxKeys || ColorKeys.Count > MaxKeys ? Checks.Fail($"sizeKeys and colorKeys hold at most {MaxKeys} points each") : Checks.Ok);
        foreach (var key in SizeKeys)
        {
            problems.Add(() => Range("sizeKeys at", key.At, 0f, 1f));
            problems.Add(() => Range("sizeKeys size", key.Size, 0f, 10_000f));
        }
        foreach (var key in ColorKeys)
        {
            problems.Add(() => Range("colorKeys at", key.At, 0f, 1f));
            problems.AddRange(J.Of(key.Color).Select(v => (Func<Result<Unit, string>>)(() => Range("colorKeys color", v, 0f, 1f))));
        }
        problems.Add(() => FlipbookColumns is < 1 or > 64 || FlipbookRows is < 1 or > 64 ? Checks.Fail("flipbookColumns and flipbookRows are 1 to 64") : Checks.Ok);
        foreach (var (field, value) in new[] { ("shape", Shape), ("render", Render), ("blend", Blend), ("space", Space) })
        {
            problems.Add(() => Options(field).Contains(value, StringComparer.Ordinal) ? Checks.Ok : Checks.Fail($"{field} is one of {string.Join(", ", Options(field))}"));
        }
        problems.Add(() => Checks.FileField("image", Image, "png", "jpg", "jpeg"));
        problems.Add(() => Checks.FileField("material", Material, "bmaterial"));
        return Checks.All([.. problems]);
    }

    public static IReadOnlyList<string> Options(string field) => field switch
    {
        "shape" => ["point", "sphere", "cone", "box", "surface"],
        "render" => ["sprite", "stretched", "mesh"],
        "blend" => ["alpha", "additive"],
        "space" => ["world", "local"],
        _ => [],
    };
}
