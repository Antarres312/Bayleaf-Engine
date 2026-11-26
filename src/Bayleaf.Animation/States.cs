using System.Numerics;

using Bayleaf.Core;
using Bayleaf.Core.Json;
using Bayleaf.Graph;

namespace Bayleaf.Animation;

/// <summary>A number a condition reads.</summary>
public abstract record Amount
{
    private Amount()
    {
    }

    public sealed record Value(float Number) : Amount;

    /// <summary>A parameter by name; 0 while none of that name is set.</summary>
    public sealed record Named(string Parameter) : Amount;

    /// <summary>Seconds since the state began.</summary>
    public sealed record Seconds : Amount;

    /// <summary>Times the state's motion has played through.</summary>
    public sealed record Runs : Amount;

    internal float Read(IReadOnlyDictionary<string, Parameter> parameters, InState state) => this switch
    {
        Value v => v.Number,
        Named n => parameters.TryGetValue(n.Parameter, out var p) ? p.AsNumber() : 0f,
        Seconds => state.Seconds,
        _ => state.Runs,
    };
}

/// <summary>How <see cref="Condition.Compare"/> compares.</summary>
public enum Test
{
    Greater,
    Less,
    AtLeast,
    AtMost,
    Equal,
    NotEqual,
}

/// <summary>What a condition reads besides the parameters: the state's time.</summary>
public readonly record struct InState(float Seconds, float Runs);

/// <summary>When a transition goes.</summary>
public abstract record Condition
{
    private Condition()
    {
    }

    public sealed record Value(bool Holds) : Condition;

    public sealed record Flag(string Name) : Condition;

    public sealed record Trigger(string Name) : Condition;

    public sealed record Compare(Amount A, Test Test, Amount B) : Condition;

    /// <summary>True when both are.</summary>
    public sealed record Both(Condition A, Condition B) : Condition;

    /// <summary>True when either is.</summary>
    public sealed record Either(Condition A, Condition B) : Condition;

    /// <summary>True when the other is false.</summary>
    public sealed record Negated(Condition Of) : Condition;

    /// <summary>Whether it holds now.</summary>
    public bool HoldsNow(IReadOnlyDictionary<string, Parameter> parameters, InState state)
    {
        switch (this)
        {
            case Value v:
                return v.Holds;
            case Flag f:
                return parameters.TryGetValue(f.Name, out var flag) && flag.AsFlag();
            case Trigger t:
                return parameters.TryGetValue(t.Name, out var trigger) && trigger.AsFlag();
            case Compare c:
                var (a, b) = (c.A.Read(parameters, state), c.B.Read(parameters, state));
                var near = MathF.Abs(a - b) <= Numbers.MachineEpsilon * Numbers.Max(Numbers.Max(MathF.Abs(a), MathF.Abs(b)), 1f);
                var far = MathF.Abs(a - b) > Numbers.MachineEpsilon * Numbers.Max(Numbers.Max(MathF.Abs(a), MathF.Abs(b)), 1f);
                return c.Test switch
                {
                    Test.Greater => a > b,
                    Test.Less => a < b,
                    Test.AtLeast => a >= b,
                    Test.AtMost => a <= b,
                    Test.Equal => near,
                    _ => far,
                };
            case Both and:
                return and.A.HoldsNow(parameters, state) && and.B.HoldsNow(parameters, state);
            case Either or:
                return or.A.HoldsNow(parameters, state) || or.B.HoldsNow(parameters, state);
            case Negated not:
                return !not.Of.HoldsNow(parameters, state);
            default:
                return false;
        }
    }

    /// <summary>The triggers it reads, which go when a transition it lets through is taken.</summary>
    public void Triggers(List<string> output)
    {
        switch (this)
        {
            case Trigger t:
                output.Add(t.Name);
                break;
            case Both and:
                and.A.Triggers(output);
                and.B.Triggers(output);
                break;
            case Either or:
                or.A.Triggers(output);
                or.B.Triggers(output);
                break;
            case Negated not:
                not.Of.Triggers(output);
                break;
            default:
                break;
        }
    }
}

/// <summary>A state: its name and what it plays.</summary>
public sealed record State(string Name, Motion Motion);

