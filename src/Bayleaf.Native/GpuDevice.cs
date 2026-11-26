using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

using Silk.NET.WebGPU;

namespace Bayleaf.Native;

/// <summary>
/// A graphics device of wgpu-native (the C interface of wgpu, reached through Silk.NET's WebGPU
/// bindings) and its queue, with what the renderer makes on it. Validation errors that no error scope
/// catches are kept (see <see cref="TakeErrors"/>), not left to wgpu-native, which would end the
/// process.
/// </summary>
public sealed class GpuDevice : IDisposable
{
    private static WebGPU? api;

    private nint instance;
    private nint adapter;
    private nint device;
    private nint queue;
    private nint poll;
    private GCHandle self;
    private readonly List<string> errors = [];

    private GpuDevice()
    {
    }

    internal static WebGPU Api => api ?? throw new InvalidOperationException("wgpu-native is not loaded");

    /// <summary>The adapter's name and the graphics interface it is reached through.</summary>
    public string Adapter { get; private set; } = "";

    /// <summary>Whether the device reads 4 x 4 block-compressed images (BC1 to BC3).</summary>
    public bool BlockCompression { get; private set; }

    public uint MaxVertexAttributes { get; private set; }

    public uint MaxTextureSide { get; private set; }

    /// <summary>
    /// A device on the first adapter wgpu-native offers (its software adapter when there is no other),
    /// or null with why not. <c>WGPU_BACKEND</c> (vulkan, dx12, metal, gl) and <c>WGPU_POWER_PREF</c>
    /// (low, high) choose another, as they do for the Rust engine.
    /// </summary>
    public static GpuDevice? Create(out string error)
    {
        try
        {
            api ??= WebGPU.GetApi();
        }
        catch (Exception e) when (e is DllNotFoundException or FileNotFoundException or BadImageFormatException or EntryPointNotFoundException or PlatformNotSupportedException or InvalidOperationException)
        {
            error = $"wgpu-native could not be loaded: {e.Message}";
            return null;
        }
        var gpu = new GpuDevice();
        error = gpu.Open() ?? "";
        if (error.Length > 0)
        {
            gpu.Dispose();
            return null;
        }
        return gpu;
    }

    private sealed class Answer
    {
        public bool Done;
        public bool Ok;
        public nint Handle;
        public string Message = "";
    }

    private static BackendType Backend() => (System.Environment.GetEnvironmentVariable("WGPU_BACKEND") ?? "").Split(',')[0].Trim().ToLowerInvariant() switch
    {
        "vulkan" or "vk" => BackendType.Vulkan,
        "dx12" or "d3d12" => BackendType.D3D12,
        "metal" or "mtl" => BackendType.Metal,
        "gl" or "opengl" or "gles" => BackendType.OpenGL,
        _ => BackendType.Undefined,
    };

    private static PowerPreference Power() => (System.Environment.GetEnvironmentVariable("WGPU_POWER_PREF") ?? "").Trim().ToLowerInvariant() switch
    {
        "low" => PowerPreference.LowPower,
        "high" => PowerPreference.HighPerformance,
        _ => PowerPreference.Undefined,
    };

