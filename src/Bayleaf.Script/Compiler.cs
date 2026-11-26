using System.Collections.Immutable;
using System.Reflection;
using System.Runtime.Loader;
using System.Text;

using Bayleaf.Core;
using Bayleaf.Script.Sdk;

using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.Text;

namespace Bayleaf.Script;

/// <summary>One problem the compiler found in a script file: where it is and what it is.</summary>
public sealed record ScriptProblem(string File, int Line, int Column, string Message)
{
    public override string ToString() => Line > 0 ? $"{File}({Line},{Column}): {Message}" : $"{File}: {Message}";
}

/// <summary>
/// The C# scripts of one project folder, compiled together: every <c>.cs</c> file under the folder
/// (build, hidden and <c>bin</c>/<c>obj</c> folders left out) is one assembly against
/// <c>Bayleaf.Script.Sdk</c>, so scripts can share their own helper types. Compiling runs no script
/// code: what each script declares is read from the compiler's view of the source
/// (<see cref="Describe"/>). Only <see cref="Load"/> brings the code into the process, into a
/// collectible load context that a reload throws away.
/// </summary>
public sealed class ScriptProject
{
    /// <summary>The extension of a C# script file.</summary>
    public const string Extension = "cs";

    /// <summary>Most script files one project compiles.</summary>
    public const int MaxFiles = 4096;

    /// <summary>The largest script file read.</summary>
    public const long MaxFileBytes = 4 << 20;

    /// <summary>Folders never compiled: build output, hidden folders and package caches.</summary>
    private static readonly string[] Skipped = ["bin", "obj", "target", "node_modules"];

    private static readonly Lazy<ImmutableArray<MetadataReference>> References = new(MakeReferences);

    private static int compiled;

    private ScriptProject(string folder, IReadOnlyList<(string Relative, long Length, DateTime Written)> stamps, CSharpCompilation? compilation, IReadOnlyList<ScriptProblem> problems, byte[]? image, IReadOnlyDictionary<string, ScriptDescription> scripts)
    {
        Folder = folder;
        Stamps = stamps;
        Compilation = compilation;
        Problems = problems;
        Image = image;
        Scripts = scripts;
    }

    /// <summary>The project folder the script paths start from.</summary>
    public string Folder { get; }

    /// <summary>Each file compiled, relative to the folder with <c>/</c>, with its length and time, to see when they change.</summary>
    public IReadOnlyList<(string Relative, long Length, DateTime Written)> Stamps { get; }

    internal CSharpCompilation? Compilation { get; }

    /// <summary>What stops the scripts compiling, or what is wrong with what they declare; empty when they compiled.</summary>
    public IReadOnlyList<ScriptProblem> Problems { get; }

    /// <summary>Whether the scripts compiled into an assembly (they may still each have problems of their own).</summary>
    public bool Compiled => Image is not null;

    private byte[]? Image { get; }

    /// <summary>Each script file's script, by its path relative to the folder (with <c>/</c>).</summary>
    public IReadOnlyDictionary<string, ScriptDescription> Scripts { get; }

    /// <summary>The first problems, as one line each, for the console.</summary>
    public string Summary(int most = 5) => string.Join("; ", Problems.Take(most).Select(p => p.ToString())) + (Problems.Count > most ? $"; and {Problems.Count - most} more" : "");

    /// <summary>The script file <paramref name="module"/> (a path relative to the folder) declares, or why it has none.</summary>
    public Result<ScriptDescription, string> Describe(string module)
    {
        var key = Key(module);
        if (Scripts.TryGetValue(key, out var found))
        {
            return found;
        }
        if (!Stamps.Any(s => s.Relative == key))
        {
            return Result.Fail($"{module} is not a C# script of the project (a .cs file in {Folder})");
        }
        var own = Problems.Where(p => p.File == key).ToList();
        return own.Count > 0 ? Result.Fail(string.Join("; ", own.Take(5).Select(p => p.ToString()))) : Result.Fail(Compiled
            ? $"{module} declares no script: give it a public class that derives from GameScript"
            : $"the project's scripts do not compile: {Summary()}");
    }

