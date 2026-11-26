using System.Numerics;

using static Bayleaf.Assets.Tests.TestFiles;

namespace Bayleaf.Assets.Tests;

/// <summary>The Rust crate's tests (<c>rust/crates/assets/src/tests.rs</c>), as behaviour.</summary>
public class AssetsTests
{
    [Fact]
    public void PngImagesBecomeRgbaAndBrokenFilesAreRefused()
    {
        var image = Loader.DecodePng(PngBytes()).Value;
        Assert.Equal((2, 1), (image.Width, image.Height));
        Assert.Equal([255, 0, 0, 255, 0, 0, 255, 255], image.Rgba);
        Assert.True(Loader.DecodePng("not a png"u8).IsError);
        var cut = PngBytes();
        Assert.True(Loader.DecodePng(cut.AsSpan(0, cut.Length / 2)).IsError);

        var dir = TempDir("png");
        File.WriteAllBytes(Path.Join(dir, "a.png"), PngBytes());
        Assert.NotEqual(0UL, Loader.LoadImage(Path.Join(dir, "a.png")).Value.Id);
        Remove(dir);
    }

    [Fact]
    public void AGltfModelLoadsWithItsNodeTransformAndMadeUpNormals()
    {
        var dir = TempDir("gltf");
        WriteTriangle(dir, "tri.bin", 2);
        var mesh = Loader.LoadModel(Path.Join(dir, "tri.gltf")).Value;
        Assert.Equal((3, 3), (mesh.Positions.Count, mesh.Indices.Count));
        Assert.All(mesh.Positions, p => Assert.True(MathF.Abs(p.Y - 2) < 1e-6f, "the node moves it up by 2"));
        Assert.All(mesh.Normals, n => Assert.True(n.Y > 0.99f, "a flat triangle facing up"));
        Assert.Equal(new Vector3(0, 2, -1), mesh.Min);
        Assert.Equal(new Vector3(1, 2, 0), mesh.Max);
        Remove(dir);
    }

    [Fact]
    public void HostileModelsAreRefusedWithAReason()
    {
        var dir = TempDir("gltf-hostile");
        var model = Path.Join(dir, "tri.gltf");
        WriteTriangle(dir, "tri.bin", 7);
        Assert.Contains("does not exist", Loader.LoadModel(model).Error, StringComparison.Ordinal);
        WriteTriangle(dir, "../outside.bin", 2);
        Assert.Contains("next to the model", Loader.LoadModel(model).Error, StringComparison.Ordinal);
        WriteTriangle(dir, "data:application/octet-stream;base64,AAAA", 2);
        Assert.Contains(".glb", Loader.LoadModel(model).Error, StringComparison.Ordinal);
        File.WriteAllBytes(Path.Join(dir, "junk.glb"), "glTF junk"u8.ToArray());
        Assert.True(Loader.LoadModel(Path.Join(dir, "junk.glb")).IsError);
        Assert.True(Loader.LoadModel(Path.Join(dir, "missing.glb")).IsError);
        Remove(dir);
    }

    [Fact]
    public void JpegImagesDecodeByTheirBytesAndOversizedOnesAreRefused()
    {
        var image = Loader.DecodeImage(GreyJpeg(16)).Value;
        Assert.Equal((16, 8), (image.Width, image.Height));
        for (var i = 0; i < image.Rgba.Length; i += 4)
        {
            Assert.True(Math.Abs(image.Rgba[i] - 128) <= 2 && image.Rgba[i + 3] == 255, "grey and opaque");
        }
        Assert.True(Loader.DecodeImage(GreyJpeg(9000)).IsError, "wider than the limit");

        var dir = TempDir("jpeg");
        // The kind comes from the bytes: a JPEG named .png still loads.
        File.WriteAllBytes(Path.Join(dir, "photo.png"), GreyJpeg(16));
        Assert.Equal(16, Loader.LoadImage(Path.Join(dir, "photo.png")).Value.Width);
        Remove(dir);
    }

