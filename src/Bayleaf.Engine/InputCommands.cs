using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Engine;

/// <summary>How an action reports itself.</summary>
public enum ActionKind
{
    /// <summary>1 in the one step it goes down, else 0: a jump, a shot.</summary>
    Trigger,

    /// <summary>1 from the step it goes down for as long as it is held, else 0.</summary>
    Held,

    /// <summary>How far it is pressed, 0 to 1: walking, steering.</summary>
    Value,
}

/// <summary>One action of a map: its name, its kind, and what presses it (<c>key:W</c>, <c>mouse:Left</c>, <c>pad:South</c>, <c>axis:LeftX+</c>).</summary>
public sealed record InputAction(string Name, ActionKind Kind, IReadOnlyList<string> Bindings)
{
    private static readonly string[] Kinds = ["trigger", "held", "value"];

    internal static InputAction Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct InputAction", true, "name", "kind", "bindings");
        return new(f.Required("name", JsonRead.Str), (ActionKind)f.Optional("kind", v => JsonRead.Variant(v, Kinds), (int)ActionKind.Held), f.Required("bindings", v => JsonRead.List(v, JsonRead.Str)));
    }

    internal JsonObject Write() => new() { { "name", new JsonString(Name) }, { "kind", new JsonString(Kinds[(int)Kind]) }, { "bindings", J.Strings(Bindings) } };
}

/// <summary>One map: its name and its actions.</summary>
public sealed record InputMapDef(string Name, IReadOnlyList<InputAction> Actions)
{
    internal static InputMapDef Read(JsonValue value)
    {
        var f = JsonFields.Of(value, "struct InputMapDef", true, "name", "actions");
        return new(f.Required("name", JsonRead.Str), f.Required("actions", v => JsonRead.List(v, InputAction.Read)));
    }

    internal JsonObject Write() => new() { { "name", new JsonString(Name) }, { "actions", new JsonArray(Actions.Select(a => (JsonValue)a.Write())) } };
}

/// <summary>
/// An Input Map asset (<c>.binput</c>): a game's input, as a list of maps (Player, Menu…), each holding
/// its actions. One map is active at a time. Scripts name an action <c>map/action</c>, or just
/// <c>action</c> in the active map.
/// </summary>
public sealed record InputAsset(string Start, IReadOnlyList<InputMapDef> Maps)
{
    public const string Extension = "binput";
    public const string Format = "bayleaf.input";

    /// <summary>Version 2 lists maps, each with its actions; version 1 listed actions named <c>map/action</c>.</summary>
    public const uint Version = 2;

    /// <summary>The largest Input Map file read.</summary>
    public const long MaxFileBytes = 1 << 20;

    public const int MaxActions = 128;
    public const int MaxBindings = 16;
    public const int MaxMaps = 32;

    public static readonly IReadOnlyList<string> PadAxes = ["LeftX", "LeftY", "RightX", "RightY", "LeftTrigger", "RightTrigger"];

    public static readonly IReadOnlyList<string> PadButtons =
        ["South", "East", "West", "North", "LeftBumper", "RightBumper", "LeftTrigger", "RightTrigger", "Select", "Start", "LeftStick", "RightStick", "Up", "Down", "Left", "Right"];

    /// <summary>A game's usual controls: Player (walking, jumping, firing) and Menu (moving and choosing).</summary>
    public static InputAsset Starter() => new("Player",
    [
        new("Player",
        [
            new("MoveForward", ActionKind.Value, ["key:W", "key:ArrowUp", "axis:LeftY+"]),
            new("MoveBack", ActionKind.Value, ["key:S", "key:ArrowDown", "axis:LeftY-"]),
            new("MoveLeft", ActionKind.Value, ["key:A", "key:ArrowLeft", "axis:LeftX-"]),
            new("MoveRight", ActionKind.Value, ["key:D", "key:ArrowRight", "axis:LeftX+"]),
            new("Jump", ActionKind.Trigger, ["key:Space", "pad:South"]),
            new("Fire", ActionKind.Held, ["mouse:Left", "axis:RightTrigger+"]),
        ]),
        new("Menu",
        [
            new("Up", ActionKind.Trigger, ["key:ArrowUp", "key:W", "pad:Up"]),
            new("Down", ActionKind.Trigger, ["key:ArrowDown", "key:S", "pad:Down"]),
            new("Confirm", ActionKind.Trigger, ["key:Enter", "key:Space", "pad:South"]),
            new("Back", ActionKind.Trigger, ["key:Escape", "pad:East"]),
        ]),
    ]);

    private static bool IsName(string name) => name.Length > 0 && name.Length <= 64 && name.All(c => char.IsAsciiLetterOrDigit(c) || c == '_');

