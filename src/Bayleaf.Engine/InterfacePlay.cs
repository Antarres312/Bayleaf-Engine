using System.Globalization;

using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.GameInterface;
using Bayleaf.Script;

using GameInterfaceComponent = Bayleaf.Scene.Components.GameInterface;

namespace Bayleaf.Engine;

/// <summary>What scripts and the player changed of one element while the game plays.</summary>
public sealed class ElementState
{
    public string? Text { get; set; }

    public double? Value { get; set; }

    public bool? Visible { get; set; }
}

/// <summary>One object's interface while the game plays: whether scripts showed or hid it, its elements, and the elements scripts added and removed.</summary>
public sealed class InterfaceState
{
    /// <summary>Most elements scripts add to one interface while the game plays.</summary>
    public const int MaxAdded = 1024;

    public bool? Visible { get; set; }

    public Dictionary<string, ElementState> Elements { get; } = new(StringComparer.Ordinal);

    /// <summary>Added by scripts, in order: the name of the element each went into ("" for the root), and the element (a part already in place).</summary>
    public List<(string Parent, Element Element)> Added { get; } = [];

    /// <summary>Named elements scripts removed.</summary>
    public List<string> Removed { get; } = [];
}

/// <summary>What the player did to an element.</summary>
public abstract record InterfaceInput
{
    private InterfaceInput()
    {
    }

    /// <summary>A button was pressed (or a toggle flipped, which also changes its value).</summary>
    public sealed record Pressed : InterfaceInput;

    /// <summary>A toggle, slider or progress bar has a new value.</summary>
    public sealed record Changed(double Value) : InterfaceInput;

    /// <summary>A text field holds new text.</summary>
    public sealed record Typed(string Text) : InterfaceInput;

    /// <summary>An element with On Click was clicked: its message, sent as it is.</summary>
    public sealed record Clicked(string Message) : InterfaceInput;
}

// The game's interfaces while it plays: what scripts changed and the player did.
public sealed partial class Session
{
    /// <summary>What scripts and the player changed of <paramref name="obj"/>'s interface while the game plays.</summary>
    public InterfaceState? InterfaceStateOf(StableId obj) => Play is { } play && play.Interfaces.TryGetValue(obj, out var state) ? state : null;

    /// <summary>
    /// The player did something to <paramref name="element"/> of <paramref name="obj"/>'s interface: the
    /// change is kept, and the object's scripts get <c>pressed:&lt;name&gt;</c> or <c>changed:&lt;name&gt;</c>
    /// with the new value (or an On Click message as it is). The error says why not.
    /// </summary>
    public Result<Unit, string> InterfaceInput(StableId obj, string element, InterfaceInput input)
    {
        if (Play is not { } play)
        {
            return Result.Fail("play mode is not running");
        }
        if (input is InterfaceInput.Clicked click)
        {
            if (click.Message.Length == 0 || Text.Bytes(click.Message) > ScriptHost.MaxMessageName)
            {
                return Result.Fail("an On Click message has 1 to 64 bytes");
            }
            play.Host.Send(Scene, obj, click.Message, 0f, play.Time);
            PlayLoop.AfterScripts(this);
            return Unit.Value;
        }
        // Names inside parts are `<part>/<name>`, longer than names in a file.
        if (element.Length == 0 || Text.Bytes(element) > Layout.MaxPartName)
        {
            return Result.Fail("name the element");
        }
        if (!play.Interfaces.TryGetValue(obj, out var state))
        {
            state = new InterfaceState();
            play.Interfaces[obj] = state;
        }
        if (!state.Elements.TryGetValue(element, out var slot))
        {
            slot = new ElementState();
            state.Elements[element] = slot;
        }
        (string Message, float Value) sent;
        switch (input)
        {
            case InterfaceInput.Changed changed:
                slot.Value = changed.Value;
                sent = ($"changed:{element}", (float)changed.Value);
                break;
            case InterfaceInput.Typed typed:
                slot.Text = typed.Text;
                sent = ($"changed:{element}", typed.Text.EnumerateRunes().Count());
                break;
            default:
                sent = ($"pressed:{element}", 0f);
                break;
        }
        // The scripts hearing it read the interface as it now is.
        InterfacePlay.RefreshValues(this);
        if (Play is { } still)
        {
            still.Host.Send(Scene, obj, sent.Message, sent.Value, still.Time);
            PlayLoop.AfterScripts(this);
        }
        return Unit.Value;
    }
}

