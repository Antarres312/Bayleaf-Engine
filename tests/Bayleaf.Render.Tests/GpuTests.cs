using System.Numerics;

using Bayleaf.Assets;
using Bayleaf.Core.Geometry;
using Bayleaf.Core.Json;
using Bayleaf.Graph;
using Bayleaf.Scene;

using static Bayleaf.Render.Tests.Scenes;

using SceneModel = Bayleaf.Scene.Scene;

namespace Bayleaf.Render.Tests;

/// <summary>The renderer's behaviour on the GPU, as the Rust renderer's tests check it.</summary>
public class GpuTests
{
    private static readonly FrameOptions Options = FrameOptions.Default;

    private static readonly Catalog Catalog = Material.Catalog;

    private static uint Add(Graph.Graph g, string kind) => g.AddNode(Catalog, kind, Vector2.Zero).Value;

    private static void Link(Graph.Graph g, uint from, string fromPin, uint to, string toPin) =>
        Assert.True(g.Connect(Catalog, new Link(from, fromPin, to, toPin)).IsOk, $"{from}.{fromPin} into {to}.{toPin}");

    private static void Set(Graph.Graph g, uint node, string name, string json) => Assert.True(g.SetValue(Catalog, node, name, J(json)).IsOk, name);

    private static CompiledMaterial Compile(Graph.Graph g)
    {
        var compiled = Material.Compile(g);
        Assert.True(compiled.IsOk, compiled.IsOk ? "" : Problem.Summary(compiled.Error));
        return compiled.Value;
    }

    /// <summary>Gives one material for every material path, or only for the one path named.</summary>
    private sealed class Materials(string? only, CompiledMaterial material) : IAssetSource
    {
        public CompiledMaterial? Material(string path) => only is null || only == path ? material : null;

        public Image? Image(string path) => null;

        public MeshData? Model(string path) => null;
    }

    /// <summary>Gives one lighting asset for every lighting path.</summary>
    private sealed class OneLighting(Lighting lighting) : IAssetSource
    {
        public CompiledMaterial? Material(string path) => null;

        public Image? Image(string path) => null;

        public MeshData? Model(string path) => null;

        public Lighting? Lighting(string path) => lighting;
    }

    private static RenderedImage RenderWithFiles(SceneModel scene, string dir, CameraView camera, FrameOptions? options = null)
    {
        var assets = FileAssets.Unconfined(dir);
        var frame = Extraction.Extract(scene, camera, options ?? Options, assets);
        Assert.Empty(assets.Warnings);
        return Gpu.Render(frame, 64, 64);
    }

    [Fact]
    public void ALitCubeIsDrawnOverTheBackground()
    {
        var scene = new SceneModel();
        Mesh(scene, "cube", Vector3.Zero);
        // Light from straight ahead of the camera.
        Object(scene, "directionalLight", """{"color": [1, 1, 1], "intensity": 3}""", Vector3.Zero);
        var camera = CameraView.LookAt(new Vector3(0, 0, 3), Vector3.Zero, 50, 1);
        var image = Gpu.Render(Extraction.Extract(scene, camera, Options), 64, 64);
        var (center, corner) = (At(image, 32, 32), At(image, 1, 1));
        Assert.True(Brightness(center) > Brightness(corner) + 60, $"centre {center} is not brighter than background {corner}");
    }

    [Fact]
    public void FacesTurnedTowardTheLightAreBrighter()
    {
        var scene = new SceneModel();
        Mesh(scene, "sphere", Vector3.Zero);
        // Light travelling toward -X, so the sphere's +X side is lit.
        Object(scene, "directionalLight", """{"color": [1, 1, 1], "intensity": 3}""", Vector3.Zero, new Vector3(0, 90, 0));
        var image = Gpu.Render(Extraction.Extract(scene, CameraView.LookAt(new Vector3(0, 0, 2.5f), Vector3.Zero, 40, 1), Options), 64, 64);
        var (lit, dark) = (At(image, 44, 32), At(image, 20, 32));
        Assert.True(Brightness(lit) > Brightness(dark), $"lit side {lit} not brighter than {dark}");
    }

    [Fact]
    public void MaterialsChangeTheDrawnColourAndEveryNodeIsValidOnTheGpu()
    {
        var scene = new SceneModel();
        var cube = Mesh(scene, "cube", Vector3.Zero);
        Object(scene, "directionalLight", """{"color": [1, 1, 1], "intensity": 3}""", Vector3.Zero);
        var camera = CameraView.LookAt(new Vector3(0, 0, 3), Vector3.Zero, 50, 1);
        var graph = Graph.Graph.Create(Catalog);
        var output = graph.Nodes[0].Id;
        Set(graph, output, "baseColor", "[0, 0, 0]");
        Set(graph, output, "emissive", "[1, 0, 0]");
        var red = Compile(graph);
        // The mesh has no material path, so the source is not asked.
        Assert.Null(Extraction.Extract(scene, camera, Options, new Materials(null, red)).Items[0].Material);
        Assert.True(scene.SetComponent(cube, "mesh", J("""{"shape": "cube", "color": [0.8, 0.8, 0.8, 1], "metallic": 0, "roughness": 0.5, "material": "red.bmaterial"}""")).IsOk);
        var image = Gpu.Render(Extraction.Extract(scene, camera, Options, new Materials("red.bmaterial", red)), 64, 64);
        var (r, g, b, _) = At(image, 32, 32);
        // The light adds a white highlight; the emission makes red dominate.
        Assert.True(r > 200 && r > g + 100 && r > b + 100, $"expected red, got {(r, g, b)}");

        // The material preview shows only the sphere, over a transparent background.
        var preview = Gpu.Render(Extraction.MaterialPreview(red, [], 1, 0), 64, 64);
        var centre = At(preview, 32, 32);
        Assert.True(centre.R > centre.G + 100 && centre.R > centre.B + 100 && centre.A == 255, $"the sphere shows the material: {centre}");
        foreach (var (x, y) in new (uint, uint)[] { (1, 1), (62, 1), (1, 62), (62, 62) })
        {
            Assert.Equal(0, At(preview, x, y).A);
        }

        var every = Compile(MaterialTests.EveryNodeMaterial());
        Gpu.Render(Extraction.Extract(scene, camera, Options, new Materials(null, every)), 32, 32);
    }

