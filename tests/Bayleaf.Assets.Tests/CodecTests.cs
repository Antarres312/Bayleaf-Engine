namespace Bayleaf.Assets.Tests;

/// <summary>Behaviour of the readers and the block encoder that the shared files do not show.</summary>
public class CodecTests
{
    private static byte[] Shared(string name) => File.ReadAllBytes(Path.Join(SharedFilesTests.RepositoryRoot(), "tests", "files", "assets", name));

    [Fact]
    public void AJpegWhoseDataEndsEarlyKeepsItsSizeAndIsGreyWhereTheDataIsMissing()
    {
        var whole = Loader.DecodeJpeg(Shared("jpeg-420.jpg")).Value;
        var bytes = Shared("jpeg-420.jpg");
        var cut = Loader.DecodeJpeg(bytes.AsSpan(0, bytes.Length * 2 / 3)).Value;
        Assert.Equal((whole.Width, whole.Height), (cut.Width, cut.Height));
        var last = cut.Rgba.AsSpan((cut.Width * (cut.Height - 1) + cut.Width - 1) * 4, 4).ToArray();
        Assert.Equal([128, 128, 128, 255], last);
        Assert.Equal(whole.Rgba.AsSpan(0, 16).ToArray(), cut.Rgba.AsSpan(0, 16).ToArray());
    }

    [Fact]
    public void AdobeRgbAndUnsupportedJpegKindsAreRefusedSayingSo()
    {
        Assert.Contains("RGB", Loader.DecodeJpeg(Shared("jpeg-rgb-adobe.jpg")).Error, StringComparison.Ordinal);
        Assert.Contains("arithmetic", Loader.DecodeJpeg(Shared("bad-jpeg-arithmetic.jpg")).Error, StringComparison.Ordinal);
        Assert.Contains("lossless", Loader.DecodeJpeg(Shared("bad-jpeg-lossless.jpg")).Error, StringComparison.Ordinal);
    }

    private static byte[][] Block(Func<int, byte[]> texel) => [.. Enumerable.Range(0, 16).Select(texel)];

    [Fact]
    public void Bc1PunchesThroughTexelsUnderHalfAlpha()
    {
        var block = Block(t => t < 8 ? [200, 100, 50, 255] : [9, 9, 9, 127]);
        var bytes = BlockCompression.EncodeBc1(block, ushort.MaxValue);
        Assert.True((bytes[0] | bytes[1] << 8) <= (bytes[2] | bytes[3] << 8), "three-colour mode");
        var decoded = BlockCompression.DecodeColour(bytes, true);
        Assert.All(decoded[..8], t => Assert.Equal(255, t[3]));
        Assert.All(decoded[8..], t => Assert.Equal([0, 0, 0, 0], t));
    }

    [Fact]
    public void TwoColourBlocksAreExactWhenTheColoursAre565()
    {
        var block = Block(t => t % 3 == 0 ? [255, 0, 0, 255] : [0, 0, 255, 255]);
        var decoded = BlockCompression.DecodeColour(BlockCompression.EncodeBc1(block, ushort.MaxValue), true);
        Assert.Equal(block, decoded);
    }

    [Fact]
    public void AlphaBlocksPickTheModeThatKeeps0And255()
    {
        var values = Enumerable.Range(0, 16).Select(t => new byte[] { 0, 255, 100, 110 }[t % 4]).ToArray();
        var bytes = BlockCompression.ValueBlock(values, ushort.MaxValue);
        Assert.True(bytes[0] <= bytes[1], "six-value mode");
        var palette = BlockCompression.AlphaPalette(bytes[0], bytes[1]);
        var bits = BitConverter.ToUInt64([.. bytes[2..8], 0, 0]);
        for (var t = 0; t < 16; t++)
        {
            Assert.True(Math.Abs(palette[(int)(bits >> (3 * t)) & 7] - values[t]) <= 2);
        }
    }

    [Fact]
    public void OpenExrColourChannelsComeFromOneLayer()
    {
        static string? Pick(params string[] names) => Exr.ColorChannels(names) is var (layout, indices) ? $"{layout} {string.Join(",", indices)}" : null;
        Assert.Equal("RGBA 3,2,1,0", Pick("A", "B", "G", "R"));
        Assert.Equal("RGB 3,2,1", Pick("A", "diffuse.B", "diffuse.G", "diffuse.R"));
        Assert.Null(Pick("a.R", "b.G", "c.B"));
        Assert.Equal("RGBA 3,4,5,6", Pick("R", "G", "B", "beauty.R", "beauty.G", "beauty.B", "beauty.A"));
        Assert.Equal("RGB 3,4,5", Pick("z.R", "z.G", "z.B", "R", "G", "B"));
        Assert.Equal("Y 0", Pick("depth.Z"));
        Assert.Equal("ViewLayer.Depth.Z", Exr.PartChannelName("ViewLayer.Depth", "ViewLayer.Depth.Z"));
        Assert.Equal("View.ViewLayer.Z", Exr.PartChannelName("View", "ViewLayer.Z"));
    }

    [Fact]
    public void AnOpenExrCompressionThatIsNotReadIsRefusedSayingSo()
    {
        // The ZIP fixture's compression attribute turned to PIZ: its chunks are not PIZ data, and it is refused before they are read.
        var bytes = Shared("exr-rgb-half-zip.exr");
        var at = bytes.AsSpan().IndexOf("compression\0compression\0"u8) + "compression\0compression\0"u8.Length + 4;
        bytes[at] = 4;
        Assert.Equal("unsupported OpenEXR: PIZ compression is not read yet", Loader.DecodeExr(bytes).Error);
    }
}