/// <summary>
/// Game interfaces in play mode: scripts change elements, elements bound to a game state value follow it,
/// and what the player does reaches the object's scripts as a message. Play keeps those changes apart from
/// the files, so stopping puts every interface back as it was written. <c>play.ui.*</c> presses, changes
/// and reads elements of a playing game, as the player would.
/// </summary>
internal static class InterfacePlay
{
    public static IEnumerable<CommandSpec> All() =>
    [
        new()
        {
            Id = "play.ui.state", Label = "Interface State",
            Params = "{id}: while playing, the object's interface as it shows: {visible, elements: [{name, kind, text, value, visible}]} for its named elements",
            Run = StateCommand,
        },
        new()
        {
            Id = "play.ui.press", Label = "Press Interface Element",
            Params = "{id, element}: while playing, presses a button or an element with On Click (its scripts get pressed:<name>, and the On Click message) or flips a toggle (changed:<name>), as the player would",
            Run = Press,
        },
        new()
        {
            Id = "play.ui.change", Label = "Change Interface Element",
            Params = "{id, element, value? | text?}: while playing, sets a slider's or toggle's value or a text field's text as the player would; its scripts get changed:<name>",
            Run = Change,
        },
    ];

    /// <summary>Reads a layout (relative to the project folder), its parts and the style files they name, each through the session's confinement.</summary>
    public static Result<(Element Root, List<StyleSheet> Styles), string> Load(Session session, string layout)
    {
        try
        {
            string Allowed(string file) => session.Allowed(Path.Combine(session.ScriptDir, file), Access.Read);
            var read = Layout.Load(Allowed(layout));
            if (read.IsError)
            {
                return read.AsFailure();
            }
            var expanded = InterfaceRules.Expand(read.Value, part => Layout.Load(Allowed(part)));
            if (expanded.IsError)
            {
                return expanded.AsFailure();
            }
            var styles = new List<StyleSheet>();
            foreach (var style in expanded.Value.Styles)
            {
                var sheet = StyleSheet.Load(Allowed(style));
                if (sheet.IsError)
                {
                    return sheet.AsFailure();
                }
                styles.Add(sheet.Value);
            }
            return (expanded.Value.Root, styles);
        }
        catch (CommandException e)
        {
            return Result.Fail(e.Error.Message);
        }
    }

    /// <summary>The element tree as it shows now: the file's, with what scripts and the player changed, and bound values from the game's state.</summary>
    public static Element Shown(Element file, InterfaceState? state, GameState? game)
    {
        var root = file.Clone();
        if (state is not null)
        {
            foreach (var name in state.Removed)
            {
                foreach (var element in root.Walk())
                {
                    element.Children.RemoveAll(c => c.Name == name);
                }
            }
            foreach (var (parent, added) in state.Added)
            {
                if (parent.Length == 0)
                {
                    if (root.Kind.HoldsChildren())
                    {
                        root.Children.Add(added.Clone());
                    }
                    continue;
                }
                if (root.Walk().FirstOrDefault(e => e.Name == parent && e.Kind.HoldsChildren()) is { } into)
                {
                    into.Children.Add(added.Clone());
                }
            }
        }
        foreach (var element in root.Walk())
        {
            if (InterfaceRules.BindTarget(element.Bind) is var (scope, name) && game is not null)
            {
                double? value = scope == BindScope.Game
                    ? game.Game.TryGetValue(name, out var g) ? g : null
                    : game.Players.Values.FirstOrDefault() is { } first && first.TryGetValue(name, out var p) ? p : null;
                if (value is { } v)
                {
                    if (element.Kind is Kind.Text or Kind.Button)
                    {
                        element.Text = v % 1 == 0 ? v.ToString("F0", CultureInfo.InvariantCulture) : v.ToString("F2", CultureInfo.InvariantCulture);
                    }
                    else
                    {
                        element.Value = v;
                    }
                }
            }
            if (element.Name.Length > 0 && state is not null && state.Elements.TryGetValue(element.Name, out var changed))
            {
                if (changed.Text is { } text)
                {
                    element.Text = text;
                }
                if (changed.Value is { } value)
                {
                    element.Value = value;
                }
                if (changed.Visible is { } visible)
                {
                    element.Style["visible"] = visible ? "true" : "false";
                }
            }
        }
        return root;
    }