    [Fact]
    public void PointAndSpotLightsFadeWithDistanceAndOutsideTheCone()
    {
        // A point light low over the left half of the floor.
        var scene = Floor(("pointLight", """{"color": [1, 1, 1], "intensity": 20, "range": 6}""", new Vector3(-3, 1, 0), Vector3.Zero));
        var frame = Extraction.Extract(scene, TopDown(), Options);
        var (nearAt, farAt) = (CameraPixel(frame.Camera, new Vector3(-3, 0, 0), 64), CameraPixel(frame.Camera, new Vector3(4.5f, 0, 0), 64));
        var image = Gpu.Render(frame, 64, 64);
        var (near, far) = (At(image, nearAt), At(image, farAt));
        Assert.True(Brightness(near) > Brightness(far) + 60, $"near {near} far {far}");

        // A spot light straight down at the centre, 30° wide.
        scene = Floor(("spotLight", """{"color": [1, 1, 1], "intensity": 40, "range": 10, "angle": 30, "softness": 0.2}""", new Vector3(0, 3, 0), new Vector3(-90, 0, 0)));
        frame = Extraction.Extract(scene, TopDown(), Options);
        var outsideAt = CameraPixel(frame.Camera, new Vector3(3.5f, 0, 0), 64);
        image = Gpu.Render(frame, 64, 64);
        var (inside, outside) = (At(image, 32, 32), At(image, outsideAt));
        Assert.True(Brightness(inside) > Brightness(outside) + 60, $"inside {inside} outside {outside}");
    }

    private const string GreyCube = """{"shape": "cube", "color": [0.8, 0.8, 0.8, 1], "metallic": 0, "roughness": 0.9}""";

    [Fact]
    public void AShapeCastsAShadowFromTheSunUnlessShadowsAreOff()
    {
        (uint Under, uint Away) Render(bool shadows)
        {
            var scene = Floor(("directionalLight", $$"""{"color": [1, 1, 1], "intensity": 1.5, "shadows": {{(shadows ? "true" : "false")}}}""", new Vector3(0, 5, 0), new Vector3(-90, 0, 0)));
            Object(scene, "mesh", GreyCube, new Vector3(3, 2, 0));
            // From the side, so the floor under the cube is visible.
            var frame = Extraction.Extract(scene, CameraView.LookAt(new Vector3(3, 6, 9), new Vector3(3, 0, 0), 50, 1), Options);
            var under = CameraPixel(frame.Camera, new Vector3(3, 0, 0.3f), 64);
            var away = CameraPixel(frame.Camera, new Vector3(-1.5f, 0, 0.3f), 64);
            var image = Gpu.Render(frame, 64, 64);
            return (Brightness(At(image, under)), Brightness(At(image, away)));
        }
        var (under, away) = Render(true);
        Assert.True(away > under + 80, $"the floor under the cube is in shadow: under {under}, away {away}");
        (under, away) = Render(false);
        Assert.True(Math.Abs((int)away - (int)under) < 30, $"no shadow when shadows are off: under {under}, away {away}");
    }

    [Fact]
    public void ASurfaceOnTheGroundPlaneHidesTheGridCompletely()
    {
        var scene = Floor(("directionalLight", """{"color": [1, 1, 1], "intensity": 3, "shadows": false}""", Vector3.Zero, new Vector3(-60, 20, 0)));
        // A low, grazing view, where depth fighting shows most.
        var camera = CameraView.LookAt(new Vector3(0, 0.6f, 8), new Vector3(0, 0, -4), 50, 1);
        var withGrid = Gpu.Render(Extraction.Extract(scene, camera, Options with { ShowGrid = true }), 96, 96);
        var without = Gpu.Render(Extraction.Extract(scene, camera, Options), 96, 96);
        var differing = 0;
        for (var p = 96 * 60; p < 96 * 96; p++)
        {
            if (Enumerable.Range(0, 4).Any(c => Math.Abs(withGrid.Pixels[p * 4 + c] - without.Pixels[p * 4 + c]) > 6))
            {
                differing++;
            }
        }
        Assert.True(differing == 0, $"{differing} floor pixels show the grid");
    }

    [Fact]
    public void SpritesShowTheirImageAndMaterialsReadImages()
    {
        var dir = TestFiles.AssetFolder("images");
        var camera = CameraView.LookAt(new Vector3(0, 0, 2), Vector3.Zero, 50, 1);
        var scene = new SceneModel();
        Object(scene, "sprite", """{"color": [1, 1, 1, 1], "size": [2, 2], "image": "stripes.png"}""", Vector3.Zero);
        var image = RenderWithFiles(scene, dir, camera);
        var (left, right) = (At(image, 14, 32), At(image, 50, 32));
        Assert.True(left.R > 200 && left.B < 60, $"left half red: {left}");
        Assert.True(right.B > 200 && right.R < 60, $"right half blue: {right}");

        // A material that shows the image as emission on a quad.
        var graph = Graph.Graph.Create(Catalog);
        var output = graph.Nodes[0].Id;
        Set(graph, output, "baseColor", "[0, 0, 0]");
        var texture = Add(graph, "texture");
        Set(graph, texture, "image", "\"stripes.png\"");
        Link(graph, texture, "color", output, "emissive");
        File.WriteAllText(Path.Join(dir, "striped.bmaterial"), graph.ToJson());
        scene = new SceneModel();
        var quad = Object(scene, "mesh", """{"shape": "quad", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 1, "material": "striped.bmaterial"}""", Vector3.Zero);
        SetTransform(scene, quad, Vector3.Zero, Vector3.Zero, new Vector3(2, 2, 1));
        image = RenderWithFiles(scene, dir, camera);
        (left, right) = (At(image, 14, 32), At(image, 50, 32));
        Assert.True(left.R > right.R + 100 && right.B > left.B + 100, $"the image shows through the material: left {left} right {right}");
        TestFiles.Remove(dir);
    }

    [Fact]
    public void MeshesDrawTheirModelAndArePickedByItsBox()
    {
        var dir = TestFiles.AssetFolder("model");
        var scene = new SceneModel();
        var wall = Object(scene, "mesh", """{"shape": "cube", "color": [0.9, 0.9, 0.9, 1], "metallic": 0, "roughness": 0.9, "model": "wall.gltf"}""", Vector3.Zero);
        Object(scene, "directionalLight", """{"color": [1, 1, 1], "intensity": 3, "shadows": false}""", Vector3.Zero);
        var camera = CameraView.LookAt(new Vector3(0, 0, 4), Vector3.Zero, 50, 1);
        var image = RenderWithFiles(scene, dir, camera);
        // The square is 2 wide, wider than the unit cube the shape would draw.
        var (edge, corner) = (At(image, 32 + 12, 32), At(image, 1, 1));
        Assert.True(Brightness(edge) > Brightness(corner) + 100, $"the model reaches past the unit cube: {edge}");
        var frame = Extraction.Extract(scene, camera, Options, FileAssets.Unconfined(dir));
        Assert.Equal(wall, Extraction.Pick(frame, new Vector2(0.45f, 0)));

        // A model's own material tints it.
        File.WriteAllText(Path.Join(dir, "wall.gltf"), TestFiles.WallGltf(", \"material\": 0", rest: """, "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [0.05, 1.0, 0.05, 1.0], "metallicFactor": 0.0}}]"""));
        image = RenderWithFiles(scene, dir, camera);
        var (r, g, b, _) = At(image, 32 + 12, 32);
        Assert.True(g > r + 60 && g > b + 60, $"the model's base colour is green: {(r, g, b)}");
        TestFiles.Remove(dir);
    }

