using Bayleaf.Core;
using Bayleaf.Core.Json;

namespace Bayleaf.Graph;

/// <summary>What a visual script declares: variables, functions, events and properties.</summary>
public sealed partial class Graph
{
    /// <summary>A variable name: 1 to 64 letters, digits or <c>_</c>.</summary>
    public static Result<Unit, string> CheckVariableName(string name) =>
        J.IsName(name) ? Result.Ok<string>() : Result.Fail($"variable name {Say.Debug(name)} must be 1 to 64 letters, digits or _");

    private static Result<Unit, GraphError> CheckName(string what, string name) =>
        J.IsName(name) ? Done : No(GraphError.Refused($"{what} name {Say.Debug(name)} must be 1 to 64 letters, digits or _"));

    /// <summary>The pins a list of params becomes, as nodes keep them: <c>name:type</c>.</summary>
    public static JsonValue ParamPins(IEnumerable<Param> parameters) => J.Strings(parameters.Select(p => $"{p.Name}:{p.Type.ShapedCode(p.Shape, p.Key)}"));

    /// <summary>A function's inputs or outputs: at most <see cref="MaxParams"/>, each named like a variable, no name twice and none a flow pin's.</summary>
    private static Result<Unit, string> CheckParams(string function, IReadOnlyList<Param> parameters)
    {
        if (parameters.Count > MaxParams)
        {
            return Result.Fail(JsonRead.Invariant($"a function has at most {MaxParams} inputs and {MaxParams} outputs"));
        }
        for (var index = 0; index < parameters.Count; index++)
        {
            var param = parameters[index];
            var named = CheckVariableName(param.Name);
            if (named.IsError)
            {
                return named;
            }
            if (FlowPins.Contains(param.Name))
            {
                return Result.Fail($"{function}: {param.Name} is the name of a flow pin");
            }
            if (parameters.Take(index).Any(p => p.Name == param.Name))
            {
                return Result.Fail($"{function}: {param.Name} is there twice");
            }
        }
        return Result.Ok<string>();
    }

    /// <summary>Sets the <c>name</c> setting of the nodes of <paramref name="kinds"/> that name <paramref name="name"/> to <paramref name="rename"/>.</summary>
    private void FollowName(IReadOnlyCollection<string> kinds, string name, string rename)
    {
        foreach (var node in Nodes.Where(n => kinds.Contains(n.Kind)))
        {
            if (node.Values.GetValueOrDefault("name") is JsonString s && s.Value == name)
            {
                node.Values["name"] = J.S(rename);
            }
        }
    }

    /// <summary>The nodes of <paramref name="kinds"/> whose <paramref name="setting"/> is <paramref name="name"/>.</summary>
    private List<uint> Naming(IReadOnlyCollection<string> kinds, string setting, string name) =>
        Nodes.Where(n => kinds.Contains(n.Kind) && n.Values.GetValueOrDefault(setting) is JsonString s && s.Value == name).Select(n => n.Id).ToList();

    public Variable? FindVariable(string name) => Variables.Find(v => v.Name == name);

    /// <summary>Declares a variable; <paramref name="start"/> defaults to the type's zero.</summary>
    public Result<Unit, GraphError> AddVariable(string name, VariableType type, JsonValue? start)
    {
        if (CheckVariableName(name).TryGet(out _, out var bad) is false)
        {
            return No(GraphError.Refused(bad));
        }
        if (FindVariable(name) is not null)
        {
            return No(GraphError.Refused($"there is already a variable {name}"));
        }
        if (Variables.Count >= MaxVariables)
        {
            return No(GraphError.Refused(JsonRead.Invariant($"a graph declares at most {MaxVariables} variables")));
        }
        var first = (start ?? type.Zero()).Canonical();
        if (!type.Fits(first))
        {
            return No(GraphError.Refused($"a {type.Label()} variable cannot start at {first}"));
        }
        Variables.Add(new(name, type, Shape.One, VariableType.Text, first));
        return Done;
    }