    private static byte[] Checker(int width, int height) =>
        [.. Enumerable.Range(0, width * height).SelectMany(i => (i % width + i / width) % 2 == 0 ? new byte[] { 0, 0, 0, 255 } : [255, 255, 255, 255])];

    [Fact]
    public void SmallerCopiesHalveDownToOnePixelAndAverageAsLight()
    {
        // A 4 x 3 black and white checkerboard.
        var levels = Loader.MipLevels(new Image(1, 4, 3, Checker(4, 3)));
        Assert.Equal([(2, 1), (1, 1)], levels.Select(l => (l.Width, l.Height)));
        Assert.All(levels, l => Assert.Equal(l.Width * l.Height * 4, l.Rgba.Length));
        var grey = levels[0].Rgba[0];
        Assert.True(grey > 160, $"half black, half white is light grey as light, not 128: {grey}");
        Assert.Equal(255, levels[0].Rgba[3]);
    }

    [Fact]
    public void AGltfMaterialGivesItsColourMetalRoughnessAndImage()
    {
        var dir = TempDir("gltf-material");
        const string materials = """
            , "materials": [{"pbrMetallicRoughness": {"baseColorFactor": [0.2, 0.8, 0.1, 1.0], "metallicFactor": 0.25, "roughnessFactor": 0.75,
                "baseColorTexture": {"index": 0}, "metallicRoughnessTexture": {"index": 0}}, "normalTexture": {"index": 0}}],
              "textures": [{"source": 0}]
            """;
        WriteTriangle(dir, "tri.bin", 2, materials + """, "images": [{"uri": "colour.png"}]""", """, "material": 0""");
        File.WriteAllBytes(Path.Join(dir, "colour.png"), PngBytes());
        var mesh = Loader.LoadModel(Path.Join(dir, "tri.gltf")).Value;
        var material = mesh.Material!;
        Assert.Equal(new Vector4(0.2f, 0.8f, 0.1f, 1f), material.BaseColor);
        Assert.Equal((0.25f, 0.75f), (material.Metallic, material.Roughness));
        Assert.Equal(2, material.Image!.Width);
        Assert.NotEqual(mesh.Id, material.Image.Id);
        Assert.True(material.MetalRoughness is not null && material.Normal is not null, "the metal/roughness and normal images load too");

        WriteTriangle(dir, "tri.bin", 2, materials + """, "images": [{"uri": "../outside.png"}]""", """, "material": 0""");
        Assert.Contains("next to the model", Loader.LoadModel(Path.Join(dir, "tri.gltf")).Error, StringComparison.Ordinal);
        Remove(dir);
    }

    [Fact]
    public void ImportSettingsBesideAModelScaleAndTurnItAndReloadItWhenChanged()
    {
        var dir = TempDir("import-settings");
        WriteTriangle(dir, "tri.bin", 2);
        var model = Path.Join(dir, "tri.gltf");
        var cache = new AssetCache();
        var plain = cache.Model(model).Value;
        Assert.True(plain.Max.Y > 1.9f, "the node lifts it along Y");

        var settings = ImportSettings.DefaultsFor(model)!;
        settings = settings with { Model = new ModelImport(3f, UpAxis.Z) };
        // Each write later than the one before, as an edit is (the file system's clock may be coarser than the test).
        var settingsPath = ImportSettings.SettingsPath(model);
        File.WriteAllText(settingsPath, settings.ToText());
        File.SetLastWriteTimeUtc(settingsPath, DateTime.UtcNow.AddSeconds(1));
        var turned = cache.Model(model).Value;
        Assert.NotEqual(plain.Id, turned.Id);
        // Up along Z becomes up along Y: what the node lifted along Y now points away from the viewer.
        Assert.True(turned.Max.Y < 0.5f && turned.Min.Z < -5f, $"scaled and turned: {turned.Min} to {turned.Max}");
        static float Size(MeshData m) => MathF.Max(MathF.Max(m.Max.X - m.Min.X, m.Max.Y - m.Min.Y), m.Max.Z - m.Min.Z);
        Assert.True(Size(turned) > Size(plain) * 2, "scaled up");

        File.WriteAllText(settingsPath, "{ broken");
        File.SetLastWriteTimeUtc(settingsPath, DateTime.UtcNow.AddSeconds(2));
        Assert.Contains("import settings", cache.Model(model).Error, StringComparison.Ordinal);
        Remove(dir);
    }