    [Fact]
    public void AnEnvironmentDrawsASkyAboveTheGroundAndLightsShapesFromIt()
    {
        var scene = new SceneModel();
        Mesh(scene, "sphere", Vector3.Zero);
        const string Env = """{"skyColor": [0.1, 0.3, 0.9], "horizonColor": [0.7, 0.75, 0.8], "groundColor": [0.4, 0.25, 0.1], "ambient": 2.0, "showSky": SHOW}""";
        var holder = Object(scene, "environment", Env.Replace("SHOW", "true", StringComparison.Ordinal), Vector3.Zero);
        var camera = CameraView.LookAt(new Vector3(0, 0, 4), Vector3.Zero, 70, 1);
        var image = Gpu.Render(Extraction.Extract(scene, camera, Options), 64, 64);
        var (top, bottom) = (At(image, 32, 1), At(image, 32, 62));
        Assert.True(top.B > top.R + 40, $"the sky above is blue: {top}");
        Assert.True(bottom.R > bottom.B + 40, $"below the horizon is the ground colour: {bottom}");
        // Shapes take the sky's light from above and the ground's from below.
        var (upper, lower) = (At(image, 32, 20), At(image, 32, 44));
        Assert.True(upper.B - upper.R > lower.B - lower.R, $"top {upper} is bluer than bottom {lower}");

        Assert.True(scene.SetComponent(holder, "environment", J(Env.Replace("SHOW", "false", StringComparison.Ordinal))).IsOk);
        image = Gpu.Render(Extraction.Extract(scene, camera, Options), 64, 64);
        Assert.Equal(image.Pixel(32, 1), image.Pixel(32, 62));
    }

    private static string Spot(bool shadows) => $$"""{"color": [1, 1, 1], "intensity": 60, "range": 12, "angle": 80, "softness": 0.1, "shadows": {{(shadows ? "true" : "false")}}}""";

    [Fact]
    public void ASpotLightCastsAShadowWhenAsked()
    {
        (uint Under, uint Beside) Render(bool shadows)
        {
            var scene = Floor(("spotLight", Spot(shadows), new Vector3(0, 6, 0), new Vector3(-90, 0, 0)));
            Object(scene, "mesh", GreyCube, new Vector3(0, 3, 0));
            var frame = Extraction.Extract(scene, CameraView.LookAt(new Vector3(0, 7, 7), Vector3.Zero, 60, 1), Options);
            // Straight under the cube, and beside its shadow but still in the cone.
            var under = CameraPixel(frame.Camera, Vector3.Zero, 64);
            var beside = CameraPixel(frame.Camera, new Vector3(2.2f, 0, 0), 64);
            var image = Gpu.Render(frame, 64, 64);
            return (Brightness(At(image, under)), Brightness(At(image, beside)));
        }
        var (under, beside) = Render(true);
        Assert.True(beside > under + 60, $"the floor under the cube is in shadow: under {under}, beside {beside}");
        Assert.True(Render(false).Under > under + 60, "without shadows the floor under the cube is lit");
    }

    [Fact]
    public void ASunShadowEndsAtTheShadowDistance()
    {
        uint Render(float reach)
        {
            var scene = Floor(("directionalLight", """{"color": [1, 1, 1], "intensity": 1.5, "shadows": true}""", new Vector3(0, 5, 0), new Vector3(-90, 0, 0)));
            // A cube far from the camera: about 16 units away.
            Object(scene, "mesh", GreyCube, new Vector3(0, 2, -5));
            var camera = CameraView.LookAt(new Vector3(0, 6, 9), new Vector3(0, 0, -3), 50, 1);
            var frame = Extraction.Extract(scene, camera, Options with { Shadows = ShadowSettings.Default with { Reach = reach } });
            var under = CameraPixel(frame.Camera, new Vector3(0, 0, -4.7f), 64);
            return Brightness(At(Gpu.Render(frame, 64, 64), under));
        }
        var (whole, near) = (Render(0), Render(5));
        Assert.True(near > whole + 80, $"past the distance the floor under the cube is lit: whole scene {whole}, 5 units {near}");
        Assert.True(Render(40) + 80 < near, "within the distance the shadow stays");
    }

    [Fact]
    public void AsManyLightsCastShadowsAsTheSettingsAllow()
    {
        // Two spot lights far apart, each over a cube of its own.
        (uint Left, uint Right) Render(uint lights)
        {
            var scene = Floor(("spotLight", Spot(true), new Vector3(-5, 6, 0), new Vector3(-90, 0, 0)), ("spotLight", Spot(true), new Vector3(5, 6, 0), new Vector3(-90, 0, 0)));
            foreach (var x in new[] { -5f, 5f })
            {
                Object(scene, "mesh", GreyCube, new Vector3(x, 3, 0));
            }
            var frame = Extraction.Extract(scene, CameraView.LookAt(new Vector3(0, 12, 12), Vector3.Zero, 70, 1), Options with { Shadows = ShadowSettings.Default with { Lights = lights } });
            var image = Gpu.Render(frame, 96, 96);
            uint AtX(float x) => Brightness(At(image, CameraPixel(frame.Camera, new Vector3(x, 0, 0), 96)));
            return (AtX(-5), AtX(5));
        }
        var (oneLeft, oneRight) = Render(1);
        var (twoLeft, twoRight) = Render(2);
        // With one light casting shadows, one cube leaves the floor under it lit; with two, neither does.
        var litWithOne = Math.Max(oneLeft, oneRight);
        Assert.True(litWithOne > Math.Max(twoLeft, twoRight) + 60, $"one: ({oneLeft}, {oneRight}), two: ({twoLeft}, {twoRight})");
        Assert.True(litWithOne > Math.Min(oneLeft, oneRight) + 60, $"with one, the other cube still casts its shadow: ({oneLeft}, {oneRight})");
    }

    [Fact]
    public void FogHidesFarShapesInTheHorizonColour()
    {
        (byte R, byte G, byte B, byte A) Render(float fog)
        {
            var scene = new SceneModel();
            Mesh(scene, "cube", new Vector3(0, 0, -30));
            Object(scene, "environment", FormattableString.Invariant($$"""{"skyColor": [0, 0, 0], "horizonColor": [0.9, 0.1, 0.1], "groundColor": [0, 0, 0], "ambient": 1, "showSky": false, "fogDistance": {{fog}}}"""), Vector3.Zero);
            var camera = CameraView.LookAt(Vector3.Zero, new Vector3(0, 0, -1), 5, 1);
            return At(Gpu.Render(Extraction.Extract(scene, camera, Options), 32, 32), 16, 16);
        }
        var (clear, foggy) = (Render(0), Render(3));
        static int Red((byte R, byte G, byte B, byte A) p) => p.R - p.G;
        Assert.True(Red(foggy) > Red(clear) + 80, $"the far cube takes the fog colour: clear {clear}, foggy {foggy}");
    }

