using System.Globalization;

using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Engine;

namespace Bayleaf.Cli;

/// <summary>The command line: <c>commands</c>, <c>run --cmd ID [--params JSON] [--scene PATH]</c> and <c>batch STEPS [--scene PATH]</c>.</summary>
internal static class Cli
{
    private const string Usage = """
        Headless Bayleaf Engine

        Usage: bayleaf-cli <COMMAND>

        Commands:
          commands  Print every registered command as a JSON array
          run       Run one command and print its JSON result
          batch     Run a JSON array of steps [{"cmd": "...", "params": {...}}] in one session
          mcp       Run an MCP server on stdio for AI agents. It refuses to start without both automation roots, and refuses file access outside them (AA 18)
        """;

    public static int Run(string[] args, TextWriter output, TextWriter errors) => Run(args, TextReader.Null, output, errors);

    public static int Run(string[] args, TextReader input, TextWriter output, TextWriter errors)
    {
        if (args.Length == 0 || args[0] is "-h" or "--help" or "help")
        {
            output.WriteLine(Usage);
            return args.Length == 0 ? 2 : 0;
        }
        if (args[0] is "-V" or "--version")
        {
            output.WriteLine($"bayleaf-cli {CommandRegistry.EngineVersion}");
            return 0;
        }
        if (args[0] == "mcp")
        {
            return Mcp.Run(args.Skip(1).ToList(), input, output, errors);
        }
        var registry = CommandRegistry.WithBuiltins();
        var session = new Session();
        var options = Options(args.Skip(1).ToList(), out var positional, out var problem);
        if (problem is not null)
        {
            errors.WriteLine($"error: {problem}");
            return 2;
        }
        switch (args[0])
        {
            case "commands":
                output.WriteLine(registry.List(session).ToPretty());
                return 0;
            case "run":
                if (!options.TryGetValue("cmd", out var cmd))
                {
                    errors.WriteLine("error: the following required arguments were not provided:\n  --cmd <CMD>");
                    return 2;
                }
                return Fail(errors, () =>
                {
                    OpenScene(registry, session, options);
                    var parameters = JsonReader.Parse(options.GetValueOrDefault("params", "{}"));
                    if (parameters.IsError)
                    {
                        return Result.Fail($"--params is not valid JSON\n\nCaused by:\n    {parameters.Error}");
                    }
                    var result = registry.Execute(session, cmd, parameters.Value);
                    if (result.IsError)
                    {
                        return Result.Fail(result.Error.Message);
                    }
                    output.WriteLine(result.Value.ToPretty());
                    return Unit.Value;
                });
            case "batch":
                if (positional.Count != 1)
                {
                    errors.WriteLine("error: the following required arguments were not provided:\n  <STEPS>");
                    return 2;
                }
                return Fail(errors, () =>
                {
                    OpenScene(registry, session, options);
                    var steps = Steps(positional[0]);
                    if (steps.IsError)
                    {
                        return steps.AsFailure();
                    }
                    var (text, failure) = Batch(registry, session, steps.Value);
                    output.WriteLine(text);
                    return failure is null ? Unit.Value : Result.Fail(failure);
                });
            default:
                errors.WriteLine($"error: unrecognized subcommand '{args[0]}'");
                return 2;
        }
    }

    /// <summary>Prints an error as the Rust CLI does (<c>Error: …</c>) and gives the exit code.</summary>
    private static int Fail(TextWriter errors, Func<Result<Unit, string>> run)
    {
        Result<Unit, string> result;
        try
        {
            result = run();
        }
        catch (CommandException e)
        {
            result = Result.Fail(e.Error.Message);
        }
        if (result.IsOk)
        {
            return 0;
        }
        errors.WriteLine($"Error: {result.Error}");
        return 1;
    }