/// <summary>A way from one state (or any) to another.</summary>
/// <param name="From">Null from any other state.</param>
/// <param name="Fade">Seconds the fade takes.</param>
public sealed record Transition(int? From, int To, float Fade, Condition When);

/// <summary>Compiled animation states.</summary>
public sealed class Machine
{
    public required IReadOnlyList<State> States { get; init; }

    public required int Start { get; init; }

    /// <summary>In the order they are tried.</summary>
    public required IReadOnlyList<Transition> Transitions { get; init; }

    /// <summary>Every parameter the graph reads, by name, and its kind.</summary>
    public required IReadOnlyDictionary<string, ParameterKind> Parameters { get; init; }

    public int? State(string name)
    {
        for (var i = 0; i < States.Count; i++)
        {
            if (States[i].Name == name)
            {
                return i;
            }
        }
        return null;
    }

    /// <summary>
    /// The transition to take from state <paramref name="current"/> now, if any: the first whose
    /// condition holds (one from any state never goes to the state it is in).
    /// </summary>
    public Transition? Next(int current, IReadOnlyDictionary<string, Parameter> parameters, InState state) =>
        Transitions.FirstOrDefault(t => (t.From is null || t.From == current) && !(t.From is null && t.To == current) && t.When.HoldsNow(parameters, state));
}

/// <summary>
/// Animation states (<c>.banim</c>, domain <c>animation</c>): a graph of states, each a motion (a
/// clip, a mix or a layer), and transitions between them on conditions over parameters that scripts
/// set. <see cref="Compile"/> turns a graph into a <see cref="Machine"/>; a <see cref="Player"/> runs
/// it on the object's base layer (see <c>docs/graphs.md</c>).
/// </summary>
public static class AnimationStates
{
    private static PinDef Pin(string name, string label, PinType type) => new(name, label, type);

    private static PinDef Num(string name, string label, float value) => new(name, label, PinType.Number, new PinDefault.Number(value));

    private static PinDef Flag(string name, string label, bool value) => new(name, label, PinType.Bool, new PinDefault.Bool(value));

    private static SettingDef Setting(string name, string label, SettingKind kind) => new(name, label, kind);

    private static readonly PinDef[] MotionOut = [Pin("motion", "Motion", PinType.Motion)];

    private static readonly PinDef[] BoolOut = [Pin("result", "Result", PinType.Bool)];

    private static readonly SettingDef[] NameSetting = [Setting("name", "Name", new SettingKind.Name(""))];