    [Fact]
    public void ANormalMapBendsTheLightTowardItsRightAndUp()
    {
        uint Lit(string name, byte[] normal, Vector3 lightRotation)
        {
            var dir = TestFiles.NormalMappedSquare(name, normal);
            var scene = new SceneModel();
            Object(scene, "mesh", """{"shape": "cube", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 1, "model": "square.gltf"}""", Vector3.Zero);
            Object(scene, "directionalLight", """{"color": [1, 1, 1], "intensity": 3, "shadows": false}""", Vector3.Zero, lightRotation);
            var image = RenderWithFiles(scene, dir, CameraView.LookAt(new Vector3(0, 0, 3), Vector3.Zero, 40, 1));
            TestFiles.Remove(dir);
            return Brightness(At(image, 32, 32));
        }
        // A light from the right (+X), grazing the square: turned toward it, the surface is brighter.
        var fromRight = new Vector3(0, 80, 0);
        var (flat, right) = (Lit("flat-x", [128, 128, 255], fromRight), Lit("right", [255, 128, 128], fromRight));
        Assert.True(right > flat + 40, $"a normal map leaning right catches light from the right: flat {flat}, leaning {right}");
        // A light from above (+Y): the map's green leans the surface up, toward the top of the image.
        var fromAbove = new Vector3(-80, 0, 0);
        var (flatUp, up) = (Lit("flat-y", [128, 128, 255], fromAbove), Lit("up", [128, 255, 128], fromAbove));
        Assert.True(up > flatUp + 40, $"a normal map leaning up catches light from above: flat {flatUp}, leaning {up}");
    }

    [Fact]
    public void AModelWithTwoMaterialsDrawsEachPartInItsOwn()
    {
        var dir = TestFiles.Folder("parts");
        // Two squares side by side, each its own glTF primitive: left green, right red.
        File.WriteAllBytes(Path.Join(dir, "two.bin"),
        [
            .. TestFiles.Floats(-2, -1, 0, 0, -1, 0, 0, 1, 0, -2, 1, 0, 0, -1, 0, 2, -1, 0, 2, 1, 0, 0, 1, 0),
            .. TestFiles.Shorts(0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7),
        ]);
        File.WriteAllText(Path.Join(dir, "two.gltf"), """
            {"asset": {"version": "2.0"}, "scene": 0, "scenes": [{"nodes": [0]}], "nodes": [{"mesh": 0}],
             "meshes": [{"primitives": [
               {"attributes": {"POSITION": 0}, "indices": 1, "material": 0},
               {"attributes": {"POSITION": 0}, "indices": 2, "material": 1}
             ]}],
             "materials": [
               {"pbrMetallicRoughness": {"baseColorFactor": [0.05, 1.0, 0.05, 1.0], "metallicFactor": 0.0}},
               {"pbrMetallicRoughness": {"baseColorFactor": [1.0, 0.05, 0.05, 1.0], "metallicFactor": 0.0}}
             ],
             "buffers": [{"uri": "two.bin", "byteLength": 120}],
             "bufferViews": [
               {"buffer": 0, "byteOffset": 0, "byteLength": 96},
               {"buffer": 0, "byteOffset": 96, "byteLength": 12},
               {"buffer": 0, "byteOffset": 108, "byteLength": 12}
             ],
             "accessors": [
               {"bufferView": 0, "componentType": 5126, "count": 8, "type": "VEC3", "min": [-2, -1, 0], "max": [2, 1, 0]},
               {"bufferView": 1, "componentType": 5123, "count": 6, "type": "SCALAR"},
               {"bufferView": 2, "componentType": 5123, "count": 6, "type": "SCALAR"}
             ]}
            """);
        Assert.Equal(2, Loader.LoadModel(Path.Join(dir, "two.gltf")).Value.Parts.Count);
        var scene = new SceneModel();
        Object(scene, "mesh", """{"shape": "cube", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 0.9, "model": "two.gltf"}""", Vector3.Zero);
        Object(scene, "directionalLight", """{"color": [1, 1, 1], "intensity": 3, "shadows": false}""", Vector3.Zero);
        var image = RenderWithFiles(scene, dir, CameraView.LookAt(new Vector3(0, 0, 5), Vector3.Zero, 50, 1));
        var (left, right) = (At(image, 20, 32), At(image, 44, 32));
        Assert.True(left.G > left.R + 60, $"the left part is green: {left}");
        Assert.True(right.R > right.G + 60, $"the right part is red: {right}");
        TestFiles.Remove(dir);
    }

    [Fact]
    public void APointLightCastsShadowsAllAroundIt()
    {
        (uint Under, uint Behind, uint Lit) Render(bool shadows)
        {
            var scene = Floor(("pointLight", $$"""{"color": [1, 1, 1], "intensity": 80, "range": 14, "shadows": {{(shadows ? "true" : "false")}}}""", new Vector3(0, 3, 0), Vector3.Zero));
            // One cube below the light, one off to the side: their shadows fall on different faces.
            Object(scene, "mesh", GreyCube, new Vector3(0, 1.5f, 0));
            Object(scene, "mesh", GreyCube, new Vector3(2, 1, 0));
            var frame = Extraction.Extract(scene, TopDown(), Options);
            var image = Gpu.Render(frame, 96, 96);
            uint AtPoint(Vector3 p) => Brightness(At(image, CameraPixel(frame.Camera, p, 96)));
            // Below the first cube (seen past its edge), behind the second, and a lit spot in between.
            return (AtPoint(new Vector3(0, 0, 0.9f)), AtPoint(new Vector3(4.2f, 0, 0)), AtPoint(new Vector3(-2, 0, 0)));
        }
        var (under, behind, lit) = Render(true);
        Assert.True(lit > under + 40, $"the floor below the first cube is in shadow: under {under}, lit {lit}");
        Assert.True(lit > behind + 20, $"the floor behind the side cube is in shadow: behind {behind}, lit {lit}");
        var (underOff, behindOff, _) = Render(false);
        Assert.True(underOff > under + 40 && behindOff > behind + 20, "without shadows both are lit");
    }