    /// <summary>Every interface's named elements as they show, for scripts to read; each layout is read once a play (again when an Interface names another file).</summary>
    public static void RefreshValues(Session session)
    {
        if (session.Play is not { } play)
        {
            return;
        }
        var scene = session.Scene;
        if (play.Changed("interface", scene))
        {
            play.InterfaceObjects = [.. ScriptHost.Order(scene)
                .Select(id => (Id: id, Layout: scene.TryGet<GameInterfaceComponent>(id, out var c) ? c.Layout : ""))
                .Where(o => o.Layout.Length > 0)];
        }
        if (play.InterfaceObjects.Count == 0)
        {
            if (play.Host.InterfaceValues.Count > 0)
            {
                play.Host.InterfaceValues = new Dictionary<(StableId, string), (double, string)>();
            }
            return;
        }
        var values = new Dictionary<(StableId, string), (double, string)>();
        foreach (var (id, layout) in play.InterfaceObjects)
        {
            if (!play.InterfaceRoots.TryGetValue(id, out var cached) || cached.Layout != layout)
            {
                var read = Load(session, layout);
                cached = (layout, read.IsOk ? read.Value.Root : null);
                play.InterfaceRoots[id] = cached;
            }
            if (cached.Root is not { } root)
            {
                continue;
            }
            foreach (var element in Shown(root, session.InterfaceStateOf(id), play.Host.State).Walk().Where(e => e.Name.Length > 0))
            {
                values[(id, element.Name)] = (element.Value, element.Text);
            }
        }
        play.Host.InterfaceValues = values;
    }

    /// <summary>A script's change to an interface, done after the step.</summary>
    public static void Apply(Session session, StableId by, StableId obj, string element, InterfaceChange change)
    {
        Element? made = null;
        if (change is InterfaceChange.Add add)
        {
            var element2 = NewElement(session, element, add.What);
            if (element2.IsError)
            {
                session.Log(Level.Error, $"[{session.Scene.NameOf(by) ?? "Object"}] cannot add {Text.Debug(element)} to the interface: {element2.Error}");
                return;
            }
            made = element2.Value;
        }
        if (session.Play is not { } play)
        {
            return;
        }
        if (!play.Interfaces.TryGetValue(obj, out var state))
        {
            state = new InterfaceState();
            play.Interfaces[obj] = state;
        }
        switch (change)
        {
            case InterfaceChange.Add a:
                if (state.Added.Count < InterfaceState.MaxAdded && made is not null)
                {
                    state.Removed.Remove(element);
                    state.Added.Add((a.Parent, made));
                }
                return;
            case InterfaceChange.Remove:
                state.Added.RemoveAll(e => e.Element.Name == element);
                if (!state.Removed.Contains(element))
                {
                    state.Removed.Add(element);
                }
                return;
        }
        if (element.Length == 0)
        {
            if (change is InterfaceChange.Visible whole)
            {
                state.Visible = whole.Shown;
            }
            return;
        }
        if (!state.Elements.TryGetValue(element, out var slot))
        {
            slot = new ElementState();
            state.Elements[element] = slot;
        }
        switch (change)
        {
            case InterfaceChange.Text t:
                slot.Text = t.Content;
                break;
            case InterfaceChange.Value v:
                slot.Value = v.Number;
                break;
            case InterfaceChange.Visible v:
                slot.Visible = v.Shown;
                break;
        }
    }

    /// <summary>The element a script adds, named <paramref name="name"/>: <paramref name="what"/> is a kind or a layout file (<c>.bui</c>), which comes in as a part put in place.</summary>
    private static Result<Element, string> NewElement(Session session, string name, string what)
    {
        if (name.Length == 0 || Text.Bytes(name) > Layout.MaxName || name.Contains(':', StringComparison.Ordinal))
        {
            return Result.Fail($"name it with 1 to {Layout.MaxName} bytes and no colons");
        }
        if (Text.AsciiLower(what).EndsWith($".{Layout.LayoutExtension}", StringComparison.Ordinal))
        {
            // Named before it is put in place, so its inner names become `<name>/<element>`, as in a file.
            var part = new Element(Kind.Part) { LayoutFile = what, Name = name };
            var host = Layout.Starter();
            host.Root = part;
            try
            {
                var expanded = InterfaceRules.Expand(host, file => Layout.Load(session.Allowed(Path.Combine(session.ScriptDir, file), Access.Read)));
                return expanded.IsOk ? expanded.Value.Root : expanded.AsFailure();
            }
            catch (CommandException e)
            {
                return Result.Fail(e.Error.Message);
            }
        }
        var kind = Kinds.All.FirstOrDefault(k => k != Kind.Part && k.Name() == what, Kind.Part);
        if (kind != Kind.Part)
        {
            return new Element(kind) { Name = name };
        }
        return Result.Fail($"{Text.Debug(what)} is not a kind of element or a .bui file; kinds: {string.Join(", ", Kinds.All.Select(k => k.Name()))}");
    }