    /// <summary>The node kinds of animation states.</summary>
    public static readonly Catalog Catalog = new()
    {
        Domain = "animation",
        Label = "Animation States",
        Extension = "banim",
        Starter = [("entry", new Vector2(40f, 40f)), ("state", new Vector2(40f, 140f)), ("clip", new Vector2(-220f, 140f))],
        Nodes =
        [
            new("entry", "Start State", "States", "The state an object starts in. Without one, it starts in the first state.",
                [], [], [Setting("state", "State", new SettingKind.Name("Idle"))], true),
            new("state", "State", "States", "A state the object can be in, by its name, playing the motion linked in (at rest when none is).",
                [Pin("motion", "Motion", PinType.Motion)], [], [Setting("name", "Name", new SettingKind.Name("Idle"))], false),
            new("transition", "Transition", "States",
                "Goes from one state (or, with From left empty, from any other) to another while When is true, fading over Fade seconds. Transitions are tried in the order they were placed.",
                [Flag("when", "When", false)], [],
                [Setting("from", "From", new SettingKind.Name("")), Setting("to", "To", new SettingKind.Name("")), Setting("fade", "Fade", new SettingKind.Number(0.25f))], false),
            new("clip", "Clip", "Motions", "One of the model's clips by name, at a speed, looping or once.", [], MotionOut,
                [Setting("clip", "Clip", new SettingKind.Text("")), Setting("speed", "Speed", new SettingKind.Number(1f)), Setting("play", "Play", new SettingKind.Choice(["Loop", "Once"]))], false),
            new("mix", "Mix", "Motions",
                "A and B by where By lies from At A to At B (walk and run by speed, aiming up and down by angle); before At A all A, past At B all B. Both play in step. Link a Mix into A or B to mix more.",
                [Pin("a", "A", PinType.Motion), Pin("b", "B", PinType.Motion), Num("by", "By", 0f)], MotionOut,
                [Setting("atA", "At A", new SettingKind.Number(0f)), Setting("atB", "At B", new SettingKind.Number(1f))], false),
            new("layer", "Layer", "Motions",
                "Top over Base on the joint named Joint and every joint under it (a wave on the arm while the legs walk), by Weight from 0 to 1.",
                [Pin("base", "Base", PinType.Motion), Pin("top", "Top", PinType.Motion), Num("weight", "Weight", 1f)], MotionOut,
                [Setting("joint", "Joint", new SettingKind.Text(""))], false),
            new("number", "Number Parameter", "Parameters", "A number parameter scripts set (0 until one does).", [], [Pin("value", "Value", PinType.Number)], NameSetting, false),
            new("flag", "True/False Parameter", "Parameters", "A true/false parameter scripts set (false until one does).", [], [Pin("value", "Value", PinType.Bool)], NameSetting, false),
            new("trigger", "Trigger Parameter", "Parameters", "True once a script triggers it, until a transition it lets through uses it up.", [],
                [Pin("value", "Value", PinType.Bool)], NameSetting, false),
            new("stateTime", "Time in State", "Conditions",
                "Seconds since the state began, and Runs: how many times its motion has played through (1 at the end of its first run).", [],
                [Pin("seconds", "Seconds", PinType.Number), Pin("runs", "Runs", PinType.Number)], [], false),
            new("compare", "Compare", "Conditions", "Whether A is greater than, less than, at least, at most, equal to or not equal to B.",
                [Num("a", "A", 0f), Num("b", "B", 0f)], BoolOut,
                [Setting("test", "Test", new SettingKind.Choice(["Greater", "Less", "At Least", "At Most", "Equal", "Not Equal"]))], false),
            new("and", "And", "Conditions", "True when A and B both are.", [Flag("a", "A", true), Flag("b", "B", true)], BoolOut, [], false),
            new("or", "Or", "Conditions", "True when A or B is.", [Flag("a", "A", false), Flag("b", "B", false)], BoolOut, [], false),
            new("not", "Not", "Conditions", "True when In is false.", [Flag("in", "In", false)], BoolOut, [], false),
        ],
    };

    /// <summary>Deepest nesting of motions or of conditions.</summary>
    private const int MaxDepth = 64;

    private sealed class Compiler(Bayleaf.Graph.Graph graph)
    {
        public List<Problem> Problems { get; } = [];

        public SortedDictionary<string, ParameterKind> Parameters { get; } = new(Utf8Order.Instance);

        public string Text(uint node, string name, string fallback) => graph.ValueOf(node, name) is JsonString s ? s.Value : fallback;

        public float Float(uint node, string name, float fallback) =>
            graph.ValueOf(node, name) is { } v && Bayleaf.Graph.Graph.AsF32(v) is { } f && float.IsFinite(f) ? f : fallback;

        private bool Bool(uint node, string name, bool fallback) => graph.ValueOf(node, name)?.AsBool() ?? fallback;

        private string Kind(uint node) => graph.FindNode(node)?.Kind ?? "";

        private string Label(uint node) => Catalog.Get(Kind(node))?.Label ?? "Node";

        /// <summary>The node and output linked into <paramref name="node"/>'s input <paramref name="pin"/>.</summary>
        private PinRef? Source(uint node, string pin) => graph.LinkInto(node, pin)?.From;

        private void On(uint node, string message) => Problems.Add(Problem.On(node, message));

        public string Parameter(uint node, ParameterKind kind)
        {
            var name = Text(node, "name", "");
            if (name.Length == 0)
            {
                On(node, $"{Label(node)} · give the parameter a name");
                return name;
            }
            if (Parameters.TryGetValue(name, out var other) && other != kind)
            {
                On(node, $"{Label(node)} · \"{name}\" is a {other.Label()} parameter elsewhere in the graph; one name holds one kind");
            }
            else
            {
                Parameters[name] = kind;
            }
            return name;
        }