    [Fact]
    public void SmoothSurfacesMirrorTheSkyAndRoughOnesBlurIt()
    {
        (byte R, byte G, byte B, byte A) Centre(float roughness)
        {
            var scene = new SceneModel();
            var ball = Mesh(scene, "sphere", Vector3.Zero);
            Assert.True(scene.SetComponent(ball, "mesh", J(FormattableString.Invariant($$"""{"shape": "sphere", "color": [1, 1, 1, 1], "metallic": 1, "roughness": {{roughness}}}"""))).IsOk);
            // A red horizon between a blue top and a green ground, no lights.
            Object(scene, "environment", """{"skyColor": [0.0, 0.0, 1.0], "horizonColor": [1.0, 0.0, 0.0], "groundColor": [0.0, 1.0, 0.0], "ambient": 1, "showSky": true}""", Vector3.Zero);
            var frame = Extraction.Extract(scene, CameraView.LookAt(new Vector3(0, 0, 2.5f), Vector3.Zero, 30, 1), Options);
            frame.Lights.Clear();
            return At(Gpu.Render(frame, 32, 32), 16, 16);
        }
        // The middle of the ball mirrors the horizon behind the camera.
        var (smooth, rough) = (Centre(0.05f), Centre(1));
        Assert.True(smooth.R > smooth.B + 60 && smooth.R > smooth.G, $"a mirror ball shows the red horizon: {smooth}");
        Assert.True(smooth.R > rough.R + 40, $"a rough ball blurs it: smooth {smooth}, rough {rough}");
    }

    [Fact]
    public void ASmallShapeNearTheCameraCastsASharpShadowInALargeWorld()
    {
        var scene = Floor(("directionalLight", """{"color": [1, 1, 1], "intensity": 2, "shadows": true}""", new Vector3(0, 10, 0), new Vector3(-90, 0, 0)));
        SetTransform(scene, scene.WorldMatrices()[0].Id, Vector3.Zero, Vector3.Zero, new Vector3(400, 1, 400));
        // A small box a little above the ground, close to the camera, and a far one that widens the scene.
        var small = Object(scene, "mesh", GreyCube, new Vector3(0, 0.6f, 0));
        SetTransform(scene, small, new Vector3(0, 0.6f, 0), Vector3.Zero, new Vector3(0.4f));
        Object(scene, "mesh", GreyCube, new Vector3(150, 0.5f, -150));
        var frame = Extraction.Extract(scene, CameraView.LookAt(new Vector3(0, 2.5f, 3), Vector3.Zero, 50, 1), Options);
        var image = Gpu.Render(frame, 128, 128);
        uint AtPoint(Vector3 p) => Brightness(At(image, CameraPixel(frame.Camera, p, 128)));
        // Straight below the box, seen past its near edge, and a little to the side of its shadow.
        var (under, beside) = (AtPoint(new Vector3(0, 0, 0.15f)), AtPoint(new Vector3(0.6f, 0, 0.15f)));
        Assert.True(beside > under + 60, $"the small box shadows the ground: under {under}, beside {beside}");
    }

    [Fact]
    public void ASceneAThousandKilometresFromTheCentreLooksAsItDoesAtTheCentre()
    {
        RenderedImage Render(double offset)
        {
            var scene = new SceneModel();
            foreach (var (shape, x) in new[] { ("cube", -0.6), ("sphere", 0.6) })
            {
                var id = Mesh(scene, shape, Vector3.Zero);
                Assert.True(scene.Set(id, new Bayleaf.Scene.Components.Transform(new Double3(offset + x, 0, offset), new Vector3(0, 30, 0), new Vector3(0.5f))).IsOk);
            }
            Object(scene, "directionalLight", """{"color": [1, 1, 1], "intensity": 3}""", Vector3.Zero, new Vector3(-40, 20, 0));
            var center = new Double3(offset, 0, offset);
            var camera = CameraView.LookAtWorld(center + new Double3(0, 0.4, 2), center, 50, 1);
            return Gpu.Render(Extraction.Extract(scene, camera, Options), 48, 48);
        }
        var (near, far) = (Render(0), Render(1e6));
        var differing = Enumerable.Range(0, 48 * 48).Count(p => near.Pixel((uint)(p % 48), (uint)(p / 48)) != far.Pixel((uint)(p % 48), (uint)(p / 48)));
        Assert.True(differing < 10, $"{differing} pixels differ between the centre and 1000 km out");
    }

    [Fact]
    public void ASelectedShapeKeepsItsColoursAndGetsAnOutlineAroundIt()
    {
        var scene = new SceneModel();
        var cube = Mesh(scene, "cube", Vector3.Zero);
        Object(scene, "directionalLight", """{"color": [1, 1, 1], "intensity": 3}""", Vector3.Zero);
        var camera = CameraView.LookAt(new Vector3(0, 0, 3), Vector3.Zero, 50, 1);
        var plain = Gpu.Render(Extraction.Extract(scene, camera, Options), 64, 64);
        var selected = Gpu.Render(Extraction.Extract(scene, camera, Options with { Selected = cube }), 64, 64);
        Assert.Equal(plain.Pixel(32, 32), selected.Pixel(32, 32));
        // The cube's left edge on the middle row, then a pixel just outside it.
        var background = plain.Pixel(1, 32);
        var edge = Enumerable.Range(0, 32).First(x => plain.Pixel((uint)x, 32) != background);
        Assert.Equal(background, plain.Pixel((uint)edge - 2, 32));
        var outside = At(selected, (uint)edge - 2, 32);
        Assert.True(outside.R > outside.B + 80, $"a selected shape has an orange band around it: {outside}");
    }

    [Fact]
    public void TheGroundGridShowsWhenItIsOnNearAndFarFromTheCentre()
    {
        var scene = new SceneModel();
        // Looking down at the ground from above, at the world's centre and far from it, as the editor's
        // view measures the frame from its eye.
        foreach (var at in new[] { Double3.Zero, new Double3(250_000.5, 0, -75_000.25) })
        {
            CameraView Camera() => CameraView.LookAtWorld(at + new Double3(0, 6, 6), at, 60, 1);
            var with = Gpu.Render(Extraction.Extract(scene, Camera(), Options with { ShowGrid = true }), 64, 64);
            var without = Gpu.Render(Extraction.Extract(scene, Camera(), Options), 64, 64);
            var changed = Enumerable.Range(0, 64 * 64).Count(p => with.Pixel((uint)(p % 64), (uint)(p / 64)) != without.Pixel((uint)(p % 64), (uint)(p / 64)));
            Assert.True(changed > 64, $"the grid draws lines on the ground around {at}");
        }
    }

    /// <summary>A sphere lit by a sun from the front-left, with <paramref name="material"/>, seen head on: the picture's middle row.</summary>
    private static List<(byte R, byte G, byte B, byte A)> SphereRow(CompiledMaterial material)
    {
        var scene = new SceneModel();
        Object(scene, "mesh", """{"shape": "sphere", "color": [0.8, 0.8, 0.8, 1], "metallic": 0, "roughness": 0.5, "material": "m.bmaterial"}""", Vector3.Zero);
        Object(scene, "directionalLight", """{"color": [1, 1, 1], "intensity": 2}""", Vector3.Zero, new Vector3(0, -60, 0));
        var frame = Extraction.Extract(scene, CameraView.LookAt(new Vector3(0, 0, 3), Vector3.Zero, 50, 1), Options, new Materials(null, material));
        var image = Gpu.Render(frame, 96, 96);
        return [.. Enumerable.Range(0, 96).Select(x => At(image, (uint)x, 48))];
    }