    /// <summary>Changes a variable's start value, or renames it; nodes of <paramref name="kinds"/> that name it by their <c>name</c> setting follow the new name.</summary>
    public Result<Unit, GraphError> SetVariable(string name, string? rename, JsonValue? start, IReadOnlyCollection<string> kinds)
    {
        if (FindVariable(name) is not { } variable)
        {
            return No(GraphError.Refused($"there is no variable {name}"));
        }
        if (start?.Canonical() is { } value)
        {
            if (!variable.Type.FitsShape(variable.Shape, variable.Key, value))
            {
                return No(GraphError.Refused($"a {variable.Type.Label()} variable cannot start at {value}"));
            }
            variable.Start = value;
        }
        if (rename is not null && rename != name)
        {
            if (CheckVariableName(rename).TryGet(out _, out var bad) is false)
            {
                return No(GraphError.Refused(bad));
            }
            if (FindVariable(rename) is not null)
            {
                return No(GraphError.Refused($"there is already a variable {rename}"));
            }
            variable.Name = rename;
            FollowName(kinds, name, rename);
        }
        return Done;
    }

    public DeclaredFunction? FindFunction(string name) => Functions.Find(f => f.Name == name);

    /// <summary>Declares a function with no inputs or outputs.</summary>
    public Result<Unit, GraphError> AddFunction(string name)
    {
        var named = CheckName("function", name);
        if (named.IsError)
        {
            return named;
        }
        if (FindFunction(name) is not null)
        {
            return No(GraphError.Refused($"there is already a function {name}"));
        }
        if (Functions.Count >= MaxFunctions)
        {
            return No(GraphError.Refused(JsonRead.Invariant($"a graph declares at most {MaxFunctions} functions")));
        }
        Functions.Add(new(name, [], []));
        return Done;
    }

    /// <summary>
    /// Renames a function or changes its inputs or outputs. Nodes of <paramref name="kinds"/> that
    /// name it (their <c>function</c> setting) follow: the new name, and their <c>inputs</c> and
    /// <c>outputs</c> pins; links to pins that went away go too.
    /// </summary>
    public Result<Unit, GraphError> SetFunction(Catalog catalog, string name, string? rename, IReadOnlyList<Param>? inputs, IReadOnlyList<Param>? outputs, IReadOnlyCollection<string> kinds)
    {
        if (FindFunction(name) is not { } function)
        {
            return No(GraphError.Refused($"there is no function {name}"));
        }
        if (rename is not null && rename != name)
        {
            var named = CheckName("function", rename);
            if (named.IsError)
            {
                return named;
            }
            if (FindFunction(rename) is not null)
            {
                return No(GraphError.Refused($"there is already a function {rename}"));
            }
            function = function with { Name = rename };
        }
        foreach (var (given, isInputs) in new[] { (inputs, true), (outputs, false) })
        {
            if (given is null)
            {
                continue;
            }
            if (CheckParams(name, given).TryGet(out _, out var bad) is false)
            {
                return No(GraphError.Refused(bad));
            }
            function = isInputs ? function with { Inputs = [.. given] } : function with { Outputs = [.. given] };
        }
        // Try it on a copy, so a refused node change leaves the graph as it was.
        var trial = Clone();
        var slot = trial.Functions.FindIndex(f => f.Name == name);
        if (slot >= 0)
        {
            trial.Functions[slot] = function;
        }
        foreach (var id in trial.Naming(kinds, "function", name))
        {
            var step = Steps(
                () => trial.SetValue(catalog, id, "inputs", ParamPins(function.Inputs)),
                () => trial.SetValue(catalog, id, "outputs", ParamPins(function.Outputs)),
                () =>
                {
                    trial.DropUnfitLinks(catalog, id);
                    return Done;
                },
                () => trial.SetValue(catalog, id, "function", J.S(function.Name)));
            if (step.IsError)
            {
                return step;
            }
        }
        Become(trial);
        return Done;
    }

    /// <summary>Runs <paramref name="steps"/> in order, stopping at the first that fails.</summary>
    private static Result<Unit, GraphError> Steps(params Func<Result<Unit, GraphError>>[] steps)
    {
        foreach (var step in steps)
        {
            var result = step();
            if (result.IsError)
            {
                return result;
            }
        }
        return Done;
    }

    public DeclaredEvent? FindEvent(string name) => Events.Find(e => e.Name == name);

    /// <summary>Declares an event that brings nothing.</summary>
    public Result<Unit, GraphError> AddEvent(string name)
    {
        var named = CheckName("event", name);
        if (named.IsError)
        {
            return named;
        }
        if (FindEvent(name) is not null)
        {
            return No(GraphError.Refused($"there is already an event {name}"));
        }
        if (Events.Count >= MaxEvents)
        {
            return No(GraphError.Refused(JsonRead.Invariant($"a graph declares at most {MaxEvents} events")));
        }
        Events.Add(new(name, []));
        return Done;
    }

