// C# counterpart of bench/cpp/bench.cpp (bench/README.md): the same scene and per-object work, two
// layouts (entities owning components with a virtual Update; packed arrays), each timed for the
// script step alone and with every object's world matrix. Prints one JSON line per layout and part.
//
// Written for C# 5, so the C# compiler that comes with Windows' .NET Framework builds it:
//   %WINDIR%\Microsoft.NET\Framework64\v4.0.30319\csc.exe /nologo /optimize+ /out:bench-cs.exe Bench.cs
// Run: bench-cs.exe <objects>   (bench/run-languages.ps1 also runs it on .NET 9)

using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;

struct Vec3d { public double X, Y, Z; }
struct Vec3 { public float X, Y, Z; }

class Transform
{
    public Vec3d Position;
    public Vec3 Rotation;
    public Vec3 Scale = new Vec3 { X = 1, Y = 1, Z = 1 };
}
class Mesh { public int Shape; public float[] Color = { 0.8f, 0.8f, 0.8f, 1f }; public float Metallic, Roughness = 0.5f; }
class Collider { public int Shape; public Vec3 Size = new Vec3 { X = 1, Y = 1, Z = 1 }; public float Friction = 0.5f, Restitution; public bool Trigger; }
class PointLight { public Vec3 Color = new Vec3 { X = 1, Y = 1, Z = 1 }; public float Intensity = 1, Range = 10; }

abstract class Script
{
    public abstract void Update(Entity self, float dt, double time);
}

class Entity
{
    public string Name;
    public Transform Transform = new Transform();
    public Mesh Mesh;
    public Collider Collider;
    public PointLight Light;
    public List<string> Tags = new List<string>();
    public List<Script> Scripts = new List<Script>();
    public double[] World = new double[16];
}

sealed class Spin : Script
{
    readonly float speed;
    public Spin(float speed) { this.speed = speed; }
    public override void Update(Entity self, float dt, double time)
    {
        self.Transform.Rotation.Y = (self.Transform.Rotation.Y + speed * dt) % 360f;
    }
}

sealed class Bob : Script
{
    readonly float speed, height, baseHeight;
    public Bob(float speed, float height, float baseHeight) { this.speed = speed; this.height = height; this.baseHeight = baseHeight; }
    public override void Update(Entity self, float dt, double time)
    {
        self.Transform.Position.Y = baseHeight + height * (float)Math.Sin((float)time * speed);
    }
}

sealed class Follow : Script
{
    readonly Entity target;
    readonly float rate;
    public Follow(Entity target, float rate) { this.target = target; this.rate = rate; }
    public override void Update(Entity self, float dt, double time)
    {
        float k = Math.Min(rate * dt, 1f);
        float x = (float)self.Transform.Position.X, z = (float)self.Transform.Position.Z;
        self.Transform.Position.X = x + ((float)target.Transform.Position.X - x) * k;
        self.Transform.Position.Z = z + ((float)target.Transform.Position.Z - z) * k;
    }
}

static class Program
{
    const float Dt = 1f / 60f;
    const int Warmup = 30, MinFrames = 100, MaxFrames = 20000;
    const double MinSeconds = 3.0;

    // The same formulas as bench/src/main.rs.
    static float SpinSpeed(int i) { return 30f + (i % 7) * 10f; }
    static float BobSpeed(int i) { return 1f + (i % 5) * 0.25f; }
    static float BobHeight(int i) { return 0.25f + (i % 3) * 0.1f; }
    static float FollowRate(int i) { return 0.05f + (i % 4) * 0.05f; }

    static void WorldMatrix(Transform t, double[] m)
    {
        const double rad = Math.PI / 180.0;
        double ax = t.Rotation.X * rad, ay = t.Rotation.Y * rad, az = t.Rotation.Z * rad;
        double cx = Math.Cos(ax), sx = Math.Sin(ax), cy = Math.Cos(ay), sy = Math.Sin(ay), cz = Math.Cos(az), sz = Math.Sin(az);
        double s0 = t.Scale.X, s1 = t.Scale.Y, s2 = t.Scale.Z;
        m[0] = cy * cz * s0; m[1] = cy * sz * s0; m[2] = -sy * s0; m[3] = 0;
        m[4] = (sx * sy * cz - cx * sz) * s1; m[5] = (sx * sy * sz + cx * cz) * s1; m[6] = sx * cy * s1; m[7] = 0;
        m[8] = (cx * sy * cz + sx * sz) * s2; m[9] = (cx * sy * sz - sx * cz) * s2; m[10] = cx * cy * s2; m[11] = 0;
        m[12] = t.Position.X; m[13] = t.Position.Y; m[14] = t.Position.Z; m[15] = 1;
    }

    static int Side(int n) { return Math.Max(1, (int)Math.Ceiling(Math.Sqrt(n))); }

    static double[] Measure(Action step)
    {
        for (int i = 0; i < Warmup; i++) step();
        var ms = new List<double>();
        var all = Stopwatch.StartNew();
        var one = new Stopwatch();
        while (ms.Count < MaxFrames && (ms.Count < MinFrames || all.Elapsed.TotalSeconds < MinSeconds))
        {
            one.Restart();
            step();
            ms.Add(one.Elapsed.TotalMilliseconds);
        }
        var sorted = ms.ToArray();
        Array.Sort(sorted);
        return sorted;
    }