    [Fact]
    public void CustomLightingBandsTheLightAndAnOutlineRimsTheShape()
    {
        // The engine's lighting: a smooth shading across the sphere.
        var smooth = SphereRow(Compile(Graph.Graph.Create(Catalog)));
        // Custom lighting: the main light's angle cut into three bands, times its colour and shadow.
        var g = Graph.Graph.Empty(Catalog);
        var output = Add(g, "customLighting");
        var light = Add(g, "mainLight");
        var normal = Add(g, "normal");
        var dot = Add(g, "dot");
        Link(g, normal, "normal", dot, "a");
        Link(g, light, "direction", dot, "b");
        var bands = Add(g, "posterize");
        Link(g, dot, "result", bands, "value");
        var lit = Add(g, "multiply");
        Link(g, bands, "result", lit, "a");
        Link(g, light, "shadow", lit, "b");
        Link(g, lit, "result", output, "color");
        var toon = SphereRow(Compile(g));
        static int Shades(List<(byte R, byte G, byte B, byte A)> row) => row.Select(p => p.R).Distinct().Count();
        Assert.True(Shades(toon) * 3 < Shades(smooth), $"flat bands: {Shades(toon)} shades against {Shades(smooth)}");

        // An outline: a red rim just outside the sphere's edge, where the background was.
        Set(g, output, "outlineWidth", "0.08");
        Set(g, output, "outlineColor", "[1.0, 0.0, 0.0]");
        var compiled = Compile(g);
        Assert.NotNull(compiled.Outline);
        var rimmed = SphereRow(compiled);
        // Background pixels are the ones like the row's first (beside the sphere).
        var background = toon[0];
        static bool Red((byte R, byte G, byte B, byte A) p) => p.R > 150 && p.G < 60 && p.B < 60;
        var rim = Enumerable.Range(0, toon.Count).Count(i => toon[i] == background && Red(rimmed[i]));
        Assert.True(rim >= 2, $"a red rim where the background was beside the sphere: {rim} pixels");
        Assert.DoesNotContain(toon, Red);
        Assert.Equal(toon[48], rimmed[48]);
    }

    [Fact]
    public void PostProcessingEffectsEachChangeThePictureAsTheySay()
    {
        // A bright orange cube in the middle on a grey floor, under a sun, against a grey background.
        var scene = Floor(("directionalLight", """{"color": [1, 1, 1], "intensity": 2}""", new Vector3(0, 5, 0), new Vector3(-60, 20, 0)));
        Object(scene, "mesh", """{"shape": "cube", "color": [1.0, 0.5, 0.1, 1], "metallic": 0, "roughness": 0.6}""", new Vector3(0, 0.5f, 0));
        var camera = CameraView.LookAt(new Vector3(0, 2.5f, 4), new Vector3(0, 0.3f, 0), 50, 1);
        var frame = Extraction.Extract(scene, camera, Options);
        RenderedImage Render(PostSettings post, Frame? from = null) => Gpu.Render((from ?? frame) with { Post = post }, 96, 96);
        var plain = Render(PostSettings.Default);
        var centre = CameraPixel(frame.Camera, new Vector3(0, 0.5f, 0.5f), 96);

        var darker = Render(new PostSettings { Exposure = 0.4f });
        Assert.True(Brightness(At(darker, centre)) + 60 < Brightness(At(plain, centre)), "less exposure, darker");

        var (r, g, b, _) = At(Render(new PostSettings { Saturation = 0 }), centre);
        Assert.True(Math.Abs(r - g) < 6 && Math.Abs(g - b) < 6, $"no saturation, grey: {(r, g, b)}");

        var corner = (2u, 2u);
        var vignetted = Render(new PostSettings { Vignette = 0.8f });
        Assert.True(Brightness(At(vignetted, corner)) + 30 < Brightness(At(plain, corner)), "darker corners");
        Assert.True(Math.Abs((int)Brightness(At(vignetted, centre)) - (int)Brightness(At(plain, centre))) < 12, "the middle as it was");

        // An overbright cube: without tone mapping its colour clips; with it, it keeps its hue. Only
        // the cube (the orange item), not the floor.
        var bright = frame with { Items = [.. frame.Items.Select(i => MathF.Abs(i.Color.Y - 0.5f) < 1e-3f ? i with { Color = i.Color * new Vector4(6, 6, 6, 1) } : i)] };
        var clipped = At(Render(PostSettings.Default, bright), centre);
        var mapped = At(Render(new PostSettings { ToneMapping = true }, bright), centre);
        Assert.True(clipped.R == 255 && clipped.G == 255, $"clipped: {clipped}");
        Assert.True(mapped.R > mapped.G && mapped.G > mapped.B, $"tone mapped keeps orange: {mapped}");

        // Bloom: the overbright cube's glow reaches past its edge onto the floor beside it.
        var beside = CameraPixel(frame.Camera, new Vector3(0.85f, 0.05f, 0.6f), 96);
        var glow = Render(new PostSettings { Bloom = 1, BloomThreshold = 1 }, bright);
        Assert.True(Brightness(At(glow, beside)) > Brightness(At(Render(PostSettings.Default, bright), beside)) + 20, "a glow beside the cube");

        // Depth fog: the far floor takes the fog colour, the near one less so.
        var fogged = Render(new PostSettings { Fog = (new Vector3(0, 0, 1), 2, 8) });
        var far = At(fogged, CameraPixel(frame.Camera, new Vector3(0, 0, -6), 96));
        Assert.True(far.B > far.R + 60, $"the far floor is blue fog: {far}");

        // Ambient occlusion: where the cube meets the floor goes darker.
        var crease = CameraPixel(frame.Camera, new Vector3(0, 0.02f, 0.55f), 96);
        var occluded = Render(new PostSettings { Occlusion = 1, OcclusionRadius = 0.4f });
        Assert.True(Brightness(At(occluded, crease)) + 15 < Brightness(At(plain, crease)), "the crease darkens");
        // Open floor away from the cube does not hide itself: it keeps its light.
        var open = CameraPixel(frame.Camera, new Vector3(1.6f, 0, 1.4f), 96);
        Assert.True(Math.Abs((int)Brightness(At(occluded, open)) - (int)Brightness(At(plain, open))) <= 6, $"flat ground stays: {At(occluded, open)} {At(plain, open)}");

        // Antialiasing changes only edges, Fast and Fine alike: the middle of the cube stays.
        foreach (var choice in new[] { Antialiasing.Fast, Antialiasing.Fine })
        {
            var smoothed = Render(new PostSettings { Antialiasing = choice });
            Assert.Equal(At(plain, centre), At(smoothed, centre));
            var changed = Enumerable.Range(0, 96 * 96).Count(p => Enumerable.Range(0, 4).Any(c => Math.Abs(smoothed.Pixels[p * 4 + c] - plain.Pixels[p * 4 + c]) > 4));
            Assert.True(changed > 0, $"{choice} softens the cube's edges");
        }
    }

