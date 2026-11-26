using Bayleaf.Core;

namespace Bayleaf.Engine;

/// <summary>Whether a path is read or written.</summary>
public enum Access
{
    Read,
    Write,
}

/// <summary>
/// Confined file access for automation (AGENTS.md rule 18). An agent entry point names the folders it
/// may read and the folders it may write; every command that touches a file asks the session's
/// confinement, which refuses anything outside, after resolving <c>..</c> and symbolic links. A write
/// root may also be read.
/// </summary>
public sealed class Confinement
{
    private readonly List<string> read;
    private readonly List<string> write;

    private Confinement(List<string> read, List<string> write)
    {
        this.read = read;
        this.write = write;
    }

    /// <summary>Both lists must name at least one existing folder.</summary>
    public static Result<Confinement, string> Create(IReadOnlyList<string> read, IReadOnlyList<string> write)
    {
        if (read.Count == 0 || write.Count == 0)
        {
            return Result.Fail("automation needs at least one --automation-read-root and one --automation-write-root");
        }
        var roots = new List<List<string>>();
        foreach (var list in new[] { read, write })
        {
            var resolved = new List<string>();
            foreach (var root in list)
            {
                string full;
                try
                {
                    full = Canonical(Path.GetFullPath(root));
                }
                catch (Exception e) when (e is IOException or UnauthorizedAccessException or ArgumentException or NotSupportedException)
                {
                    return Result.Fail($"automation root {root}: {e.Message}");
                }
                if (File.Exists(full))
                {
                    return Result.Fail($"automation root {root} is not a folder");
                }
                if (!Directory.Exists(full))
                {
                    return Result.Fail($"automation root {root}: the system cannot find the path specified");
                }
                resolved.Add(full);
            }
            roots.Add(resolved);
        }
        return new Confinement(roots[0], roots[1]);
    }

    /// <summary>Relative paths resolve against the first write root.</summary>
    public string Base => write.Count > 0 ? write[0] : ".";

    public IReadOnlyList<string> ReadRoots => read;

    public IReadOnlyList<string> WriteRoots => write;

    /// <summary>The absolute, resolved path if the access is allowed; otherwise a message that names the path, the access and the roots.</summary>
    public Result<string, string> Check(string path, Access access)
    {
        var resolved = Resolve(path, Base);
        var roots = access == Access.Read ? read.Concat(write).ToList() : write;
        if (roots.Any(root => Inside(resolved, root)))
        {
            return resolved;
        }
        var word = access == Access.Read ? "read" : "write";
        return Result.Fail($"automation may not {word} {resolved}: it is outside the allowed folders ({string.Join(", ", roots)})");
    }

    private static bool Inside(string path, string root)
    {
        var comparison = OperatingSystem.IsWindows() ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal;
        var trimmed = root.TrimEnd(Path.DirectorySeparatorChar);
        return string.Equals(path, trimmed, comparison) || path.StartsWith(trimmed + Path.DirectorySeparatorChar, comparison);
    }

    /// <summary>
    /// Makes <paramref name="path"/> absolute against <paramref name="baseDir"/>, removes <c>.</c> and
    /// <c>..</c> without touching the disk, then resolves symbolic links in the longest part that exists.
    /// </summary>
    private static string Resolve(string path, string baseDir)
    {
        var joined = Path.IsPathRooted(path) ? path : Path.Join(baseDir, path);
        var (root, names) = Files.Parts(joined);
        // A ".." at the top stays at the root.
        var lexical = root + string.Join(Path.DirectorySeparatorChar, names.Where(n => n != ".."));
        var existing = lexical;
        var rest = new List<string>();
        while (!File.Exists(existing) && !Directory.Exists(existing))
        {
            var parent = Path.GetDirectoryName(existing);
            var name = Path.GetFileName(existing);
            if (parent is null || name.Length == 0)
            {
                return lexical;
            }
            rest.Add(name);
            existing = parent;
        }
        var resolved = Canonical(existing);
        foreach (var name in Enumerable.Reverse(rest))
        {
            resolved = Path.Join(resolved, name);
        }
        return resolved;
    }

    /// <summary>The path with every symbolic link on it followed.</summary>
    private static string Canonical(string path)
    {
        var (root, names) = Files.Parts(Path.GetFullPath(path));
        var current = root;
        foreach (var name in names)
        {
            current = Path.Join(current, name);
            FileSystemInfo info = Directory.Exists(current) ? new DirectoryInfo(current) : new FileInfo(current);
            if (info.Exists && info.LinkTarget is not null && info.ResolveLinkTarget(true) is { } target)
            {
                current = target.FullName;
            }
        }
        return current;
    }
}