    /// <summary>A module path as the project keeps it: relative to the folder, <c>/</c> between folders, no leading <c>./</c>.</summary>
    public static string Key(string module)
    {
        var path = module.Replace('\\', '/');
        while (path.StartsWith("./", StringComparison.Ordinal))
        {
            path = path[2..];
        }
        return path;
    }

    /// <summary>Whether a module path names a C# script file.</summary>
    public static bool IsScript(string module) => module.EndsWith("." + Extension, StringComparison.OrdinalIgnoreCase);

    /// <summary>The script files under <paramref name="folder"/>, relative with <c>/</c>, in order, with their lengths and times.</summary>
    public static IReadOnlyList<(string Relative, long Length, DateTime Written)> Scan(string folder)
    {
        var found = new List<(string, long, DateTime)>();
        var pending = new Stack<string>();
        pending.Push(folder);
        while (pending.TryPop(out var dir) && found.Count <= MaxFiles)
        {
            IEnumerable<string> entries;
            try
            {
                entries = Directory.EnumerateFileSystemEntries(dir).ToList();
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                continue;
            }
            foreach (var entry in entries)
            {
                var name = Path.GetFileName(entry);
                if (name.StartsWith('.'))
                {
                    continue;
                }
                if (Directory.Exists(entry))
                {
                    if (!Skipped.Contains(name, StringComparer.OrdinalIgnoreCase) && !File.GetAttributes(entry).HasFlag(FileAttributes.ReparsePoint))
                    {
                        pending.Push(entry);
                    }
                    continue;
                }
                if (!IsScript(name))
                {
                    continue;
                }
                try
                {
                    var info = new FileInfo(entry);
                    found.Add((Path.GetRelativePath(folder, entry).Replace('\\', '/'), info.Length, info.LastWriteTimeUtc));
                }
                catch (Exception e) when (e is IOException or UnauthorizedAccessException)
                {
                }
            }
        }
        found.Sort((a, b) => string.CompareOrdinal(a.Item1, b.Item1));
        return found;
    }

    /// <summary>Whether the script files changed since this was compiled: one came, went, or was written.</summary>
    public bool Changed() => !Scan(Folder).SequenceEqual(Stamps);