    /// <summary>
    /// Renames an event or changes what it brings. Nodes of <paramref name="kinds"/> that name it
    /// (their <c>event</c> setting) follow: the new name, and their <c>params</c> pins; links to pins
    /// that went away go too.
    /// </summary>
    public Result<Unit, GraphError> SetEvent(Catalog catalog, string name, string? rename, IReadOnlyList<Param>? parameters, IReadOnlyCollection<string> kinds)
    {
        if (FindEvent(name) is not { } e)
        {
            return No(GraphError.Refused($"there is no event {name}"));
        }
        if (rename is not null && rename != name)
        {
            var named = CheckName("event", rename);
            if (named.IsError)
            {
                return named;
            }
            if (FindEvent(rename) is not null)
            {
                return No(GraphError.Refused($"there is already an event {rename}"));
            }
            e = e with { Name = rename };
        }
        if (parameters is not null)
        {
            if (CheckParams(name, parameters).TryGet(out _, out var bad) is false)
            {
                return No(GraphError.Refused(bad));
            }
            e = e with { Params = [.. parameters] };
        }
        var trial = Clone();
        var slot = trial.Events.FindIndex(x => x.Name == name);
        if (slot >= 0)
        {
            trial.Events[slot] = e;
        }
        foreach (var id in trial.Naming(kinds, "event", name))
        {
            var step = Steps(
                () => trial.SetValue(catalog, id, "params", ParamPins(e.Params)),
                () =>
                {
                    trial.DropUnfitLinks(catalog, id);
                    return Done;
                },
                () => trial.SetValue(catalog, id, "event", J.S(e.Name)));
            if (step.IsError)
            {
                return step;
            }
        }
        Become(trial);
        return Done;
    }

    /// <summary>
    /// Gives a variable another type, shape or key type (a map's); its start stays where it still
    /// fits, else it goes back to the new type's zero (an empty list or map). Returns the old type
    /// and shape. Its nodes are the caller's to follow (they depend on the domain's node kinds).
    /// </summary>
    public Result<(VariableType Type, Shape Shape), GraphError> RetypeVariable(string name, VariableType type, Shape shape, VariableType key)
    {
        if (FindVariable(name) is not { } variable)
        {
            return Result.Fail(GraphError.Refused($"there is no variable {name}"));
        }
        var old = (variable.Type, variable.Shape);
        (variable.Type, variable.Shape, variable.Key) = (type, shape, key);
        if (!type.FitsShape(shape, key, variable.Start))
        {
            variable.Start = type.ZeroOf(shape);
        }
        return old;
    }

    /// <summary>Gives a property another type or shape, as <see cref="RetypeVariable"/> does a variable.</summary>
    public Result<(VariableType Type, Shape Shape), GraphError> RetypeProperty(string name, VariableType type, Shape shape, VariableType key)
    {
        if (FindProperty(name) is not { } property)
        {
            return Result.Fail(GraphError.Refused($"there is no property {name}"));
        }
        var old = (property.Type, property.Shape);
        (property.Type, property.Shape, property.Key) = (type, shape, key);
        if (!type.FitsShape(shape, key, property.Default))
        {
            property.Default = type.ZeroOf(shape);
        }
        return old;
    }

    /// <summary>
    /// Makes node <paramref name="id"/> a node of another kind in place: it keeps its id, place and
    /// the values the new kind still has, and loses the links the new kind's pins no longer take.
    /// </summary>
    public Result<Unit, GraphError> ChangeKind(Catalog catalog, uint id, string kind)
    {
        if (catalog.Get(kind) is null)
        {
            return No(GraphError.UnknownKind(kind, catalog.Domain));
        }
        var trial = Clone();
        if (trial.FindNode(id) is not { } node)
        {
            return No(GraphError.NoNode(id));
        }
        node.Kind = kind;
        if (catalog.DefOf(node) is not { } def)
        {
            return No(GraphError.UnknownKind(kind, catalog.Domain));
        }
        foreach (var name in node.Values.Keys.Where(n => def.Input(n) is null && def.Setting(n) is null).ToList())
        {
            node.Values.Remove(name);
        }
        trial.DropUnfitLinks(catalog, id);
        Become(trial);
        return Done;
    }

