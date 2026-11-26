using System.Numerics;

using Bayleaf.Assets;
using Bayleaf.Core.Geometry;
using Bayleaf.Scene;
using Bayleaf.Scene.Components;

using static Bayleaf.Render.Tests.Scenes;

using SceneModel = Bayleaf.Scene.Scene;

namespace Bayleaf.Render.Tests;

/// <summary>What a frame holds, worked out without a GPU.</summary>
public class ExtractTests
{
    private static readonly FrameOptions Options = FrameOptions.Default;

    [Fact]
    public void PickReturnsTheNearestObjectUnderThePoint()
    {
        var scene = new SceneModel();
        var far = Mesh(scene, "cube", new Vector3(0, 0, -5));
        var near = Mesh(scene, "sphere", new Vector3(0, 0, -2));
        Mesh(scene, "cube", new Vector3(4, 0, -2));
        var camera = CameraView.LookAt(new Vector3(0, 0, 3), new Vector3(0, 0, -1), 60, 1);
        Assert.Equal(near, Extraction.Pick(Extraction.Extract(scene, camera, Options), Vector2.Zero));
        Assert.True(scene.DeleteObject(near).IsOk);
        var frame = Extraction.Extract(scene, camera, Options);
        Assert.Equal(far, Extraction.Pick(frame, Vector2.Zero));
        Assert.Null(Extraction.Pick(frame, new Vector2(0, 0.95f)));
    }

    [Fact]
    public void LightDirectionFollowsTheObjectRotation()
    {
        var scene = new SceneModel();
        // Turned -90° about X, the light's -Z axis points straight down.
        Object(scene, "directionalLight", """{"color": [1, 1, 1], "intensity": 2}""", Vector3.Zero, new Vector3(-90, 0, 0));
        var frame = Extraction.Extract(scene, CameraView.Framing(Double3.Zero, 1, 1), Options);
        Assert.Single(frame.Lights);
        Assert.True(Vector3.Dot(frame.Lights[0].Direction, -Vector3.UnitY) > 0.99f);
    }

    [Fact]
    public void SpritesAreUnlitTransparentAndSized()
    {
        var scene = new SceneModel();
        Object(scene, "sprite", """{"color": [1, 0, 0, 1], "size": [3, 1]}""", Vector3.Zero);
        var sprite = Extraction.Extract(scene, CameraView.Framing(Double3.Zero, 1, 1), Options).Items[0];
        Assert.True(sprite.Unlit && sprite.IsTransparent);
        Assert.True(new Vector3(sprite.Model.M11, sprite.Model.M12, sprite.Model.M13).Length() > new Vector3(sprite.Model.M21, sprite.Model.M22, sprite.Model.M23).Length());
    }

    [Fact]
    public void TheFirstCameraObjectIsTheSceneCamera()
    {
        var scene = new SceneModel();
        Assert.Null(Extraction.SceneCamera(scene, 1));
        Object(scene, "camera", """{"projection": "perspective", "fov": 60, "size": 5, "near": 0.1, "far": 100}""", new Vector3(0, 5, 0));
        var eye = Extraction.SceneCamera(scene, 1)!.Value.Eye;
        Assert.True((eye - new Double3(0, 5, 0)).Length < 1e-5);
    }

    /// <summary>Gives one lighting asset for every lighting path.</summary>
    private sealed class OneLighting(Lighting lighting) : IAssetSource
    {
        public CompiledMaterial? Material(string path) => null;

        public Image? Image(string path) => null;

        public MeshData? Model(string path) => null;

        public Lighting? Lighting(string path) => lighting;
    }

    [Fact]
    public void ALightingAssetGivesTheSkyAndASunWhereTheSceneHasNone()
    {
        var scene = new SceneModel();
        Assert.True(scene.SetSettings(scene.Settings with { Lighting = "day.blight" }).IsOk);
        var camera = CameraView.LookAt(new Vector3(0, 0, 3), Vector3.Zero, 50, 1);
        var frame = Extraction.Extract(scene, camera, Options, new OneLighting(Lighting.Default));
        Assert.NotNull(frame.Sky);
        Assert.Single(frame.Lights);
        // A scene with its own sun keeps it alone.
        Object(scene, "directionalLight", """{"color": [1, 1, 1], "intensity": 3}""", Vector3.Zero);
        frame = Extraction.Extract(scene, camera, Options, new OneLighting(Lighting.Default));
        Assert.Single(frame.Lights);
        Assert.Equal(3f, frame.Lights[0].Radiance.X);
    }

