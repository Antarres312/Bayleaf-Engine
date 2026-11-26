// Moves the object on the ground plane with W, A, S and D (or the arrow keys) and hops with Space.
// Properties: `speed` in units per second (default 3), `jump` hop height (default 1).

using System;

using Bayleaf.Script.Sdk;

namespace Bayleaf.Examples;

public class Mover : GameScript
{
    private const float Gravity = 9.81f;

    [ScriptProperty(Hint = "Units per second")]
    private float speed = 3f;

    [ScriptProperty(Hint = "How high a hop goes")]
    private float jump = 1f;

    private double? ground;
    private float vertical;

    private static float Axis(string plus, string plusToo, string minus, string minusToo) =>
        (Controls.KeyDown(plus) || Controls.KeyDown(plusToo) ? 1f : 0f) - (Controls.KeyDown(minus) || Controls.KeyDown(minusToo) ? 1f : 0f);

    protected override void Advance(float seconds)
    {
        var pose = Pose;
        var floor = ground ??= pose.Position.Y;
        pose.Position.X += Axis("D", "ArrowRight", "A", "ArrowLeft") * speed * seconds;
        pose.Position.Z += Axis("S", "ArrowDown", "W", "ArrowUp") * speed * seconds;

        // A simple hop: launch at the speed that reaches `jump` units, fall back with gravity.
        if (Controls.KeyDown("Space") && pose.Position.Y <= floor)
        {
            vertical = MathF.Sqrt(2f * Gravity * jump);
        }
        vertical -= Gravity * seconds;
        pose.Position.Y = Math.Max(pose.Position.Y + vertical * seconds, floor);
        if (pose.Position.Y <= floor)
        {
            vertical = 0f;
        }
        Pose = pose;
    }
}
