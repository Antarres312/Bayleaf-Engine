using Bayleaf.Core;

namespace Bayleaf.Assets;

/// <summary>Loaded images and models, read again when their file or its import settings change.</summary>
public sealed class AssetCache
{
    private readonly Dictionary<string, ((DateTime?, DateTime?) Seen, Result<Image, string> Result)> images = new(StringComparer.Ordinal);
    private readonly Dictionary<string, ((DateTime?, DateTime?) Seen, Result<MeshData, string> Result)> models = new(StringComparer.Ordinal);

    public Result<Image, string> Image(string path) => Cached(images, path, Loader.LoadImage);

    public Result<MeshData, string> Model(string path) => Cached(models, path, Loader.LoadModel);

    private static Result<T, string> Cached<T>(Dictionary<string, ((DateTime?, DateTime?) Seen, Result<T, string> Result)> map, string path, Func<string, Result<T, string>> load)
    {
        // The file and its import settings: either changing loads it again.
        var modified = (Files.Modified(path), Loader.SettingsModified(path));
        if (map.TryGetValue(path, out var entry) && entry.Seen == modified && modified.Item1 is not null)
        {
            return entry.Result;
        }
        var result = load(path);
        map[path] = (modified, result);
        return result;
    }
}