    [Fact]
    public void AnObjectFollowingAJointMovesWithItAndTheSkeletonCanBeDrawn()
    {
        var dir = TestFiles.Folder("follow");
        Assert.True(Fixtures.SkinnedArm(dir).IsOk);
        var scene = new SceneModel();
        var arm = Object(scene, "mesh", """{"shape": "cube", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 1, "model": "arm.gltf"}""", Vector3.Zero);
        Assert.True(scene.AddComponent(arm, "modelAnimation").IsOk);
        Assert.True(scene.SetComponent(arm, "modelAnimation", J("""{"animation": "Stretch", "playing": true, "looping": false, "speed": 1}""")).IsOk);
        // A sword in the hand: half a unit above the Hand joint.
        var sword = scene.CreateObject(arm, "Sword").Value;
        Assert.True(scene.AddComponent(sword, "mesh").IsOk);
        SetTransform(scene, sword, new Vector3(0, 0.5f, 0), Vector3.Zero, Vector3.One);
        Assert.True(scene.AddComponent(sword, "followJoint").IsOk);
        Assert.True(scene.SetComponent(sword, "followJoint", J("""{"joint": "Hand"}""")).IsOk);
        var assets = FileAssets.Unconfined(dir);
        var camera = CameraView.LookAt(new Vector3(0, 0, 5), Vector3.Zero, 60, 1);
        (Vector3 Sword, IReadOnlyList<(Vector3 From, Vector3 To)> Lines) SwordAt(float time)
        {
            var frame = Extraction.Extract(scene, camera, Options with { Time = time }, assets);
            var lines = Extraction.SkeletonLines(scene, arm, time, assets);
            Assert.NotNull(lines);
            return (frame.Items.First(i => i.ObjectId == sword).Model.Translation, lines);
        }
        var ((rest, restLines), (stretched, stretchedLines)) = (SwordAt(0), SwordAt(0.9f));
        Assert.True(MathF.Abs(rest.X - 1) < 1e-3f && MathF.Abs(rest.Y - 0.5f) < 1e-3f, $"at rest, the Hand joint's place plus its own: {rest}");
        Assert.True(stretched.X > rest.X + 1 && MathF.Abs(stretched.Y - 0.5f) < 1e-3f, $"it goes where the joint goes: {stretched}");
        Assert.Equal(2, restLines.Count);
        static float Reach(IReadOnlyList<(Vector3 From, Vector3 To)> lines) => lines.Max(l => l.To.X);
        Assert.True(Reach(stretchedLines) > Reach(restLines) + 1, "the drawn skeleton follows the pose");
        // Without its component, the sword stays where its Transform puts it.
        Assert.True(scene.RemoveComponent(sword, "followJoint").IsOk);
        Assert.True(MathF.Abs(SwordAt(0.9f).Sword.X) < 1e-3f);
        TestFiles.Remove(dir);
    }

    [Fact]
    public void ThePosesTheEngineGivesComeBeforeTheModelAnimation()
    {
        var dir = TestFiles.Folder("posed");
        Assert.True(Fixtures.SkinnedArm(dir).IsOk);
        var scene = new SceneModel();
        var arm = Object(scene, "mesh", """{"shape": "cube", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 1, "model": "arm.gltf"}""", Vector3.Zero);
        var files = FileAssets.Unconfined(dir);
        var skeleton = files.Model("arm.gltf")!.Skeleton!;
        var camera = CameraView.LookAt(new Vector3(0, 0, 5), Vector3.Zero, 60, 1);
        var still = Extraction.Extract(scene, camera, Options, files).Items[0].LocalBounds();
        var stretch = skeleton.Animation("Stretch");
        var posed = new PosedAssets(files, new Dictionary<Bayleaf.Core.StableId, Matrix4x4[]> { [arm] = skeleton.PoseAt(stretch, 0.9f) });
        var moved = Extraction.Extract(scene, camera, Options, posed).Items[0].LocalBounds();
        Assert.True(moved.Max.X > still.Max.X + 1, $"the given pose stretches the arm: {still.Max.X} then {moved.Max.X}");
        TestFiles.Remove(dir);
    }
}

/// <summary>The built-in shapes.</summary>
public class ShapeTests
{
    /// <summary>Every triangle winds counter-clockwise seen from outside, so lighting and culling agree.</summary>
    [Fact]
    public void TrianglesFaceTheWayTheirNormalsPoint()
    {
        foreach (var shape in Enum.GetValues<Shape>())
        {
            var (vertices, indices) = Shapes.Build(shape);
            for (var t = 0; t < indices.Length; t += 3)
            {
                var (a, b, c) = (vertices[indices[t]], vertices[indices[t + 1]], vertices[indices[t + 2]]);
                var face = Vector3.Cross(b.Position - a.Position, c.Position - a.Position);
                if (face.Length() < 1e-9f)
                {
                    continue; // degenerate triangles at the sphere poles
                }
                Assert.True(Vector3.Dot(face, a.Normal + b.Normal + c.Normal) > 0, $"{shape} has an inward-facing triangle");
            }
        }
    }

    [Fact]
    public void TextureCoordinatesCoverEachFaceOnce()
    {
        foreach (var shape in Enum.GetValues<Shape>())
        {
            var (vertices, _) = Shapes.Build(shape);
            Assert.Equal((Vector2.Zero, Vector2.One), (vertices.Aggregate(new Vector2(float.MaxValue), (m, v) => Vector2.Min(m, v.Uv)), vertices.Aggregate(new Vector2(float.MinValue), (m, v) => Vector2.Max(m, v.Uv))));
        }
    }

    [Fact]
    public void ShapesFitInTheUnitBox()
    {
        foreach (var shape in Enum.GetValues<Shape>())
        {
            Assert.All(Shapes.Build(shape).Vertices, v => Assert.True(MathF.Abs(v.Position.X) <= 0.5f + 1e-6f && MathF.Abs(v.Position.Y) <= 0.5f + 1e-6f && MathF.Abs(v.Position.Z) <= 0.5f + 1e-6f, $"{shape} vertex outside the unit box"));
        }
    }
}