        /// <summary>The motion linked into <paramref name="node"/>'s input <paramref name="pin"/>; at rest when nothing is.</summary>
        public Motion MotionInto(uint node, string pin, int depth) => Source(node, pin) is { } from ? MotionAt(from.Node, depth + 1) : Motion.AtRest;

        private Motion MotionAt(uint node, int depth)
        {
            if (depth > MaxDepth)
            {
                On(node, "motions are nested more than 64 deep");
                return Motion.AtRest;
            }
            switch (Kind(node))
            {
                case "clip":
                    var clip = Text(node, "clip", "");
                    if (clip.Length == 0)
                    {
                        On(node, "Clip · name one of the model's clips");
                    }
                    var speed = Float(node, "speed", 1f);
                    if (!(speed is >= 0f and <= 10f))
                    {
                        On(node, "Clip · speed must be from 0 to 10");
                    }
                    return new Motion.Clip(clip, Math.Clamp(speed, 0f, 10f), Text(node, "play", "Loop") != "Once");
                case "mix":
                    var (a, b) = (MotionInto(node, "a", depth), MotionInto(node, "b", depth));
                    if (Source(node, "a") is null || Source(node, "b") is null)
                    {
                        On(node, "Mix · link a motion into both A and B");
                    }
                    var by = NumberInto(node, "by", 0f);
                    return new Motion.Mix([new(a, Float(node, "atA", 0f)), new(b, Float(node, "atB", 1f))], by);
                case "layer":
                    var (lower, upper) = (MotionInto(node, "base", depth), MotionInto(node, "top", depth));
                    if (Source(node, "top") is null)
                    {
                        On(node, "Layer · link the motion it plays into Top");
                    }
                    var joint = Text(node, "joint", "");
                    var weight = NumberInto(node, "weight", 1f);
                    return new Motion.Layer(lower, upper, joint, weight);
                default:
                    On(node, $"{Label(node)} gives no motion");
                    return Motion.AtRest;
            }
        }

        /// <summary>A number a motion reads, linked into <paramref name="pin"/> (a number parameter) or typed in.</summary>
        private Number NumberInto(uint node, string pin, float fallback)
        {
            if (Source(node, pin) is not { } from)
            {
                return Number.Of(Float(node, pin, fallback));
            }
            if (Kind(from.Node) == "number")
            {
                return Number.Named(Parameter(from.Node, ParameterKind.Number));
            }
            On(node, $"{Label(node)} · a motion reads a number parameter or a number typed in, not {Label(from.Node)}");
            return Number.Of(fallback);
        }

        /// <summary>A number a condition reads.</summary>
        private Amount AmountInto(uint node, string pin)
        {
            if (Source(node, pin) is not { } from)
            {
                return new Amount.Value(Float(node, pin, 0f));
            }
            switch (Kind(from.Node))
            {
                case "number":
                    return new Amount.Named(Parameter(from.Node, ParameterKind.Number));
                case "stateTime" when from.Pin == "runs":
                    return new Amount.Runs();
                case "stateTime":
                    return new Amount.Seconds();
                default:
                    On(node, $"{Label(node)} · cannot read a number from {Label(from.Node)}");
                    return new Amount.Value(0f);
            }
        }

        public Condition ConditionInto(uint node, string pin, bool fallback, int depth) =>
            Source(node, pin) is { } from ? ConditionAt(from.Node, depth + 1) : new Condition.Value(Bool(node, pin, fallback));

        private Condition ConditionAt(uint node, int depth)
        {
            if (depth > MaxDepth)
            {
                On(node, "conditions are nested more than 64 deep");
                return new Condition.Value(false);
            }
            switch (Kind(node))
            {
                case "flag":
                    return new Condition.Flag(Parameter(node, ParameterKind.Flag));
                case "trigger":
                    return new Condition.Trigger(Parameter(node, ParameterKind.Trigger));
                case "compare":
                    var test = Text(node, "test", "Greater") switch
                    {
                        "Less" => Test.Less,
                        "At Least" => Test.AtLeast,
                        "At Most" => Test.AtMost,
                        "Equal" => Test.Equal,
                        "Not Equal" => Test.NotEqual,
                        _ => Test.Greater,
                    };
                    return new Condition.Compare(AmountInto(node, "a"), test, AmountInto(node, "b"));
                case "and":
                    return new Condition.Both(ConditionInto(node, "a", true, depth), ConditionInto(node, "b", true, depth));
                case "or":
                    return new Condition.Either(ConditionInto(node, "a", false, depth), ConditionInto(node, "b", false, depth));
                case "not":
                    return new Condition.Negated(ConditionInto(node, "in", false, depth));
                default:
                    On(node, $"{Label(node)} gives no condition");
                    return new Condition.Value(false);
            }
        }
    }

