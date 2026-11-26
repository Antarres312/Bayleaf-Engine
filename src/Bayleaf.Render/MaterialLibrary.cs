using Bayleaf.Core;
using Bayleaf.Graph;
using Bayleaf.Scene;

namespace Bayleaf.Render;

/// <summary>
/// Material files, compiled once and again when the file changes; and the lighting and
/// post-processing files views use, read once and again when they change.
/// </summary>
public sealed class MaterialLibrary
{
    private readonly Dictionary<string, (DateTime? Modified, Result<CompiledMaterial, string> Result)> entries = new(StringComparer.Ordinal);

    /// <summary>
    /// Graphs being edited, used instead of their files so changes show before they are saved: by
    /// file, the graph's text (an unchanged graph is not compiled again) and what it compiled to.
    /// </summary>
    private readonly Dictionary<string, (string Graph, Result<CompiledMaterial, string> Result)> live = new(StringComparer.Ordinal);

    private readonly Dictionary<string, (DateTime? Modified, Result<Lighting, string> Result)> lightings = new(StringComparer.Ordinal);

    private readonly Dictionary<string, (DateTime? Modified, Result<PostProcessing, string> Result)> posts = new(StringComparer.Ordinal);

    /// <summary>A lighting file (<c>.blight</c>), read again when it changes on disk.</summary>
    public Result<Lighting, string> Lighting(string path) => Cached(lightings, path, key => Bayleaf.Scene.Lighting.Load(key).MapError(e => e.ToString()));

    /// <summary>A post-processing file (<c>.bpost</c>), read again when it changes on disk.</summary>
    public Result<PostProcessing, string> Post(string path) => Cached(posts, path, key => PostProcessing.Load(key).MapError(e => e.ToString()));

    private static Result<T, string> Cached<T>(Dictionary<string, (DateTime? Modified, Result<T, string> Result)> cache, string path, Func<string, Result<T, string>> load)
    {
        var key = Normalize(path);
        var modified = Files.Modified(key);
        if (cache.TryGetValue(key, out var seen) && seen.Modified == modified && modified is not null)
        {
            return seen.Result;
        }
        var result = load(key);
        cache[key] = (modified, result);
        return result;
    }

    /// <summary>Uses <paramref name="graph"/> for <paramref name="path"/> instead of the file, or stops with null.</summary>
    public void SetLive(string? path, Graph.Graph? graph) => SetLives(path is not null && graph is not null ? [(path, graph)] : []);

    /// <summary>Uses each graph for its file instead of the file; files not named here are read again.</summary>
    public void SetLives(IReadOnlyList<(string Path, Graph.Graph Graph)> lives)
    {
        var keys = lives.Select(l => Normalize(l.Path)).ToList();
        foreach (var gone in live.Keys.Where(k => !keys.Contains(k)).ToList())
        {
            live.Remove(gone);
        }
        for (var i = 0; i < lives.Count; i++)
        {
            var text = lives[i].Graph.ToJson();
            if (live.TryGetValue(keys[i], out var seen) && seen.Graph == text)
            {
                continue;
            }
            live[keys[i]] = (text, Material.Compile(lives[i].Graph).MapError(Problem.Summary));
        }
    }

    /// <summary>The compiled material at <paramref name="path"/>, reading and compiling the file if it is new or changed.</summary>
    public Result<CompiledMaterial, string> Get(string path)
    {
        var key = Normalize(path);
        if (live.TryGetValue(key, out var edited))
        {
            return edited.Result;
        }
        var modified = Files.Modified(path);
        if (entries.TryGetValue(key, out var entry) && entry.Modified == modified && modified is not null)
        {
            return entry.Result;
        }
        var result = Load(path);
        entries[key] = (modified, result);
        return result;
    }

    private static Result<CompiledMaterial, string> Load(string path)
    {
        if (!Files.ReadText(path).TryGet(out var text, out var error))
        {
            return Result.Fail($"cannot read {path}: {error}");
        }
        if (!Graph.Graph.FromJson(text, Material.Catalog).TryGet(out var graph, out var bad))
        {
            return Result.Fail($"{path}: {bad.Message}");
        }
        return Material.Compile(graph).MapError(p => $"{path}: {Problem.Summary(p)}");
    }

    /// <summary>One spelling per file, so a live graph matches the path a mesh uses for it.</summary>
    private static string Normalize(string path)
    {
        try
        {
            return Path.GetFullPath(path);
        }
        catch (Exception e) when (e is ArgumentException or NotSupportedException or PathTooLongException or System.Security.SecurityException)
        {
            return path;
        }
    }
}
