using System.Numerics;

using Bayleaf.Core;
using Bayleaf.Core.Geometry;
using Bayleaf.Scene.Components;

using SceneModel = Bayleaf.Scene.Scene;

namespace Bayleaf.Gameplay;

/// <summary>What Player Control asks of one object (<paramref name="Target"/>) this step.</summary>
/// <param name="Walk">Walking velocity, world units per second.</param>
/// <param name="Jump">Jump with this upward speed.</param>
/// <param name="Facing">Turn about Y to this angle in degrees (facing where it walks).</param>
public readonly record struct ControlCommand(StableId Target, Vector3 Walk, float? Jump, float? Facing);

/// <summary>Player Control: the player steers a Character with the project's input actions.</summary>
public static class Controller
{
    /// <summary>The actions Player Control reads.</summary>
    public const string MoveForward = "MoveForward";
    public const string MoveBack = "MoveBack";
    public const string MoveLeft = "MoveLeft";
    public const string MoveRight = "MoveRight";
    public const string Jump = "Jump";

    /// <summary>
    /// Commands for every object with Player Control. <paramref name="value"/> gives an action's
    /// value (0 to 1) for an object and <paramref name="pressed"/> whether it went down this step:
    /// each player may read its own Input Map. Directions are relative to where the scene's first
    /// camera looks, flattened onto the ground; without a camera, forward is -Z.
    /// </summary>
    public static List<ControlCommand> Drive(SceneModel scene, Func<StableId, string, float> value, Func<StableId, string, bool> pressed)
    {
        var forward = CameraForward(scene);
        var right = new Vector3(-forward.Z, 0f, forward.X);
        var commands = new List<ControlCommand>();
        foreach (var (id, _) in scene.WorldMatrices())
        {
            if (!scene.TryGet<PlayerControl>(id, out var control))
            {
                continue;
            }
            var (x, z) = (value(id, MoveRight) - value(id, MoveLeft), value(id, MoveForward) - value(id, MoveBack));
            var direction = (right * x) + (forward * z);
            if (direction.Length() > 1f)
            {
                direction = Vector3.Normalize(direction);
            }
            var jump = pressed(id, Jump);
            float? facing = control.TurnToMove && direction.LengthSquared() > 1e-4f ? MathF.Atan2(-direction.X, -direction.Z) * (180f / MathF.PI) : null;
            commands.Add(new ControlCommand(id, direction * control.Speed, jump ? control.JumpSpeed : null, facing));
        }
        return commands;
    }

    /// <summary>Where the scene's first camera looks, on the ground plane (unit length).</summary>
    private static Vector3 CameraForward(SceneModel scene)
    {
        var camera = scene.WorldMatrices().FirstOrDefault(m => scene.TryGet<Camera>(m.Id, out _));
        var looks = camera.Id == default ? new Double3(0, 0, -1) : camera.Matrix.TransformVector3(new Double3(0, 0, -1));
        var flat = new Vector3((float)looks.X, 0f, (float)looks.Z);
        var length = flat.Length();
        return length > 0f && float.IsFinite(length) ? flat / length : -Vector3.UnitZ;
    }

    /// <summary>
    /// The scene's first camera, moved to trail the first object with Player Control set to follow:
    /// <c>cameraDistance</c> away along the ground from where it is now, <c>cameraHeight</c> up,
    /// looking at the object. Null when nothing follows or there is no camera (or the camera has a parent).
    /// </summary>
    public static (StableId Camera, Transform Transform)? FollowCamera(SceneModel scene)
    {
        var all = scene.WorldMatrices();
        (DMat4 Matrix, PlayerControl Control)? following = null;
        foreach (var (id, matrix) in all)
        {
            if (scene.TryGet<PlayerControl>(id, out var c) && c.Camera == CameraFollow.Follow)
            {
                following = (matrix, c);
                break;
            }
        }
        if (following is not var (targetWorld, control))
        {
            return null;
        }
        (StableId Id, DMat4 Matrix)? found = null;
        foreach (var entry in all)
        {
            if (scene.TryGet<Camera>(entry.Id, out _))
            {
                found = entry;
                break;
            }
        }
        if (found is not var (camera, cameraWorld) || scene.Parent(camera) is not null)
        {
            return null;
        }
        var target = targetWorld.WAxis.Xyz;
        var away = cameraWorld.WAxis.Xyz - target;
        var ground = new Double3(away.X, 0, away.Z);
        var along = ground.Length is var l && l > 0 && double.IsFinite(l) ? ground * (1 / l) : new Double3(0, 0, 1);
        var eye = target + (along * control.CameraDistance) + (new Double3(0, 1, 0) * control.CameraHeight);
        var look = DMat4.LookAtRh(eye, target, new Double3(0, 1, 0)).Inverse();
        var (_, rotation, _) = look.ToScaleRotationTranslation();
        var (rx, ry, rz) = rotation.ToEulerXyz();
        static float Degrees(double radians) => (float)(radians * (180.0 / Math.PI));
        var scale = scene.TryGet<Transform>(camera, out var t) ? t.Scale : Vector3.One;
        var transform = new Transform(eye, new Vector3(Degrees(rx), Degrees(ry), Degrees(rz)), scale);
        return transform.IsFinite ? (camera, transform) : null;
    }
}
