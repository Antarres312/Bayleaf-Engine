using System.Text;

using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Script;

namespace Bayleaf.Engine;

/// <summary>
/// C# scripts (docs/script-sdk.md): <c>script.new</c> writes one from a template, <c>script.build</c>
/// compiles the project's scripts and says what is wrong, and <c>project.scripts.allow</c> is the
/// person's consent to run a project's scripts in play mode, which agents cannot give.
/// </summary>
internal static class ScriptCommands
{
    /// <summary>Why agents may not allow scripts: a C# script runs in the editor's process with nothing around it.</summary>
    private const string AllowRefused = "only the person at the editor may allow a project's scripts to run: they run in the editor's process with no sandbox";

    public static IEnumerable<CommandSpec> All() =>
    [
        new()
        {
            Id = "script.new", Label = "New Script",
            Params = "{folder, name}: makes a C# script `folder/<name>.cs` from a template (a class deriving from GameScript); returns {path, module}",
            Run = New,
        },
        new()
        {
            Id = "script.build", Label = "Build Script",
            Params = "{path}: a C# script of the project (or any path in it); compiles the project's scripts, running none of them; returns {module, log}, or fails with the compiler's problems",
            Run = Build,
        },
        new()
        {
            Id = "project.scripts.allow", Label = "Allow Scripts", Menu = "Project/Allow Scripts",
            Params = "{allow?: true}: lets the open project's C# scripts run in play mode (they run in the editor's process with no sandbox, so the editor asks when a project is first opened); allow: false takes it back. Agents cannot give it; returns {allowed, folder}",
            Enabled = s => s.Confinement is not null ? AllowRefused : null, Run = Allow,
        },
    ];

    /// <summary>A file name from a script name: lower case words joined by <c>_</c> (<c>Door Opener</c> → <c>door_opener</c>); null when nothing is left.</summary>
    public static string? FileName(string name)
    {
        var spaced = new string([.. name.Select(c => char.IsAsciiLetterOrDigit(c) ? char.ToLowerInvariant(c) : ' ')]);
        var words = spaced.Split(' ', StringSplitOptions.RemoveEmptyEntries);
        var joined = string.Join('_', words);
        return joined.Length > 0 && joined.Length <= 64 && !char.IsAsciiDigit(joined[0]) ? joined : null;
    }

    /// <summary>The class name for a file name: each word with a capital (<c>door_opener</c> → <c>DoorOpener</c>).</summary>
    private static string TypeName(string file) => string.Concat(file.Split('_').Where(w => w.Length > 0).Select(w => char.ToUpperInvariant(w[0]) + w[1..]));

    private static string Template(string title, string type) => $$"""
        // {{title}}: a Bayleaf Engine script in C#.
        //
        // Add it to an object in the Inspector (Add Component > Scripts). The engine compiles the project's
        // scripts when play starts and again when a file changes; docs/script-sdk.md describes what a script
        // can do.

        public class {{type}} : GameScript
        {
            /// <summary>Turns per second about Y. The Inspector shows it for each object; the scene sets it.</summary>
            [ScriptProperty(Hint = "Turns per second")]
            private float speed = 1f;

            protected override void Begin()
            {
            }

            protected override void Advance(float seconds)
            {
                // An example to replace: turn around Y by speed turns per second.
                var pose = Pose;
                pose.Rotation.Y = (pose.Rotation.Y + speed * 360f * seconds) % 360f;
                Pose = pose;
            }
        }

        """;

    /// <summary>Writes a new C# script into <paramref name="folder"/> (already allowed); its path.</summary>
    internal static string Write(string folder, string name)
    {
        var file = FileName(name) ?? throw Stop.BadParams("a script name needs letters or digits, may not start with a digit, and has at most 64 characters");
        var path = Path.Join(folder, $"{file}.{ScriptProject.Extension}");
        if (AssetCommands.Exists(path))
        {
            throw Stop.Failed($"{path} already exists; pick another name");
        }
        Files.CreateDirectory(folder).MapError(e => $"cannot make {folder}: {e}").OrFail();
        var text = Template(name.Trim(), TypeName(file)).ReplaceLineEndings("\n");
        Files.Write(path, Encoding.UTF8.GetBytes(text)).MapError(e => $"cannot write {path}: {e}").OrFail();
        return path;
    }

    private static JsonObject New(Session session, JsonValue p)
    {
        var f = Params.Of(p, "folder", "name");
        var folder = session.Allowed(Path.Combine(session.ScriptDir, f.Required("folder", JsonRead.Str)), Access.Write);
        var path = Write(folder, f.Required("name", JsonRead.Str));
        return J.Obj(("path", J.S(path)), ("module", J.S(path)));
    }

    private static JsonObject Build(Session session, JsonValue p)
    {
        var given = Params.Of(p, "path").Required("path", JsonRead.Str);
        var path = session.Allowed(Path.Combine(session.ScriptDir, given), Access.Read);
        if (!File.Exists(path) && !Directory.Exists(path))
        {
            throw Stop.Failed($"{path} does not exist");
        }
        foreach (var (relative, _, _) in ScriptProject.Scan(session.ScriptDir))
        {
            session.Allowed(Path.Combine(session.ScriptDir, relative), Access.Read);
        }
        var project = session.CompiledScripts();
        if (project.Problems.Count > 0)
        {
            throw Stop.Failed($"the scripts did not compile:\n{string.Join("\n", project.Problems.Take(40))}");
        }
        var count = project.Scripts.Count;
        return J.Obj(("module", J.S(path)), ("log", J.S($"{project.Stamps.Count} script files compiled; {count} scripts declared")));
    }

    private static JsonObject Allow(Session session, JsonValue p)
    {
        var allow = Params.Of(p, "allow").Optional("allow", JsonRead.Bool, true);
        session.AllowScripts(allow);
        session.Log(Level.Info, allow ? $"The scripts of {session.ScriptDir} may run in play mode." : $"The scripts of {session.ScriptDir} no longer run in play mode.");
        return J.Obj(("allowed", J.B(session.ScriptsAllowed)), ("folder", J.S(session.ScriptDir)));
    }
}