    [Fact]
    public void ATargetWithoutMultisamplingDrawsTheSameSceneWithHarderEdges()
    {
        var scene = Floor(("directionalLight", """{"color": [1, 1, 1], "intensity": 2}""", new Vector3(0, 5, 0), new Vector3(-60, 20, 0)));
        Object(scene, "mesh", """{"shape": "cube", "color": [1.0, 0.5, 0.1, 1], "metallic": 0, "roughness": 0.6}""", new Vector3(0, 0.5f, 0), new Vector3(0, 30, 0));
        var frame = Extraction.Extract(scene, CameraView.LookAt(new Vector3(0, 2.5f, 4), new Vector3(0, 0.3f, 0), 50, 1), Options);
        var four = Gpu.Render(frame, 96, 96, 4);
        var one = Gpu.Render(frame, 96, 96, 1);
        var centre = CameraPixel(frame.Camera, new Vector3(0, 0.5f, 0.5f), 96);
        Assert.True(Math.Abs((int)Brightness(At(one, centre)) - (int)Brightness(At(four, centre))) <= 6, "the same scene");
        // Multisampled edges step through in-between colours; without, they jump from side to side.
        static int Jumps(RenderedImage image)
        {
            var count = 0;
            for (var y = 0u; y < 96; y++)
            {
                for (var x = 0u; x < 95; x++)
                {
                    if (Math.Abs((int)Brightness(At(image, x, y)) - (int)Brightness(At(image, x + 1, y))) > 120)
                    {
                        count++;
                    }
                }
            }
            return count;
        }
        Assert.True(Jumps(one) > Jumps(four), $"multisampling softens the cube's edges: {Jumps(one)} hard steps without, {Jumps(four)} with");
    }

    [Fact]
    public void AnAdditiveItemAddsItsLightAndAnAlphaOneCovers()
    {
        // A red unlit quad behind, a blue unlit quad in front: covering, it shows blue; adding, purple.
        var scene = new SceneModel();
        Object(scene, "sprite", """{"image": "", "color": [1.0, 0.0, 0.0, 1.0], "size": [2.0, 2.0]}""", new Vector3(0, 0, -0.5f));
        var frame = Extraction.Extract(scene, CameraView.LookAt(new Vector3(0, 0, 3), Vector3.Zero, 50, 1), Options);
        (byte R, byte G, byte B, byte A) Front(bool add)
        {
            var item = frame.Items[0] with { Model = Matrix4x4.CreateTranslation(new Vector3(0, 0, 0.5f) - CameraView.Single(frame.Camera.Origin)), Color = new Vector4(0, 0, 1, 1), Additive = add };
            return At(Gpu.Render(frame with { Items = [frame.Items[0], item] }, 32, 32), 16, 16);
        }
        var (covered, added) = (Front(false), Front(true));
        Assert.True(covered.B > 200 && covered.R < 40, $"covering: blue {covered}");
        Assert.True(added.B > 200 && added.R > 200, $"adding: red and blue {added}");
    }

    [Fact]
    public void ADataImageDrawsItsNumbersAndACompressedOneKeepsItsColour()
    {
        var scene = new SceneModel();
        Mesh(scene, "cube", Vector3.Zero);
        Object(scene, "directionalLight", """{"color": [1, 1, 1], "intensity": 3}""", Vector3.Zero);
        var camera = CameraView.LookAt(new Vector3(0, 0, 3), Vector3.Zero, 50, 1);
        (byte R, byte G, byte B, byte A) With(Image image)
        {
            var frame = Extraction.Extract(scene, camera, Options);
            return At(Gpu.Render(frame with { Items = [frame.Items[0] with { Textures = [image] }] }, 64, 64), 32, 32);
        }
        static byte[] Repeat(byte[] pixel) => [.. Enumerable.Repeat(pixel, 16).SelectMany(p => p)];
        var grey = Repeat([128, 128, 128, 255]);
        var colour = With(new Image(11, 4, 4, grey));
        var data = With(new Image(13, 4, 4, grey) { Data = true });
        Assert.True(Brightness(data) > Brightness(colour) + 30, $"128 as a number is brighter than 128 as an sRGB colour: {data} {colour}");

        var red = new Image(15, 4, 4, Repeat([230, 20, 20, 255]));
        red = red with { Blocks = Loader.Compress(red, Compression.Bc1).Value };
        var drawn = With(red);
        Assert.True(drawn.R > drawn.B + 60, $"the compressed red image draws red: {drawn}");
        // Image ids are hashes: one with its top bit set draws its own colour too.
        var high = With(new Image((1UL << 63) | 17, 4, 4, Repeat([20, 20, 230, 255])));
        Assert.True(high.B > high.R + 60, $"an image with a large id draws blue: {high}");
    }

    /// <summary>The columns of row <paramref name="row"/> brighter than 300 when <paramref name="renderer"/> draws <paramref name="frame"/>.</summary>
    private static List<uint> LitColumns(Renderer renderer, RenderTarget target, Frame frame, uint row)
    {
        var gpu = Gpu.Get();
        lock (Gpu.Gate)
        {
            renderer.Render(target, frame);
            var image = gpu.ReadBack(target);
            Assert.True(image.IsOk, image.Error?.Message);
            return [.. Enumerable.Range(0, 64).Select(x => (uint)x).Where(x => Brightness(At(image.Value, x, row)) > 300)];
        }
    }

    [Fact]
    public void AModelAnimationMovesTheModelFromFrameToFrame()
    {
        var gpu = Gpu.Get();
        var dir = TestFiles.AssetFolder("animated");
        // The wall, sliding 1.5 to the right over a second.
        File.WriteAllBytes(Path.Join(dir, "slide.bin"), TestFiles.Floats(0, 1, 0, 0, 0, 1.5f, 0, 0));
        File.WriteAllText(Path.Join(dir, "slide.gltf"), TestFiles.WallGltf("",
            """, {"uri": "slide.bin", "byteLength": 32}""",
            """, {"buffer": 1, "byteOffset": 0, "byteLength": 8}, {"buffer": 1, "byteOffset": 8, "byteLength": 24}""",
            """, {"bufferView": 2, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1]}, {"bufferView": 3, "componentType": 5126, "count": 2, "type": "VEC3"}""",
            """, "animations": [{"name": "Slide", "channels": [{"sampler": 0, "target": {"node": 0, "path": "translation"}}], "samplers": [{"input": 2, "output": 3}]}]"""));
        var scene = new SceneModel();
        var wall = Object(scene, "mesh", """{"shape": "cube", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 1, "model": "slide.gltf"}""", Vector3.Zero);
        Assert.True(scene.AddComponent(wall, "modelAnimation").IsOk);
        Object(scene, "directionalLight", """{"color": [1, 1, 1], "intensity": 3}""", Vector3.Zero);
        var camera = CameraView.LookAt(new Vector3(0, 0, 4), Vector3.Zero, 60, 1);
        var assets = FileAssets.Unconfined(dir);
        using var renderer = new Renderer(gpu.Device);
        using var target = new RenderTarget(gpu.Device, 64, 64);
        var start = LitColumns(renderer, target, Extraction.Extract(scene, camera, Options, assets), 32);
        var later = LitColumns(renderer, target, Extraction.Extract(scene, camera, Options with { Time = 0.9f }, assets), 32);
        Assert.True(start.Count > 0 && later.Count > 0, "the wall shows");
        Assert.True(later[0] > start[0] + 8, $"later the same renderer draws it further right: {start[0]} then {later[0]}");
        TestFiles.Remove(dir);
    }

