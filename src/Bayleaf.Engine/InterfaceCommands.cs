using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.GameInterface;

namespace Bayleaf.Engine;

/// <summary>
/// Game interface files: <c>ui.layout.*</c> and <c>ui.element.*</c> read and edit layouts
/// (<c>.bui</c>), <c>ui.style.*</c> style files (<c>.bstyle</c>). Each edit reads the file, changes
/// it, checks the result and writes it; nothing is written when the change or the check fails. The
/// <c>play.ui.*</c> commands come with play mode.
/// </summary>
internal static class InterfaceCommands
{
    public static IEnumerable<CommandSpec> All() =>
    [
        new() { Id = "ui.layout.get", Label = "Read Interface", Params = "{path}: an interface layout (.bui) as {layout: {format, version, styles, root}}", Run = LayoutGet },
        new()
        {
            Id = "ui.layout.set", Label = "Write Interface",
            Params = "{path, layout}: checks and writes an interface layout (.bui): elements {kind, name?, classes?, text?, image?, value?, min?, max?, bind?, style?, children?}",
            Run = LayoutSet,
        },
        new()
        {
            Id = "ui.element.add", Label = "Add Interface Element",
            Params = "{path, into?, kind, index?}: adds an element of kind to a layout, inside the element at into (child indices from the root; the root when left out), at index or last. Returns {at}",
            Run = ElementAdd,
        },
        new()
        {
            Id = "ui.element.remove", Label = "Remove Interface Element",
            Params = "{path, at}: removes the element at at (child indices from the root) and what it holds", Run = ElementRemove,
        },
        new()
        {
            Id = "ui.element.move", Label = "Move Interface Element",
            Params = "{path, at, into, index}: moves the element at at into the element at into, at index (counted without it). Returns {at}", Run = ElementMove,
        },
        new()
        {
            Id = "ui.element.set", Label = "Edit Interface Element",
            Params = "{path, at, element: {kind?, name?, classes?, text?, image?, layout?, onClick?, value?, min?, max?, bind?, style?}}: changes the given fields of the element at at; style replaces the element's whole style",
            Run = ElementSet,
        },
        new()
        {
            Id = "ui.element.saveAsPart", Label = "Save Interface Part",
            Params = "{path, at, to}: writes the element at at, with what it holds, as a layout file of its own (to, a new .bui) and puts a part using it in its place; inside it, its names are then <part>/<name>",
            Run = ElementSaveAsPart,
        },
        new() { Id = "ui.style.get", Label = "Read Interface Style", Params = "{path}: an interface style file (.bstyle) as {style: {format, version, rules}}", Run = StyleGet },
        new()
        {
            Id = "ui.style.set", Label = "Write Interface Style",
            Params = "{path, style}: checks and writes a style file (.bstyle): rules {kind | class | name, set: {property: value}}", Run = StyleSet,
        },
    ];

    private static string Checked(Session session, string path, string extension, Access access)
    {
        if (!Text.AsciiLower(path).EndsWith($".{extension}", StringComparison.Ordinal))
        {
            throw Stop.BadParams($"give a .{extension} file");
        }
        return session.Allowed(Path.Combine(session.ScriptDir, path), access);
    }

    private static void Write(string path, string text) =>
        Files.Write(path, System.Text.Encoding.UTF8.GetBytes(text)).MapError(e => $"cannot write {path}: {e}").OrFail();

    private static List<int> Indices(JsonValue value) => JsonRead.List(value, Params.Index);

    private static string NoElement(IReadOnlyList<int> at) => $"there is no element at [{string.Join(", ", at)}]";

    private static JsonObject LayoutGet(Session session, JsonValue p)
    {
        var path = Checked(session, Params.Of(p, "path").Required("path", JsonRead.Str), Layout.LayoutExtension, Access.Read);
        return J.Obj(("layout", Layout.Load(path).OrFail().ToJson()));
    }

