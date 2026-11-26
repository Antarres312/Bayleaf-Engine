using System.Reflection;

using Bayleaf.Core.Json;

namespace Bayleaf.Engine;

/// <summary>
/// The Rust engine's commands that the C# engine does not run yet. They are listed with the same id,
/// label, menu, shortcut, params and prompt, so front ends and agents see every command, but they are
/// disabled and say why (AGENTS.md rule 15). Each port stage that brings one removes it from
/// <c>Pending.json</c>, which is the Rust engine's <c>commands</c> list.
/// </summary>
internal static class PendingCommands
{
    private static readonly Lazy<IReadOnlyList<CommandSpec>> Listed = new(Load);

    /// <summary>Why the command <paramref name="id"/> cannot run in this engine; null when it can.</summary>
    public static string? Reason(string id) =>
        "this command is not in the C# engine yet; the Rust engine (rust/) runs it until its part is ported (docs/csharp-redesign.md, \"Stages\")";

    /// <summary>The pending commands, leaving out those <paramref name="ported"/> already holds.</summary>
    public static IEnumerable<CommandSpec> All(IReadOnlySet<string> ported) => Listed.Value.Where(c => !ported.Contains(c.Id));

    private static List<CommandSpec> Load()
    {
        using var stream = Assembly.GetExecutingAssembly().GetManifestResourceStream("Bayleaf.Engine.Pending.json");
        if (stream is null)
        {
            return [];
        }
        using var reader = new StreamReader(stream);
        var parsed = JsonReader.Parse(reader.ReadToEnd());
        if (parsed.IsError || parsed.Value is not JsonArray list)
        {
            return [];
        }
        var specs = new List<CommandSpec>();
        foreach (var row in list)
        {
            var id = row.Get("id")?.AsString();
            if (id is null)
            {
                continue;
            }
            specs.Add(new CommandSpec
            {
                Id = id,
                Label = row.Get("label")?.AsString() ?? id,
                Menu = row.Get("menu")?.AsString() ?? "",
                Shortcut = row.Get("shortcut")?.AsString() ?? "",
                Params = row.Get("params")?.AsString() ?? "",
                Prompt = PromptOf(row.Get("prompt")),
                Enabled = _ => Reason(id),
                Run = (_, _) => throw Stop.Failed(Reason(id)!),
            });
        }
        return specs;
    }

    private static Prompt PromptOf(JsonValue? value)
    {
        if (value?.Get("fields") is JsonArray fields)
        {
            return Prompt.Form([.. fields.Select(f => new PromptField(
                f.Get("name")?.AsString() ?? "", f.Get("label")?.AsString() ?? "", f.Get("default")?.AsString() ?? "", f.Get("number")?.AsBool() ?? false))]);
        }
        var name = value?.AsString() ?? "none";
        var kind = Enum.GetValues<PromptKind>().FirstOrDefault(k => string.Equals(k.ToString(), name, StringComparison.OrdinalIgnoreCase));
        return new Prompt(kind, []);
    }
}