    [Fact]
    public void AWorldPositionPatternStaysOnItsSurfaceWhereverTheViewsOriginIs()
    {
        // A floor with a checker of world positions, half a unit to a cell.
        var graph = Graph.Graph.Create(Catalog);
        var output = graph.Nodes.First(n => n.Kind == "output").Id;
        var checker = Add(graph, "checker");
        Set(graph, checker, "scale", "2.0");
        Link(graph, checker, "value", output, "baseColor");
        var material = Compile(graph);
        var scene = Floor(("directionalLight", """{"color": [1, 1, 1], "intensity": 2}""", new Vector3(0, 5, 0), new Vector3(-60, 20, 0)));
        RenderedImage Draw(CameraView camera)
        {
            var frame = Extraction.Extract(scene, camera, Options);
            return Gpu.Render(frame with { Items = [.. frame.Items.Select(i => i with { Material = material })] }, 64, 64);
        }
        static int Differing(RenderedImage a, RenderedImage b) =>
            Enumerable.Range(0, 64 * 64).Count(p => Enumerable.Range(0, 4).Any(c => Math.Abs(a.Pixels[p * 4 + c] - b.Pixels[p * 4 + c]) > 8));
        // The same view, measured from the world's centre and from the eye (as cameras in the scene
        // are), and from the eye after it has moved a little sideways.
        var (eye, at) = (new Double3(0.3, 3, 4), new Double3(0.3, 0, 0));
        var centred = Draw(CameraView.LookAt(CameraView.Single(eye), CameraView.Single(at), 50, 1));
        var fromEye = Draw(CameraView.LookAtWorld(eye, at, 50, 1));
        Assert.True(Differing(centred, fromEye) < 40, "the cells stay where they are in the world");
        // Moved by one whole cell pair, the floor looks the same.
        var moved = Draw(CameraView.LookAtWorld(eye + new Double3(1, 0, 0), at + new Double3(1, 0, 0), 50, 1));
        Assert.True(Differing(fromEye, moved) < 40, "a unit along, the checker repeats");
    }

    [Fact]
    public void AMaterialShaderTheGpuRefusesIsReportedAndDrawnPlainly()
    {
        var gpu = Gpu.Get();
        var plain = Compile(Graph.Graph.Create(Catalog));
        var broken = plain with { Surface = "fn surface(s: SurfaceIn) -> Surface { return not_a_function(); }", Key = plain.Key ^ 1 };
        var scene = new SceneModel();
        Object(scene, "mesh", """{"shape": "sphere", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 0.5, "material": "m.bmaterial"}""", Vector3.Zero);
        var frame = Extraction.Extract(scene, CameraView.LookAt(new Vector3(0, 0, 3), Vector3.Zero, 50, 1), Options, new Materials(null, broken));
        lock (Gpu.Gate)
        {
            var refused = gpu.Render(frame, 32, 32);
            Assert.True(refused.IsError && refused.Error.Kind == RenderErrorKind.Gpu && refused.Error.Message.Contains("refused", StringComparison.Ordinal), refused.IsOk ? "drawn" : refused.Error.Message);
            // The renderer itself goes on, drawing the sphere with the built-in surface.
            using var renderer = new Renderer(gpu.Device);
            using var target = new RenderTarget(gpu.Device, 32, 32);
            renderer.Render(target, frame);
            Assert.True(renderer.Refused(broken));
            Assert.True(gpu.ReadBack(target).IsOk);
        }
    }

    [Fact]
    public void ASkinnedModelBendsOnTheGpuAsOnTheCpu()
    {
        var dir = TestFiles.Folder("skinned");
        Assert.True(Fixtures.SkinnedArm(dir).IsOk);
        var scene = new SceneModel();
        var arm = Object(scene, "mesh", """{"shape": "cube", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 1, "model": "arm.gltf"}""", Vector3.Zero);
        Assert.True(scene.AddComponent(arm, "modelAnimation").IsOk);
        Assert.True(scene.SetComponent(arm, "modelAnimation", J("""{"animation": "Stretch", "playing": true, "looping": false, "speed": 1}""")).IsOk);
        Object(scene, "directionalLight", """{"color": [1, 1, 1], "intensity": 3}""", Vector3.Zero);
        var camera = CameraView.LookAt(new Vector3(0, 0, 5), Vector3.Zero, 60, 1);
        var assets = FileAssets.Unconfined(dir);
        var (rest, stretched) = (Extraction.Extract(scene, camera, Options, assets), Extraction.Extract(scene, camera, Options with { Time = 0.9f }, assets));
        Assert.Empty(assets.Warnings);
        Assert.NotNull(rest.Items[0].Skin);
        Assert.True(stretched.Items[0].LocalBounds().Max.X > rest.Items[0].LocalBounds().Max.X + 1, "the stretched arm's box reaches further");

        var gpu = Gpu.Get();
        using var renderer = new Renderer(gpu.Device);
        using var target = new RenderTarget(gpu.Device, 64, 64);
        List<uint> Columns(Frame frame, bool cpu)
        {
            renderer.CpuSkinning = cpu;
            return LitColumns(renderer, target, frame, 40);
        }
        var (gpuRest, gpuStretched) = (Columns(rest, false), Columns(stretched, false));
        var (cpuRest, cpuStretched) = (Columns(rest, true), Columns(stretched, true));
        Assert.True(gpuRest.Count > 0 && gpuStretched.Count > 0, "the arm shows");
        Assert.True(gpuStretched[^1] > gpuRest[^1] + 8, $"its right edge follows the Arm joint: {gpuRest[^1]} then {gpuStretched[^1]}");
        Assert.True(Math.Abs((int)gpuStretched[0] - (int)gpuRest[0]) <= 1, $"its left edge stays with Root: {gpuRest[0]} then {gpuStretched[0]}");
        foreach (var (onGpu, onCpu) in new[] { (gpuRest, cpuRest), (gpuStretched, cpuStretched) })
        {
            Assert.True(Math.Abs((int)onGpu[0] - (int)onCpu[0]) <= 1 && Math.Abs((int)onGpu[^1] - (int)onCpu[^1]) <= 1, "the GPU and the CPU pose it alike");
        }
        TestFiles.Remove(dir);
    }
}