    static void Print(string layout, string part, int n, double[] ms, double check)
    {
        double sum = 0;
        foreach (double v in ms) sum += v;
        double mean = sum / ms.Length, var = 0;
        foreach (double v in ms) var += (v - mean) * (v - mean);
        Func<double, double> at = q => ms[(int)Math.Round((ms.Length - 1) * q)];
        var inv = CultureInfo.InvariantCulture;
        // The closing braces go after the formatted part: "}}" right after a format specifier reads
        // as part of it on .NET Framework.
        Console.WriteLine(string.Format(inv,
            "{{\"mode\":\"cs-{0}\",\"runtime\":\"{1}\",\"scripts\":\"cs\",\"part\":\"{2}\",\"objects\":{3},\"checksum\":{4:G6},\"stats\":{{\"frames\":{5},\"meanMs\":{6:F6},\"medianMs\":{7:F6},\"p5Ms\":{8:F6},\"p95Ms\":{9:F6},\"stdevMs\":{10:F6}",
            layout, Environment.Version, part, n, check, ms.Length, mean, at(0.5), at(0.05), at(0.95), Math.Sqrt(var / ms.Length)) + "}}");
    }

    static List<Entity> BuildScene(int n)
    {
        var entities = new List<Entity>(n + 1);
        var target = new Entity { Name = "Target" };
        entities.Add(target);
        int side = Side(n);
        for (int i = 0; i < n; i++)
        {
            var e = new Entity { Name = "Object " + i, Mesh = new Mesh(), Collider = new Collider() };
            e.Transform.Position.X = (i % side) * 2.0 - side;
            e.Transform.Position.Z = (i / side) * 2.0 - side;
            e.Tags.Add("bench");
            if (i % 10 == 0) e.Light = new PointLight();
            e.Scripts.Add(new Spin(SpinSpeed(i)));
            e.Scripts.Add(new Bob(BobSpeed(i), BobHeight(i), 0.5f));
            e.Scripts.Add(new Follow(target, FollowRate(i)));
            entities.Add(e);
        }
        return entities;
    }

    static void RunVirtual(int n, bool withMatrices)
    {
        var entities = BuildScene(n);
        double time = 0;
        var ms = Measure(() =>
        {
            time += Dt;
            foreach (var e in entities)
                foreach (var s in e.Scripts) s.Update(e, Dt, time);
            if (withMatrices)
                foreach (var e in entities) WorldMatrix(e.Transform, e.World);
        });
        double check = 0;
        foreach (var e in entities) check += e.Transform.Position.X + e.Transform.Position.Y + e.Transform.Position.Z + e.Transform.Rotation.Y + e.World[0];
        Print("virtual", withMatrices ? "step" : "scripts", n, ms, check);
    }

    struct SpinData { public int Entity; public float Speed; }
    struct BobData { public int Entity; public float Speed, Height, Base; }
    struct FollowData { public int Entity, Target; public float Rate; }

    // The same data in flat arrays, one loop per script kind.
    static void RunPacked(int n, bool withMatrices)
    {
        var pos = new Vec3d[n + 1];
        var rot = new Vec3[n + 1];
        var scale = new Vec3[n + 1];
        var world = new double[(n + 1) * 16];
        var spins = new SpinData[n];
        var bobs = new BobData[n];
        var follows = new FollowData[n];
        int side = Side(n);
        for (int i = 0; i <= n; i++) scale[i] = new Vec3 { X = 1, Y = 1, Z = 1 };
        for (int i = 0; i < n; i++)
        {
            pos[i + 1].X = (i % side) * 2.0 - side;
            pos[i + 1].Z = (i / side) * 2.0 - side;
            spins[i] = new SpinData { Entity = i + 1, Speed = SpinSpeed(i) };
            bobs[i] = new BobData { Entity = i + 1, Speed = BobSpeed(i), Height = BobHeight(i), Base = 0.5f };
            follows[i] = new FollowData { Entity = i + 1, Target = 0, Rate = FollowRate(i) };
        }
        var t = new Transform();
        var m = new double[16];
        double time = 0;
        var ms = Measure(() =>
        {
            time += Dt;
            for (int i = 0; i < spins.Length; i++) { int e = spins[i].Entity; rot[e].Y = (rot[e].Y + spins[i].Speed * Dt) % 360f; }
            for (int i = 0; i < bobs.Length; i++) pos[bobs[i].Entity].Y = bobs[i].Base + bobs[i].Height * (float)Math.Sin((float)time * bobs[i].Speed);
            for (int i = 0; i < follows.Length; i++)
            {
                int e = follows[i].Entity, g = follows[i].Target;
                float k = Math.Min(follows[i].Rate * Dt, 1f);
                float x = (float)pos[e].X, z = (float)pos[e].Z;
                pos[e].X = x + ((float)pos[g].X - x) * k;
                pos[e].Z = z + ((float)pos[g].Z - z) * k;
            }
            if (withMatrices)
                for (int i = 0; i <= n; i++)
                {
                    t.Position = pos[i]; t.Rotation = rot[i]; t.Scale = scale[i];
                    WorldMatrix(t, m);
                    Array.Copy(m, 0, world, i * 16, 16);
                }
        });
        double check = 0;
        for (int i = 0; i <= n; i++) check += pos[i].X + pos[i].Y + rot[i].Y + world[i * 16];
        Print("packed", withMatrices ? "step" : "scripts", n, ms, check);
    }

    static void Main(string[] args)
    {
        int n = args.Length > 0 ? int.Parse(args[0], CultureInfo.InvariantCulture) : 5000;
        RunVirtual(n, false);
        RunVirtual(n, true);
        RunPacked(n, false);
        RunPacked(n, true);
    }
}