    private static Dictionary<string, string> Options(List<string> args, out List<string> positional, out string? problem)
    {
        var options = new Dictionary<string, string>(StringComparer.Ordinal);
        positional = [];
        problem = null;
        for (var i = 0; i < args.Count; i++)
        {
            var arg = args[i];
            if (!arg.StartsWith("--", StringComparison.Ordinal))
            {
                positional.Add(arg);
                continue;
            }
            var name = arg[2..];
            string value;
            var equals = name.IndexOf('=', StringComparison.Ordinal);
            if (equals >= 0)
            {
                (name, value) = (name[..equals], name[(equals + 1)..]);
            }
            else if (i + 1 < args.Count)
            {
                value = args[++i];
            }
            else
            {
                problem = $"a value is required for '--{name}' but none was supplied";
                return options;
            }
            if (name is not ("cmd" or "params" or "scene"))
            {
                problem = $"unexpected argument '--{name}' found";
                return options;
            }
            options[name] = value;
        }
        return options;
    }

    private static void OpenScene(CommandRegistry registry, Session session, Dictionary<string, string> options)
    {
        if (options.TryGetValue("scene", out var scene))
        {
            var opened = registry.Execute(session, "scene.open", new JsonObject { { "path", new JsonString(scene) } });
            if (opened.IsError)
            {
                throw new CommandException(opened.Error);
            }
        }
    }

    internal sealed record Step(string Cmd, JsonValue Params);

    internal static Result<List<Step>, string> Steps(string path)
    {
        string text;
        try
        {
            text = File.ReadAllText(path);
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or ArgumentException or NotSupportedException)
        {
            return Result.Fail($"cannot read {path}\n\nCaused by:\n    {e.Message}");
        }
        var parsed = JsonReader.Parse(text);
        var steps = parsed.IsOk
            ? JsonRead.Try(() => JsonRead.List(parsed.Value, s =>
            {
                var f = JsonFields.Of(s, "struct Step", true, "cmd", "params");
                return new Step(f.Required("cmd", JsonRead.Str), f.Optional("params", v => v, JsonValue.Null));
            }))
            : Result.Fail(parsed.Error);
        return steps.IsOk ? steps.Value : Result.Fail($"the steps file must be a JSON array of {{cmd, params}}\n\nCaused by:\n    {steps.Error}");
    }

    /// <summary>Runs the steps in one session; the text printed, and the failure when a step fails.</summary>
    internal static (string Text, string? Failure) Batch(CommandRegistry registry, Session session, IReadOnlyList<Step> steps)
    {
        var (results, failure) = RunSteps(registry, session, steps);
        if (failure is var (index, error))
        {
            var report = new JsonObject
            {
                { "error", new JsonString(error) },
                { "failedStep", JsonNumber.Of((long)index) },
                { "results", new JsonArray(results) },
            };
            return (report.ToPretty(), string.Create(CultureInfo.InvariantCulture, $"step {index} ({steps[index].Cmd}) failed: {error}"));
        }
        return (new JsonObject { { "results", new JsonArray(results) } }.ToPretty(), null);
    }

    /// <summary>Runs the steps in one session until one fails; the results so far, and the failed step with its error.</summary>
    internal static (List<JsonValue> Results, (int Index, string Error)? Failure) RunSteps(CommandRegistry registry, Session session, IReadOnlyList<Step> steps)
    {
        var results = new List<JsonValue>();
        for (var index = 0; index < steps.Count; index++)
        {
            var result = registry.Execute(session, steps[index].Cmd, Substitute(steps[index].Params, results));
            if (result.IsError)
            {
                return (results, (index, result.Error.Message));
            }
            results.Add(result.Value);
        }
        return (results, null);
    }

    /// <summary>Replaces string params of the form <c>$N.field</c> with that field of step N's result.</summary>
    internal static JsonValue Substitute(JsonValue value, IReadOnlyList<JsonValue> results) => value switch
    {
        JsonString s => Reference(s.Value, results) ?? s,
        JsonArray a => new JsonArray(a.Select(v => Substitute(v, results))),
        JsonObject o => new JsonObject(o.Entries.Select(e => new KeyValuePair<string, JsonValue>(e.Key, Substitute(e.Value, results)))),
        _ => value,
    };

    private static JsonValue? Reference(string text, IReadOnlyList<JsonValue> results)
    {
        var dot = text.IndexOf('.', StringComparison.Ordinal);
        if (!text.StartsWith('$') || dot < 0)
        {
            return null;
        }
        var step = text[1..dot];
        return ulong.TryParse(step, NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture, out var n) && !step.StartsWith('-') && n < (ulong)results.Count
            ? results[(int)n].Get(text[(dot + 1)..])
            : null;
    }
}