    /// <summary>Opens the instance, the adapter, the device and its queue; why not, or null.</summary>
    private string? Open()
    {
        // SAFETY: every pointer passed lives in the arena or on the stack for the call; the answers
        // come back through callbacks that wgpu-native calls before the request returns (checked:
        // a request without an answer is an error), and the handle they write through is freed after.
        unsafe
        {
            using var arena = new Arena();
            instance = (nint)Api.CreateInstance(null);
            if (instance == 0)
            {
                return "wgpu-native made no instance";
            }
            nint Request(bool fallback, out string message)
            {
                var options = new RequestAdapterOptions { PowerPreference = Power(), BackendType = Backend(), ForceFallbackAdapter = fallback };
                var answer = new Answer();
                var handle = GCHandle.Alloc(answer);
                try
                {
                    Api.InstanceRequestAdapter((Instance*)instance, &options, new PfnRequestAdapterCallback(&OnAdapter), (void*)GCHandle.ToIntPtr(handle));
                }
                finally
                {
                    handle.Free();
                }
                message = answer.Done ? answer.Message : "the adapter request was not answered";
                return answer.Ok ? answer.Handle : 0;
            }
            adapter = Request(false, out var why);
            if (adapter == 0)
            {
                adapter = Request(true, out var fallbackWhy);
                if (adapter == 0)
                {
                    return why.Length > 0 ? why : fallbackWhy;
                }
            }
            var properties = new AdapterProperties();
            Api.AdapterGetProperties((Silk.NET.WebGPU.Adapter*)adapter, &properties);
            Adapter = $"{Marshal.PtrToStringUTF8((nint)properties.Name)} ({properties.BackendType})";
            BlockCompression = Api.AdapterHasFeature((Silk.NET.WebGPU.Adapter*)adapter, FeatureName.TextureCompressionBC);
            var features = arena.Alloc<FeatureName>();
            var count = 0;
            if (BlockCompression)
            {
                features[count++] = FeatureName.TextureCompressionBC;
            }
            var desc = arena.Alloc<DeviceDescriptor>();
            desc->Label = arena.Text("bayleaf headless");
            desc->RequiredFeatureCount = (nuint)count;
            desc->RequiredFeatures = features;
            desc->DefaultQueue = new QueueDescriptor { Label = arena.Text("bayleaf queue") };
            desc->DeviceLostCallback = new PfnDeviceLostCallback(&OnLost);
            var deviceAnswer = new Answer();
            var deviceHandle = GCHandle.Alloc(deviceAnswer);
            try
            {
                Api.AdapterRequestDevice((Silk.NET.WebGPU.Adapter*)adapter, desc, new PfnRequestDeviceCallback(&OnDevice), (void*)GCHandle.ToIntPtr(deviceHandle));
            }
            finally
            {
                deviceHandle.Free();
            }
            if (!deviceAnswer.Ok || deviceAnswer.Handle == 0)
            {
                return deviceAnswer.Done ? deviceAnswer.Message : "the device request was not answered";
            }
            device = deviceAnswer.Handle;
            queue = (nint)Api.DeviceGetQueue((Device*)device);
            self = GCHandle.Alloc(this);
            Api.DeviceSetUncapturedErrorCallback((Device*)device, new PfnErrorCallback(&OnUncaptured), (void*)GCHandle.ToIntPtr(self));
            var limits = new SupportedLimits();
            Api.DeviceGetLimits((Device*)device, &limits);
            MaxVertexAttributes = limits.Limits.MaxVertexAttributes;
            MaxTextureSide = limits.Limits.MaxTextureDimension2D;
            // wgpu-native's own call that waits for the GPU (it is not in webgpu.h).
            poll = (nint)Api.Context.GetProcAddress("wgpuDevicePoll");
            return null;
        }
    }

    // SAFETY: called by wgpu-native with the userdata we passed, a GCHandle to an Answer that is
    // alive for the whole request; the message is a zero-ended string or null.
    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static unsafe void OnAdapter(RequestAdapterStatus status, Silk.NET.WebGPU.Adapter* adapter, byte* message, void* userdata)
    {
        if (GCHandle.FromIntPtr((nint)userdata).Target is Answer answer)
        {
            answer.Done = true;
            answer.Ok = status == RequestAdapterStatus.Success;
            answer.Handle = (nint)adapter;
            answer.Message = Marshal.PtrToStringUTF8((nint)message) ?? status.ToString();
        }
    }

    // SAFETY: as OnAdapter, for the device request.
    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static unsafe void OnDevice(RequestDeviceStatus status, Device* device, byte* message, void* userdata)
    {
        if (GCHandle.FromIntPtr((nint)userdata).Target is Answer answer)
        {
            answer.Done = true;
            answer.Ok = status == RequestDeviceStatus.Success;
            answer.Handle = (nint)device;
            answer.Message = Marshal.PtrToStringUTF8((nint)message) ?? status.ToString();
        }
    }

    // SAFETY: called by wgpu-native when the device is lost or dropped; nothing is read but the reason.
    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static unsafe void OnLost(DeviceLostReason reason, byte* message, void* userdata)
    {
    }

    // SAFETY: called by wgpu-native with the userdata we set, a GCHandle to this device that lives
    // until the device is released; the message is a zero-ended string or null.
    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static unsafe void OnUncaptured(ErrorType type, byte* message, void* userdata)
    {
        if (userdata != null && GCHandle.FromIntPtr((nint)userdata).Target is GpuDevice gpu)
        {
            lock (gpu.errors)
            {
                gpu.errors.Add(Marshal.PtrToStringUTF8((nint)message) ?? type.ToString());
            }
        }
    }

