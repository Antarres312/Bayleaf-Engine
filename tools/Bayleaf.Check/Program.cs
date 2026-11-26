// The C# tree's repository checks (docs/csharp-redesign.md, "Layout").
//
//   layers    every project is in its layer and refers only to lower ones (a test project to the project
//             it tests and lower), is in Bayleaf.slnx, keeps the shared code rules on, and no source
//             file silences a warning.
//   licences  every package the projects restore (after `dotnet restore Bayleaf.slnx`) declares a
//             licence on the Craft list, as `cargo xtask licences` checks the crates.

using System.Text.Json;
using System.Text.RegularExpressions;
using System.Xml.Linq;

var root = FindRoot();
var command = args.Length > 0 ? args[0] : "";
var problems = command switch
{
    "layers" => Layers.Check(root),
    "licences" => Licences.Check(root),
    _ => ["usage: Bayleaf.Check layers|licences"],
};
foreach (var problem in problems)
{
    Console.Error.WriteLine(problem);
}
if (problems.Count == 0)
{
    Console.WriteLine($"{command}: ok");
}
return problems.Count == 0 ? 0 : 1;

static string FindRoot()
{
    var dir = new DirectoryInfo(Directory.GetCurrentDirectory());
    while (dir is not null && !File.Exists(Path.Join(dir.FullName, "Bayleaf.slnx")))
    {
        dir = dir.Parent;
    }
    return dir?.FullName ?? Directory.GetCurrentDirectory();
}

internal static partial class Layers
{
    /// <summary>The layer of every project of src/ and apps/; a project refers only to projects of lower layers.</summary>
    private static readonly Dictionary<string, int> Layer = new(StringComparer.Ordinal)
    {
        ["Bayleaf.Core"] = 0,
        ["Bayleaf.Native"] = 0,
        ["Bayleaf.Ecs"] = 1,
        ["Bayleaf.Scene"] = 2,
        ["Bayleaf.Assets"] = 2,
        ["Bayleaf.Graph"] = 2,
        ["Bayleaf.Render"] = 3,
        ["Bayleaf.Animation"] = 3,
        ["Bayleaf.Script"] = 4,
        ["Bayleaf.Gameplay"] = 4,
        ["Bayleaf.Streaming"] = 4,
        ["Bayleaf.Physics"] = 4,
        ["Bayleaf.Audio"] = 4,
        ["Bayleaf.Particles"] = 4,
        ["Bayleaf.Interface"] = 2,
        ["Bayleaf.Engine"] = 5,
        ["Bayleaf.Editor"] = 6,
        ["Bayleaf.Automation"] = 6,
        ["Bayleaf"] = 7,
        ["Bayleaf.Cli"] = 7,
        ["Bayleaf.Player"] = 7,
    };

    /// <summary>The folders that hold the C# tree's projects (the Rust tree, in rust/, has its own checks).</summary>
    private static readonly string[] Areas = ["src", "tests", "tools", "apps"];

    /// <summary>The one project whose code may be unsafe: the interop with native libraries.</summary>
    private const string Native = "Bayleaf.Native";

    /// <summary>Settings no project may set, as they turn the shared code rules off.</summary>
    private static readonly string[] Forbidden = ["NoWarn", "WarningsNotAsErrors", "WarningLevel", "RunAnalyzers", "RunAnalyzersDuringBuild", "EnableNETAnalyzers"];

    [GeneratedRegex(@"#pragma\s+warning\s+disable|SuppressMessage\s*\(|#nullable\s+disable")]
    private static partial Regex Silencing();

    public static List<string> Check(string root)
    {
        var problems = new List<string>();
        var solution = XDocument.Load(Path.Join(root, "Bayleaf.slnx"));
        var listed = solution.Descendants("Project").Select(p => Path.GetFullPath(Path.Join(root, (string?)p.Attribute("Path") ?? ""))).ToHashSet(StringComparer.OrdinalIgnoreCase);
        var projects = Areas.Where(a => Directory.Exists(Path.Join(root, a))).SelectMany(a => Directory.EnumerateFiles(Path.Join(root, a), "*.csproj", SearchOption.AllDirectories)).Order(StringComparer.Ordinal).ToList();
        foreach (var path in projects)
        {
            var name = Path.GetFileNameWithoutExtension(path);
            var relative = Path.GetRelativePath(root, path).Replace('\\', '/');
            var area = relative.Split('/')[0];
            if (!listed.Contains(Path.GetFullPath(path)))
            {
                problems.Add($"{relative} is not in Bayleaf.slnx");
            }
            var subject = area == "tests" && name.EndsWith(".Tests", StringComparison.Ordinal) ? name[..^".Tests".Length] : name;
            int? layer = Layer.TryGetValue(subject, out var l) ? l : null;
            if (area is "src" or "apps" or "tests" && layer is null)
            {
                problems.Add($"{relative}: {subject} has no layer; add it to the table in tools/Bayleaf.Check/Program.cs");
            }
            var project = XDocument.Load(path);
            foreach (var reference in project.Descendants("ProjectReference"))
            {
                var target = Path.GetFileNameWithoutExtension((string?)reference.Attribute("Include") ?? "");
                if (!Layer.TryGetValue(target, out var targetLayer))
                {
                    problems.Add($"{relative} refers to {target}, which has no layer");
                    continue;
                }
                var allowed = area == "tests" ? targetLayer <= layer : targetLayer < layer;
                if (area != "tools" && !allowed)
                {
                    problems.Add($"{relative} (L{layer}) refers to {target} (L{targetLayer}), which is not below it");
                }
            }
            foreach (var setting in project.Descendants().Where(e => Forbidden.Contains(e.Name.LocalName)))
            {
                problems.Add($"{relative} sets {setting.Name.LocalName}, which turns the shared code rules off");
            }
            CheckValue(project, "TreatWarningsAsErrors", "true", relative, problems);
            CheckValue(project, "Nullable", "enable", relative, problems);
            if (project.Descendants("AllowUnsafeBlocks").Any(e => e.Value.Trim() == "true") && subject != Native)
            {
                problems.Add($"{relative} allows unsafe code; only {Native} may");
            }
            if (project.Descendants("AnalysisLevel").Any())
            {
                problems.Add($"{relative} sets its own AnalysisLevel; the shared one is in Directory.Build.props");
            }
            foreach (var source in Directory.EnumerateFiles(Path.GetDirectoryName(path)!, "*.cs", SearchOption.AllDirectories))
            {
                var text = File.ReadAllText(source);
                if (Silencing().IsMatch(text))
                {
                    problems.Add($"{Path.GetRelativePath(root, source)} silences a warning or nullable checks; fix the code instead (AGENTS.md rule 13)");
                }
                if (text.Contains("unsafe", StringComparison.Ordinal) && subject != Native && UnsafeKeyword().IsMatch(text))
                {
                    problems.Add($"{Path.GetRelativePath(root, source)} has unsafe code; only {Native} may");
                }
            }
        }
        return problems;
    }