    [Fact]
    public void AnImageWithoutMipmapsInItsSettingsSaysSo()
    {
        var dir = TempDir("import-image");
        var path = Path.Join(dir, "a.png");
        File.WriteAllBytes(path, PngBytes());
        Assert.True(Loader.LoadImage(path).Value.Mipmaps, "on unless turned off");
        var settings = ImportSettings.DefaultsFor(path)!;
        File.WriteAllText(ImportSettings.SettingsPath(path), (settings with { Image = settings.Image! with { Mipmaps = false } }).ToText());
        Assert.False(Loader.LoadImage(path).Value.Mipmaps);
        Remove(dir);
    }

    [Fact]
    public void AnExrPicturesLinearLightBecomesSrgbAndBrokenOnesAreRefused()
    {
        // 2 x 1: linear half grey, then black, as an uncompressed OpenEXR file.
        var linear = Enumerable.Range(0, 256).Select(i => i / 255f).ToArray();
        var exr = Exr([128, 128, 128, 255, 0, 0, 0, 255], 2, 1, linear);
        var image = Loader.DecodeImage(exr).Value;
        Assert.Equal((2, 1), (image.Width, image.Height));
        var grey = image.Rgba[0];
        Assert.True(grey is > 170 and < 200, $"linear half grey is stored as its sRGB number: {grey}");
        Assert.Equal([0, 0, 0], image.Rgba[4..7]);
        Assert.True(Loader.DecodeImage(exr.AsSpan(0, exr.Length / 2)).IsError, "a cut file is refused");
    }

    [Fact]
    public void AnObjModelReadsItsMaterialLibraryAndImageBesideIt()
    {
        var dir = TempDir("obj");
        // A square of two triangles, its material red with an image, and a second part with none.
        File.WriteAllText(Path.Join(dir, "square.obj"),
            "mtllib square.mtl\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nusemtl Red\nf 1/1 2/2 3/3 4/4\nusemtl Missing\nf 1/1 3/3 4/4\n");
        File.WriteAllText(Path.Join(dir, "square.mtl"), "newmtl Red\nKd 1 0 0\nmap_Kd red.png\nnewmtl Missing\nKd 0 0 1\nmap_Kd C:\\Artists\\gone.png\n");
        File.WriteAllBytes(Path.Join(dir, "red.png"), PngBytes());
        var model = Loader.LoadModel(Path.Join(dir, "square.obj")).Value;
        Assert.Equal(9, model.Indices.Count);
        Assert.Equal(2, model.Parts.Count);
        var red = model.Parts[0].Material!;
        Assert.True(red.BaseColor.X > 0.9f && red.BaseColor.Z < 0.1f && red.Image is not null, $"{red}");
        Assert.Null(model.Parts[1].Material!.Image);
        Assert.Contains(model.Warnings, w => w.Contains("gone.png", StringComparison.Ordinal));
        var files = Loader.ModelFiles(Path.Join(dir, "square.obj")).Value;
        files.Sort(StringComparer.Ordinal);
        Assert.Equal(["red.png", "square.mtl"], files);
        // Broken and hostile models are refused with a reason.
        File.WriteAllText(Path.Join(dir, "bad.obj"), "v 0 0 0\nf 1 2 3\n");
        Assert.True(Loader.LoadModel(Path.Join(dir, "bad.obj")).IsError, "a face naming vertices that are not there");
        File.WriteAllText(Path.Join(dir, "empty.obj"), "# nothing\n");
        Assert.True(Loader.LoadModel(Path.Join(dir, "empty.obj")).IsError, "a model with nothing to draw");
        Remove(dir);
    }

