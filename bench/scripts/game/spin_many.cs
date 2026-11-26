// Benchmark script: `spin` written for many objects: one call turns every object.
public class SpinMany : GameScript, IAdvanceMany<SpinMany>
{
    [ScriptProperty] private float speed = 90f;

    public static void AdvanceMany(Many<SpinMany> objects, float seconds)
    {
        var scripts = objects.Scripts;
        var poses = objects.Poses;
        for (var i = 0; i < scripts.Length; i++)
        {
            ref var rotation = ref poses[i].Rotation;
            rotation.Y = (rotation.Y + scripts[i].speed * seconds) % 360f;
        }
    }
}