    private static string? CheckBinding(string text)
    {
        var usage = $"{Text.Debug(text)}: write key:NAME, mouse:Left, pad:BUTTON or axis:AXIS+ / axis:AXIS-";
        var colon = text.IndexOf(':', StringComparison.Ordinal);
        if (colon < 0)
        {
            return usage;
        }
        var (kind, name) = (text[..colon], text[(colon + 1)..]);
        switch (kind)
        {
            case "key" when name.Length > 0 && Text.Bytes(name) <= 32:
                return null;
            case "mouse":
                return name is "Left" or "Right" or "Middle" ? null : $"{Text.Debug(text)}: the mouse buttons are Left, Right and Middle";
            case "pad":
                return PadButtons.Contains(name) ? null : $"{Text.Debug(text)}: gamepad buttons are {string.Join(", ", PadButtons)}";
            case "axis":
                if (!name.EndsWith('+') && !name.EndsWith('-'))
                {
                    return $"{Text.Debug(text)}: end an axis with + or -";
                }
                return PadAxes.Contains(name[..^1]) ? null : $"{Text.Debug(text)}: gamepad axes are {string.Join(", ", PadAxes)}";
            default:
                return usage;
        }
    }

    /// <summary>The maps' names in order, after checking names, bindings and limits; the error says what is wrong.</summary>
    public Result<IReadOnlyList<string>, string> CheckedMaps()
    {
        if (Maps.Count > MaxMaps)
        {
            return Result.Fail($"an Input Map may have at most {MaxMaps} maps");
        }
        if (Maps.Sum(m => m.Actions.Count) > MaxActions)
        {
            return Result.Fail($"an Input Map may have at most {MaxActions} actions");
        }
        var names = new List<string>();
        foreach (var map in Maps)
        {
            if (!IsName(map.Name))
            {
                return Result.Fail($"map name {Text.Debug(map.Name)} must be 1 to 64 letters, digits or _");
            }
            if (names.Contains(map.Name))
            {
                return Result.Fail($"map {map.Name} is named twice");
            }
            names.Add(map.Name);
            var seen = new HashSet<string>(StringComparer.Ordinal);
            foreach (var action in map.Actions)
            {
                if (!IsName(action.Name))
                {
                    return Result.Fail($"action name {Text.Debug(action.Name)} in {map.Name} must be 1 to 64 letters, digits or _");
                }
                if (!seen.Add(action.Name))
                {
                    return Result.Fail($"action {map.Name}/{action.Name} is named twice");
                }
                if (action.Bindings.Count > MaxBindings)
                {
                    return Result.Fail($"action {map.Name}/{action.Name} may have at most {MaxBindings} bindings");
                }
                foreach (var binding in action.Bindings)
                {
                    if (CheckBinding(binding) is { } problem)
                    {
                        return Result.Fail($"action {map.Name}/{action.Name}: {problem}");
                    }
                }
            }
        }
        var start = Start.Length == 0 ? names.FirstOrDefault() ?? "" : Start;
        if (names.Count > 0 && !names.Contains(start))
        {
            return Result.Fail($"there is no map {start} to start with; the maps are {string.Join(", ", names)}");
        }
        return names;
    }

    /// <summary>Reads the file text: version 2, or version 1 with its <c>map/action</c> names sorted into maps.</summary>
    public static Result<InputAsset, string> FromText(string text)
    {
        var parsed = JsonReader.Parse(text);
        if (parsed.IsError)
        {
            return Result.Fail(parsed.Error);
        }
        var value = parsed.Value;
        if (value.Get("format") is not JsonString { Value: Format })
        {
            return Result.Fail("not an Input Map");
        }
        var version = value.Get("version") is JsonNumber n ? n.AsUInt64() : null;
        return version switch
        {
            2 => JsonRead.Try(() =>
            {
                var f = JsonFields.Of(value, "struct InputAsset", true, "format", "version", "start", "maps");
                f.Required("format", JsonRead.Str);
                f.Required("version", JsonRead.U32);
                return new InputAsset(f.Optional("start", JsonRead.Str, ""), f.Required("maps", v => JsonRead.List(v, InputMapDef.Read)));
            }),
            1 => ReadFlat(value),
            _ => Result.Fail($"not a version {Version} Input Map"),
        };
    }

    private static Result<InputAsset, string> ReadFlat(JsonValue value)
    {
        var body = new JsonObject(((JsonObject)value).Entries.Where(e => e.Key is not ("format" or "version")));
        var flat = JsonRead.Try(() =>
        {
            var f = JsonFields.Of(body, "struct FlatAsset", true, "start", "actions");
            return (Start: f.Optional("start", JsonRead.Str, ""), Actions: f.Required("actions", v => JsonRead.List(v, InputAction.Read)));
        });
        if (flat.IsError)
        {
            return flat.AsFailure();
        }
        var maps = new List<(string Name, List<InputAction> Actions)>();
        foreach (var action in flat.Value.Actions)
        {
            var slash = action.Name.IndexOf('/', StringComparison.Ordinal);
            if (slash < 0)
            {
                return Result.Fail($"{Text.Debug(action.Name)} is not named map/action");
            }
            var (map, name) = (action.Name[..slash], action.Name[(slash + 1)..]);
            var entry = action with { Name = name };
            var found = maps.FindIndex(m => m.Name == map);
            if (found >= 0)
            {
                maps[found].Actions.Add(entry);
            }
            else
            {
                maps.Add((map, [entry]));
            }
        }
        return new InputAsset(flat.Value.Start, maps.Select(m => new InputMapDef(m.Name, m.Actions)).ToList());
    }

