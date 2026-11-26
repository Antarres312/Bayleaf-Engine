namespace Bayleaf.Core;

/// <summary>
/// The game's files. Everything a running game reads or writes goes through here: the file system,
/// under a game's pack when the player mounted one (an exported game's files, read from memory).
/// Writes (saved values) go to the disk. Editor-only code (importing, exporting, building scripts)
/// may use <see cref="File"/> directly.
/// </summary>
public static class Files
{
    private static readonly Lock PackLock = new();
    private static readonly FileStore Pack = new();

    /// <summary>Puts a file of a game's pack in memory, where reads find it before the disk.</summary>
    public static void Mount(string path, byte[] bytes)
    {
        lock (PackLock)
        {
            Pack.Mount(path, bytes);
        }
    }

    private static T Packed<T>(Func<FileStore, T> read)
    {
        lock (PackLock)
        {
            return read(Pack);
        }
    }

    /// <summary>The whole file.</summary>
    public static Result<byte[], string> Read(string path)
    {
        var packed = Packed(p => p.Read(path));
        if (packed.IsOk)
        {
            return packed;
        }
        try
        {
            return File.ReadAllBytes(path);
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or ArgumentException or NotSupportedException)
        {
            return Result.Fail(e.Message);
        }
    }

    /// <summary>The whole file as UTF-8 text (a byte-order mark kept, invalid bytes refused).</summary>
    public static Result<string, string> ReadText(string path)
    {
        var bytes = Read(path);
        return bytes.IsOk ? Json.JsonReader.DecodeUtf8(bytes.Value) : bytes.AsFailure();
    }

    /// <summary>Writes the whole file, replacing what it held.</summary>
    public static Result<Unit, string> Write(string path, ReadOnlySpan<byte> bytes)
    {
        try
        {
            File.WriteAllBytes(path, bytes);
            return Unit.Value;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or ArgumentException or NotSupportedException)
        {
            return Result.Fail(e.Message);
        }
    }

    /// <summary>The file's size in bytes.</summary>
    public static Result<long, string> Length(string path)
    {
        var packed = Packed(p => p.Length(path));
        if (packed.IsOk)
        {
            return packed;
        }
        try
        {
            var info = new FileInfo(path);
            return info.Exists ? info.Length : Result.Fail($"cannot find {path}");
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or ArgumentException or NotSupportedException)
        {
            return Result.Fail(e.Message);
        }
    }

    /// <summary>When the file last changed; null for a packed file, which never changes while the game runs.</summary>
    public static DateTime? Modified(string path)
    {
        if (Packed(p => p.IsFile(path)) || !File.Exists(path))
        {
            return null;
        }
        return File.GetLastWriteTimeUtc(path);
    }

    public static bool IsFile(string path) => Packed(p => p.IsFile(path)) || File.Exists(path);

    public static bool IsDirectory(string path) => Packed(p => p.IsDirectory(path)) || Directory.Exists(path);

    /// <summary>The files and folders directly in a folder: the pack's, then the disk's that the pack lacks.</summary>
    public static Result<List<string>, string> ReadDirectory(string path)
    {
        var packed = Packed(p => p.ReadDirectory(path));
        List<string>? disk = null;
        try
        {
            disk = Directory.Exists(path) ? [.. Directory.EnumerateFileSystemEntries(path)] : null;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or ArgumentException or NotSupportedException)
        {
            disk = null;
        }
        if (packed.IsOk)
        {
            var listed = packed.Value;
            if (disk is not null)
            {
                listed.AddRange(disk.Where(d => !Packed(p => p.IsFile(d) || p.IsDirectory(d))));
            }
            return listed;
        }
        return disk is not null ? disk : Result.Fail($"cannot read the folder {path}");
    }

    public static Result<Unit, string> CreateDirectory(string path)
    {
        try
        {
            Directory.CreateDirectory(path);
            return Unit.Value;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or ArgumentException or NotSupportedException)
        {
            return Result.Fail(e.Message);
        }
    }

    public static Result<Unit, string> RemoveFile(string path)
    {
        try
        {
            if (!File.Exists(path))
            {
                return Result.Fail($"cannot find {path}");
            }
            File.Delete(path);
            return Unit.Value;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or ArgumentException or NotSupportedException)
        {
            return Result.Fail(e.Message);
        }
    }

    /// <summary>
    /// Writes <paramref name="text"/> through a temporary file at <paramref name="temporary"/>, so a
    /// failed write keeps the old file.
    /// </summary>
    public static Result<Unit, string> WriteReplacing(string path, string temporary, string text)
    {
        try
        {
            File.WriteAllBytes(temporary, System.Text.Encoding.UTF8.GetBytes(text));
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or ArgumentException or NotSupportedException)
        {
            return Result.Fail($"cannot write {temporary}: {e.Message}");
        }
        try
        {
            File.Move(temporary, path, true);
            return Unit.Value;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or ArgumentException or NotSupportedException)
        {
            return Result.Fail($"cannot replace {path}: {e.Message}");
        }
    }

    private static bool IsSeparator(char c) => c == '/' || c == Path.DirectorySeparatorChar || c == Path.AltDirectorySeparatorChar;

