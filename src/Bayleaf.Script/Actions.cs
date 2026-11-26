using Bayleaf.Core;

namespace Bayleaf.Script;

/// <summary>How an action reports itself.</summary>
public enum ActionMode
{
    /// <summary>1 in the one step it goes down, else 0: a jump, a shot.</summary>
    Trigger,

    /// <summary>1 from the step it goes down for as long as it is held, else 0.</summary>
    Held,

    /// <summary>How far it is pressed, 0 to 1 (a stick or trigger leans part way).</summary>
    Value,
}

/// <summary>An action as an Input Map writes it: its map, name, how it reports itself and what presses it (<c>key:W</c>, <c>mouse:Left</c>, <c>pad:South</c>, <c>axis:LeftX+</c>).</summary>
public sealed record ActionSpec(string Map, string Name, ActionMode Mode, IReadOnlyList<string> Bindings);

/// <summary>
/// The actions of an Input Map, sorted into maps (already checked: <c>InputAsset.CheckedMaps</c>); one
/// map is active at a time, the start map first. Scripts ask for the action rather than the key, so
/// players can rebind.
/// </summary>
public sealed class ActionMaps
{
    /// <summary>How far a stick must lean before it counts.</summary>
    public const float StickDeadZone = 0.15f;

    private readonly List<Action> actions;

    private ActionMaps(List<Action> actions, IReadOnlyList<string> maps, string start)
    {
        this.actions = actions;
        Maps = maps;
        Start = start;
    }

    /// <summary>The maps, in their order.</summary>
    public IReadOnlyList<string> Maps { get; }

    /// <summary>The map that is active when play starts.</summary>
    public string Start { get; }

    private sealed record Binding(int Kind, string Key, int Index, bool Positive)
    {
        public float Value(PlayInput input) => Kind switch
        {
            0 => input.Keys.Contains(Key) ? 1f : 0f,
            1 => (input.Buttons >> Index) & 1,
            2 => input.Pad is { } pad && ((pad.Buttons >> Index) & 1) == 1 ? 1f : 0f,
            _ => input.Pad is { } p ? Lean(Index < p.Axes.Count ? p.Axes[Index] : 0f) : 0f,
        };

        private float Lean(float v)
        {
            var along = Positive ? v : -v;
            return along < StickDeadZone ? 0f : Math.Min((along - StickDeadZone) / (1f - StickDeadZone), 1f);
        }

        public static Binding? Parse(string text)
        {
            var colon = text.IndexOf(':', StringComparison.Ordinal);
            if (colon < 0)
            {
                return null;
            }
            var (kind, name) = (text[..colon], text[(colon + 1)..]);
            switch (kind)
            {
                case "key":
                    return new Binding(0, name, 0, true);
                case "mouse":
                    var button = name switch
                    {
                        "Left" => 0,
                        "Right" => 1,
                        "Middle" => 2,
                        _ => -1,
                    };
                    return button < 0 ? null : new Binding(1, "", button, true);
                case "pad":
                    var index = PlayInput.PadButtons.ToList().IndexOf(name);
                    return index < 0 ? null : new Binding(2, "", index, true);
                case "axis" when name.Length > 1 && name[^1] is '+' or '-':
                    var axis = PlayInput.PadAxes.ToList().IndexOf(name[..^1]);
                    return axis < 0 ? null : new Binding(3, "", axis, name[^1] == '+');
                default:
                    return null;
            }
        }
    }

    private sealed record Action(string Map, string Name, ActionMode Mode, IReadOnlyList<Binding> Bindings)
    {
        public string Full { get; } = $"{Map}/{Name}";

        public float Raw(PlayInput input)
        {
            var strongest = 0f;
            foreach (var binding in Bindings)
            {
                strongest = Math.Max(strongest, binding.Value(input));
            }
            return strongest;
        }
    }

    /// <summary>Maps from checked actions; bindings that do not read are left out.</summary>
    public static ActionMaps Of(IReadOnlyList<string> maps, string start, IEnumerable<ActionSpec> specs)
    {
        var actions = specs.Select(s => new Action(s.Map, s.Name, s.Mode, s.Bindings.Select(Binding.Parse).OfType<Binding>().ToList())).ToList();
        return new ActionMaps(actions, maps, start.Length == 0 ? (maps.Count > 0 ? maps[0] : "") : start);
    }

    /// <summary>The action <paramref name="name"/> means while <paramref name="active"/> is the active map; actions of other maps are not active.</summary>
    private Action? Find(string active, string name)
    {
        var slash = name.IndexOf('/', StringComparison.Ordinal);
        var (map, action) = slash >= 0 ? (name[..slash], name[(slash + 1)..]) : (active, name);
        return map == active ? actions.Find(a => a.Map == map && a.Name == action) : null;
    }

    /// <summary>An Input Map in play: which map is active, and which actions were down in the last step and this one.</summary>
    public sealed class State(ActionMaps maps)
    {
        private HashSet<string> before = new(StringComparer.Ordinal);
        private HashSet<string> now = new(StringComparer.Ordinal);

        public ActionMaps Maps { get; } = maps;

        public string Active { get; private set; } = maps.Start;

        /// <summary>Makes <paramref name="map"/> active; false when the asset has no such map.</summary>
        public bool SetActive(string map)
        {
            if (!Maps.Maps.Contains(map))
            {
                return false;
            }
            Active = map;
            return true;
        }

        /// <summary>Takes the step's input: which actions of every map are down now, so one held while its map becomes active does not count as going down then.</summary>
        public void Step(PlayInput input)
        {
            (before, now) = (now, before);
            now.Clear();
            foreach (var action in Maps.actions)
            {
                if (action.Raw(input) > 0.5f)
                {
                    now.Add(action.Full);
                }
            }
        }

        /// <summary>The action's value now, by its kind; 0 for an action of an inactive map or an unknown one.</summary>
        public float Value(string name, PlayInput input)
        {
            if (Maps.Find(Active, name) is not { } action)
            {
                return 0f;
            }
            return action.Mode switch
            {
                ActionMode.Value => action.Raw(input),
                ActionMode.Held => now.Contains(action.Full) ? 1f : 0f,
                _ => now.Contains(action.Full) && !before.Contains(action.Full) ? 1f : 0f,
            };
        }

        /// <summary>True in the step the action went down, whatever its kind.</summary>
        public bool Pressed(string name) => Maps.Find(Active, name) is { } a && now.Contains(a.Full) && !before.Contains(a.Full);
    }
}

/// <summary>The Input Maps scripts read in play: the project's, and objects' own (an Input Map component).</summary>
public sealed class InputSet
{
    public ActionMaps.State? Project { get; set; }

    public Dictionary<StableId, ActionMaps.State> Objects { get; } = [];

    /// <summary>What an object's scripts read: its own Input Map, else the project's.</summary>
    public ActionMaps.State? Of(StableId id) => Objects.TryGetValue(id, out var own) ? own : Project;

    /// <summary>Takes the step's input in every map.</summary>
    public void Step(PlayInput input)
    {
        Project?.Step(input);
        foreach (var state in Objects.Values)
        {
            state.Step(input);
        }
    }
}