    public JsonObject ToJson() => new()
    {
        { "format", new JsonString(Format) },
        { "version", JsonNumber.Of((ulong)Version) },
        { "start", new JsonString(Start) },
        { "maps", new JsonArray(Maps.Select(m => (JsonValue)m.Write())) },
    };

    public string ToText() => ToJson().ToPretty() + "\n";
}

/// <summary>Input Map commands. Switching and reading the active map come with play mode.</summary>
internal static class InputCommands
{
    public static IEnumerable<CommandSpec> All() =>
    [
        new()
        {
            Id = "input.asset", Label = "Read Input Map",
            Params = "{path}: a .binput file; returns {asset: {start, maps: [{name, actions: [{name, kind: \"trigger\" | \"held\" | \"value\", bindings: [key:W | mouse:Left | pad:South | axis:LeftX+]}]}]}, maps, problem}; scripts name an action map/action",
            Run = Asset,
        },
        new()
        {
            Id = "input.asset.set", Label = "Change Input Map",
            Params = "{path, asset: {start, maps: [{name, actions: [{name, kind, bindings}]}]}}: checks the maps and actions (names of letters, digits and _, unique; known bindings) and writes the file; play mode reads it from its next start",
            Run = AssetSet,
        },
    ];

    /// <summary>Reads an Input Map asset; the path is relative to the project folder or absolute.</summary>
    internal static Result<InputAsset, string> ReadAsset(Session session, string path)
    {
        string full;
        try
        {
            full = session.Allowed(Path.Combine(session.ScriptDir, path), Access.Read);
        }
        catch (CommandException e)
        {
            return Result.Fail(e.Error.Message);
        }
        var size = Files.Length(full);
        if (size.IsError)
        {
            return Result.Fail($"{full}: {size.Error}");
        }
        if (size.Value > InputAsset.MaxFileBytes)
        {
            return Result.Fail($"{full} is larger than {InputAsset.MaxFileBytes / 1024} KiB");
        }
        var text = Files.ReadText(full);
        if (text.IsError)
        {
            return Result.Fail($"{full}: {text.Error}");
        }
        var asset = InputAsset.FromText(text.Value);
        return asset.IsOk ? asset : Result.Fail($"{full}: {asset.Error}");
    }

    /// <summary>Writes an Input Map asset after checking it.</summary>
    internal static string WriteAsset(Session session, string path, InputAsset asset)
    {
        if (!string.Equals(Path.GetExtension(path).TrimStart('.'), InputAsset.Extension, StringComparison.Ordinal))
        {
            throw Stop.BadParams($"Input Map files end in .{InputAsset.Extension}");
        }
        asset.CheckedMaps().OrBadParams();
        var full = session.Allowed(Path.Combine(session.ScriptDir, path), Access.Write);
        if (Path.GetDirectoryName(full) is { Length: > 0 } folder)
        {
            Files.CreateDirectory(folder).MapError(e => $"cannot make {folder}: {e}").OrFail();
        }
        Files.Write(full, System.Text.Encoding.UTF8.GetBytes(asset.ToText())).MapError(e => $"cannot write {full}: {e}").OrFail();
        return full;
    }

    private static JsonObject Asset(Session session, JsonValue p)
    {
        var path = Params.Of(p, "path").Required("path", JsonRead.Str);
        var asset = ReadAsset(session, path).OrFail();
        var maps = asset.CheckedMaps();
        var body = J.Obj(("start", J.S(asset.Start)), ("maps", new JsonArray(asset.Maps.Select(m => (JsonValue)m.Write()))));
        return J.Obj(("asset", body), ("maps", maps.IsOk ? J.Strings(maps.Value) : new JsonArray()), ("problem", maps.IsOk ? JsonValue.Null : J.S(maps.Error)));
    }

    private static JsonObject AssetSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "asset");
        var path = f.Required("path", JsonRead.Str);
        var asset = f.Required("asset", v =>
        {
            var a = JsonFields.Of(v, "struct AssetBody", true, "start", "maps");
            return new InputAsset(a.Optional("start", JsonRead.Str, ""), a.Required("maps", m => JsonRead.List(m, InputMapDef.Read)));
        });
        return J.Obj(("path", J.S(WriteAsset(session, path, asset))));
    }
}
