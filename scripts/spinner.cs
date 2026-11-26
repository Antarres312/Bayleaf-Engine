// Turns the object around its Y axis. Property `speed`: degrees per second (default 90).
//
// It steps every spinning object in one call: each object's script and pose, side by side.

using Bayleaf.Script.Sdk;

namespace Bayleaf.Examples;

public class Spinner : GameScript, IAdvanceMany<Spinner>
{
    [ScriptProperty(Hint = "Degrees per second")]
    private float speed = 90f;

    protected override void Begin() => Log("spinner: started");

    public static void AdvanceMany(Many<Spinner> objects, float seconds)
    {
        for (var i = 0; i < objects.Count; i++)
        {
            ref var rotation = ref objects.Poses[i].Rotation;
            rotation.Y = (rotation.Y + objects.Scripts[i].speed * seconds) % 360f;
        }
    }
}