    /// <summary>Compiles every script file under <paramref name="folder"/>. Runs no script code.</summary>
    public static ScriptProject Compile(string folder)
    {
        var stamps = Scan(folder);
        var problems = new List<ScriptProblem>();
        var unread = false;
        if (stamps.Count > MaxFiles)
        {
            problems.Add(new ScriptProblem(".", 0, 0, $"a project compiles at most {MaxFiles} script files"));
            return new ScriptProject(folder, stamps.Take(MaxFiles).ToList(), null, problems, null, new Dictionary<string, ScriptDescription>());
        }
        var parse = new CSharpParseOptions(LanguageVersion.CSharp14);
        var trees = new List<SyntaxTree> { CSharpSyntaxTree.ParseText(GlobalUsings, parse, path: "<global usings>") };
        foreach (var (relative, length, _) in stamps)
        {
            if (length > MaxFileBytes)
            {
                problems.Add(new ScriptProblem(relative, 0, 0, $"is larger than {MaxFileBytes >> 20} MiB"));
                unread = true;
                continue;
            }
            var text = Files.ReadText(Path.Join(folder, relative));
            if (text.IsError)
            {
                problems.Add(new ScriptProblem(relative, 0, 0, text.Error));
                unread = true;
                continue;
            }
            trees.Add(CSharpSyntaxTree.ParseText(SourceText.From(text.Value, Encoding.UTF8), parse, path: relative));
        }
        var name = $"BayleafScripts{Interlocked.Increment(ref compiled)}";
        var options = new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary, optimizationLevel: OptimizationLevel.Release, nullableContextOptions: NullableContextOptions.Enable, allowUnsafe: false, concurrentBuild: false, deterministic: true);
        var compilation = CSharpCompilation.Create(name, trees, References.Value, options);
        var descriptions = Describer.All(compilation, problems);
        using var stream = new MemoryStream();
        // A file that could not be read leaves the scripts uncompiled rather than compiled without it.
        var emitted = unread ? null : compilation.Emit(stream);
        foreach (var diagnostic in emitted?.Diagnostics.Where(d => d.Severity == DiagnosticSeverity.Error) ?? [])
        {
            var span = diagnostic.Location.GetLineSpan();
            problems.Add(new ScriptProblem(span.Path is { Length: > 0 } p ? p : ".", span.StartLinePosition.Line + 1, span.StartLinePosition.Character + 1, $"{diagnostic.Id}: {diagnostic.GetMessage(System.Globalization.CultureInfo.InvariantCulture)}"));
        }
        var image = emitted is { Success: true } ? stream.ToArray() : null;
        return new ScriptProject(folder, stamps, compilation, problems, image, descriptions);
    }

    /// <summary>
    /// Brings the compiled scripts into the process, in a collectible load context of their own: their
    /// types, by script file. Running them is the host's (it constructs them); loading runs no script code.
    /// </summary>
    public Result<LoadedScripts, string> Load()
    {
        if (Image is null)
        {
            return Result.Fail($"the project's scripts do not compile: {Summary()}");
        }
        var context = new ScriptLoadContext();
        Assembly assembly;
        try
        {
            using var stream = new MemoryStream(Image, writable: false);
            assembly = context.LoadFromStream(stream);
        }
        catch (Exception e) when (e is BadImageFormatException or FileLoadException)
        {
            context.Unload();
            return Result.Fail($"the compiled scripts do not load: {e.Message}");
        }
        var types = new Dictionary<string, ScriptType>(StringComparer.Ordinal);
        foreach (var (file, description) in Scripts)
        {
            if (description.Problems.Count == 0 && assembly.GetType(description.TypeName, throwOnError: false) is { } type)
            {
                types[file] = ScriptType.Of(type, description);
            }
        }
        return new LoadedScripts(this, context, types);
    }

    /// <summary>What every script file starts with, so scripts need not say it.</summary>
    private const string GlobalUsings = """
        global using System;
        global using System.Collections.Generic;
        global using System.Linq;
        global using System.Numerics;
        global using Bayleaf.Script.Sdk;
        """;

    /// <summary>What scripts compile against: .NET's own assemblies and the script SDK; nothing of the engine.</summary>
    private static ImmutableArray<MetadataReference> MakeReferences()
    {
        var paths = new List<string>();
        if (AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES") is string platform)
        {
            paths.AddRange(platform.Split(Path.PathSeparator).Where(p =>
            {
                var name = Path.GetFileNameWithoutExtension(p);
                return name is "mscorlib" or "netstandard" || name.StartsWith("System.", StringComparison.Ordinal) || name == "System";
            }));
        }
        paths.Add(typeof(GameScript).Assembly.Location);
        return [.. paths.Where(p => p.Length > 0 && File.Exists(p)).Distinct(StringComparer.OrdinalIgnoreCase).Select(p => (MetadataReference)MetadataReference.CreateFromFile(p))];
    }
}

/// <summary>
/// A load context of its own for one compile of a project's scripts. The SDK and .NET come from the
/// engine's own context, so a script's <see cref="GameScript"/> is the host's; unloading it lets the
/// garbage collector take the scripts' code once nothing uses it.
/// </summary>
internal sealed class ScriptLoadContext() : AssemblyLoadContext("Bayleaf scripts", isCollectible: true)
{
    protected override Assembly? Load(AssemblyName assemblyName) => null;
}

/// <summary>A project's scripts brought into the process: each script file's type, in a load context that <see cref="Unload"/> throws away.</summary>
public sealed class LoadedScripts
{
    private readonly ScriptLoadContext context;
    private bool unloaded;

    internal LoadedScripts(ScriptProject project, ScriptLoadContext context, IReadOnlyDictionary<string, ScriptType> types)
    {
        Project = project;
        this.context = context;
        Types = types;
    }

    public ScriptProject Project { get; }

    /// <summary>Each script file's script type, by its path relative to the project folder.</summary>
    internal IReadOnlyDictionary<string, ScriptType> Types { get; }

    /// <summary>Lets the scripts' code go, once no host runs it any more.</summary>
    public void Unload()
    {
        if (!unloaded)
        {
            unloaded = true;
            context.Unload();
        }
    }
}