    /// <summary>The parts of a plain path: its root (empty for a relative path) and its names.</summary>
    public static (string Root, List<string> Names) Parts(string path)
    {
        var root = Path.GetPathRoot(path) ?? "";
        var names = new List<string>();
        char[] separators = Path.DirectorySeparatorChar == '/' ? ['/'] : ['/', Path.DirectorySeparatorChar];
        foreach (var part in path[root.Length..].Split(separators, StringSplitOptions.RemoveEmptyEntries))
        {
            switch (part)
            {
                case ".":
                    break;
                case "..":
                    if (names.Count > 0 && names[^1] != "..")
                    {
                        names.RemoveAt(names.Count - 1);
                    }
                    else if (root.Length == 0)
                    {
                        names.Add("..");
                    }
                    break;
                default:
                    names.Add(part);
                    break;
            }
        }
        root = string.Concat(root.Select(c => IsSeparator(c) ? Path.DirectorySeparatorChar : c));
        return (root, names);
    }

    /// <summary>
    /// The folder <paramref name="path"/> is in, keeping its text as written (as Rust's
    /// <c>Path::parent</c> does, where <see cref="Path.GetDirectoryName(string)"/> turns separators):
    /// "" for a bare name, null for a root or an empty path.
    /// </summary>
    public static string? Parent(string path)
    {
        // The root as written: GetPathRoot turns its separators.
        var root = path[..(Path.GetPathRoot(path) ?? "").Length];
        var rest = path[root.Length..].TrimEnd('/', Path.DirectorySeparatorChar);
        if (rest.Length == 0)
        {
            return null;
        }
        var at = rest.LastIndexOfAny(['/', Path.DirectorySeparatorChar]);
        return at < 0 ? root : root + rest[..at].TrimEnd('/', Path.DirectorySeparatorChar);
    }

    /// <summary><paramref name="path"/> with <c>.</c> and <c>..</c> worked out without asking the file system, so the same file has one name.</summary>
    public static string Plain(string path)
    {
        var (root, names) = Parts(path);
        return root + string.Join(Path.DirectorySeparatorChar, names);
    }
}

/// <summary>A store of files in memory: what a game's pack holds, and what the game wrote since.</summary>
internal sealed class FileStore
{
    private readonly SortedDictionary<string, byte[]> files = new(StringComparer.Ordinal);
    private readonly List<string> written = [];

    private static Result<T, string> Missing<T>(string path) => Result.Fail($"the game has no file {path}");

    public void Mount(string path, byte[] bytes) => files[Files.Plain(path)] = bytes;

    public Result<byte[], string> Read(string path) => files.TryGetValue(Files.Plain(path), out var bytes) ? bytes.ToArray() : Missing<byte[]>(path);

    /// <summary>Writes a file into the store and notes it for <see cref="TakeWritten"/>.</summary>
    public void Write(string path, byte[] bytes)
    {
        var plain = Files.Plain(path);
        files[plain] = bytes;
        if (!written.Contains(plain, StringComparer.Ordinal))
        {
            written.Add(plain);
        }
    }

    /// <summary>The files written since this was last asked.</summary>
    public List<string> TakeWritten()
    {
        var taken = written.ToList();
        written.Clear();
        return taken;
    }

    public Result<long, string> Length(string path) => files.TryGetValue(Files.Plain(path), out var bytes) ? bytes.LongLength : Missing<long>(path);

    public bool IsFile(string path) => files.ContainsKey(Files.Plain(path));

    /// <summary>A folder is there when a file is below it.</summary>
    public bool IsDirectory(string path)
    {
        var folder = Files.Parts(path);
        return files.Keys.Any(f => Below(Files.Parts(f), folder) is not null);
    }

    /// <summary>The first name below <paramref name="folder"/>, when <paramref name="file"/> is below it.</summary>
    private static string? Below((string Root, List<string> Names) file, (string Root, List<string> Names) folder)
    {
        if (!string.Equals(file.Root, folder.Root, StringComparison.Ordinal) || file.Names.Count <= folder.Names.Count)
        {
            return null;
        }
        for (var i = 0; i < folder.Names.Count; i++)
        {
            if (!string.Equals(file.Names[i], folder.Names[i], StringComparison.Ordinal))
            {
                return null;
            }
        }
        return file.Names[folder.Names.Count];
    }

    public Result<List<string>, string> ReadDirectory(string path)
    {
        var folder = Files.Parts(path);
        var plain = Files.Plain(path);
        var entries = new List<string>();
        foreach (var file in files.Keys)
        {
            var first = Below(Files.Parts(file), folder);
            if (first is not null)
            {
                var entry = Path.Join(plain, first);
                if (entries.Count == 0 || !string.Equals(entries[^1], entry, StringComparison.Ordinal))
                {
                    entries.Add(entry);
                }
            }
        }
        return entries.Count == 0 ? Missing<List<string>>(path) : entries;
    }

    public Result<Unit, string> RemoveFile(string path) => files.Remove(Files.Plain(path)) ? Unit.Value : Missing<Unit>(path);
}