    private static JsonObject LayoutSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "layout");
        var given = f.Required("path", JsonRead.Str);
        var layout = f.Required("layout", Layout.Read);
        if (layout.Validate().TryGet(out _, out var invalid) is false)
        {
            throw Stop.BadParams(invalid);
        }
        var path = Checked(session, given, Layout.LayoutExtension, Access.Write);
        Write(path, layout.ToText());
        return J.Obj(("path", J.S(path)));
    }

    /// <summary>Reads a layout, changes it with <paramref name="change"/> (which says why it cannot by throwing a bad-params error), checks the result and writes it.</summary>
    private static T EditLayout<T>(Session session, string given, Func<Layout, T> change)
    {
        var path = Checked(session, given, Layout.LayoutExtension, Access.Write);
        var layout = Layout.Load(path).OrFail();
        var result = change(layout);
        if (layout.Validate().TryGet(out _, out var invalid) is false)
        {
            throw Stop.BadParams(invalid);
        }
        Write(path, layout.ToText());
        return result;
    }

    private static JsonObject ElementAdd(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "into", "kind", "index");
        var given = f.Required("path", JsonRead.Str);
        var into = f.Optional("into", Indices, []);
        var kind = f.Required("kind", Kinds.Read);
        int? index = f.NullableValue("index", Params.Index);
        var at = EditLayout(session, given, layout =>
        {
            var parent = layout.ElementAt(into) ?? throw Stop.BadParams(NoElement(into));
            if (!parent.Kind.HoldsChildren())
            {
                throw Stop.BadParams($"a {parent.Kind.Name()} holds no other elements; add into a panel, row, column or list");
            }
            var place = Math.Min(index ?? parent.Children.Count, parent.Children.Count);
            var element = new Element(kind);
            // Something to see and press, to change from.
            if (kind is Kind.Text or Kind.Button or Kind.Toggle)
            {
                var name = kind.Name();
                element.Text = char.ToUpperInvariant(name[0]) + name[1..];
            }
            parent.Children.Insert(place, element);
            return new List<int>(into) { place };
        });
        return J.Obj(("at", new JsonArray(at.Select(i => (JsonValue)J.N(i)))));
    }

    private static JsonObject ElementRemove(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "at");
        var given = f.Required("path", JsonRead.Str);
        var at = f.Required("at", Indices);
        EditLayout(session, given, layout =>
        {
            if (at.Count == 0)
            {
                throw Stop.BadParams("the root element stays; change it instead");
            }
            var parent = layout.ElementAt(at.Take(at.Count - 1)) ?? throw Stop.BadParams(NoElement(at));
            if (at[^1] >= parent.Children.Count)
            {
                throw Stop.BadParams(NoElement(at));
            }
            parent.Children.RemoveAt(at[^1]);
            return 0;
        });
        return [];
    }

    private static JsonObject ElementMove(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "at", "into", "index");
        var given = f.Required("path", JsonRead.Str);
        var at = f.Required("at", Indices);
        var into = f.Required("into", Indices);
        var index = f.Required("index", Params.Index);
        var moved = EditLayout(session, given, layout =>
        {
            if (at.Count == 0)
            {
                throw Stop.BadParams("the root element cannot move");
            }
            var (last, from) = (at[^1], at.Take(at.Count - 1).ToList());
            if (into.Count >= at.Count && into.Take(at.Count).SequenceEqual(at))
            {
                throw Stop.BadParams("an element cannot move into itself");
            }
            var target = layout.ElementAt(into) ?? throw Stop.BadParams(NoElement(into));
            if (!target.Kind.HoldsChildren())
            {
                throw Stop.BadParams($"a {target.Kind.Name()} holds no other elements");
            }
            var parent = layout.ElementAt(from) ?? throw Stop.BadParams(NoElement(at));
            if (last >= parent.Children.Count)
            {
                throw Stop.BadParams(NoElement(at));
            }
            var element = parent.Children[last];
            parent.Children.RemoveAt(last);
            // Taking it out shifts its later siblings, which may hold the target.
            var path = new List<int>(into);
            if (path.Count > from.Count && path.Take(from.Count).SequenceEqual(from) && path[from.Count] > last)
            {
                path[from.Count]--;
            }
            var newParent = layout.ElementAt(path) ?? throw Stop.BadParams(NoElement(into));
            var place = Math.Min(index, newParent.Children.Count);
            newParent.Children.Insert(place, element);
            path.Add(place);
            return path;
        });
        return J.Obj(("at", new JsonArray(moved.Select(i => (JsonValue)J.N(i)))));
    }

    private static JsonObject ElementSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "at", "element");
        var given = f.Required("path", JsonRead.Str);
        var at = f.Required("at", Indices);
        var fields = f.Required("element", v => v as JsonObject ?? throw new JsonShapeException($"invalid type: {JsonRead.Describe(v)}, expected a map"));
        if (fields.ContainsKey("children"))
        {
            throw Stop.BadParams("children change with ui.element.add, remove and move");
        }
        EditLayout(session, given, layout =>
        {
            var element = layout.ElementAt(at) ?? throw Stop.BadParams(NoElement(at));
            var value = element.Write();
            foreach (var (key, field) in fields.OrderBy(e => e.Key, Utf8Order.Instance))
            {
                value.Set(key, field);
            }
            Element changed;
            try
            {
                changed = Element.Read(value);
            }
            catch (JsonShapeException e)
            {
                throw Stop.BadParams($"element: {e.Message}");
            }
            if (changed.Children.Count > 0 && !changed.Kind.HoldsChildren())
            {
                throw Stop.BadParams($"a {changed.Kind.Name()} holds no other elements; remove its children first");
            }
            element.Become(changed);
            return 0;
        });
        return [];
    }

    private static JsonObject ElementSaveAsPart(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "at", "to");
        var given = f.Required("path", JsonRead.Str);
        var at = f.Required("at", Indices);
        var toParam = f.Required("to", JsonRead.Str);
        if (at.Count == 0)
        {
            throw Stop.BadParams("the root is the whole layout already; save a part of it");
        }
        var to = Checked(session, toParam, Layout.LayoutExtension, Access.Write);
        if (AssetCommands.Exists(to))
        {
            throw Stop.Failed($"{to} already exists; pick another name");
        }
        var source = Checked(session, given, Layout.LayoutExtension, Access.Read);
        var host = Layout.Load(source).OrFail();
        var element = host.ElementAt(at)?.Clone() ?? throw Stop.BadParams(NoElement(at));
        if (element.Kind == Kind.Part)
        {
            throw Stop.BadParams("it is a part already");
        }
        // The part keeps the style files the layout uses, so it looks the same on its own.
        var part = new Layout(Layout.LayoutFormat, Layout.CurrentVersion, [.. host.Styles], element.Clone());
        if (part.Validate().TryGet(out _, out var invalid) is false)
        {
            throw Stop.BadParams(invalid);
        }
        // Names inside a part are its own: the bigger layout calls them `<part>/<name>`.
        EditLayout(session, given, layout =>
        {
            var slot = layout.ElementAt(at) ?? throw Stop.BadParams(NoElement(at));
            slot.Become(new Element(Kind.Part) { Name = element.Name, LayoutFile = toParam.Replace('\\', '/') });
            return 0;
        });
        if (Files.Parent(to) is { Length: > 0 } folder)
        {
            try
            {
                Directory.CreateDirectory(folder);
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                throw Stop.Failed($"cannot make {folder}: {e.Message}");
            }
        }
        Write(to, part.ToText());
        return J.Obj(("path", J.S(to)));
    }

    private static JsonObject StyleGet(Session session, JsonValue p)
    {
        var path = Checked(session, Params.Of(p, "path").Required("path", JsonRead.Str), StyleSheet.StyleExtension, Access.Read);
        return J.Obj(("style", StyleSheet.Load(path).OrFail().ToJson()));
    }

    private static JsonObject StyleSet(Session session, JsonValue p)
    {
        var f = Params.Of(p, "path", "style");
        var given = f.Required("path", JsonRead.Str);
        var style = f.Required("style", StyleSheet.Read);
        if (style.Validate().TryGet(out _, out var invalid) is false)
        {
            throw Stop.BadParams(invalid);
        }
        var path = Checked(session, given, StyleSheet.StyleExtension, Access.Write);
        Write(path, style.ToText());
        return J.Obj(("path", J.S(path)));
    }
}