    /// <summary>
    /// The nodes of <paramref name="kinds"/> that name <paramref name="name"/> (their <c>name</c>
    /// setting) keep the type codes of what it holds and of its keys in <c>item</c>
    /// (<c>["number", "text"]</c>); when they change, links that no longer fit go.
    /// </summary>
    public Result<Unit, GraphError> FollowItem(Catalog catalog, string name, VariableType item, VariableType key, IReadOnlyCollection<string> kinds)
    {
        var code = J.Strings([item.Code(), key.Code()]);
        var named = Naming(kinds, "name", name);
        var trial = Clone();
        foreach (var id in named)
        {
            if (trial.ValueOf(id, "item") is { } held && held.Equals(code))
            {
                continue;
            }
            var set = trial.SetValue(catalog, id, "item", code);
            if (set.IsError)
            {
                return set;
            }
            trial.DropUnfitLinks(catalog, id);
        }
        Become(trial);
        return Done;
    }

    /// <summary>Takes an event away; nodes naming it stay (a Trigger Event may name another script's event).</summary>
    public Result<Unit, GraphError> RemoveEvent(string name) =>
        Events.RemoveAll(e => e.Name == name) > 0 ? Done : No(GraphError.Refused($"there is no event {name}"));

    /// <summary>Takes a function away; nodes naming it stay, and the compiler says they name no function.</summary>
    public Result<Unit, GraphError> RemoveFunction(string name) =>
        Functions.RemoveAll(f => f.Name == name) > 0 ? Done : No(GraphError.Refused($"there is no function {name}"));

    public DeclaredProperty? FindProperty(string name) => Properties.Find(p => p.Name == name);

    /// <summary>Declares a property; <paramref name="default"/> defaults to the type's zero.</summary>
    public Result<Unit, GraphError> AddProperty(string name, VariableType type, JsonValue? @default)
    {
        var named = CheckName("property", name);
        if (named.IsError)
        {
            return named;
        }
        if (FindProperty(name) is not null)
        {
            return No(GraphError.Refused($"there is already a property {name}"));
        }
        if (Properties.Count >= MaxProperties)
        {
            return No(GraphError.Refused(JsonRead.Invariant($"a graph declares at most {MaxProperties} properties")));
        }
        var value = (@default ?? type.Zero()).Canonical();
        if (!type.Fits(value))
        {
            return No(GraphError.Refused($"a {type.Label()} property cannot default to {value}"));
        }
        Properties.Add(new(name, type, Shape.One, VariableType.Text, value, PropertyOptions.Empty));
        return Done;
    }

    /// <summary>Changes a property's default, or renames it; nodes of <paramref name="kinds"/> that name it by their <c>name</c> setting follow the new name.</summary>
    public Result<Unit, GraphError> SetProperty(string name, string? rename, JsonValue? @default, IReadOnlyCollection<string> kinds)
    {
        if (FindProperty(name) is not { } property)
        {
            return No(GraphError.Refused($"there is no property {name}"));
        }
        if (@default?.Canonical() is { } value)
        {
            if (!property.Type.FitsShape(property.Shape, property.Key, value))
            {
                return No(GraphError.Refused($"a {property.Type.Label()} property cannot default to {value}"));
            }
            property.Default = value;
        }
        if (rename is not null && rename != name)
        {
            var named = CheckName("property", rename);
            if (named.IsError)
            {
                return named;
            }
            if (FindProperty(rename) is not null)
            {
                return No(GraphError.Refused($"there is already a property {rename}"));
            }
            property.Name = rename;
            FollowName(kinds, name, rename);
        }
        return Done;
    }

    /// <summary>Sets how a property shows in the Inspector (checked by whoever knows the rules: the script host).</summary>
    public Result<Unit, GraphError> SetPropertyOptions(string name, PropertyOptions options)
    {
        if (FindProperty(name) is not { } property)
        {
            return No(GraphError.Refused($"there is no property {name}"));
        }
        property.Inspector = options;
        return Done;
    }

    /// <summary>Takes a property away; nodes naming it stay, reading the type's zero unless an object sets it.</summary>
    public Result<Unit, GraphError> RemoveProperty(string name) =>
        Properties.RemoveAll(p => p.Name == name) > 0 ? Done : No(GraphError.Refused($"there is no property {name}"));

    /// <summary>Takes a declaration away; nodes naming it keep working, starting from the type's zero.</summary>
    public Result<Unit, GraphError> RemoveVariable(string name) =>
        Variables.RemoveAll(v => v.Name == name) > 0 ? Done : No(GraphError.Refused($"there is no variable {name}"));
}