    // SAFETY: called by wgpu-native with the userdata we passed, a GCHandle to an Answer alive for
    // the call that pops the scope.
    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static unsafe void OnScope(ErrorType type, byte* message, void* userdata)
    {
        if (GCHandle.FromIntPtr((nint)userdata).Target is Answer answer)
        {
            answer.Done = true;
            answer.Ok = type == ErrorType.NoError;
            answer.Message = Marshal.PtrToStringUTF8((nint)message) ?? type.ToString();
        }
    }

    // SAFETY: called by wgpu-native while the device polls, with the userdata we passed, a GCHandle
    // to an Answer alive until the read has finished.
    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvCdecl)])]
    private static unsafe void OnMapped(BufferMapAsyncStatus status, void* userdata)
    {
        if (GCHandle.FromIntPtr((nint)userdata).Target is Answer answer)
        {
            answer.Done = true;
            answer.Ok = status == BufferMapAsyncStatus.Success;
            answer.Message = status.ToString();
        }
    }

    /// <summary>The validation errors no error scope caught since the last call, in order.</summary>
    public IReadOnlyList<string> TakeErrors()
    {
        lock (errors)
        {
            var taken = errors.ToList();
            errors.Clear();
            return taken;
        }
    }

    private void Live() => ObjectDisposedException.ThrowIf(device == 0, this);

    /// <summary>Starts catching validation errors, until <see cref="PopErrorScope"/>.</summary>
    public void PushErrorScope()
    {
        // SAFETY: the device is live (checked).
        unsafe
        {
            Live();
            Api.DevicePushErrorScope((Device*)device, ErrorFilter.Validation);
        }
    }

    /// <summary>The first validation error since <see cref="PushErrorScope"/>, or null.</summary>
    public string? PopErrorScope()
    {
        // SAFETY: the device is live; wgpu-native answers before the call returns, through the
        // handle that is freed after it.
        unsafe
        {
            Live();
            var answer = new Answer();
            var handle = GCHandle.Alloc(answer);
            try
            {
                Api.DevicePopErrorScope((Device*)device, new PfnErrorCallback(&OnScope), (void*)GCHandle.ToIntPtr(handle));
            }
            finally
            {
                handle.Free();
            }
            return answer.Done && !answer.Ok ? answer.Message : null;
        }
    }

    public GpuBuffer CreateBuffer(string label, ulong size, GpuBufferUsage usage)
    {
        // SAFETY: the device is live; the descriptor lives in the arena for the call.
        unsafe
        {
            Live();
            using var arena = new Arena();
            var desc = new BufferDescriptor { Label = arena.Text(label), Usage = Conv.Usage(usage), Size = size, MappedAtCreation = false };
            return new GpuBuffer((nint)Api.DeviceCreateBuffer((Device*)device, &desc), size);
        }
    }

    /// <summary>A buffer holding <paramref name="contents"/> (its size rounded up to 4 bytes).</summary>
    public GpuBuffer CreateBuffer(string label, ReadOnlySpan<byte> contents, GpuBufferUsage usage)
    {
        var size = (ulong)((contents.Length + 3) & ~3);
        if (size == 0)
        {
            return CreateBuffer(label, 0, usage);
        }
        // SAFETY: the device is live; a buffer mapped at creation can be written through its mapped
        // range, which holds `size` bytes, until it is unmapped.
        unsafe
        {
            Live();
            using var arena = new Arena();
            var desc = new BufferDescriptor { Label = arena.Text(label), Usage = Conv.Usage(usage), Size = size, MappedAtCreation = true };
            var buffer = Api.DeviceCreateBuffer((Device*)device, &desc);
            var mapped = Api.BufferGetMappedRange(buffer, 0, (nuint)size);
            if (mapped != null)
            {
                contents.CopyTo(new Span<byte>(mapped, contents.Length));
            }
            Api.BufferUnmap(buffer);
            return new GpuBuffer((nint)buffer, size);
        }
    }

    public void WriteBuffer(GpuBuffer buffer, ulong offset, ReadOnlySpan<byte> data)
    {
        // SAFETY: the device and the buffer are live; the data is pinned for the call, which copies it.
        unsafe
        {
            Live();
            fixed (byte* bytes = data)
            {
                Api.QueueWriteBuffer((Queue*)queue, (Silk.NET.WebGPU.Buffer*)buffer.Handle, offset, bytes, (nuint)data.Length);
            }
        }
    }

    public GpuTexture CreateTexture(GpuTextureDesc d)
    {
        // SAFETY: the device is live; the descriptor and its view format live in the arena for the call.
        unsafe
        {
            Live();
            using var arena = new Arena();
            var desc = new TextureDescriptor
            {
                Label = arena.Text(d.Label),
                Usage = Conv.Usage(d.Usage),
                Dimension = TextureDimension.Dimension2D,
                Size = new Extent3D { Width = Math.Max(d.Width, 1), Height = Math.Max(d.Height, 1), DepthOrArrayLayers = Math.Max(d.Layers, 1) },
                Format = Conv.Format(d.Format),
                MipLevelCount = Math.Max(d.MipLevels, 1),
                SampleCount = Math.Max(d.Samples, 1),
            };
            if (d.ViewFormat is { } view)
            {
                desc.ViewFormatCount = 1;
                desc.ViewFormats = arena.One(Conv.Format(view));
            }
            return new GpuTexture((nint)Api.DeviceCreateTexture((Device*)device, &desc), d);
        }
    }

    /// <summary>
    /// A texture filled with <paramref name="data"/>: each level in turn, largest first, its rows of
    /// texels (or 4 x 4 blocks) packed. It may also be copied into.
    /// </summary>
    public GpuTexture CreateTexture(GpuTextureDesc d, ReadOnlySpan<byte> data)
    {
        var texture = CreateTexture(d with { Usage = d.Usage | GpuTextureUsage.CopyDestination });
        var (side, bytes) = Conv.Block(d.Format);
        var offset = 0;
        for (var level = 0u; level < Math.Max(d.MipLevels, 1); level++)
        {
            var (width, height) = (Math.Max(Math.Max(d.Width, 1) >> (int)level, 1), Math.Max(Math.Max(d.Height, 1) >> (int)level, 1));
            // A block format's level covers whole blocks.
            var (across, down) = ((width + side - 1) / side, (height + side - 1) / side);
            var size = (int)(across * down * bytes);
            if (offset + size > data.Length)
            {
                break;
            }
            WriteTexture(texture, level, data.Slice(offset, size), across * bytes, down, across * side, down * side);
            offset += size;
        }
        return texture;
    }

    /// <summary>Writes level <paramref name="level"/> of a texture: <paramref name="rows"/> rows of <paramref name="bytesPerRow"/> bytes.</summary>
    public void WriteTexture(GpuTexture texture, uint level, ReadOnlySpan<byte> data, uint bytesPerRow, uint rows, uint width, uint height)
    {
        // SAFETY: the device and the texture are live; the data is pinned and the descriptors on the
        // stack for the call, which copies the data.
        unsafe
        {
            Live();
            var destination = new ImageCopyTexture { Texture = (Texture*)texture.Handle, MipLevel = level, Aspect = TextureAspect.All };
            var layout = new TextureDataLayout { Offset = 0, BytesPerRow = bytesPerRow, RowsPerImage = rows };
            var size = new Extent3D { Width = width, Height = height, DepthOrArrayLayers = 1 };
            fixed (byte* bytes = data)
            {
                Api.QueueWriteTexture((Queue*)queue, &destination, bytes, (nuint)data.Length, &layout, &size);
            }
        }
    }

    public GpuSampler CreateSampler(GpuSamplerDesc d)
    {
        // SAFETY: the device is live; the descriptor lives in the arena for the call.
        unsafe
        {
            Live();
            using var arena = new Arena();
            var address = d.Repeat ? AddressMode.Repeat : AddressMode.ClampToEdge;
            var filter = d.Linear ? FilterMode.Linear : FilterMode.Nearest;
            var desc = new SamplerDescriptor
            {
                Label = arena.Text(d.Label),
                AddressModeU = address,
                AddressModeV = address,
                AddressModeW = AddressMode.ClampToEdge,
                MagFilter = filter,
                MinFilter = filter,
                MipmapFilter = d.LinearMipmaps ? MipmapFilterMode.Linear : MipmapFilterMode.Nearest,
                LodMinClamp = 0,
                LodMaxClamp = 32,
                Compare = d.Compare ? CompareFunction.LessEqual : CompareFunction.Undefined,
                MaxAnisotropy = d.Anisotropy,
            };
            return new GpuSampler((nint)Api.DeviceCreateSampler((Device*)device, &desc));
        }
    }

    public GpuBindGroupLayout CreateBindGroupLayout(string label, IReadOnlyList<GpuLayoutEntry> entries)
    {
        // SAFETY: the device is live; the descriptor and its entries live in the arena for the call.
        unsafe
        {
            Live();
            using var arena = new Arena();
            var list = arena.Alloc<BindGroupLayoutEntry>(entries.Count);
            for (var i = 0; i < entries.Count; i++)
            {
                var e = entries[i];
                list[i].Binding = e.Binding;
                list[i].Visibility = Conv.Stages(e.Visibility);
                switch (e)
                {
                    case GpuUniformLayout u:
                        list[i].Buffer = new BufferBindingLayout { Type = BufferBindingType.Uniform, HasDynamicOffset = u.DynamicOffset, MinBindingSize = u.MinSize };
                        break;
                    case GpuTextureLayout t:
                        list[i].Texture = new TextureBindingLayout
                        {
                            SampleType = t.Sample switch
                            {
                                GpuSampleType.Filterable => TextureSampleType.Float,
                                GpuSampleType.Unfilterable => TextureSampleType.UnfilterableFloat,
                                _ => TextureSampleType.Depth,
                            },
                            ViewDimension = t.Dimension == GpuViewDimension.D2Array ? TextureViewDimension.Dimension2DArray : TextureViewDimension.Dimension2D,
                            Multisampled = t.Multisampled,
                        };
                        break;
                    case GpuSamplerLayout s:
                        list[i].Sampler = new SamplerBindingLayout { Type = s.Type == GpuSamplerType.Comparison ? SamplerBindingType.Comparison : SamplerBindingType.Filtering };
                        break;
                    default:
                        break;
                }
            }
            var desc = new BindGroupLayoutDescriptor { Label = arena.Text(label), EntryCount = (nuint)entries.Count, Entries = list };
            return new GpuBindGroupLayout((nint)Api.DeviceCreateBindGroupLayout((Device*)device, &desc));
        }
    }

    public GpuBindGroup CreateBindGroup(string label, GpuBindGroupLayout layout, IReadOnlyList<GpuBinding> entries)
    {
        // SAFETY: the device, the layout and every bound object are live; the descriptor and its
        // entries live in the arena for the call.
        unsafe
        {
            Live();
            using var arena = new Arena();
            var list = arena.Alloc<BindGroupEntry>(entries.Count);
            for (var i = 0; i < entries.Count; i++)
            {
                list[i].Binding = entries[i].Binding;
                switch (entries[i])
                {
                    case GpuBufferBinding b:
                        list[i].Buffer = (Silk.NET.WebGPU.Buffer*)b.Buffer.Handle;
                        list[i].Offset = b.Offset;
                        list[i].Size = b.Size ?? b.Buffer.Size - b.Offset;
                        break;
                    case GpuViewBinding v:
                        list[i].TextureView = (TextureView*)v.View.Handle;
                        break;
                    case GpuSamplerBinding s:
                        list[i].Sampler = (Sampler*)s.Sampler.Handle;
                        break;
                    default:
                        break;
                }
            }
            var desc = new BindGroupDescriptor { Label = arena.Text(label), Layout = (BindGroupLayout*)layout.Handle, EntryCount = (nuint)entries.Count, Entries = list };
            return new GpuBindGroup((nint)Api.DeviceCreateBindGroup((Device*)device, &desc));
        }
    }

    public GpuPipelineLayout CreatePipelineLayout(string label, IReadOnlyList<GpuBindGroupLayout> groups)
    {
        // SAFETY: the device and the group layouts are live; the descriptor and the list live in the
        // arena for the call.
        unsafe
        {
            Live();
            using var arena = new Arena();
            var list = arena.Alloc<nint>(groups.Count);
            for (var i = 0; i < groups.Count; i++)
            {
                list[i] = groups[i].Handle;
            }
            var desc = new PipelineLayoutDescriptor { Label = arena.Text(label), BindGroupLayoutCount = (nuint)groups.Count, BindGroupLayouts = (BindGroupLayout**)list };
            return new GpuPipelineLayout((nint)Api.DeviceCreatePipelineLayout((Device*)device, &desc));
        }
    }

    /// <summary>A shader module of WGSL source; one the device refuses reports to the error scope.</summary>
    public GpuShader CreateShader(string label, string wgsl)
    {
        // SAFETY: the device is live; the descriptors and the source live in the arena for the call.
        unsafe
        {
            Live();
            using var arena = new Arena();
            var source = arena.Alloc<ShaderModuleWGSLDescriptor>();
            source->Chain = new ChainedStruct { SType = SType.ShaderModuleWgslDescriptor };
            source->Code = arena.Text(wgsl);
            var desc = new ShaderModuleDescriptor { Label = arena.Text(label), NextInChain = (ChainedStruct*)source };
            return new GpuShader((nint)Api.DeviceCreateShaderModule((Device*)device, &desc));
        }
    }

    public GpuPipeline CreatePipeline(GpuPipelineDesc d)
    {
        // SAFETY: the device, the layout and the shader are live; every descriptor, list and string
        // lives in the arena for the call.
        unsafe
        {
            Live();
            using var arena = new Arena();
            var buffers = arena.Alloc<VertexBufferLayout>(d.Buffers.Count);
            for (var i = 0; i < d.Buffers.Count; i++)
            {
                var b = d.Buffers[i];
                var attributes = arena.Alloc<VertexAttribute>(b.Attributes.Count);
                for (var a = 0; a < b.Attributes.Count; a++)
                {
                    attributes[a] = new VertexAttribute { Format = Conv.Vertex(b.Attributes[a].Format), Offset = b.Attributes[a].Offset, ShaderLocation = b.Attributes[a].Location };
                }
                buffers[i] = new VertexBufferLayout { ArrayStride = b.Stride, StepMode = b.PerInstance ? VertexStepMode.Instance : VertexStepMode.Vertex, AttributeCount = (nuint)b.Attributes.Count, Attributes = attributes };
            }
            var desc = arena.Alloc<RenderPipelineDescriptor>();
            desc->Label = arena.Text(d.Label);
            desc->Layout = (PipelineLayout*)d.Layout.Handle;
            desc->Vertex = new VertexState { Module = (ShaderModule*)d.Shader.Handle, EntryPoint = arena.Text(d.VertexEntry), BufferCount = (nuint)d.Buffers.Count, Buffers = buffers };
            desc->Primitive = new PrimitiveState
            {
                Topology = PrimitiveTopology.TriangleList,
                StripIndexFormat = IndexFormat.Undefined,
                FrontFace = FrontFace.Ccw,
                CullMode = d.Cull == GpuCull.Front ? CullMode.Front : CullMode.None,
            };
            if (d.Depth is { } depth)
            {
                var keep = new StencilFaceState { Compare = CompareFunction.Always, FailOp = StencilOperation.Keep, DepthFailOp = StencilOperation.Keep, PassOp = StencilOperation.Keep };
                desc->DepthStencil = arena.One(new DepthStencilState
                {
                    Format = Conv.Format(depth.Format),
                    DepthWriteEnabled = depth.Write,
                    DepthCompare = Conv.Compare(depth.Compare),
                    StencilFront = keep,
                    StencilBack = keep,
                    DepthBias = depth.Bias,
                    DepthBiasSlopeScale = depth.SlopeScale,
                    DepthBiasClamp = 0,
                });
            }
            desc->Multisample = new MultisampleState { Count = Math.Max(d.Samples, 1), Mask = uint.MaxValue, AlphaToCoverageEnabled = false };
            if (d.FragmentEntry is { } entry && d.Target is { } target)
            {
                var color = arena.Alloc<ColorTargetState>();
                color->Format = Conv.Format(target.Format);
                color->Blend = Conv.Blend(target.Blend) is { } blend ? arena.One(blend) : null;
                color->WriteMask = ColorWriteMask.All;
                desc->Fragment = arena.One(new FragmentState { Module = (ShaderModule*)d.Shader.Handle, EntryPoint = arena.Text(entry), TargetCount = 1, Targets = color });
            }
            return new GpuPipeline((nint)Api.DeviceCreateRenderPipeline((Device*)device, desc));
        }
    }

    public GpuEncoder CreateEncoder(string label)
    {
        // SAFETY: the device is live; the descriptor lives in the arena for the call.
        unsafe
        {
            Live();
            using var arena = new Arena();
            var desc = new CommandEncoderDescriptor { Label = arena.Text(label) };
            return new GpuEncoder((nint)Api.DeviceCreateCommandEncoder((Device*)device, &desc));
        }
    }

    /// <summary>Sends the recorded commands to the GPU, and lets them go.</summary>
    public void Submit(GpuCommands commands)
    {
        // SAFETY: the device and the command buffer are live; the one-element list is on the stack for the call.
        unsafe
        {
            Live();
            var buffer = (CommandBuffer*)commands.Handle;
            Api.QueueSubmit((Queue*)queue, 1, &buffer);
        }
        commands.Dispose();
    }

    /// <summary>Waits for the GPU to finish what was sent, or until <paramref name="done"/> says so.</summary>
    private void Wait(Func<bool> done)
    {
        // SAFETY: `poll` is wgpu-native's wgpuDevicePoll(device, wait, submission index), found by name
        // in the same library; the device is live.
        unsafe
        {
            if (poll != 0)
            {
                ((delegate* unmanaged[Cdecl]<Device*, uint, void*, uint>)poll)((Device*)device, 1, null);
            }
            for (var tries = 0; !done() && tries < 10_000; tries++)
            {
                Api.InstanceProcessEvents((Instance*)instance);
                Thread.Sleep(1);
            }
        }
    }

    /// <summary>Waits for the GPU to finish what was sent.</summary>
    public void WaitIdle()
    {
        Live();
        Wait(() => true);
    }

    /// <summary>Reads a buffer made with <see cref="GpuBufferUsage.MapRead"/> once the GPU has written it; why not, or null.</summary>
    public string? ReadBuffer(GpuBuffer buffer, Span<byte> into)
    {
        // SAFETY: the device and the buffer are live; the answer's handle stays allocated until the
        // map has been answered or given up on, and the mapped range holds the buffer's bytes until
        // it is unmapped.
        unsafe
        {
            Live();
            var answer = new Answer();
            var handle = GCHandle.Alloc(answer);
            try
            {
                Api.BufferMapAsync((Silk.NET.WebGPU.Buffer*)buffer.Handle, MapMode.Read, 0, (nuint)buffer.Size, new PfnBufferMapCallback(&OnMapped), (void*)GCHandle.ToIntPtr(handle));
                Wait(() => answer.Done);
                if (!answer.Done)
                {
                    return "the GPU did not finish reading back the picture";
                }
                if (!answer.Ok)
                {
                    return $"reading back the picture failed: {answer.Message}";
                }
                var mapped = Api.BufferGetConstMappedRange((Silk.NET.WebGPU.Buffer*)buffer.Handle, 0, (nuint)buffer.Size);
                if (mapped == null)
                {
                    Api.BufferUnmap((Silk.NET.WebGPU.Buffer*)buffer.Handle);
                    return "the picture read back could not be reached";
                }
                new ReadOnlySpan<byte>(mapped, (int)Math.Min((ulong)into.Length, buffer.Size)).CopyTo(into);
                Api.BufferUnmap((Silk.NET.WebGPU.Buffer*)buffer.Handle);
                return null;
            }
            finally
            {
                if (answer.Done)
                {
                    handle.Free();
                }
            }
        }
    }

    public void Dispose()
    {
        // SAFETY: each handle is released once, the queue and device before the adapter and the instance.
        unsafe
        {
            if (queue != 0)
            {
                Api.QueueRelease((Queue*)queue);
                queue = 0;
            }
            if (device != 0)
            {
                // Objects not yet released may keep the device alive: their errors no longer reach this one.
                Api.DeviceSetUncapturedErrorCallback((Device*)device, new PfnErrorCallback(&OnUncaptured), null);
                Api.DeviceRelease((Device*)device);
                device = 0;
            }
            if (adapter != 0)
            {
                Api.AdapterRelease((Silk.NET.WebGPU.Adapter*)adapter);
                adapter = 0;
            }
            if (instance != 0)
            {
                Api.InstanceRelease((Instance*)instance);
                instance = 0;
            }
        }
        if (self.IsAllocated)
        {
            self.Free();
        }
    }
}