    [Fact]
    public void ADataImageAveragesItsNumbersAndCompressionKeepsEveryLevelInBlocks()
    {
        var checker = Checker(4, 4);
        var colour = Loader.MipLevels(new Image(1, 4, 4, checker))[0].Rgba[0];
        var data = Loader.MipLevels(new Image(1, 4, 4, checker) { Data = true })[0].Rgba[0];
        Assert.True(Math.Abs(data - 128) <= 1 && colour > data + 40, $"numbers average as numbers ({data}), colours as light ({colour})");

        var image = new Image(1, 8, 8, [.. Enumerable.Repeat(checker, 4).SelectMany(c => c)]);
        var bc1 = Loader.Compress(image, Compression.Bc1).Value;
        Assert.Equal([32, 8, 8, 8], bc1.Levels.Select(l => l.Length));
        var bc3 = Loader.Compress(image, Compression.Bc3).Value;
        Assert.Equal(bc1.Levels[0].Length * 2, bc3.Levels[0].Length);
        Assert.True(Loader.Compress(new Image(1, 6, 4, new byte[96]), Compression.Bc1).IsError, "sides must be multiples of 4");
    }

    [Fact]
    public void AModelsAnimationMovesItsPartsAndItsSkin()
    {
        var dir = TempDir("animated");
        foreach (var skinned in new[] { false, true })
        {
            var model = Loader.LoadModel(AnimatedTriangle(dir, skinned)).Value;
            var skeleton = model.Skeleton!;
            var slide = skeleton.Animation("Slide")!.Value;
            Assert.Equal(slide, skeleton.Animation(""));
            Assert.True(MathF.Abs(skeleton.Animations[slide].Duration - 1) < 1e-6f);
            static float X(MeshData mesh) => mesh.Positions[1].X;
            Assert.True(MathF.Abs(X(model) - 1) < 1e-5f, $"at rest where it was made (skinned: {skinned})");
            var half = skeleton.Posed(model, slide, 0.5f, false, 7, 1);
            Assert.True(MathF.Abs(X(half) - 2) < 1e-4f, $"half way, a unit along (skinned: {skinned}): {X(half)}");
            var looped = skeleton.Posed(model, slide, 1.25f, true, 7, 2);
            Assert.True(MathF.Abs(X(looped) - 1.5f) < 1e-4f, $"looping wraps round: {X(looped)}");
            var held = skeleton.Posed(model, slide, 3f, false, 7, 3);
            Assert.True(MathF.Abs(X(held) - 3) < 1e-4f, $"not looping, it stays at the end: {X(held)}");
            Assert.True(half.Max.X > model.Max.X, "its box follows");
        }
        Remove(dir);
    }

    /// <summary>From the animation crate's tests of the generated arm: a mask covers a joint and the nodes under it.</summary>
    [Fact]
    public void AMaskCoversAJointAndTheNodesUnderIt()
    {
        var dir = TempDir("arm");
        var skeleton = Loader.LoadModel(Fixtures.SkinnedArm(dir).Value).Value.Skeleton!;
        Remove(dir);
        Assert.Equal([0f, 1f, 1f], skeleton.Mask("Arm")!);
        Assert.Null(skeleton.Mask("Missing"));
        Assert.Equal([1f, 1f, 1f], skeleton.Mask("")!);
        var pose = skeleton.Sampled(skeleton.Animation("Stretch")!.Value, 1f);
        var mixed = skeleton.RestPose().MixMasked(pose, 1f, skeleton.Mask("Hand")!);
        Assert.Equal(skeleton.RestPose().Locals[1], mixed.Locals[1]);
        Assert.True(skeleton.RestPose().Mix(pose, 0.5f).Locals[1].Translation.X is > 0.7f and < 0.8f, "half way to a stretch of 1.5");
    }
}