    /// <summary>Compiles animation states; every problem found, each on its node where it has one.</summary>
    public static Result<Machine, IReadOnlyList<Problem>> Compile(Bayleaf.Graph.Graph graph)
    {
        if (graph.Domain != Catalog.Domain)
        {
            return Result.Fail<IReadOnlyList<Problem>>([Problem.OfGraph($"this is a {graph.Domain} graph, not animation states")]);
        }
        var c = new Compiler(graph);
        var states = new List<State>();
        foreach (var node in graph.Nodes.Where(n => n.Kind == "state"))
        {
            var name = c.Text(node.Id, "name", "Idle");
            if (name.Length == 0)
            {
                c.Problems.Add(Problem.On(node.Id, "State · give the state a name"));
            }
            else if (states.Any(s => s.Name == name))
            {
                c.Problems.Add(Problem.On(node.Id, $"State · another state is named \"{name}\""));
            }
            var motion = c.MotionInto(node.Id, "motion", 0);
            states.Add(new State(name, motion));
        }
        if (states.Count == 0)
        {
            c.Problems.Add(Problem.OfGraph("add a State: animation states need one to start in"));
        }
        int? Find(string name)
        {
            var i = states.FindIndex(s => s.Name == name);
            return i >= 0 ? i : null;
        }
        var start = 0;
        if (graph.Nodes.Find(n => n.Kind == "entry") is { } entry)
        {
            var name = c.Text(entry.Id, "state", "Idle");
            if (Find(name) is { } i)
            {
                start = i;
            }
            else
            {
                c.Problems.Add(Problem.On(entry.Id, $"Start State · there is no state named \"{name}\""));
            }
        }
        var transitions = new List<Transition>();
        foreach (var node in graph.Nodes.Where(n => n.Kind == "transition"))
        {
            var (from, to) = (c.Text(node.Id, "from", ""), c.Text(node.Id, "to", ""));
            var fromIndex = from.Length == 0 ? null : Find(from);
            if (from.Length > 0 && fromIndex is null)
            {
                c.Problems.Add(Problem.On(node.Id, $"Transition · there is no state named \"{from}\" to go from"));
            }
            if (Find(to) is not { } toIndex)
            {
                c.Problems.Add(Problem.On(node.Id, to.Length == 0 ? "Transition · name the state it goes to" : $"Transition · there is no state named \"{to}\" to go to"));
                continue;
            }
            var fade = c.Float(node.Id, "fade", 0.25f);
            if (!(fade is >= 0f and <= Player.MaxFade))
            {
                c.Problems.Add(Problem.On(node.Id, $"Transition · fade must be from 0 to {Bayleaf.Core.Text.Number(Player.MaxFade)} seconds"));
            }
            var when = c.ConditionInto(node.Id, "when", false, 0);
            if (from.Length > 0 && fromIndex is null)
            {
                continue;
            }
            transitions.Add(new Transition(fromIndex, toIndex, Math.Clamp(fade, 0f, Player.MaxFade), when));
        }
        // Parameter nodes no condition or motion reads still declare their parameter.
        foreach (var node in graph.Nodes)
        {
            switch (node.Kind)
            {
                case "number":
                    c.Parameter(node.Id, ParameterKind.Number);
                    break;
                case "flag":
                    c.Parameter(node.Id, ParameterKind.Flag);
                    break;
                case "trigger":
                    c.Parameter(node.Id, ParameterKind.Trigger);
                    break;
                default:
                    break;
            }
        }
        if (c.Problems.Count == 0)
        {
            return new Machine { States = states, Start = start, Transitions = transitions, Parameters = c.Parameters };
        }
        return Result.Fail<IReadOnlyList<Problem>>([.. c.Problems.Distinct()]);
    }
}