    [GeneratedRegex(@"\bunsafe\s+(\{|static|void|class|struct|fixed|\w+\s*\()")]
    private static partial Regex UnsafeKeyword();

    private static void CheckValue(XDocument project, string setting, string required, string relative, List<string> problems)
    {
        foreach (var element in project.Descendants(setting).Where(e => !string.Equals(e.Value.Trim(), required, StringComparison.OrdinalIgnoreCase)))
        {
            problems.Add($"{relative} sets {setting} to {element.Value.Trim()}; it must stay {required}");
        }
    }
}

internal static class Licences
{
    /// <summary>The licences allowed by craft-contrib standards/licences.md (the list of <c>xtask/src/licences.rs</c>).</summary>
    private static readonly HashSet<string> Allowed = new(StringComparer.Ordinal)
    {
        "ISC", "MIT", "BSD-2-Clause", "BSD-3-Clause", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "Unicode-3.0", "Zlib", "BSL-1.0", "0BSD", "Unlicense",
        "IJG", "NCSA", "CDLA-Permissive-2.0", "CC0-1.0", "CC-BY-4.0", "CC-BY-SA-4.0", "OFL-1.1", "Ubuntu-font-1.0", "Bitstream-Vera", "LicenseRef-Public-Domain", "MPL-2.0",
    };

    /// <summary>
    /// Packages that name their licence only in a file, read by hand: package and version, and the
    /// licence the file holds (each is recorded in ATTRIBUTION.md too). Another version is read again.
    /// </summary>
    private static readonly Dictionary<string, string> CheckedByHand = new(StringComparer.Ordinal)
    {
        // Through Silk.NET.Core; its LICENSE.TXT is the MIT licence of the .NET Foundation.
        ["Microsoft.DotNet.PlatformAbstractions/3.1.6"] = "MIT",
    };

    /// <summary>The identifiers in a licence expression; it is allowed when every one is.</summary>
    private static IEnumerable<string> Identifiers(string expression) =>
        expression.Replace('(', ' ').Replace(')', ' ').Split(" OR ").SelectMany(p => p.Split(" AND ")).Select(p => string.Join(' ', p.Split(' ', StringSplitOptions.RemoveEmptyEntries)))
            .Where(p => p.Length > 0);

    public static List<string> Check(string root)
    {
        var problems = new List<string>();
        var assets = Directory.Exists(Path.Join(root, "target", "dotnet", "obj"))
            ? Directory.EnumerateFiles(Path.Join(root, "target", "dotnet", "obj"), "project.assets.json", SearchOption.AllDirectories).ToList()
            : [];
        if (assets.Count == 0)
        {
            return ["no restored projects: run `dotnet restore Bayleaf.slnx` first"];
        }
        var seen = new SortedDictionary<string, string>(StringComparer.Ordinal);
        foreach (var file in assets)
        {
            using var json = JsonDocument.Parse(File.ReadAllText(file));
            var folders = json.RootElement.GetProperty("packageFolders").EnumerateObject().Select(f => f.Name).ToList();
            foreach (var library in json.RootElement.GetProperty("libraries").EnumerateObject())
            {
                if (library.Value.GetProperty("type").GetString() != "package" || seen.ContainsKey(library.Name))
                {
                    continue;
                }
                var path = library.Value.GetProperty("path").GetString() ?? "";
                var nuspec = folders.Select(f => Path.Join(f, path)).Where(Directory.Exists).SelectMany(d => Directory.EnumerateFiles(d, "*.nuspec")).FirstOrDefault();
                var licence = nuspec is null ? null : XDocument.Load(nuspec).Descendants().FirstOrDefault(e => e.Name.LocalName == "license" && (string?)e.Attribute("type") == "expression")?.Value;
                licence ??= CheckedByHand.GetValueOrDefault(library.Name);
                seen[library.Name] = licence ?? "";
                if (licence is null)
                {
                    problems.Add($"{library.Name} declares no licence expression; check it by hand and record it");
                }
                else if (!Identifiers(licence).All(Allowed.Contains))
                {
                    problems.Add($"{library.Name} is under {licence}, which is not on the Craft list (AGENTS.md rule 26)");
                }
            }
        }
        foreach (var (package, licence) in seen)
        {
            Console.WriteLine($"{package}: {licence}");
        }
        return problems;
    }
}