    /// <summary>The playing object's interface as it shows now: its elements, whether it shows at all, and its style files.</summary>
    private static (Element Root, bool Visible, List<StyleSheet> Styles) Playing(Session session, StableId id)
    {
        if (!session.IsPlaying)
        {
            throw Stop.Failed("play mode is not running");
        }
        if (!session.Scene.TryGet<GameInterfaceComponent>(id, out var component))
        {
            throw Stop.BadParams($"{id} has no Interface");
        }
        if (component.Layout.Length == 0)
        {
            throw Stop.Failed("the Interface names no layout file");
        }
        var (root, styles) = Load(session, component.Layout).OrFail();
        var state = session.InterfaceStateOf(id);
        return (Shown(root, state, session.Play!.Host.State), state?.Visible ?? component.Visible, styles);
    }

    private static JsonObject StateCommand(Session session, JsonValue p)
    {
        var id = Params.Of(p, "id").Required("id", JsonRead.Id);
        var (root, visible, styles) = Playing(session, id);
        var elements = root.Walk().Where(e => e.Name.Length > 0).Select(e => (JsonValue)J.Obj(
            ("name", J.S(e.Name)),
            ("kind", J.S(e.Kind.Name())),
            ("text", J.S(e.Text)),
            ("value", JsonNumber.Of(e.Value)),
            ("visible", J.B(Style.Of(e, styles).Visible))));
        return J.Obj(("visible", J.B(visible)), ("elements", new JsonArray(elements)));
    }

    private static Element Named(Element root, string name) =>
        root.Walk().FirstOrDefault(e => e.Name == name) ?? throw Stop.BadParams($"no element is named {Text.Debug(name)}");

    private static JsonObject Press(Session session, JsonValue p)
    {
        var f = Params.Of(p, "id", "element");
        var id = f.Required("id", JsonRead.Id);
        var name = f.Required("element", JsonRead.Str);
        var (root, _, _) = Playing(session, id);
        var element = Named(root, name);
        InterfaceInput input = element.Kind switch
        {
            Kind.Button => new InterfaceInput.Pressed(),
            Kind.Toggle => new InterfaceInput.Changed(element.Value != 0 ? 0 : 1),
            _ when element.OnClick.Length > 0 => new InterfaceInput.Pressed(),
            var other => throw Stop.BadParams($"{Text.Debug(name)} is a {other.Name()} without On Click; only buttons, toggles and elements with On Click are pressed"),
        };
        session.InterfaceInput(id, name, input).OrFail();
        if (element.OnClick.Length > 0)
        {
            session.InterfaceInput(id, name, new InterfaceInput.Clicked(element.OnClick)).OrFail();
        }
        return [];
    }

    private static JsonObject Change(Session session, JsonValue p)
    {
        var f = Params.Of(p, "id", "element", "value", "text");
        var id = f.Required("id", JsonRead.Id);
        var name = f.Required("element", JsonRead.Str);
        double? value = f.NullableValue("value", JsonRead.F64);
        var text = f.Nullable("text", JsonRead.Str);
        var (root, _, _) = Playing(session, id);
        var element = Named(root, name);
        InterfaceInput input = (element.Kind, value, text) switch
        {
            (Kind.Slider, { } v, null) when double.IsFinite(v) => new InterfaceInput.Changed(Math.Clamp(v, element.Min, Math.Max(element.Min, element.Max))),
            (Kind.Toggle, { } v, null) when double.IsFinite(v) => new InterfaceInput.Changed(v != 0 ? 1 : 0),
            (Kind.TextField, null, { } t) when Text.Bytes(t) <= Layout.MaxText => new InterfaceInput.Typed(t),
            var (kind, _, _) => throw Stop.BadParams($"{Text.Debug(name)} is a {kind.Name()}: give a slider or toggle a finite value, or a text field its text (at most {Layout.MaxText} bytes)"),
        };
        session.InterfaceInput(id, name, input).OrFail();
        return [];
    }
}
