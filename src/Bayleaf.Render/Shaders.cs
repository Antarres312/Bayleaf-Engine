using System.Reflection;

namespace Bayleaf.Render;

/// <summary>
/// The renderer's WGSL. There is one copy of each shader, the Rust renderer's
/// (<c>rust/crates/render/src/shaders</c>), embedded in this assembly when it builds.
/// </summary>
internal static class Shaders
{
    private static string Read(string name)
    {
        using var stream = Assembly.GetExecutingAssembly().GetManifestResourceStream($"Bayleaf.Render.Shaders.{name}");
        if (stream is null)
        {
            return "";
        }
        using var reader = new StreamReader(stream);
        return reader.ReadToEnd().Replace("\r\n", "\n", StringComparison.Ordinal);
    }

    private static readonly Lazy<string> Common = new(() => Read("common.wgsl"));

    public static string Shadow => WithCommon(Read("shadow.wgsl"));

    public static string Grid => WithCommon(Read("grid.wgsl"));

    public static string Sky => WithCommon(Read("sky.wgsl"));

    public static string Outline => WithCommon(Read("outline.wgsl"));

    public static string Post => WithCommon(Read("post.wgsl"));

    private static readonly Lazy<string> MeshTemplate = new(() => Read("mesh.wgsl"));

    /// <summary>The built-in surface: the mesh component's colour, metallic and roughness.</summary>
    public const string DefaultSurface = """
        fn surface(s: SurfaceIn) -> Surface {
            let image = textureSample(t0, material_sampler, s.uv);
            // glTF images: roughness in green and metallic in blue (t1), a tangent-space normal map (t2).
            // Both hold plain numbers, not colours, and are bound to be read as stored.
            let metal_rough = textureSample(t1, material_sampler, s.uv).rgb;
            let mapped = textureSample(t2, material_sampler, s.uv).rgb * 2.0 - vec3<f32>(1.0);
            let normal = bl_normal_map(s.normal, s.position, s.uv, mapped);
            return Surface(s.color.rgb * image.rgb, s.params.x * metal_rough.b, s.params.y * metal_rough.g, vec3<f32>(0.0), s.color.a * image.a, normal);
        }
        const USES_LIGHTS: bool = false;
        const CUSTOM_LIGHTING: bool = false;
        const OUTLINE: vec4<f32> = vec4<f32>(0.0);
        """;

    /// <summary>The shader with the shared per-frame values in front.</summary>
    private static string WithCommon(string shader) => $"{Common.Value}\n{shader}";

    /// <summary>The mesh shader with <paramref name="surface"/> filled in.</summary>
    public static string Mesh(string surface) => WithCommon(MeshTemplate.Value.Replace("// @surface", surface, StringComparison.Ordinal));
}
