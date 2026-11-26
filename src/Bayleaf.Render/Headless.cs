using Bayleaf.Core;
using Bayleaf.Native;

namespace Bayleaf.Render;

public enum RenderErrorKind
{
    /// <summary>No graphics adapter could be used.</summary>
    NoGpu,

    /// <summary>The GPU refused or failed something.</summary>
    Gpu,
}

/// <summary>Why rendering failed. The message is shown to the user, worded as the Rust engine words it.</summary>
public sealed record RenderError(RenderErrorKind Kind, string Detail)
{
    public string Message => Kind == RenderErrorKind.NoGpu ? $"no usable graphics adapter: {Detail}" : $"graphics error: {Detail}";

    public override string ToString() => Message;
}

/// <summary>An RGBA8 image, rows top to bottom.</summary>
public sealed record RenderedImage(uint Width, uint Height, byte[] Pixels)
{
    /// <summary>The pixel at (<paramref name="x"/>, <paramref name="y"/>), or null outside the image.</summary>
    public (byte R, byte G, byte B, byte A)? Pixel(uint x, uint y)
    {
        if (x >= Width || y >= Height)
        {
            return null;
        }
        var at = (int)((y * Width + x) * 4);
        return (Pixels[at], Pixels[at + 1], Pixels[at + 2], Pixels[at + 3]);
    }
}

/// <summary>A device for rendering without a window: a frame to RGBA pixels.</summary>
public sealed class HeadlessGpu : IDisposable
{
    private HeadlessGpu(GpuDevice device) => Device = device;

    public GpuDevice Device { get; }

    /// <summary>
    /// Picks a GPU, falling back to a software adapter when there is none. <c>WGPU_BACKEND</c> and
    /// <c>WGPU_POWER_PREF</c> choose another.
    /// </summary>
    public static Result<HeadlessGpu, RenderError> Create()
    {
        var device = GpuDevice.Create(out var error);
        return device is null ? Result.Fail(new RenderError(RenderErrorKind.NoGpu, error)) : new HeadlessGpu(device);
    }

    /// <summary>Renders <paramref name="frame"/> at <paramref name="width"/> by <paramref name="height"/> and reads the pixels back.</summary>
    public Result<RenderedImage, RenderError> Render(Frame frame, uint width, uint height) => RenderWithSamples(frame, width, height, RenderTarget.DefaultSamples);

    /// <summary><see cref="Render"/> with <paramref name="samples"/> per pixel: 1 without multisampling.</summary>
    public Result<RenderedImage, RenderError> RenderWithSamples(Frame frame, uint width, uint height, uint samples)
    {
        using var renderer = new Renderer(Device, samples);
        using var target = new RenderTarget(Device, width, height, samples);
        renderer.Render(target, frame);
        if (renderer.MaterialError is { } error)
        {
            Device.TakeErrors();
            return Result.Fail(new RenderError(RenderErrorKind.Gpu, $"a material shader was refused: {error}"));
        }
        return ReadBack(target);
    }

    /// <summary>What was last drawn into <paramref name="target"/>, as pixels; an error the GPU reported meanwhile fails it.</summary>
    public Result<RenderedImage, RenderError> ReadBack(RenderTarget target)
    {
        var (width, height) = (target.Width, target.Height);
        var unpadded = width * 4;
        var padded = (unpadded + GpuEncoder.RowAlignment - 1) / GpuEncoder.RowAlignment * GpuEncoder.RowAlignment;
        using var buffer = Device.CreateBuffer("bayleaf readback", (ulong)padded * height, GpuBufferUsage.CopyDestination | GpuBufferUsage.MapRead);
        using (var encoder = Device.CreateEncoder("bayleaf readback"))
        {
            encoder.CopyTextureToBuffer(target.Color, buffer, padded, width, height);
            Device.Submit(encoder.Finish());
        }
        var data = new byte[padded * height];
        if (Device.ReadBuffer(buffer, data) is { } why)
        {
            return Result.Fail(new RenderError(RenderErrorKind.Gpu, why));
        }
        if (Device.TakeErrors() is { Count: > 0 } errors)
        {
            return Result.Fail(new RenderError(RenderErrorKind.Gpu, errors[0]));
        }
        var pixels = new byte[unpadded * height];
        for (var row = 0; row < height; row++)
        {
            Array.Copy(data, row * padded, pixels, row * unpadded, unpadded);
        }
        return new RenderedImage(width, height, pixels);
    }

    public void Dispose() => Device.Dispose();
}
